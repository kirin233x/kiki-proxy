import { useEffect, useState } from "react";
import { AlertTriangle, ArrowDownToLine, ArrowUpFromLine, Coins, Send } from "lucide-react";
import { api, Stats, UsageRow } from "../api";
import { useApp } from "../App";
import { fmtCost, fmtMs, fmtNum } from "../utils";

const RANGES = [
  [1, "今天"],
  [7, "7 天"],
  [30, "30 天"],
  [90, "90 天"],
] as const;

type Metric = "requests" | "tokens" | "cost";

export default function StatsPage() {
  const { config } = useApp();
  const [days, setDays] = useState<number>(7);
  const [stats, setStats] = useState<Stats | null>(null);
  const [metric, setMetric] = useState<Metric>("tokens");

  useEffect(() => {
    api.getStats(days).then(setStats);
  }, [days]);

  const t = stats?.total;
  const daily = stats?.daily ?? [];
  const val = (d: (typeof daily)[number]) => (metric === "requests" ? d.requests : metric === "cost" ? d.cost : d.input + d.output);
  const max = Math.max(1, ...daily.map(val));

  return (
    <div className="page">
      <div className="page-inner">
        <div className="page-header">
          <div>
            <h1 className="page-title">用量统计</h1>
            <div className="page-subtitle">费用按端点里填写的模型价格估算；没填价格的模型不计费</div>
          </div>
          <div className="segmented">
            {RANGES.map(([d, l]) => (
              <button key={d} className={days === d ? "on" : ""} onClick={() => setDays(d)}>
                {l}
              </button>
            ))}
          </div>
        </div>

        <div className="kpi-grid">
          <Kpi icon={<Send size={17} />} color="#6d5bff" num={fmtNum(t?.requests ?? 0)} label="请求" />
          <Kpi icon={<ArrowDownToLine size={17} />} color="#0ea5e9" num={fmtNum((t?.input ?? 0) + (t?.cacheRead ?? 0) + (t?.cacheWrite ?? 0))} label={`输入 token${t?.cacheRead ? ` · 缓存命中 ${pct(t.cacheRead, t.input + t.cacheRead + t.cacheWrite)}` : ""}`} />
          <Kpi icon={<ArrowUpFromLine size={17} />} color="#10b981" num={fmtNum(t?.output ?? 0)} label="输出 token" />
          <Kpi icon={<Coins size={17} />} color="#f59e0b" num={fmtCost(t?.cost ?? 0)} label="估算费用" />
          <Kpi icon={<AlertTriangle size={17} />} color="#ef4444" num={t && t.requests ? pct(t.errors, t.requests) : "0%"} label={`失败 ${t?.errors ?? 0} 次`} />
        </div>

        {days > 1 && (
          <div className="panel panel-pad">
            <div className="chart-head">
              <div className="panel-title">每日趋势</div>
              <div className="segmented small">
                {(
                  [
                    ["tokens", "Token"],
                    ["requests", "请求"],
                    ["cost", "费用"],
                  ] as const
                ).map(([k, l]) => (
                  <button key={k} className={metric === k ? "on" : ""} onClick={() => setMetric(k)}>
                    {l}
                  </button>
                ))}
              </div>
            </div>
            <div className="daily-chart">
              {daily.map((d) => (
                <div key={d.day} className="daily-col" title={`${d.day}\n请求 ${d.requests}（失败 ${d.errors}）\n输入 ${fmtNum(d.input)} · 输出 ${fmtNum(d.output)}\n${fmtCost(d.cost)}`}>
                  <span className="daily-val">{metric === "cost" ? fmtCost(val(d)) : fmtNum(val(d))}</span>
                  <div className="daily-bar">
                    {metric === "tokens" ? (
                      <>
                        <i className="in" style={{ height: `${(d.input / max) * 100}%` }} />
                        <i className="out" style={{ height: `${(d.output / max) * 100}%` }} />
                      </>
                    ) : (
                      <i className="one" style={{ height: `${(val(d) / max) * 100}%` }} />
                    )}
                  </div>
                  <span className="daily-day">{d.day.slice(5).replace("-", "/")}</span>
                </div>
              ))}
              {daily.length === 0 && <div className="muted-sm">这段时间没有请求</div>}
            </div>
            {metric === "tokens" && (
              <div className="legend-inline">
                <span>
                  <i className="in" />
                  输入
                </span>
                <span>
                  <i className="out" />
                  输出
                </span>
              </div>
            )}
          </div>
        )}

        <div className="stats-grid two">
          <UsageTable title="按模型" rows={stats?.byModel ?? []} total={t} mono />
          <UsageTable
            title="按端点"
            rows={(stats?.byEndpoint ?? []).map((r) => ({ ...r, key: config.endpoints.find((e) => e.id === r.key)?.name ?? r.label ?? r.key }))}
            total={t}
          />
        </div>
      </div>
    </div>
  );
}

const pct = (a: number, b: number) => (b ? `${((a / b) * 100).toFixed(a / b < 0.1 ? 1 : 0)}%` : "0%");

function Kpi(props: { icon: React.ReactNode; color: string; num: string; label: string }) {
  return (
    <div className="kpi">
      <div className="kpi-icon" style={{ background: props.color + "24", color: props.color }}>
        {props.icon}
      </div>
      <div className="kpi-num">{props.num}</div>
      <div className="kpi-label">{props.label}</div>
    </div>
  );
}

function UsageTable(props: { title: string; rows: UsageRow[]; total?: UsageRow; mono?: boolean }) {
  const max = Math.max(1, ...props.rows.map((r) => r.input + r.cacheRead + r.output));
  return (
    <div className="panel panel-pad">
      <div className="panel-title">{props.title}</div>
      <div className="usage-table">
        <div className="usage-row head">
          <span>名称</span>
          <span>请求</span>
          <span>Token</span>
          <span>平均耗时</span>
          <span>费用</span>
        </div>
        {props.rows.map((r) => {
          const tokens = r.input + r.cacheRead + r.cacheWrite + r.output;
          return (
            <div key={r.key} className="usage-row">
              <span className={props.mono ? "mono" : ""}>
                {r.key}
                <i className="usage-bar" style={{ width: `${(tokens / max) * 100}%` }} />
              </span>
              <span>
                {fmtNum(r.requests)}
                {r.errors > 0 && <em className="err-num"> · {r.errors} 失败</em>}
              </span>
              <span>{fmtNum(tokens)}</span>
              <span>{fmtMs(r.avgMs)}</span>
              <span>{fmtCost(r.cost)}</span>
            </div>
          );
        })}
        {props.rows.length === 0 && <div className="muted-sm" style={{ padding: "16px 0" }}>没有数据</div>}
      </div>
    </div>
  );
}
