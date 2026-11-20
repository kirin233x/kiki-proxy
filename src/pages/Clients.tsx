import { useEffect, useState } from "react";
import { CheckCircle2, Monitor, RotateCcw, TerminalSquare, Wrench } from "lucide-react";
import { api, ApplyOptions, ClientInfo } from "../api";
import { useApp } from "../App";
import ModelInput from "../components/ModelInput";
import { CopyButton, Field, Switch, useToast } from "../components/ui";

const DESC: Record<ClientInfo["id"], string> = {
  claude: "写入 env：ANTHROPIC_BASE_URL、ANTHROPIC_AUTH_TOKEN，以及模型设置",
  codex: "添加 model_providers.kiki（Responses 协议），并设为默认 provider",
  gemini: "写入 GOOGLE_GEMINI_BASE_URL、GEMINI_API_KEY，并切换到 API Key 登录方式",
};

export default function Clients() {
  const { config, baseUrl } = useApp();
  const toast = useToast();
  const [clients, setClients] = useState<ClientInfo[]>([]);
  const [opts, setOpts] = useState<Record<string, ApplyOptions>>({});
  const refresh = () => api.listClients().then(setClients);
  useEffect(() => {
    refresh();
  }, []);

  const o = (id: string): ApplyOptions => opts[id] ?? { model: clients.find((c) => c.id === id)?.model ?? "", smallModel: "", discovery: true };
  const setO = (id: string, patch: Partial<ApplyOptions>) => setOpts({ ...opts, [id]: { ...o(id), ...patch } });

  const apply = async (c: ClientInfo) => {
    try {
      await api.applyClient(c.id, o(c.id));
      toast(`已配置 ${c.name}，重新打开 ${c.name} 后生效`);
      refresh();
    } catch (e) {
      toast(String(e), true);
    }
  };
  const restore = async (c: ClientInfo) => {
    try {
      await api.restoreClient(c.id);
      toast(`已恢复 ${c.name} 原来的配置`);
      refresh();
    } catch (e) {
      toast(String(e), true);
    }
  };

  const key = config.server.accessKey || "kiki-proxy";
  const snippets = [
    { name: "Anthropic 协议", hint: "Claude Code、dsh（Anthropic 模式）等", url: baseUrl, path: "/v1/messages" },
    { name: "OpenAI 协议", hint: "Chat Completions 与 Responses，dsh（OpenAI 模式）、Cursor、Cherry Studio 等", url: `${baseUrl}/v1`, path: "/chat/completions · /responses" },
    { name: "Gemini 协议", hint: "Gemini CLI 等", url: baseUrl, path: "/v1beta/models/{模型}:generateContent" },
  ];

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 940 }}>
        <div className="page-header">
          <div>
            <h1 className="page-title">客户端接入</h1>
            <div className="page-subtitle">客户端始终连本机网关，切换端点不需要再改客户端配置</div>
          </div>
        </div>

        <div className="client-list">
          {clients.map((c) => (
            <div key={c.id} className="panel panel-pad client-card">
              <div className="client-head">
                <div className="client-icon">
                  <TerminalSquare size={20} />
                </div>
                <div className="client-title">
                  <b>
                    {c.name}
                    {c.connected && (
                      <span className="connected">
                        <CheckCircle2 size={13} />
                        已接入
                      </span>
                    )}
                  </b>
                  <span>{DESC[c.id]}</span>
                  <code className="mono-sm">{c.path}</code>
                </div>
                <div className="client-actions">
                  {c.hasBackup && (
                    <button className="btn btn-sm btn-ghost" onClick={() => restore(c)} title="恢复成接入前的配置文件">
                      <RotateCcw size={14} />
                      恢复原配置
                    </button>
                  )}
                  <button className="btn btn-sm btn-primary" onClick={() => apply(c)}>
                    {c.connected ? "更新配置" : "一键接入"}
                  </button>
                </div>
              </div>
              <div className="client-opts">
                <Field label="默认模型" help={c.id === "claude" ? "留空则使用 Claude Code 自己的默认模型（会经过路由规则 / 默认模型）" : undefined}>
                  <ModelInput value={o(c.id).model} onChange={(v) => setO(c.id, { model: v })} placeholder="例如 internal/sonnet" />
                </Field>
                {c.id === "claude" && (
                  <>
                    <Field label="后台小任务模型" help="Claude Code 生成标题、摘要等用的 haiku 模型">
                      <ModelInput value={o(c.id).smallModel} onChange={(v) => setO(c.id, { smallModel: v })} placeholder="例如 ds/deepseek-chat" />
                    </Field>
                    <div className="span-2 setting-inline">
                      <Switch on={o(c.id).discovery} onChange={(v) => setO(c.id, { discovery: v })} />
                      <div>
                        <b>在 /model 里列出网关的所有模型</b>
                        <span>设置 CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1，Claude Code 会从网关拉取模型列表</span>
                      </div>
                    </div>
                  </>
                )}
                {c.id === "codex" && config.server.accessKey && (
                  <div className="span-2 muted-sm">设置了访问密钥：请在 shell 里 export KIKI_PROXY_KEY={config.server.accessKey}</div>
                )}
              </div>
            </div>
          ))}
        </div>

        <div className="settings-group" style={{ marginTop: 28 }}>
          <h3>
            <Monitor size={14} /> Claude Code 桌面版
          </h3>
          <div className="panel panel-pad desktop-steps">
            <p>桌面版不读取 settings.json，需要在它的「第三方推理」设置里手动填写：</p>
            <ol>
              <li>菜单 Help → Troubleshooting → Enable Developer Mode</li>
              <li>
                Developer → Configure Third-Party Inference，网关地址填 <code>{baseUrl}</code> <CopyButton text={baseUrl} small />
              </li>
              <li>
                密钥随便填（如 <code>{key}</code>）；如果在设置里配置了访问密钥，就填访问密钥
              </li>
            </ol>
          </div>
        </div>

        <div className="settings-group">
          <h3>
            <Wrench size={14} /> 其他工具
          </h3>
          <div className="panel">
            {snippets.map((s) => (
              <div key={s.name} className="setting-row">
                <div className="setting-text">
                  <b>{s.name}</b>
                  <span>
                    {s.hint}
                    <br />
                    接口：<code>{s.path}</code>
                  </span>
                </div>
                <div className="snippet">
                  <code>{s.url}</code>
                  <CopyButton text={s.url} small />
                </div>
              </div>
            ))}
            <div className="setting-row">
              <div className="setting-text">
                <b>API Key</b>
                <span>{config.server.accessKey ? "设置里配置的访问密钥" : "没有设置访问密钥，填任意非空值即可"}</span>
              </div>
              <div className="snippet">
                <code>{key}</code>
                <CopyButton text={key} small />
              </div>
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>模型名</b>
                <span>使用「端点前缀/模型名」，例如 {config.endpoints[0] ? `${config.endpoints[0].id}/${config.endpoints[0].models[0]?.name ?? "模型"}` : "ds/deepseek-chat"}</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
