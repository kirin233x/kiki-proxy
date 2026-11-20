//! 插件测试模板：用模拟的「登录服务 + 内部模型服务」把示例插件（plugins/example.rs）完整走一遍。
//!
//! 适配新插件时，复制这个文件为 `tests/plugin_<你的插件>.rs`，改三处：
//!   1. `mock_*` 函数：模拟你们内部服务真实的登录 / token / 接口行为（照抄真实报文最好）
//!   2. `setup()` 里的端点配置：插件 id、插件设置项、地址、模型
//!   3. 断言：上游收到的请求长什么样（鉴权头、特殊字段……）
//!
//! 运行：`cd src-tauri && cargo test -p kiki-core --test plugin_example`

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use kiki_core::config::*;
use kiki_core::ir::Protocol;
use kiki_core::plugin::*;
use kiki_core::Gateway;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

// ------------------------------------------------------------------ 模拟的内部服务

#[derive(Clone, Default)]
struct Mock {
    /// 内部模型接口收到的 (请求头, 请求体)
    calls: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
    /// token 接口被调用的 grant_type
    grants: Arc<Mutex<Vec<String>>>,
}

/// token 接口：code 换 token、refresh_token 续期
async fn mock_token(State(m): State<Mock>, body: Bytes) -> Response {
    let v: Value = serde_json::from_slice(&body).unwrap_or_default();
    let grant = v["grant_type"].as_str().unwrap_or("").to_string();
    m.grants.lock().unwrap().push(grant.clone());
    match grant.as_str() {
        // expires_in 故意给得很短（小于 60 秒就算已过期），这样第二次请求会触发续期
        "authorization_code" if v["code"] == "test-code" => {
            axum::Json(json!({"access_token":"tok-1","refresh_token":"r-1","expires_in":30})).into_response()
        }
        "refresh_token" if v["refresh_token"] == "r-1" => {
            axum::Json(json!({"access_token":"tok-2","refresh_token":"r-2","expires_in":3600})).into_response()
        }
        _ => (StatusCode::BAD_REQUEST, r#"{"error":{"message":"bad grant"}}"#).into_response(),
    }
}

/// 内部模型接口：标准 OpenAI Chat Completions
async fn mock_chat(State(m): State<Mock>, headers: HeaderMap, body: Bytes) -> Response {
    let v: Value = serde_json::from_slice(&body).unwrap_or_default();
    m.calls.lock().unwrap().push((headers.clone(), v.clone()));
    let auth = headers.get("authorization").and_then(|h| h.to_str().ok()).unwrap_or("");
    if auth != "Bearer tok-1" && auth != "Bearer tok-2" {
        return (StatusCode::UNAUTHORIZED, r#"{"error":{"message":"invalid token"}}"#).into_response();
    }
    if v["stream"] == json!(true) {
        let sse = [
            r#"{"id":"c1","model":"m1","choices":[{"index":0,"delta":{"role":"assistant","content":"Hel"}}]}"#,
            r#"{"id":"c1","model":"m1","choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":"stop"}],"usage":{"prompt_tokens":9,"completion_tokens":2}}"#,
            "[DONE]",
        ]
        .iter()
        .map(|d| format!("data: {d}\n\n"))
        .collect::<String>();
        return ([("content-type", "text/event-stream")], sse).into_response();
    }
    axum::Json(json!({
        "id": "c1", "object": "chat.completion", "model": "m1",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 9, "completion_tokens": 1}
    }))
    .into_response()
}

/// 代替「打开浏览器让用户登录」：直接访问回调地址，带上 code，就像用户登录成功后浏览器跳回来
struct AutoLoginHost;

#[async_trait]
impl Host for AutoLoginHost {
    async fn open_url(&self, url: &str) -> anyhow::Result<()> {
        let u = url::Url::parse(url)?;
        let redirect = u.query_pairs().find(|(k, _)| k == "redirect_uri").map(|(_, v)| v.to_string()).expect("登录地址里应该带 redirect_uri");
        tokio::spawn(async move {
            let _ = reqwest::get(format!("{redirect}?code=test-code")).await;
        });
        Ok(())
    }
    async fn prompt(&self, _form: PromptForm) -> anyhow::Result<Option<BTreeMap<String, String>>> {
        anyhow::bail!("这个插件不应该弹表单")
    }
    fn notify(&self, _title: &str, _body: &str) {}
}

// ------------------------------------------------------------------ 准备网关

async fn serve(router: Router) -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    port
}

