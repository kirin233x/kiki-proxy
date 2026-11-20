import { fmtContext } from "../api";

const OPTIONS = [0, 32_768, 65_536, 131_072, 200_000, 262_144, 400_000, 1_000_000, 2_000_000];

/** 上下文窗口选择；价格表里查到的非标准值也能显示 */
export default function ContextSelect(props: { value: number; onChange: (v: number) => void }) {
  const opts = OPTIONS.includes(props.value) ? OPTIONS : [...OPTIONS, props.value].sort((a, b) => a - b);
  return (
    <select
      className={`select ctx-select ${props.value >= 1_000_000 ? "is-1m" : ""}`}
      value={props.value}
      onChange={(e) => props.onChange(Number(e.target.value))}
      title={props.value >= 1_000_000 ? "Claude Code 会按 1M 上下文使用" : undefined}
    >
      {opts.map((v) => (
        <option key={v} value={v}>
          {fmtContext(v)}
        </option>
      ))}
    </select>
  );
}
