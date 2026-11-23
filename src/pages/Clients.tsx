import { ReactNode, useEffect, useState } from "react";
import { ArrowDown, ArrowUp, Check, Download, FileJson, Plus, RotateCcw, X } from "lucide-react";
import { api, ApplyOptions, ApplyReport, ClaudeInspection, ClientInfo, DesktopRoute, fmtContext } from "../api";
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

  // 调整模型列表顺序：立即保存，网关的 /v1/models 按这个顺序返回
  const moveModel = (from: number, to: number) => {
    const ids = models.map((m) => m.id);
    const [x] = ids.splice(from, 1);
    ids.splice(to, 0, x);
    saveConfig({ ...config, routing: { ...config.routing, modelOrder: ids } }).catch(() => {});
  };

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
                  label="Subagent 模型"
                  help="所有 subagent 固定用这个模型（写入 CLAUDE_CODE_SUBAGENT_MODEL）。留空由 Claude Code 决定：一般跟随当前模型，Explore 等内置 agent 用 Haiku 档"
                >
                  <ModelInput value={o("claude").subagentModel} onChange={(v) => setO("claude", { subagentModel: v })} placeholder="留空：由 Claude Code 决定" />
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

            <div className="panel cc-card">
              <div className="cc-head">
                <div className="cc-title">
                  <b>/model 列表顺序</b>
                  <span>网关返回模型列表的顺序。Claude Code 的 /model、桌面版的模型发现、本应用里的模型选择都按这个顺序；调整后立即保存，重启 Claude Code 生效</span>
                </div>
              </div>
              <div className="cc-body">
                <OrderList
                  items={models}
                  keyOf={(m) => m.id}
                  onMove={moveModel}
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
          <ClientCard glyph="X" color="#10a37f" title="Codex CLI" sub="写入 ~/.codex/config.toml" info={codex} onApply={() => apply(codex)} onRestore={() => restore(codex)}>
            <div className="cc-grid">
              <Row label="默认模型" help={config.server.accessKey ? `设置了访问密钥：请在 shell 里 export KIKI_PROXY_KEY=${config.server.accessKey}` : "Codex 默认使用的模型，可以在 Codex 里用 /model 切换"}>
                <ModelInput value={o("codex").model} onChange={(v) => setO("codex", { model: v })} placeholder="例如 internal/sonnet" />
              </Row>
            </div>
          </ClientCard>
        )}
        {tab === "gemini" && gemini && (
          <ClientCard glyph="G" color="#4f7cff" title="Gemini CLI" sub="写入 ~/.gemini/.env" info={gemini} onApply={() => apply(gemini)} onRestore={() => restore(gemini)}>
            <div className="cc-grid">
              <Row label="默认模型" help="Gemini CLI 默认使用的模型">
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

/** 可以上下调整顺序的列表 */
function OrderList<T>(props: {
  items: T[];
  keyOf: (t: T) => string;
  render: (t: T, i: number) => ReactNode;
  onMove: (from: number, to: number) => void;
  extra?: (t: T, i: number) => ReactNode;
  empty?: ReactNode;
}) {
  const n = props.items.length;
  if (!n) return <div className="order-empty">{props.empty ?? "没有模型"}</div>;
  return (
    <div className="order-list">
      {props.items.map((t, i) => (
        <div key={props.keyOf(t)} className="order-row">
          <span className="order-num">{i + 1}</span>
          <div className="order-main">{props.render(t, i)}</div>
          {props.extra?.(t, i)}
          <div className="order-btns">
            <button className="icon-btn" title="上移" disabled={i === 0} onClick={() => props.onMove(i, i - 1)}>
              <ArrowUp size={14} />
            </button>
            <button className="icon-btn" title="下移" disabled={i === n - 1} onClick={() => props.onMove(i, i + 1)}>
              <ArrowDown size={14} />
            </button>
          </div>
        </div>
      ))}
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
    setRows(
      saved.length
        ? saved.map((r) => ({ model: r.model, role: roleOf(r.id) }))
        : models.slice(0, 6).map((m) => ({ model: m.id, role: guessRole(m.id) })),
    );
    setInited(true);
  }, [models, saved, inited]);
  const routes = buildRoutes(rows, models);
  const dirty = JSON.stringify(routes) !== JSON.stringify(saved);
  const rest = models.filter((m) => !rows.some((r) => r.model === m.id));

  const move = (from: number, to: number) => {
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
      sub="写入桌面版的「第三方推理」配置库，完全退出桌面版再打开生效"
      connected={props.connected}
      canRestore={props.connected}
      busy={busy || !routes.length}
      applyText={props.connected ? (dirty ? "更新配置" : "重新写入") : "一键接入"}
      onApply={() => run("apply")}
      onRestore={() => run("restore")}
    >
      <div className="cc-grid">
        <Row
          label="桌面版模型菜单"
          help={
            <>
              第一个是默认模型。档位决定桌面版按哪种 Claude 模型组织请求（如思考强度），实际请求发给左边的模型；某个模型报参数错误时可以换成 Haiku 档试试。
              {dirty && props.connected && <b className="desk-dirty"> 有改动还没写入桌面版</b>}
            </>
          }
        >
          <OrderList
            items={rows}
            keyOf={(r) => r.model}
            onMove={move}
            empty="还没有选择模型"
            render={(r, i) => {
              const route = routes.find((x) => x.model === r.model);
              const m = models.find((x) => x.id === r.model);
              return (
                <>
                  <code>{r.model}</code>
                  {i === 0 && <span className="tag accent">默认</span>}
                  {m && m.contextWindow >= 1_000_000 && <span className="tag">1M</span>}
                  {route && <span className="order-sub" title="桌面版看到的模型名">{route.id}</span>}
                </>
              );
            }}
            extra={(r) => (
              <>
                <select
                  className="select select-sm"
                  value={r.role}
                  title="桌面版按哪个档位处理这个模型"
                  onChange={(e) => setRows(rows.map((x) => (x.model === r.model ? { ...x, role: e.target.value as Role } : x)))}
                >
                  {ROLES.map((x) => (
                    <option key={x} value={x}>
                      {ROLE_NAME[x]} 档
                    </option>
                  ))}
                </select>
                <button className="icon-btn" title="从菜单里移除" onClick={() => setRows(rows.filter((x) => x.model !== r.model))}>
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
                <option value="">添加模型…</option>
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
              桌面版里打开 Help → Troubleshooting → <b>Enable Developer Mode</b>（只需一次）
            </li>
            <li>
              点右上角的「{props.connected ? "更新配置" : "一键接入"}」
            </li>
            <li>
              <b>完全退出</b>桌面版（⌘Q）再打开
            </li>
          </ol>
          <details className="desk-manual">
            <summary>想自己导入？生成配置文件</summary>
            <p className="muted-sm">
              Developer → <b>Configure Third-Party Inference…</b> → <b>Import configuration…</b>，选择生成的 <FileJson size={12} /> 文件，再 Apply
              Changes、Relaunch。
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
            桌面版只接受 claude-sonnet-*、claude-opus-* 这类模型名，所以 kiki-proxy 给每个模型分配一个这样的名字（同档多个时加 -r2、-r3），菜单里显示真实模型名，请求到网关再换回来。用了网关后，桌面版的云端环境和
            Remote Control 不可用（官方限制）。
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
