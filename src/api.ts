// 与 Rust 后端通信的类型与方法。
// 在浏览器里直接 `npm run dev` 时（没有 Tauri 环境），自动使用 mock 数据，方便调试界面。

export type Protocol = "anthropic" | "openai_chat" | "openai_responses" | "gemini";
export type AuthMode = "auto" | "bearer" | "x_api_key" | "goog_api_key" | "none";
export type ProxyKind = "http" | "socks5";

export const PROTOCOLS: { value: Protocol; label: string; hint: string }[] = [
  { value: "anthropic", label: "Anthropic", hint: "/v1/messages · Claude Code" },
  { value: "openai_chat", label: "OpenAI Chat", hint: "/v1/chat/completions · 大部分兼容服务" },
  { value: "openai_responses", label: "OpenAI Responses", hint: "/v1/responses · Codex" },
  { value: "gemini", label: "Gemini", hint: "/v1beta/models · Gemini CLI" },
];

export const protocolLabel = (p: Protocol | string) => PROTOCOLS.find((x) => x.value === p)?.label ?? p;

export interface Compat {
  sendReasoningContent: boolean;
  useMaxCompletionTokens: boolean;
  sendReasoningEffort: boolean;
  dropSamplingParams: boolean;
}

export interface Price {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

export interface ModelConfig {
  name: string;
  upstream: string;
  displayName: string;
  enabled: boolean;
  vision: boolean;
  /** 上下文窗口（token），0 表示未知 */
  contextWindow: number;
  price: Price;
  fallbacks: string[];
}

export interface EndpointConfig {
  id: string;
  name: string;
  enabled: boolean;
  protocol: Protocol;
  baseUrl: string;
  auth: AuthMode;
  proxy: string;
  /** 由 kiki-proxy 启动并守护的本地程序（比如公司内部服务的命令行代理） */
  process: ProcessConfig;
  headers: Record<string, string>;
  timeoutSecs: number;
  compat: Compat;
  models: ModelConfig[];
  fallbacks: string[];
  note: string;
}

export interface ProcessConfig {
  enabled: boolean;
  /** 启动命令，和在终端里敲的一样 */
  command: string;
  /** 工作目录，可以用 ~ */
  workdir: string;
  /** 额外的环境变量，每行一个 KEY=VALUE */
  env: string;
  /** 本地程序自己访问外网：空 = 跟随终端；direct = 去掉代理变量；其他 = 出口代理 id */
  network: string;
}

export interface ProcessStatus {
  state: "running" | "starting" | "restarting" | "stopped" | "failed";
  pid: number | null;
  startedAt: number;
  restarts: number;
  lastExit: string;
  command: string;
  /** 手动停止了，或者连续失败后放弃了，需要手动启动 */
  held: boolean;
  /** 网关已暂停 */
  paused: boolean;
  logs: string[];
}

export interface ProxyConfig {
  id: string;
  name: string;
  kind: ProxyKind;
  url: string;
  username: string;
}

export interface RuleConfig {
  id: string;
  name: string;
  enabled: boolean;
  pattern: string;
  client: Protocol | null;
  target: string;
  proxy: string;
}

export interface Config {
  server: { port: number; accessKey: string };
  routing: { defaultModel: string; claudeAlias: boolean; modelOrder: string[] };
  failover: { onConnectError: boolean; onTimeout: boolean; on5xx: boolean; on429: boolean; extraStatus: number[] };
  logging: { bodyRetentionHours: number; statsRetentionDays: number; maxBodyKb: number };
  ui: { theme: "auto" | "light" | "dark"; probeUrl: string };
  pricing: { url: string; proxy: string };
  proxies: ProxyConfig[];
  endpoints: EndpointConfig[];
  rules: RuleConfig[];
  /** Claude 桌面版的模型路由 */
  desktop: { routes: DesktopRoute[] };
}

export interface ServerStatus {
  running: boolean;
  port: number;
  error: string | null;
  startedAt: number;
}

export interface TestStep {
  kind: "network" | "auth" | "model";
  label: string;
  status: "ok" | "fail" | "skip";
  detail: string;
  ms: number;
}

export interface TestReport {
  ok: boolean;
  steps: TestStep[];
}

export interface TestResult {
  ok: boolean;
  status: number;
  latencyMs: number;
  message: string;
}

export interface Usage {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

export interface Attempt {
  gatewayModel: string;
  endpoint: string;
  status: number;
  error: string;
  durationMs: number;
}

export interface LogRecord {
  id: string;
  ts: number;
  clientProtocol: string;
  path: string;
  requestedModel: string;
  gatewayModel: string;
  endpointId: string;
  endpointName: string;
  upstreamModel: string;
  upstreamProtocol: string;
  proxy: string;
  rule: string;
  status: number;
  error: string;
  durationMs: number;
  ttfbMs: number;
  stream: boolean;
  usage: Usage;
  cost: number;
  attempts: Attempt[];
  pending?: boolean;
}

export interface LogBodies {
  clientRequest: string;
  clientHeaders: string;
  upstreamUrl: string;
  upstreamRequest: string;
  upstreamHeaders: string;
  upstreamStatus: number;
  upstreamResponse: string;
  clientResponse: string;
}

export interface LogQuery {
  q?: string;
  endpoint?: string;
  status?: "all" | "ok" | "error";
  before?: number;
  limit?: number;
}

export interface UsageRow {
  key: string;
  label: string;
  requests: number;
  errors: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
  avgMs: number;
}

export interface DailyRow {
  day: string;
  requests: number;
  errors: number;
  input: number;
  output: number;
  cost: number;
}

export interface Stats {
  total: UsageRow;
  byModel: UsageRow[];
  byEndpoint: UsageRow[];
  daily: DailyRow[];
}

export interface ClientInfo {
  id: "claude" | "codex" | "gemini";
  name: string;
  path: string;
  connected: boolean;
  hasBackup: boolean;
  model: string;
  maxContextTokens: number;
  /** Claude Code：当前的模型映射 */
  tiers?: TierInfo[];
  subagentModel?: string;
}

export interface TierInfo {
  role: "opus" | "sonnet" | "haiku";
  model: string;
  label: string;
  supports1m: boolean;
}

export interface ApplyReport {
  file: string;
  changes: string[];
}

export interface ConfigIssue {
  file: string;
  key: string;
  value: string;
  problem: string;
}

export interface ClaudeInspection {
  files: { path: string; modified: number; entries: [string, string][] }[];
  issues: ConfigIssue[];
}

/** 桌面版只接受 claude-sonnet-* 这类名字，网关按这张表换回真实模型 */
export interface DesktopRoute {
  /** 给桌面版看的名字，如 claude-sonnet-5、claude-sonnet-5-r2 */
  id: string;
  /** 实际使用的网关模型 */
  model: string;
  /** 桌面版菜单里显示的名字，留空显示网关模型名 */
  label: string;
  supports1m: boolean;
}

export interface ApplyOptions {
  model: string;
  /** /model 里 Opus 档，留空跟默认模型一致 */
  opusModel: string;
  /** /model 里 Sonnet 档，留空跟默认模型一致 */
  sonnetModel: string;
  /** Haiku 档（后台小任务），留空跟默认模型一致 */
  smallModel: string;
  discovery: boolean;
  maxContextTokens: number;
  /** 所有 subagent 固定用的模型，留空由 Claude Code 决定 */
  subagentModel: string;
  /** 各角色在 /model 菜单里的显示名，留空用模型显示名 */
  tierLabels: Record<string, string>;
  /** 各角色是否声明支持 1M */
  tier1m: Record<string, boolean>;
}

export interface ListedModel {
  id: string;
  displayName: string;
  endpoint: string;
  vision: boolean;
  contextWindow: number;
}

/** 价格表里查到的模型信息（价格单位：美元 / 百万 token） */
export interface ModelInfo {
  matched: string;
  provider: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  contextWindow: number;
  maxOutput: number;
  vision: boolean;
}

export interface PricingStatus {
  count: number;
  updatedAt: number;
  url: string;
}

export interface RouteHop {
  gatewayModel: string;
  endpointId: string;
  endpointName: string;
  protocol: Protocol;
  upstreamModel: string;
  proxy: string;
  rule: string;
}

// ------------------------------------------------------------------ 钥匙串里的密钥名（与 Rust 端 secrets::keys 一致）

export const secretKeys = {
  apiKey: (ep: string) => `endpoint/${ep}/api_key`,
  proxyPassword: (id: string) => `proxy/${id}/password`,
};

// ------------------------------------------------------------------ 调用

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
let invokeImpl: Promise<InvokeFn> | null = null;

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!invokeImpl) {
    invokeImpl = isTauri
      ? import("@tauri-apps/api/core").then((m) => m.invoke as InvokeFn)
      : import("./mock").then((m) => m.mockInvoke as InvokeFn);
  }
  return invokeImpl.then((f) => f<T>(cmd, args));
}

