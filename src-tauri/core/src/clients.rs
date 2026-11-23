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
    /// Claude Code：/model 里「Opus」档的模型，留空跟默认模型一致
    pub opus_model: String,
    /// Claude Code：/model 里「Sonnet」档的模型，留空跟默认模型一致
    pub sonnet_model: String,
    /// Claude Code：「Haiku」档，后台小任务（生成标题、摘要）用，留空跟默认模型一致
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
            // 读回来时去掉 [1m] 后缀，界面上显示网关模型名
            model: v.pointer("/env/ANTHROPIC_MODEL").and_then(Value::as_str).map(crate::router::normalize_name).unwrap_or_default(),
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


/// 一次接入改了哪些东西（给界面展示）
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub file: String,
    /// 每一项改动，例如「设置 ANTHROPIC_MODEL = internal/sonnet」「删除 apiKeyHelper（原来是 …）」
    pub changes: Vec<String>,
}

struct Changes(Vec<String>);

impl Changes {
    fn show(v: &Value) -> String {
        let s = match v {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        if s.chars().count() > 60 {
            format!("{}…", s.chars().take(60).collect::<String>())
        } else {
            s
        }
    }
    /// 设置一个值；没变化不记
    fn set(&mut self, map: &mut serde_json::Map<String, Value>, prefix: &str, k: &str, v: Value) {
        let old = map.get(k).cloned();
        if old.as_ref() == Some(&v) {
            return;
        }
        let shown = if is_secret(k) { "••••".to_string() } else { Self::show(&v) };
        self.0.push(match old {
            Some(o) if !is_secret(k) => format!("修改 {prefix}{k}：{} → {shown}", Self::show(&o)),
            _ => format!("设置 {prefix}{k} = {shown}"),
        });
        map.insert(k.to_string(), v);
    }
    fn remove(&mut self, map: &mut serde_json::Map<String, Value>, prefix: &str, k: &str, why: &str) {
        if let Some(o) = map.remove(k) {
            let old = if is_secret(k) { "••••".to_string() } else { Self::show(&o) };
            self.0.push(format!("删除 {prefix}{k}（原来是 {old}；{why}）"));
        }
    }
}

fn is_secret(k: &str) -> bool {
    let k = k.to_uppercase();
    k.contains("KEY") || k.contains("TOKEN") || k.contains("SECRET") || k.contains("HELPER")
}

/// 网关模型的显示名和是否 1M（从配置里查；查不到就用原名）
struct ModelMeta {
    id: String,
    name: String,
    is_1m: bool,
    context: u32,
}

fn meta(cfg: &crate::config::Config, id: &str) -> ModelMeta {
    let t = crate::router::resolve_direct(cfg, &crate::router::normalize_name(id));
    let m = t.as_ref().and_then(|t| t.model.clone());
    let gateway_id = t.as_ref().map(|t| t.gateway_model.clone()).unwrap_or_else(|| crate::router::normalize_name(id));
    ModelMeta {
        name: m.as_ref().map(|m| m.display_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| gateway_id.clone()),
        is_1m: m.as_ref().is_some_and(|m| m.context_window >= 1_000_000),
        context: m.as_ref().map(|m| m.context_window).unwrap_or(0),
        id: gateway_id,
    }
}

pub fn apply(client: &str, cfg: &crate::config::Config, opts: &ApplyOptions) -> Result<ApplyReport> {
    let (port, key) = (cfg.server.port, cfg.server.access_key.as_str());
    match client {
        "claude" => apply_claude(port, key, cfg, opts),
        "codex" => apply_codex(port, key, cfg, opts),
        "gemini" => apply_gemini(port, key, opts),
        _ => Err(anyhow!("未知客户端：{client}")),
    }
}

fn apply_claude(port: u16, key: &str, cfg: &crate::config::Config, opts: &ApplyOptions) -> Result<ApplyReport> {
    let p = claude_path()?;
    let mut v = read_json(&p)?;
    backup(&p)?;
    if !v.is_object() {
        v = json!({});
    }
    let mut ch = Changes(Vec::new());
    let root = v.as_object_mut().unwrap();
    if !root.get("env").is_some_and(Value::is_object) {
        root.insert("env".into(), json!({}));
    }

    // 顶层设置：固定的模型、apiKeyHelper 都会让请求不按网关来
    {
        if opts.model.is_empty() {
            ch.remove(root, "", "model", "会把会话固定在这个模型上");
        } else {
            let m = meta(cfg, &opts.model);
            let id = if m.is_1m { format!("{}[1m]", m.id) } else { m.id.clone() };
            ch.set(root, "", "model", json!(id));
        }
        ch.remove(root, "", "apiKeyHelper", "它会顶掉网关用的 token");
    }

    let env = root.get_mut("env").unwrap().as_object_mut().unwrap();
    let e = "env.";
    ch.set(env, e, "ANTHROPIC_BASE_URL", json!(base_url(port)));
    ch.set(env, e, "ANTHROPIC_AUTH_TOKEN", json!(token(key)));
    // 两个都设置时 Claude Code 会提示冲突，网关模式下用 AUTH_TOKEN
    ch.remove(env, e, "ANTHROPIC_API_KEY", "和 ANTHROPIC_AUTH_TOKEN 冲突");
    // 这些会让 Claude Code 直接去连 Bedrock / Vertex / Foundry，绕过网关
    for k in ["CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY", "CLAUDE_CODE_USE_MANTLE"] {
        ch.remove(env, e, k, "会绕过网关");
    }
    if opts.discovery {
        ch.set(env, e, "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY", json!("1"));
    } else {
        ch.remove(env, e, "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY", "已关闭模型发现");
    }

    // 主模型
    if opts.model.is_empty() {
        ch.remove(env, e, "ANTHROPIC_MODEL", "没有指定默认模型");
    } else {
        let m = meta(cfg, &opts.model);
        ch.set(env, e, "ANTHROPIC_MODEL", json!(if m.is_1m { format!("{}[1m]", m.id) } else { m.id }));
    }

    // /model 里的 Opus / Sonnet / Haiku 三档：没单独指定就跟主模型一致
    let tiers = [("OPUS", &opts.opus_model), ("SONNET", &opts.sonnet_model), ("HAIKU", &opts.small_model)];
    for (tier, chosen) in tiers {
        let id = if chosen.is_empty() { &opts.model } else { chosen };
        let base = format!("ANTHROPIC_DEFAULT_{tier}_MODEL");
        if id.is_empty() {
            for suffix in ["", "_NAME", "_DESCRIPTION", "_SUPPORTED_CAPABILITIES"] {
                ch.remove(env, e, &format!("{base}{suffix}"), "没有指定模型，恢复 Claude Code 默认");
            }
            continue;
        }
        let m = meta(cfg, id);
        ch.set(env, e, &base, json!(if m.is_1m { format!("{}[1m]", m.id) } else { m.id.clone() }));
        ch.set(env, e, &format!("{base}_NAME"), json!(m.name));
        ch.set(env, e, &format!("{base}_DESCRIPTION"), json!(format!("{} · kiki-proxy", m.id)));
        ch.remove(env, e, &format!("{base}_SUPPORTED_CAPABILITIES"), "网关模式下不生效");
    }
    // Fable 档、旧版小模型、子代理模型、自定义模型选项：都可能残留旧服务的模型名
    for suffix in ["", "_NAME", "_DESCRIPTION", "_SUPPORTED_CAPABILITIES"] {
        ch.remove(env, e, &format!("ANTHROPIC_DEFAULT_FABLE_MODEL{suffix}"), "旧服务的模型设置");
    }
    ch.remove(env, e, "ANTHROPIC_SMALL_FAST_MODEL", "旧版设置，已由 Haiku 档代替");
    ch.remove(env, e, "CLAUDE_CODE_SUBAGENT_MODEL", "会让子代理固定用这个模型");
    for k in ["ANTHROPIC_CUSTOM_MODEL_OPTION", "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME", "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION"] {
        ch.remove(env, e, k, "模型列表改由网关提供");
    }

    if opts.max_context_tokens > 0 {
        ch.set(env, e, "CLAUDE_CODE_MAX_CONTEXT_TOKENS", json!(opts.max_context_tokens.to_string()));
    } else {
        ch.remove(env, e, "CLAUDE_CODE_MAX_CONTEXT_TOKENS", "没有设置上下文窗口");
    }
    write_json(&p, &v)?;
    Ok(ApplyReport { file: p.display().to_string(), changes: ch.0 })
}

fn apply_codex(port: u16, key: &str, cfg: &crate::config::Config, opts: &ApplyOptions) -> Result<ApplyReport> {
    use toml_edit::{value, DocumentMut, Item, Table};
    let p = codex_path()?;
    let raw = if p.exists() { std::fs::read_to_string(&p)? } else { String::new() };
    let mut doc: DocumentMut = raw.parse().with_context(|| format!("{} 格式不正确，请先手动修正", p.display()))?;
    backup(&p)?;
    let mut ch = Vec::new();
    let set = |doc: &mut DocumentMut, k: &str, v: &str, ch: &mut Vec<String>| {
        let old = doc.get(k).and_then(|x| x.as_str()).map(str::to_string);
        if old.as_deref() != Some(v) {
            ch.push(match old {
                Some(o) => format!("修改 {k}：{o} → {v}"),
                None => format!("设置 {k} = {v}"),
            });
            doc[k] = value(v);
        }
    };
    set(&mut doc, "model_provider", "kiki", &mut ch);
    let m = (!opts.model.is_empty()).then(|| meta(cfg, &opts.model));
    if let Some(m) = &m {
        set(&mut doc, "model", &m.id, &mut ch);
    }
    // profile 会覆盖上面的 model_provider / model
    if let Some(old) = doc.remove("profile") {
        ch.push(format!("删除 profile（原来是 {}；它会覆盖网关设置）", old.as_str().unwrap_or("?")));
    }
    // 上下文窗口：跟着新模型走，旧值删掉
    match m.as_ref().filter(|m| m.context > 0) {
        Some(m) => {
            let old = doc.get("model_context_window").and_then(|x| x.as_integer());
            if old != Some(m.context as i64) {
                doc["model_context_window"] = value(m.context as i64);
                ch.push(format!("设置 model_context_window = {}", m.context));
            }
        }
        None => {
            if let Some(old) = doc.remove("model_context_window") {
                ch.push(format!("删除 model_context_window（原来是 {}；属于之前的模型）", old.to_string().trim()));
            }
        }
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
    ch.push(format!("设置 [model_providers.kiki]：base_url = {}/v1，wire_api = responses", base_url(port)));
    doc["model_providers"]["kiki"] = Item::Table(prov);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, doc.to_string())?;
    Ok(ApplyReport { file: p.display().to_string(), changes: ch })
}

fn apply_gemini(port: u16, key: &str, opts: &ApplyOptions) -> Result<ApplyReport> {
    let p = gemini_env_path()?;
    let raw = if p.exists() { std::fs::read_to_string(&p)? } else { String::new() };
    backup(&p)?;
    let mut lines: Vec<String> = raw.lines().map(str::to_string).collect();
    let mut ch = Vec::new();
    let key_of = |l: &str| l.trim().strip_prefix("export ").unwrap_or(l.trim()).split_once('=').map(|(k, v)| (k.trim().to_string(), v.to_string()));
    let mut set = |k: &str, v: &str, why: &str, ch: &mut Vec<String>| {
        let old = lines.iter().find_map(|l| key_of(l).filter(|(kk, _)| kk == k).map(|(_, v)| v));
        lines.retain(|l| key_of(l).map(|(kk, _)| kk != k).unwrap_or(true));
        let shown = |x: &str| if is_secret(k) { "••••".to_string() } else { x.to_string() };
        match (old, v.is_empty()) {
            (Some(o), true) => ch.push(format!("删除 {k}（原来是 {}；{why}）", shown(&o))),
            (Some(o), false) if o.trim_matches('"') != v => ch.push(format!("修改 {k}：{} → {}", shown(&o), shown(v))),
            (None, false) => ch.push(format!("设置 {k} = {}", shown(v))),
            _ => {}
        }
        if !v.is_empty() {
            lines.push(format!("{k}={v}"));
        }
    };
    set("GOOGLE_GEMINI_BASE_URL", &base_url(port), "", &mut ch);
    set("GEMINI_API_KEY", &token(key), "", &mut ch);
    set("GEMINI_MODEL", &opts.model, "没有指定默认模型", &mut ch);
    // 这两个会让 Gemini CLI 改走 Vertex AI / Google 账号，绕过网关
    set("GOOGLE_GENAI_USE_VERTEXAI", "", "会绕过网关", &mut ch);
    set("GOOGLE_GENAI_USE_GCA", "", "会绕过网关", &mut ch);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, lines.join("\n") + "\n")?;

