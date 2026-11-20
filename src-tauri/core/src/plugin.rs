//! 插件接口。
//!
//! 插件是编译进 App 的 Rust 代码，代表一种「端点类型」，比如「公司内部服务」。
//! 在界面上它和 Anthropic、OpenAI 一样，是新建端点时可选的类型。
//!
//! 插件声明内部服务说的是哪种标准格式（`protocol()`，默认 OpenAI Chat），
//! 网关负责把任何客户端（Claude Code、Codex、Gemini CLI……）的请求转换成这种格式，
//! 插件只需要关心：登录状态（登录、存 token、续期），以及内部格式和标准格式不一样的小地方。
//!
//! 写一个插件的步骤见 `plugins/mod.rs` 顶部的说明。

use crate::ir::Protocol;
use crate::sse::SseEvent;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ------------------------------------------------------------------ 插件声明的设置项

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    /// 存到钥匙串，界面上用密码框
    Secret,
    Url,
    /// 多行文本
    Multiline,
}

/// 插件需要用户填写的设置（显示在端点编辑页里）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingField {
    pub key: String,
    pub label: String,
    pub help: String,
    pub kind: FieldKind,
    pub default: String,
    pub required: bool,
}

impl SettingField {
    pub fn text(key: &str, label: &str) -> Self {
        SettingField { key: key.into(), label: label.into(), help: String::new(), kind: FieldKind::Text, default: String::new(), required: false }
    }
    pub fn secret(key: &str, label: &str) -> Self {
        SettingField { kind: FieldKind::Secret, ..Self::text(key, label) }
    }
    pub fn url(key: &str, label: &str) -> Self {
        SettingField { kind: FieldKind::Url, ..Self::text(key, label) }
    }
    pub fn help(mut self, h: &str) -> Self {
        self.help = h.into();
        self
    }
    pub fn default_value(mut self, d: &str) -> Self {
        self.default = d.into();
        self
    }
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

// ------------------------------------------------------------------ 凭证

/// 插件登录后拿到的凭证。网关负责缓存（内存 + 钥匙串）和过期刷新。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// 过期时间（Unix 秒）；None 表示不会过期，直到上游返回鉴权失败
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// 插件自己需要的其他东西，例如 cookie、用户名
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

impl Credential {
    pub fn bearer(token: impl Into<String>) -> Self {
        Credential { token: token.into(), ..Default::default() }
    }

    pub fn expires_in(mut self, secs: i64) -> Self {
        self.expires_at = Some(crate::ir::now_secs() + secs);
        self
    }

    /// 提前 60 秒视为过期，避免请求发出去的路上过期
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|t| crate::ir::now_secs() + 60 >= t)
    }
}

// ------------------------------------------------------------------ 请求与响应

/// 发往上游的请求。插件可以改其中任何东西。
#[derive(Debug, Clone)]
pub struct UpstreamRequest {
    pub method: reqwest::Method,
    pub url: String,
    pub headers: HeaderMap,
    /// 已经转换成上游协议的 JSON 请求体
    pub body: Value,
    /// 如果插件需要发非 JSON 的请求体，设置这个，网关会忽略 `body`
    pub raw_body: Option<Vec<u8>>,
}

impl UpstreamRequest {
    pub fn set_header(&mut self, name: &str, value: &str) -> Result<()> {
        self.headers.insert(HeaderName::from_bytes(name.as_bytes())?, HeaderValue::from_str(value)?);
        Ok(())
    }

    pub fn remove_header(&mut self, name: &str) {
        self.headers.remove(name);
    }

    pub fn body_bytes(&self) -> Vec<u8> {
        match &self.raw_body {
            Some(b) => b.clone(),
            None => serde_json::to_vec(&self.body).unwrap_or_default(),
        }
    }
}

/// 上游的非流式响应。插件可以在网关解析之前改写它（比如把私有格式改成标准格式）。
#[derive(Debug, Clone)]
pub struct UpstreamResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

// ------------------------------------------------------------------ 宿主能力

/// 弹窗表单里的一项
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptField {
    pub key: String,
    pub label: String,
    /// text / password / code
    pub kind: String,
    pub placeholder: String,
    pub default: String,
}

impl PromptField {
    pub fn text(key: &str, label: &str) -> Self {
        PromptField { key: key.into(), label: label.into(), kind: "text".into(), placeholder: String::new(), default: String::new() }
    }
    pub fn password(key: &str, label: &str) -> Self {
        PromptField { kind: "password".into(), ..Self::text(key, label) }
    }
    pub fn code(key: &str, label: &str) -> Self {
        PromptField { kind: "code".into(), ..Self::text(key, label) }
    }
    pub fn placeholder(mut self, p: &str) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn default_value(mut self, d: &str) -> Self {
        self.default = d.into();
        self
    }
}

