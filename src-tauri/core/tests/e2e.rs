//! 端到端测试：起一个模拟上游（4 种协议），通过真实的 HTTP 请求走一遍网关。
//! 覆盖 4×4 种「客户端协议 × 上游协议」组合的流式和非流式、故障切换、插件重新登录。

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use kiki_core::config::*;
use kiki_core::ir::*;
use kiki_core::plugin::*;
use kiki_core::protocol::{self, Compat, EncodeCtx, Normalizer};
use kiki_core::sse::SseParser;
use kiki_core::Gateway;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

// ------------------------------------------------------------------ 模拟上游

#[derive(Clone, Default)]
struct Upstream {
    /// 每个前缀最近一次收到的请求体
    seen: Arc<Mutex<Vec<(String, String, HeaderMap)>>>,
}

fn canned_response() -> Response_ {
    Response_ {
        id: "up_1".into(),
        model: "upstream-model".into(),
        content: vec![
            Part::Thinking { text: "let me think".into(), signature: Some("sig123".into()) },
            Part::Text("Hello".into()),
            Part::ToolUse { id: "call_9".into(), name: "get_weather".into(), input: json!({"city":"SF"}) },
        ],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input: 10, output: 5, cache_read: 2, cache_write: 0 },
    }
}

type Response_ = kiki_core::ir::Response;

fn upstream_reply(p: Protocol, stream: bool) -> Response {
    let resp = canned_response();
    let ctx = EncodeCtx { model: "upstream-model".into(), ..Default::default() };
    if !stream {
        let v = protocol::build_response(p, &resp, &ctx);
        return (StatusCode::OK, [("content-type", "application/json")], v.to_string()).into_response();
    }
    let mut enc = protocol::encoder(p, ctx);
    let mut norm = Normalizer::new("upstream-model");
    let mut out = String::new();
    for ev in protocol::response_to_events(&resp) {
        // 文本拆成两段，模拟真实的增量
        let evs = match ev {
            Event::TextDelta { index, text } => vec![
                Event::TextDelta { index, text: text[..2].to_string() },
                Event::TextDelta { index, text: text[2..].to_string() },
            ],
            e => vec![e],
        };
        for e in evs {
            for ne in norm.push(e) {
                enc.encode(&ne).iter().for_each(|s| out.push_str(&s.to_bytes()));
            }
        }
    }
    (StatusCode::OK, [("content-type", "text/event-stream")], out).into_response()
}

async fn up_handler(State(u): State<Upstream>, uri: axum::http::Uri, headers: HeaderMap, body: Bytes) -> Response {
    let path = uri.path().to_string();
    let text = String::from_utf8_lossy(&body).to_string();
    u.seen.lock().unwrap().push((path.clone(), text.clone(), headers.clone()));
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    let stream = v.get("stream").and_then(Value::as_bool).unwrap_or(false) || uri.query().is_some_and(|q| q.contains("alt=sse"));
    let prefix = path.split('/').nth(1).unwrap_or("");
    match prefix {
        "fail" => (StatusCode::SERVICE_UNAVAILABLE, r#"{"error":{"message":"down"}}"#).into_response(),
        "long" => (
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"This model's maximum context length is 131072 tokens. However, you requested 200000 tokens.","type":"invalid_request_error","code":"context_length_exceeded"}}"#,
        )
            .into_response(),
        "an" if path.ends_with("/models") => {
            (StatusCode::OK, r#"{"data":[{"id":"claude-b"},{"id":"claude-a"}],"has_more":false}"#).into_response()
        }
        "auth" => {
            if headers.get("x-test-token").and_then(|v| v.to_str().ok()) == Some("token-2") {
                upstream_reply(Protocol::OpenaiChat, stream)
            } else {
                (StatusCode::UNAUTHORIZED, r#"{"error":{"message":"expired"}}"#).into_response()
            }
        }
        "an" => upstream_reply(Protocol::Anthropic, stream),
        "oa" => upstream_reply(Protocol::OpenaiChat, stream),
        "rs" => upstream_reply(Protocol::OpenaiResponses, stream),
        "gm" => upstream_reply(Protocol::Gemini, stream),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve(router: Router) -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    port
}

// ------------------------------------------------------------------ 测试插件：第一次登录给过期 token

struct TestAuth {
    logins: Arc<Mutex<u32>>,
}

#[async_trait]
impl Plugin for TestAuth {
    fn id(&self) -> &'static str {
        "test_auth"
    }
    fn name(&self) -> &'static str {
        "测试鉴权"
    }
    fn uses_login(&self) -> bool {
        true
    }
    async fn login(&self, _ctx: &PluginCtx) -> anyhow::Result<Credential> {
        let mut n = self.logins.lock().unwrap();
        *n += 1;
        Ok(Credential::bearer(format!("token-{n}")))
    }
    fn apply_credential(&self, cred: &Credential, req: &mut UpstreamRequest) -> anyhow::Result<()> {
        req.set_header("x-test-token", &cred.token)
    }
}

