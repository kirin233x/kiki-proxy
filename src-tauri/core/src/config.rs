//! 配置（存成可读的 TOML，密钥类的东西不在这里，在钥匙串里）。

use crate::ir::Protocol;
use crate::protocol::Compat;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 41521;
/// 内置的两个出口：直连和跟随系统代理
pub const PROXY_DIRECT: &str = "direct";
pub const PROXY_SYSTEM: &str = "system";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct Config {
    pub server: ServerConfig,
    pub routing: RoutingConfig,
    pub failover: FailoverConfig,
    pub logging: LogConfig,
    pub ui: UiConfig,
    pub pricing: PricingConfig,
    pub proxies: Vec<ProxyConfig>,
    pub endpoints: Vec<EndpointConfig>,
    pub rules: Vec<RuleConfig>,
    pub desktop: DesktopConfig,
}

/// Claude 桌面版的模型路由。桌面版只接受 `claude-sonnet-*`、`claude-opus-*` 这类名字（有一个不合格就整个列表无效），
/// 所以给每个真实模型分配一个这样的名字，请求到了网关再换回来
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DesktopConfig {
    pub routes: Vec<DesktopRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DesktopRoute {
    /// 给桌面版看的名字，如 `claude-sonnet-5`、`claude-sonnet-5-r2`
    pub id: String,
    /// 实际使用的网关模型，如 `internal/sonnet`
    pub model: String,
    /// 桌面版模型菜单里显示的名字，留空就显示网关模型名
    pub label: String,
    #[serde(rename = "supports1m")]
    pub supports_1m: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub port: u16,
    /// 客户端访问网关时要带的密钥；留空表示不校验（只监听 127.0.0.1）
    pub access_key: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig { port: DEFAULT_PORT, access_key: String::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RoutingConfig {
    /// 请求的模型名匹配不到任何端点和规则时使用的模型（网关模型名，例如 `ds/deepseek-chat`）
    pub default_model: String,
    /// 给 Claude Code 的模型列表里，名字不含 claude 的模型加上 `claude@` 前缀，
    /// 否则 Claude Code 的 /model 会把它们过滤掉
    pub claude_alias: bool,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        RoutingConfig { default_model: String::new(), claude_alias: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FailoverConfig {
    /// 连不上（DNS、拒绝连接、代理错误、TLS 错误）
    pub on_connect_error: bool,
    /// 等首字节超时
    pub on_timeout: bool,
    /// 上游返回 5xx
    pub on_5xx: bool,
    /// 上游返回 429（限流）
    pub on_429: bool,
    /// 额外触发切换的状态码，例如 [403]
    pub extra_status: Vec<u16>,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        FailoverConfig { on_connect_error: true, on_timeout: true, on_5xx: true, on_429: true, extra_status: vec![] }
    }
}

impl FailoverConfig {
    pub fn should_failover_status(&self, status: u16) -> bool {
        (self.on_5xx && (500..600).contains(&status)) || (self.on_429 && status == 429) || self.extra_status.contains(&status)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LogConfig {
    /// 完整的请求/响应内容保留多久
    pub body_retention_hours: u32,
    /// 请求摘要（用量统计用）保留多久
    pub stats_retention_days: u32,
    /// 单个请求体/响应体最多记录多少 KB，超出部分截断
    pub max_body_kb: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig { body_retention_hours: 24, stats_retention_days: 90, max_body_kb: 8192 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct UiConfig {
    /// auto / light / dark
    pub theme: String,
    /// 测试代理连通性时访问的地址
    pub probe_url: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig { theme: "auto".into(), probe_url: "https://www.gstatic.com/generate_204".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PricingConfig {
    /// 价格表地址（LiteLLM 格式），访问不了 GitHub 时可以换成镜像
    pub url: String,
    /// 下载价格表走哪个出口
    pub proxy: String,
}

impl Default for PricingConfig {
    fn default() -> Self {
        PricingConfig { url: crate::pricing::DEFAULT_URL.into(), proxy: PROXY_SYSTEM.into() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    Http,
    Socks5,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    pub id: String,
    pub name: String,
    pub kind: ProxyKind,
    /// 例如 `http://proxy.corp:8080`、`127.0.0.1:7890`
    pub url: String,
    /// 用户名（密码在钥匙串里）
    #[serde(default)]
    pub username: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// 按协议的惯例：Anthropic 用 x-api-key，OpenAI 用 Bearer，Gemini 用 x-goog-api-key
    #[default]
    Auto,
    Bearer,
    XApiKey,
    GoogApiKey,
    /// 不带密钥（由插件负责鉴权，或者服务本身不需要）
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Price {
    /// 美元 / 百万 token
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Price {
    pub fn cost(&self, u: &crate::ir::Usage) -> f64 {
        (u.input as f64 * self.input
            + u.output as f64 * self.output
            + u.cache_read as f64 * self.cache_read
            + u.cache_write as f64 * self.cache_write)
            / 1_000_000.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelConfig {
    /// 网关里的名字（前缀后面那段），例如 `sonnet`
    pub name: String,
    /// 发给上游的真实模型名，留空则与 name 相同
    pub upstream: String,
    pub display_name: String,
    pub enabled: bool,
    /// 支持图片输入
    pub vision: bool,
    /// 上下文窗口（token），0 表示未知。≥ 1M 时 Claude Code 会按 1M 使用
    pub context_window: u32,
    pub price: Price,
    /// 这个模型失败时依次尝试的模型（网关模型名）；为空时用端点的设置
    pub fallbacks: Vec<String>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        ModelConfig {
            name: String::new(),
            upstream: String::new(),
            display_name: String::new(),
            enabled: true,
            vision: true,
            context_window: 0,
            price: Price::default(),
            fallbacks: vec![],
        }
    }
}

impl ModelConfig {
    pub fn upstream_id(&self) -> &str {
        if self.upstream.is_empty() {
            &self.name
        } else {
            &self.upstream
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EndpointConfig {
    /// 同时也是模型前缀：`{id}/{model}`
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub protocol: Protocol,
    pub base_url: String,
    pub auth: AuthMode,
    /// 出口：`direct`、`system` 或某个代理的 id
    pub proxy: String,
    /// 由 kiki-proxy 启动并守护的本地程序（比如公司内部服务的命令行代理）
    pub process: ProcessConfig,
    /// 额外附加的请求头
    pub headers: BTreeMap<String, String>,
    /// 等待上游返回响应头的最长时间
    pub timeout_secs: u64,
    pub compat: Compat,
    pub models: Vec<ModelConfig>,
    /// 这个端点故障时依次尝试的模型（网关模型名）
    pub fallbacks: Vec<String>,
    pub note: String,
}

impl Default for EndpointConfig {
    fn default() -> Self {
        EndpointConfig {
            id: String::new(),
            name: String::new(),
            enabled: true,
            protocol: Protocol::Anthropic,
            base_url: String::new(),
            auth: AuthMode::Auto,
            proxy: PROXY_SYSTEM.into(),
            process: ProcessConfig::default(),
            headers: BTreeMap::new(),
            timeout_secs: 300,
            compat: Compat::default(),
            models: vec![],
            fallbacks: vec![],
            note: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ProcessConfig {
    pub enabled: bool,
    /// 启动命令，和在终端里敲的一样（通过登录 shell 执行）
    pub command: String,
    /// 工作目录，可以用 ~；留空为用户目录
    pub workdir: String,
    /// 额外的环境变量，每行一个 KEY=VALUE（优先于从终端读到的）
    pub env: String,
    /// 本地程序自己访问外网走哪条线：空 = 跟随终端里的代理变量；`direct` = 去掉代理变量；其他 = 某个出口代理的 id
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RuleConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// 匹配请求里的模型名，支持 `*` 和 `?`，例如 `claude-*haiku*`
    pub pattern: String,
    /// 只对某个客户端协议生效；为空表示所有协议
    pub client: Option<Protocol>,
    /// 改成哪个网关模型；`ds/*` 表示发到 ds 端点、模型名保持原样；留空表示不改模型
    pub target: String,
    /// 覆盖出口代理；为空表示用端点自己的设置
    pub proxy: String,
}

impl Default for RuleConfig {
    fn default() -> Self {
        RuleConfig {
            id: String::new(),
            name: String::new(),
            enabled: true,
            pattern: String::new(),
            client: None,
            target: String::new(),
            proxy: String::new(),
        }
    }
}

impl Config {
    pub fn endpoint(&self, id: &str) -> Option<&EndpointConfig> {
        self.endpoints.iter().find(|e| e.id == id)
    }

    pub fn proxy(&self, id: &str) -> Option<&ProxyConfig> {
        self.proxies.iter().find(|p| p.id == id)
    }

    /// 保存前检查，返回给用户看得懂的错误
    pub fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for e in &self.endpoints {
            if e.id.is_empty() {
                bail!("端点「{}」缺少前缀", e.name);
            }
            if e.id.contains('/') || e.id.contains(' ') || e.id.contains('@') {
                bail!("端点前缀「{}」不能包含 / @ 或空格", e.id);
            }
            if !ids.insert(e.id.as_str()) {
                bail!("端点前缀「{}」重复了", e.id);
            }
            if e.base_url.is_empty() {
                bail!("端点「{}」缺少地址", e.name);
            }
            url::Url::parse(&e.base_url).with_context(|| format!("端点「{}」的地址不正确", e.name))?;
            if e.proxy != PROXY_DIRECT && e.proxy != PROXY_SYSTEM && self.proxy(&e.proxy).is_none() {
                bail!("端点「{}」使用的代理不存在", e.name);
            }
            let mut names = HashSet::new();
            for m in &e.models {
                if m.name.is_empty() {
                    bail!("端点「{}」里有模型没有名字", e.name);
                }
                if !names.insert(m.name.as_str()) {
                    bail!("端点「{}」里的模型「{}」重复了", e.name, m.name);
                }
            }
        }
        let mut pids = HashSet::new();
        for p in &self.proxies {
            if p.id == PROXY_DIRECT || p.id == PROXY_SYSTEM || !pids.insert(p.id.as_str()) {
                bail!("代理 id「{}」重复或是保留字", p.id);
            }
        }
        Ok(())
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn from_toml(s: &str) -> Result<Self> {
        let c: Config = toml::from_str(s).context("配置文件格式不正确")?;
        Ok(c)
    }
}

/// 负责读写配置文件
pub struct ConfigFile {
    path: PathBuf,
}

impl ConfigFile {
    pub fn new(dir: &Path) -> Self {
        ConfigFile { path: dir.join("config.toml") }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Config> {
        if !self.path.exists() {
            return Ok(Config::default());
        }
        let s = std::fs::read_to_string(&self.path)?;
        Config::from_toml(&s)
    }

    pub fn save(&self, cfg: &Config) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, cfg.to_toml()?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}
