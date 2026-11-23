import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, CloudDownload, Plus, Save, Sparkles, TerminalSquare, Trash2, Zap } from "lucide-react";
import { api, applyModelInfo, AuthMode, EndpointConfig, ModelConfig, newEndpoint, newModel, PROTOCOLS, secretKeys } from "../api";
import { useApp } from "../App";
import { ModelChain } from "../components/ModelInput";
import { Field, SecretInput, Switch, useToast } from "../components/ui";
import ContextSelect from "../components/ContextSelect";
import UpstreamModelsDialog from "../components/UpstreamModelsDialog";
import TestDialog from "../components/TestDialog";
import ProcessPanel from "../components/ProcessPanel";
import { protocolColor, slugify } from "../utils";

/** 常见服务的预设，填上去再改 */
const PRESETS: { label: string; ep: Partial<EndpointConfig>; models: string[] }[] = [
  {
    label: "本地命令行服务（如 cc-proxy）",
    ep: {
      id: "internal",
      name: "内部服务",
      protocol: "anthropic",
      baseUrl: "http://127.0.0.1:8787",
      auth: "none",
      proxy: "direct",
      process: { enabled: true, command: "", workdir: "", env: "", network: "" },
    },
    models: [],
  },
  {
    label: "Anthropic",
    ep: { id: "anthropic", name: "Anthropic", protocol: "anthropic", baseUrl: "https://api.anthropic.com" },
    models: ["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"],
  },
  { label: "OpenAI", ep: { id: "openai", name: "OpenAI", protocol: "openai_responses", baseUrl: "https://api.openai.com/v1" }, models: ["gpt-5", "gpt-5-codex"] },
  {
    label: "DeepSeek",
    ep: {
      id: "ds",
      name: "DeepSeek",
      protocol: "openai_chat",
      baseUrl: "https://api.deepseek.com",
      compat: { sendReasoningContent: true, useMaxCompletionTokens: false, sendReasoningEffort: false, dropSamplingParams: false },
    },
    models: ["deepseek-chat", "deepseek-reasoner"],
  },
  {
    label: "DeepSeek（Anthropic 格式）",
    ep: { id: "dsa", name: "DeepSeek Anthropic", protocol: "anthropic", baseUrl: "https://api.deepseek.com/anthropic" },
    models: ["deepseek-chat"],
  },
  {
    label: "Gemini",
    ep: { id: "gm", name: "Gemini", protocol: "gemini", baseUrl: "https://generativelanguage.googleapis.com" },
    models: ["gemini-2.5-pro", "gemini-2.5-flash"],
  },
  { label: "OpenRouter", ep: { id: "or", name: "OpenRouter", protocol: "openai_chat", baseUrl: "https://openrouter.ai/api/v1" }, models: [] },
];

const AUTH_OPTIONS: { value: AuthMode; label: string }[] = [
  { value: "auto", label: "自动（按接口协议）" },
  { value: "bearer", label: "Bearer Token（Authorization）" },
  { value: "x_api_key", label: "x-api-key 请求头" },
  { value: "goog_api_key", label: "x-goog-api-key 请求头" },
  { value: "none", label: "无需认证" },
];

