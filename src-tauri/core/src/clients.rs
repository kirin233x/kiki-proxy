//! 一键把客户端（Claude Code / Codex / Gemini CLI）指向本地网关。
//! 修改前会把原文件备份成 `*.kiki-backup`，可以一键恢复。

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    /// 当前配置已经指向本网关
    pub connected: bool,
    pub has_backup: bool,
    /// 当前配置的模型
    pub model: String,
    pub max_context_tokens: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApplyOptions {
    /// 默认模型（网关模型名）
    pub model: String,
    /// Claude Code：后台小任务用的模型（ANTHROPIC_DEFAULT_HAIKU_MODEL）
    pub small_model: String,
    /// Claude Code：开启从网关拉取模型列表（/model 里能看到所有模型）
    pub discovery: bool,
    /// Claude Code：CLAUDE_CODE_MAX_CONTEXT_TOKENS，不带 [1m] 的模型按这个窗口压缩上下文；0 表示不设置
    pub max_context_tokens: u32,
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| anyhow!("找不到用户目录"))
}

fn claude_path() -> Result<PathBuf> {
    Ok(home()?.join(".claude/settings.json"))
}
fn codex_path() -> Result<PathBuf> {
    Ok(home()?.join(".codex/config.toml"))
}
fn gemini_env_path() -> Result<PathBuf> {
    Ok(home()?.join(".gemini/.env"))
}
fn gemini_settings_path() -> Result<PathBuf> {
    Ok(home()?.join(".gemini/settings.json"))
}

fn backup_path(p: &std::path::Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".kiki-backup");
    PathBuf::from(s)
}

/// 第一次修改前备份；原来没有文件时写一个空标记，恢复时删除文件
fn backup(p: &std::path::Path) -> Result<()> {
    let b = backup_path(p);
    if b.exists() {
        return Ok(());
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    if p.exists() {
        std::fs::copy(p, &b)?;
    } else {
        std::fs::write(&b, "")?;
    }
    Ok(())
}

fn restore_file(p: &std::path::Path) -> Result<()> {
    let b = backup_path(p);
    if !b.exists() {
        return Ok(());
    }
    let content = std::fs::read(&b)?;
    if content.is_empty() {
        let _ = std::fs::remove_file(p);
    } else {
        std::fs::write(p, content)?;
    }
    std::fs::remove_file(&b)?;
    Ok(())
}

fn read_json(p: &std::path::Path) -> Result<Value> {
    if !p.exists() {
        return Ok(json!({}));
    }
    let s = std::fs::read_to_string(p)?;
    if s.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&s).with_context(|| format!("{} 不是合法的 JSON，请先手动修正", p.display()))
}

fn write_json(p: &std::path::Path, v: &Value) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(v)? + "\n")?;
    Ok(())
}

pub fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn token(key: &str) -> String {
    if key.is_empty() {
        "kiki-proxy".into()
    } else {
        key.into()
    }
}

pub fn list(port: u16) -> Vec<ClientInfo> {
    let base = base_url(port);
    let mut out = Vec::new();
    if let Ok(p) = claude_path() {
        let v = read_json(&p).unwrap_or_default();
        out.push(ClientInfo {
            id: "claude".into(),
            name: "Claude Code".into(),
            connected: v.pointer("/env/ANTHROPIC_BASE_URL").and_then(Value::as_str) == Some(base.as_str()),
            model: v.pointer("/env/ANTHROPIC_MODEL").and_then(Value::as_str).unwrap_or("").into(),
            max_context_tokens: v
                .pointer("/env/CLAUDE_CODE_MAX_CONTEXT_TOKENS")
                .and_then(|x| x.as_str().and_then(|s| s.parse().ok()).or_else(|| x.as_u64()))
                .unwrap_or(0) as u32,
            has_backup: backup_path(&p).exists(),
            path: p.display().to_string(),
        });
    }
    if let Ok(p) = codex_path() {
        let doc = std::fs::read_to_string(&p).unwrap_or_default().parse::<toml_edit::DocumentMut>().unwrap_or_default();
        let connected = doc.get("model_provider").and_then(|v| v.as_str()) == Some("kiki")
            && doc
                .get("model_providers")
                .and_then(|t| t.get("kiki"))
                .and_then(|t| t.get("base_url"))
                .and_then(|v| v.as_str())
                .is_some_and(|u| u.starts_with(&base));
        out.push(ClientInfo {
            id: "codex".into(),
            name: "Codex CLI".into(),
            connected,
            model: doc.get("model").and_then(|v| v.as_str()).unwrap_or("").into(),
            max_context_tokens: 0,
            has_backup: backup_path(&p).exists(),
            path: p.display().to_string(),
        });
    }
    if let Ok(p) = gemini_env_path() {
        let env = std::fs::read_to_string(&p).unwrap_or_default();
        let get = |k: &str| {
            env.lines().find_map(|l| l.strip_prefix(&format!("{k}=")).map(|v| v.trim_matches('"').to_string())).unwrap_or_default()
        };
        out.push(ClientInfo {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            connected: get("GOOGLE_GEMINI_BASE_URL") == base,
            model: get("GEMINI_MODEL"),
            max_context_tokens: 0,
            has_backup: backup_path(&p).exists(),
            path: p.display().to_string(),
        });
    }
    out
}

