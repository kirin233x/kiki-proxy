//! 示例插件：公司内部的 OpenAI 格式模型服务。复制这个文件改成你们的真实逻辑。
//!
//! 假想的内部服务：
//! - 接口是标准的 OpenAI Chat Completions（`/v1/chat/completions`）
//!   → 只要 `protocol()` 返回 `OpenaiChat`，网关会把 Claude Code / Codex / Gemini CLI 的请求都转换好，
//!   插件里不用写任何格式转换。
//! - 登录：打开网页登录，登录成功后浏览器跳回本机，URL 上带 `code`；用 code 换 token。
//!   token 有有效期，到期前用 refresh_token 续期。
//! - 调接口时 token 放在 `Authorization: Bearer <token>`（默认行为，所以不用重写 apply_credential）。
//!
//! 如果内部格式和标准 OpenAI 有小差异（多一个字段、换个请求头、URL 不同），在 `on_request` 里改；
//! 响应有差异就在 `on_response` / `on_stream_event` 里改。完全一样的话这几个方法都可以删掉。

use crate::ir::Protocol;
use crate::plugin::*;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct CorpExample;

impl CorpExample {
    /// 调 token 接口，把返回解析成 Credential
    async fn token_request(&self, ctx: &PluginCtx, body: Value) -> Result<Credential> {
        let url = ctx.require("token_url")?;
        let resp = ctx.http.post(url).json(&body).send().await?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or_default();
        if !status.is_success() {
            bail!("获取 token 失败（{status}）：{}", crate::protocol::error_message(v.to_string().as_bytes()));
        }
        let token = v.get("access_token").and_then(Value::as_str).ok_or_else(|| anyhow!("token 接口没有返回 access_token"))?;
        let mut cred = Credential::bearer(token);
        cred.refresh_token = v.get("refresh_token").and_then(Value::as_str).map(str::to_string);
        if let Some(secs) = v.get("expires_in").and_then(Value::as_i64) {
            cred = cred.expires_in(secs);
        }
        Ok(cred)
    }
}

#[async_trait]
impl Plugin for CorpExample {
    fn id(&self) -> &'static str {
        "corp_example"
    }

    fn name(&self) -> &'static str {
        "公司内部服务（示例）"
    }

    fn description(&self) -> &'static str {
        "OpenAI Chat 格式 + 网页登录 + 自动续期。复制 src-tauri/core/src/plugins/example.rs 改成你们的真实逻辑。"
    }

    /// 内部服务说的是 OpenAI Chat 格式
    fn protocol(&self) -> Protocol {
        Protocol::OpenaiChat
    }

    fn default_base_url(&self) -> &'static str {
        "https://llm.corp.example/v1"
    }

    fn settings(&self) -> Vec<SettingField> {
        vec![
            SettingField::url("login_url", "登录页地址").required().help("打开这个网页登录，登录后会跳回 kiki-proxy"),
            SettingField::url("token_url", "Token 接口").required().help("用登录拿到的 code 换 token，也用来续期"),
            SettingField::text("callback_port", "回调端口").help("登录页要求固定回调端口时填写，否则留空"),
        ]
    }

    fn uses_login(&self) -> bool {
        true
    }

    /// 打开网页登录（App 启动时、token 失效时会自动调用）
    async fn login(&self, ctx: &PluginCtx) -> Result<Credential> {
        let port = ctx.setting("callback_port").and_then(|p| p.parse().ok()).unwrap_or(0);
        let cb = CallbackServer::bind(port).await?;
        let redirect = cb.redirect_uri("/callback");
        let mut url = url::Url::parse(ctx.require("login_url")?)?;
        url.query_pairs_mut().append_pair("redirect_uri", &redirect);
        ctx.notify("kiki-proxy", &format!("请在浏览器里登录「{}」", ctx.endpoint_name));
        ctx.open_url(url.as_str()).await?;
        let hit = cb.wait(Duration::from_secs(300)).await?;
        let code = hit.query.get("code").ok_or_else(|| anyhow!("登录回调里没有 code"))?;
        self.token_request(ctx, json!({ "grant_type": "authorization_code", "code": code, "redirect_uri": redirect }))
            .await
    }

    /// 快到期时在后台续期；没有 refresh_token 就重新打开网页登录
    async fn refresh(&self, ctx: &PluginCtx, old: &Credential) -> Result<Credential> {
        match &old.refresh_token {
            Some(rt) => self.token_request(ctx, json!({ "grant_type": "refresh_token", "refresh_token": rt })).await,
            None => self.login(ctx).await,
        }
    }

    /// 和标准 OpenAI 不一样的地方写在这里，例如：
    async fn on_request(&self, _ctx: &PluginCtx, req: &mut UpstreamRequest) -> Result<()> {
        // 内部服务要求带一个来源标记
        req.set_header("x-client", "kiki-proxy")?;
        // 内部服务不认识 stream_options 字段
        if let Some(o) = req.body.as_object_mut() {
            o.remove("stream_options");
        }
        Ok(())
    }
}
