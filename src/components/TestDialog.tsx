import { useEffect, useRef, useState } from "react";
import { Check, Minus, RefreshCw, X, Zap } from "lucide-react";
import { api, EndpointConfig, TestStep } from "../api";
import { Modal } from "./ui";
import { fmtMs } from "../utils";

type Row = Omit<TestStep, "status"> & { status: TestStep["status"] | "busy" | "wait" };

/** 分步测试：网络 → 登录 / 鉴权 → 每个模型能否回复。每一步完成就立刻显示。 */
export default function TestDialog(props: { endpoint: EndpointConfig; onClose: () => void }) {
  const ep = props.endpoint;
  const models = ep.models.filter((m) => m.enabled).slice(0, 12);
  const initial = (): Row[] => [
    { kind: "network", label: "网络", status: "wait", detail: "", ms: 0 },
    { kind: "auth", label: "登录 / 鉴权", status: "wait", detail: "", ms: 0 },
    ...models.map((m): Row => ({ kind: "model", label: m.name, status: "wait", detail: "", ms: 0 })),
  ];
  const [rows, setRows] = useState<Row[]>(initial);
  const [running, setRunning] = useState(false);
  const runId = useRef(0);

  const run = async () => {
    const id = ++runId.current;
    setRunning(true);
    let cur = initial();
    const put = (i: number, r: Partial<Row>) => {
      if (id !== runId.current) return; // 已经开始了新一轮
      cur = cur.map((x, j) => (j === i ? { ...x, ...r } : x));
      setRows(cur);
    };
    // 前一步失败：后面的步骤标成「未测试」
    const skipFrom = (i: number, why: string) => cur.forEach((_, j) => j >= i && put(j, { status: "skip", detail: why }));

    try {
      put(0, { status: "busy" });
      const net = await api.testStep(ep.id, "network");
      put(0, net);
      if (net.status === "fail") return skipFrom(1, "网络不通，未测试");

      put(1, { status: "busy" });
      const auth = await api.testStep(ep.id, "auth");
      put(1, auth);
      if (auth.status === "fail") return skipFrom(2, "登录 / 鉴权失败，未测试");

      if (!models.length) return;
      models.forEach((_, k) => put(2 + k, { status: "busy" }));
      // 各模型并行测，谁先回来先显示谁
      await Promise.all(
        models.map((m, k) =>
          api
            .testStep(ep.id, "model", m.name)
            .then((r) => put(2 + k, r))
            .catch((e) => put(2 + k, { status: "fail", detail: String(e) })),
        ),
      );
    } catch (e) {
      const i = cur.findIndex((x) => x.status === "busy");
      if (i >= 0) put(i, { status: "fail", detail: String(e) });
    } finally {
      if (id === runId.current) setRunning(false);
    }
  };

  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const failed = rows.find((r) => r.status === "fail");
  const summary = running
    ? null
    : !models.length
      ? "这个端点还没有配置模型"
      : failed
        ? failed.kind === "network"
          ? "网络不通，没有继续测试"
          : failed.kind === "auth"
            ? "登录 / 鉴权失败，没有继续测试"
            : `${rows.filter((r) => r.kind === "model" && r.status === "fail").length} 个模型不可用`
        : `全部通过：${models.length} 个模型都能正常回复`;

  return (
    <Modal
      title={`测试「${ep.name}」`}
      icon={<Zap size={18} />}
      onClose={() => {
        runId.current++; // 关掉后不再更新
        props.onClose();
      }}
      footer={
        <>
          <button className="btn" onClick={run} disabled={running}>
            <RefreshCw size={15} />
            重新测试
          </button>
          <button
            className="btn btn-primary"
            onClick={() => {
              runId.current++;
              props.onClose();
            }}
          >
            完成
          </button>
        </>
      }
    >
      <p className="muted-sm" style={{ marginTop: 0 }}>
        依次检查：网络能否连到上游 → 登录 / API Key 是否有效 → 每个启用的模型实际发一句话，能否正常回复。
      </p>
      <div className="steps">
        {rows.map((s, i) => (
          <div key={i} className={`step ${s.status}`}>
            <span className="step-icon">
              {s.status === "ok" ? (
                <Check size={14} />
              ) : s.status === "fail" ? (
                <X size={14} />
              ) : s.status === "skip" ? (
                <Minus size={14} />
              ) : s.status === "busy" ? (
                <i className="spin" />
              ) : null}
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
              <span>{s.status === "busy" ? "测试中…" : s.status === "wait" ? "等待中" : s.detail}</span>
            </div>
            {s.ms > 0 && s.status !== "busy" && <span className="step-ms">{fmtMs(s.ms)}</span>}
          </div>
        ))}
      </div>
      {summary && (
        <div className={`banner ${failed || !models.length ? "err" : "ok"}`} style={{ marginTop: 14, marginBottom: 0 }}>
          <b>{summary}</b>
        </div>
      )}
    </Modal>
  );
}