pub fn apply(client: &str, port: u16, access_key: &str, opts: &ApplyOptions) -> Result<()> {
    match client {
        "claude" => apply_claude(port, access_key, opts),
        "codex" => apply_codex(port, access_key, opts),
        "gemini" => apply_gemini(port, access_key, opts),
        _ => Err(anyhow!("未知客户端：{client}")),
    }
}

pub fn restore(client: &str) -> Result<()> {
    match client {
        "claude" => restore_file(&claude_path()?),
        "codex" => restore_file(&codex_path()?),
        "gemini" => {
            restore_file(&gemini_env_path()?)?;
            restore_file(&gemini_settings_path()?)
        }
        _ => Err(anyhow!("未知客户端：{client}")),
    }
}

fn apply_claude(port: u16, key: &str, opts: &ApplyOptions) -> Result<()> {
    let p = claude_path()?;
    let mut v = read_json(&p)?;
    backup(&p)?;
    if !v.is_object() {
        v = json!({});
    }
    if !v.get("env").is_some_and(Value::is_object) {
        v["env"] = json!({});
    }
    let env = v["env"].as_object_mut().unwrap();
    env.insert("ANTHROPIC_BASE_URL".into(), json!(base_url(port)));
    env.insert("ANTHROPIC_AUTH_TOKEN".into(), json!(token(key)));
    // 两个都设置时 Claude Code 会提示冲突，网关模式下用 AUTH_TOKEN
    env.remove("ANTHROPIC_API_KEY");
    if opts.discovery {
        env.insert("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY".into(), json!("1"));
    } else {
        env.remove("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY");
    }
    let set = |env: &mut serde_json::Map<String, Value>, k: &str, val: &str| {
        if val.is_empty() {
            env.remove(k);
        } else {
            env.insert(k.into(), json!(val));
        }
    };
    set(env, "ANTHROPIC_MODEL", &opts.model);
    set(env, "ANTHROPIC_DEFAULT_HAIKU_MODEL", &opts.small_model);
    let ctx = if opts.max_context_tokens > 0 { opts.max_context_tokens.to_string() } else { String::new() };
    set(env, "CLAUDE_CODE_MAX_CONTEXT_TOKENS", &ctx);
    write_json(&p, &v)
}

fn apply_codex(port: u16, key: &str, opts: &ApplyOptions) -> Result<()> {
    use toml_edit::{value, DocumentMut, Item, Table};
    let p = codex_path()?;
    let raw = if p.exists() { std::fs::read_to_string(&p)? } else { String::new() };
    let mut doc: DocumentMut = raw.parse().with_context(|| format!("{} 格式不正确，请先手动修正", p.display()))?;
    backup(&p)?;
    doc["model_provider"] = value("kiki");
    if !opts.model.is_empty() {
        doc["model"] = value(opts.model.as_str());
    }
    if !doc.contains_key("model_providers") {
        let mut t = Table::new();
        t.set_implicit(true);
        doc["model_providers"] = Item::Table(t);
    }
    let mut prov = Table::new();
    prov["name"] = value("kiki-proxy");
    prov["base_url"] = value(format!("{}/v1", base_url(port)));
    prov["wire_api"] = value("responses");
    if !key.is_empty() {
        // Codex 从环境变量读密钥
        prov["env_key"] = value("KIKI_PROXY_KEY");
    }
    doc["model_providers"]["kiki"] = Item::Table(prov);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, doc.to_string())?;
    Ok(())
}