// ------------------------------------------------------------------ 准备网关

fn endpoint(id: &str, p: Protocol, base: String, model: &str) -> EndpointConfig {
    EndpointConfig {
        id: id.into(),
        name: id.to_uppercase(),
        protocol: p,
        base_url: base,
        proxy: PROXY_DIRECT.into(),
        compat: Compat { send_reasoning_content: true, ..Default::default() },
        models: vec![ModelConfig {
            name: model.into(),
            upstream: format!("{model}-real"),
            price: Price { input: 1.0, output: 2.0, ..Default::default() },
            ..Default::default()
        }],
        ..Default::default()
    }
}

struct Env {
    gw_port: u16,
    upstream: Upstream,
    gateway: Gateway,
    logins: Arc<Mutex<u32>>,
}

async fn setup() -> Env {
    let upstream = Upstream::default();
    let up_port = serve(Router::new().fallback(up_handler).with_state(upstream.clone())).await;
    let base = |p: &str| format!("http://127.0.0.1:{up_port}/{p}");
    let mut cfg = Config::default();
    cfg.endpoints = vec![
        {
            let mut e = endpoint("an", Protocol::Anthropic, base("an"), "claude");
            e.models[0].context_window = 1_000_000;
            e
        },
        endpoint("long", Protocol::OpenaiChat, base("long"), "small"),
        endpoint("oa", Protocol::OpenaiChat, base("oa"), "gpt"),
        endpoint("rs", Protocol::OpenaiResponses, base("rs"), "gpt5"),
        endpoint("gm", Protocol::Gemini, base("gm"), "gemini"),
        EndpointConfig { fallbacks: vec!["oa/gpt".into()], ..endpoint("bad", Protocol::OpenaiChat, base("fail"), "x") },
        EndpointConfig { plugin: "test_auth".into(), ..endpoint("corp", Protocol::OpenaiChat, base("auth"), "m") },
    ];
    let logins = Arc::new(Mutex::new(0));
    let gateway = Gateway::in_memory(
        cfg,
        Arc::new(MemorySecrets::default()),
        Arc::new(HeadlessHost),
        vec![Arc::new(TestAuth { logins: logins.clone() })],
    )
    .unwrap();
    let gw_port = serve(gateway.router()).await;
    Env { gw_port, upstream, gateway, logins }
}

fn client_request(model: &str, stream: bool) -> Request {
    Request {
        model: model.into(),
        system: Some("be nice".into()),
        messages: vec![
            Message {
                role: Role::User,
                content: vec![
                    Part::Text("What's the weather?".into()),
                    Part::Image(ImageSource::Base64 { media_type: "image/png".into(), data: "aGVsbG8=".into() }),
                ],
            },
            Message {
                role: Role::Assistant,
                content: vec![Part::ToolUse { id: "call_1".into(), name: "get_weather".into(), input: json!({"city":"Paris"}) }],
            },
            Message {
                role: Role::User,
                content: vec![
                    Part::ToolResult { tool_use_id: "call_1".into(), content: vec![Part::Text("sunny".into())], is_error: false },
                    Part::Text("and SF?".into()),
                ],
            },
        ],
        tools: vec![Tool {
            name: "get_weather".into(),
            description: Some("天气".into()),
            schema: json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}),
            kind: ToolKind::Function,
        }],
        max_tokens: Some(256),
        stream,
        ..Default::default()
    }
}

