//! 模型价格表：从 LiteLLM 的公开价格表一键获取价格、上下文窗口、是否支持图片。
//! 下载后缓存到本地（pricing.json），离线也能用。

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// 查到的模型信息（价格单位：美元 / 百万 token）
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// 命中的价格表条目名
    pub matched: String,
    pub provider: String,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub context_window: u32,
    pub max_output: u32,
    pub vision: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingStatus {
    pub count: usize,
    /// 本地缓存的更新时间（毫秒），0 表示还没下载过
    pub updated_at: i64,
    pub url: String,
}

#[derive(Default)]
pub struct PriceTable {
    entries: HashMap<String, ModelInfo>,
    updated_at: i64,
}

fn per_m(v: &Value, key: &str) -> f64 {
    let x = v.get(key).and_then(Value::as_f64).unwrap_or(0.0) * 1_000_000.0;
    (x * 10_000.0).round() / 10_000.0
}

impl PriceTable {
    /// 解析 LiteLLM 格式的 JSON
    pub fn parse(raw: &str, updated_at: i64) -> Result<Self> {
        let v: Value = serde_json::from_str(raw).map_err(|e| anyhow!("价格表格式不正确：{e}"))?;
        let obj = v.as_object().ok_or_else(|| anyhow!("价格表格式不正确"))?;
        let mut entries = HashMap::new();
        for (k, e) in obj {
            let mode = e.get("mode").and_then(Value::as_str).unwrap_or("chat");
            if !matches!(mode, "chat" | "responses" | "completion") {
                continue;
            }
            let ctx = e.get("max_input_tokens").or_else(|| e.get("max_tokens")).and_then(Value::as_u64).unwrap_or(0);
            entries.insert(
                k.to_lowercase(),
                ModelInfo {
                    matched: k.clone(),
                    provider: e.get("litellm_provider").and_then(Value::as_str).unwrap_or("").to_string(),
                    input: per_m(e, "input_cost_per_token"),
                    output: per_m(e, "output_cost_per_token"),
                    cache_read: per_m(e, "cache_read_input_token_cost"),
                    cache_write: per_m(e, "cache_creation_input_token_cost"),
                    context_window: ctx.min(u32::MAX as u64) as u32,
                    max_output: e.get("max_output_tokens").and_then(Value::as_u64).unwrap_or(0).min(u32::MAX as u64) as u32,
                    vision: e.get("supports_vision").and_then(Value::as_bool).unwrap_or(false),
                },
            );
        }
        Ok(PriceTable { entries, updated_at })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn updated_at(&self) -> i64 {
        self.updated_at
    }

    /// 按模型名查找。`provider_hint` 用来在多个同名条目里挑一个（anthropic / openai / gemini / deepseek …）
    pub fn lookup(&self, model: &str, provider_hint: &str) -> Option<ModelInfo> {
        let base = crate::router::normalize_name(model).to_lowercase();
        let mut names = vec![base.clone()];
        // openrouter 之类的「厂商/模型」写法，以及 claude-sonnet-4.5 → claude-sonnet-4-5
        if let Some((_, tail)) = base.rsplit_once('/') {
            names.push(tail.to_string());
        }
        for n in names.clone() {
            if n.contains('.') {
                names.push(n.replace('.', "-"));
            }
            if let Some(s) = strip_date(&n) {
                names.push(s);
            }
        }
        for n in &names {
            if let Some(hit) = self.find(n, provider_hint) {
                return Some(hit);
            }
        }
        None
    }

    fn find(&self, name: &str, hint: &str) -> Option<ModelInfo> {
        let suffix = format!("/{name}");
        let mut cands: Vec<&ModelInfo> = self
            .entries
            .iter()
            .filter(|(k, _)| *k == name || k.ends_with(&suffix))
            .map(|(_, v)| v)
            .collect();
        if cands.is_empty() {
            return None;
        }
        // 优先：厂商匹配 > 没有前缀的原名 > 有价格的 > 名字短的
        cands.sort_by_key(|v| {
            let k = v.matched.to_lowercase();
            (
                !(hint.is_empty() || v.provider.starts_with(hint) || k.starts_with(&format!("{hint}/"))),
                k != name,
                v.input == 0.0,
                k.len(),
            )
        });
        cands.first().map(|v| (*v).clone())
    }
}

/// 去掉 -20250929 / -2025-08-07 这类日期后缀
fn strip_date(n: &str) -> Option<String> {
    let parts: Vec<&str> = n.split('-').collect();
    if let Some(last) = parts.last() {
        if last.len() == 8 && last.chars().all(|c| c.is_ascii_digit()) {
            return Some(parts[..parts.len() - 1].join("-"));
        }
    }
    if parts.len() >= 4 {
        let tail = &parts[parts.len() - 3..];
        if tail[0].len() == 4 && tail.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
            return Some(parts[..parts.len() - 3].join("-"));
        }
    }
    None
}