fn apply_gemini(port: u16, key: &str, opts: &ApplyOptions) -> Result<()> {
    let p = gemini_env_path()?;
    let raw = if p.exists() { std::fs::read_to_string(&p)? } else { String::new() };
    backup(&p)?;
    let mut lines: Vec<String> = raw.lines().map(str::to_string).collect();
    let mut set = |k: &str, v: &str| {
        lines.retain(|l| !l.starts_with(&format!("{k}=")));
        if !v.is_empty() {
            lines.push(format!("{k}={v}"));
        }
    };
    set("GOOGLE_GEMINI_BASE_URL", &base_url(port));
    set("GEMINI_API_KEY", &token(key));
    set("GEMINI_MODEL", &opts.model);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, lines.join("\n") + "\n")?;
    // 让 Gemini CLI 使用 API Key 方式登录
    let sp = gemini_settings_path()?;
    let mut s = read_json(&sp)?;
    backup(&sp)?;
    if !s.is_object() {
        s = json!({});
    }
    if !s.get("security").is_some_and(Value::is_object) {
        s["security"] = json!({});
    }
    if !s["security"].get("auth").is_some_and(Value::is_object) {
        s["security"]["auth"] = json!({});
    }
    s["security"]["auth"]["selectedType"] = json!("gemini-api-key");
    write_json(&sp, &s)
}

// ------------------------------------------------------------------ Claude Code 桌面版

/// 桌面版模型列表里的一项
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopModel {
    /// 发给网关的模型名（名字里没有 claude 的会加上 claude@ 前缀，桌面版只认 Claude 模型）
    pub name: String,
    /// 列表里显示的名字
    pub label: String,
    pub supports_1m: bool,
}

/// 桌面版「Import configuration…」可以导入的 JSON 配置（键名见 Claude 桌面版第三方推理配置文档）
pub fn desktop_config(port: u16, access_key: &str, models: &[DesktopModel]) -> Value {
    let list: Vec<Value> = models
        .iter()
        .map(|m| {
            let lower = m.name.to_lowercase();
            let name = if lower.contains("claude") || lower.contains("anthropic") {
                m.name.clone()
            } else {
                format!("{}{}", crate::router::CLAUDE_ALIAS, m.name)
            };
            let mut o = json!({ "name": name });
            if !m.label.is_empty() {
                o["labelOverride"] = json!(m.label);
            }
            if m.supports_1m {
                o["supports1m"] = json!(true);
            }
            o
        })
        .collect();
    json!({
        "inferenceProvider": "gateway",
        "inferenceGatewayBaseUrl": base_url(port),
        "inferenceGatewayApiKey": token(access_key),
        "inferenceGatewayAuthScheme": "bearer",
        "inferenceModels": list,
    })
}

/// 写到「下载」文件夹，返回文件路径
pub fn write_desktop_config(port: u16, access_key: &str, models: &[DesktopModel]) -> Result<PathBuf> {
    let dir = home()?.join("Downloads");
    let dir = if dir.is_dir() { dir } else { home()? };
    let p = dir.join("kiki-proxy-claude-desktop.json");
    write_json(&p, &desktop_config(port, access_key, models))?;
    Ok(p)
}

/// 桌面版当前应用的配置是否指向本网关（只是尽力检查：在它的配置库里找网关地址）
pub fn desktop_connected(port: u16) -> bool {
    let Ok(h) = home() else { return false };
    let dir = h.join("Library/Application Support/Claude-3p/configLibrary");
    let needle = base_url(port);
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .any(|e| std::fs::read_to_string(e.path()).is_ok_and(|s| s.contains(&needle)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_config_shape() {
        let v = desktop_config(
            41521,
            "",
            &[
                DesktopModel { name: "internal/sonnet".into(), label: "Sonnet（内部）".into(), supports_1m: true },
                DesktopModel { name: "anthropic/claude-haiku-4-5".into(), label: String::new(), supports_1m: false },
            ],
        );
        assert_eq!(v["inferenceProvider"], json!("gateway"));
        assert_eq!(v["inferenceGatewayBaseUrl"], json!("http://127.0.0.1:41521"));
        assert_eq!(v["inferenceGatewayApiKey"], json!("kiki-proxy"));
        assert_eq!(v["inferenceModels"][0], json!({"name":"claude@internal/sonnet","labelOverride":"Sonnet（内部）","supports1m":true}));
        assert_eq!(v["inferenceModels"][1], json!({"name":"anthropic/claude-haiku-4-5"}));
    }
}
