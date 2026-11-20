import { useEffect, useMemo, useState } from "react";
import { ArrowLeft, KeyRound, LogIn, LogOut, Plus, Puzzle, Save, Trash2, Zap } from "lucide-react";
import {
  api,
  AuthMode,
  CredentialStatus,
  EndpointConfig,
  ModelConfig,
  newEndpoint,
  newModel,
  PROTOCOLS,
  secretKeys,
  TestResult,
} from "../api";
import { useApp } from "../App";
import { ModelChain } from "../components/ModelInput";
import { Field, SecretInput, StatusDot, Switch, useToast } from "../components/ui";
import { fmtMs, protocolColor, slugify } from "../utils";

/** 常见服务的预设，填上去再改 */
const PRESETS: { label: string; ep: Partial<EndpointConfig>; models: string[] }[] = [
  { label: "Anthropic", ep: { id: "anthropic", name: "Anthropic", protocol: "anthropic", baseUrl: "https://api.anthropic.com" }, models: ["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"] },
  { label: "OpenAI", ep: { id: "openai", name: "OpenAI", protocol: "openai_responses", baseUrl: "https://api.openai.com/v1" }, models: ["gpt-5", "gpt-5-codex"] },
  {
    label: "DeepSeek",
    ep: { id: "ds", name: "DeepSeek", protocol: "openai_chat", baseUrl: "https://api.deepseek.com", compat: { sendReasoningContent: true, useMaxCompletionTokens: false, sendReasoningEffort: false, dropSamplingParams: false } },
    models: ["deepseek-chat", "deepseek-reasoner"],
  },
  { label: "DeepSeek（Anthropic 格式）", ep: { id: "dsa", name: "DeepSeek Anthropic", protocol: "anthropic", baseUrl: "https://api.deepseek.com/anthropic" }, models: ["deepseek-chat"] },
  { label: "Gemini", ep: { id: "gm", name: "Gemini", protocol: "gemini", baseUrl: "https://generativelanguage.googleapis.com" }, models: ["gemini-2.5-pro", "gemini-2.5-flash"] },
  { label: "OpenRouter", ep: { id: "or", name: "OpenRouter", protocol: "openai_chat", baseUrl: "https://openrouter.ai/api/v1" }, models: [] },
];

const AUTH_OPTIONS: { value: AuthMode; label: string }[] = [
  { value: "auto", label: "按协议自动" },
  { value: "bearer", label: "Authorization: Bearer" },
  { value: "x_api_key", label: "x-api-key" },
  { value: "goog_api_key", label: "x-goog-api-key" },
  { value: "none", label: "不带密钥" },
];

