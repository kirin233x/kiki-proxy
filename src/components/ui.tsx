import { createContext, ReactNode, useCallback, useContext, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, Copy, Info, X } from "lucide-react";

// ------------------------------------------------------------------ 悬停提示

/** 小号「i」图标，悬停或聚焦时显示说明（渲染到 body，不会被卡片裁掉） */
export function Hint(props: { children: ReactNode }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [at, setAt] = useState<DOMRect | null>(null);
  const show = () => ref.current && setAt(ref.current.getBoundingClientRect());
  const hide = () => setAt(null);
  return (
    <span ref={ref} className="hint" tabIndex={0} onMouseEnter={show} onMouseLeave={hide} onFocus={show} onBlur={hide}>
      <Info size={13} />
      {at &&
        createPortal(
          <span
            className="hint-pop"
            style={{
              left: Math.max(12, Math.min(at.left + at.width / 2 - 150, window.innerWidth - 312)),
              ...(at.top > 140 ? { bottom: window.innerHeight - at.top + 8 } : { top: at.bottom + 8 }),
            }}
          >
            {props.children}
          </span>,
          document.body,
        )}
    </span>
  );
}

// ------------------------------------------------------------------ Modal

export function Modal(props: { title: string; icon?: ReactNode; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean; closable?: boolean }) {
  const { onClose, closable = true } = props;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && closable) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, closable]);
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && closable && onClose()}>
      <div className={`modal ${props.wide ? "wide" : ""}`} role="dialog">
        <div className="modal-head">
          {props.icon && <div className="modal-icon">{props.icon}</div>}
          <h2 className="modal-title">{props.title}</h2>
          {closable && (
            <button className="icon-btn" onClick={onClose} aria-label="关闭">
              <X size={18} />
            </button>
          )}
        </div>
        <div className="modal-body">{props.children}</div>
        {props.footer && <div className="modal-foot">{props.footer}</div>}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ Toast

interface ToastItem {
  id: number;
  text: string;
  error?: boolean;
}

const ToastCtx = createContext<(text: string, error?: boolean) => void>(() => {});

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const show = useCallback((text: string, error = false) => {
    const id = Date.now() + Math.random();
    setItems((xs) => [...xs, { id, text, error }]);
    setTimeout(() => setItems((xs) => xs.filter((x) => x.id !== id)), error ? 5000 : 2600);
  }, []);
  return (
    <ToastCtx.Provider value={show}>
      {children}
      <div className="toast-wrap">
        {items.map((t) => (
          <div key={t.id} className={`toast ${t.error ? "error" : ""}`}>
            {t.text}
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

export const useToast = () => useContext(ToastCtx);

// ------------------------------------------------------------------ Dropdown menu

export function Menu(props: { anchor: HTMLElement; onClose: () => void; items: ({ label: string; icon?: ReactNode; danger?: boolean; onClick: () => void } | "sep")[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const rect = props.anchor.getBoundingClientRect();
  const { onClose } = props;
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    setTimeout(() => window.addEventListener("mousedown", onDown), 0);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);
  const left = Math.min(rect.right - 170, window.innerWidth - 186);
  return (
    <div ref={ref} className="menu" style={{ position: "fixed", top: rect.bottom + 6, left: Math.max(8, left) }}>
      {props.items.map((it, i) =>
        it === "sep" ? (
          <hr key={i} />
        ) : (
          <button
            key={i}
            className={it.danger ? "danger" : ""}
            onClick={() => {
              onClose();
              it.onClick();
            }}
          >
            {it.icon}
            {it.label}
          </button>
        ),
      )}
    </div>
  );
}

// ------------------------------------------------------------------ Confirm

export function Confirm(props: { title: string; message: ReactNode; confirmText?: string; danger?: boolean; onConfirm: () => void; onClose: () => void }) {
  return (
    <Modal
      title={props.title}
      onClose={props.onClose}
      footer={
        <>
          <button className="btn" onClick={props.onClose}>
            取消
          </button>
          <button
            className={`btn ${props.danger ? "btn-danger" : "btn-primary"}`}
            autoFocus
            onClick={() => {
              props.onConfirm();
              props.onClose();
            }}
          >
            {props.confirmText ?? "确定"}
          </button>
        </>
      }
    >
      <div style={{ color: "var(--muted)", lineHeight: 1.7 }}>{props.message}</div>
    </Modal>
  );
}

export function Switch(props: { on: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button className={`switch ${props.on ? "on" : ""}`} role="switch" aria-checked={props.on} disabled={props.disabled} onClick={() => props.onChange(!props.on)} />
  );
}

// ------------------------------------------------------------------ 小部件

export function Field(props: { label: string; help?: ReactNode; children: ReactNode; required?: boolean }) {
  return (
    <div className="field">
      <label className="field-label">
        <span>
          {props.label}
          {props.required && <i className="req"> *</i>}
          {props.help && <Hint>{props.help}</Hint>}
        </span>
      </label>
      {props.children}
    </div>
  );
}

export function CopyButton(props: { text: string; label?: string; small?: boolean }) {
  const [done, setDone] = useState(false);
  return (
    <button
      className={`btn ${props.small ? "btn-xs" : "btn-sm"} btn-ghost`}
      onClick={(e) => {
        e.stopPropagation();
        navigator.clipboard?.writeText(props.text).catch(() => {});
        setDone(true);
        setTimeout(() => setDone(false), 1400);
      }}
    >
      {done ? <Check size={14} /> : <Copy size={14} />}
      {props.label !== undefined ? (done ? "已复制" : props.label) : null}
    </button>
  );
}

/** 只写不读的密钥输入框：显示「已设置」，输入新值才会覆盖 */
export function SecretInput(props: { isSet: boolean; value: string; onChange: (v: string) => void; onClear?: () => void; placeholder?: string }) {
  return (
    <div className="secret-input">
      <input
        className="input"
        type="password"
        autoComplete="off"
        value={props.value}
        placeholder={props.isSet ? "已保存在钥匙串中，输入新值可覆盖" : (props.placeholder ?? "未设置")}
        onChange={(e) => props.onChange(e.target.value)}
      />
      {props.isSet && !props.value && <span className="secret-badge">已设置</span>}
      {props.isSet && props.onClear && (
        <button className="btn btn-sm btn-ghost" onClick={props.onClear} title="从钥匙串删除">
          清除
        </button>
      )}
    </div>
  );
}

export function StatusDot(props: { tone: "ok" | "warn" | "err" | "off" | "busy"; title?: string }) {
  return <i className={`sdot ${props.tone}`} title={props.title} />;
}

export function Empty(props: { icon: ReactNode; title: string; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="empty-mini">
      <div className="empty-mini-icon">{props.icon}</div>
      <b>{props.title}</b>
      {props.children && <p>{props.children}</p>}
      {props.action}
    </div>
  );
}
