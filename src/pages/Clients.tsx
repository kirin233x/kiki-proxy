import { ReactNode, useEffect, useState } from "react";
import { Check, Download, FileJson, Plus, RotateCcw, X } from "lucide-react";
import { api, ApplyOptions, ApplyReport, ClaudeInspection, ClientInfo, DesktopRoute, fmtContext } from "../api";
import { useApp } from "../App";
import ModelInput from "../components/ModelInput";
import SortableList from "../components/SortableList";
import { CopyButton, Modal, Switch, useToast } from "../components/ui";

const CTX_OPTIONS = [32_768, 65_536, 131_072, 200_000, 262_144, 400_000];

export default function Clients() {
  const { config, baseUrl, saveConfig, models } = useApp();
  const toast = useToast();
  const [clients, setClients] = useState<ClientInfo[]>([]);
  const [opts, setOpts] = useState<Record<string, ApplyOptions>>({});
  const [report, setReport] = useState<(ApplyReport & { name: string }) | null>(null);
  const [tab, setTab] = useState<TabId>(() => {
    try {
      return (localStorage.getItem("kiki.clientTab") as TabId) || "claude";
    } catch {
      return "claude";
    }
  });
  const [deskOn, setDeskOn] = useState(false);
  const refresh = () => api.listClients().then(setClients);
  useEffect(() => {
    refresh();
    api.desktopConnected().then(setDeskOn);
  }, []);
  const pick = (t: TabId) => {
    setTab(t);
    try {
      localStorage.setItem("kiki.clientTab", t);
    } catch {
      /* 存不了就算了 */
    }
  };

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
        subagentModel: "",
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
  const isOn = (t: TabId) => (t === "desktop" ? deskOn : t === "other" ? false : !!client(t)?.connected);

  // 模型排序：先在本地生效，再保存；网关的 /v1/models 按这个顺序返回
  const [ordered, setOrdered] = useState(models);
  useEffect(() => setOrdered(models), [models]);
  const moveModel = (from: number, to: number) => {
    const next = [...ordered];
    const [x] = next.splice(from, 1);
    next.splice(to, 0, x);
    setOrdered(next);
    saveConfig({ ...config, routing: { ...config.routing, modelOrder: next.map((m) => m.id) } }).catch(() => setOrdered(models));
  };

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 960 }}>
        <div className="page-header">
          <div>
            <h1 className="page-title">客户端接入</h1>
            <div className="page-subtitle">客户端统一连接本机网关，切换模型或端点无需修改客户端配置</div>
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
            {!config.server.accessKey && <em>未启用访问密钥，可填写任意值</em>}
          </div>
        </div>

        <div className="client-tabs">
          {TABS.map((t) => (
            <button key={t.id} className={tab === t.id ? "on" : ""} onClick={() => pick(t.id)}>
              <span className="client-tab-glyph" style={{ background: t.color }}>
                {t.glyph}
              </span>
              {t.name}
              {isOn(t.id) && <i className="sdot ok" title="已接入" />}
            </button>
          ))}
        </div>

        {/* ---------------------------------------------------------------- Claude Code */}
        {tab === "claude" && claude && (
          <>
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
                <Row label="默认模型" help="写入 ANTHROPIC_MODEL。留空时由路由规则或兜底模型决定">
                  <ModelInput value={o("claude").model} onChange={(v) => setO("claude", { model: v })} placeholder="例如 internal/sonnet" />
                </Row>
                <Row
                  label="模型映射"
                  help="/model 中 Opus、Sonnet、Haiku 对应的网关模型（ANTHROPIC_DEFAULT_*_MODEL）。留空与默认模型相同；Haiku 同时用于标题生成等后台任务"
                >
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
                        <ModelInput value={o("claude")[k]} onChange={(v) => setO("claude", { [k]: v })} placeholder={o("claude").model || "同默认模型"} />
                      </label>
                    ))}
                  </div>
                </Row>
                <Row label="Subagent 模型" help="写入 CLAUDE_CODE_SUBAGENT_MODEL，所有 subagent 统一使用该模型。留空时由 Claude Code 按 agent 定义选择">
                  <ModelInput value={o("claude").subagentModel} onChange={(v) => setO("claude", { subagentModel: v })} placeholder="按 agent 定义" />
                </Row>
                <Row
                  label="上下文上限"
                  help={
                    <>
                      写入 CLAUDE_CODE_MAX_CONTEXT_TOKENS。Claude Code 默认将网关模型视为 1M 上下文，未标记 1M 的模型按此值触发自动压缩。
                      {suggestedCtx > 0 && `推荐 ${fmtContext(suggestedCtx)}（已启用模型中的最小窗口）。`}
                    </>
                  }
                >
                  <select className="select" value={o("claude").maxContextTokens} onChange={(e) => setO("claude", { maxContextTokens: Number(e.target.value) })}>
                    <option value={0}>不限制</option>
                    {[...new Set([...CTX_OPTIONS, ...(suggestedCtx ? [suggestedCtx] : []), ...(o("claude").maxContextTokens ? [o("claude").maxContextTokens] : [])])]
                      .sort((a, b) => a - b)
                      .map((v) => (
                        <option key={v} value={v}>
                          {fmtContext(v)}
                          {v === suggestedCtx ? "（推荐）" : ""}
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
                  title="模型发现"
                  desc="在 /model 中列出网关的全部模型（CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY）"
                />
                <SwitchRow
                  on={config.routing.claudeAlias}
                  onChange={(v) => saveConfig({ ...config, routing: { ...config.routing, claudeAlias: v } }).catch(() => {})}
                  title="显示非 Claude 模型"
                  desc="以 claude@前缀/模型 的形式列出，即时生效"
                />
              </div>
            </ClientCard>

            <div className="panel cc-card">
              <div className="cc-head">
                <div className="cc-title">
                  <b>模型排序</b>
                  <span>拖动调整网关模型列表的顺序，作用于 Claude Code 的 /model、桌面版模型发现及本应用的模型选择。修改即时保存，重启 Claude Code 后生效</span>
                </div>
              </div>
              <div className="cc-body">
                <SortableList
                  items={ordered}
                  keyOf={(m) => m.id}
                  onReorder={moveModel}
                  render={(m) => (
                    <>
                      <code>{m.id}</code>
                      {m.displayName !== m.id && <span className="order-sub">{m.displayName}</span>}
                      {m.contextWindow >= 1_000_000 && <span className="tag accent">1M</span>}
                    </>
                  )}
                />
              </div>
            </div>
          </>
        )}

        {/* ---------------------------------------------------------------- Claude 桌面版 */}
        {tab === "desktop" && <DesktopCard baseUrl={baseUrl} apiKey={key} connected={deskOn} onConnected={setDeskOn} />}

        {/* ---------------------------------------------------------------- Codex / Gemini */}
        {tab === "codex" && codex && (
          <ClientCard
            glyph="X"
            color="#10a37f"
            title="Codex CLI"
            sub="写入 ~/.codex/config.toml"
            info={codex}
            onApply={() => apply(codex)}
            onRestore={() => restore(codex)}
          >
            <div className="cc-grid">
              <Row
                label="默认模型"
                help={
                  config.server.accessKey
                    ? `已设置访问密钥，请在 shell 中设置环境变量 KIKI_PROXY_KEY=${config.server.accessKey}`
                    : "写入 config.toml 的 model，可在 Codex 中通过 /model 切换"
                }
              >
                <ModelInput value={o("codex").model} onChange={(v) => setO("codex", { model: v })} placeholder="例如 internal/sonnet" />
              </Row>
            </div>
          </ClientCard>
        )}
        {tab === "gemini" && gemini && (
          <ClientCard
            glyph="G"
            color="#4f7cff"
            title="Gemini CLI"
            sub="写入 ~/.gemini/.env"
            info={gemini}
            onApply={() => apply(gemini)}
            onRestore={() => restore(gemini)}
          >
            <div className="cc-grid">
              <Row label="默认模型" help="写入 GEMINI_MODEL">
                <ModelInput value={o("gemini").model} onChange={(v) => setO("gemini", { model: v })} placeholder="例如 gm/gemini-2.5-pro" />
              </Row>
            </div>
          </ClientCard>
        )}

        {/* ---------------------------------------------------------------- 其他 */}
        {tab === "other" && (
          <div className="panel cc-card">
            <div className="cc-head">
              <span className="cc-glyph" style={{ background: "#646a80" }}>
                …
              </span>
              <div className="cc-title">
                <b>其他工具</b>
                <span>Cursor、Cherry Studio、dsh 等工具按所支持的协议填写以下地址，模型名使用「前缀/模型 ID」</span>
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
        )}

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
      </div>
    </div>
  );
}