/// 让 App 弹出的表单（账号密码、验证码等）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptForm {
    pub title: String,
    pub message: String,
    pub fields: Vec<PromptField>,
    pub submit_text: String,
    /// 可选：表单上方显示的图片（例如图形验证码），data URL 或 http URL
    #[serde(default)]
    pub image: Option<String>,
}

impl PromptForm {
    pub fn new(title: &str) -> Self {
        PromptForm { title: title.into(), message: String::new(), fields: vec![], submit_text: "确定".into(), image: None }
    }
    pub fn message(mut self, m: &str) -> Self {
        self.message = m.into();
        self
    }
    pub fn field(mut self, f: PromptField) -> Self {
        self.fields.push(f);
        self
    }
    pub fn submit(mut self, s: &str) -> Self {
        self.submit_text = s.into();
        self
    }
    pub fn image(mut self, url: &str) -> Self {
        self.image = Some(url.into());
        self
    }
}

/// App（Tauri 那一层）提供给插件的能力
#[async_trait]
pub trait Host: Send + Sync {
    /// 用系统默认浏览器打开链接
    async fn open_url(&self, url: &str) -> Result<()>;
    /// 弹出表单并等待用户填写；用户取消时返回 None
    async fn prompt(&self, form: PromptForm) -> Result<Option<BTreeMap<String, String>>>;
    /// 发一条系统通知
    fn notify(&self, title: &str, body: &str);
}

/// 没有界面时用的宿主（单元测试、无界面运行）
pub struct HeadlessHost;

#[async_trait]
impl Host for HeadlessHost {
    async fn open_url(&self, url: &str) -> Result<()> {
        tracing::info!("请在浏览器里打开：{url}");
        Ok(())
    }
    async fn prompt(&self, _form: PromptForm) -> Result<Option<BTreeMap<String, String>>> {
        bail!("当前没有界面，无法弹出登录表单")
    }
    fn notify(&self, title: &str, body: &str) {
        tracing::info!("{title}: {body}");
    }
}

// ------------------------------------------------------------------ 密钥存储

/// 钥匙串的抽象（App 里用 macOS 钥匙串实现，测试里用内存实现）
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

#[derive(Default)]
pub struct MemorySecrets(parking_lot::Mutex<std::collections::HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().remove(key);
        Ok(())
    }
}

/// 钥匙串里各类密钥的命名
pub mod keys {
    pub fn endpoint_api_key(endpoint: &str) -> String {
        format!("endpoint/{endpoint}/api_key")
    }
    pub fn endpoint_credential(endpoint: &str) -> String {
        format!("endpoint/{endpoint}/credential")
    }
    pub fn plugin_secret(endpoint: &str, key: &str) -> String {
        format!("endpoint/{endpoint}/plugin/{key}")
    }
    pub fn plugin_data(endpoint: &str, key: &str) -> String {
        format!("endpoint/{endpoint}/data/{key}")
    }
    pub fn proxy_password(proxy: &str) -> String {
        format!("proxy/{proxy}/password")
    }
}

// ------------------------------------------------------------------ 插件上下文

/// 每次调用插件时传入的上下文
#[derive(Clone)]
pub struct PluginCtx {
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub base_url: String,
    /// 插件设置（包括从钥匙串读出来的敏感项）
    pub settings: BTreeMap<String, String>,
    pub client_protocol: Option<Protocol>,
    pub upstream_protocol: Protocol,
    /// 本次请求的上游模型名（登录时为空）
    pub model: String,
    /// 走端点出口代理的 HTTP 客户端，插件调登录接口时用它
    pub http: reqwest::Client,
    pub host: Arc<dyn Host>,
    pub secrets: Arc<dyn SecretStore>,
}

impl PluginCtx {
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings.get(key).map(String::as_str).filter(|s| !s.is_empty())
    }

    pub fn require(&self, key: &str) -> Result<&str> {
        self.setting(key).ok_or_else(|| anyhow!("端点「{}」的插件设置缺少「{key}」", self.endpoint_name))
    }

    /// 插件自己的持久化数据（存在钥匙串里，按端点隔离）
    pub fn load(&self, key: &str) -> Option<String> {
        self.secrets.get(&keys::plugin_data(&self.endpoint_id, key)).ok().flatten()
    }

    pub fn save(&self, key: &str, value: &str) -> Result<()> {
        self.secrets.set(&keys::plugin_data(&self.endpoint_id, key), value)
    }

    pub async fn open_url(&self, url: &str) -> Result<()> {
        self.host.open_url(url).await
    }

    /// 弹出表单；用户取消时返回错误
    pub async fn prompt(&self, form: PromptForm) -> Result<BTreeMap<String, String>> {
        self.host.prompt(form).await?.ok_or_else(|| anyhow!("已取消登录"))
    }

    pub fn notify(&self, title: &str, body: &str) {
        self.host.notify(title, body)
    }
}

// ------------------------------------------------------------------ 插件接口