async fn call(env: &Env, client: Protocol, model: &str, stream: bool) -> (u16, String) {
    let req = client_request(model, stream);
    let body = protocol::build_request(client, &req, model, &Compat::default());
    let base = format!("http://127.0.0.1:{}", env.gw_port);
    let url = match client {
        Protocol::Anthropic => format!("{base}/v1/messages"),
        Protocol::OpenaiChat => format!("{base}/v1/chat/completions"),
        Protocol::OpenaiResponses => format!("{base}/v1/responses"),
        Protocol::Gemini => {
            let action = if stream { "streamGenerateContent?alt=sse" } else { "generateContent" };
            format!("{base}/v1beta/models/{model}:{action}")
        }
    };
    let r = reqwest::Client::new().post(url).json(&body).send().await.unwrap();
    let status = r.status().as_u16();
    (status, r.text().await.unwrap())
}

/// 用客户端协议自己的解码器把响应读回 IR
fn decode(client: Protocol, stream: bool, body: &str) -> Response_ {
    if !stream {
        let v: Value = serde_json::from_str(body).unwrap_or_else(|_| panic!("不是 JSON：{body}"));
        return protocol::parse_response(client, &v).unwrap();
    }
    let mut p = SseParser::new();
    let mut dec = protocol::decoder(client);
    let mut evs = Vec::new();
    for e in p.feed(body.as_bytes()).into_iter().chain(p.finish()) {
        evs.extend(dec.feed(&e));
    }
    evs.extend(dec.finish());
    protocol::aggregate(&evs)
}

const ALL: [Protocol; 4] = [Protocol::Anthropic, Protocol::OpenaiChat, Protocol::OpenaiResponses, Protocol::Gemini];

fn model_for(p: Protocol) -> &'static str {
    match p {
        Protocol::Anthropic => "an/claude",
        Protocol::OpenaiChat => "oa/gpt",
        Protocol::OpenaiResponses => "rs/gpt5",
        Protocol::Gemini => "gm/gemini",
    }
}

#[tokio::test]
async fn all_protocol_pairs() {
    let env = setup().await;
    for stream in [false, true] {
        for client in ALL {
            for up in ALL {
                let model = model_for(up);
                let (status, body) = call(&env, client, model, stream).await;
                let tag = format!("{client:?} → {up:?} stream={stream}");
                assert_eq!(status, 200, "{tag}: {body}");
                let r = decode(client, stream, &body);
                let text = parts_text(&r.content);
                assert_eq!(text, "Hello", "{tag}: {body}");
                let tool = r.content.iter().find_map(|p| match p {
                    Part::ToolUse { name, input, .. } => Some((name.clone(), input.clone())),
                    _ => None,
                });
                assert_eq!(tool, Some(("get_weather".into(), json!({"city":"SF"}))), "{tag}: {body}");
                assert_eq!(r.stop_reason, StopReason::ToolUse, "{tag}");
                assert_eq!(r.usage.output, 5, "{tag}: usage {:?}", r.usage);
                assert_eq!(r.usage.total_input(), 12, "{tag}: usage {:?}", r.usage);

                // 上游收到的请求：模型名换成了真实名字，图片、工具、工具结果都在
                let (path, sent, _) = env.upstream.seen.lock().unwrap().last().cloned().unwrap();
                let real = format!("{}-real", model.split('/').nth(1).unwrap());
                assert!(sent.contains(&real) || path.contains(&real), "{tag}: 模型名没替换 {path} {sent}");
                assert!(sent.contains("aGVsbG8="), "{tag}: 图片丢了 {sent}");
                assert!(sent.contains("get_weather"), "{tag}: 工具丢了");
                assert!(sent.contains("sunny"), "{tag}: 工具结果丢了");
                assert!(sent.contains("and SF?"), "{tag}: 用户消息丢了");
            }
        }
    }
}