type TabId = "claude" | "desktop" | "codex" | "gemini" | "other";
const TABS: { id: TabId; name: string; glyph: string; color: string }[] = [
  { id: "claude", name: "Claude Code", glyph: "C", color: "#d97757" },
  { id: "desktop", name: "Claude 桌面版", glyph: "D", color: "#1b1f2e" },
  { id: "codex", name: "Codex CLI", glyph: "X", color: "#10a37f" },
  { id: "gemini", name: "Gemini CLI", glyph: "G", color: "#4f7cff" },
  { id: "other", name: "其他工具", glyph: "…", color: "#646a80" },
];

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
  canRestore?: boolean;
  busy?: boolean;
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
          {(props.canRestore ?? props.info?.hasBackup) && props.onRestore && (
            <button className="btn btn-sm btn-ghost" onClick={props.onRestore} disabled={props.busy} title="恢复成接入前的配置">
              <RotateCcw size={14} />
              恢复
            </button>
          )}
          {props.onApply && (
            <button className="btn btn-sm btn-primary" onClick={props.onApply} disabled={props.busy}>
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

/** Claude Code 桌面版：桌面版只接受 claude-sonnet-* 这类名字，给每个模型分配一个「档位名」，请求到网关再换回真实模型 */
function DesktopCard(props: { baseUrl: string; apiKey: string; connected: boolean; onConnected: (v: boolean) => void }) {
  const { models, config, saveConfig } = useApp();
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [file, setFile] = useState("");
  const saved = config?.desktop?.routes ?? [];
  // 选中的模型和档位；顺序就是桌面版菜单的顺序，第一个是默认模型
  const [rows, setRows] = useState<{ model: string; role: Role }[]>([]);
  const [inited, setInited] = useState(false);
  useEffect(() => {
    if (inited || !models.length) return;
    setRows(saved.length ? saved.map((r) => ({ model: r.model, role: roleOf(r.id) })) : models.slice(0, 6).map((m) => ({ model: m.id, role: guessRole(m.id) })));
    setInited(true);
  }, [models, saved, inited]);
  const routes = buildRoutes(rows, models);
  const dirty = JSON.stringify(routes) !== JSON.stringify(saved);
  const rest = models.filter((m) => !rows.some((r) => r.model === m.id));

  const reorder = (from: number, to: number) => {
    const next = [...rows];
    const [x] = next.splice(from, 1);
    next.splice(to, 0, x);
    setRows(next);
  };

  const run = async (kind: "apply" | "export" | "restore") => {
    if (!config) return;
    setBusy(true);
    try {
      if (kind === "restore") {
        await api.restoreDesktopConfig();
        toast("已恢复桌面版原来的配置，重启桌面版生效");
      } else {
        // 先保存路由表，网关才知道 claude-sonnet-5 这些名字对应哪个模型
        await saveConfig({ ...config, desktop: { routes } });
        if (kind === "apply") {
          await api.applyDesktopConfig();
          toast("已写入桌面版，完全退出桌面版（⌘Q）再打开即可");
        } else {
          setFile(await api.exportDesktopConfig());
          toast("配置文件已生成");
        }
      }
      props.onConnected(await api.desktopConnected());
    } catch (e) {
      toast(String(e), true);
    } finally {
      setBusy(false);
    }
  };

  return (
    <ClientCard
      glyph="D"
      color="#1b1f2e"
      title="Claude Code 桌面版"
      sub="写入「第三方推理」配置库，重启桌面版后生效"
      connected={props.connected}
      canRestore={props.connected}
      busy={busy || !routes.length}
      applyText={props.connected ? (dirty ? "更新配置" : "重新写入") : "一键接入"}
      onApply={() => run("apply")}
      onRestore={() => run("restore")}
    >
      <div className="cc-grid">
        <Row
          label="模型列表"
          help={
            <>
              拖动调整顺序，首项为默认模型。档位决定桌面版的请求参数（如思考强度），请求仍发送至所选模型；若上游不支持相关参数，可改用 Haiku 档位。
              {dirty && props.connected && <b className="desk-dirty"> 有尚未写入桌面版的修改。</b>}
            </>
          }
        >
          <SortableList
            items={rows}
            keyOf={(r) => r.model}
            onReorder={reorder}
            empty="尚未添加模型"
            render={(r, i) => {
              const route = routes.find((x) => x.model === r.model);
              const m = models.find((x) => x.id === r.model);
              return (
                <>
                  <code>{r.model}</code>
                  {i === 0 && <span className="tag accent">默认</span>}
                  {m && m.contextWindow >= 1_000_000 && <span className="tag">1M</span>}
                  {route && (
                    <span className="order-sub" title="桌面版看到的模型名">
                      {route.id}
                    </span>
                  )}
                </>
              );
            }}
            extra={(r) => (
              <>
                <select
                  className="select select-sm"
                  value={r.role}
                  title="桌面版档位"
                  onChange={(e) => setRows(rows.map((x) => (x.model === r.model ? { ...x, role: e.target.value as Role } : x)))}
                >
                  {ROLES.map((x) => (
                    <option key={x} value={x}>
                      {ROLE_NAME[x]}
                    </option>
                  ))}
                </select>
                <button className="icon-btn" title="移除" onClick={() => setRows(rows.filter((x) => x.model !== r.model))}>
                  <X size={14} />
                </button>
              </>
            )}
          />
          {rest.length > 0 && (
            <div className="order-add">
              <Plus size={14} />
              <select
                className="select select-sm"
                value=""
                onChange={(e) => e.target.value && setRows([...rows, { model: e.target.value, role: guessRole(e.target.value) }])}
              >
                <option value="">添加模型</option>
                {rest.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.id}
                  </option>
                ))}
              </select>
            </div>
          )}
        </Row>
        <Row label="接入步骤">
          <ol className="desk-ol">
            <li>
              在桌面版中启用开发者模式：Help → Troubleshooting → <b>Enable Developer Mode</b>
            </li>
            <li>点击右上角「{props.connected ? "更新配置" : "一键接入"}」</li>
            <li>完全退出（⌘Q）并重新打开桌面版</li>
          </ol>
          <details className="desk-manual">
            <summary>手动导入配置文件</summary>
            <p className="muted-sm">
              生成配置文件后，在桌面版中依次选择 Developer → <b>Configure Third-Party Inference…</b> → <b>Import configuration…</b>，导入该 <FileJson size={12} />{" "}
              文件，然后 Apply Changes 并 Relaunch。
            </p>
            <div className="desk-export">
              <button className="btn btn-sm" onClick={() => run("export")} disabled={!routes.length || busy}>
                <Download size={14} />
                生成配置文件
              </button>
              {file && (
                <span className="desk-file">
                  <Check size={13} /> 已保存到 <code>{file}</code>
                </span>
              )}
            </div>
            <table>
              <tbody>
                <tr>
                  <td>Gateway base URL</td>
                  <td>
                    <code>{props.baseUrl}/claude-desktop</code> <CopyButton text={`${props.baseUrl}/claude-desktop`} small />
                  </td>
                </tr>
                <tr>
                  <td>Gateway API key</td>
                  <td>
                    <code>{props.apiKey}</code> <CopyButton text={props.apiKey} small />
                  </td>
                </tr>
              </tbody>
            </table>
          </details>
          <p className="muted-sm">
            桌面版仅接受 claude-sonnet-*、claude-opus-* 等模型 ID。kiki-proxy 为每个模型分配对应档位的 ID（同档位依次追加
            -r2、-r3），菜单中显示真实模型名，网关收到请求后映射回原模型。启用网关后，桌面版的云端环境与 Remote Control 不可用。
          </p>
        </Row>
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
    const tick = () =>
      api
        .inspectClaude()
        .then((x) => alive && setR(x))
        .catch(() => {});
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
        <b>{r.issues.length ? `配置检查：发现 ${r.issues.length} 个问题` : "配置检查：未发现问题"}</b>
        <span>实时读取 {r.files.map((f) => f.path.replace(/^\/Users\/[^/]+/, "~")).join("、") || "（未找到配置文件）"}</span>
      </div>
      {touched && <div className="cc-check-warn">接入后配置文件被其他程序修改。若 cc-switch 等工具仍在运行，可能会覆盖当前配置，请退出后重新点击「更新配置」。</div>}
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

type Role = "sonnet" | "opus" | "haiku" | "fable";
const ROLES: Role[] = ["sonnet", "opus", "haiku", "fable"];
const ROLE_NAME: Record<Role, string> = { sonnet: "Sonnet", opus: "Opus", haiku: "Haiku", fable: "Fable" };
/** 每个档位给桌面版的名字（和 cc-switch 用的一样，桌面版都认识） */
const ROLE_ID: Record<Role, string> = {
  sonnet: "claude-sonnet-5",
  opus: "claude-opus-5",
  haiku: "claude-haiku-4-5",
  fable: "claude-fable-5",
};

function roleOf(id: string): Role {
  return ROLES.find((r) => id.includes(r)) ?? "sonnet";
}

function guessRole(model: string): Role {
  const l = model.toLowerCase();
  if (l.includes("opus")) return "opus";
  if (l.includes("haiku") || l.includes("flash") || l.includes("mini")) return "haiku";
  return "sonnet";
}

/** 同档的第二个起加 -r2、-r3… */
function buildRoutes(rows: { model: string; role: Role }[], models: { id: string; displayName: string; contextWindow: number }[]): DesktopRoute[] {
  const used = new Map<Role, number>();
  return rows
    .filter((r) => models.some((m) => m.id === r.model))
    .map((r) => {
      const n = (used.get(r.role) ?? 0) + 1;
      used.set(r.role, n);
      const m = models.find((x) => x.id === r.model)!;
      return {
        id: n === 1 ? ROLE_ID[r.role] : `${ROLE_ID[r.role]}-r${n}`,
        model: r.model,
        label: m.displayName && m.displayName !== m.id ? `${m.displayName}` : m.id,
        supports1m: m.contextWindow >= 1_000_000,
      };
    });
}
