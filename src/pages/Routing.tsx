import { useEffect, useState } from "react";
import { ArrowDown, ArrowRight, ArrowUp, Plus, Route as RouteIcon, Save, Trash2, Wand2 } from "lucide-react";
import { api, Config, Protocol, PROTOCOLS, protocolLabel, RouteHop, RuleConfig, uid } from "../api";
import { useApp } from "../App";
import ModelInput from "../components/ModelInput";
import { Empty, Field, Switch, useToast } from "../components/ui";
import { protocolColor } from "../utils";

export default function Routing() {
  const { config, saveConfig } = useApp();
  const toast = useToast();
  const [draft, setDraft] = useState<Config>(() => structuredClone(config));
  const dirty = JSON.stringify(draft) !== JSON.stringify(config);
  useEffect(() => setDraft(structuredClone(config)), [config]);

  const setRule = (i: number, patch: Partial<RuleConfig>) => setDraft((d) => ({ ...d, rules: d.rules.map((r, j) => (j === i ? { ...r, ...patch } : r)) }));
  const move = (i: number, dir: -1 | 1) =>
    setDraft((d) => {
      const rules = [...d.rules];
      const j = i + dir;
      if (j < 0 || j >= rules.length) return d;
      [rules[i], rules[j]] = [rules[j], rules[i]];
      return { ...d, rules };
    });

  const save = () => saveConfig(draft).then(() => toast("已保存"));

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 980 }}>
        <div className="page-header">
          <div>
            <h1 className="page-title">路由与切换</h1>
            <div className="page-subtitle">决定每个请求发到哪个端点、走哪条线路、失败时换到哪里</div>
          </div>
          <div className="header-actions">
            <button className="btn btn-primary" onClick={save} disabled={!dirty}>
              <Save size={16} />
              保存
            </button>
          </div>
        </div>

        <div className="flow">
          <div className="flow-step">
            <b>1</b>
            <span>
              按顺序匹配<strong>规则</strong>
            </span>
          </div>
          <ArrowRight size={16} />
          <div className="flow-step">
            <b>2</b>
            <span>
              模型名带<strong>端点前缀</strong>就直接用
            </span>
          </div>
          <ArrowRight size={16} />
          <div className="flow-step">
            <b>3</b>
            <span>
              都不匹配用<strong>默认模型</strong>
            </span>
          </div>
          <ArrowRight size={16} />
          <div className="flow-step">
            <b>4</b>
            <span>
              失败时按<strong>备选模型</strong>切换
            </span>
          </div>
        </div>

        <RouteTester />

        {/* ---------------------------------------------------------------- 规则 */}
        <div className="settings-group">
          <h3>
            规则
            <span className="h3-hint">适合处理客户端写死的模型名，比如 Claude Code 后台任务用的 haiku</span>
          </h3>
          <div className="panel">
            {draft.rules.length === 0 ? (
              <Empty icon={<RouteIcon size={22} />} title="没有规则">
                不加规则也能用：客户端直接写「前缀/模型名」即可
              </Empty>
            ) : (
              <div className="rules">
                <div className="rule-row rule-head">
                  <span />
                  <span>匹配模型名</span>
                  <span>客户端</span>
                  <span />
                  <span>改用模型</span>
                  <span>出口</span>
                  <span />
                </div>
                {draft.rules.map((r, i) => (
                  <div key={r.id} className={`rule-row ${r.enabled ? "" : "off"}`}>
                    <Switch on={r.enabled} onChange={(v) => setRule(i, { enabled: v })} />
                    <div>
                      <input className="input mono" value={r.pattern} placeholder="claude-*haiku*" onChange={(e) => setRule(i, { pattern: e.target.value.trim() })} />
                      <input className="input rule-name" value={r.name} placeholder="规则说明（可选）" onChange={(e) => setRule(i, { name: e.target.value })} />
                    </div>
                    <select className="select" value={r.client ?? ""} onChange={(e) => setRule(i, { client: (e.target.value || null) as Protocol | null })}>
                      <option value="">全部</option>
                      {PROTOCOLS.map((p) => (
                        <option key={p.value} value={p.value}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                    <ArrowRight size={16} className="rule-arrow" />
                    <ModelInput value={r.target} onChange={(v) => setRule(i, { target: v })} placeholder="不改模型" extra={draft.endpoints.map((e) => `${e.id}/*`)} />
                    <select className="select" value={r.proxy} onChange={(e) => setRule(i, { proxy: e.target.value })}>
                      <option value="">端点默认</option>
                      <option value="direct">直连</option>
                      <option value="system">系统代理</option>
                      {draft.proxies.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.name}
                        </option>
                      ))}
                    </select>
                    <div className="rule-actions">
                      <button className="icon-btn" disabled={i === 0} onClick={() => move(i, -1)}>
                        <ArrowUp size={14} />
                      </button>
                      <button className="icon-btn" disabled={i === draft.rules.length - 1} onClick={() => move(i, 1)}>
                        <ArrowDown size={14} />
                      </button>
                      <button className="icon-btn" onClick={() => setDraft((d) => ({ ...d, rules: d.rules.filter((_, j) => j !== i) }))}>
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
            <div className="table-foot">
              <button
                className="btn btn-sm btn-soft"
                onClick={() =>
                  setDraft((d) => ({ ...d, rules: [...d.rules, { id: uid("r"), name: "", enabled: true, pattern: "", client: null, target: "", proxy: "" }] }))
                }
              >
                <Plus size={14} />
                添加规则
              </button>
              <span className="muted-sm">
                通配符：<code>*</code> 任意字符，<code>?</code> 单个字符；目标写 <code>ds/*</code> 表示换到 ds 端点、模型名不变
              </span>
            </div>
          </div>
        </div>

        {/* ---------------------------------------------------------------- 默认 */}
        <div className="settings-group">
          <h3>默认模型</h3>
          <div className="panel">
            <div className="setting-row">
              <div className="setting-text">
                <b>兜底模型</b>
                <span>模型名既没有前缀也不匹配任何规则时使用（例如 Claude Code 默认发的 claude-sonnet-4-5）</span>
              </div>
              <div style={{ width: 300 }}>
                <ModelInput value={draft.routing.defaultModel} onChange={(v) => setDraft({ ...draft, routing: { ...draft.routing, defaultModel: v } })} placeholder="不设置则返回 404" />
              </div>
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>让 Claude Code 的 /model 列出所有模型</b>
                <span>
                  Claude Code 只显示名字含 claude 的模型，开启后其他模型会以 <code>claude@前缀/模型</code> 的名字出现，网关会自动去掉这个前缀
                </span>
              </div>
              <Switch on={draft.routing.claudeAlias} onChange={(v) => setDraft({ ...draft, routing: { ...draft.routing, claudeAlias: v } })} />
            </div>
          </div>
        </div>

        {/* ---------------------------------------------------------------- 故障切换 */}
        <div className="settings-group">
          <h3>
            故障切换的触发条件
            <span className="h3-hint">只在上游还没开始返回内容时切换，已经输出一半的流不会被打断重来</span>
          </h3>
          <div className="panel">
            {(
              [
                ["onConnectError", "连接失败", "DNS 解析失败、连接被拒、代理不可用、TLS 错误、插件登录失败"],
                ["onTimeout", "首字节超时", "超过端点设置的等待时间仍没有响应"],
                ["on5xx", "上游 5xx", "服务端错误、过载（500 / 502 / 503 / 529 等）"],
                ["on429", "上游 429", "被限流"],
              ] as const
            ).map(([k, title, desc]) => (
              <div key={k} className="setting-row">
                <div className="setting-text">
                  <b>{title}</b>
                  <span>{desc}</span>
                </div>
                <Switch on={draft.failover[k]} onChange={(v) => setDraft({ ...draft, failover: { ...draft.failover, [k]: v } })} />
              </div>
            ))}
            <div className="setting-row">
              <div className="setting-text">
                <b>其他状态码</b>
                <span>额外触发切换的状态码，逗号分隔，例如 401, 403</span>
              </div>
              <input
                className="input mono"
                style={{ width: 200 }}
                defaultValue={draft.failover.extraStatus.join(", ")}
                placeholder="例如 403"
                onBlur={(e) =>
                  setDraft({
                    ...draft,
                    failover: {
                      ...draft.failover,
                      extraStatus: e.target.value
                        .split(/[,，\s]+/)
                        .map(Number)
                        .filter((n) => n >= 100 && n < 600),
                    },
                  })
                }
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function RouteTester() {
  const [model, setModel] = useState("claude-haiku-4-5");
  const [client, setClient] = useState<Protocol>("anthropic");
  const [hops, setHops] = useState<RouteHop[] | null>(null);
  const [error, setError] = useState("");
  const { config } = useApp();

  const run = async () => {
    setError("");
    try {
      setHops(await api.explainRoute(model, client));
    } catch (e) {
      setHops(null);
      setError(String(e));
    }
  };
  useEffect(() => {
    run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config]);

  const proxyName = (p: string) => (p === "direct" ? "直连" : p === "system" || !p ? "系统代理" : config.proxies.find((x) => x.id === p)?.name ?? p);

  return (
    <div className="panel panel-pad tester">
      <div className="panel-title">
        <Wand2 size={16} />
        路由测试
      </div>
      <div className="panel-sub">输入一个模型名，看看它会被发到哪里（已保存的配置）</div>
      <div className="tester-form">
        <Field label="客户端">
          <select className="select" value={client} onChange={(e) => setClient(e.target.value as Protocol)}>
            {PROTOCOLS.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="请求的模型名">
          <ModelInput value={model} onChange={setModel} placeholder="claude-sonnet-4-5" />
        </Field>
        <button className="btn btn-primary" onClick={run}>
          测试
        </button>
      </div>
      {error && <div className="banner err">{error}</div>}
      {hops && (
        <div className="hops">
          {hops.map((h, i) => (
            <div key={i} className="hop">
              <span className="hop-n">{i === 0 ? "首选" : `备选 ${i}`}</span>
              <span className="proto-tag" style={{ ["--c" as string]: protocolColor[h.protocol], ["--cb" as string]: protocolColor[h.protocol] + "1f" }}>
                {protocolLabel(h.protocol)}
              </span>
              <b>{h.endpointName}</b>
              <code>{h.upstreamModel}</code>
              <span className="muted-sm">经 {proxyName(h.proxy)}</span>
              {h.rule && i === 0 && <span className="tag accent">{h.rule}</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
