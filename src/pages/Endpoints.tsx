import { useEffect, useState } from "react";
import { Copy, MoreHorizontal, Pencil, Plus, ScrollText, Server, Trash2, Zap } from "lucide-react";
import { api, EndpointConfig, fmtContext, protocolLabel, UsageRow } from "../api";
import { useApp } from "../App";
import TestDialog from "../components/TestDialog";
import { ProcessBadge } from "../components/ProcessPanel";
import { Confirm, Empty, Menu, Switch, useToast } from "../components/ui";
import { fmtCost, fmtNum, protocolColor } from "../utils";

export default function Endpoints() {
  const { config, saveConfig, go } = useApp();
  const toast = useToast();
  const [menu, setMenu] = useState<{ el: HTMLElement; ep: EndpointConfig } | null>(null);
  const [del, setDel] = useState<EndpointConfig | null>(null);
  const [testing, setTesting] = useState<EndpointConfig | null>(null);
  const [usage, setUsage] = useState<Record<string, UsageRow>>({});

  useEffect(() => {
    api.getStats(1).then((s) => setUsage(Object.fromEntries(s.byEndpoint.map((r) => [r.key, r]))));
  }, []);

  const update = (id: string, patch: Partial<EndpointConfig>) =>
    saveConfig({ ...config, endpoints: config.endpoints.map((e) => (e.id === id ? { ...e, ...patch } : e)) });

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
            <div className="page-subtitle">客户端用「前缀/模型名」访问，例如 {config.endpoints[0] ? `${config.endpoints[0].id}/${config.endpoints[0].models[0]?.name ?? "模型"}` : "internal/sonnet"}</div>
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
          <div className="ep-cards">
            {config.endpoints.map((e) => {
              const u = usage[e.id];
              const typeLabel = protocolLabel(e.protocol);
              const color = protocolColor[e.protocol];
              return (
                <div key={e.id} className={`ep2 panel ${e.enabled ? "" : "off"}`}>
                  <div className="ep2-head">
                    <span className="ep-glyph" style={{ background: color }}>
                      {e.name.slice(0, 1).toUpperCase()}
                    </span>
                    <div className="ep2-title" onClick={() => go({ name: "endpoint", id: e.id })}>
                      <b>
                        {e.name}
                        <code className="prefix">{e.id}/</code>
                      </b>
                      <span>
                        {typeLabel}
                        {e.process.enabled && e.enabled && (
                          <>
                            {" · "}
                            <ProcessBadge endpointId={e.id} />
                          </>
                        )}
                      </span>
                    </div>
                    <label className="ep2-toggle" title={e.enabled ? "点击停用：停用后不再转发到这个端点，也不会启动它的本地程序" : "点击启用"}>
                      <span>{e.enabled ? "已启用" : "已停用"}</span>
                      <Switch on={e.enabled} onChange={(v) => update(e.id, { enabled: v })} />
                    </label>
                  </div>

                  <div className="ep2-usage">
                    <div>
                      <b>{fmtNum(u?.requests ?? 0)}</b>
                      <span>今日请求</span>
                    </div>
                    <div>
                      <b className={u && u.errors / u.requests > 0.05 ? "bad" : ""}>{u?.requests ? `${((u.errors / u.requests) * 100).toFixed(u.errors ? 1 : 0)}%` : "—"}</b>
                      <span>失败率</span>
                    </div>
                    <div>
                      <b>{fmtNum((u?.input ?? 0) + (u?.cacheRead ?? 0) + (u?.output ?? 0))}</b>
                      <span>Token</span>
                    </div>
                    <div>
                      <b>{u?.cost ? fmtCost(u.cost) : "—"}</b>
                      <span>费用</span>
                    </div>
                  </div>

                  <div className="ep2-models">
                    {e.models.length === 0 && <span className="muted-sm">还没有配置模型</span>}
                    {e.models.map((m) => (
                      <div key={m.name} className={`ep2-model ${m.enabled ? "" : "off"}`}>
                        <code>{m.name}</code>
                        <span>{m.contextWindow ? fmtContext(m.contextWindow) : ""}</span>
                      </div>
                    ))}
                  </div>

                  <div className="ep2-foot">
                    <button className="btn btn-sm" onClick={() => setTesting(e)} disabled={!e.enabled}>
                      <Zap size={14} />
                      测试
                    </button>
                    <button className="btn btn-sm btn-ghost" onClick={() => go({ name: "logs", endpoint: e.id })}>
                      <ScrollText size={14} />
                      日志
                    </button>
                    <span style={{ flex: 1 }} />
                    <button className="btn btn-sm btn-ghost" onClick={() => go({ name: "endpoint", id: e.id })}>
                      <Pencil size={14} />
                      编辑
                    </button>
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
      {testing && <TestDialog endpoint={testing} onClose={() => setTesting(null)} />}
      {menu && (
        <Menu
          anchor={menu.el}
          onClose={() => setMenu(null)}
          items={[
            { label: "复制", icon: <Copy size={15} />, onClick: () => duplicate(menu.ep) },
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