export const api = {
  getConfig: () => invoke<Config>("get_config"),
  saveConfig: (config: Config) => invoke<void>("save_config", { config }),
  getStatus: () => invoke<ServerStatus>("get_status"),
  appInfo: () => invoke<{ version: string; gitHash: string; buildTime: number }>("app_info"),
  startServer: () => invoke<ServerStatus>("start_server"),
  stopServer: () => invoke<ServerStatus>("stop_server"),
  exportConfig: () => invoke<string>("export_config"),
  importConfig: (toml: string) => invoke<Config>("import_config", { toml }),
  revealConfig: () => invoke<void>("reveal_config"),
  listModels: () => invoke<ListedModel[]>("list_models"),
  explainRoute: (model: string, client: Protocol) => invoke<RouteHop[]>("explain_route", { model, client }),

  secretFlags: (keys: string[]) => invoke<Record<string, boolean>>("secret_flags", { keys }),
  setSecret: (key: string, value: string) => invoke<void>("set_secret", { key, value }),

  processStatus: (id: string) => invoke<ProcessStatus | null>("process_status", { id }),
  restartProcess: (id: string) => invoke<void>("restart_process", { id }),
  startProcess: (id: string) => invoke<void>("start_process", { id }),
  stopProcess: (id: string) => invoke<void>("stop_process", { id }),

  testEndpoint: (id: string, model?: string) => invoke<TestResult>("test_endpoint", { id, model: model ?? null }),
  testProxy: (id: string) => invoke<TestResult>("test_proxy", { id }),
  testEndpointReport: (id: string) => invoke<TestReport>("test_endpoint_report", { id }),
  /** 分步测试的单独一步：network / auth / model */
  testStep: (id: string, step: "network" | "auth" | "model", model?: string) =>
    invoke<TestStep>("test_step", { id, step, model: model ?? null }),

  pricingStatus: () => invoke<PricingStatus>("pricing_status"),
  refreshPricing: () => invoke<PricingStatus>("refresh_pricing"),
  lookupModels: (models: string[], baseUrl: string, protocol: Protocol) =>
    invoke<(ModelInfo | null)[]>("lookup_models", { models, baseUrl, protocol }),
  listUpstreamModels: (id: string) => invoke<string[]>("list_upstream_models", { id }),

  listLogs: (query: LogQuery) => invoke<LogRecord[]>("list_logs", { query }),
  getLog: (id: string) => invoke<{ record: LogRecord; bodies: LogBodies | null } | null>("get_log", { id }),
  clearLogs: () => invoke<void>("clear_logs"),
  /** 只清空失败的请求，返回删除条数 */
  clearFailedLogs: () => invoke<number>("clear_failed_logs"),
  getStats: (days: number) => invoke<Stats>("get_stats", { days, tzOffsetMin: -new Date().getTimezoneOffset() }),

  listClients: () => invoke<ClientInfo[]>("list_clients"),
  applyClient: (client: string, options: ApplyOptions) => invoke<ApplyReport>("apply_client", { client, options }),
  restoreClient: (client: string) => invoke<void>("restore_client", { client }),
  /** 生成 Claude 桌面版可导入的配置文件，返回文件路径 */
  /** 按已保存的桌面版模型路由生成配置文件 */
  exportDesktopConfig: () => invoke<string>("export_desktop_config"),
  /** 直接写进桌面版配置库并设为当前 */
  applyDesktopConfig: () => invoke<string>("apply_desktop_config"),
  restoreDesktopConfig: () => invoke<void>("restore_desktop_config"),
  desktopConnected: () => invoke<boolean>("desktop_connected"),
  /** 检查 Claude Code 实际会读到的配置 */
  inspectClaude: () => invoke<ClaudeInspection>("inspect_claude"),

  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
};

