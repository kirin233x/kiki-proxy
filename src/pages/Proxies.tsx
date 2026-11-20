import { useEffect, useState } from "react";
import { Globe, Laptop, Pencil, Plus, Shield, Trash2, Zap } from "lucide-react";
import { api, ProxyConfig, secretKeys, TestResult } from "../api";
import { useApp } from "../App";
import { Confirm, Field, Modal, SecretInput, StatusDot, useToast } from "../components/ui";
import { fmtMs, slugify } from "../utils";

export default function Proxies() {
  const { config, saveConfig } = useApp();
  const [editing, setEditing] = useState<ProxyConfig | "new" | null>(null);
  const [del, setDel] = useState<ProxyConfig | null>(null);
  const [tests, setTests] = useState<Record<string, TestResult | "busy">>({});

  const usedBy = (id: string) => config.endpoints.filter((e) => e.proxy === id);
  const test = async (id: string) => {
    setTests((t) => ({ ...t, [id]: "busy" }));
    const r = await api.testProxy(id);
    setTests((t) => ({ ...t, [id]: r }));
  };

  const card = (id: string, name: string, desc: string, icon: React.ReactNode, actions?: React.ReactNode) => {
    const t = tests[id];
    const eps = usedBy(id);
    return (
      <div key={id} className="proxy-card panel">
        <div className="proxy-icon">{icon}</div>
        <div className="proxy-main">
          <b>{name}</b>
          <span className="mono-sm">{desc}</span>
          <div className="proxy-used">
            {eps.length ? eps.map((e) => <span key={e.id} className="tag">{e.name}</span>) : <span className="muted-sm">没有端点使用</span>}
          </div>
        </div>
        <div className="proxy-side">
          {t && t !== "busy" && (
            <span className={`test-result ${t.ok ? "ok" : "err"}`} title={t.message}>
              <StatusDot tone={t.ok ? "ok" : "err"} />
              {t.ok ? fmtMs(t.latencyMs) : t.message.slice(0, 36)}
            </span>
          )}
          <button className="btn btn-sm" onClick={() => test(id)} disabled={t === "busy"}>
            <Zap size={14} />
            {t === "busy" ? "测试中…" : "测试"}
          </button>
          {actions}
        </div>
      </div>
    );
  };

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 940 }}>
        <div className="page-header">
          <div>
            <h1 className="page-title">出口代理</h1>
            <div className="page-subtitle">每个端点可以走不同的线路：内部服务直连，官方 API 走代理</div>
          </div>
          <div className="header-actions">
            <button className="btn btn-primary" onClick={() => setEditing("new")}>
              <Plus size={16} />
              添加代理
            </button>
          </div>
        </div>
        <div className="proxy-list">
          {card("direct", "直连", "不经过任何代理，即使系统开了代理也不走", <Shield size={18} />)}
          {card("system", "跟随系统代理", "使用 macOS 网络设置里的代理（以及 HTTPS_PROXY 等环境变量）", <Laptop size={18} />)}
          {config.proxies.map((p) =>
            card(
              p.id,
              p.name,
              `${p.kind === "socks5" ? "SOCKS5" : "HTTP"} · ${p.url}${p.username ? ` · ${p.username}` : ""}`,
              <Globe size={18} />,
              <>
                <button className="icon-btn" onClick={() => setEditing(p)}>
                  <Pencil size={15} />
                </button>
                <button className="icon-btn" onClick={() => setDel(p)}>
                  <Trash2 size={15} />
                </button>
              </>,
            ),
          )}
        </div>
        <p className="muted-sm" style={{ marginTop: 16 }}>
          测试会通过该线路访问 <code>{config.ui.probeUrl}</code>，可以在设置里修改。SOCKS5 代理由代理端负责解析域名，可以访问内网域名。
        </p>
      </div>
      {editing && (
        <ProxyDialog
          initial={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSave={async (p, password) => {
            const exists = config.proxies.some((x) => x.id === p.id);
            await saveConfig({ ...config, proxies: exists ? config.proxies.map((x) => (x.id === p.id ? p : x)) : [...config.proxies, p] });
            if (password !== null) await api.setSecret(secretKeys.proxyPassword(p.id), password);
            setEditing(null);
          }}
        />
      )}
      {del && (
        <Confirm
          title={`删除「${del.name}」？`}
          message={usedBy(del.id).length ? `正在使用它的端点会改为「跟随系统代理」：${usedBy(del.id).map((e) => e.name).join("、")}` : "这个代理没有被使用。"}
          danger
          confirmText="删除"
          onClose={() => setDel(null)}
          onConfirm={() =>
            saveConfig({
              ...config,
              proxies: config.proxies.filter((x) => x.id !== del.id),
              endpoints: config.endpoints.map((e) => (e.proxy === del.id ? { ...e, proxy: "system" } : e)),
              rules: config.rules.map((r) => (r.proxy === del.id ? { ...r, proxy: "" } : r)),
            })
          }
        />
      )}
    </div>
  );
}

function ProxyDialog(props: { initial: ProxyConfig | null; onClose: () => void; onSave: (p: ProxyConfig, password: string | null) => Promise<void> }) {
  const toast = useToast();
  const { config } = useApp();
  const [p, setP] = useState<ProxyConfig>(props.initial ?? { id: "", name: "", kind: "http", url: "", username: "" });
  const [password, setPassword] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  useEffect(() => {
    if (props.initial) api.secretFlags([secretKeys.proxyPassword(props.initial.id)]).then((f) => setHasPassword(!!f[secretKeys.proxyPassword(props.initial!.id)]));
  }, [props.initial]);

  const submit = async () => {
    const id = props.initial?.id || slugify(p.name) || `proxy${config.proxies.length + 1}`;
    if (!p.name || !p.url) return toast("请填写名称和地址", true);
    if (!props.initial && (config.proxies.some((x) => x.id === id) || id === "direct" || id === "system")) return toast("名称和已有代理重复了", true);
    await props.onSave({ ...p, id }, password ? password : null);
  };

  return (
    <Modal
      title={props.initial ? "编辑代理" : "添加代理"}
      icon={<Globe size={18} />}
      onClose={props.onClose}
      footer={
        <>
          <button className="btn" onClick={props.onClose}>
            取消
          </button>
          <button className="btn btn-primary" onClick={submit}>
            保存
          </button>
        </>
      }
    >
      <Field label="名称">
        <input className="input" value={p.name} autoFocus placeholder="例如：公司代理" onChange={(e) => setP({ ...p, name: e.target.value })} />
      </Field>
      <Field label="类型">
        <div className="segmented">
          <button className={p.kind === "http" ? "on" : ""} onClick={() => setP({ ...p, kind: "http" })}>
            HTTP / HTTPS
          </button>
          <button className={p.kind === "socks5" ? "on" : ""} onClick={() => setP({ ...p, kind: "socks5" })}>
            SOCKS5
          </button>
        </div>
      </Field>
      <Field label="地址" help={p.kind === "socks5" ? "例如 127.0.0.1:7890" : "例如 http://proxy.corp.example:8080"}>
        <input className="input mono" value={p.url} placeholder={p.kind === "socks5" ? "127.0.0.1:7890" : "http://host:port"} onChange={(e) => setP({ ...p, url: e.target.value.trim() })} />
      </Field>
      <div className="row">
        <Field label="用户名（可选）">
          <input className="input" value={p.username} onChange={(e) => setP({ ...p, username: e.target.value })} />
        </Field>
        <Field label="密码（可选）">
          <SecretInput isSet={hasPassword} value={password} onChange={setPassword} />
        </Field>
      </div>
    </Modal>
  );
}