#[tokio::test]
async fn failover_to_backup_model() {
    let env = setup().await;
    let (status, body) = call(&env, Protocol::OpenaiChat, "bad/x", false).await;
    assert_eq!(status, 200, "{body}");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let logs = env.gateway.store().list(&Default::default()).unwrap();
    let rec = &logs[0];
    assert_eq!(rec.gateway_model, "oa/gpt");
    assert_eq!(rec.attempts.len(), 1);
    assert_eq!(rec.attempts[0].status, 503);
    assert!(rec.cost > 0.0);
}

#[tokio::test]
async fn plugin_relogin_on_401() {
    let env = setup().await;
    let (status, body) = call(&env, Protocol::Anthropic, "corp/m", true).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(*env.logins.lock().unwrap(), 2, "应该先用 token-1 失败，再登录拿 token-2");
    // 再请求一次不需要重新登录
    let (status, _) = call(&env, Protocol::Anthropic, "corp/m", false).await;
    assert_eq!(status, 200);
    assert_eq!(*env.logins.lock().unwrap(), 2);
}

#[tokio::test]
async fn unknown_model_and_listing() {
    let env = setup().await;
    let (status, body) = call(&env, Protocol::Anthropic, "nope", false).await;
    assert_eq!(status, 404);
    assert!(body.contains("找不到模型"));
    let r = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}/v1/models", env.gw_port))
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let ids: Vec<&str> = v["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"an/claude[1m]"));
    assert!(ids.contains(&"claude@oa/gpt"), "{ids:?}");
    // 带 claude@ 前缀的名字能正常路由
    let (status, _) = call(&env, Protocol::Anthropic, "claude@oa/gpt", false).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn context_1m_and_overflow() {
    let env = setup().await;
    // 1M 模型在给 Claude Code 的列表里带 [1m]
    let v: Value = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}/v1/models", env.gw_port))
        .header("x-api-key", "k")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = v["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"an/claude[1m]"), "{ids:?}");
    // 带 [1m] 的名字能路由，并且自动加了 1M beta 头
    let (status, body) = call(&env, Protocol::Anthropic, "an/claude[1m]", false).await;
    assert_eq!(status, 200, "{body}");
    let (_, _, headers) = env.upstream.seen.lock().unwrap().last().cloned().unwrap();
    assert!(headers.get("anthropic-beta").unwrap().to_str().unwrap().contains("context-1m-2025-08-07"));
    // 上游的上下文超长错误改写成 Claude Code 认识的格式
    let (status, body) = call(&env, Protocol::Anthropic, "long/small", false).await;
    assert_eq!(status, 400);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["type"], json!("invalid_request_error"));
    assert!(v["error"]["message"].as_str().unwrap().starts_with("prompt is too long"), "{body}");
    // 从上游拉模型列表
    let ids = env.gateway.list_upstream_models("an").await.unwrap();
    assert_eq!(ids, vec!["claude-a", "claude-b"]);
}

#[tokio::test]
async fn stepwise_test_report() {
    let env = setup().await;
    let r = env.gateway.test_endpoint_report("oa").await;
    let kinds: Vec<(&str, &str)> = r.steps.iter().map(|s| (s.kind.as_str(), s.status.as_str())).collect();
    assert_eq!(kinds, vec![("network", "ok"), ("auth", "skip"), ("model", "ok")], "{:?}", r.steps);
    assert!(r.ok);
    assert!(r.steps[2].detail.contains("Hello"));

    // 插件端点：登录这一步会真的去登录
    let r = env.gateway.test_endpoint_report("corp").await;
    assert_eq!(r.steps[1].label, "登录");
    assert_eq!(r.steps[1].status, "ok", "{:?}", r.steps);

    // 上游不可达：停在网络这一步
    let mut cfg = (*env.gateway.config()).clone();
    cfg.endpoints.push(EndpointConfig { ..endpoint("dead", Protocol::OpenaiChat, "http://127.0.0.1:1".into(), "x") });
    env.gateway.save_config(cfg).await.unwrap();
    let r = env.gateway.test_endpoint_report("dead").await;
    assert_eq!(r.steps.len(), 1);
    assert_eq!(r.steps[0].status, "fail");
    assert!(!r.ok);
}