/** 订阅后端事件（实时日志、插件弹窗等）；浏览器里走 mock */
export async function listen<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  if (isTauri) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<T>(event, (e) => cb(e.payload));
  }
  const { mockListen } = await import("./mock");
  return mockListen(event, cb as (p: unknown) => void);
}

// ------------------------------------------------------------------ 默认值

export function newEndpoint(): EndpointConfig {
  return {
    id: "",
    name: "",
    enabled: true,
    protocol: "anthropic",
    baseUrl: "",
    auth: "auto",
    proxy: "system",
    process: { enabled: false, command: "", workdir: "", env: "", network: "" },
    headers: {},
    timeoutSecs: 300,
    compat: { sendReasoningContent: false, useMaxCompletionTokens: false, sendReasoningEffort: false, dropSamplingParams: false },
    models: [],
    fallbacks: [],
    note: "",
  };
}

export function newModel(): ModelConfig {
  return {
    name: "",
    upstream: "",
    displayName: "",
    enabled: true,
    vision: true,
    contextWindow: 0,
    price: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    fallbacks: [],
  };
}

export function uid(prefix: string) {
  return `${prefix}${Math.random().toString(36).slice(2, 8)}`;
}

/** 把模型信息写进模型配置（只覆盖查到的字段） */
export function applyModelInfo(m: ModelConfig, info: ModelInfo): ModelConfig {
  return {
    ...m,
    contextWindow: m.contextWindow >= 1_000_000 ? m.contextWindow : info.contextWindow || m.contextWindow,
    vision: info.vision,
    price: { input: info.input, output: info.output, cacheRead: info.cacheRead, cacheWrite: info.cacheWrite },
  };
}

export function fmtContext(n: number): string {
  if (!n) return "未知";
  if (n >= 1_000_000) return `${+(n / 1_000_000).toFixed(1)}M`;
  // 131072 这种按 1024 算的写成 128K
  return n % 1024 === 0 ? `${n / 1024}K` : `${Math.round(n / 1000)}K`;
}