#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// 唯一 id，写在配置文件里，不要改
    fn id(&self) -> &'static str;
    /// 显示名
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str {
        ""
    }
    /// 需要用户在端点设置里填的东西
    fn settings(&self) -> Vec<SettingField> {
        vec![]
    }

    /// 内部服务使用的接口格式。网关会把客户端请求转换成这种格式后再交给插件。
    fn protocol(&self) -> Protocol {
        Protocol::OpenaiChat
    }

    /// 新建这种端点时预填的地址
    fn default_base_url(&self) -> &'static str {
        ""
    }

    /// App 启动时如果没有有效凭证就主动登录（像命令行工具启动时打开网页登录一样）
    fn login_on_start(&self) -> bool {
        self.uses_login()
    }

    /// 凭证到期前多少秒在后台主动续期（调用 `refresh`）
    fn refresh_before_secs(&self) -> i64 {
        300
    }

    /// 是否使用网关的凭证管理（login / refresh / apply_credential）
    fn uses_login(&self) -> bool {
        false
    }

    /// 登录，拿到凭证。可以打开浏览器、弹表单、调用登录接口等。
    /// 第一次请求时、凭证过期且刷新失败时、上游返回鉴权失败时会调用。
    async fn login(&self, _ctx: &PluginCtx) -> Result<Credential> {
        bail!("插件「{}」没有实现登录", self.name())
    }

    /// 凭证过期时刷新；默认直接重新登录
    async fn refresh(&self, ctx: &PluginCtx, _old: &Credential) -> Result<Credential> {
        self.login(ctx).await
    }

    /// 把凭证放进请求；默认是 `Authorization: Bearer <token>`
    fn apply_credential(&self, cred: &Credential, req: &mut UpstreamRequest) -> Result<()> {
        req.set_header("authorization", &format!("Bearer {}", cred.token))
    }

    /// 判断上游响应是不是「凭证失效」；是的话网关会重新登录并重试一次
    fn is_auth_failure(&self, status: u16, _body: &[u8]) -> bool {
        status == 401
    }

    /// 发送前改写请求（URL、请求头、请求体都可以改）
    async fn on_request(&self, _ctx: &PluginCtx, _req: &mut UpstreamRequest) -> Result<()> {
        Ok(())
    }

    /// 改写非流式响应（在网关解析之前）
    async fn on_response(&self, _ctx: &PluginCtx, _resp: &mut UpstreamResponse) -> Result<()> {
        Ok(())
    }

    /// 改写流式响应的每个 SSE 事件（在网关解析之前）；可以返回 0 个或多个事件
    fn on_stream_event(&self, _ctx: &PluginCtx, ev: SseEvent) -> Vec<SseEvent> {
        vec![ev]
    }
}

// ------------------------------------------------------------------ 本地回调服务（浏览器 SSO 用）

/// 在本机起一个临时 HTTP 服务，接收 SSO 登录完成后浏览器的跳转。
///
/// ```ignore
/// let cb = CallbackServer::bind(0).await?;           // 0 = 随机端口；有的 SSO 要求固定端口
/// let redirect = cb.redirect_uri("/callback");
/// ctx.open_url(&format!("{login}?redirect_uri={redirect}")).await?;
/// let hit = cb.wait(Duration::from_secs(300)).await?;
/// let code = hit.query.get("code");
/// ```
pub struct CallbackServer {
    listener: tokio::net::TcpListener,
    port: u16,
}

#[derive(Debug, Clone)]
pub struct CallbackHit {
    pub path: String,
    pub query: BTreeMap<String, String>,
}

impl CallbackServer {
    pub async fn bind(port: u16) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        let port = listener.local_addr()?.port();
        Ok(CallbackServer { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn redirect_uri(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    /// 等待第一个非 favicon 的请求，回一个「登录成功」页面
    pub async fn wait(self, timeout: Duration) -> Result<CallbackHit> {
        let fut = async {
            loop {
                let (mut sock, _) = self.listener.accept().await?;
                let mut buf = vec![0u8; 16 * 1024];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/").to_string();
                if target.starts_with("/favicon") {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
                    continue;
                }
                let html = "<!doctype html><meta charset=utf-8><title>kiki-proxy</title>\
                    <body style=\"font-family:-apple-system,sans-serif;display:grid;place-items:center;height:90vh;color:#1b1f2e;background:#f5f6fb\">\
                    <div style=\"text-align:center\"><div style=\"font-size:40px\">✓</div><h2>登录成功</h2><p style=\"color:#646a80\">可以关闭这个页面，回到 kiki-proxy 了。</p></div>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let parsed = url::Url::parse(&format!("http://localhost{target}"))?;
                return Ok::<_, anyhow::Error>(CallbackHit {
                    path: parsed.path().to_string(),
                    query: parsed.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                });
            }
        };
        tokio::time::timeout(timeout, fut).await.map_err(|_| anyhow!("等待浏览器登录超时"))?
    }
}
