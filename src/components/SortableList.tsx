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
  const [drag, setDrag] = useState<{ from: number; to: number; dy: number } | null>(null);

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
      startY.current = e.clientY;
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      setDrag({ from: i, to: i, dy: 0 });
    },
    onPointerMove: (e: React.PointerEvent) => {
      if (!drag) return;
      const r = rects.current;
      const self = r[drag.from];
      const dy = Math.max(r[0].top - self.top, Math.min(r[r.length - 1].bottom - self.bottom, e.clientY - startY.current));
      const center = self.top + self.height / 2 + dy;
      let to = drag.from;
      for (let j = 0; j < drag.from; j++) {
        if (center < r[j].top + r[j].height / 2) {
          to = j;
          break;
        }
      }
      if (to === drag.from) {
        for (let j = r.length - 1; j > drag.from; j--) {
          if (center > r[j].top + r[j].height / 2) {
            to = j;
            break;
          }
        }
      }
      setDrag({ ...drag, dy, to });
    },
    onPointerUp: () => finish(),
    onPointerCancel: () => finish(),
  });

  const finish = () => {
    if (!drag) return;
    const { from, to } = drag;
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