async fn setup() -> (Gateway, u16, Mock) {
    let mock = Mock::default();
    let up = serve(
        Router::new()
            .route("/token", post(mock_token))
            .route("/v1/chat/completions", post(mock_chat))
            .with_state(mock.clone()),
    )
    .await;

    let mut cfg = Config::default();
    cfg.endpoints.push(EndpointConfig {
        id: "corp".into(),
        name: "内部服务".into(),
        plugin: "corp_example".into(), // ← 插件 id
        base_url: format!("http://127.0.0.1:{up}/v1"),
        proxy: PROXY_DIRECT.into(),
        plugin_settings: [
            ("login_url".to_string(), format!("http://127.0.0.1:{up}/login")),
            ("token_url".to_string(), format!("http://127.0.0.1:{up}/token")),
        ]
        .into(),
        models: vec![ModelConfig { name: "m1".into(), ..Default::default() }],
        ..Default::default()
    });
    let gw = Gateway::in_memory(cfg, Arc::new(MemorySecrets::default()), Arc::new(AutoLoginHost), vec![]).unwrap();
    let port = serve(gw.router()).await;
    (gw, port, mock)
}

/// 模拟 Claude Code 发一个请求
async fn claude_code(port: u16, stream: bool) -> (u16, String) {
    let r = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/messages"))
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "corp/m1", "max_tokens": 100, "stream": stream,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

// ------------------------------------------------------------------ 测试

#[tokio::test]
async fn plugin_endpoint_uses_declared_protocol() {
    let (gw, _, _) = setup().await;
    // 配置里没写 protocol，以插件声明的为准
    assert_eq!(gw.config().endpoint("corp").unwrap().protocol, Protocol::OpenaiChat);
}

#[tokio::test]
async fn login_then_request_then_refresh() {
    let (gw, port, mock) = setup().await;

    // 1. 第一次请求：没有凭证 → 网页登录拿到 tok-1 → 以 OpenAI 格式调用内部服务
    let (status, body) = claude_code(port, false).await;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["content"][0]["text"], json!("Hello"), "Claude Code 应该收到 Anthropic 格式的回复");

    let (headers, sent) = mock.calls.lock().unwrap().last().cloned().unwrap();
    assert_eq!(headers.get("authorization").unwrap(), "Bearer tok-1");
    assert_eq!(headers.get("x-client").unwrap(), "kiki-proxy", "on_request 加的请求头");
    assert_eq!(sent["model"], json!("m1"));
    assert_eq!(sent["messages"][0]["content"], json!("hi"), "请求已转换成 OpenAI 格式");

    // 2. tok-1 已经过期 → 用 refresh_token 续期拿到 tok-2；流式请求也正常
    let (status, body) = claude_code(port, true).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("Hel") && body.contains("message_stop"), "{body}");
    let (headers, sent) = mock.calls.lock().unwrap().last().cloned().unwrap();
    assert_eq!(headers.get("authorization").unwrap(), "Bearer tok-2");
    assert!(sent.get("stream_options").is_none(), "on_request 去掉了 stream_options");
    assert_eq!(*mock.grants.lock().unwrap(), vec!["authorization_code", "refresh_token"]);

    // 3. 端点状态：已登录
    assert!(gw.credential_status("corp").logged_in);
}

#[tokio::test]
async fn stepwise_test_shows_login() {
    let (gw, _, _) = setup().await;
    assert_eq!(gw.test_step_network("corp").await.status, "ok");
    let auth = gw.test_step_auth("corp").await;
    assert_eq!((auth.label.as_str(), auth.status.as_str()), ("登录", "ok"), "{}", auth.detail);
    let m = gw.test_step_model("corp", "m1").await;
    assert_eq!(m.status, "ok", "{}", m.detail);
}
