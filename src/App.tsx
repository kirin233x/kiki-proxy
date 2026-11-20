import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import {
  BarChart3,
  GitBranch,
  Globe,
  LayoutDashboard,
  Pause,
  Play,
  Plus,
  ScrollText,
  Server,
  Settings as SettingsIcon,
  TerminalSquare,
} from "lucide-react";
import { api, Config, listen, ListedModel, PluginInfo, PromptForm, ServerStatus } from "./api";
import { CopyButton, StatusDot, ToastProvider, useToast } from "./components/ui";
import PromptDialog from "./components/PromptDialog";
import Overview from "./pages/Overview";
import Endpoints from "./pages/Endpoints";
import EndpointEditor from "./pages/EndpointEditor";
import Routing from "./pages/Routing";
import Proxies from "./pages/Proxies";
import Logs from "./pages/Logs";
import StatsPage from "./pages/Stats";
import Clients from "./pages/Clients";
import SettingsPage from "./pages/Settings";
import { protocolColor } from "./utils";

export type Route =
  | { name: "overview" }
  | { name: "endpoints" }
  | { name: "endpoint"; id: string | null }
  | { name: "routing" }
  | { name: "proxies" }
  | { name: "logs"; endpoint?: string }
  | { name: "stats" }
  | { name: "clients" }
  | { name: "settings" };

interface AppCtx {
  config: Config;
  /** 保存整份配置；失败时弹出错误并抛出 */
  saveConfig: (c: Config) => Promise<void>;
  status: ServerStatus;
  setStatus: (s: ServerStatus) => void;
  plugins: PluginInfo[];
  models: ListedModel[];
  route: Route;
  go: (r: Route) => void;
  dark: boolean;
  baseUrl: string;
}

const Ctx = createContext<AppCtx>(null as unknown as AppCtx);
export const useApp = () => useContext(Ctx);

function useSystemDark() {
  const [dark, setDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const fn = () => setDark(mq.matches);
    mq.addEventListener("change", fn);
    return () => mq.removeEventListener("change", fn);
  }, []);
  return dark;
}

