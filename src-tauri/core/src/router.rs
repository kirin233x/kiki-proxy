//! 模型名 → 端点的路由，以及故障切换链。

use crate::config::{Config, EndpointConfig, ModelConfig};
use crate::ir::Protocol;
use anyhow::{anyhow, Result};

/// Claude Code 的模型发现只保留名字里带 claude 的模型，所以给其他模型加这个前缀
pub const CLAUDE_ALIAS: &str = "claude@";

#[derive(Debug, Clone)]
pub struct Target {
    pub endpoint: EndpointConfig,
    pub model: Option<ModelConfig>,
    /// 网关模型名 `{端点}/{模型}`，日志和统计里用
    pub gateway_model: String,
    /// 发给上游的真实模型名
    pub upstream_model: String,
    /// 出口代理 id
    pub proxy: String,
    /// 命中的规则名（如果有）
    pub rule: Option<String>,
}

/// 去掉客户端可能加上的修饰：`claude@` 前缀、Claude Code 的 `[1m]` 后缀
pub fn normalize_name(name: &str) -> String {
    let mut n = name.trim();
    if let Some(rest) = n.strip_prefix(CLAUDE_ALIAS) {
        n = rest;
    }
    if let Some(i) = n.rfind('[') {
        if n.ends_with(']') {
            n = &n[..i];
        }
    }
    n.to_string()
}

