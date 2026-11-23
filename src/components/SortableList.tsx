import { CSSProperties, ReactNode, useRef, useState } from "react";
import { GripVertical } from "lucide-react";

/**
 * 拖动排序：按住手柄拖动，其他行实时让位，松手后落到新位置。
 * 用指针事件实现（不用 HTML5 拖放，Tauri 窗口会拦截它）。
 * 行需要带 data-sort-row 属性，并放在 listRef 指向的容器里
 */
export function useSortable(onReorder: (from: number, to: number) => void) {
  const listRef = useRef<HTMLDivElement>(null);
  const rects = useRef<DOMRect[]>([]);
  const startY = useRef(0);
  // 页面滚动容器：拖到边缘时自动滚动，位置计算要把滚过的距离算进去
  const scroller = useRef<HTMLElement | null>(null);
  const startScroll = useRef(0);
  const lastY = useRef(0);
  const raf = useRef(0);
  const [drag, setDragState] = useState<{ from: number; to: number; dy: number } | null>(null);
  const dragRef = useRef(drag);
  const setDrag = (d: typeof drag) => {
    dragRef.current = d;
    setDragState(d);
  };

  const update = (clientY: number) => {
    const d = dragRef.current;
    if (!d) return;
    const r = rects.current;
    const self = r[d.from];
    const scrolled = (scroller.current?.scrollTop ?? 0) - startScroll.current;
    // 允许稍微超出首尾一点，保证能拖到第一个/最后一个
    const dy = Math.max(r[0].top - self.top - 10, Math.min(r[r.length - 1].bottom - self.bottom + 10, clientY - startY.current + scrolled));
    const center = self.top + self.height / 2 + dy;
    let to = d.from;
    for (let j = 0; j < d.from; j++) {
      if (center <= r[j].top + r[j].height / 2) {
        to = j;
        break;
      }
    }
    if (to === d.from) {
      for (let j = r.length - 1; j > d.from; j--) {
        if (center >= r[j].top + r[j].height / 2) {
          to = j;
          break;
        }
      }
    }
    setDrag({ ...d, dy, to });
  };

  // 指针靠近滚动区域上下边缘时自动滚动
  const autoScroll = () => {
    const el = scroller.current;
    if (!dragRef.current || !el) return;
    const box = el.getBoundingClientRect();
    const edge = 70;
    const y = lastY.current;
    let v = 0;
    if (y < box.top + edge) v = -Math.ceil(((box.top + edge - y) / edge) * 14);
    else if (y > box.bottom - edge) v = Math.ceil(((y - (box.bottom - edge)) / edge) * 14);
    if (v) {
      const before = el.scrollTop;
      el.scrollTop += v;
      if (el.scrollTop !== before) update(lastY.current);
    }
    raf.current = requestAnimationFrame(autoScroll);
  };

  const shift = (j: number) => {
    if (!drag || j === drag.from) return 0;
    const r = rects.current;
    const h = r[drag.from] ? r[drag.from].height + gapAfter(r, drag.from) : 0;
    if (drag.from < drag.to && j > drag.from && j <= drag.to) return -h;
    if (drag.to < drag.from && j >= drag.to && j < drag.from) return h;
    return 0;
  };

  const handle = (i: number) => ({
    onPointerDown: (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      const rows = Array.from(listRef.current?.querySelectorAll<HTMLElement>("[data-sort-row]") ?? []);
      rects.current = rows.map((r) => r.getBoundingClientRect());
      startY.current = lastY.current = e.clientY;
      scroller.current = scrollParent(listRef.current);
      startScroll.current = scroller.current?.scrollTop ?? 0;
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      setDrag({ from: i, to: i, dy: 0 });
      cancelAnimationFrame(raf.current);
      raf.current = requestAnimationFrame(autoScroll);
    },
    onPointerMove: (e: React.PointerEvent) => {
      if (!dragRef.current) return;
      lastY.current = e.clientY;
      update(e.clientY);
    },
    onPointerUp: () => finish(),
    onPointerCancel: () => finish(),
  });

  const finish = () => {
    cancelAnimationFrame(raf.current);
    const d = dragRef.current;
    if (!d) return;
    const { from, to } = d;
    setDrag(null);
    if (from !== to) onReorder(from, to);
  };

  return {
    listRef,
    sorting: !!drag,
    handle,
    /** 行的样式：被拖的行跟着指针，其他行让位 */
    row: (i: number): { className: string; style: CSSProperties; "data-sort-row": true } => ({
      className: drag?.from === i ? "sort-dragging" : "",
      style: drag ? { transform: `translateY(${drag.from === i ? drag.dy : shift(i)}px)` } : {},
      "data-sort-row": true,
    }),
    /** 拖动中这一行落下后的序号 */
    index: (i: number) => (!drag ? i : drag.from === i ? drag.to : i + Math.sign(shift(i))),
  };
}

function scrollParent(el: HTMLElement | null): HTMLElement | null {
  for (let p = el?.parentElement; p; p = p.parentElement) {
    const o = getComputedStyle(p).overflowY;
    if ((o === "auto" || o === "scroll") && p.scrollHeight > p.clientHeight) return p;
  }
  return (document.scrollingElement as HTMLElement) ?? null;
}

/** 两行之间的间距（行之间有 gap 时也要让出来） */
function gapAfter(r: DOMRect[], i: number) {
  if (i + 1 < r.length) return Math.max(0, r[i + 1].top - r[i].bottom);
  if (i > 0) return Math.max(0, r[i].top - r[i - 1].bottom);
  return 0;
}

export function SortHandle(props: React.HTMLAttributes<HTMLSpanElement>) {
  return (
    <span className="sort-handle" title="拖动调整顺序" {...props}>
      <GripVertical size={15} />
    </span>
  );
}

/** 可拖动排序的列表 */
export default function SortableList<T>(props: {
  items: T[];
  keyOf: (t: T) => string;
  render: (t: T, i: number) => ReactNode;
  /** 行右侧的额外操作 */
  extra?: (t: T, i: number) => ReactNode;
  onReorder: (from: number, to: number) => void;
  empty?: ReactNode;
}) {
  const s = useSortable(props.onReorder);
  if (!props.items.length) return <div className="sort-empty">{props.empty ?? "暂无数据"}</div>;
  return (
    <div ref={s.listRef} className={`sort-list ${s.sorting ? "is-sorting" : ""}`}>
      {props.items.map((t, i) => {
        const r = s.row(i);
        return (
          <div key={props.keyOf(t)} {...r} className={`sort-row ${r.className}`}>
            <SortHandle {...s.handle(i)} />
            <span className="sort-num">{s.index(i) + 1}</span>
            <div className="sort-main">{props.render(t, i)}</div>
            {props.extra?.(t, i)}
          </div>
        );
      })}
    </div>
  );
}