function Shell() {
  const toast = useToast();
  const [config, setConfig] = useState<Config | null>(null);
  const [status, setStatus] = useState<ServerStatus>({ running: false, port: 0, error: null, startedAt: 0 });
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [models, setModels] = useState<ListedModel[]>([]);
  const [route, setRoute] = useState<Route>({ name: "overview" });
  const [prompt, setPrompt] = useState<{ id: string; form: PromptForm } | null>(null);
  const systemDark = useSystemDark();
  const theme = config?.ui.theme ?? "auto";
  const dark = theme === "dark" || (theme === "auto" && systemDark);

  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);

  useEffect(() => {
    api.getConfig().then(setConfig).catch((e) => toast(String(e), true));
    api.getStatus().then(setStatus);
    api.listPlugins().then(setPlugins);
    api.listModels().then(setModels);
  }, [toast]);

  // 后端事件：网关状态、插件弹窗、通知
  useEffect(() => {
    const offs: Promise<() => void>[] = [
      listen<ServerStatus | null>("status", (s) => s && setStatus(s)),
      listen<{ id: string; form: PromptForm }>("plugin-prompt", setPrompt),
      listen<{ title: string; body: string }>("notify", (n) => toast(n.body)),
    ];
    return () => offs.forEach((p) => p.then((f) => f()));
  }, [toast]);

  const saveConfig = useCallback(
    async (c: Config) => {
      try {
        await api.saveConfig(c);
        setConfig(c);
        api.listModels().then(setModels);
        api.getStatus().then(setStatus);
      } catch (e) {
        toast(String(e), true);
        throw e;
      }
    },
    [toast],
  );

  const go = useCallback((r: Route) => setRoute(r), []);

  const ctx = useMemo<AppCtx | null>(
    () =>
      config && {
        config,
        saveConfig,
        status,
        setStatus,
        plugins,
        models,
        route,
        go,
        dark,
        baseUrl: `http://127.0.0.1:${status.port || config.server.port}`,
      },
    [config, saveConfig, status, plugins, models, route, go, dark],
  );

  if (!ctx) return <div className="app" />;

  const nav = [
    { key: "overview", label: "概览", icon: <LayoutDashboard size={18} />, route: { name: "overview" } as Route },
    { key: "endpoints", label: "端点", icon: <Server size={18} />, route: { name: "endpoints" } as Route },
    { key: "routing", label: "路由与切换", icon: <GitBranch size={18} />, route: { name: "routing" } as Route },
    { key: "proxies", label: "出口代理", icon: <Globe size={18} />, route: { name: "proxies" } as Route },
    { key: "logs", label: "请求日志", icon: <ScrollText size={18} />, route: { name: "logs" } as Route },
    { key: "stats", label: "用量统计", icon: <BarChart3 size={18} />, route: { name: "stats" } as Route },
    { key: "clients", label: "客户端接入", icon: <TerminalSquare size={18} />, route: { name: "clients" } as Route },
    { key: "settings", label: "设置", icon: <SettingsIcon size={18} />, route: { name: "settings" } as Route },
  ];
  const activeKey = route.name === "endpoint" ? "endpoints" : route.name;

  const toggleServer = async () => {
    try {
      setStatus(status.running ? await api.stopServer() : await api.startServer());
    } catch (e) {
      toast(String(e), true);
      api.getStatus().then(setStatus);
    }
  };

  return (
    <Ctx.Provider value={ctx}>
      <div className="app">
        <aside className="sidebar">
          <div className="drag-region" data-tauri-drag-region />
          <div className="brand" data-tauri-drag-region>
            <div className="brand-logo">
              <svg width="20" height="20" viewBox="0 0 44 44" fill="none" stroke="white" strokeWidth="3.6" strokeLinecap="round">
                <path d="M8 22 H17 M17 22 C22 22 22 12 28 12 H31 M17 22 H31 M17 22 C22 22 22 32 28 32 H31" />
                <circle cx="8" cy="22" r="4" fill="white" stroke="none" />
                <circle cx="34" cy="12" r="3.4" fill="white" stroke="none" />
                <circle cx="34" cy="22" r="3.4" fill="#ffd1e8" stroke="none" />
                <circle cx="34" cy="32" r="3.4" fill="white" stroke="none" />
              </svg>
            </div>
            <div>
              <div className="brand-name">kiki-proxy</div>
              <div className="brand-sub">一个入口 · 多路分流</div>
            </div>
          </div>
          <nav className="nav">
            {nav.map((n) => (
              <button key={n.key} className={`nav-item ${activeKey === n.key ? "active" : ""}`} onClick={() => go(n.route)}>
                {n.icon}
                {n.label}
              </button>
            ))}
          </nav>
          <div className="sidebar-section">端点</div>
          <div className="sidebar-decks">
            {config!.endpoints.map((e) => (
              <button
                key={e.id}
                className={`sidebar-deck ${route.name === "endpoint" && route.id === e.id ? "active" : ""}`}
                onClick={() => go({ name: "endpoint", id: e.id })}
                style={{ opacity: e.enabled ? 1 : 0.5 }}
              >
                <span className="dot" style={{ background: protocolColor[e.protocol] }} />
                <span className="name">{e.name}</span>
                <span className="num">{e.id}/</span>
              </button>
            ))}
            <button className="sidebar-deck" style={{ color: "var(--muted)" }} onClick={() => go({ name: "endpoint", id: null })}>
              <Plus size={14} />
              <span className="name">添加端点</span>
            </button>
          </div>
          <div className="sidebar-footer">
            <div className={`gw-card ${status.running ? "on" : ""}`}>
              <div className="gw-row">
                <StatusDot tone={status.running ? "ok" : status.error ? "err" : "off"} />
                <div className="gw-text">
                  <b>{status.running ? "网关运行中" : status.error ? "网关未启动" : "网关已暂停"}</b>
                  <span title={status.error ?? ""}>{status.error ? status.error : `127.0.0.1:${status.port || config!.server.port}`}</span>
                </div>
                <button className="icon-btn" onClick={toggleServer} title={status.running ? "暂停" : "启动"}>
                  {status.running ? <Pause size={16} /> : <Play size={16} />}
                </button>
              </div>
              {status.running && (
                <div className="gw-url">
                  <code>{ctx.baseUrl}</code>
                  <CopyButton text={ctx.baseUrl} small />
                </div>
              )}
            </div>
          </div>
        </aside>
        <main className="main">
          <div className="main-drag" data-tauri-drag-region />
          {route.name === "overview" && <Overview />}
          {route.name === "endpoints" && <Endpoints />}
          {route.name === "endpoint" && <EndpointEditor key={route.id ?? "new"} id={route.id} />}
          {route.name === "routing" && <Routing />}
          {route.name === "proxies" && <Proxies />}
          {route.name === "logs" && <Logs initialEndpoint={route.endpoint} />}
          {route.name === "stats" && <StatsPage />}
          {route.name === "clients" && <Clients />}
          {route.name === "settings" && <SettingsPage />}
        </main>
      </div>
      {prompt && (
        <PromptDialog
          form={prompt.form}
          onSubmit={(v) => {
            api.answerPrompt(prompt.id, v);
            setPrompt(null);
          }}
          onCancel={() => {
            api.answerPrompt(prompt.id, null);
            setPrompt(null);
          }}
        />
      )}
    </Ctx.Provider>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}