/// 根据端点猜厂商，用来在同名条目里挑
pub fn provider_hint(base_url: &str, protocol: crate::ir::Protocol) -> &'static str {
    let u = base_url.to_lowercase();
    for (needle, p) in [
        ("deepseek", "deepseek"),
        ("anthropic.com", "anthropic"),
        ("openai.com", "openai"),
        ("googleapis", "gemini"),
        ("openrouter", "openrouter"),
        ("moonshot", "moonshot"),
        ("x.ai", "xai"),
        ("mistral", "mistral"),
        ("dashscope", "dashscope"),
    ] {
        if u.contains(needle) {
            return p;
        }
    }
    match protocol {
        crate::ir::Protocol::Anthropic => "anthropic",
        crate::ir::Protocol::Gemini => "gemini",
        _ => "openai",
    }
}

/// 缓存文件管理
pub struct PriceCache {
    path: PathBuf,
    table: parking_lot::RwLock<PriceTable>,
}

impl PriceCache {
    pub fn new(dir: &Path) -> Self {
        let path = dir.join("pricing.json");
        let table = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| {
                let ts = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                PriceTable::parse(&s, ts).ok()
            })
            .unwrap_or_default();
        PriceCache { path, table: parking_lot::RwLock::new(table) }
    }

    pub fn status(&self, url: &str) -> PricingStatus {
        let t = self.table.read();
        PricingStatus { count: t.len(), updated_at: t.updated_at(), url: url.to_string() }
    }

    pub fn lookup(&self, model: &str, hint: &str) -> Option<ModelInfo> {
        self.table.read().lookup(model, hint)
    }

    pub fn is_empty(&self) -> bool {
        self.table.read().is_empty()
    }

    pub async fn refresh(&self, http: &reqwest::Client, url: &str) -> Result<PricingStatus> {
        let resp = tokio::time::timeout(std::time::Duration::from_secs(60), http.get(url).send())
            .await
            .map_err(|_| anyhow!("下载价格表超时，可以在设置里换一个出口或镜像地址"))??;
        if !resp.status().is_success() {
            return Err(anyhow!("下载价格表失败：HTTP {}", resp.status()));
        }
        let raw = resp.text().await?;
        let table = PriceTable::parse(&raw, crate::ir::now_millis())?;
        if table.is_empty() {
            return Err(anyhow!("价格表是空的"));
        }
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&self.path, raw)?;
        *self.table.write() = table;
        Ok(self.status(url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "sample_spec": {"mode": "chat"},
      "claude-sonnet-4-5": {"litellm_provider":"anthropic","mode":"chat","max_input_tokens":200000,"max_output_tokens":64000,"input_cost_per_token":3e-06,"output_cost_per_token":1.5e-05,"cache_read_input_token_cost":3e-07,"cache_creation_input_token_cost":3.75e-06,"supports_vision":true},
      "deepseek/deepseek-chat": {"litellm_provider":"deepseek","mode":"chat","max_input_tokens":131072,"input_cost_per_token":2.8e-07,"output_cost_per_token":4.2e-07,"cache_read_input_token_cost":2.8e-08},
      "gemini-2.5-pro": {"litellm_provider":"vertex_ai-language-models","mode":"chat","max_input_tokens":1048576,"input_cost_per_token":1.25e-06,"output_cost_per_token":1e-05,"supports_vision":true},
      "gemini/gemini-2.5-pro": {"litellm_provider":"gemini","mode":"chat","max_input_tokens":1048576,"input_cost_per_token":1.25e-06,"output_cost_per_token":1e-05,"supports_vision":true},
      "text-embedding-3-small": {"litellm_provider":"openai","mode":"embedding","input_cost_per_token":2e-08}
    }"#;

    #[test]
    fn lookups() {
        let t = PriceTable::parse(SAMPLE, 0).unwrap();
        let s = t.lookup("claude-sonnet-4-5-20250929", "anthropic").unwrap();
        assert_eq!((s.input, s.output, s.cache_read, s.cache_write), (3.0, 15.0, 0.3, 3.75));
        assert_eq!(s.context_window, 200000);
        assert!(s.vision);
        assert_eq!(t.lookup("anthropic/claude-sonnet-4.5", "openrouter").unwrap().matched, "claude-sonnet-4-5");
        let d = t.lookup("deepseek-chat", "deepseek").unwrap();
        assert_eq!(d.context_window, 131072);
        assert_eq!(d.input, 0.28);
        assert_eq!(t.lookup("gemini-2.5-pro", "gemini").unwrap().provider, "gemini");
        assert!(t.lookup("text-embedding-3-small", "openai").is_none());
        assert!(t.lookup("my-internal-model", "openai").is_none());
    }
}