export default function EndpointEditor({ id }: { id: string | null }) {
  const { config, saveConfig, go, setLeaveGuard } = useApp();
  const toast = useToast();
  const original = id ? config.endpoints.find((e) => e.id === id) : undefined;
  const isNew = !original;
  const [ep, setEp] = useState<EndpointConfig>(() => (original ? structuredClone(original) : newEndpoint()));
  const [idTouched, setIdTouched] = useState(!isNew);
  // 密钥只写不读：这里存的是用户新输入的值
  const [apiKey, setApiKey] = useState("");
  const [flags, setFlags] = useState<Record<string, boolean>>({});
  const [headersText, setHeadersText] = useState(() =>
    Object.entries(ep.headers)
      .map(([k, v]) => `${k}: ${v}`)
      .join("\n"),
  );
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [filling, setFilling] = useState(false);
  const [fetching, setFetching] = useState(false);

  const autoFill = async () => {
    setFilling(true);
    try {
      const infos = await api.lookupModels(
        ep.models.map((m) => m.upstream || m.name),
        ep.baseUrl,
        ep.protocol,
      );
      const hit = infos.filter(Boolean).length;
      setEp((x) => ({ ...x, models: x.models.map((m, i) => (infos[i] ? applyModelInfo(m, infos[i]!) : m)) }));
      const miss = ep.models.filter((_, i) => !infos[i]).map((m) => m.upstream || m.name);
      toast(miss.length ? `已填充 ${hit} 个模型；未收录：${miss.join("、")}` : `已填充 ${hit} 个模型的上下文与图像支持，保存后生效`, !hit);
    } catch (e) {
      toast(String(e), true);
    } finally {
      setFilling(false);
    }
  };

  const set = <K extends keyof EndpointConfig>(k: K, v: EndpointConfig[K]) => setEp((e) => ({ ...e, [k]: v }));

  const secretNames = useMemo(() => [secretKeys.apiKey(ep.id)], [ep.id]);
  useEffect(() => {
    if (ep.id) api.secretFlags(secretNames).then(setFlags);
  }, [secretNames, ep.id]);

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
        : config.endpoints.map((e) =>
            e.id === original!.id ? next : { ...e, fallbacks: e.fallbacks.map(rename), models: e.models.map((m) => ({ ...m, fallbacks: m.fallbacks.map(rename) })) },
          );
      await saveConfig({
        ...config,
        endpoints,
        rules: config.rules.map((r) => ({ ...r, target: rename(r.target) })),
        routing: { ...config.routing, defaultModel: rename(config.routing.defaultModel) },
      });
      if (apiKey) await api.setSecret(secretKeys.apiKey(next.id), apiKey);
      setApiKey("");
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
    setTesting(true);
  };

  // 有没有未保存的修改（和打开时的配置比）
  const headersOf = (e: EndpointConfig) =>
    Object.entries(e.headers)
      .map(([k, v]) => `${k}: ${v}`)
      .join("\n");
  const dirty = isNew
    ? !!(ep.name || ep.baseUrl || ep.models.length)
    : JSON.stringify(ep) !== JSON.stringify(original) || headersText !== headersOf(original!) || !!apiKey;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty && !saving;
  useEffect(() => {
    setLeaveGuard(() => dirtyRef.current);
    return () => setLeaveGuard(null);
  }, [setLeaveGuard]);
  const discard = () => {
    if (original) {
      setEp(structuredClone(original));
      setHeadersText(headersOf(original));
    } else setEp(newEndpoint());
    setApiKey("");
  };

  const updateModel = (i: number, patch: Partial<ModelConfig>) =>
    set(
      "models",
      ep.models.map((m, j) => (j === i ? { ...m, ...patch } : m)),
    );

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 1060 }}>
        <div className="page-header">
          <div>
            <button className="back-link" onClick={() => go({ name: "endpoints" })}>
              <ArrowLeft size={15} />
              端点
            </button>
            <h1 className="page-title">{isNew ? "添加端点" : ep.name || "未命名端点"}</h1>
          </div>
          <div className="header-actions">
            <label className={`head-toggle ${ep.enabled ? "on" : ""}`} title="停用后网关不再向该端点转发请求，也不会启动其本地服务">
              <Switch on={ep.enabled} onChange={(v) => set("enabled", v)} />
              {ep.enabled ? "已启用" : "已停用"}
            </label>
            {!isNew && (
              <button className="btn" onClick={runTest}>
                <Zap size={16} />
                保存并测试
              </button>
            )}
            <button className="btn btn-primary" onClick={() => save()} disabled={saving}>
              <Save size={16} />
              {isNew ? "添加" : "保存"}
            </button>
          </div>
        </div>

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
            <Field
              label="模型前缀"
              required
              help={
                <>
                  客户端通过 <code>{ep.id || "前缀"}/模型 ID</code> 调用该端点
                </>
              }
            >
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
              <Field label="接口协议" help="上游服务的 API 格式。客户端可使用任意协议，由网关自动转换">
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
              <Field
                label="Base URL"
                required
                help={ep.process.enabled ? "本地服务的监听地址，如 http://127.0.0.1:8787" : "可包含或省略 /v1，如 https://api.deepseek.com、https://api.openai.com/v1"}
              >
                <input className="input mono" value={ep.baseUrl} placeholder="https://" onChange={(e) => set("baseUrl", e.target.value.trim())} />
              </Field>
            </div>
            <Field label="API Key" help="存储于 macOS 钥匙串，不写入配置文件">
              <SecretInput
                isSet={!!flags[secretKeys.apiKey(ep.id)]}
                value={apiKey}
                onChange={setApiKey}
                placeholder={ep.process.enabled ? "由本地服务负责认证时留空" : "sk-..."}
                onClear={async () => {
                  await api.setSecret(secretKeys.apiKey(ep.id), "");
                  api.secretFlags(secretNames).then(setFlags);
                }}
              />
            </Field>
            <Field label="认证方式">
              <select className="select" value={ep.auth} onChange={(e) => set("auth", e.target.value as AuthMode)}>
                {AUTH_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </Field>
            <Field
              label="网络出口"
              help={
                /^https?:\/\/(127\.|localhost|\[::1\])/i.test(ep.baseUrl)
                  ? "本机地址始终直连。本地服务自身的网络在「本地服务」中设置"
                  : "网关访问该端点使用的线路，可在「出口代理」中添加"
              }
            >
              <select className="select" value={ep.proxy} onChange={(e) => set("proxy", e.target.value)}>
                <option value="system">系统代理</option>
                <option value="direct">直连</option>
                {config.proxies.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}（{p.kind === "socks5" ? "SOCKS5" : "HTTP"}）
                  </option>
                ))}
              </select>
            </Field>
          </div>
        </div>

        {/* ---------------------------------------------------------------- 本地程序 */}
        <div className="settings-group">
          <h3>
            本地服务
            <span className="h3-hint">托管一个命令行服务进程（如内部服务代理），由其负责认证与协议转换</span>
          </h3>
          <div className="panel">
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  <TerminalSquare size={14} /> 托管本地服务进程
                </b>
                <span>随 App 启动，异常退出后自动重启，App 退出时一并结束</span>
              </div>
              <Switch on={ep.process.enabled} onChange={(v) => set("process", { ...ep.process, enabled: v })} />
            </div>
            {ep.process.enabled && (
              <div className="process-form">
                <Field label="启动命令" required help="在登录 shell 中执行，可使用终端的 PATH；支持多行">
                  <textarea
                    className="textarea mono"
                    rows={2}
                    value={ep.process.command}
                    placeholder="~/bin/cc-proxy --port 8787"
                    onChange={(e) => set("process", { ...ep.process, command: e.target.value })}
                  />
                </Field>
                <Field label="工作目录" help="默认为用户主目录">
                  <input className="input mono" value={ep.process.workdir} placeholder="~" onChange={(e) => set("process", { ...ep.process, workdir: e.target.value })} />
                </Field>
                <Field
                  label="进程网络"
                  help={
                    ep.process.network === "direct"
                      ? "启动时移除 HTTP_PROXY、HTTPS_PROXY、ALL_PROXY 等代理变量"
                      : ep.process.network
                        ? "启动时将上述代理变量设置为所选出口代理"
                        : "继承终端环境中的代理变量（如 ~/.zshrc 中的 HTTPS_PROXY），实际线路见运行日志"
                  }
                >
                  <select className="select" value={ep.process.network} onChange={(e) => set("process", { ...ep.process, network: e.target.value })}>
                    <option value="">继承终端环境</option>
                    <option value="direct">直连（移除代理变量）</option>
                    {config.proxies.map((p) => (
                      <option key={p.id} value={p.id}>
                        出口代理：{p.name}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="环境变量" help="已自动继承终端环境变量。在此追加或覆盖，每行一个 KEY=VALUE">
                  <textarea
                    className="textarea mono"
                    rows={3}
                    value={ep.process.env}
                    placeholder={"HTTPS_PROXY=http://proxy.corp:8080\nSSL_CERT_FILE=~/certs/corp-ca.pem"}
                    onChange={(e) => set("process", { ...ep.process, env: e.target.value })}
                  />
                </Field>
                {!isNew && original?.process.enabled && <ProcessPanel endpointId={ep.id} dirty={JSON.stringify(original.process) !== JSON.stringify(ep.process)} />}
              </div>
            )}
          </div>
        </div>

        {/* ---------------------------------------------------------------- 模型 */}
        <div className="settings-group">
          <h3>
            模型
            <span className="h3-hint">未列出的模型也可通过「{ep.id || "前缀"}/模型 ID」直接透传</span>
          </h3>
          <div className="panel">
            <table className="grid-table">
              <thead>
                <tr>
                  <th style={{ width: "24%" }}>模型 ID</th>
                  <th style={{ width: "24%" }}>上游模型 ID</th>
                  <th>显示名称</th>
                  <th style={{ width: 96 }}>上下文</th>
                  <th style={{ width: 56 }}>图像</th>
                  <th style={{ width: 56 }}>启用</th>
                  <th style={{ width: 36 }} />
                </tr>
              </thead>
              <tbody>
                {ep.models.map((m, i) => (
                  <tr key={i}>
                    <td>
                      <input className="input mono" value={m.name} placeholder="sonnet" onChange={(e) => updateModel(i, { name: e.target.value.trim() })} />
                    </td>
                    <td>
                      <input
                        className="input mono"
                        value={m.upstream}
                        placeholder={m.name || "同模型 ID"}
                        onChange={(e) => updateModel(i, { upstream: e.target.value.trim() })}
                      />
                    </td>
                    <td>
                      <input
                        className="input"
                        value={m.displayName}
                        placeholder={`${ep.id}/${m.name}`}
                        onChange={(e) => updateModel(i, { displayName: e.target.value })}
                      />
                    </td>
                    <td>
                      <ContextSelect value={m.contextWindow} onChange={(v) => updateModel(i, { contextWindow: v })} />
                    </td>
                    <td>
                      <Switch on={m.vision} onChange={(v) => updateModel(i, { vision: v })} />
                    </td>
                    <td>
                      <Switch on={m.enabled} onChange={(v) => updateModel(i, { enabled: v })} />
                    </td>
                    <td>
                      <button
                        className="icon-btn"
                        onClick={() =>
                          set(
                            "models",
                            ep.models.filter((_, j) => j !== i),
                          )
                        }
                      >
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
              <button className="btn btn-sm" disabled={isNew} title={isNew ? "请先保存端点" : "调用上游的 /models 接口"} onClick={() => setFetching(true)}>
                <CloudDownload size={14} />
                从上游获取
              </button>
              <button className="btn btn-sm" disabled={!ep.models.length || filling} onClick={autoFill}>
                <Sparkles size={14} />
                {filling ? "查询中…" : "自动填充上下文"}
              </button>
              <span className="muted-sm foot-note">数据来源：LiteLLM 公开模型表；未收录的模型需手动填写</span>
            </div>
          </div>
          {ep.models.some((m) => m.contextWindow >= 1_000_000) && (
            <p className="muted-sm" style={{ margin: "8px 4px 0" }}>
              上下文 ≥ 1M 的模型在 Claude Code 中以 <code>[1m]</code> 后缀列出并按 1M 上下文使用；Anthropic 端点会自动附加 1M beta 标头。
            </p>
          )}
        </div>

        {/* ---------------------------------------------------------------- 故障切换 */}
        <div className="settings-group">
          <h3>
            故障切换
            <span className="h3-hint">该端点请求失败时依次尝试以下模型，触发条件在「设置」中配置</span>
          </h3>
          <div className="panel panel-pad">
            <ModelChain value={ep.fallbacks} onChange={(v) => set("fallbacks", v)} exclude={ep.models.map((m) => `${ep.id}/${m.name}`)} />
            {ep.models.length > 0 && (
              <details className="per-model">
                <summary>按模型单独设置（优先于端点设置）</summary>
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
      {testing && <TestDialog endpoint={ep} onClose={() => setTesting(false)} />}
      {dirty && (
        <div className="unsaved-bar">
          <span>
            <i className="sdot warn" />
            {isNew ? "端点尚未添加" : "有未保存的修改"}
          </span>
          <button className="btn btn-sm btn-ghost" onClick={discard}>
            放弃修改
          </button>
          <button className="btn btn-sm btn-primary" onClick={() => save(true)} disabled={saving}>
            {isNew ? "添加" : "保存"}
          </button>
        </div>
      )}
      {fetching && (
        <UpstreamModelsDialog
          endpoint={ep}
          beforeFetch={() => save(true)}
          onClose={() => setFetching(false)}
          onAdd={(models) => {
            setEp((x) => ({ ...x, models: [...x.models, ...models] }));
            setFetching(false);
            toast(`已添加 ${models.length} 个模型，保存后生效`);
          }}
        />
      )}
    </div>
  );
}
