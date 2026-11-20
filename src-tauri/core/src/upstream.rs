//! 出口代理（每种代理设置一个 HTTP 客户端，复用连接池）和上游 URL 拼接。

use crate::config::{AuthMode, Config, EndpointConfig, ProxyKind, PROXY_DIRECT, PROXY_SYSTEM};
use crate::ir::Protocol;
use crate::plugin::{keys, SecretStore};
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Default)]
pub struct Clients {
    cache: Mutex<HashMap<String, reqwest::Client>>,
}

impl Clients {
    /// 配置变了以后清空，下次按新设置重建
    pub fn clear(&self) {
        self.cache.lock().clear();
    }

    pub fn get(&self, cfg: &Config, proxy_id: &str, secrets: &dyn SecretStore) -> Result<reqwest::Client> {
        let (key, proxy) = match proxy_id {
            "" | PROXY_SYSTEM => (PROXY_SYSTEM.to_string(), None),
            PROXY_DIRECT => (PROXY_DIRECT.to_string(), None),
            id => {
                let p = cfg.proxy(id).ok_or_else(|| anyhow!("代理「{id}」不存在"))?;
                let pwd = secrets.get(&keys::proxy_password(id)).ok().flatten().unwrap_or_default();
                (format!("{id}|{:?}|{}|{}|{}", p.kind, p.url, p.username, pwd), Some((p.clone(), pwd)))
            }
        };
        if let Some(c) = self.cache.lock().get(&key) {
            return Ok(c.clone());
        }
        let mut b = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(30))
            .user_agent(concat!("kiki-proxy/", env!("CARGO_PKG_VERSION")));
        match (proxy_id, proxy) {
            (PROXY_DIRECT, _) => b = b.no_proxy(),
            (_, Some((p, pwd))) => {
                let url = proxy_url(p.kind, &p.url);
                let mut px = reqwest::Proxy::all(&url).map_err(|e| anyhow!("代理地址「{}」不正确：{e}", p.url))?;
                if !p.username.is_empty() {
                    px = px.basic_auth(&p.username, &pwd);
                }
                b = b.proxy(px);
            }
            _ => {} // system：reqwest 默认读取环境变量和 macOS 系统代理设置
        }
        let c = b.build()?;
        self.cache.lock().insert(key, c.clone());
        Ok(c)
    }
}

fn proxy_url(kind: ProxyKind, raw: &str) -> String {
    let raw = raw.trim();
    if raw.contains("://") {
        // socks5:// 改成 socks5h://，让代理负责 DNS 解析（访问内网域名时需要）
        return raw.replacen("socks5://", "socks5h://", 1);
    }
    match kind {
        ProxyKind::Http => format!("http://{raw}"),
        ProxyKind::Socks5 => format!("socks5h://{raw}"),
    }
}

fn is_versioned(seg: &str) -> bool {
    let s = seg.strip_prefix('v').unwrap_or("");
    !s.is_empty() && s.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// 根据端点的 base_url 拼出请求地址。
/// base_url 可以带也可以不带版本号：`https://api.openai.com` 和 `https://api.openai.com/v1` 都行。
pub fn upstream_url(base: &str, p: Protocol, model: &str, stream: bool, count_tokens: bool) -> String {
    let b = base.trim().trim_end_matches('/');
    let versioned = b.rsplit('/').next().is_some_and(is_versioned);
    let v = |default: &str| if versioned { b.to_string() } else { format!("{b}/{default}") };
    match p {
        Protocol::Anthropic => {
            if count_tokens {
                format!("{}/messages/count_tokens", v("v1"))
            } else {
                format!("{}/messages", v("v1"))
            }
        }
        Protocol::OpenaiChat => format!("{}/chat/completions", v("v1")),
        Protocol::OpenaiResponses => format!("{}/responses", v("v1")),
        Protocol::Gemini => {
            let action = if count_tokens {
                "countTokens"
            } else if stream {
                "streamGenerateContent?alt=sse"
            } else {
                "generateContent"
            };
            format!("{}/models/{model}:{action}", v("v1beta"))
        }
    }
}

/// 端点自身的鉴权头 + 自定义头
pub fn base_headers(ep: &EndpointConfig, api_key: &str, stream: bool) -> Result<HeaderMap> {
    let mut h = HeaderMap::new();
    h.insert("content-type", HeaderValue::from_static("application/json"));
    h.insert("accept", HeaderValue::from_static(if stream { "text/event-stream" } else { "application/json" }));
    if ep.protocol == Protocol::Anthropic {
        h.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    }
    if !api_key.is_empty() {
        let mode = match ep.auth {
            AuthMode::Auto => match ep.protocol {
                Protocol::Anthropic => AuthMode::XApiKey,
                Protocol::Gemini => AuthMode::GoogApiKey,
                _ => AuthMode::Bearer,
            },
            m => m,
        };
        match mode {
            AuthMode::Bearer => {
                h.insert("authorization", HeaderValue::from_str(&format!("Bearer {api_key}"))?);
            }
            AuthMode::XApiKey => {
                h.insert("x-api-key", HeaderValue::from_str(api_key)?);
            }
            AuthMode::GoogApiKey => {
                h.insert("x-goog-api-key", HeaderValue::from_str(api_key)?);
            }
            _ => {}
        }
    }
    for (k, v) in &ep.headers {
        h.insert(HeaderName::from_bytes(k.trim().as_bytes())?, HeaderValue::from_str(v.trim())?);
    }
    Ok(h)
}

/// 给日志用：把敏感的头打码
pub fn mask_headers(h: &HeaderMap) -> String {
    let mut lines = Vec::new();
    for (k, v) in h {
        let name = k.as_str();
        let val = v.to_str().unwrap_or("<binary>");
        let sensitive = matches!(name, "authorization" | "x-api-key" | "x-goog-api-key" | "cookie" | "proxy-authorization")
            || name.contains("token")
            || name.contains("secret");
        let shown = if sensitive {
            let tail: String = val.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            format!("••••{tail}")
        } else {
            val.to_string()
        };
        lines.push(format!("{name}: {shown}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(upstream_url("https://api.openai.com/v1/", Protocol::OpenaiChat, "", false, false), "https://api.openai.com/v1/chat/completions");
        assert_eq!(upstream_url("https://api.deepseek.com", Protocol::OpenaiChat, "", false, false), "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(upstream_url("https://api.deepseek.com/anthropic", Protocol::Anthropic, "", true, false), "https://api.deepseek.com/anthropic/v1/messages");
        assert_eq!(
            upstream_url("https://generativelanguage.googleapis.com", Protocol::Gemini, "gemini-2.5-pro", true, false),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
        assert_eq!(upstream_url("https://x.com/api/v3", Protocol::OpenaiResponses, "", true, false), "https://x.com/api/v3/responses");
    }

    #[test]
    fn proxy_urls() {
        assert_eq!(proxy_url(ProxyKind::Socks5, "127.0.0.1:7890"), "socks5h://127.0.0.1:7890");
        assert_eq!(proxy_url(ProxyKind::Http, "proxy.corp:8080"), "http://proxy.corp:8080");
        assert_eq!(proxy_url(ProxyKind::Socks5, "socks5://a:1"), "socks5h://a:1");
    }
}
