import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown, Plus } from "lucide-react";
import { useApp } from "../App";
import { fmtContext } from "../api";

type Option = { id: string; displayName: string; endpoint: string; contextWindow: number };

/**
 * 选择网关模型：点击弹出按端点分组的列表，输入即筛选；也可以手动输入（比如 `ds/*` 或未列出的模型）。
 * 不用原生 datalist：WebKit 里的样式又挤又没法改
 */
export default function ModelInput(props: { value: string; onChange: (v: string) => void; placeholder?: string; extra?: string[]; style?: React.CSSProperties }) {
  const { models } = useApp();
  const inputRef = useRef<HTMLInputElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const [pos, setPos] = useState<{ left: number; width: number; top?: number; bottom?: number } | null>(null);

  const options: Option[] = [...models, ...(props.extra ?? []).map((id) => ({ id, displayName: "", endpoint: "通配", contextWindow: 0 }))];
  const q = (query ?? "").toLowerCase();
  const list = q ? options.filter((m) => (m.id + " " + m.displayName).toLowerCase().includes(q)) : options;
  const groups: [string, Option[]][] = [];
  list.forEach((m) => {
    const g = groups.find((x) => x[0] === m.endpoint);
    if (g) g[1].push(m);
    else groups.push([m.endpoint, [m]]);
  });
  const flat = groups.flatMap((g) => g[1]);

  const place = () => {
    const r = inputRef.current?.getBoundingClientRect();
    if (!r) return;
    const width = Math.max(r.width, 380);
    const left = Math.min(r.left, window.innerWidth - width - 12);
    const below = window.innerHeight - r.bottom > 340 || r.top < 340;
    setPos(below ? { left, width, top: r.bottom + 6 } : { left, width, bottom: window.innerHeight - r.top + 6 });
  };

  const show = () => {
    place();
    setQuery(null);
    const i = options.findIndex((m) => m.id === props.value);
    setActive(Math.max(0, i));
    setOpen(true);
  };
  const close = () => {
    setOpen(false);
    setQuery(null);
  };
  const pick = (id: string) => {
    props.onChange(id);
    close();
  };

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!popRef.current?.contains(t) && !inputRef.current?.contains(t)) close();
    };
    const onScroll = (e: Event) => !popRef.current?.contains(e.target as Node) && place();
    window.addEventListener("mousedown", onDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", place);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", place);
    };
  }, [open]);

  // 键盘选中的项滚到可见位置
  useLayoutEffect(() => {
    if (!open) return;
    popRef.current?.querySelector(".combo-opt.active")?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) return show();
      setActive((a) => (flat.length ? (a + (e.key === "ArrowDown" ? 1 : -1) + flat.length) % flat.length : 0));
    } else if (e.key === "Enter" && open) {
      e.preventDefault();
      if (flat[active]) pick(flat[active].id);
      else close();
    } else if (e.key === "Escape" || e.key === "Tab") {
      close();
    }
  };

  let idx = -1;
  return (
    <div className={`combo ${open ? "open" : ""}`} style={props.style}>
      <input
        ref={inputRef}
        className="input mono"
        value={props.value}
        placeholder={props.placeholder ?? "前缀/模型 ID"}
        spellCheck={false}
        onFocus={() => !open && show()}
        onClick={() => !open && show()}
        onKeyDown={onKey}
        onChange={(e) => {
          props.onChange(e.target.value.trim());
          setQuery(e.target.value.trim());
          setActive(0);
          if (!open) {
            place();
            setOpen(true);
          }
        }}
      />
      <ChevronDown size={15} className="combo-caret" onMouseDown={(e) => (e.preventDefault(), open ? close() : (inputRef.current?.focus(), show()))} />
      {open &&
        pos &&
        createPortal(
          <div ref={popRef} className="combo-pop" style={pos}>
            {flat.length === 0 ? (
              <div className="combo-empty">{props.value ? <>没有匹配的模型，将按输入值「{props.value}」使用</> : "暂无模型"}</div>
            ) : (
              groups.map(([ep, ms]) => (
                <div key={ep} className="combo-group">
                  <div className="combo-group-title">{ep}</div>
                  {ms.map((m) => {
                    idx++;
                    const i = idx;
                    const selected = m.id === props.value;
                    return (
                      <button
                        key={m.id}
                        type="button"
                        className={`combo-opt ${i === active ? "active" : ""} ${selected ? "selected" : ""}`}
                        onMouseEnter={() => setActive(i)}
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => pick(m.id)}
                      >
                        <span className="combo-check">{selected && <Check size={14} />}</span>
                        <span className="combo-text">
                          <code>{m.id}</code>
                          {m.displayName && m.displayName !== m.id && <small>{m.displayName}</small>}
                        </span>
                        {m.contextWindow > 0 && <span className={`combo-ctx ${m.contextWindow >= 1_000_000 ? "is-1m" : ""}`}>{fmtContext(m.contextWindow)}</span>}
                      </button>
                    );
                  })}
                </div>
              ))
            )}
          </div>,
          document.body,
        )}
    </div>
  );
}