    // settings.json：用 API Key 方式登录；里面固定的模型也改成新的
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
    if s["security"]["auth"]["selectedType"] != json!("gemini-api-key") {
        ch.push("设置 settings.json security.auth.selectedType = gemini-api-key".into());
        s["security"]["auth"]["selectedType"] = json!("gemini-api-key");
    }
    if !opts.model.is_empty() {
        match s.get("model").cloned() {
            Some(Value::String(old)) if old != opts.model => {
                ch.push(format!("修改 settings.json model：{old} → {}", opts.model));
                s["model"] = json!(opts.model);
            }
            Some(Value::Object(o)) if o.get("name").and_then(Value::as_str).is_some_and(|n| n != opts.model) => {
                ch.push(format!("修改 settings.json model.name：{} → {}", o["name"].as_str().unwrap_or(""), opts.model));
                s["model"]["name"] = json!(opts.model);
            }
            _ => {}
        }
    }
    write_json(&sp, &s)?;
    Ok(ApplyReport { file: format!("{}、{}", p.display(), sp.display()), changes: ch })
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




// ------------------------------------------------------------------ Claude Code 桌面版

/// kiki-proxy 写进桌面版配置库的配置 id（固定一个，重复应用时原地更新）
pub const DESKTOP_PROFILE_ID: &str = "6b696b69-7072-4f78-8079-6b696b690001";
/// 桌面版请求走的路径前缀：这个前缀下的模型名按「桌面版模型路由」换成真实模型
pub const DESKTOP_PATH: &str = "/claude-desktop";

pub fn desktop_base_url(port: u16) -> String {
    format!("{}{DESKTOP_PATH}", base_url(port))
}

/// 桌面版「第三方推理」配置（键名见 Claude 桌面版文档，写法和 cc-switch 一致）
pub fn desktop_config(port: u16, access_key: &str, routes: &[crate::config::DesktopRoute]) -> Value {
    let models: Vec<Value> = routes
        .iter()
        .map(|r| {
            let label = if r.label.trim().is_empty() { r.model.as_str() } else { r.label.trim() };
            let mut o = json!({ "name": r.id, "labelOverride": label });
            if r.supports_1m {
                o["supports1m"] = json!(true);
                // 只列一行，交给 Claude Code 时自动按 1M 用
                o["prefer1m"] = json!(true);
            }
            o
        })
        .collect();
    json!({
        "inferenceProvider": "gateway",
        "inferenceGatewayBaseUrl": desktop_base_url(port),
        "inferenceGatewayApiKey": token(access_key),
        "inferenceGatewayAuthScheme": "bearer",
        "inferenceModels": models,
        "modelDiscoveryEnabled": true,
    })
}

/// 桌面版能接受的模型名：`claude-`（或 `anthropic/claude-`）+ sonnet/opus/haiku/fable + `-` + 版本
pub fn is_desktop_safe_id(id: &str) -> bool {
    let l = id.trim().to_ascii_lowercase();
    let Some(tail) = l.strip_prefix("anthropic/claude-").or_else(|| l.strip_prefix("claude-")) else {
        return false;
    };
    tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && ["sonnet-", "opus-", "haiku-", "fable-"]
            .iter()
            .any(|p| tail.strip_prefix(p).is_some_and(|rest| !rest.is_empty()))
}

/// 写到「下载」文件夹，返回文件路径
pub fn write_desktop_config(port: u16, access_key: &str, routes: &[crate::config::DesktopRoute]) -> Result<PathBuf> {
    let dir = home()?.join("Downloads");
    let dir = if dir.is_dir() { dir } else { home()? };
    let p = dir.join("kiki-proxy-claude-desktop.json");
    write_json(&p, &desktop_config(port, access_key, routes))?;
    Ok(p)
}

fn desktop_dirs() -> Result<(PathBuf, PathBuf)> {
    let base = home()?.join("Library/Application Support");
    Ok((base.join("Claude"), base.join("Claude-3p")))
}

fn read_json_obj(p: &std::path::Path) -> Value {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// 直接写进桌面版的配置库并设为当前配置（和 cc-switch 的做法一样），重启桌面版生效
pub fn apply_desktop(port: u16, access_key: &str, routes: &[crate::config::DesktopRoute]) -> Result<PathBuf> {
    if routes.is_empty() {
        return Err(anyhow!("至少选择一个模型"));
    }
    let (normal, threep) = desktop_dirs()?;
    let lib = threep.join("configLibrary");
    let profile = lib.join(format!("{DESKTOP_PROFILE_ID}.json"));
    // 1. 配置本身：保留用户在桌面版里改过的其他项
    let mut p = read_json_obj(&profile);
    for (k, v) in desktop_config(port, access_key, routes).as_object().unwrap() {
        p[k] = v.clone();
    }
    write_json(&profile, &p)?;
    // 2. 配置库目录：登记并设为当前
    let meta_path = lib.join("_meta.json");
    backup(&meta_path)?;
    let mut meta = read_json_obj(&meta_path);
    let entries = meta["entries"].as_array().cloned().unwrap_or_default();
    let mut entries: Vec<Value> = entries.into_iter().filter(|e| e["id"] != json!(DESKTOP_PROFILE_ID)).collect();
    entries.push(json!({ "id": DESKTOP_PROFILE_ID, "name": "kiki-proxy" }));
    meta["entries"] = json!(entries);
    meta["appliedId"] = json!(DESKTOP_PROFILE_ID);
    write_json(&meta_path, &meta)?;
    // 3. 切到第三方模式
    for dir in [&normal, &threep] {
        let f = dir.join("claude_desktop_config.json");
        backup(&f)?;
        let mut v = read_json_obj(&f);
        v["deploymentMode"] = json!("3p");
        write_json(&f, &v)?;
    }
    Ok(profile)
}

/// 撤销 apply_desktop：恢复桌面版原来的配置，删掉 kiki-proxy 的配置
pub fn restore_desktop() -> Result<()> {
    let (normal, threep) = desktop_dirs()?;
    let lib = threep.join("configLibrary");
    for f in [lib.join("_meta.json"), normal.join("claude_desktop_config.json"), threep.join("claude_desktop_config.json")] {
        if backup_path(&f).exists() {
            restore_file(&f)?;
        }
    }
    let _ = std::fs::remove_file(lib.join(format!("{DESKTOP_PROFILE_ID}.json")));
    Ok(())
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

    /// 改 HOME 的测试不能并行
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 用户实际遇到的情况：cc-switch 留下的各档模型设置，接入后必须全部换成网关的模型
    #[test]
    fn claude_apply_replaces_leftovers() {
        use crate::config::*;
        let home = std::env::temp_dir().join(format!("kiki-home-{}", crate::ir::new_id("")));
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{
              "model": "gpt-6.2-sol[1M]",
              "apiKeyHelper": "~/bin/get-key.sh",
              "permissions": {"allow": ["Bash(ls)"]},
              "env": {
                "ANTHROPIC_BASE_URL": "https://old.example.com",
                "ANTHROPIC_API_KEY": "sk-old",
                "ANTHROPIC_MODEL": "gpt-6.2-sol[1M]",
                "ANTHROPIC_DEFAULT_FABLE_MODEL": "gpt-6.2-sol[1M]",
                "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME": "gpt-6.2-sol",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "internal/haiku",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "gpt-6.2-sol",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "gpt-6.2-sol[1M]",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "gpt-6.2-sol",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "gpt-6.2-sol[1M]",
                "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "gpt-6.2-sol",
                "ANTHROPIC_SMALL_FAST_MODEL": "gpt-6.2-sol",
                "CLAUDE_CODE_SUBAGENT_MODEL": "gpt-6.2-sol",
                "CLAUDE_CODE_USE_BEDROCK": "1",
                "MY_OWN_VAR": "keep me"
              }
            }"#,
        )
        .unwrap();
        // SAFETY: 只有这个测试改 HOME
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("HOME", &home) };

        let mut cfg = Config::default();
        cfg.endpoints.push(EndpointConfig {
            id: "internal".into(),
            name: "内部".into(),
            base_url: "http://127.0.0.1:8787".into(),
            models: vec![
                ModelConfig { name: "sonnet".into(), display_name: "Sonnet（内部）".into(), context_window: 1_000_000, ..Default::default() },
                ModelConfig { name: "haiku".into(), context_window: 200_000, ..Default::default() },
            ],
            ..Default::default()
        });
        let opts = ApplyOptions { model: "internal/sonnet".into(), small_model: "internal/haiku".into(), discovery: true, ..Default::default() };
        let report = apply("claude", &cfg, &opts).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap()).unwrap();
        let env = &v["env"];
        assert_eq!(env["ANTHROPIC_MODEL"], json!("internal/sonnet[1m]"));
        // Opus / Sonnet 没单独指定 → 跟主模型一致，名字用显示名
        for tier in ["OPUS", "SONNET"] {
            assert_eq!(env[format!("ANTHROPIC_DEFAULT_{tier}_MODEL")], json!("internal/sonnet[1m]"));
            assert_eq!(env[format!("ANTHROPIC_DEFAULT_{tier}_MODEL_NAME")], json!("Sonnet（内部）"));
        }
        assert_eq!(env["ANTHROPIC_DEFAULT_HAIKU_MODEL"], json!("internal/haiku"));
        assert_eq!(env["ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"], json!("internal/haiku"));
        assert_eq!(env["ANTHROPIC_BASE_URL"], json!("http://127.0.0.1:41521"));
        for gone in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_SMALL_FAST_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_USE_BEDROCK",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
        ] {
            assert!(env.get(gone).is_none(), "{gone} 应该被删掉");
        }
        assert_eq!(v["model"], json!("internal/sonnet[1m]"));
        assert!(v.get("apiKeyHelper").is_none());
        // 和网关无关的设置原样保留
        assert_eq!(env["MY_OWN_VAR"], json!("keep me"));
        assert_eq!(v["permissions"]["allow"][0], json!("Bash(ls)"));
        // 改动清单里不显示密钥原文
        let all = report.changes.join("\n");
        assert!(all.contains("ANTHROPIC_DEFAULT_OPUS_MODEL：gpt-6.2-sol[1M] → internal/sonnet[1m]"), "{all}");
        assert!(!all.contains("sk-old"), "{all}");
        assert!(home.join(".claude/settings.json.kiki-backup").exists());

        // 接入后检查应该干净；模拟别的程序（比如 cc-switch）把旧配置写回去，检查要能查出来
        assert!(inspect_claude(&cfg).issues.is_empty(), "{:?}", inspect_claude(&cfg).issues);
        let mut v2 = v.clone();
        v2["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"] = json!("gpt-6.2-sol[1M]");
        v2["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"] = json!("gpt-6.2-sol");
        std::fs::write(home.join(".claude/settings.json"), v2.to_string()).unwrap();
        let found: Vec<String> = inspect_claude(&cfg).issues.into_iter().map(|i| i.key).collect();
        assert!(found.contains(&"ANTHROPIC_DEFAULT_OPUS_MODEL".to_string()), "{found:?}");
        assert!(found.contains(&"ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME".to_string()), "{found:?}");
    }

    #[test]
    fn desktop_config_shape() {
        use crate::config::DesktopRoute;
        let routes = vec![
            DesktopRoute { id: "claude-sonnet-5".into(), model: "internal/sonnet".into(), label: "Sonnet（内部）".into(), supports_1m: true },
            DesktopRoute { id: "claude-sonnet-5-r2".into(), model: "ds/deepseek-chat".into(), label: String::new(), supports_1m: false },
        ];
        let v = desktop_config(41521, "", &routes);
        assert_eq!(v["inferenceProvider"], json!("gateway"));
        assert_eq!(v["inferenceGatewayBaseUrl"], json!("http://127.0.0.1:41521/claude-desktop"));
        assert_eq!(v["inferenceGatewayApiKey"], json!("kiki-proxy"));
        assert_eq!(
            v["inferenceModels"][0],
            json!({"name":"claude-sonnet-5","labelOverride":"Sonnet（内部）","supports1m":true,"prefer1m":true})
        );
        assert_eq!(v["inferenceModels"][1], json!({"name":"claude-sonnet-5-r2","labelOverride":"ds/deepseek-chat"}));
        // 写出去的名字必须全部能被桌面版接受，否则整个列表无效
        for m in v["inferenceModels"].as_array().unwrap() {
            assert!(is_desktop_safe_id(m["name"].as_str().unwrap()));
        }
        for bad in ["claude@internal/sonnet", "claude-sonnet-4-6@internal/sonnet", "internal/sonnet", "claude-sonnet-", "claude-sonnet-5[1m]"] {
            assert!(!is_desktop_safe_id(bad), "{bad}");
        }
    }

    #[test]
    fn desktop_apply_and_restore() {
        use crate::config::DesktopRoute;
        let home = std::env::temp_dir().join(format!("kiki-home-{}", crate::ir::new_id("")));
        let threep = home.join("Library/Application Support/Claude-3p");
        std::fs::create_dir_all(threep.join("configLibrary")).unwrap();
        let other = json!({"entries":[{"id":"aaaa","name":"Mine"}],"appliedId":"aaaa"});
        std::fs::write(threep.join("configLibrary/_meta.json"), other.to_string()).unwrap();
        std::fs::write(threep.join("claude_desktop_config.json"), r#"{"deploymentMode":"1p","keep":1}"#).unwrap();
        let _g = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("HOME", &home) };
        let routes = vec![DesktopRoute { id: "claude-opus-5".into(), model: "internal/opus".into(), label: String::new(), supports_1m: false }];
        apply_desktop(41521, "", &routes).unwrap();
        let meta = read_json_obj(&threep.join("configLibrary/_meta.json"));
        assert_eq!(meta["appliedId"], json!(DESKTOP_PROFILE_ID));
        assert_eq!(meta["entries"].as_array().unwrap().len(), 2);
        let c = read_json_obj(&threep.join("claude_desktop_config.json"));
        assert_eq!(c, json!({"deploymentMode":"3p","keep":1}));
        let p = read_json_obj(&threep.join(format!("configLibrary/{DESKTOP_PROFILE_ID}.json")));
        assert_eq!(p["inferenceModels"][0]["name"], json!("claude-opus-5"));
        assert!(desktop_connected(41521));
        // 再应用一次不会重复登记
        apply_desktop(41521, "", &routes).unwrap();
        assert_eq!(read_json_obj(&threep.join("configLibrary/_meta.json"))["entries"].as_array().unwrap().len(), 2);
        restore_desktop().unwrap();
        assert_eq!(read_json_obj(&threep.join("configLibrary/_meta.json")), other);
        assert_eq!(read_json_obj(&threep.join("claude_desktop_config.json")), json!({"deploymentMode":"1p","keep":1}));
        assert!(!threep.join(format!("configLibrary/{DESKTOP_PROFILE_ID}.json")).exists());
        assert!(!home.join("Library/Application Support/Claude/claude_desktop_config.json").exists());
    }
}

