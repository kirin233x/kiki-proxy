import { useEffect, useState } from "react";
import { Download, FolderOpen, Monitor, Moon, Sun, Upload } from "lucide-react";
import { api, Config } from "../api";
import { useApp } from "../App";
import { ProxiesSection } from "./Proxies";
import LogoMark from "../components/Logo";
import { Confirm, Modal, SecretInput, Switch, useToast, Hint } from "../components/ui";

export default function SettingsPage() {
  const { config, saveConfig } = useApp();
  const toast = useToast();
  const [autostart, setAutostart] = useState(false);
  const [info, setInfo] = useState<{ version: string; gitHash: string; buildTime: number } | null>(null);
  const [port, setPort] = useState(String(config.server.port));
  const [accessKey, setAccessKey] = useState("");
  const [exported, setExported] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [clear, setClear] = useState(false);

  useEffect(() => {
    api.getAutostart().then(setAutostart);
    api
      .appInfo()
      .then(setInfo)
      .catch(() => {});
  }, []);

  const save = (patch: (c: Config) => Config) => saveConfig(patch(structuredClone(config))).catch(() => {});

  const applyPort = () => {
    const p = Number(port);
    if (!Number.isInteger(p) || p < 1024 || p > 65535) {
      toast("端口需要在 1024 ~ 65535 之间", true);
      setPort(String(config.server.port));
      return;
    }
    if (p !== config.server.port) save((c) => ({ ...c, server: { ...c.server, port: p } })).then(() => toast(`网关已切换到端口 ${p}，记得更新客户端配置`));
  };

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 760 }}>
        <div className="page-header">
          <div>
            <h1 className="page-title">设置</h1>
          </div>
        </div>

        <div className="settings-group">
          <h3>外观</h3>
          <div className="panel">
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  主题
                  <Hint>跟随系统，或固定使用浅色 / 深色</Hint>
                </b>
              </div>
              <div className="segmented">
                {(
                  [
                    ["auto", "自动", <Monitor size={14} key="a" />],
                    ["light", "浅色", <Sun size={14} key="l" />],
                    ["dark", "深色", <Moon size={14} key="d" />],
                  ] as const
                ).map(([k, l, icon]) => (
                  <button key={k} className={config.ui.theme === k ? "on" : ""} onClick={() => save((c) => ({ ...c, ui: { ...c.ui, theme: k } }))}>
                    {icon}
                    {l}
                  </button>
                ))}
              </div>
            </div>
          </div>
        </div>

        <div className="settings-group">
          <h3>网关</h3>
          <div className="panel">
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  端口
                  <Hint>只监听 127.0.0.1，局域网里的其他设备访问不到</Hint>
                </b>
              </div>
              <input
                className="input mono"
                style={{ width: 110 }}
                value={port}
                onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
                onBlur={applyPort}
                onKeyDown={(e) => e.key === "Enter" && applyPort()}
              />
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  访问密钥
                  <Hint>设置后，客户端必须带上这个密钥（任意请求头里的 API Key）才能使用网关</Hint>
                </b>
              </div>
              <div style={{ width: 260 }}>
                <SecretInput
                  isSet={!!config.server.accessKey}
                  value={accessKey}
                  placeholder="不设置"
                  onChange={setAccessKey}
                  onClear={() => save((c) => ({ ...c, server: { ...c.server, accessKey: "" } }))}
                />
                {accessKey && (
                  <button
                    className="btn btn-sm btn-primary"
                    style={{ marginTop: 8 }}
                    onClick={() => save((c) => ({ ...c, server: { ...c.server, accessKey } })).then(() => setAccessKey(""))}
                  >
                    保存密钥
                  </button>
                )}
              </div>
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  开机自动启动
                  <Hint>登录后在菜单栏静默运行；关闭窗口不会停止网关</Hint>
                </b>
              </div>
              <Switch
                on={autostart}
                onChange={(v) =>
                  api
                    .setAutostart(v)
                    .then(() => setAutostart(v))
                    .catch((e) => toast(String(e), true))
                }
              />
            </div>
          </div>
        </div>

        <ProxiesSection />

        <div className="settings-group">
          <h3>
            故障切换
            <Hint>出现下面这些情况时，换到端点 / 模型设置的备选；已经开始输出的回答不会被打断重来</Hint>
          </h3>
          <div className="panel">
            {(
              [
                ["onConnectError", "连接失败", "DNS 解析失败、连接被拒、代理不可用、TLS 错误、登录失败"],
                ["onTimeout", "首字节超时", "超过端点设置的等待时间仍没有响应"],
                ["on5xx", "上游 5xx", "服务端错误、过载（500 / 502 / 503 / 529 等）"],
                ["on429", "上游 429", "被限流"],
              ] as const
            ).map(([k, title, desc]) => (
              <div key={k} className="setting-row">
                <div className="setting-text">
                  <b>
                    {title}
                    <Hint>{desc}</Hint>
                  </b>
                </div>
                <Switch on={config.failover[k]} onChange={(v) => save((c) => ({ ...c, failover: { ...c.failover, [k]: v } }))} />
              </div>
            ))}
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  其他状态码
                  <Hint>额外触发切换的状态码，逗号分隔，例如 401, 403</Hint>
                </b>
              </div>
              <input
                className="input mono"
                style={{ width: 200 }}
                defaultValue={config.failover.extraStatus.join(", ")}
                placeholder="例如 403"
                onBlur={(e) => {
                  const extraStatus = e.target.value
                    .split(/[,，\s]+/)
                    .map(Number)
                    .filter((n) => n >= 100 && n < 600);
                  if (extraStatus.join() !== config.failover.extraStatus.join()) save((c) => ({ ...c, failover: { ...c.failover, extraStatus } }));
                }}
              />
            </div>
          </div>
        </div>

        <div className="settings-group">
          <h3>日志</h3>
          <div className="panel">
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  完整内容保留
                  <Hint>请求体、响应体、请求头（含图片），用于排查问题</Hint>
                </b>
              </div>
              <select
                className="select"
                style={{ width: 130 }}
                value={config.logging.bodyRetentionHours}
                onChange={(e) => save((c) => ({ ...c, logging: { ...c.logging, bodyRetentionHours: Number(e.target.value) } }))}
              >
                {[1, 6, 12, 24, 48, 72, 168].map((h) => (
                  <option key={h} value={h}>
                    {h < 24 ? `${h} 小时` : `${h / 24} 天`}
                  </option>
                ))}
              </select>
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  用量摘要保留
                  <Hint>模型、token、耗时，用于统计</Hint>
                </b>
              </div>
              <select
                className="select"
                style={{ width: 130 }}
                value={config.logging.statsRetentionDays}
                onChange={(e) => save((c) => ({ ...c, logging: { ...c.logging, statsRetentionDays: Number(e.target.value) } }))}
              >
                {[7, 30, 90, 180, 365].map((d) => (
                  <option key={d} value={d}>
                    {d} 天
                  </option>
                ))}
              </select>
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  单条记录上限
                  <Hint>超过的部分会被截断，避免大图片把日志撑得太大</Hint>
                </b>
              </div>
              <select
                className="select"
                style={{ width: 130 }}
                value={config.logging.maxBodyKb}
                onChange={(e) => save((c) => ({ ...c, logging: { ...c.logging, maxBodyKb: Number(e.target.value) } }))}
              >
                {[256, 1024, 4096, 8192, 32768].map((k) => (
                  <option key={k} value={k}>
                    {k >= 1024 ? `${k / 1024} MB` : `${k} KB`}
                  </option>
                ))}
              </select>
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  清空日志
                  <Hint>删除所有请求记录和统计</Hint>
                </b>
              </div>
              <button className="btn btn-danger btn-sm" onClick={() => setClear(true)}>
                清空
              </button>
            </div>
          </div>
        </div>

        <div className="settings-group">
          <h3>配置文件</h3>
          <div className="panel">
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  导出 / 导入
                  <Hint>TOML 格式，不包含 API Key 等密钥（密钥在钥匙串里）</Hint>
                </b>
              </div>
              <div style={{ display: "flex", gap: 8 }}>
                <button className="btn btn-sm" onClick={() => api.exportConfig().then(setExported)}>
                  <Download size={14} />
                  导出
                </button>
                <button className="btn btn-sm" onClick={() => setImporting(true)}>
                  <Upload size={14} />
                  导入
                </button>
                <button className="btn btn-sm" onClick={() => api.revealConfig()}>
                  <FolderOpen size={14} />
                  在访达中显示
                </button>
              </div>
            </div>
            <div className="setting-row">
              <div className="setting-text">
                <b>
                  测速地址
                  <Hint>测试出口代理时访问的地址</Hint>
                </b>
              </div>
              <input
                className="input mono"
                style={{ width: 300 }}
                defaultValue={config.ui.probeUrl}
                onBlur={(e) => e.target.value !== config.ui.probeUrl && save((c) => ({ ...c, ui: { ...c.ui, probeUrl: e.target.value.trim() } }))}
              />
            </div>
          </div>
        </div>

        <div className="about">
          <div className="brand-logo">
            <LogoMark size={22} />
          </div>
          <div>
            <b>kiki-proxy {info ? info.version : ""}</b>
            {info && (
              <span className="mono-sm">
                构建 {info.gitHash} · {new Date(info.buildTime * 1000).toLocaleString("zh-CN")}
              </span>
            )}
            <span>本地 AI 网关 · 配置文件、日志数据库保存在本机，不会上传到任何地方</span>
          </div>
        </div>
      </div>

      {exported !== null && (
        <Modal
          title="导出配置"
          onClose={() => setExported(null)}
          wide
          footer={
            <button className="btn btn-primary" onClick={() => navigator.clipboard?.writeText(exported).then(() => toast("已复制"))}>
              复制
            </button>
          }
        >
          <pre className="code" style={{ maxHeight: 420 }}>
            {exported}
          </pre>
        </Modal>
      )}
      {importing && <ImportDialog onClose={() => setImporting(false)} />}
      {clear && (
        <Confirm
          title="清空所有日志？"
          message="请求记录和用量统计都会被删除，不能恢复。"
          danger
          confirmText="清空"
          onClose={() => setClear(false)}
          onConfirm={() => api.clearLogs().then(() => toast("已清空"))}
        />
      )}
    </div>
  );
}

function ImportDialog({ onClose }: { onClose: () => void }) {
  const toast = useToast();
  const [text, setText] = useState("");
  return (
    <Modal
      title="导入配置"
      wide
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            取消
          </button>
          <button
            className="btn btn-primary"
            disabled={!text.trim()}
            onClick={() =>
              api
                .importConfig(text)
                .then(() => {
                  toast("已导入，正在重新加载");
                  setTimeout(() => location.reload(), 600);
                })
                .catch((e) => toast(String(e), true))
            }
          >
            导入并替换当前配置
          </button>
        </>
      }
    >
      <p className="muted-sm" style={{ marginTop: 0 }}>
        粘贴导出的 TOML。导入会替换当前所有端点、代理和规则；API Key 需要重新填写。
      </p>
      <textarea
        className="textarea mono"
        rows={16}
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="[server]&#10;port = 41521"
      />
    </Modal>
  );
}