/** 多个模型（故障切换链），按顺序 */
export function ModelChain(props: { value: string[]; onChange: (v: string[]) => void; exclude?: string[] }) {
  const list = props.value;
  return (
    <div className="chain">
      {list.map((m, i) => (
        <span key={m} className="chain-item">
          <em>{i + 1}</em>
          <code title={m}>{m}</code>
          {i > 0 && (
            <button title="提前" onClick={() => props.onChange(list.map((x, j) => (j === i - 1 ? m : j === i ? list[i - 1] : x)))}>
              ↑
            </button>
          )}
          <button title="移除" onClick={() => props.onChange(list.filter((x) => x !== m))}>
            ×
          </button>
        </span>
      ))}
      <ModelPicker exclude={[...list, ...(props.exclude ?? [])]} onPick={(id) => props.onChange([...list, id])} label={list.length ? "添加" : "添加备选模型"} />
    </div>
  );
}

/** 「＋ 添加」按钮 + 弹出的模型列表（按端点分组、可搜索） */
export function ModelPicker(props: { exclude?: string[]; onPick: (id: string) => void; label: string }) {
  const { models } = useApp();
  const [open, setOpen] = useState<DOMRect | null>(null);
  const [q, setQ] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => ref.current && !ref.current.contains(e.target as Node) && setOpen(null);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(null);
    // 弹层是 fixed 定位，页面滚动时直接关掉（列表自己的滚动除外）
    const onScroll = (e: Event) => ref.current && !ref.current.contains(e.target as Node) && setOpen(null);
    setTimeout(() => window.addEventListener("mousedown", onDown), 0);
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open]);

  const avail = models.filter((m) => !(props.exclude ?? []).includes(m.id) && (m.id + m.displayName + m.endpoint).toLowerCase().includes(q.toLowerCase()));
  const groups = new Map<string, typeof avail>();
  avail.forEach((m) => groups.set(m.endpoint, [...(groups.get(m.endpoint) ?? []), m]));
  // 放不下时往上弹
  const below = open ? window.innerHeight - open.bottom > 340 : true;

  return (
    <>
      <button
        className="chain-add"
        onClick={(e) => {
          setQ("");
          setOpen(open ? null : e.currentTarget.getBoundingClientRect());
        }}
      >
        <Plus size={14} />
        {props.label}
      </button>
      {open &&
        createPortal(
          <div
            ref={ref}
            className="combo-pop with-search"
            style={{
              left: Math.min(open.left, window.innerWidth - 392),
              width: 380,
              ...(below ? { top: open.bottom + 6 } : { bottom: window.innerHeight - open.top + 6 }),
            }}
          >
            <input className="input combo-search" autoFocus placeholder="搜索模型" value={q} onChange={(e) => setQ(e.target.value)} />
            {[...groups.entries()].map(([ep, ms]) => (
              <div key={ep} className="combo-group">
                <div className="combo-group-title">{ep}</div>
                {ms.map((m) => (
                  <button
                    key={m.id}
                    type="button"
                    className="combo-opt"
                    onClick={() => {
                      props.onPick(m.id);
                      setOpen(null);
                    }}
                  >
                    <span className="combo-check" />
                    <span className="combo-text">
                      <code>{m.id}</code>
                      {m.displayName !== m.id && <small>{m.displayName}</small>}
                    </span>
                    {m.contextWindow > 0 && <span className={`combo-ctx ${m.contextWindow >= 1_000_000 ? "is-1m" : ""}`}>{fmtContext(m.contextWindow)}</span>}
                  </button>
                ))}
              </div>
            ))}
            {avail.length === 0 && <div className="combo-empty">没有可添加的模型</div>}
          </div>,
          document.body,
        )}
    </>
  );
}
