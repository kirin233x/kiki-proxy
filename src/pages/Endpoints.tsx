import { useState } from "react";
import { Copy, MoreHorizontal, Pencil, Plus, Power, Server, Trash2, Zap } from "lucide-react";
import { api, EndpointConfig, protocolLabel, TestResult } from "../api";
import { useApp } from "../App";
import { Confirm, Empty, Menu, StatusDot, Switch, useToast } from "../components/ui";
import { fmtMs, protocolColor } from "../utils";

export default function Endpoints() {
  const { config, saveConfig, go } = useApp();
  const toast = useToast();
  const [menu, setMenu] = useState<{ el: HTMLElement; ep: EndpointConfig } | null>(null);
  const [del, setDel] = useState<EndpointConfig | null>(null);
  const [tests, setTests] = useState<Record<string, TestResult | "busy">>({});

  const update = (id: string, patch: Partial<EndpointConfig>) =>
    saveConfig({ ...config, endpoints: config.endpoints.map((e) => (e.id === id ? { ...e, ...patch } : e)) });

  const test = async (e: EndpointConfig) => {
    setTests((t) => ({ ...t, [e.id]: "busy" }));
    const r = await api.testEndpoint(e.id);
    setTests((t) => ({ ...t, [e.id]: r }));
  };

  const duplicate = (e: EndpointConfig) => {
    let id = `${e.id}-2`;
    let n = 2;
    while (config.endpoints.some((x) => x.id === id)) id = `${e.id}-${++n}`;
    saveConfig({ ...config, endpoints: [...config.endpoints, { ...structuredClone(e), id, name: `${e.name} 副本` }] }).then(() =>
      toast(`已复制为「${e.name} 副本」，API Key 需要重新填写`),
    );
  };

  return (
    <div className="page">
      <div className="page-inner">
        <div className="page-header">
          <div>
            <h1 className="page-title">端点</h1>
            <div className="page-subtitle">每个端点有一个前缀，客户端用「前缀/模型名」访问，例如 internal/sonnet</div>
          </div>
          <div className="header-actions">
            <button className="btn btn-primary" onClick={() => go({ name: "endpoint", id: null })}>
              <Plus size={16} />
              添加端点
            </button>
          </div>
        </div>

        {config.endpoints.length === 0 ? (
          <div className="panel">
            <Empty icon={<Server size={22} />} title="还没有端点" action={<button className="btn btn-primary" onClick={() => go({ name: "endpoint", id: null })}>添加端点</button>}>
              官方 API、中转站、公司内部服务都可以作为端点
            </Empty>
          </div>
        ) : (
          <div className="ep-list">
            {config.endpoints.map((e) => {
              const t = tests[e.id];
              const proxy =
                e.proxy === "direct" ? "直连" : e.proxy === "system" ? "跟随系统代理" : config.proxies.find((p) => p.id === e.proxy)?.name ?? e.proxy;
              return (
                <div key={e.id} className={`ep-row panel ${e.enabled ? "" : "off"}`}>
                  <div className="ep-row-main" onClick={() => go({ name: "endpoint", id: e.id })}>
                    <div className="ep-row-head">
                      <span className="ep-glyph" style={{ background: protocolColor[e.protocol] }}>
                        {e.name.slice(0, 1).toUpperCase()}
                      </span>
                      <div style={{ minWidth: 0 }}>
                        <div className="ep-row-name">
                          {e.name}
                          <code className="prefix">{e.id}/</code>
                        </div>
                        <div className="ep-row-url">{e.baseUrl}</div>
                      </div>
                    </div>
                    <div className="ep-row-tags">
                      <span className="proto-tag" style={{ ["--c" as string]: protocolColor[e.protocol], ["--cb" as string]: protocolColor[e.protocol] + "1f" }}>
                        {protocolLabel(e.protocol)}
                      </span>
                      <span className="tag">出口：{proxy}</span>
                      {e.plugin && <span className="tag accent">插件：{e.plugin}</span>}
                      {e.fallbacks.length > 0 && <span className="tag">故障时 → {e.fallbacks.join("、")}</span>}
                    </div>
                    <div className="ep-models">
                      {e.models.map((m) => (
                        <span key={m.name} className={`model-chip ${m.enabled ? "" : "off"}`}>
                          {m.name}
                          {m.upstream && m.upstream !== m.name && <em>→ {m.upstream}</em>}
                        </span>
                      ))}
                      {e.models.length === 0 && <span className="muted-sm">还没有模型（仍然可以用「{e.id}/任意模型名」直接透传）</span>}
                    </div>
                  </div>
                  <div className="ep-row-side">
                    <Switch on={e.enabled} onChange={(v) => update(e.id, { enabled: v })} />
                    <button className="btn btn-sm" onClick={() => test(e)} disabled={t === "busy"}>
                      <Zap size={14} />
                      {t === "busy" ? "测试中…" : "测试"}
                    </button>
                    {t && t !== "busy" && (
                      <div className={`test-result ${t.ok ? "ok" : "err"}`} title={t.message}>
                        <StatusDot tone={t.ok ? "ok" : "err"} />
                        {t.ok ? fmtMs(t.latencyMs) : t.message.slice(0, 40)}
                      </div>
                    )}
                    <button className="icon-btn" onClick={(ev) => setMenu({ el: ev.currentTarget, ep: e })}>
                      <MoreHorizontal size={18} />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
      {menu && (
        <Menu
          anchor={menu.el}
          onClose={() => setMenu(null)}
          items={[
            { label: "编辑", icon: <Pencil size={15} />, onClick: () => go({ name: "endpoint", id: menu.ep.id }) },
            { label: "复制", icon: <Copy size={15} />, onClick: () => duplicate(menu.ep) },
            { label: menu.ep.enabled ? "停用" : "启用", icon: <Power size={15} />, onClick: () => update(menu.ep.id, { enabled: !menu.ep.enabled }) },
            "sep",
            { label: "删除", icon: <Trash2 size={15} />, danger: true, onClick: () => setDel(menu.ep) },
          ]}
        />
      )}
      {del && (
        <Confirm
          title={`删除「${del.name}」？`}
          message={<>使用 <code>{del.id}/</code> 前缀的请求将无法路由。钥匙串里的 API Key 不会自动删除。</>}
          danger
          confirmText="删除"
          onClose={() => setDel(null)}
          onConfirm={() => saveConfig({ ...config, endpoints: config.endpoints.filter((x) => x.id !== del.id) })}
        />
      )}
    </div>
  );
}
