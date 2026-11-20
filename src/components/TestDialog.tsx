import { useEffect, useRef, useState } from "react";
import { Check, Minus, RefreshCw, X, Zap } from "lucide-react";
import { api, EndpointConfig, TestReport } from "../api";
import { Modal } from "./ui";
import { fmtMs } from "../utils";

/** 分步测试：网络 → 登录 / 鉴权 → 每个模型能否回复 */
export default function TestDialog(props: { endpoint: EndpointConfig; onClose: () => void }) {
  const [report, setReport] = useState<TestReport | null>(null);
  const started = useRef(false);
  const run = async () => {
    setReport(null);
    setReport(await api.testEndpointReport(props.endpoint.id));
  };
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const models = props.endpoint.models.filter((m) => m.enabled);
  const summary = !report
    ? null
    : report.ok
      ? `全部通过：${report.steps.filter((s) => s.kind === "model" && s.status === "ok").length} 个模型都能正常回复`
      : (() => {
          const f = report.steps.find((s) => s.status === "fail")!;
          return f.kind === "network" ? "网络不通，没有继续测试" : f.kind === "auth" ? "登录 / 鉴权失败，没有继续测试" : "有模型不可用";
        })();

  return (
    <Modal
      title={`测试「${props.endpoint.name}」`}
      icon={<Zap size={18} />}
      onClose={props.onClose}
      footer={
        <>
          <button className="btn" onClick={run} disabled={!report}>
            <RefreshCw size={15} />
            重新测试
          </button>
          <button className="btn btn-primary" onClick={props.onClose}>
            完成
          </button>
        </>
      }
    >
      <p className="muted-sm" style={{ marginTop: 0 }}>
        依次检查：网络能否连到上游 → 登录 / API Key 是否有效 → 每个启用的模型实际发一句话，能否正常回复。
      </p>
      <div className="steps">
        {(report?.steps ?? placeholder(models.map((m) => m.name))).map((s, i) => (
          <div key={i} className={`step ${s.status}`}>
            <span className="step-icon">
              {s.status === "ok" ? <Check size={14} /> : s.status === "fail" ? <X size={14} /> : s.status === "skip" ? <Minus size={14} /> : <i className="sdot busy" />}
            </span>
            <div className="step-body">
              <b>
                {s.kind === "model" ? (
                  <>
                    模型 <code>{s.label}</code>
                  </>
                ) : (
                  s.label
                )}
              </b>
              <span>{s.detail}</span>
            </div>
            {s.ms > 0 && <span className="step-ms">{fmtMs(s.ms)}</span>}
          </div>
        ))}
      </div>
      {summary && <div className={`banner ${report!.ok ? "ok" : "err"}`} style={{ marginTop: 14, marginBottom: 0 }}><b>{summary}</b></div>}
    </Modal>
  );
}

function placeholder(models: string[]) {
  return [
    { kind: "network", label: "网络", status: "busy", detail: "连接中…", ms: 0 },
    { kind: "auth", label: "登录 / 鉴权", status: "busy", detail: "", ms: 0 },
    ...models.map((m) => ({ kind: "model", label: m, status: "busy", detail: "", ms: 0 })),
  ];
}
