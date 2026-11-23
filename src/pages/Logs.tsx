import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowRight, CornerDownRight, Radio, RefreshCw, ScrollText, Search, Terminal, Trash2, X } from "lucide-react";
import { api, listen, LogBodies, LogQuery, LogRecord } from "../api";
import { useApp } from "../App";
import { CopyButton, Empty, Modal, StatusDot, useToast } from "../components/ui";
import { fmtCost, fmtMs, fmtNum, fmtTime, pretty, statusTone } from "../utils";

const PAGE = 100;

export default function Logs({ initialEndpoint }: { initialEndpoint?: string }) {
  const { config } = useApp();
  const [q, setQ] = useState("");
  const [endpoint, setEndpoint] = useState(initialEndpoint ?? "");
  const [status, setStatus] = useState<LogQuery["status"]>("all");
  const [rows, setRows] = useState<LogRecord[]>([]);
  const [more, setMore] = useState(false);
  const [live, setLive] = useState(true);
  const [selected, setSelected] = useState<string | null>(null);
  const [clear, setClear] = useState(false);
  const toast = useToast();
  const query = useRef<LogQuery>({});

  const load = useCallback(async () => {
    query.current = { q, endpoint, status, limit: PAGE };
    const r = await api.listLogs(query.current);
    setRows(r);
    setMore(r.length === PAGE);
  }, [q, endpoint, status]);

  useEffect(() => {
    const t = setTimeout(load, 200);
    return () => clearTimeout(t);
  }, [load]);

  // 实时追加（只在没有筛选时）
  useEffect(() => {
    if (!live) return;
    const off = listen<LogRecord>("log", (rec) => {
      const f = query.current;
      if (f.q || f.endpoint || (f.status && f.status !== "all")) return;
      setRows((xs) => [rec, ...xs.filter((x) => x.id !== rec.id)]);
    });
    return () => {
      off.then((f) => f());
    };
  }, [live]);

  const loadMore = async () => {
    const before = rows[rows.length - 1]?.ts;
    const r = await api.listLogs({ ...query.current, before });
    setRows((xs) => [...xs, ...r]);
    setMore(r.length === PAGE);
  };

  return (
    <div className={`browse ${selected ? "has-drawer" : ""}`}>
      <div className="browse-main">
        <div className="page-header" style={{ marginTop: 0 }}>
          <div>
            <h1 className="page-title">请求日志</h1>
            <div className="page-subtitle">
              完整的请求和响应内容保留 {config.logging.bodyRetentionHours} 小时，摘要保留 {config.logging.statsRetentionDays} 天
            </div>
          </div>
          <div className="header-actions">
            <button className={`btn ${live ? "btn-soft" : ""}`} onClick={() => setLive(!live)} title="有新请求时自动出现在列表顶部">
              <Radio size={15} />
              {live ? "实时" : "已暂停"}
            </button>
            <button className="btn" onClick={load}>
              <RefreshCw size={15} />
            </button>
            <button className="btn" onClick={() => setClear(true)}>
              <Trash2 size={15} />
            </button>
          </div>
        </div>
        <div className="toolbar">
          <div className="search-box">
            <Search size={16} />
            <input className="input" placeholder="搜索模型名、错误信息、请求 ID" value={q} onChange={(e) => setQ(e.target.value)} />
          </div>
          <select className="select" value={endpoint} onChange={(e) => setEndpoint(e.target.value)}>
            <option value="">全部端点</option>
            {config.endpoints.map((e) => (
              <option key={e.id} value={e.id}>
                {e.name}
              </option>
            ))}
          </select>
          <div className="segmented">
            {(
              [
                ["all", "全部"],
                ["ok", "成功"],
                ["error", "失败"],
              ] as const
            ).map(([k, l]) => (
              <button key={k} className={status === k ? "on" : ""} onClick={() => setStatus(k)}>
                {l}
              </button>
            ))}
          </div>
        </div>
        <div className="table-wrap">
          {rows.length === 0 ? (
            <Empty icon={<ScrollText size={22} />} title="没有请求记录" />
          ) : (
            <table className="cards logs">
              <thead>
                <tr>
                  <th style={{ width: 92 }}>时间</th>
                  <th style={{ width: 72 }}>状态</th>
                  <th>模型</th>
                  <th className="col-optional" style={{ width: 128 }}>端点</th>
                  <th className="col-optional" style={{ width: 128 }}>客户端</th>
                  <th className="col-optional" style={{ width: 118, textAlign: "right" }}>输入 / 输出</th>
                  <th style={{ width: 84, textAlign: "right" }}>耗时</th>
                  <th className="col-optional" style={{ width: 76, textAlign: "right" }}>费用</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.id} className={selected === r.id ? "selected" : ""} onClick={() => setSelected(r.id)}>
                    <td className="mono-sm">{fmtTime(r.ts)}</td>
                    <td>
                      {r.pending ? (
                        <span className="st busy">
                          <StatusDot tone="busy" />
                        </span>
                      ) : (
                        <span className={`st ${statusTone(r.status)}`}>{r.status}</span>
                      )}
                    </td>
                    <td>
                      <div className="log-model">
                        <code>{r.gatewayModel || r.requestedModel}</code>
                        {r.attempts.length > 0 && <span className="tag warn">切换 ×{r.attempts.length}</span>}
                        {r.rule && r.rule !== "故障切换" && <span className="tag">{r.rule}</span>}
                      </div>
                      {r.error ? (
                        <div className="log-err">{r.error}</div>
                      ) : (
                        r.requestedModel !== r.gatewayModel &&
                        r.gatewayModel && (
                          <div className="log-sub">
                            <CornerDownRight size={11} />
                            请求的是 {r.requestedModel}
                          </div>
                        )
                      )}
                    </td>
                    <td className="col-optional">{r.endpointName || "—"}</td>
                    <td className="muted-cell col-optional">
                      {r.clientProtocol}
                      {r.stream && <span className="stream-tag">流</span>}
                    </td>
                    <td className="num col-optional">{r.pending ? "…" : `${fmtNum(r.usage.input + r.usage.cacheRead + r.usage.cacheWrite)} / ${fmtNum(r.usage.output)}`}</td>
                    <td className="num">{r.pending ? "…" : fmtMs(r.durationMs)}</td>
                    <td className="num col-optional">{r.cost ? fmtCost(r.cost) : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {more && (
            <div className="table-foot center">
              <button className="btn btn-sm" onClick={loadMore}>
                加载更多
              </button>
            </div>
          )}
        </div>
      </div>
      {selected && <LogDrawer id={selected} onClose={() => setSelected(null)} />}
      {clear && (
        <Modal
          title="清空请求日志"
          icon={<Trash2 size={18} />}
          onClose={() => setClear(false)}
          footer={
            <>
              <button className="btn btn-ghost" onClick={() => setClear(false)}>
                取消
              </button>
              <button
                className="btn btn-danger"
                onClick={() => {
                  setClear(false);
                  api.clearLogs().then(load);
                }}
              >
                清空全部
              </button>
              <button
                className="btn btn-primary"
                onClick={() => {
                  setClear(false);
                  api.clearFailedLogs().then((n) => {
                    toast(n ? `已删除 ${n} 条失败的请求` : "没有失败的请求");
                    load();
                  });
                }}
              >
                只清空失败的
              </button>
            </>
          }
        >
          <p style={{ margin: 0 }}>
            「只清空失败的」会删除状态码不是 2xx、或者带错误信息的请求，成功的请求和用量统计都保留。
            <br />
            「清空全部」会删除所有请求记录和用量统计，不能恢复。
          </p>
        </Modal>
      )}
    </div>
  );
}

type Tab = "clientRequest" | "upstreamRequest" | "upstreamResponse" | "clientResponse" | "headers";

function LogDrawer({ id, onClose }: { id: string; onClose: () => void }) {
  const { baseUrl } = useApp();
  const [data, setData] = useState<{ record: LogRecord; bodies: LogBodies | null } | null>(null);
  const [tab, setTab] = useState<Tab>("clientRequest");

  useEffect(() => {
    let alive = true;
    const fetch = () => api.getLog(id).then((d) => alive && setData(d));
    fetch();
    // 进行中的流式请求结束后刷新
    const off = listen<LogRecord>("log", (r) => r.id === id && !r.pending && fetch());
    return () => {
      alive = false;
      off.then((f) => f());
    };
  }, [id]);

  if (!data) return <aside className="drawer" />;
  const { record: r, bodies: b } = data;
  const tabs: [Tab, string][] = [
    ["clientRequest", "客户端请求"],
    ["upstreamRequest", "上游请求"],
    ["upstreamResponse", "上游响应"],
    ["clientResponse", "返回客户端"],
    ["headers", "请求头"],
  ];
  const content = !b
    ? ""
    : tab === "headers"
      ? `# 客户端 → 网关\n${b.clientHeaders}\n\n# 网关 → 上游  ${b.upstreamUrl}\n${b.upstreamHeaders}`
      : tab === "upstreamRequest"
        ? pretty(b.upstreamRequest)
        : pretty(b[tab]);
  const curl = b ? `curl -sS ${baseUrl}${r.path.startsWith("/v1beta") ? r.path : r.path} \\\n  -H 'content-type: application/json' \\\n  --data-binary @- <<'EOF'\n${b.clientRequest}\nEOF` : "";

  return (
    <aside className="drawer log-drawer">
      <div className="drawer-head">
        <h3>
          <span className={`st ${r.pending ? "busy" : statusTone(r.status)}`}>{r.pending ? "…" : r.status}</span>
          {r.gatewayModel || r.requestedModel}
        </h3>
        <button className="icon-btn" onClick={onClose}>
          <X size={18} />
        </button>
      </div>
      <div className="drawer-body">
        {r.error && <div className="banner err">{r.error}</div>}
        <div className="info-grid">
          <div className="info-item">
            <span>时间</span>
            <b>{new Date(r.ts).toLocaleString("zh-CN")}</b>
          </div>
          <div className="info-item">
            <span>耗时 / 首字节</span>
            <b>
              {fmtMs(r.durationMs)} / {fmtMs(r.ttfbMs)}
            </b>
          </div>
          <div className="info-item">
            <span>客户端</span>
            <b>
              {r.clientProtocol} {r.stream ? "· 流式" : ""}
            </b>
          </div>
          <div className="info-item">
            <span>上游</span>
            <b>
              {r.endpointName || "—"} · {r.upstreamProtocol || "—"}
            </b>
          </div>
          <div className="info-item">
            <span>请求模型</span>
            <b className="mono">{r.requestedModel}</b>
          </div>
          <div className="info-item">
            <span>上游模型</span>
            <b className="mono">{r.upstreamModel || "—"}</b>
          </div>
          <div className="info-item">
            <span>出口 / 规则</span>
            <b>
              {r.proxy || "—"}
              {r.rule ? ` · ${r.rule}` : ""}
            </b>
          </div>
          <div className="info-item">
            <span>Token（入/缓存/出）</span>
            <b>
              {fmtNum(r.usage.input)} / {fmtNum(r.usage.cacheRead + r.usage.cacheWrite)} / {fmtNum(r.usage.output)}
            </b>
          </div>
        </div>

        {r.attempts.length > 0 && (
          <div className="drawer-section">
            <h4>故障切换</h4>
            <div className="attempts">
              {r.attempts.map((a, i) => (
                <div key={i} className="attempt">
                  <span className="st err">{a.status}</span>
                  <code>{a.gatewayModel}</code>
                  <span className="attempt-err">{a.error}</span>
                  <span className="mono-sm">{fmtMs(a.durationMs)}</span>
                </div>
              ))}
              <div className="attempt ok">
                <span className={`st ${statusTone(r.status)}`}>{r.status}</span>
                <code>{r.gatewayModel}</code>
                <ArrowRight size={12} />
                <span className="attempt-err">{r.status === 200 ? "成功" : "最终失败"}</span>
              </div>
            </div>
          </div>
        )}

        <div className="drawer-section">
          <div className="tabs">
            {tabs.map(([k, l]) => (
              <button key={k} className={tab === k ? "on" : ""} onClick={() => setTab(k)}>
                {l}
              </button>
            ))}
          </div>
          {!b ? (
            <div className="muted-sm" style={{ padding: 16 }}>
              完整内容已过保留期被清理（只保留摘要）
            </div>
          ) : (
            <>
              <div className="code-actions">
                {tab === "upstreamRequest" && b.upstreamUrl && <code className="url">POST {b.upstreamUrl}</code>}
                {tab === "upstreamResponse" && b.upstreamStatus > 0 && <code className="url">HTTP {b.upstreamStatus}</code>}
                <span style={{ flex: 1 }} />
                {tab === "clientRequest" && <CopyButton text={curl} label="复制为 curl" />}
                <CopyButton text={content} label="复制" />
              </div>
              <pre className="code">{content || "（空）"}</pre>
            </>
          )}
        </div>
        <div className="muted-sm mono" style={{ marginTop: 8 }}>
          <Terminal size={11} /> {r.id}
        </div>
      </div>
    </aside>
  );
}