// ------------------------------------------------------------------ Claude Code 当前生效配置检查

/// 一个会让 Claude Code 不按网关来的配置项
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigIssue {
    pub file: String,
    pub key: String,
    pub value: String,
    pub problem: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFileState {
    pub path: String,
    /// 最后修改时间（毫秒）
    pub modified: i64,
    /// 这个文件里关于模型、地址的设置（键 → 值，密钥打码）
    pub entries: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeInspection {
    pub files: Vec<ConfigFileState>,
    pub issues: Vec<ConfigIssue>,
}

/// Claude Code 会读的配置文件（优先级从高到低）
fn claude_files() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json")];
    if let Ok(h) = home() {
        v.push(h.join(".claude/settings.local.json"));
        v.push(h.join(".claude/settings.json"));
    }
    v
}

const MODEL_KEYS: [&str; 7] = [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];
const BYPASS_KEYS: [&str; 4] = ["CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY", "CLAUDE_CODE_USE_MANTLE"];

/// 检查一个模型名：网关里能不能直接找到；找不到的话会被规则 / 兜底模型接走还是直接报错
fn model_problem(cfg: &crate::config::Config, v: &str) -> Option<String> {
    let n = crate::router::normalize_name(v);
    if crate::router::resolve_direct(cfg, &n).is_some() {
        return None;
    }
    Some(match crate::router::resolve(cfg, v, crate::ir::Protocol::Anthropic) {
        Ok(t) => format!("不是网关里的模型名，会被{}转到 {}", t.rule.as_deref().unwrap_or("路由"), t.gateway_model),
        Err(_) => "不是网关里的模型名，用到这一档时请求会失败".into(),
    })
}

pub fn inspect_claude(cfg: &crate::config::Config) -> ClaudeInspection {
    let base = base_url(cfg.server.port);
    let mut files = Vec::new();
    let mut issues = Vec::new();
    for p in claude_files() {
        let Ok(fs_meta) = std::fs::metadata(&p) else { continue };
        let file = p.display().to_string();
        let modified = fs_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let v = match read_json(&p) {
            Ok(v) => v,
            Err(e) => {
                issues.push(ConfigIssue { file: file.clone(), key: "（整个文件）".into(), value: String::new(), problem: e.to_string() });
                continue;
            }
        };
        let mut entries = Vec::new();
        let mut issue = |key: &str, value: &str, problem: String| {
            issues.push(ConfigIssue { file: file.clone(), key: key.into(), value: value.into(), problem });
        };
        if let Some(m) = v.get("model").and_then(Value::as_str) {
            entries.push(("model".to_string(), m.to_string()));
            if let Some(pb) = model_problem(cfg, m) {
                issue("model", m, pb);
            }
        }
        if v.get("apiKeyHelper").is_some() {
            entries.push(("apiKeyHelper".to_string(), "••••".to_string()));
            issue("apiKeyHelper", "••••", "会顶掉网关用的 token".into());
        }
        let Some(env) = v.get("env").and_then(Value::as_object) else {
            files.push(ConfigFileState { path: file, modified, entries });
            continue;
        };
        let mut keys: Vec<&String> = env.keys().filter(|k| k.starts_with("ANTHROPIC_") || k.starts_with("CLAUDE_CODE_")).collect();
        keys.sort();
        for k in keys {
            let val = env[k].as_str().map(str::to_string).unwrap_or_else(|| env[k].to_string());
            entries.push((k.clone(), if is_secret(k) { "••••".into() } else { val.clone() }));
            if k == "ANTHROPIC_BASE_URL" && val.trim_end_matches('/') != base {
                issue(k, &val, format!("没有指向 kiki-proxy（{base}）"));
            }
            if BYPASS_KEYS.contains(&k.as_str()) {
                issue(k, &val, "会让 Claude Code 绕过网关直连云服务".into());
            }
            if MODEL_KEYS.contains(&k.as_str()) {
                if let Some(pb) = model_problem(cfg, &val) {
                    issue(k, &val, pb);
                }
            }
            if k == "ANTHROPIC_API_KEY" && env.contains_key("ANTHROPIC_AUTH_TOKEN") {
                issue(k, "••••", "和 ANTHROPIC_AUTH_TOKEN 同时存在，Claude Code 会提示冲突".into());
            }
        }
        // 显示名和实际模型对不上（比如模型已经换成网关的，名字还是旧的）
        for tier in ["OPUS", "SONNET", "HAIKU", "FABLE"] {
            let (Some(m), Some(name)) = (
                env.get(&format!("ANTHROPIC_DEFAULT_{tier}_MODEL")).and_then(Value::as_str),
                env.get(&format!("ANTHROPIC_DEFAULT_{tier}_MODEL_NAME")).and_then(Value::as_str),
            ) else {
                continue;
            };
            let m_ok = model_problem(cfg, m).is_none();
            if m_ok && model_problem(cfg, name).is_some() && !name.contains(&crate::router::normalize_name(m)) {
                let want = meta(cfg, m).name;
                if name != want {
                    issue(&format!("ANTHROPIC_DEFAULT_{tier}_MODEL_NAME"), name, format!("显示名和实际模型（{m}）对不上，/model 里会显示错的名字"));
                }
            }
        }
        files.push(ConfigFileState { path: file, modified, entries });
    }
    ClaudeInspection { files, issues }
}
