import { useEffect, useState } from "react";
import { Plus, Save, Trash2, Wand2 } from "lucide-react";
import { api, Config, Protocol, PROTOCOLS, RouteHop, RuleConfig, uid } from "../api";
import { useApp } from "../App";
import { SortHandle, useSortable } from "../components/SortableList";
import ModelInput, { ModelChain } from "../components/ModelInput";
import { Switch, useToast } from "../components/ui";

export default function Routing() {
  const { config, saveConfig, go } = useApp();
  const toast = useToast();
  const [draft, setDraft] = useState<Config>(() => structuredClone(config));
  const dirty = JSON.stringify(draft) !== JSON.stringify(config);
  useEffect(() => setDraft(structuredClone(config)), [config]);

  const proxyName = (p: string) => (p === "direct" ? "直连" : p === "system" || !p ? "系统代理" : (draft.proxies.find((x) => x.id === p)?.name ?? p));
  const find = (id: string) => {
    const [prefix, name] = id.split("/");
    const ep = draft.endpoints.find((e) => e.id === prefix);
    return { ep, m: ep?.models.find((m) => m.name === name) };
  };
  /** 一个模型实际的备选链：模型自己设置了就用模型的，否则用端点的 */
  const chainOf = (id: string) => {
    const { ep, m } = find(id);
    if (!ep) return [];
    return m?.fallbacks.length ? m.fallbacks : ep.fallbacks;
  };
  const setChain = (epId: string, name: string, list: string[]) =>
    setDraft((d) => ({
      ...d,
      endpoints: d.endpoints.map((e) => (e.id !== epId ? e : { ...e, models: e.models.map((m) => (m.name === name ? { ...m, fallbacks: list } : m)) })),
    }));
  const setRule = (i: number, patch: Partial<RuleConfig>) => setDraft((d) => ({ ...d, rules: d.rules.map((r, j) => (j === i ? { ...r, ...patch } : r)) }));
  const sortRules = useSortable((from, to) =>
    setDraft((d) => {
      const rules = [...d.rules];
      const [x] = rules.splice(from, 1);
      rules.splice(to, 0, x);
      return { ...d, rules };
    }),
  );

  /** 「发到」那一列：端点名 · 出口 */
  const dest = (target: string, proxyOverride = "") => {
    if (!target) return <span className="muted-sm">未设置</span>;
    const prefix = target.split("/")[0];
    const ep = draft.endpoints.find((e) => e.id === prefix);
    if (!ep) return <span className="dest-bad">找不到端点「{prefix}」</span>;
    const typeName = ep.process.enabled ? "本地程序" : "";
    return (
      <span className="dest">
        <b>{ep.name}</b>
        <span>
          {typeName ? `${typeName} · ` : ""}
          {proxyName(proxyOverride || ep.proxy)}
          {!ep.enabled && " · 已停用"}
        </span>
      </span>
    );
  };
  const chainText = (target: string) => {
    const c = target.endsWith("/*") ? [] : chainOf(target);
    return c.length ? (
      <span className="chain-ro">
        {c.map((x, i) => (
          <code key={x}>
            {i + 1}. {x}
          </code>
        ))}
      </span>
    ) : (
      <span className="muted-sm">—</span>
    );
  };

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 1100 }}>
        <div className="page-header">
          <div>
            <h1 className="page-title">路由与切换</h1>
            <div className="page-subtitle">按模型名查看请求的转发目标与故障切换顺序</div>
          </div>
          <div className="header-actions">
            <button className="btn btn-primary" onClick={() => saveConfig(draft).then(() => toast("已保存"))} disabled={!dirty}>
              <Save size={16} />
              保存
            </button>
          </div>
        </div>

        <RouteTester />

        <div className="panel route-table">
          <div className="rt-row rt-head">
            <span>请求模型</span>
            <span>转发至</span>
            <span>故障切换</span>
          </div>

          {/* ---------------------------------------------------------------- 规则 */}
          <div className="rt-group">
            <b>规则</b>
            <span>优先匹配，自上而下，拖动调整顺序。适用于客户端固定使用的模型名（如 Claude Code 后台任务的 haiku），支持通配符 * 和 ?</span>
          </div>
          <div ref={sortRules.listRef} className={`rt-rules ${sortRules.sorting ? "is-sorting" : ""}`}>
            {draft.rules.map((r, i) => {
              const sr = sortRules.row(i);
              return (
                <div key={r.id} {...sr} className={`rt-row rule ${r.enabled ? "" : "off"} ${sr.className}`}>
                  <SortHandle {...sortRules.handle(i)} />
                  <div className="rt-cell">
                    <div className="rule-line">
                      <Switch on={r.enabled} onChange={(v) => setRule(i, { enabled: v })} />
                      <input className="input mono" value={r.pattern} placeholder="claude-*haiku*" onChange={(e) => setRule(i, { pattern: e.target.value.trim() })} />
                    </div>
                    <div className="rule-line sub">
                      <select className="select" value={r.client ?? ""} onChange={(e) => setRule(i, { client: (e.target.value || null) as Protocol | null })}>
                        <option value="">所有客户端</option>
                        {PROTOCOLS.map((p) => (
                          <option key={p.value} value={p.value}>
                            仅 {p.label}
                          </option>
                        ))}
                      </select>
                      <input className="input" value={r.name} placeholder="备注" onChange={(e) => setRule(i, { name: e.target.value })} />
                    </div>
                  </div>
                  <div className="rt-cell">
                    <ModelInput value={r.target} onChange={(v) => setRule(i, { target: v })} placeholder="目标模型" extra={draft.endpoints.map((e) => `${e.id}/*`)} />
                    <div className="rule-line sub">
                      {dest(r.target, r.proxy)}
                      <select className="select mini" value={r.proxy} onChange={(e) => setRule(i, { proxy: e.target.value })} title="网络出口（覆盖端点设置）">
                        <option value="">沿用端点出口</option>
                        <option value="direct">直连</option>
                        <option value="system">系统代理</option>
                        {draft.proxies.map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.name}
                          </option>
                        ))}
                      </select>
                    </div>
                  </div>
                  <div className="rt-cell rt-chain">
                    {chainText(r.target)}
                    <div className="rule-actions">
                      <button className="icon-btn" onClick={() => setDraft((d) => ({ ...d, rules: d.rules.filter((_, j) => j !== i) }))} title="删除">
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
          <div className="rt-add">
            <button
              className="chain-add"
              onClick={() =>
                setDraft((d) => ({ ...d, rules: [...d.rules, { id: uid("r"), name: "", enabled: true, pattern: "", client: null, target: "", proxy: "" }] }))
              }
            >
              <Plus size={14} />
              添加规则
            </button>
          </div>

          {/* ---------------------------------------------------------------- 端点模型 */}
          <div className="rt-group">
            <b>端点模型</b>
            <span>
              「前缀/模型名」直接发到对应端点。备选在这里或端点编辑页设置，切换条件在
              <a onClick={() => go({ name: "settings" })}>设置</a>里
            </span>
          </div>
          {draft.endpoints
            .filter((e) => e.enabled)
            .flatMap((e) =>
              e.models
                .filter((m) => m.enabled)
                .map((m) => {
                  const id = `${e.id}/${m.name}`;
                  const inherited = !m.fallbacks.length && e.fallbacks.length > 0;
                  return (
                    <div key={id} className="rt-row">
                      <div className="rt-cell">
                        <code className="rt-model">{id}</code>
                        {m.displayName && <span className="muted-sm">{m.displayName}</span>}
                      </div>
                      <div className="rt-cell">
                        {dest(id)}
                        {m.upstream && m.upstream !== m.name && <span className="muted-sm">上游模型 {m.upstream}</span>}
                      </div>
                      <div className="rt-cell rt-chain">
                        <ModelChain value={chainOf(id)} onChange={(v) => setChain(e.id, m.name, v)} exclude={[id]} />
                        {inherited && <span className="inherit">沿用端点「{e.name}」的设置；修改后仅对该模型生效</span>}
                      </div>
                    </div>
                  );
                }),
            )}

          {/* ---------------------------------------------------------------- 兜底 */}
          <div className="rt-group">
            <b>兜底</b>
            <span>不带前缀且未匹配任何规则的模型名（如 Claude Code 默认的 claude-sonnet-4-5）</span>
          </div>
          <div className="rt-row">
            <div className="rt-cell">
              <span className="rt-model other">其余模型名</span>
            </div>
            <div className="rt-cell">
              <ModelInput
                value={draft.routing.defaultModel}
                onChange={(v) => setDraft({ ...draft, routing: { ...draft.routing, defaultModel: v } })}
                placeholder="不设置则返回「找不到模型」"
              />
              {draft.routing.defaultModel && <div className="rule-line sub">{dest(draft.routing.defaultModel)}</div>}
            </div>
            <div className="rt-cell rt-chain">{draft.routing.defaultModel ? chainText(draft.routing.defaultModel) : <span className="muted-sm">—</span>}</div>
          </div>
        </div>
      </div>
    </div>
  );
}

