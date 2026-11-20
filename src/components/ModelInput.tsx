import { useId } from "react";
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
  const { models } = useApp();
  const list = props.value;
  const available = models.filter((m) => !list.includes(m.id) && !(props.exclude ?? []).includes(m.id));
  return (
    <div className="chain">
      {list.map((m, i) => (
        <span key={m} className="chain-item">
          <em>{i + 1}</em>
          <code>{m}</code>
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
      <select
        className="select chain-add"
        value=""
        onChange={(e) => e.target.value && props.onChange([...list, e.target.value])}
      >
        <option value="">＋ 添加备选模型</option>
        {available.map((m) => (
          <option key={m.id} value={m.id}>
            {m.id}（{m.endpoint}）
          </option>
        ))}
      </select>
    </div>
  );
}
