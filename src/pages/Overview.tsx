import { useEffect, useState } from "react";
import { Activity, ArrowRight, Coins, Gauge, Plus, Radio, RefreshCw, Server, Zap } from "lucide-react";
import { api, listen, LogRecord, protocolLabel, Stats, TestResult } from "../api";
import { useApp } from "../App";
import { CopyButton, Empty, StatusDot } from "../components/ui";
import { fmtCost, fmtMs, fmtNum, fmtTime, protocolColor, statusTone } from "../utils";

export default function Overview() {
  const { config, status, go, baseUrl } = useApp();
  const [stats, setStats] = useState<Stats | null>(null);
  const [recent, setRecent] = useState<LogRecord[]>([]);
  const [health, setHealth] = useState<Record<string, TestResult | "busy">>({});

  useEffect(() => {
    api.getStats(1).then(setStats);
    api.listLogs({ limit: 8 }).then(setRecent);
    const off = listen<LogRecord>("log", (rec) => {
      setRecent((xs) => [rec, ...xs.filter((x) => x.id !== rec.id)].slice(0, 8));
      if (!rec.pending) api.getStats(1).then(setStats);
    });
    return () => {
      off.then((f) => f());
    };
  }, []);

  const testAll = () => {
    config.endpoints
      .filter((e) => e.enabled)
      .forEach((e) => {
        setHealth((h) => ({ ...h, [e.id]: "busy" }));
        api.testEndpoint(e.id).then((r) => setHealth((h) => ({ ...h, [e.id]: r })));
      });
  };

  const t = stats?.total;
  const errRate = t && t.requests ? (t.errors / t.requests) * 100 : 0;
  const modelCount = config.endpoints.filter((e) => e.enabled).reduce((n, e) => n + e.models.filter((m) => m.enabled).length, 0);

  if (!config.endpoints.length) {
    return (
      <div className="page">
        <div className="page-inner">
          <div className="empty">
            <div className="empty-art">
              <div className="c1" />
              <div className="c2" />
              <div className="c3">
                <Server size={46} />
              </div>
            </div>
            <h2>先添加一个端点</h2>
            <p>
              端点就是一个上游服务：官方 API、中转站，或者你们的内部服务。
              <br />
              添加后，客户端用「端点前缀/模型名」就能访问它。
            </p>
            <div className="empty-actions">
              <button className="btn btn-primary btn-lg" onClick={() => go({ name: "endpoint", id: null })}>
                <Plus size={16} />
                添加端点
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <div className="page-inner">
        <div className="page-header">
          <div>
            <h1 className="page-title">概览</h1>
            <div className="page-subtitle">所有客户端都连 {baseUrl}，按模型名分流到不同端点</div>
          </div>
        </div>

        <div className="hero">
          <div className="hero-main">
            <div className="hero-label">今日请求</div>
            <div className="hero-number">
              {fmtNum(t?.requests ?? 0)}
              <small>次</small>
            </div>
            <div className="hero-split">
              <span>
                <i style={{ background: "#a5f3fc" }} />
                输入 {fmtNum((t?.input ?? 0) + (t?.cacheRead ?? 0))}
              </span>
              <span>
                <i style={{ background: "#fde68a" }} />
                输出 {fmtNum(t?.output ?? 0)}
              </span>
              <span>
                <i style={{ background: errRate > 5 ? "#fca5a5" : "#bbf7d0" }} />
                失败率 {errRate.toFixed(1)}%
              </span>
            </div>
            <div style={{ display: "flex", gap: 10 }}>
              <button className="btn" onClick={() => go({ name: "logs" })}>
                <Activity size={16} />
                查看请求日志
              </button>
              <button className="btn hero-ghost" onClick={() => go({ name: "clients" })}>
                接入客户端
                <ArrowRight size={15} />
              </button>
            </div>
          </div>
          <div className="hero-stats">
            <div className="hero-stat">
              <div className="hero-stat-num">{fmtCost(t?.cost ?? 0)}</div>
              <div className="hero-stat-label">
                <Coins size={12} />
                今日费用
              </div>
            </div>
            <div className="hero-stat">
              <div className="hero-stat-num">{t?.requests ? fmtMs(t.avgMs) : "—"}</div>
              <div className="hero-stat-label">
                <Gauge size={12} />
                平均耗时
              </div>
            </div>
            <div className="hero-stat">
              <div className="hero-stat-num">{modelCount}</div>
              <div className="hero-stat-label">
                <Zap size={12} />
                可用模型
              </div>
            </div>
          </div>
        </div>

        <div className="section-head">
          <h2 className="section-title">端点</h2>
          <button className="btn btn-sm" onClick={testAll}>
            <RefreshCw size={14} />
            全部测试
          </button>
        </div>
        <div className="ep-grid">
          {config.endpoints.map((e) => {
            const h = health[e.id];
            const proxyName =
              e.proxy === "direct" ? "直连" : e.proxy === "system" ? "系统代理" : config.proxies.find((p) => p.id === e.proxy)?.name ?? e.proxy;
            return (
              <button key={e.id} className={`ep-card ${e.enabled ? "" : "off"}`} onClick={() => go({ name: "endpoint", id: e.id })}>
                <div className="ep-card-top">
                  <span className="proto-tag" style={{ ["--c" as string]: protocolColor[e.protocol], ["--cb" as string]: protocolColor[e.protocol] + "1f" }}>
                    {protocolLabel(e.protocol)}
                  </span>
                  {h === "busy" ? (
                    <StatusDot tone="busy" />
                  ) : h ? (
                    <span className={`health ${h.ok ? "ok" : "err"}`} title={h.message}>
                      <StatusDot tone={h.ok ? "ok" : "err"} />
                      {h.ok ? fmtMs(h.latencyMs) : h.status || "失败"}
                    </span>
                  ) : !e.enabled ? (
                    <span className="muted-sm">已停用</span>
                  ) : null}
                </div>
                <div className="ep-card-name">{e.name}</div>
                <div className="ep-card-id">
                  <code>{e.id}/</code>
                  <span>{e.models.length} 个模型</span>
                </div>
                <div className="ep-card-meta">
                  <span>
                    <Radio size={12} />
                    {proxyName}
                  </span>
                  {e.plugin && <span className="plugin-tag">插件</span>}
                  {e.fallbacks.length > 0 && <span>↪ {e.fallbacks.length} 个备选</span>}
                </div>
              </button>
            );
          })}
        </div>

        <div className="section-head" style={{ marginTop: 30 }}>
          <h2 className="section-title">最近请求</h2>
          <button className="btn btn-sm btn-ghost" onClick={() => go({ name: "logs" })}>
            全部
            <ArrowRight size={14} />
          </button>
        </div>
        <div className="panel">
          {recent.length === 0 ? (
            <Empty icon={<Activity size={22} />} title={status.running ? "还没有请求" : "网关已暂停"}>
              把客户端的地址设为 <code>{baseUrl}</code> 就能看到请求了
              <span style={{ marginLeft: 6 }}>
                <CopyButton text={baseUrl} small />
              </span>
            </Empty>
          ) : (
            <div className="recent">
              {recent.map((r) => (
                <button key={r.id} className="recent-row" onClick={() => go({ name: "logs" })}>
                  <StatusDot tone={r.pending ? "busy" : statusTone(r.status)} />
                  <span className="recent-time">{fmtTime(r.ts)}</span>
                  <span className="recent-model">
                    <code>{r.requestedModel}</code>
                    {r.gatewayModel && r.gatewayModel !== r.requestedModel && (
                      <>
                        <ArrowRight size={12} />
                        <code>{r.gatewayModel}</code>
                      </>
                    )}
                  </span>
                  <span className="recent-client">{r.clientProtocol}</span>
                  <span className="recent-num">{r.pending ? "进行中…" : `${fmtNum(r.usage.input + r.usage.cacheRead)} / ${fmtNum(r.usage.output)}`}</span>
                  <span className="recent-num">{r.pending ? "" : fmtMs(r.durationMs)}</span>
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