function RouteTester() {
  const [model, setModel] = useState("");
  const [client, setClient] = useState<Protocol>("anthropic");
  const [hops, setHops] = useState<RouteHop[] | null>(null);
  const [error, setError] = useState("");
  const { config } = useApp();
  const proxyName = (p: string) => (p === "direct" ? "直连" : p === "system" || !p ? "系统代理" : (config.proxies.find((x) => x.id === p)?.name ?? p));

  const run = async () => {
    if (!model) return;
    setError("");
    try {
      setHops(await api.explainRoute(model, client));
    } catch (e) {
      setHops(null);
      setError(String(e));
    }
  };

  return (
    <div className="tester2">
      <Wand2 size={16} />
      <span>路由测试：</span>
      <select className="select" value={client} onChange={(e) => setClient(e.target.value as Protocol)}>
        {PROTOCOLS.map((p) => (
          <option key={p.value} value={p.value}>
            {p.label} 客户端
          </option>
        ))}
      </select>
      <span>发来</span>
      <div style={{ flex: 1 }}>
        <ModelInput value={model} onChange={setModel} placeholder="输入模型名，如 claude-haiku-4-5" />
      </div>
      <button className="btn btn-sm btn-primary" onClick={run} disabled={!model}>
        解析
      </button>
      {(hops || error) && (
        <div className="tester2-result">
          {error ? (
            <span className="dest-bad">{error}</span>
          ) : (
            hops!.map((h, i) => (
              <span key={i} className="hop2">
                {i > 0 && <em>失败时 →</em>}
                <b>{h.endpointName}</b>
                <code>{h.upstreamModel}</code>
                <span>经{proxyName(h.proxy)}</span>
                {i === 0 && h.rule && <span className="tag accent">{h.rule}</span>}
              </span>
            ))
          )}
        </div>
      )}
    </div>
  );
}
