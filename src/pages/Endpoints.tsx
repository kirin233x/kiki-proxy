import { useEffect, useState } from "react";
import { Copy, MoreHorizontal, Pencil, Plus, ScrollText, Server, Trash2, Zap } from "lucide-react";
import { api, EndpointConfig, fmtContext, protocolLabel, UsageRow } from "../api";
import { useApp } from "../App";
import TestDialog from "../components/TestDialog";
import { ProcessBadge } from "../components/ProcessPanel";
import { Confirm, Empty, Menu, Switch, useToast } from "../components/ui";
import { SortHandle, useSortable } from "../components/SortableList";
import { fmtMs, fmtNum, protocolColor } from "../utils";

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

  // 拖动排序：先在本地生效再保存，侧栏和模型列表都按这个顺序
  const [order, setOrder] = useState(config.endpoints);
  useEffect(() => setOrder(config.endpoints), [config.endpoints]);
  const sort = useSortable(
    (from, to) => {
      const next = [...order];
      const [x] = next.splice(from, 1);
      next.splice(to, 0, x);
      setOrder(next);
      saveConfig({ ...config, endpoints: next }).catch(() => setOrder(config.endpoints));
    },
    { grid: true },
  );

  const update = (id: string, patch: Partial<EndpointConfig>) =>
    saveConfig({ ...config, endpoints: config.endpoints.map((e) => (e.id === id ? { ...e, ...patch } : e)) });

  const duplicate = (e: EndpointConfig) => {
    let id = `${e.id}-2`;
    let n = 2;
    while (config.endpoints.some((x) => x.id === id)) id = `${e.id}-${++n}`;
    saveConfig({ ...config, endpoints: [...config.endpoints, { ...structuredClone(e), id, name: `${e.name} 副本` }] }).then(() =>
      toast(`已复制为「${e.name} 副本」，请重新填写 API Key`),
    );
  };

  return (
    <div className="page">
      <div className="page-inner">
        <div className="page-header">
          <div>
            <h1 className="page-title">端点</h1>
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
            <Empty
              icon={<Server size={22} />}
              title="暂无端点"
              action={
                <button className="btn btn-primary" onClick={() => go({ name: "endpoint", id: null })}>
                  添加端点
                </button>
              }
            >
              官方 API、中转服务和内部服务均可作为端点
            </Empty>
          </div>
        ) : (
          <div ref={sort.listRef} className={`ep-cards ${sort.sorting ? "is-sorting" : ""}`}>
            {order.map((e, i) => {
              const sr = sort.row(i);
              const u = usage[e.id];
              const typeLabel = protocolLabel(e.protocol);
              const color = protocolColor[e.protocol];
              return (
                <div key={e.id} {...sr} className={`ep2 panel ${e.enabled ? "" : "off"} ${sr.className}`}>
                  <div className="ep2-head">
                    <SortHandle {...sort.handle(i)} />
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
                    <label className="ep2-toggle" title={e.enabled ? "停用后网关不再向该端点转发请求，也不会启动其本地服务" : "启用该端点"}>
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
                      <b className={u && u.errors / u.requests > 0.05 ? "bad" : ""}>
                        {u?.requests ? `${((u.errors / u.requests) * 100).toFixed(u.errors ? 1 : 0)}%` : "—"}
                      </b>
                      <span>失败率</span>
                    </div>
                    <div>
                      <b>{fmtNum((u?.input ?? 0) + (u?.cacheRead ?? 0) + (u?.output ?? 0))}</b>
                      <span>Token</span>
                    </div>
                    <div>
                      <b>{u?.requests ? fmtMs(u.avgMs) : "—"}</b>
                      <span>平均耗时</span>
                    </div>
                  </div>

                  <div className="ep2-models">
                    {e.models.length === 0 && <span className="muted-sm">未配置模型</span>}
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
          message={
            <>
              使用 <code>{del.id}/</code> 前缀的请求将无法路由。钥匙串里的 API Key 不会自动删除。
            </>
          }
          danger
          confirmText="删除"
          onClose={() => setDel(null)}
          onConfirm={() => saveConfig({ ...config, endpoints: config.endpoints.filter((x) => x.id !== del.id) })}
        />
      )}
    </div>
  );
}
