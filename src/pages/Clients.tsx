import { ReactNode, useEffect, useMemo, useState } from "react";
import { Check, Download, FileJson, RotateCcw } from "lucide-react";
import { api, ApplyOptions, ApplyReport, ClaudeInspection, ClientInfo, DesktopModel, fmtContext } from "../api";
import { useApp } from "../App";
import ModelInput from "../components/ModelInput";
import { CopyButton, Modal, Switch, useToast } from "../components/ui";

const CTX_OPTIONS = [32_768, 65_536, 131_072, 200_000, 262_144, 400_000];

export default function Clients() {
  const { config, baseUrl, saveConfig, models } = useApp();
  const toast = useToast();
  const [clients, setClients] = useState<ClientInfo[]>([]);
  const [opts, setOpts] = useState<Record<string, ApplyOptions>>({});
  const [report, setReport] = useState<(ApplyReport & { name: string }) | null>(null);
  const refresh = () => api.listClients().then(setClients);
  useEffect(() => {
    refresh();
  }, []);

  // Claude Code 对不认识的模型名默认按 1M 上下文；不带 [1m] 的模型按这个值压缩上下文。建议取非 1M 模型里最小的窗口
  const smallWindows = models.map((m) => m.contextWindow).filter((c) => c > 0 && c < 1_000_000);
  const suggestedCtx = smallWindows.length ? Math.min(...smallWindows) : 0;
  const o = (id: string): ApplyOptions => {
    const c = clients.find((x) => x.id === id);
    return (
      opts[id] ?? {
        model: c?.model ?? "",
        opusModel: "",
        sonnetModel: "",
        smallModel: "",
        discovery: true,
        maxContextTokens: c?.maxContextTokens || (id === "claude" ? suggestedCtx : 0),
      }
    );
  };
  const setO = (id: string, patch: Partial<ApplyOptions>) => setOpts({ ...opts, [id]: { ...o(id), ...patch } });
  const client = (id: ClientInfo["id"]) => clients.find((c) => c.id === id);

  const apply = async (c: ClientInfo) => {
    try {
      const r = await api.applyClient(c.id, o(c.id));
      setReport({ ...r, name: c.name });
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
  const claude = client("claude");
  const codex = client("codex");
  const gemini = client("gemini");

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 960 }}>
        <div className="page-header">
          <div>
            <h1 className="page-title">客户端接入</h1>
            <div className="page-subtitle">所有客户端都只连本机网关；以后切换模型或端点，不用再改客户端</div>
          </div>
        </div>

        <div className="conn-strip">
          <div>
            <span>网关地址</span>
            <code>{baseUrl}</code>
            <CopyButton text={baseUrl} small />
          </div>
          <div>
            <span>API Key</span>
            <code>{key}</code>
            <CopyButton text={key} small />
            {!config.server.accessKey && <em>未设置访问密钥，填任意值都可以</em>}
          </div>
        </div>

        {/* ---------------------------------------------------------------- Claude Code */}
        {claude && (
          <ClientCard
            glyph="C"
            color="#d97757"
            title="Claude Code"
            sub="命令行版 · 写入 ~/.claude/settings.json"
            info={claude}
            onApply={() => apply(claude)}
            onRestore={() => restore(claude)}
          >
            <div className="cc-grid">
              <Row label="默认模型" help="留空则用 Claude Code 自己的默认模型（按路由规则或兜底模型转发）">
                <ModelInput value={o("claude").model} onChange={(v) => setO("claude", { model: v })} placeholder="例如 internal/sonnet" />
              </Row>
              <Row label="/model 里的三档" help="Claude Code 的 Opus、Sonnet、Haiku 实际用哪个模型。留空就跟默认模型一致；Haiku 也用于生成标题、摘要等后台小任务">
                <div className="tier-grid">
                  {(
                    [
                      ["opusModel", "Opus"],
                      ["sonnetModel", "Sonnet"],
                      ["smallModel", "Haiku"],
                    ] as const
                  ).map(([k, label]) => (
                    <label key={k}>
                      <span>{label}</span>
                      <ModelInput value={o("claude")[k]} onChange={(v) => setO("claude", { [k]: v })} placeholder={o("claude").model || "跟默认模型一致"} />
                    </label>
                  ))}
                </div>
              </Row>
              <Row
                label="非 1M 模型的上下文窗口"
                help={
                  <>
                    Claude Code 默认把网关里的模型当成 1M 上下文；设置后，不带 [1m] 的模型会按这个大小压缩上下文。
                    {suggestedCtx > 0 && ` 建议 ${fmtContext(suggestedCtx)}（已启用模型里最小的窗口）`}
                  </>
                }
              >
                <select className="select" value={o("claude").maxContextTokens} onChange={(e) => setO("claude", { maxContextTokens: Number(e.target.value) })}>
                  <option value={0}>不设置</option>
                  {[...new Set([...CTX_OPTIONS, ...(suggestedCtx ? [suggestedCtx] : []), ...(o("claude").maxContextTokens ? [o("claude").maxContextTokens] : [])])]
                    .sort((a, b) => a - b)
                    .map((v) => (
                      <option key={v} value={v}>
                        {fmtContext(v)}
                        {v === suggestedCtx ? "（建议）" : ""}
                      </option>
                    ))}
                </select>
              </Row>
            </div>
            <ClaudeCheck refreshKey={report} />
            <div className="cc-switches">
              <SwitchRow
                on={o("claude").discovery}
                onChange={(v) => setO("claude", { discovery: v })}
                title="/model 里列出网关的所有模型"
                desc="写入 CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1"
              />
              <SwitchRow
                on={config.routing.claudeAlias}
                onChange={(v) => saveConfig({ ...config, routing: { ...config.routing, claudeAlias: v } }).catch(() => {})}
                title="列表里也显示非 Claude 模型"
                desc="名字不含 claude 的模型显示为 claude@前缀/模型，立即生效"
              />
            </div>
          </ClientCard>
        )}

        {/* ---------------------------------------------------------------- Claude 桌面版 */}
        <DesktopCard baseUrl={baseUrl} apiKey={key} />

        {/* ---------------------------------------------------------------- Codex / Gemini */}
        <div className="cc-pair">
          {codex && (
            <ClientCard glyph="X" color="#10a37f" title="Codex CLI" sub="写入 ~/.codex/config.toml" info={codex} onApply={() => apply(codex)} onRestore={() => restore(codex)} compact>
              <Row label="默认模型">
                <ModelInput value={o("codex").model} onChange={(v) => setO("codex", { model: v })} placeholder="例如 internal/sonnet" />
              </Row>
              {config.server.accessKey && <p className="muted-sm">设置了访问密钥：请在 shell 里 export KIKI_PROXY_KEY={config.server.accessKey}</p>}
            </ClientCard>
          )}
          {gemini && (
            <ClientCard glyph="G" color="#4f7cff" title="Gemini CLI" sub="写入 ~/.gemini/.env" info={gemini} onApply={() => apply(gemini)} onRestore={() => restore(gemini)} compact>
              <Row label="默认模型">
                <ModelInput value={o("gemini").model} onChange={(v) => setO("gemini", { model: v })} placeholder="例如 gm/gemini-2.5-pro" />
              </Row>
            </ClientCard>
          )}
        </div>

        {report && (
          <Modal
            title={`已写入 ${report.name} 的配置`}
            icon={<Check size={18} />}
            wide
            onClose={() => setReport(null)}
            footer={
              <button className="btn btn-primary" onClick={() => setReport(null)}>
                好的
              </button>
            }
          >
            <p className="muted-sm" style={{ marginTop: 0 }}>
              文件：<code>{report.file}</code>。原文件已备份，可以随时「恢复」。重新打开 {report.name} 后生效。
            </p>
            {report.changes.length === 0 ? (
              <div className="banner ok">配置已经是最新的，没有需要改动的地方</div>
            ) : (
              <ul className="change-list">
                {report.changes.map((c, i) => (
                  <li key={i} className={c.startsWith("删除") ? "del" : c.startsWith("修改") ? "mod" : "add"}>
                    {c}
                  </li>
                ))}
              </ul>
            )}
          </Modal>
        )}

        {/* ---------------------------------------------------------------- 其他 */}
        <div className="panel cc-card">
          <div className="cc-head">
            <span className="cc-glyph" style={{ background: "#646a80" }}>…</span>
            <div className="cc-title">
              <b>其他工具</b>
              <span>Cursor、Cherry Studio、dsh 等，按工具支持的格式填下面的地址；模型名写「端点前缀/模型名」</span>
            </div>
          </div>
          <table className="other-table">
            <tbody>
              {[
                ["Anthropic 格式", baseUrl, "/v1/messages"],
                ["OpenAI 格式", `${baseUrl}/v1`, "/chat/completions、/responses"],
                ["Gemini 格式", baseUrl, "/v1beta/models/{模型}:generateContent"],
              ].map(([name, url, path]) => (
                <tr key={name}>
                  <td>{name}</td>
                  <td>
                    <code>{url}</code>
                    <CopyButton text={url} small />
                  </td>
                  <td className="muted-sm">{path}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}

function ClientCard(props: {
  glyph: string;
  color: string;
  title: string;
  sub: string;
  info?: ClientInfo;
  connected?: boolean;
  onApply?: () => void;
  onRestore?: () => void;
  applyText?: string;
  compact?: boolean;
  children: ReactNode;
}) {
  const connected = props.connected ?? props.info?.connected;
  return (
    <div className={`panel cc-card ${props.compact ? "compact" : ""}`}>
      <div className="cc-head">
        <span className="cc-glyph" style={{ background: props.color }}>
          {props.glyph}
        </span>
        <div className="cc-title">
          <b>
            {props.title}
            <span className={`pill ${connected ? "on" : ""}`}>{connected ? "已接入" : "未接入"}</span>
          </b>
          <span>{props.sub}</span>
        </div>
        <div className="cc-actions">
          {props.info?.hasBackup && props.onRestore && (
            <button className="btn btn-sm btn-ghost" onClick={props.onRestore} title="恢复成接入前的配置文件">
              <RotateCcw size={14} />
              恢复
            </button>
          )}
          {props.onApply && (
            <button className="btn btn-sm btn-primary" onClick={props.onApply}>
              {props.applyText ?? (connected ? "更新配置" : "一键接入")}
            </button>
          )}
        </div>
      </div>
      <div className="cc-body">{props.children}</div>
    </div>
  );
}

function Row(props: { label: string; help?: ReactNode; children: ReactNode }) {
  return (
    <div className="cc-row">
      <label>{props.label}</label>
      <div>
        {props.children}
        {props.help && <p>{props.help}</p>}
      </div>
    </div>
  );
}

function SwitchRow(props: { on: boolean; onChange: (v: boolean) => void; title: string; desc: string }) {
  return (
    <div className="cc-switch">
      <Switch on={props.on} onChange={props.onChange} />
      <div>
        <b>{props.title}</b>
        <span>{props.desc}</span>
      </div>
    </div>
  );
}

/** Claude Code 桌面版：生成可导入的配置文件，或者手动填写 */
function DesktopCard(props: { baseUrl: string; apiKey: string }) {
  const { models } = useApp();
  const toast = useToast();
  const [connected, setConnected] = useState(false);
  const [picked, setPicked] = useState<string[] | null>(null);
  const [first, setFirst] = useState("");
  const [file, setFile] = useState("");
  const [behaves, setBehaves] = useState<Record<string, string>>({});
  useEffect(() => {
    api.desktopConnected().then(setConnected);
  }, []);
  const behavesOf = (id: string) => behaves[id] ?? guessBehaves(id);
  const list = picked ?? models.map((m) => m.id);
  const ordered = useMemo(() => {
    const def = first && list.includes(first) ? first : list[0];
    return def ? [def, ...list.filter((x) => x !== def)] : [];
  }, [list, first]);

  const exportFile = async () => {
    const payload: DesktopModel[] = ordered.map((id) => {
      const m = models.find((x) => x.id === id)!;
      return {
        name: id,
        label: m.displayName !== id ? m.displayName : "",
        supports1m: m.contextWindow >= 1_000_000,
        behavesAs: namesClaude(id) ? "" : behavesOf(id),
      };
    });
    try {
      setFile(await api.exportDesktopConfig(payload));
      toast("配置文件已生成");
    } catch (e) {
      toast(String(e), true);
    }
  };

  return (
    <ClientCard glyph="D" color="#1b1f2e" title="Claude Code 桌面版" sub="桌面版不读 settings.json，用它自己的「第三方推理」配置" connected={connected}>
      <div className="desk">
        <div className="desk-models">
          <div className="desk-label">
            放进桌面版模型列表的模型
            <span>第一个是默认模型</span>
          </div>
          <div className="desk-list">
            {models.map((m) => {
              const on = list.includes(m.id);
              return (
                <label key={m.id} className={on ? "on" : ""}>
                  <input
                    type="checkbox"
                    checked={on}
                    onChange={() => setPicked(on ? list.filter((x) => x !== m.id) : [...list, m.id])}
                  />
                  <code>{m.id}</code>
                  {m.contextWindow >= 1_000_000 && <span className="tag accent">1M</span>}
                  {on && !namesClaude(m.id) && (
                    <select
                      className="desk-behaves"
                      title="桌面版只认 Claude 模型，这个模型在桌面版里按哪个 Claude 模型处理（思考强度、上下文上限）。实际请求仍然发给这个模型"
                      value={behavesOf(m.id)}
                      onClick={(e) => e.preventDefault()}
                      onChange={(e) => setBehaves({ ...behaves, [m.id]: e.target.value })}
                    >
                      {BEHAVES.map(([v, t]) => (
                        <option key={v} value={v}>
                          按 {t}
                        </option>
                      ))}
                    </select>
                  )}
                  {on && (
                    <button className={`desk-def ${ordered[0] === m.id ? "is" : ""}`} onClick={(e) => (e.preventDefault(), setFirst(m.id))}>
                      {ordered[0] === m.id ? "默认" : "设为默认"}
                    </button>
                  )}
                </label>
              );
            })}
          </div>
        </div>

        <div className="desk-steps">
          <div className="desk-label">接入步骤</div>
          <ol>
            <li>
              <button className="btn btn-sm btn-primary" onClick={exportFile} disabled={!ordered.length}>
                <Download size={14} />
                生成配置文件
              </button>
              {file && (
                <span className="desk-file">
                  <Check size={13} /> 已保存到 <code>{file}</code>
                </span>
              )}
            </li>
            <li>
              打开桌面版：Help → Troubleshooting → <b>Enable Developer Mode</b>（只需一次）
            </li>
            <li>
              Developer → <b>Configure Third-Party Inference…</b> → <b>Import configuration…</b>，选择刚生成的 <FileJson size={12} /> 文件
            </li>
            <li>
              点 <b>Apply Changes</b>，再 <b>Relaunch</b>
            </li>
          </ol>
          <details className="desk-manual">
            <summary>不想导入文件？也可以在那个窗口里手动填</summary>
            <table>
              <tbody>
                <tr>
                  <td>Inference provider</td>
                  <td>
                    <code>Gateway</code>
                  </td>
                </tr>
                <tr>
                  <td>Gateway base URL</td>
                  <td>
                    <code>{props.baseUrl}</code> <CopyButton text={props.baseUrl} small />
                  </td>
                </tr>
                <tr>
                  <td>Gateway API key</td>
                  <td>
                    <code>{props.apiKey}</code> <CopyButton text={props.apiKey} small />
                  </td>
                </tr>
                <tr>
                  <td>Gateway auth scheme</td>
                  <td>
                    <code>Bearer</code>
                  </td>
                </tr>
                <tr>
                  <td>Model list</td>
                  <td className="muted-sm">留空即可，桌面版会从网关拉取模型列表</td>
                </tr>
              </tbody>
            </table>
          </details>
          <p className="muted-sm desk-note">
            用了网关后，桌面版的云端环境和 Remote Control 不可用（官方限制）。官方文档没有说明是否允许 http://127.0.0.1 这样的本机地址，如果提示 Gateway was unreachable，请告诉我。
          </p>
        </div>
      </div>
    </ClientCard>
  );
}

/** 实时检查 Claude Code 会读到的配置：哪些项没指向网关、用了网关里没有的模型、会绕开网关 */
function ClaudeCheck(props: { refreshKey: unknown }) {
  const [r, setR] = useState<ClaudeInspection | null>(null);
  const [appliedAt, setAppliedAt] = useState(0);
  useEffect(() => {
    if (props.refreshKey) setAppliedAt(Date.now());
  }, [props.refreshKey]);
  useEffect(() => {
    let alive = true;
    const tick = () => api.inspectClaude().then((x) => alive && setR(x)).catch(() => {});
    tick();
    const t = setInterval(tick, 3000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);
  if (!r) return null;
  // 接入之后文件又被改了（比如 cc-switch 把它的配置写了回去）
  const touched = appliedAt > 0 && r.files.some((f) => f.modified > appliedAt + 1500);
  return (
    <div className={`cc-check ${r.issues.length ? "bad" : "ok"}`}>
      <div className="cc-check-head">
        <b>{r.issues.length ? `当前生效的配置有 ${r.issues.length} 处问题` : "当前生效的配置没有问题"}</b>
        <span>实时读取 {r.files.map((f) => f.path.replace(/^\/Users\/[^/]+/, "~")).join("、") || "（没有找到配置文件）"}</span>
      </div>
      {touched && (
        <div className="cc-check-warn">
          接入之后，配置文件又被别的程序改动过。如果 cc-switch 之类的工具还在运行，它可能会把自己的配置写回去，请先退出它，再点一次「更新配置」。
        </div>
      )}
      {r.issues.length > 0 && (
        <ul>
          {r.issues.map((i, k) => (
            <li key={k}>
              <code>{i.key}</code>
              {i.value && <code className="val">{i.value}</code>}
              <span>{i.problem}</span>
              {r.files.length > 1 && <em>{i.file.replace(/^\/Users\/[^/]+/, "~")}</em>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** 桌面版认识的 Claude 模型，用来说明网关里的其他模型「按哪个处理」 */
const BEHAVES: [string, string][] = [
  ["claude-sonnet-4-6", "Sonnet 4.6"],
  ["claude-opus-4-6", "Opus 4.6"],
  ["claude-haiku-4-5", "Haiku 4.5"],
  ["claude-sonnet-5", "Sonnet 5"],
  ["claude-opus-5", "Opus 5"],
];

/** 名字里已经带了带版本号的 Claude 模型，桌面版能直接认出来 */
function namesClaude(id: string) {
  return /claude-(opus|sonnet|haiku|fable)-\d/i.test(id);
}

function guessBehaves(id: string) {
  const l = id.toLowerCase();
  if (l.includes("opus")) return "claude-opus-4-6";
  if (l.includes("haiku") || l.includes("flash") || l.includes("mini")) return "claude-haiku-4-5";
  return "claude-sonnet-4-6";
}