/// 只支持 `*` 和 `?` 的通配符匹配（大小写不敏感）
pub fn glob(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let s: Vec<char> = s.to_lowercase().chars().collect();
    let (mut pi, mut si, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// 解析一个明确的网关模型名 `{端点}/{模型}`
pub fn resolve_direct(cfg: &Config, name: &str) -> Option<Target> {
    let (prefix, model) = name.split_once('/')?;
    let ep = cfg.endpoint(prefix).filter(|e| e.enabled)?;
    let m = ep.models.iter().find(|m| m.name == model || m.upstream == model).cloned();
    if m.as_ref().is_some_and(|m| !m.enabled) {
        return None;
    }
    let upstream_model = m.as_ref().map(|m| m.upstream_id().to_string()).unwrap_or_else(|| model.to_string());
    let gateway_model = format!("{prefix}/{}", m.as_ref().map(|m| m.name.as_str()).unwrap_or(model));
    Some(Target { endpoint: ep.clone(), model: m, gateway_model, upstream_model, proxy: ep.proxy.clone(), rule: None })
}

/// 客户端请求的模型名 → 第一个目标
pub fn resolve(cfg: &Config, requested: &str, client: Protocol) -> Result<Target> {
    let name = normalize_name(requested);
    for r in cfg.rules.iter().filter(|r| r.enabled && !r.pattern.is_empty()) {
        if r.client.is_some_and(|c| c != client) || !glob(&r.pattern, &name) {
            continue;
        }
        let target = if r.target.is_empty() {
            resolve_direct(cfg, &name)
        } else if let Some(prefix) = r.target.strip_suffix("/*") {
            // 保留原模型名，只换端点；原名带前缀时去掉前缀
            let bare = name.split_once('/').map(|(_, m)| m).unwrap_or(&name);
            resolve_direct(cfg, &format!("{prefix}/{bare}"))
        } else {
            resolve_direct(cfg, &r.target)
        };
        if let Some(mut t) = target {
            if !r.proxy.is_empty() {
                t.proxy = r.proxy.clone();
            }
            t.rule = Some(r.name.clone());
            return Ok(t);
        }
    }
    if let Some(t) = resolve_direct(cfg, &name) {
        return Ok(t);
    }
    if !cfg.routing.default_model.is_empty() {
        if let Some(mut t) = resolve_direct(cfg, &cfg.routing.default_model) {
            t.rule = Some("默认模型".into());
            return Ok(t);
        }
    }
    Err(anyhow!(
        "kiki-proxy 找不到模型「{requested}」：请使用「端点前缀/模型名」的形式，或在路由里配置规则 / 默认模型"
    ))
}

/// 故障切换链：第一个目标 + 它配置的备选（只展开一层，自动去重和跳过停用的）
pub fn failover_chain(cfg: &Config, first: Target) -> Vec<Target> {
    let fallbacks = match &first.model {
        Some(m) if !m.fallbacks.is_empty() => m.fallbacks.clone(),
        _ => first.endpoint.fallbacks.clone(),
    };
    let mut chain = vec![first];
    for f in fallbacks {
        if let Some(mut t) = resolve_direct(cfg, &f) {
            if chain.iter().any(|c| c.gateway_model == t.gateway_model) {
                continue;
            }
            t.rule = Some("故障切换".into());
            chain.push(t);
        }
    }
    chain
}

/// 路由测试用：一次请求会依次尝试哪些目标
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteHop {
    pub gateway_model: String,
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub protocol: Protocol,
    pub upstream_model: String,
    pub proxy: String,
    pub rule: String,
}

pub fn explain(cfg: &Config, requested: &str, client: Protocol) -> Result<Vec<RouteHop>> {
    let first = resolve(cfg, requested, client)?;
    Ok(failover_chain(cfg, first)
        .into_iter()
        .map(|t| RouteHop {
            gateway_model: t.gateway_model,
            endpoint_id: t.endpoint.id.clone(),
            endpoint_name: t.endpoint.name.clone(),
            protocol: t.endpoint.protocol,
            upstream_model: t.upstream_model,
            proxy: t.proxy,
            rule: t.rule.unwrap_or_default(),
        })
        .collect())
}

/// 网关对外提供的模型列表
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedModel {
    pub id: String,
    pub display_name: String,
    pub endpoint: String,
    pub vision: bool,
}

pub fn list_models(cfg: &Config) -> Vec<ListedModel> {
    let mut out = Vec::new();
    for e in cfg.endpoints.iter().filter(|e| e.enabled) {
        for m in e.models.iter().filter(|m| m.enabled) {
            out.push(ListedModel {
                id: format!("{}/{}", e.id, m.name),
                display_name: if m.display_name.is_empty() { format!("{}/{}", e.id, m.name) } else { m.display_name.clone() },
                endpoint: e.name.clone(),
                vision: m.vision,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    fn cfg() -> Config {
        let mut c = Config::default();
        c.endpoints.push(EndpointConfig {
            id: "internal".into(),
            name: "内部".into(),
            base_url: "https://x".into(),
            models: vec![ModelConfig { name: "sonnet".into(), upstream: "claude-sonnet-4-5".into(), ..Default::default() }],
            fallbacks: vec!["ds/deepseek-chat".into(), "internal/sonnet".into()],
            ..Default::default()
        });
        c.endpoints.push(EndpointConfig {
            id: "ds".into(),
            name: "DeepSeek".into(),
            protocol: Protocol::OpenaiChat,
            base_url: "https://api.deepseek.com".into(),
            models: vec![ModelConfig { name: "deepseek-chat".into(), ..Default::default() }],
            ..Default::default()
        });
        c.rules.push(RuleConfig {
            name: "haiku 走 ds".into(),
            pattern: "claude-*haiku*".into(),
            target: "ds/deepseek-chat".into(),
            proxy: "direct".into(),
            ..Default::default()
        });
        c
    }

    #[test]
    fn glob_works() {
        assert!(glob("claude-*haiku*", "claude-3-5-haiku-20241022"));
        assert!(glob("ds/*", "ds/x"));
        assert!(!glob("ds/*", "internal/x"));
        assert!(glob("a?c", "ABC"));
    }

    #[test]
    fn resolves_prefix_alias_and_rules() {
        let c = cfg();
        let t = resolve(&c, "internal/sonnet", Protocol::Anthropic).unwrap();
        assert_eq!(t.upstream_model, "claude-sonnet-4-5");
        let t = resolve(&c, "claude@ds/deepseek-chat[1m]", Protocol::Anthropic).unwrap();
        assert_eq!(t.gateway_model, "ds/deepseek-chat");
        let t = resolve(&c, "claude-haiku-4-5", Protocol::Anthropic).unwrap();
        assert_eq!(t.gateway_model, "ds/deepseek-chat");
        assert_eq!(t.proxy, "direct");
        // 没列出的模型名也允许直接透传给端点
        let t = resolve(&c, "ds/deepseek-reasoner", Protocol::OpenaiChat).unwrap();
        assert_eq!(t.upstream_model, "deepseek-reasoner");
        assert!(resolve(&c, "unknown", Protocol::OpenaiChat).is_err());
    }

    #[test]
    fn failover_chain_dedups() {
        let c = cfg();
        let t = resolve(&c, "internal/sonnet", Protocol::Anthropic).unwrap();
        let chain = failover_chain(&c, t);
        let names: Vec<_> = chain.iter().map(|t| t.gateway_model.as_str()).collect();
        assert_eq!(names, vec!["internal/sonnet", "ds/deepseek-chat"]);
    }
}
