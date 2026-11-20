import { useEffect, useId, useRef, useState } from "react";
import { Plus } from "lucide-react";
import { useApp } from "../App";

/** 选择网关模型（也可以手动输入，比如 `ds/*` 或未列出的模型） */
export default function ModelInput(props: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  extra?: string[];
  style?: React.CSSProperties;
}) {
  const { models } = useApp();
  const id = useId();
  return (
    <>
      <input
        className="input mono"
        list={id}
        value={props.value}
        placeholder={props.placeholder ?? "端点前缀/模型名"}
        onChange={(e) => props.onChange(e.target.value.trim())}
        style={props.style}
        spellCheck={false}
      />
      <datalist id={id}>
        {(props.extra ?? []).map((m) => (
          <option key={m} value={m} />
        ))}
        {models.map((m) => (
          <option key={m.id} value={m.id}>
            {m.displayName} · {m.endpoint}
          </option>
        ))}
      </datalist>
    </>
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
  const below = open ? window.innerHeight - open.bottom > 320 : true;

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
      {open && (
        <div
          ref={ref}
          className="picker"
          style={{
            left: Math.min(open.left, window.innerWidth - 340),
            ...(below ? { top: open.bottom + 6 } : { bottom: window.innerHeight - open.top + 6 }),
          }}
        >
          <input className="input" autoFocus placeholder="搜索模型" value={q} onChange={(e) => setQ(e.target.value)} />
          <div className="picker-list">
            {[...groups.entries()].map(([ep, ms]) => (
              <div key={ep}>
                <div className="picker-group">{ep}</div>
                {ms.map((m) => (
                  <button
                    key={m.id}
                    onClick={() => {
                      props.onPick(m.id);
                      setOpen(null);
                    }}
                  >
                    <code>{m.id}</code>
                    {m.displayName !== m.id && <span>{m.displayName}</span>}
                  </button>
                ))}
              </div>
            ))}
            {avail.length === 0 && <div className="muted-sm" style={{ padding: 12 }}>没有可添加的模型</div>}
          </div>
        </div>
      )}
    </>
  );
}
