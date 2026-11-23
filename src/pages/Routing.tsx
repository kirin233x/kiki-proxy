import { useEffect, useState } from "react";
import { Plus, Save, Trash2, Wand2 } from "lucide-react";
import { api, Config, PROTOCOLS, RouteHop, RuleConfig, uid } from "../api";
import { useApp } from "../App";
import { SortHandle, useSortable } from "../components/SortableList";
import ModelInput from "../components/ModelInput";
import { Hint, Switch, useToast } from "../components/ui";

export default function Routing() {
  const { config, saveConfig } = useApp();
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
            <h1 className="page-title">路由</h1>
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
            <span>模型名</span>
            <span>转发至</span>
            <span>
              故障切换
              <Hint>在端点编辑页设置备选模型，触发条件在「设置」中配置</Hint>
            </span>
          </div>

          {/* ---------------------------------------------------------------- 规则 */}
          <div className="rt-group">
            <b>
              规则
              <Hint>自上而下优先匹配，拖动调整顺序。适用于客户端固定使用的模型名（如 Claude Code 后台任务的 haiku），支持通配符 * 和 ?</Hint>
            </b>
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
                      <input className="input" value={r.name} placeholder="备注" onChange={(e) => setRule(i, { name: e.target.value })} />
                      {r.client && (
                        <button className="chip-x" title="该规则只对此协议的客户端生效，点击取消限制" onClick={() => setRule(i, { client: null })}>
                          仅 {PROTOCOLS.find((p) => p.value === r.client)?.label ?? r.client} ×
                        </button>
                      )}
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

          {/* ---------------------------------------------------------------- 兜底 */}
          <div className="rt-group">
            <b>
              兜底
              <Hint>不带前缀且未匹配任何规则的模型名（如 Claude Code 默认的 claude-sonnet-4-5）</Hint>
            </b>
          </div>
          <div className="rt-row">
            <div className="rt-cell">
              <span className="rt-model other">其余模型名</span>
            </div>
            <div className="rt-cell">
              <ModelInput
                value={draft.routing.defaultModel}
                onChange={(v) => setDraft({ ...draft, routing: { ...draft.routing, defaultModel: v } })}
                placeholder="未设置"
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
  const [hops, setHops] = useState<RouteHop[] | null>(null);
  const [error, setError] = useState("");
  const { config } = useApp();
  const proxyName = (p: string) => (p === "direct" ? "直连" : p === "system" || !p ? "系统代理" : (config.proxies.find((x) => x.id === p)?.name ?? p));

  const run = async () => {
    if (!model) return;
    setError("");
    try {
      setHops(await api.explainRoute(model, "anthropic"));
    } catch (e) {
      setHops(null);
      setError(String(e));
    }
  };

  return (
    <div className="tester2">
      <Wand2 size={16} />
      <div style={{ flex: 1 }}>
        <ModelInput value={model} onChange={setModel} placeholder="输入模型名测试路由，如 claude-haiku-4-5" />
      </div>
      <button className="btn btn-sm btn-primary" onClick={run} disabled={!model}>
        测试
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
