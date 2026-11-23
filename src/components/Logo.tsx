/** KP 字母标志：白色圆头线条 + 粉色圆点（与应用图标一致） */
export default function LogoMark(props: { size?: number }) {
  const s = props.size ?? 20;
  return (
    <svg width={s} height={s} viewBox="250 300 560 430" fill="none" aria-label="kiki-proxy">
      <g stroke="white" strokeWidth="80" strokeLinecap="round" strokeLinejoin="round">
        <path d="M300 342 V682" />
        <path d="M466 342 L318 522 L478 682" />
        <path d="M598 682 V342 H678 A100 100 0 0 1 678 542 H598" />
      </g>
      <circle cx="768" cy="662" r="40" fill="#ff8cc4" />
    </svg>
  );
}
