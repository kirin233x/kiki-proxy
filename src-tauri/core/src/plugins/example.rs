//! 示例插件：演示浏览器 SSO 登录、账号密码 + 验证码登录、token 刷新、改写请求和响应。
//!
//! 假想的内部服务是这样的：
//! - 浏览器 SSO：打开 `{sso_url}?redirect_uri=...`，登录成功后跳回本机，URL 上带 `code`；
//!   再用 code 调 `{token_url}` 换 token。
//! - 账号密码：`POST {token_url}`，`{"username","password","otp"}` → `{"access_token","refresh_token","expires_in"}`
//! - 调接口时要带 `X-Corp-Token: <token>`，并且请求体里要多一个 `"biz": "..."` 字段
//! - token 失效时返回 HTTP 200 但 body 是 `{"code": 40101}`（非标准的鉴权失败）
//!
//! 照着改成你们内部服务的真实逻辑即可。

use crate::plugin::*;
use crate::sse::SseEvent;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct ExampleSso;

impl ExampleSso {
    /// 调 token 接口，把返回解析成 Credential
    async fn exchange(&self, ctx: &PluginCtx, body: Value) -> Result<Credential> {
        let url = ctx.require("token_url")?;
        let resp = ctx.http.post(url).json(&body).send().await?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or_default();
        if !status.is_success() {
            bail!("登录失败（{status}）：{}", crate::protocol::error_message(v.to_string().as_bytes()));
        }
        let token = v.get("access_token").and_then(Value::as_str).ok_or_else(|| anyhow!("登录接口没有返回 access_token"))?;
        let mut cred = Credential::bearer(token);
        cred.refresh_token = v.get("refresh_token").and_then(Value::as_str).map(str::to_string);
        if let Some(secs) = v.get("expires_in").and_then(Value::as_i64) {
            cred = cred.expires_in(secs);
        }
        Ok(cred)
    }

    async fn login_with_browser(&self, ctx: &PluginCtx, sso: &str) -> Result<Credential> {
        // 有的 SSO 只允许固定的回调端口，可以在设置里填；0 表示随机端口
        let port = ctx.setting("callback_port").and_then(|p| p.parse().ok()).unwrap_or(0);
        let cb = CallbackServer::bind(port).await?;
        let redirect = cb.redirect_uri("/callback");
        let mut url = url::Url::parse(sso)?;
        url.query_pairs_mut().append_pair("redirect_uri", &redirect);
        ctx.notify("kiki-proxy", &format!("请在浏览器里登录「{}」", ctx.endpoint_name));
        ctx.open_url(url.as_str()).await?;
        let hit = cb.wait(Duration::from_secs(300)).await?;
        let code = hit.query.get("code").ok_or_else(|| anyhow!("回调里没有 code"))?;
        self.exchange(ctx, json!({ "grant_type": "authorization_code", "code": code, "redirect_uri": redirect })).await
    }

    async fn login_with_password(&self, ctx: &PluginCtx) -> Result<Credential> {
        let last_user = ctx.load("username").unwrap_or_default();
        let form = PromptForm::new(&format!("登录「{}」", ctx.endpoint_name))
            .message("请输入内部账号和动态验证码，登录后 kiki-proxy 会自动续期。")
            .field(PromptField::text("username", "账号").default_value(&last_user))
            .field(PromptField::password("password", "密码"))
            .field(PromptField::code("otp", "验证码").placeholder("6 位数字"))
            .submit("登录");
        let v = ctx.prompt(form).await?;
        let username = v.get("username").cloned().unwrap_or_default();
        ctx.save("username", &username)?;
        self.exchange(
            ctx,
            json!({
                "grant_type": "password",
                "username": username,
                "password": v.get("password").cloned().unwrap_or_default(),
                "otp": v.get("otp").cloned().unwrap_or_default(),
            }),
        )
        .await
    }
}

#[async_trait]
impl Plugin for ExampleSso {
    fn id(&self) -> &'static str {
        "example_sso"
    }

    fn name(&self) -> &'static str {
        "示例：内部 SSO"
    }

    fn description(&self) -> &'static str {
        "演示浏览器 SSO / 账号密码登录、token 刷新、改写请求与响应。复制 plugins/example.rs 改成你自己的。"
    }

    fn settings(&self) -> Vec<SettingField> {
        vec![
            SettingField::url("sso_url", "SSO 登录地址").help("填了就用浏览器登录；留空则弹出账号密码表单"),
            SettingField::url("token_url", "Token 接口").required(),
            SettingField::text("callback_port", "回调端口").help("SSO 要求固定回调端口时填写，否则留空").default_value(""),
            SettingField::text("biz", "业务线").help("会写进请求体的 biz 字段").default_value("ai-coding"),
            SettingField::secret("app_secret", "App Secret").help("存在钥匙串里，会作为 X-App-Secret 请求头发送"),
        ]
    }

    fn uses_login(&self) -> bool {
        true
    }

    async fn login(&self, ctx: &PluginCtx) -> Result<Credential> {
        match ctx.setting("sso_url") {
            Some(sso) => self.login_with_browser(ctx, sso).await,
            None => self.login_with_password(ctx).await,
        }
    }

    async fn refresh(&self, ctx: &PluginCtx, old: &Credential) -> Result<Credential> {
        let rt = old.refresh_token.clone().ok_or_else(|| anyhow!("没有 refresh_token"))?;
        self.exchange(ctx, json!({ "grant_type": "refresh_token", "refresh_token": rt })).await
    }

    fn apply_credential(&self, cred: &Credential, req: &mut UpstreamRequest) -> Result<()> {
        req.set_header("x-corp-token", &cred.token)
    }

    fn is_auth_failure(&self, status: u16, body: &[u8]) -> bool {
        if status == 401 {
            return true;
        }
        // 这个服务 token 失效时返回 200 + 业务错误码
        serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("code").and_then(Value::as_i64)) == Some(40101)
    }

    async fn on_request(&self, ctx: &PluginCtx, req: &mut UpstreamRequest) -> Result<()> {
        if let Some(biz) = ctx.setting("biz") {
            req.body["biz"] = json!(biz);
        }
        if let Some(secret) = ctx.setting("app_secret") {
            req.set_header("x-app-secret", secret)?;
        }
        Ok(())
    }

    async fn on_response(&self, _ctx: &PluginCtx, resp: &mut UpstreamResponse) -> Result<()> {
        // 例子：上游把真正的响应包在 {"data": {...}} 里，拆出来
        if let Ok(v) = serde_json::from_slice::<Value>(&resp.body) {
            if let Some(inner) = v.get("data").filter(|d| d.is_object()) {
                resp.body = serde_json::to_vec(inner)?;
            }
        }
        Ok(())
    }

    fn on_stream_event(&self, _ctx: &PluginCtx, ev: SseEvent) -> Vec<SseEvent> {
        // 例子：上游的流里夹杂了自定义的心跳事件，丢掉
        if ev.event.as_deref() == Some("heartbeat") {
            return vec![];
        }
        vec![ev]
    }
}
