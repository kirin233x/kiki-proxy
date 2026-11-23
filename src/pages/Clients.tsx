import { ReactNode, useEffect, useState } from "react";
import { Check, Download, FileJson, Plus, RotateCcw, Trash2 } from "lucide-react";
import { api, ApplyOptions, ApplyReport, ClaudeInspection, ClientInfo, DesktopRoute, fmtContext, ListedModel } from "../api";
import { useApp } from "../App";
import ModelInput from "../components/ModelInput";
import { SortHandle, useSortable } from "../components/SortableList";
import { CopyButton, Hint, Modal, Switch, useToast } from "../components/ui";

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
      /* 浏览器存储不可用时忽略 */
    }
  };

  const modelOf = (id: string) => models.find((m) => m.id === id);
  // Claude Code 对不认识的模型名默认按 1M 上下文；不带 [1m] 的模型按这个值压缩上下文。建议取非 1M 模型里最小的窗口
  const smallWindows = models.map((m) => m.contextWindow).filter((c) => c > 0 && c < 1_000_000);
  const suggestedCtx = smallWindows.length ? Math.min(...smallWindows) : 0;
  const o = (id: string): ApplyOptions => {
    const c = clients.find((x) => x.id === id);
    if (opts[id]) return opts[id];
    // 从当前配置读回：显示名和模型默认名字一样的，视为「自动」
    const tier = (role: string) => c?.tiers?.find((t) => t.role === role);
    const labels: Record<string, string> = {};

    for (const role of CLAUDE_TIERS) {
      const t = tier(role);
      if (!t?.model) continue;
      const m = modelOf(t.model);
      if (t.label && t.label !== t.model && t.label !== m?.displayName) labels[role] = t.label;
    }
    return {
      model: c?.model ?? "",
      opusModel: tier("opus")?.model ?? "",
      sonnetModel: tier("sonnet")?.model ?? "",
      smallModel: tier("haiku")?.model ?? "",
      discovery: true,
      maxContextTokens: c?.maxContextTokens || (id === "claude" ? suggestedCtx : 0),
      subagentModel: c?.subagentModel ?? "",
      tierLabels: labels,
      tier1m: {},
    };
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
      setOpts({});
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
    const next = move(ordered, from, to);
    setOrdered(next);
    saveConfig({ ...config, routing: { ...config.routing, modelOrder: next.map((m) => m.id) } }).catch(() => setOrdered(models));
  };

  // Claude Code 模型映射的三行
  const co = o("claude");
  const tierModelKey = { opus: "opusModel", sonnet: "sonnetModel", haiku: "smallModel" } as const;
  const tierRows = CLAUDE_TIERS.map((role) => {
    const chosen = co[tierModelKey[role]];
    const effective = chosen || co.model;
    const m = modelOf(effective);
    return {
      role,
      chosen,
      effective,
      label: co.tierLabels[role] ?? "",
      autoLabel: m?.displayName ?? effective,
      ctx: m?.contextWindow ?? 0,
    };
  });
  const setTier = (role: (typeof CLAUDE_TIERS)[number], patch: { model?: string; label?: string }) => {
    const next: Partial<ApplyOptions> = {};
    if (patch.model !== undefined) next[tierModelKey[role]] = patch.model;
    if (patch.label !== undefined) next.tierLabels = { ...co.tierLabels, [role]: patch.label };
    setO("claude", next);
  };

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 980 }}>
        <div className="page-header">
          <div>
            <h1 className="page-title">客户端接入</h1>
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
            {!config.server.accessKey && <Hint>未启用访问密钥，客户端填写任意值即可</Hint>}
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
              sub="~/.claude/settings.json"
              info={claude}
              onApply={() => apply(claude)}
              onRestore={() => restore(claude)}
            >
              <Section title="基本设置">
                <div className="cc-grid">
                  <Row label="默认模型" help="ANTHROPIC_MODEL。留空时由路由规则或兜底模型决定">
                    <ModelInput value={co.model} onChange={(v) => setO("claude", { model: v })} placeholder="例如 internal/sonnet" />
                  </Row>
                  <Row label="Subagent 模型" help="CLAUDE_CODE_SUBAGENT_MODEL。所有 subagent 统一使用该模型；留空时按 agent 定义选择">
                    <ModelInput value={co.subagentModel} onChange={(v) => setO("claude", { subagentModel: v })} placeholder="按 agent 定义" />
                  </Row>
                  <Row
                    label="上下文上限"
                    help={
                      <>
                        CLAUDE_CODE_MAX_CONTEXT_TOKENS。不支持 1M 的模型按此值自动压缩上下文。
                        {suggestedCtx > 0 && `推荐 ${fmtContext(suggestedCtx)}，即已启用模型中的最小窗口。`}
                      </>
                    }
                  >
                    <select className="select" value={co.maxContextTokens} onChange={(e) => setO("claude", { maxContextTokens: Number(e.target.value) })}>
                      <option value={0}>不限制</option>
                      {[...new Set([...CTX_OPTIONS, ...(suggestedCtx ? [suggestedCtx] : []), ...(co.maxContextTokens ? [co.maxContextTokens] : [])])]
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
              </Section>

              <Section title="模型映射" desc="/model 中三个角色对应的网关模型。留空沿用默认模型；Haiku 同时用于标题生成等后台任务">
                <MapTable
                  cols="150px minmax(0,1fr) minmax(0,1.3fr) 72px"
                  head={["模型角色", "菜单显示名", "实际请求模型", "上下文"]}
                  items={tierRows}
                  keyOf={(r) => r.role}
                  cells={(r) => [
                    <RoleChip key="r" role={r.role} />,
                    <input
                      key="l"
                      className="input"
                      value={r.label}
                      placeholder={r.autoLabel || "同模型显示名"}
                      onChange={(e) => setTier(r.role, { label: e.target.value })}
                    />,
                    <ModelInput key="m" value={r.chosen} onChange={(v) => setTier(r.role, { model: v })} placeholder={co.model || "同默认模型"} />,
                    <CtxTag key="c" n={r.ctx} />,
                  ]}
                />
              </Section>

              <ClaudeCheck refreshKey={report} />
              <div className="cc-switches">
                <SwitchRow
                  on={co.discovery}
                  onChange={(v) => setO("claude", { discovery: v })}
                  title="模型发现"
                  desc="在 /model 中列出网关的全部模型（CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY）"
                />
                <SwitchRow
                  on={config.routing.claudeAlias}
                  onChange={(v) => saveConfig({ ...config, routing: { ...config.routing, claudeAlias: v } }).catch(() => {})}
                  title="显示非 Claude 模型"
                  desc="名称不含 claude 的模型以 claude@前缀/模型 列出，即时生效"
                />
              </div>
            </ClientCard>

            <div className="panel cc-card">
              <div className="cc-body">
                <Section title="模型排序" desc="拖动调整顺序，作用于 /model、桌面版模型发现和本应用的模型选择。即时保存，重启 Claude Code 后生效">
                  <MapTable
                    cols="minmax(0,1.4fr) minmax(0,1fr) minmax(0,0.8fr) 80px"
                    head={["模型", "显示名称", "端点", "上下文"]}
                    items={ordered}
                    keyOf={(m) => m.id}
                    onReorder={moveModel}
                    cells={(m) => [
                      <code key="id" className="map-model">
                        {m.id}
                      </code>,
                      <span key="n" className="map-text">
                        {m.displayName !== m.id ? m.displayName : "—"}
                      </span>,
                      <span key="e" className="map-muted">
                        {m.endpoint}
                      </span>,
                      <CtxTag key="c" n={m.contextWindow} />,
                    ]}
                    empty="暂无可用模型"
                  />
                </Section>
              </div>
            </div>
          </>
        )}

        {/* ---------------------------------------------------------------- Claude 桌面版 */}
        {tab === "desktop" && <DesktopCard baseUrl={baseUrl} apiKey={key} connected={deskOn} onConnected={setDeskOn} />}

        {/* ---------------------------------------------------------------- Codex / Gemini */}
        {tab === "codex" && codex && (
          <ClientCard glyph="X" color="#10a37f" title="Codex CLI" sub="~/.codex/config.toml" info={codex} onApply={() => apply(codex)} onRestore={() => restore(codex)}>
            <Section title="基本设置">
              <div className="cc-grid">
                <Row label="默认模型" help="config.toml 中的 model，可在 Codex 中通过 /model 切换">
                  <ModelInput value={o("codex").model} onChange={(v) => setO("codex", { model: v })} placeholder="例如 internal/sonnet" />
                  {config.server.accessKey && (
                    <div className="inline-note">
                      需在 shell 中设置 <code>KIKI_PROXY_KEY={config.server.accessKey}</code>
                    </div>
                  )}
                </Row>
              </div>
            </Section>
          </ClientCard>
        )}
        {tab === "gemini" && gemini && (
          <ClientCard glyph="G" color="#4f7cff" title="Gemini CLI" sub="~/.gemini/.env" info={gemini} onApply={() => apply(gemini)} onRestore={() => restore(gemini)}>
            <Section title="基本设置">
              <div className="cc-grid">
                <Row label="默认模型" help="GEMINI_MODEL">
                  <ModelInput value={o("gemini").model} onChange={(v) => setO("gemini", { model: v })} placeholder="例如 gm/gemini-2.5-pro" />
                </Row>
              </div>
            </Section>
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
                <span>按工具支持的协议填写地址，模型名为「前缀/模型 ID」</span>
              </div>
            </div>
            <table className="other-table">
              <tbody>
                {[
                  ["Anthropic", baseUrl, "/v1/messages"],
                  ["OpenAI", `${baseUrl}/v1`, "/chat/completions、/responses"],
                  ["Gemini", baseUrl, "/v1beta/models/{模型}:generateContent"],
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
                完成
              </button>
            }
          >
            <p className="muted-sm" style={{ marginTop: 0 }}>
              文件：<code>{report.file}</code>。原文件已备份，可随时恢复。重新打开 {report.name} 后生效。
            </p>
            {report.changes.length === 0 ? (
              <div className="banner ok">配置已是最新，无需改动</div>
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

const CLAUDE_TIERS = ["opus", "sonnet", "haiku"] as const;

function move<T>(list: T[], from: number, to: number): T[] {
  const next = [...list];
  const [x] = next.splice(from, 1);
  next.splice(to, 0, x);
  return next;
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
  children: ReactNode;
}) {
  const connected = props.connected ?? props.info?.connected;
  return (
    <div className="panel cc-card">
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

/** 卡片里的一节：标题 + 悬停说明 */
function Section(props: { title: string; desc?: ReactNode; badge?: ReactNode; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="cc-section">
      <div className="cc-section-head">
        <h4>
          {props.title}
          {props.desc && <Hint>{props.desc}</Hint>}
          {props.badge}
        </h4>
        {props.action}
      </div>
      {props.children}
    </section>
  );
}

function Row(props: { label: string; help?: ReactNode; children: ReactNode }) {
  return (
    <div className="cc-row">
      <label>
        {props.label}
        {props.help && <Hint>{props.help}</Hint>}
      </label>
      <div>{props.children}</div>
    </div>
  );
}

function SwitchRow(props: { on: boolean; onChange: (v: boolean) => void; title: string; desc: string }) {
  return (
    <label className="cc-switch">
      <span>
        {props.title}
        <Hint>{props.desc}</Hint>
      </span>
      <Switch on={props.on} onChange={props.onChange} />
    </label>
  );
}

/** 映射表：表头 + 每行若干单元格；给了 onReorder 就能拖动排序 */
function MapTable<T>(props: {
  cols: string;
  head: string[];
  items: T[];
  keyOf: (t: T) => string;
  cells: (t: T, i: number) => ReactNode[];
  onReorder?: (from: number, to: number) => void;
  empty?: ReactNode;
}) {
  const sortable = !!props.onReorder;
  const s = useSortable(props.onReorder ?? (() => {}));
  const cols = sortable ? `30px ${props.cols}` : props.cols;
  return (
    <div className="map-table">
      <div className="map-head" style={{ gridTemplateColumns: cols }}>
        {sortable && <span />}
        {props.head.map((h, i) => (
          <span key={i}>{h}</span>
        ))}
      </div>
      {props.items.length === 0 ? (
        <div className="map-empty">{props.empty ?? "暂无数据"}</div>
      ) : (
        <div ref={s.listRef} className={s.sorting ? "is-sorting" : ""}>
          {props.items.map((t, i) => {
            const r = s.row(i);
            return (
              <div
                key={props.keyOf(t)}
                {...(sortable ? r : {})}
                className={`map-row ${sortable ? r.className : ""}`}
                style={{ ...(sortable ? r.style : {}), gridTemplateColumns: cols }}
              >
                {sortable && <SortHandle {...s.handle(i)} />}
                {props.cells(t, i).map((c, j) => (
                  <div key={j} className="map-cell">
                    {c}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

type Role = "sonnet" | "opus" | "haiku" | "fable";
const ROLES: Role[] = ["sonnet", "opus", "haiku", "fable"];
const ROLE_NAME: Record<Role, string> = { sonnet: "Sonnet", opus: "Opus", haiku: "Haiku", fable: "Fable" };
/** 每个角色给桌面版的模型 ID（和 cc-switch 用的一样，桌面版都认识） */
const ROLE_ID: Record<Role, string> = {
  sonnet: "claude-sonnet-5",
  opus: "claude-opus-5",
  haiku: "claude-haiku-4-5",
  fable: "claude-fable-5",
};

function RoleChip(props: { role: Role }) {
  return (
    <span className={`role-chip role-${props.role}`}>
      <i />
      {ROLE_NAME[props.role]}
    </span>
  );
}

function RoleSelect(props: { value: Role; onChange: (r: Role) => void }) {
  return (
    <select
      className={`role-select role-${props.value}`}
      value={props.value}
      onChange={(e) => props.onChange(e.target.value as Role)}
      title="桌面版按该角色处理请求参数（如思考强度）"
    >
      {ROLES.map((r) => (
        <option key={r} value={r}>
          {ROLE_NAME[r]}
        </option>
      ))}
    </select>
  );
}

function CtxTag(props: { n: number }) {
  if (!props.n) return <span className="map-muted">—</span>;
  return <span className={`ctx-tag ${props.n >= 1_000_000 ? "is-1m" : ""}`}>{fmtContext(props.n)}</span>;
}

function roleOf(id: string): Role {
  return ROLES.find((r) => id.includes(r)) ?? "sonnet";
}

function guessRole(model: string): Role {
  const l = model.toLowerCase();
  if (l.includes("opus")) return "opus";
  if (l.includes("haiku") || l.includes("flash") || l.includes("mini")) return "haiku";
  return "sonnet";
}

type DeskRow = { key: string; model: string; role: Role; label: string };

/** 生成给桌面版的路由：同一角色的第二个起加 -r2、-r3…（界面上不展示） */
function buildRoutes(rows: DeskRow[], models: ListedModel[]): DesktopRoute[] {
  const used = new Map<Role, number>();
  return rows
    .filter((r) => r.model)
    .map((r) => {
      const n = (used.get(r.role) ?? 0) + 1;
      used.set(r.role, n);
      const m = models.find((x) => x.id === r.model);
      return {
        id: n === 1 ? ROLE_ID[r.role] : `${ROLE_ID[r.role]}-r${n}`,
        model: r.model,
        label: r.label.trim() || (m && m.displayName !== m.id ? m.displayName : r.model),
        // 是否 1M 以端点里模型的设置为准
        supports1m: (m?.contextWindow ?? 0) >= 1_000_000,
      };
    });
}

let rowSeq = 0;
const newKey = () => `row${++rowSeq}`;

/** Claude Code 桌面版：桌面版只接受 claude-sonnet-* 这类名字，给每个模型分配一个角色 ID，请求到网关再换回真实模型 */
function DesktopCard(props: { baseUrl: string; apiKey: string; connected: boolean; onConnected: (v: boolean) => void }) {
  const { models, config, saveConfig } = useApp();
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [file, setFile] = useState("");
  const [manual, setManual] = useState(false);
  const saved = config?.desktop?.routes ?? [];
  // 顺序就是桌面版菜单的顺序，第一个是默认模型
  const [rows, setRows] = useState<DeskRow[]>([]);
  const [inited, setInited] = useState(false);
  const modelOf = (id: string) => models.find((m) => m.id === id);
  useEffect(() => {
    if (inited || !models.length) return;
    setRows(
      saved.length
        ? saved.map((r) => {
            const m = modelOf(r.model);
            const auto = m && m.displayName !== m.id ? m.displayName : r.model;
            return { key: newKey(), model: r.model, role: roleOf(r.id), label: r.label === auto ? "" : r.label };
          })
        : models.slice(0, 4).map((m) => ({ key: newKey(), model: m.id, role: guessRole(m.id), label: "" })),
    );
    setInited(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [models, saved, inited]);
  const routes = buildRoutes(rows, models);
  const dirty = JSON.stringify(routes) !== JSON.stringify(saved);
  const setRow = (key: string, patch: Partial<DeskRow>) => setRows(rows.map((r) => (r.key === key ? { ...r, ...patch } : r)));

  const run = async (kind: "apply" | "export" | "restore") => {
    if (!config) return;
    setBusy(true);
    try {
      if (kind === "restore") {
        await api.restoreDesktopConfig();
        toast("已恢复桌面版原来的配置，重启桌面版后生效");
      } else {
        // 先保存路由表，网关才知道 claude-sonnet-5 这些名字对应哪个模型
        await saveConfig({ ...config, desktop: { routes } });
        if (kind === "apply") {
          await api.applyDesktopConfig();
          toast("已写入桌面版，完全退出（⌘Q）并重新打开后生效");
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
      sub="第三方推理配置库"
      connected={props.connected}
      canRestore={props.connected}
      busy={busy || !routes.length}
      applyText={props.connected ? "更新配置" : "一键接入"}
      onApply={() => run("apply")}
      onRestore={() => run("restore")}
    >
      <Section
        title="模型映射"
        desc="拖动调整菜单顺序，首项为默认模型。桌面版仅接受 claude-sonnet-* 等模型 ID，模型角色决定请求参数（如思考强度），请求仍发送至实际请求模型"
        badge={dirty && props.connected ? <span className="sync-badge">未同步</span> : null}
        action={
          <button className="btn btn-sm btn-soft" onClick={() => setRows([...rows, { key: newKey(), model: "", role: "sonnet", label: "" }])}>
            <Plus size={14} />
            添加模型
          </button>
        }
      >
        <MapTable
          cols="150px minmax(0,1fr) minmax(0,1.3fr) 72px 36px"
          head={["模型角色", "菜单显示名", "实际请求模型", "上下文", ""]}
          items={rows}
          keyOf={(r) => r.key}
          onReorder={(from, to) => setRows(move(rows, from, to))}
          empty="尚未添加模型"
          cells={(r, i) => {
            const m = modelOf(r.model);
            return [
              <div key="role" className="role-cell">
                <RoleSelect value={r.role} onChange={(role) => setRow(r.key, { role })} />
                {i === 0 && <span className="default-tag">默认</span>}
              </div>,
              <input
                key="label"
                className="input"
                value={r.label}
                placeholder={m && m.displayName !== m.id ? m.displayName : r.model || "菜单中显示的名称"}
                onChange={(e) => setRow(r.key, { label: e.target.value })}
              />,
              <ModelInput
                key="model"
                value={r.model}
                onChange={(v) => {
                  const nm = modelOf(v);
                  setRow(r.key, { model: v, ...(nm && !r.model ? { role: guessRole(v) } : {}) });
                }}
                placeholder="选择模型"
              />,
              <CtxTag key="c" n={m?.contextWindow ?? 0} />,
              <button key="del" className="icon-btn danger" title="移除" onClick={() => setRows(rows.filter((x) => x.key !== r.key))}>
                <Trash2 size={15} />
              </button>,
            ];
          }}
        />
      </Section>

      <Section title="接入步骤" desc="启用网关后，桌面版的云端环境与 Remote Control 不可用">
        <div className="dsteps">
          <div className="dstep">
            <em>1</em>
            <span>开启开发者模式</span>
            <Hint>
              Help → Troubleshooting → <b>Enable Developer Mode</b>
            </Hint>
          </div>
          <i className="dstep-line" />
          <div className="dstep">
            <em>2</em>
            <span>{props.connected ? "更新配置" : "一键接入"}</span>
          </div>
          <i className="dstep-line" />
          <div className="dstep">
            <em>3</em>
            <span>重启桌面版</span>
            <Hint>⌘Q 完全退出后重新打开</Hint>
          </div>
          <button className="btn btn-xs btn-ghost dsteps-manual" onClick={() => setManual(true)}>
            <FileJson size={13} />
            手动导入
          </button>
        </div>
      </Section>
      {manual && (
        <Modal title="手动导入配置" icon={<FileJson size={18} />} onClose={() => setManual(false)}>
          <ol className="desk-ol">
            <li>生成配置文件</li>
            <li>
              桌面版中选择 Developer → <b>Configure Third-Party Inference…</b> → <b>Import configuration…</b>
            </li>
            <li>导入后点击 Apply Changes 并 Relaunch</li>
          </ol>
          <div className="desk-export">
            <button className="btn btn-sm btn-primary" onClick={() => run("export")} disabled={!routes.length || busy}>
              <Download size={14} />
              生成配置文件
            </button>
            {file && (
              <span className="desk-file">
                <Check size={13} /> <code>{file}</code>
              </span>
            )}
          </div>
          <table className="kv-table">
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
        </Modal>
      )}
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
  if (!r.issues.length && !touched) return null;
  return (
    <div className={`cc-check ${r.issues.length ? "bad" : "ok"}`}>
      <div className="cc-check-head">
        <b>
          {r.issues.length ? `发现 ${r.issues.length} 个配置问题` : "配置文件已被修改"}
          <Hint>实时读取 {r.files.map((f) => f.path.replace(/^\/Users\/[^/]+/, "~")).join("、") || "（未找到配置文件）"}</Hint>
        </b>
      </div>
      {touched && <div className="cc-check-warn">接入后被其他程序修改（如 cc-switch），请退出该程序后重新「更新配置」</div>}
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
