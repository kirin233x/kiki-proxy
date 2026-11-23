import { useEffect, useRef, useState } from "react";
import { Play, RotateCw, Square } from "lucide-react";
import { api, ProcessStatus } from "../api";
import { StatusDot, useToast } from "./ui";
import { fmtAgo } from "../utils";

const STATE: Record<ProcessStatus["state"], { label: string; tone: "ok" | "warn" | "err" | "off" | "busy" }> = {
  running: { label: "运行中", tone: "ok" },
  starting: { label: "正在启动", tone: "busy" },
  restarting: { label: "正在重启", tone: "warn" },
  stopped: { label: "已停止", tone: "off" },
  failed: { label: "启动失败", tone: "err" },
};

/** 本地程序的运行状态 + 输出日志（每 2 秒刷新） */
export default function ProcessPanel(props: { endpointId: string; dirty?: boolean }) {
  const [st, setSt] = useState<ProcessStatus | null | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const toast = useToast();
  const act = async (f: (id: string) => Promise<void>) => {
    setBusy(true);
    try {
      await f(props.endpointId);
      setSt(await api.processStatus(props.endpointId));
    } catch (e) {
      toast(String(e), true);
    } finally {
      setBusy(false);
    }
  };
  const logRef = useRef<HTMLPreElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    let alive = true;
    const tick = () => api.processStatus(props.endpointId).then((s) => alive && setSt(s));
    tick();
    const t = setInterval(tick, 2000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [props.endpointId]);

  // 新输出到来时，停在底部（用户往上翻了就不自动滚）
  useEffect(() => {
    const el = logRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [st?.logs.length]);

  if (st === undefined) return null;
  const meta = st ? STATE[st.state] : { label: "未运行", tone: "off" as const };
  const alive = st && ["running", "starting", "restarting"].includes(st.state);
  // 停下来的原因：网关暂停 / 手动停止 / 连续失败放弃
  const why = st && !alive ? (st.paused ? "网关已暂停，启动网关后会自动运行" : st.lastExit) : "";

  return (
    <div className="proc">
      <div className="proc-head">
        <StatusDot tone={meta.tone} />
        <b>{meta.label}</b>
        {st?.pid && <span className="mono-sm">PID {st.pid}</span>}
        {st?.state === "running" && st.startedAt > 0 && <span className="muted-sm">{fmtAgo(st.startedAt)}启动</span>}
        {!!st?.restarts && (
          <span className="muted-sm" title={st.lastExit}>
            自动重启过 {st.restarts} 次
          </span>
        )}
        <span style={{ flex: 1 }} />
        {props.dirty && <span className="muted-sm">命令改了，保存后会自动重启</span>}
        {alive ? (
          <>
            <button className="btn btn-sm" disabled={busy} onClick={() => act(api.stopProcess)}>
              <Square size={13} />
              停止
            </button>
            <button className="btn btn-sm" disabled={busy} onClick={() => act(api.restartProcess)}>
              <RotateCw size={14} />
              重启
            </button>
          </>
        ) : (
          <button className="btn btn-sm btn-primary" disabled={busy || !!st?.paused} onClick={() => act(api.startProcess)}>
            <Play size={13} />
            启动
          </button>
        )}
      </div>
      {why && <div className={`proc-why ${st?.state === "failed" ? "err" : ""}`}>{why}</div>}
      <pre
        className="proc-log"
        ref={logRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {st?.logs.length ? st.logs.join("\n") : "还没有输出"}
      </pre>
    </div>
  );
}

/** 端点卡片上的小标签：本地程序是否在运行（每 3 秒刷新） */
export function ProcessBadge(props: { endpointId: string }) {
  const [st, setSt] = useState<ProcessStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const tick = () => api.processStatus(props.endpointId).then((s) => alive && setSt(s));
    tick();
    const t = setInterval(tick, 3000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [props.endpointId]);
  const meta = st ? (st.paused ? { label: "已暂停", tone: "off" as const } : STATE[st.state]) : { label: "未运行", tone: "off" as const };
  return (
    <span className="proc-badge" title={st?.lastExit || st?.command}>
      <StatusDot tone={meta.tone} />
      本地程序{meta.label}
    </span>
  );
}