export default function EndpointEditor({ id }: { id: string | null }) {
  const { config, saveConfig, go, plugins } = useApp();
  const toast = useToast();
  const original = id ? config.endpoints.find((e) => e.id === id) : undefined;
  const isNew = !original;
  const [ep, setEp] = useState<EndpointConfig>(() => (original ? structuredClone(original) : newEndpoint()));
  const [idTouched, setIdTouched] = useState(!isNew);
  // 密钥只写不读：这里存的是用户新输入的值
  const [apiKey, setApiKey] = useState("");
  const [pluginSecrets, setPluginSecrets] = useState<Record<string, string>>({});
  const [flags, setFlags] = useState<Record<string, boolean>>({});
  const [headersText, setHeadersText] = useState(() =>
    Object.entries(ep.headers)
      .map(([k, v]) => `${k}: ${v}`)
      .join("\n"),
  );
  const [saving, setSaving] = useState(false);
  const [test, setTest] = useState<TestResult | "busy" | null>(null);
  const [cred, setCred] = useState<CredentialStatus | null>(null);
  const [loggingIn, setLoggingIn] = useState(false);

  const plugin = plugins.find((p) => p.id === ep.plugin);
  const set = <K extends keyof EndpointConfig>(k: K, v: EndpointConfig[K]) => setEp((e) => ({ ...e, [k]: v }));

  const secretNames = useMemo(
    () => [secretKeys.apiKey(ep.id), ...(plugin?.settings.filter((s) => s.kind === "secret").map((s) => secretKeys.pluginSecret(ep.id, s.key)) ?? [])],
    [ep.id, plugin],
  );
  useEffect(() => {
    if (ep.id) api.secretFlags(secretNames).then(setFlags);
  }, [secretNames, ep.id]);
  useEffect(() => {
    if (!isNew && plugin?.usesLogin) api.credentialStatus(ep.id).then(setCred);
  }, [isNew, plugin, ep.id]);

  const applyPreset = (p: (typeof PRESETS)[number]) => {
    let pid = p.ep.id!;
    let n = 2;
    while (config.endpoints.some((x) => x.id === pid)) pid = `${p.ep.id}${n++}`;
    setEp({ ...newEndpoint(), ...p.ep, id: pid, models: p.models.map((m) => ({ ...newModel(), name: m })) } as EndpointConfig);
    setIdTouched(true);
  };

  const parseHeaders = (): Record<string, string> => {
    const out: Record<string, string> = {};
    for (const line of headersText.split("\n")) {
      const i = line.indexOf(":");
      if (i > 0) out[line.slice(0, i).trim()] = line.slice(i + 1).trim();
    }
    return out;
  };

  const save = async (stay = false) => {
    const next = { ...ep, headers: parseHeaders(), models: ep.models.filter((m) => m.name.trim()) };
    if (!next.id) return toast("请填写端点前缀", true);
    if (!next.name) return toast("请填写名称", true);
    if (!next.baseUrl) return toast("请填写地址", true);
    if (isNew && config.endpoints.some((x) => x.id === next.id)) return toast(`前缀「${next.id}」已经被占用了`, true);
    setSaving(true);
    try {
      // 改了前缀的话，把引用旧前缀的故障切换和规则一起改掉
      const renamed = original && original.id !== next.id;
      const rename = (m: string) => (renamed && m.startsWith(original!.id + "/") ? next.id + m.slice(original!.id.length) : m);
      const endpoints = isNew
        ? [...config.endpoints, next]
        : config.endpoints.map((e) => (e.id === original!.id ? next : { ...e, fallbacks: e.fallbacks.map(rename), models: e.models.map((m) => ({ ...m, fallbacks: m.fallbacks.map(rename) })) }));
      await saveConfig({
        ...config,
        endpoints,
        rules: config.rules.map((r) => ({ ...r, target: rename(r.target) })),
        routing: { ...config.routing, defaultModel: rename(config.routing.defaultModel) },
      });
      if (apiKey) await api.setSecret(secretKeys.apiKey(next.id), apiKey);
      for (const [k, v] of Object.entries(pluginSecrets)) if (v) await api.setSecret(secretKeys.pluginSecret(next.id, k), v);
      setApiKey("");
      setPluginSecrets({});
      api.secretFlags(secretNames).then(setFlags);
      toast(isNew ? `已添加「${next.name}」` : "已保存");
      if (isNew || renamed) go({ name: "endpoint", id: next.id });
      else if (!stay) go({ name: "endpoints" });
    } catch {
      /* saveConfig 已经提示过 */
    } finally {
      setSaving(false);
    }
  };

  const runTest = async () => {
    await save(true);
    setTest("busy");
    setTest(await api.testEndpoint(ep.id));
  };

  const login = async () => {
    setLoggingIn(true);
    try {
      setCred(await api.loginEndpoint(ep.id));
      toast("登录成功");
    } catch (e) {
      toast(String(e), true);
    } finally {
      setLoggingIn(false);
    }
  };

  const updateModel = (i: number, patch: Partial<ModelConfig>) => set("models", ep.models.map((m, j) => (j === i ? { ...m, ...patch } : m)));

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 940 }}>
        <div className="page-header">
          <div>
            <button className="back-link" onClick={() => go({ name: "endpoints" })}>
              <ArrowLeft size={15} />
              端点
            </button>
            <h1 className="page-title">{isNew ? "添加端点" : ep.name || "未命名端点"}</h1>
          </div>
          <div className="header-actions">
            {!isNew && (
              <button className="btn" onClick={runTest} disabled={test === "busy"}>
                <Zap size={16} />
                {test === "busy" ? "测试中…" : "保存并测试"}
              </button>
            )}
            <button className="btn btn-primary" onClick={() => save()} disabled={saving}>
              <Save size={16} />
              {isNew ? "添加" : "保存"}
            </button>
          </div>
        </div>

        {test && test !== "busy" && (
          <div className={`banner ${test.ok ? "ok" : "err"}`}>
            <StatusDot tone={test.ok ? "ok" : "err"} />
            <b>{test.ok ? `可用 · ${fmtMs(test.latencyMs)}` : `失败${test.status ? `（HTTP ${test.status}）` : ""}`}</b>
            <span>{test.message}</span>
          </div>
        )}

        {isNew && (
          <div className="presets">
            <span>从预设开始：</span>
            {PRESETS.map((p) => (
              <button key={p.label} className="chip" onClick={() => applyPreset(p)}>
                <i style={{ background: protocolColor[p.ep.protocol!] }} />
                {p.label}
              </button>
            ))}
          </div>
        )}

        {/* ---------------------------------------------------------------- 基本信息 */}
        <div className="settings-group">
          <h3>基本信息</h3>
          <div className="panel panel-pad form-grid">
            <Field label="名称" required>
              <input
                className="input"
                value={ep.name}
                placeholder="例如：内部模型服务"
                onChange={(e) => {
                  const name = e.target.value;
                  setEp((x) => ({ ...x, name, id: idTouched ? x.id : slugify(name) }));
                }}
              />
            </Field>
            <Field label="前缀" required help={<>客户端写 <code>{ep.id || "前缀"}/模型名</code> 来使用这个端点</>}>
              <input
                className="input mono"
                value={ep.id}
                placeholder="internal"
                onChange={(e) => {
                  setIdTouched(true);
                  set("id", e.target.value.replace(/[\s/@]/g, ""));
                }}
              />
            </Field>
            <div className="span-2">
              <Field label="协议">
                <div className="proto-pick">
                  {PROTOCOLS.map((p) => (
                    <button key={p.value} className={ep.protocol === p.value ? "on" : ""} onClick={() => set("protocol", p.value)}>
                      <i style={{ background: protocolColor[p.value] }} />
                      <b>{p.label}</b>
                      <span>{p.hint}</span>
                    </button>
                  ))}
                </div>
              </Field>
            </div>
            <div className="span-2">
              <Field label="地址" required help="带不带 /v1 都可以，例如 https://api.deepseek.com 或 https://api.openai.com/v1">
                <input className="input mono" value={ep.baseUrl} placeholder="https://" onChange={(e) => set("baseUrl", e.target.value.trim())} />
              </Field>
            </div>
            <Field label="API Key" help="保存在 macOS 钥匙串里，不会写进配置文件">
              <SecretInput
                isSet={!!flags[secretKeys.apiKey(ep.id)]}
                value={apiKey}
                onChange={setApiKey}
                placeholder={ep.plugin ? "由插件负责鉴权时可留空" : "sk-..."}
                onClear={async () => {
                  await api.setSecret(secretKeys.apiKey(ep.id), "");
                  api.secretFlags(secretNames).then(setFlags);
                }}
              />
            </Field>
            <Field label="密钥放在">
              <select className="select" value={ep.auth} onChange={(e) => set("auth", e.target.value as AuthMode)}>
                {AUTH_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="出口" help="访问这个端点走哪条线路，可在「出口代理」里添加">
              <select className="select" value={ep.proxy} onChange={(e) => set("proxy", e.target.value)}>
                <option value="system">跟随系统代理</option>
                <option value="direct">直连（不走任何代理）</option>
                {config.proxies.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}（{p.kind === "socks5" ? "SOCKS5" : "HTTP"}）
                  </option>
                ))}
              </select>
            </Field>
            <Field label="启用">
              <div style={{ height: 38, display: "flex", alignItems: "center" }}>
                <Switch on={ep.enabled} onChange={(v) => set("enabled", v)} />
              </div>
            </Field>
          </div>
        </div>

        {/* ---------------------------------------------------------------- 模型 */}
        <div className="settings-group">
          <h3>
            模型
            <span className="h3-hint">没列出的模型名也可以用「{ep.id || "前缀"}/模型名」直接透传</span>
          </h3>
          <div className="panel">
            <table className="grid-table">
              <thead>
                <tr>
                  <th style={{ width: "22%" }}>网关里的名字</th>
                  <th style={{ width: "24%" }}>上游真实模型</th>
                  <th>显示名</th>
                  <th style={{ width: 64 }}>图片</th>
                  <th style={{ width: 150 }}>价格 $/M（入 / 出）</th>
                  <th style={{ width: 64 }}>启用</th>
                  <th style={{ width: 40 }} />
                </tr>
              </thead>
              <tbody>
                {ep.models.map((m, i) => (
                  <tr key={i}>
                    <td>
                      <input className="input mono" value={m.name} placeholder="sonnet" onChange={(e) => updateModel(i, { name: e.target.value.trim() })} />
                    </td>
                    <td>
                      <input className="input mono" value={m.upstream} placeholder={m.name || "同左"} onChange={(e) => updateModel(i, { upstream: e.target.value.trim() })} />
                    </td>
                    <td>
                      <input className="input" value={m.displayName} placeholder={`${ep.id}/${m.name}`} onChange={(e) => updateModel(i, { displayName: e.target.value })} />
                    </td>
                    <td>
                      <Switch on={m.vision} onChange={(v) => updateModel(i, { vision: v })} />
                    </td>
                    <td>
                      <div className="price-pair">
                        <input
                          className="input"
                          type="number"
                          min={0}
                          step="0.01"
                          value={m.price.input || ""}
                          placeholder="0"
                          onChange={(e) => updateModel(i, { price: { ...m.price, input: Number(e.target.value), cacheRead: Number(e.target.value) * 0.1 } })}
                        />
                        <input
                          className="input"
                          type="number"
                          min={0}
                          step="0.01"
                          value={m.price.output || ""}
                          placeholder="0"
                          onChange={(e) => updateModel(i, { price: { ...m.price, output: Number(e.target.value) } })}
                        />
                      </div>
                    </td>
                    <td>
                      <Switch on={m.enabled} onChange={(v) => updateModel(i, { enabled: v })} />
                    </td>
                    <td>
                      <button className="icon-btn" onClick={() => set("models", ep.models.filter((_, j) => j !== i))}>
                        <Trash2 size={15} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="table-foot">
              <button className="btn btn-sm btn-soft" onClick={() => set("models", [...ep.models, newModel()])}>
                <Plus size={14} />
                添加模型
              </button>
            </div>
          </div>
        </div>

        {/* ---------------------------------------------------------------- 故障切换 */}
        <div className="settings-group">
          <h3>
            故障切换
            <span className="h3-hint">这个端点出错时，按顺序改用下面的模型（触发条件在「路由与切换」里设置）</span>
          </h3>
          <div className="panel panel-pad">
            <ModelChain value={ep.fallbacks} onChange={(v) => set("fallbacks", v)} exclude={ep.models.map((m) => `${ep.id}/${m.name}`)} />
            {ep.models.length > 0 && (
              <details className="per-model">
                <summary>为单个模型单独设置（优先于端点设置）</summary>
                {ep.models.map((m, i) => (
                  <div key={i} className="per-model-row">
                    <code>
                      {ep.id}/{m.name}
                    </code>
                    <ModelChain value={m.fallbacks} onChange={(v) => updateModel(i, { fallbacks: v })} exclude={[`${ep.id}/${m.name}`]} />
                  </div>
                ))}
              </details>
            )}
          </div>
        </div>

        {/* ---------------------------------------------------------------- 插件 */}
        <div className="settings-group">
          <h3>
            插件
            <span className="h3-hint">特殊鉴权、登录、改写请求和响应，在 src-tauri/core/src/plugins/ 里编写</span>
          </h3>
          <div className="panel panel-pad">
            <Field label="使用插件">
              <select className="select" value={ep.plugin} onChange={(e) => set("plugin", e.target.value)}>
                <option value="">不使用</option>
                {plugins.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
                {ep.plugin && !plugin && <option value={ep.plugin}>{ep.plugin}（未编译进当前版本）</option>}
              </select>
            </Field>
            {plugin && (
              <>
                {plugin.description && (
                  <div className="plugin-desc">
                    <Puzzle size={15} />
                    {plugin.description}
                  </div>
                )}
                <div className="form-grid">
                  {plugin.settings.map((f) => (
                    <div key={f.key} className={f.kind === "multiline" ? "span-2" : ""}>
                      <Field label={f.label} help={f.help} required={f.required}>
                        {f.kind === "secret" ? (
                          <SecretInput
                            isSet={!!flags[secretKeys.pluginSecret(ep.id, f.key)]}
                            value={pluginSecrets[f.key] ?? ""}
                            onChange={(v) => setPluginSecrets({ ...pluginSecrets, [f.key]: v })}
                          />
                        ) : f.kind === "multiline" ? (
                          <textarea
                            className="textarea mono"
                            rows={4}
                            value={ep.pluginSettings[f.key] ?? f.default}
                            onChange={(e) => set("pluginSettings", { ...ep.pluginSettings, [f.key]: e.target.value })}
                          />
                        ) : (
                          <input
                            className={`input ${f.kind === "url" ? "mono" : ""}`}
                            value={ep.pluginSettings[f.key] ?? f.default}
                            placeholder={f.default}
                            onChange={(e) => set("pluginSettings", { ...ep.pluginSettings, [f.key]: e.target.value })}
                          />
                        )}
                      </Field>
                    </div>
                  ))}
                </div>
                {plugin.usesLogin && !isNew && (
                  <div className="login-box">
                    <KeyRound size={18} />
                    <div className="login-text">
                      <b>{cred?.loggedIn ? (cred.expired ? "登录已过期" : "已登录") : "未登录"}</b>
                      <span>
                        {cred?.loggedIn && cred.expiresAt
                          ? `凭证 ${new Date(cred.expiresAt * 1000).toLocaleString("zh-CN")} 过期，到期前会自动续期`
                          : "第一次请求时也会自动弹出登录；凭证失效时会自动重新登录并重试"}
                      </span>
                    </div>
                    {cred?.loggedIn && (
                      <button
                        className="btn btn-sm"
                        onClick={async () => {
                          await api.logoutEndpoint(ep.id);
                          setCred(await api.credentialStatus(ep.id));
                        }}
                      >
                        <LogOut size={14} />
                        退出
                      </button>
                    )}
                    <button className="btn btn-sm btn-primary" onClick={login} disabled={loggingIn}>
                      <LogIn size={14} />
                      {loggingIn ? "登录中…" : cred?.loggedIn ? "重新登录" : "登录"}
                    </button>
                  </div>
                )}
              </>
            )}
          </div>
        </div>

        {/* ---------------------------------------------------------------- 高级 */}
        <div className="settings-group">
          <h3>高级</h3>
          <div className="panel">
            {ep.protocol === "openai_chat" && (
              <>
                <div className="setting-row">
                  <div className="setting-text">
                    <b>回传思考内容（reasoning_content）</b>
                    <span>DeepSeek 等在工具调用的多轮对话中需要把思考内容带回去</span>
                  </div>
                  <Switch on={ep.compat.sendReasoningContent} onChange={(v) => set("compat", { ...ep.compat, sendReasoningContent: v })} />
                </div>
                <div className="setting-row">
                  <div className="setting-text">
                    <b>使用 max_completion_tokens</b>
                    <span>OpenAI 的推理模型不接受 max_tokens</span>
                  </div>
                  <Switch on={ep.compat.useMaxCompletionTokens} onChange={(v) => set("compat", { ...ep.compat, useMaxCompletionTokens: v })} />
                </div>
                <div className="setting-row">
                  <div className="setting-text">
                    <b>发送 reasoning_effort</b>
                    <span>把客户端的思考强度转成 low / medium / high</span>
                  </div>
                  <Switch on={ep.compat.sendReasoningEffort} onChange={(v) => set("compat", { ...ep.compat, sendReasoningEffort: v })} />
                </div>
              </>
            )}
            <div className="setting-row">
              <div className="setting-text">
                <b>不发送 temperature / top_p</b>
                <span>部分推理模型传了会报错</span>
              </div>
              <Switch on={ep.compat.dropSamplingParams} onChange={(v) => set("compat", { ...ep.compat, dropSamplingParams: v })} />
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>首字节超时</b>
                <span>等待上游开始响应的最长时间，超时会触发故障切换</span>
              </div>
              <div className="input-suffix">
                <input
                  className="input"
                  type="number"
                  min={5}
                  value={ep.timeoutSecs}
                  onChange={(e) => set("timeoutSecs", Math.max(5, Number(e.target.value)))}
                  style={{ width: 90 }}
                />
                <span>秒</span>
              </div>
            </div>
            <div className="setting-row column">
              <div className="setting-text">
                <b>附加请求头</b>
                <span>每行一个，格式 Name: Value</span>
              </div>
              <textarea className="textarea mono" rows={3} value={headersText} placeholder="X-Team: ai-infra" onChange={(e) => setHeadersText(e.target.value)} />
            </div>
            <div className="setting-row column">
              <div className="setting-text">
                <b>备注</b>
              </div>
              <textarea className="textarea" rows={2} value={ep.note} onChange={(e) => set("note", e.target.value)} />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
