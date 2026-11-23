//! 网关：本地 HTTP 服务 + 请求处理主流程。
//!
//! 客户端请求 → 鉴权 → 路由（规则 / 前缀 / 默认模型）→ 故障切换链逐个尝试：
//!   协议转换 → 出口代理发送 →
//!   失败且允许切换则换下一个 → 成功则（流式）转换响应回客户端协议 → 记日志

use crate::config::{Config, ConfigFile};
use crate::ir::{estimate_tokens, new_id, now_millis, Message, Part, Protocol, Request, Role};
use crate::process::{ProcessStatus, Processes};
use crate::secrets::{keys, SecretStore};
use crate::protocol::{self, EncodeCtx, Normalizer};
use crate::router::{self, Target, CLAUDE_ALIAS};
use crate::sse::{SseEvent, SseParser};
use crate::store::{Attempt, LogBodies, LogRecord, Store};
use crate::upstream::{base_headers, mask_headers, upstream_url, Clients};
use anyhow::{anyhow, Result};
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxPath, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures::StreamExt;
use parking_lot::RwLock;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
    pub started_at: i64,
}

/// 分步测试的一步
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestStep {
    /// network / auth / model
    pub kind: String,
    pub label: String,
    /// ok / fail / skip
    pub status: String,
    pub detail: String,
    pub ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub ok: bool,
    pub steps: Vec<TestStep>,
}

impl TestStep {
    fn new(kind: &str, label: &str, status: &str, detail: impl Into<String>, ms: i64) -> Self {
        TestStep { kind: kind.into(), label: label.into(), status: status.into(), detail: detail.into(), ms }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    pub status: u16,
    pub latency_ms: i64,
    pub message: String,
}

struct RunningServer {
    shutdown: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

pub struct Inner {
    dir: PathBuf,
    config_file: ConfigFile,
    config: RwLock<Arc<Config>>,
    pub secrets: Arc<dyn SecretStore>,
    processes: Arc<Processes>,
    clients: Clients,
    store: Arc<Store>,
    prices: crate::pricing::PriceCache,
    live: broadcast::Sender<LogRecord>,
    server: tokio::sync::Mutex<Option<RunningServer>>,
    status: RwLock<ServerStatus>,
    janitor_started: std::sync::atomic::AtomicBool,
}

/// 网关句柄，可以随便 clone
#[derive(Clone)]
pub struct Gateway(Arc<Inner>);

/// 一次尝试失败的原因
struct UpstreamResponse {
    status: u16,
    body: Vec<u8>,
}

struct Failure {
    status: u16,
    message: String,
    /// 是否允许切到下一个备选
    failover: bool,
    /// 上游的原始错误响应（同协议时原样转给客户端）
    raw: Option<(u16, Vec<u8>)>,
}

/// 发往上游的请求
struct UpstreamRequest {
    method: reqwest::Method,
    url: String,
    headers: HeaderMap,
    body: Value,
}

impl UpstreamRequest {
    fn body_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.body).unwrap_or_default()
    }
}

struct Prepared {
    req: UpstreamRequest,
    http: reqwest::Client,
}

impl Gateway {
    pub fn new(dir: &Path, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        let config_file = ConfigFile::new(dir);
        let config = config_file.load().unwrap_or_else(|e| {
            tracing::error!("读取配置失败，使用默认配置：{e}");
            Config::default()
        });
        let store = Arc::new(Store::open(&dir.join("logs.db"))?);
        Ok(Self::build(dir, config_file, config, store, secrets))
    }

    /// 测试用：内存数据库，不读写配置文件
    pub fn in_memory(config: Config, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("kiki-test-{}", new_id("")));
        Ok(Self::build(&dir, ConfigFile::new(&dir), config, Arc::new(Store::memory()?), secrets))
    }

    fn build(dir: &Path, config_file: ConfigFile, config: Config, store: Arc<Store>, secrets: Arc<dyn SecretStore>) -> Self {
        let (live, _) = broadcast::channel(256);
        Gateway(Arc::new(Inner {
            dir: dir.to_path_buf(),
            config_file,
            config: RwLock::new(Arc::new(config)),
            secrets,
            processes: Processes::new(),
            clients: Clients::default(),
            prices: crate::pricing::PriceCache::new(dir),
            store,
            live,
            server: tokio::sync::Mutex::new(None),
            status: RwLock::new(ServerStatus::default()),
            janitor_started: Default::default(),
        }))
    }

    pub fn data_dir(&self) -> &Path {
        &self.0.dir
    }

    pub fn config_path(&self) -> &Path {
        self.0.config_file.path()
    }

    pub fn config(&self) -> Arc<Config> {
        self.0.config.read().clone()
    }

    pub fn store(&self) -> &Store {
        &self.0.store
    }

    pub fn secrets(&self) -> &dyn SecretStore {
        &*self.0.secrets
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogRecord> {
        self.0.live.subscribe()
    }

    pub fn status(&self) -> ServerStatus {
        self.0.status.read().clone()
    }

    /// 保存配置；端口变了会自动重启服务
    pub async fn save_config(&self, cfg: Config) -> Result<()> {
        cfg.validate()?;
        let old_port = self.config().server.port;
        self.0.config_file.save(&cfg)?;
        *self.0.config.write() = Arc::new(cfg.clone());
        self.0.clients.clear();
        self.0.processes.sync(&cfg);
        if old_port != cfg.server.port && self.status().running {
            // 只重启网关服务本身，不连带重启本地程序
            self.stop_server().await;
            self.start_server().await?;
        }
        Ok(())
    }

    /// 密钥改了（代理密码等）以后调用，让 HTTP 客户端重建
    pub fn secrets_changed(&self) {
        self.0.clients.clear();
    }

    // ------------------------------------------------------------------ 服务启停

    /// 启动网关，并恢复本地程序（手动停止的除外）
    pub async fn start(&self) -> Result<()> {
        self.start_janitor();
        self.0.processes.resume();
        self.start_server().await
    }

    /// 暂停网关：本地程序也一起停
    pub async fn stop(&self) {
        self.0.processes.pause();
        self.stop_server().await;
    }

    async fn start_server(&self) -> Result<()> {
        self.start_janitor();
        let mut guard = self.0.server.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let port = self.config().server.port;
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(e) => {
                let msg = if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!("端口 {port} 已被其他程序占用，请在设置里换一个端口")
                } else {
                    format!("无法监听端口 {port}：{e}")
                };
                *self.0.status.write() = ServerStatus { running: false, port, error: Some(msg.clone()), started_at: 0 };
                return Err(anyhow!(msg));
            }
        };
        let app = self.router();
        let (tx, rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        *guard = Some(RunningServer { shutdown: tx, handle });
        *self.0.status.write() = ServerStatus { running: true, port, error: None, started_at: now_millis() };
        tracing::info!("kiki-proxy 网关已启动：http://127.0.0.1:{port}");
        Ok(())
    }

    async fn stop_server(&self) {
        let server = self.0.server.lock().await.take();
        if let Some(s) = server {
            let _ = s.shutdown.send(());
            // 正在进行的流式请求最多等 3 秒
            let abort = s.handle.abort_handle();
            if tokio::time::timeout(Duration::from_secs(3), s.handle).await.is_err() {
                abort.abort();
            }
        }
        let port = self.config().server.port;
        *self.0.status.write() = ServerStatus { running: false, port, error: None, started_at: 0 };
    }

    fn start_janitor(&self) {
        if self.0.janitor_started.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.0.processes.sync(&self.config());
        let g = self.clone();
        tokio::spawn(async move {
            loop {
                let cfg = g.config();
                let store = g.0.store.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    store.cleanup(cfg.logging.body_retention_hours, cfg.logging.stats_retention_days)
                })
                .await;
                tokio::time::sleep(Duration::from_secs(1800)).await;
            }
        });
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/", get(|| async { Json(json!({"name":"kiki-proxy","ok":true})) }))
            .route("/api/hello", get(|| async { StatusCode::OK }).head(|| async { StatusCode::OK }))
            .route("/v1/models", get(list_models))
            .route("/models", get(list_models))
            .route("/v1beta/models", get(list_gemini_models))
            .route("/v1/messages", post(anthropic_messages))
            .route("/v1/messages/count_tokens", post(anthropic_count_tokens))
            .route("/v1/chat/completions", post(openai_chat))
            .route("/chat/completions", post(openai_chat))
            .route("/v1/responses", post(openai_responses))
            .route("/responses", post(openai_responses))
            .route("/v1beta/models/{*rest}", post(gemini))
            .route("/v1/models/{*rest}", post(gemini))
            .with_state(self.clone())
    }

    // ------------------------------------------------------------------ 本地程序

    pub fn process_status(&self, endpoint_id: &str) -> Option<ProcessStatus> {
        self.0.processes.status(endpoint_id)
    }

    pub fn restart_process(&self, endpoint_id: &str) -> Result<()> {
        self.0.processes.restart(endpoint_id)
    }

    pub fn start_process(&self, endpoint_id: &str) -> Result<()> {
        self.0.processes.start(endpoint_id)
    }

    pub fn stop_process(&self, endpoint_id: &str) {
        self.0.processes.stop(endpoint_id)
    }

    /// App 退出时调用
    pub fn shutdown_processes(&self) {
        self.0.processes.stop_all();
    }

    /// 端点托管的本地程序没在运行时，给连接失败补一句原因
    fn process_hint(&self, ep: &crate::config::EndpointConfig) -> Option<String> {
        if !ep.process.enabled {
            return None;
        }
        match self.0.processes.status(&ep.id) {
            Some(st) if st.state == "running" => None,
            Some(st) => Some(format!(
                "本地程序没有在运行（{}）{}",
                match st.state.as_str() {
                    "starting" => "正在启动",
                    "restarting" => "正在重启",
                    "failed" => "启动失败",
                    _ => "已停止",
                },
                if st.last_exit.is_empty() { String::new() } else { format!("，{}", st.last_exit) }
            )),
            None => Some("本地程序没有启动（端点停用或命令为空）".into()),
        }
    }

    // ------------------------------------------------------------------ 发送

    #[allow(clippy::too_many_arguments)]
    async fn prepare(
        &self,
        cfg: &Config,
        target: &Target,
        client: Option<Protocol>,
        body: Value,
        stream: bool,
        client_headers: Option<&HeaderMap>,
        count_tokens: bool,
    ) -> Result<Prepared> {
        let ep = &target.endpoint;
        let http = self.0.clients.get(cfg, &target.proxy, &*self.0.secrets)?;
        let api_key = self.0.secrets.get(&keys::endpoint_api_key(&ep.id)).ok().flatten().unwrap_or_default();
        let mut headers = base_headers(ep, &api_key, stream)?;
        // Claude Code → Anthropic 上游：带上 beta 开关（1M 上下文、交错思考等）
        if client == Some(Protocol::Anthropic) && ep.protocol == Protocol::Anthropic {
            if let Some(h) = client_headers {
                for name in ["anthropic-beta", "anthropic-version"] {
                    if let Some(v) = h.get(name) {
                        headers.insert(name, v.clone());
                    }
                }
            }
        }
        // 标了 1M 上下文的 Claude 模型：带上 1M 的 beta 开关（Claude Code 对网关里的模型名不会自动加）
        let is_1m = target.model.as_ref().is_some_and(|m| m.context_window >= 1_000_000);
        if is_1m && ep.protocol == Protocol::Anthropic && target.upstream_model.to_lowercase().starts_with("claude") {
            let cur = headers.get("anthropic-beta").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            if !cur.contains(CONTEXT_1M_BETA) {
                let v = if cur.is_empty() { CONTEXT_1M_BETA.to_string() } else { format!("{cur},{CONTEXT_1M_BETA}") };
                headers.insert("anthropic-beta", HeaderValue::from_str(&v)?);
            }
        }
        let url = upstream_url(&ep.base_url, ep.protocol, &target.upstream_model, stream, count_tokens);
        let req = UpstreamRequest { method: reqwest::Method::POST, url, headers, body };
        Ok(Prepared { req, http })
    }

    async fn send(&self, p: &Prepared, timeout_secs: u64, fo: &crate::config::FailoverConfig) -> Result<reqwest::Response, Failure> {
        let fut = p.http.request(p.req.method.clone(), &p.req.url).headers(p.req.headers.clone()).body(p.req.body_bytes()).send();
        match tokio::time::timeout(Duration::from_secs(timeout_secs.max(5)), fut).await {
            Err(_) => Err(Failure {
                status: 504,
                message: format!("等待上游响应超时（{timeout_secs} 秒）"),
                failover: fo.on_timeout,
                raw: None,
            }),
            Ok(Err(e)) => Err(Failure { status: 502, message: describe_reqwest_error(&e), failover: fo.on_connect_error, raw: None }),
            Ok(Ok(r)) => Ok(r),
        }
    }

    // ------------------------------------------------------------------ 主流程

    pub async fn handle(
        &self,
        client: Protocol,
        path: &str,
        headers: HeaderMap,
        query: Option<String>,
        body: Bytes,
        gemini: Option<(String, String)>,
    ) -> Response {
        let cfg = self.config();
        let started = Instant::now();
        if let Err(r) = check_access(&cfg, &headers, query.as_deref(), client) {
            return r;
        }
        let json: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return error_response(client, 400, &format!("请求体不是合法的 JSON：{e}")),
        };
        let (requested, stream) = match &gemini {
            Some((m, action)) => (m.clone(), action == "streamGenerateContent"),
            None => (
                json.get("model").and_then(Value::as_str).unwrap_or("").to_string(),
                json.get("stream").and_then(Value::as_bool).unwrap_or(false),
            ),
        };
        let max_kb = cfg.logging.max_body_kb;
        let mut rec = LogRecord {
            id: new_id(""),
            ts: now_millis(),
            client_protocol: client.label().into(),
            path: path.into(),
            requested_model: requested.clone(),
            stream,
            ..Default::default()
        };
        let mut bodies = LogBodies { client_request: cap(&body, max_kb), client_headers: mask_headers(&headers), ..Default::default() };

        let chain = match router::resolve(&cfg, &requested, client) {
            Ok(first) => router::failover_chain(&cfg, first),
            Err(e) => {
                rec.status = 404;
                rec.error = e.to_string();
                rec.duration_ms = started.elapsed().as_millis() as i64;
                self.log(rec, Some(bodies));
                return error_response(client, 404, &e.to_string());
            }
        };

        let mut ir: Option<Request> = None;
        let total = chain.len();
        'targets: for (i, target) in chain.into_iter().enumerate() {
            let last = i + 1 == total;
            let ep = target.endpoint.clone();
            let up = ep.protocol;
            rec.gateway_model = target.gateway_model.clone();
            rec.endpoint_id = ep.id.clone();
            rec.endpoint_name = ep.name.clone();
            rec.upstream_model = target.upstream_model.clone();
            rec.upstream_protocol = up.label().into();
            rec.proxy = target.proxy.clone();
            rec.rule = target.rule.clone().unwrap_or_default();

            // 1. 请求体：同协议透传（只换模型名），否则经 IR 转换
            let up_body = if up == client {
                let mut b = json.clone();
                if up != Protocol::Gemini {
                    b["model"] = json!(target.upstream_model);
                }
                b
            } else {
                if ir.is_none() {
                    let mut src = json.clone();
                    if client == Protocol::Gemini {
                        src["model"] = json!(requested);
                    }
                    match protocol::parse_request(client, &src) {
                        Ok(mut r) => {
                            r.stream = stream;
                            ir = Some(r);
                        }
                        Err(e) => return error_response(client, 400, &format!("无法解析请求：{e}")),
                    }
                }
                protocol::build_request(up, ir.as_ref().unwrap(), &target.upstream_model, &ep.compat)
            };

            let attempt_start = Instant::now();
            #[allow(clippy::never_loop)]
            loop {
                let fail = |rec: &mut LogRecord, f: &Failure| {
                    rec.attempts.push(Attempt {
                        gateway_model: target.gateway_model.clone(),
                        endpoint: ep.name.clone(),
                        status: f.status,
                        error: f.message.chars().take(600).collect(),
                        duration_ms: attempt_start.elapsed().as_millis() as i64,
                    });
                };

                // 2. 鉴权、出口代理
                let prep = match self.prepare(&cfg, &target, Some(client), up_body.clone(), stream, Some(&headers), false).await {
                    Ok(p) => p,
                    Err(e) => {
                        let f = Failure { status: 502, message: format!("[{}] {e}", ep.name), failover: cfg.failover.on_connect_error, raw: None };
                        fail(&mut rec, &f);
                        if f.failover && !last {
                            continue 'targets;
                        }
                        return self.final_failure(client, up, rec, bodies, f, started);
                    }
                };
                bodies.upstream_url = prep.req.url.clone();
                bodies.upstream_headers = mask_headers(&prep.req.headers);
                bodies.upstream_request = cap(&prep.req.body_bytes(), max_kb);

                // 3. 发送
                let resp = match self.send(&prep, ep.timeout_secs, &cfg.failover).await {
                    Ok(r) => r,
                    Err(mut f) => {
                        f.message = match self.process_hint(&ep) {
                            Some(h) => format!("[{}] {}；{h}", ep.name, f.message),
                            None => format!("[{}] {}", ep.name, f.message),
                        };
                        fail(&mut rec, &f);
                        if f.failover && !last {
                            continue 'targets;
                        }
                        return self.final_failure(client, up, rec, bodies, f, started);
                    }
                };
                let status = resp.status().as_u16();
                let resp_headers = resp.headers().clone();
                let is_sse = resp_headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|c| c.contains("event-stream"));
                rec.ttfb_ms = started.elapsed().as_millis() as i64;

                // 4a. 流式成功：边收边转
                if (200..300).contains(&status) && stream && is_sse {
                    return self.respond_stream(client, up, &requested, ir.as_ref(), prep, resp, rec, bodies, started, &target);
                }

                // 4b. 其余情况：读完整个响应
                let raw = match resp.bytes().await {
                    Ok(b) => b.to_vec(),
                    Err(e) => {
                        let f = Failure {
                            status: 502,
                            message: format!("[{}] 读取上游响应失败：{}", ep.name, describe_reqwest_error(&e)),
                            failover: cfg.failover.on_connect_error,
                            raw: None,
                        };
                        fail(&mut rec, &f);
                        if f.failover && !last {
                            continue 'targets;
                        }
                        return self.final_failure(client, up, rec, bodies, f, started);
                    }
                };
                let _ = resp_headers;
                let ur = UpstreamResponse { status, body: raw };
                bodies.upstream_status = ur.status;
                bodies.upstream_response = cap(&ur.body, max_kb);

                if !(200..300).contains(&ur.status) {
                    let f = Failure {
                        status: ur.status,
                        message: format!("[{}] {}", ep.name, protocol::error_message(&ur.body)),
                        failover: cfg.failover.should_failover_status(ur.status),
                        raw: Some((ur.status, ur.body.clone())),
                    };
                    fail(&mut rec, &f);
                    if f.failover && !last {
                        continue 'targets;
                    }
                    return self.final_failure(client, up, rec, bodies, f, started);
                }
                return self.respond_full(client, up, &requested, ir.as_ref(), stream, ur.body, rec, bodies, started, &target);
            }
        }
        // chain 至少有一个元素，走不到这里
        error_response(client, 500, "没有可用的上游")
    }

    fn final_failure(&self, client: Protocol, up: Protocol, mut rec: LogRecord, bodies: LogBodies, f: Failure, started: Instant) -> Response {
        rec.status = f.status;
        rec.error = f.message.clone();
        rec.duration_ms = started.elapsed().as_millis() as i64;
        let resp = match f.raw {
            // 同协议：把上游的错误原样给客户端，信息最完整
            Some((status, body)) if up == client => raw_response(status, body, "application/json"),
            // 上下文超长：改写成 Claude Code 认识的提示，它会自动压缩上下文后重试
            _ if client == Protocol::Anthropic && matches!(f.status, 400 | 413) && is_context_overflow(&f.message) => {
                error_response(client, 400, &format!("prompt is too long: {}", f.message))
            }
            _ => {
                let msg = if rec.attempts.len() > 1 {
                    let tried: Vec<String> = rec.attempts.iter().map(|a| a.gateway_model.clone()).collect();
                    format!("{}（已依次尝试：{}）", f.message, tried.join(" → "))
                } else {
                    f.message.clone()
                };
                error_response(client, f.status, &msg)
            }
        };
        self.log(rec, Some(bodies));
        resp
    }

    #[allow(clippy::too_many_arguments)]
    fn respond_full(
        &self,
        client: Protocol,
        up: Protocol,
        requested: &str,
        ir: Option<&Request>,
        stream: bool,
        body: Vec<u8>,
        mut rec: LogRecord,
        mut bodies: LogBodies,
        started: Instant,
        target: &Target,
    ) -> Response {
        let max_kb = self.config().logging.max_body_kb;
        let parsed = serde_json::from_slice::<Value>(&body).map_err(anyhow::Error::from).and_then(|v| protocol::parse_response(up, &v));
        let ectx = EncodeCtx::new(requested, ir);
        let resp = if up == client && !stream {
            if let Ok(r) = &parsed {
                rec.usage = r.usage;
            }
            raw_response(200, body, "application/json")
        } else {
            let r = match parsed {
                Ok(r) => r,
                Err(e) => {
                    let f = Failure { status: 502, message: format!("无法解析上游响应：{e}"), failover: false, raw: None };
                    return self.final_failure(client, up, rec, bodies, f, started);
                }
            };
            rec.usage = r.usage;
            if stream {
                // 客户端要流式但上游给了完整 JSON：拆成事件再编码
                let mut norm = Normalizer::new(requested);
                let mut enc = protocol::encoder(client, ectx);
                let mut out = String::new();
                for ev in protocol::response_to_events(&r) {
                    for e in norm.push(ev) {
                        enc.encode(&e).iter().for_each(|s| out.push_str(&s.to_bytes()));
                    }
                }
                for e in norm.end() {
                    enc.encode(&e).iter().for_each(|s| out.push_str(&s.to_bytes()));
                }
                bodies.client_response = cap(out.as_bytes(), max_kb);
                sse_response(Body::from(out))
            } else {
                let v = protocol::build_response(client, &r, &ectx);
                let bytes = serde_json::to_vec(&v).unwrap_or_default();
                bodies.client_response = cap(&bytes, max_kb);
                raw_response(200, bytes, "application/json")
            }
        };
        rec.status = 200;
        rec.duration_ms = started.elapsed().as_millis() as i64;
        rec.cost = target.model.as_ref().map(|m| m.price.cost(&rec.usage)).unwrap_or(0.0);
        self.log(rec, Some(bodies));
        resp
    }

    #[allow(clippy::too_many_arguments)]
    fn respond_stream(
        &self,
        client: Protocol,
        up: Protocol,
        requested: &str,
        ir: Option<&Request>,
        _prep: Prepared,
        resp: reqwest::Response,
        mut rec: LogRecord,
        mut bodies: LogBodies,
        started: Instant,
        target: &Target,
    ) -> Response {
        let (tx, rx) = mpsc::channel::<Bytes>(64);
        let passthrough = up == client;
        let mut norm = Normalizer::new(requested);
        let mut dec = protocol::decoder(up);
        let mut enc = protocol::encoder(client, EncodeCtx::new(requested, ir));
        let price = target.model.as_ref().map(|m| m.price.clone()).unwrap_or_default();
        let max_bytes = self.config().logging.max_body_kb as usize * 1024;
        let g = self.clone();
        bodies.upstream_status = resp.status().as_u16();

        // 进行中的请求先推给界面
        let mut pending = rec.clone();
        pending.pending = true;
        pending.status = 200;
        let _ = self.0.live.send(pending);

        tokio::spawn(async move {
            let mut parser = SseParser::new();
            let mut up_log = String::new();
            let mut client_log = String::new();
            let mut upstream = resp.bytes_stream();
            let mut client_gone = false;
            let mut stream_error: Option<String> = None;

            // 处理一个上游 SSE 事件，返回要发给客户端的字节
            let mut handle_event = |ev: SseEvent, norm: &mut Normalizer, out: &mut String| {
                {
                    if passthrough {
                        out.push_str(&ev.to_bytes());
                        for e in dec.feed(&ev) {
                            norm.push(e); // 只为了统计 usage
                        }
                    } else {
                        for e in dec.feed(&ev) {
                            for ne in norm.push(e) {
                                enc.encode(&ne).iter().for_each(|s| out.push_str(&s.to_bytes()));
                            }
                        }
                    }
                }
            };

            loop {
                // 上游 5 分钟没有任何数据视为断流
                let next = match tokio::time::timeout(Duration::from_secs(300), upstream.next()).await {
                    Ok(n) => n,
                    Err(_) => {
                        stream_error = Some("上游 5 分钟没有返回数据，已断开".into());
                        break;
                    }
                };
                let Some(chunk) = next else { break };
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        stream_error = Some(format!("上游连接中断：{}", describe_reqwest_error(&e)));
                        break;
                    }
                };
                if up_log.len() < max_bytes {
                    up_log.push_str(&String::from_utf8_lossy(&chunk));
                }
                let mut out = String::new();
                for ev in parser.feed(&chunk) {
                    handle_event(ev, &mut norm, &mut out);
                }
                if !out.is_empty() {
                    if client_log.len() < max_bytes {
                        client_log.push_str(&out);
                    }
                    if tx.send(Bytes::from(out)).await.is_err() {
                        client_gone = true;
                        break;
                    }
                }
            }

            // 收尾：解析器里剩下的、解码器的结束事件、规范化器补齐
            let mut out = String::new();
            if !client_gone {
                for ev in parser.finish() {
                    handle_event(ev, &mut norm, &mut out);
                }
                if passthrough {
                    for e in dec.finish() {
                        norm.push(e);
                    }
                } else {
                    let mut tail = dec.finish();
                    if let Some(msg) = &stream_error {
                        tail.push(crate::ir::Event::Error { message: msg.clone() });
                    }
                    for e in tail {
                        for ne in norm.push(e) {
                            enc.encode(&ne).iter().for_each(|s| out.push_str(&s.to_bytes()));
                        }
                    }
                    for ne in norm.end() {
                        enc.encode(&ne).iter().for_each(|s| out.push_str(&s.to_bytes()));
                    }
                }
                if !out.is_empty() {
                    client_log.push_str(&out);
                    let _ = tx.send(Bytes::from(out)).await;
                }
            }
            drop(tx);

            rec.usage = norm.usage;
            rec.cost = price.cost(&rec.usage);
            rec.duration_ms = started.elapsed().as_millis() as i64;
            rec.status = 200;
            if let Some(e) = stream_error.or(norm.error.clone()) {
                rec.status = 502;
                rec.error = e;
            } else if client_gone {
                rec.error = "客户端提前断开".into();
            }
            bodies.upstream_response = truncate_str(up_log, max_bytes);
            bodies.client_response = if passthrough { "（与上游响应相同，透传）".into() } else { truncate_str(client_log, max_bytes) };
            g.log(rec, Some(bodies));
        });

        let body_stream = futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|b| (Ok::<_, std::convert::Infallible>(b), rx)) });
        sse_response(Body::from_stream(body_stream))
    }

    fn log(&self, rec: LogRecord, bodies: Option<LogBodies>) {
        let _ = self.0.live.send(rec.clone());
        let store = self.0.store.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = store.insert(&rec, bodies.as_ref()) {
                tracing::warn!("写日志失败：{e}");
            }
        });
    }

    // ------------------------------------------------------------------ count_tokens

    async fn count_tokens(&self, client: Protocol, headers: HeaderMap, query: Option<String>, body: Bytes, model: Option<String>) -> Response {
        let cfg = self.config();
        if let Err(r) = check_access(&cfg, &headers, query.as_deref(), client) {
            return r;
        }
        let mut json: Value = serde_json::from_slice(&body).unwrap_or_default();
        if let Some(m) = &model {
            json["model"] = json!(m);
        }
        let requested = json.get("model").and_then(Value::as_str).unwrap_or("").to_string();
        // Anthropic 上游有真正的 count_tokens 接口，直接转发
        if client == Protocol::Anthropic {
            if let Ok(t) = router::resolve(&cfg, &requested, client) {
                if t.endpoint.protocol == Protocol::Anthropic {
                    let mut b = json.clone();
                    b["model"] = json!(t.upstream_model);
                    if let Ok(p) = self.prepare(&cfg, &t, Some(client), b, false, Some(&headers), true).await {
                        if let Ok(r) = self.send(&p, 30, &cfg.failover).await {
                            if r.status().is_success() {
                                if let Ok(bytes) = r.bytes().await {
                                    return raw_response(200, bytes.to_vec(), "application/json");
                                }
                            }
                        }
                    }
                }
            }
        }
        let n = protocol::parse_request(client, &json).map(|r| estimate_tokens(&r)).unwrap_or(1);
        match client {
            Protocol::Gemini => Json(json!({ "totalTokens": n })).into_response(),
            _ => Json(json!({ "input_tokens": n })).into_response(),
        }
    }

    // ------------------------------------------------------------------ 测试

    // ------------------------------------------------------------------ 价格表与上游模型

    pub fn pricing_status(&self) -> crate::pricing::PricingStatus {
        self.0.prices.status(&self.config().pricing.url)
    }

    pub async fn refresh_pricing(&self) -> Result<crate::pricing::PricingStatus> {
        let cfg = self.config();
        let http = self.0.clients.get(&cfg, &cfg.pricing.proxy, &*self.0.secrets)?;
        self.0.prices.refresh(&http, &cfg.pricing.url).await
    }

    /// 查一批模型的价格 / 上下文；本地还没有价格表时先下载
    pub async fn lookup_models(
        &self,
        models: &[String],
        base_url: &str,
        protocol: Protocol,
    ) -> Result<Vec<Option<crate::pricing::ModelInfo>>> {
        if self.0.prices.is_empty() {
            self.refresh_pricing().await?;
        }
        let hint = crate::pricing::provider_hint(base_url, protocol);
        Ok(models.iter().map(|m| self.0.prices.lookup(m, hint)).collect())
    }

    /// 从上游的 /models 接口拉取模型列表（端点需要先保存）
    pub async fn list_upstream_models(&self, endpoint_id: &str) -> Result<Vec<String>> {
        let cfg = self.config();
        let ep = cfg.endpoint(endpoint_id).cloned().ok_or_else(|| anyhow!("端点不存在，请先保存"))?;
        let target = Target {
            gateway_model: String::new(),
            upstream_model: String::new(),
            proxy: ep.proxy.clone(),
            model: None,
            endpoint: ep.clone(),
            rule: None,
        };
        let prep = self.prepare(&cfg, &target, None, Value::Null, false, None, false).await?;
        let b = ep.base_url.trim().trim_end_matches('/');
        let versioned = b.rsplit('/').next().is_some_and(|s| s.len() > 1 && s.starts_with('v') && s[1..2].chars().all(|c| c.is_ascii_digit()));
        let url = match ep.protocol {
            Protocol::Gemini => format!("{}/models?pageSize=1000", if versioned { b.to_string() } else { format!("{b}/v1beta") }),
            _ => format!("{}/models", if versioned { b.to_string() } else { format!("{b}/v1") }),
        };
        let mut headers = prep.req.headers.clone();
        headers.remove("content-type");
        let resp = tokio::time::timeout(Duration::from_secs(30), prep.http.get(&url).headers(headers).send())
            .await
            .map_err(|_| anyhow!("获取模型列表超时"))?
            .map_err(|e| anyhow!(describe_reqwest_error(&e)))?;
        let status = resp.status();
        let body = resp.bytes().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("上游返回 {status}：{}（不是所有服务都提供模型列表接口）", protocol::error_message(&body)));
        }
        let v: Value = serde_json::from_slice(&body).map_err(|_| anyhow!("上游的模型列表不是 JSON"))?;
        let mut ids: Vec<String> = match ep.protocol {
            Protocol::Gemini => v
                .get("models")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|m| {
                    m.get("supportedGenerationMethods")
                        .and_then(Value::as_array)
                        .is_none_or(|a| a.iter().any(|x| x.as_str() == Some("generateContent")))
                })
                .filter_map(|m| m.get("name").and_then(Value::as_str))
                .map(|n| n.strip_prefix("models/").unwrap_or(n).to_string())
                .collect(),
            _ => v
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                .collect(),
        };
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// 用一个最小请求测试端点是否可用
    pub async fn test_endpoint(&self, endpoint_id: &str, model: Option<String>) -> TestResult {
        let cfg = self.config();
        let Some(ep) = cfg.endpoint(endpoint_id).cloned() else {
            return TestResult { message: "端点不存在".into(), ..Default::default() };
        };
        let m = match model.filter(|m| !m.is_empty()) {
            Some(m) => ep.models.iter().find(|x| x.name == m).cloned(),
            None => ep.models.iter().find(|x| x.enabled).cloned(),
        };
        let Some(m) = m else {
            return TestResult { message: "这个端点还没有配置模型".into(), ..Default::default() };
        };
        self.test_model(&cfg, &ep, m).await
    }

    /// 分步测试：网络 → 登录 / 鉴权 → 每个模型能否回复。前一步失败就停下。
    /// 界面上是逐步调用下面三个 `test_step_*`，每完成一步就显示一步；这里是一次跑完的版本。
    pub async fn test_endpoint_report(&self, endpoint_id: &str) -> TestReport {
        let mut steps = Vec::new();
        let done = |steps: Vec<TestStep>| TestReport { ok: steps.iter().all(|s| s.status != "fail"), steps };
        let net = self.test_step_network(endpoint_id).await;
        let failed = net.status == "fail";
        steps.push(net);
        if failed {
            return done(steps);
        }
        let auth = self.test_step_auth(endpoint_id).await;
        let failed = auth.status == "fail";
        steps.push(auth);
        if failed {
            return done(steps);
        }
        let models: Vec<String> = self
            .config()
            .endpoint(endpoint_id)
            .map(|e| e.models.iter().filter(|m| m.enabled).take(12).map(|m| m.name.clone()).collect())
            .unwrap_or_default();
        if models.is_empty() {
            steps.push(TestStep::new("model", "模型", "fail", "还没有配置模型", 0));
            return done(steps);
        }
        steps.extend(futures::future::join_all(models.iter().map(|m| self.test_step_model(endpoint_id, m))).await);
        done(steps)
    }

    /// 第 1 步：通过端点的出口访问上游地址，收到任何 HTTP 响应都算通
    pub async fn test_step_network(&self, endpoint_id: &str) -> TestStep {
        let cfg = self.config();
        let Some(ep) = cfg.endpoint(endpoint_id).cloned() else {
            return TestStep::new("network", "网络", "fail", "端点不存在，请先保存", 0);
        };
        let proxy_name = match ep.proxy.as_str() {
            crate::config::PROXY_DIRECT => "直连".to_string(),
            crate::config::PROXY_SYSTEM | "" => "系统代理".to_string(),
            id => cfg.proxy(id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string()),
        };
        let host = url::Url::parse(&ep.base_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        if let Some(h) = self.process_hint(&ep) {
            return TestStep::new("network", "网络", "fail", h, 0);
        }
        let t0 = Instant::now();
        let net = match self.0.clients.get(&cfg, &ep.proxy, &*self.0.secrets) {
            Err(e) => Err(e.to_string()),
            Ok(http) => match tokio::time::timeout(Duration::from_secs(15), http.get(&ep.base_url).send()).await {
                Err(_) => Err("15 秒内没有响应".to_string()),
                Ok(Err(e)) => Err(describe_reqwest_error(&e)),
                Ok(Ok(r)) => Ok(r.status().as_u16()),
            },
        };
        let ms = t0.elapsed().as_millis() as i64;
        match net {
            Ok(_) => TestStep::new("network", "网络", "ok", format!("经{proxy_name}访问 {host} 正常"), ms),
            Err(e) => TestStep::new("network", "网络", "fail", format!("经{proxy_name}访问 {host} 失败：{e}"), ms),
        }
    }

    /// 第 2 步：检查是否配置了 API Key（真正有效与否在第 3 步验证）
    pub async fn test_step_auth(&self, endpoint_id: &str) -> TestStep {
        let cfg = self.config();
        let Some(ep) = cfg.endpoint(endpoint_id).cloned() else {
            return TestStep::new("auth", "鉴权", "fail", "端点不存在，请先保存", 0);
        };
        if self.0.secrets.get(&keys::endpoint_api_key(&ep.id)).ok().flatten().is_some_and(|k| !k.is_empty()) {
            TestStep::new("auth", "鉴权", "ok", "已配置 API Key，下一步实际调用模型时验证", 0)
        } else {
            TestStep::new("auth", "鉴权", "skip", "没有配置 API Key（服务不需要的话可以忽略）", 0)
        }
    }

    /// 第 3 步：给一个模型发一句话，看能否正常回复
    pub async fn test_step_model(&self, endpoint_id: &str, model: &str) -> TestStep {
        let cfg = self.config();
        let Some(ep) = cfg.endpoint(endpoint_id).cloned() else {
            return TestStep::new("model", model, "fail", "端点不存在，请先保存", 0);
        };
        let Some(m) = ep.models.iter().find(|m| m.name == model).cloned() else {
            return TestStep::new("model", model, "fail", "端点里没有这个模型", 0);
        };
        let r = self.test_model(&cfg, &ep, m).await;
        let detail = if r.ok {
            format!("回复了「{}」", r.message)
        } else if matches!(r.status, 401 | 403) {
            format!("HTTP {}：{}（鉴权失败：检查 API Key 或登录状态、是否有这个模型的权限）", r.status, r.message)
        } else if r.status == 404 {
            format!("HTTP 404：{}（模型名可能不对）", r.message)
        } else if r.status > 0 {
            format!("HTTP {}：{}", r.status, r.message)
        } else {
            r.message.clone()
        };
        TestStep::new("model", model, if r.ok { "ok" } else { "fail" }, detail, r.latency_ms)
    }

    async fn test_model(&self, cfg: &Config, ep: &crate::config::EndpointConfig, m: crate::config::ModelConfig) -> TestResult {
        let cfg = cfg.clone();
        let ep = ep.clone();
        let target = Target {
            gateway_model: format!("{}/{}", ep.id, m.name),
            upstream_model: m.upstream_id().to_string(),
            proxy: ep.proxy.clone(),
            model: Some(m),
            endpoint: ep.clone(),
            rule: None,
        };
        let ir = Request {
            model: target.upstream_model.clone(),
            messages: vec![Message { role: Role::User, content: vec![Part::Text("Hi, reply with one word.".into())] }],
            max_tokens: Some(32),
            ..Default::default()
        };
        let body = protocol::build_request(ep.protocol, &ir, &target.upstream_model, &ep.compat);
        let t0 = Instant::now();
        let prep = match self.prepare(&cfg, &target, None, body, false, None, false).await {
            Ok(p) => p,
            Err(e) => return TestResult { message: e.to_string(), ..Default::default() },
        };
        let resp = match self.send(&prep, ep.timeout_secs.min(60), &cfg.failover).await {
            Ok(r) => r,
            Err(f) => return TestResult { status: f.status, message: f.message, latency_ms: t0.elapsed().as_millis() as i64, ok: false },
        };
        let status = resp.status().as_u16();
        let ur = UpstreamResponse { status, body: resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default() };
        let latency_ms = t0.elapsed().as_millis() as i64;
        if !(200..300).contains(&status) {
            return TestResult { ok: false, status, latency_ms, message: protocol::error_message(&ur.body) };
        }
        let reply = serde_json::from_slice::<Value>(&ur.body)
            .ok()
            .and_then(|v| protocol::parse_response(ep.protocol, &v).ok())
            .map(|r| crate::ir::parts_text(&r.content))
            .unwrap_or_default();
        TestResult { ok: true, status, latency_ms, message: if reply.is_empty() { "可用".into() } else { reply.chars().take(80).collect() } }
    }

    /// 测试出口代理：访问探测地址
    pub async fn test_proxy(&self, proxy_id: &str) -> TestResult {
        let cfg = self.config();
        let http = match self.0.clients.get(&cfg, proxy_id, &*self.0.secrets) {
            Ok(c) => c,
            Err(e) => return TestResult { message: e.to_string(), ..Default::default() },
        };
        let t0 = Instant::now();
        match tokio::time::timeout(Duration::from_secs(15), http.get(&cfg.ui.probe_url).send()).await {
            Err(_) => TestResult { message: "超时（15 秒）".into(), latency_ms: 15000, ..Default::default() },
            Ok(Err(e)) => TestResult { message: describe_reqwest_error(&e), latency_ms: t0.elapsed().as_millis() as i64, ..Default::default() },
            Ok(Ok(r)) => {
                let status = r.status().as_u16();
                TestResult {
                    ok: status < 500,
                    status,
                    latency_ms: t0.elapsed().as_millis() as i64,
                    message: format!("HTTP {status}"),
                }
            }
        }
    }
}

// ------------------------------------------------------------------ axum 处理函数

struct Json(Value);

impl IntoResponse for Json {
    fn into_response(self) -> Response {
        raw_response(200, serde_json::to_vec(&self.0).unwrap_or_default(), "application/json")
    }
}

async fn anthropic_messages(State(g): State<Gateway>, headers: HeaderMap, RawQuery(q): RawQuery, body: Bytes) -> Response {
    g.handle(Protocol::Anthropic, "/v1/messages", headers, q, body, None).await
}

async fn anthropic_count_tokens(State(g): State<Gateway>, headers: HeaderMap, RawQuery(q): RawQuery, body: Bytes) -> Response {
    g.count_tokens(Protocol::Anthropic, headers, q, body, None).await
}

async fn openai_chat(State(g): State<Gateway>, headers: HeaderMap, RawQuery(q): RawQuery, body: Bytes) -> Response {
    g.handle(Protocol::OpenaiChat, "/v1/chat/completions", headers, q, body, None).await
}

async fn openai_responses(State(g): State<Gateway>, headers: HeaderMap, RawQuery(q): RawQuery, body: Bytes) -> Response {
    g.handle(Protocol::OpenaiResponses, "/v1/responses", headers, q, body, None).await
}

async fn gemini(State(g): State<Gateway>, AxPath(rest): AxPath<String>, headers: HeaderMap, RawQuery(q): RawQuery, body: Bytes) -> Response {
    // rest 形如 `ds/deepseek-chat:streamGenerateContent`（模型名里可能有 /）
    let Some((model, action)) = rest.rsplit_once(':') else {
        return error_response(Protocol::Gemini, 404, "未知的 Gemini 接口");
    };
    let model = model.strip_prefix("models/").unwrap_or(model).to_string();
    match action {
        "generateContent" | "streamGenerateContent" => {
            g.handle(Protocol::Gemini, &format!("/v1beta/models/{rest}"), headers, q, body, Some((model, action.to_string()))).await
        }
        "countTokens" => g.count_tokens(Protocol::Gemini, headers, q, body, Some(model)).await,
        _ => error_response(Protocol::Gemini, 404, &format!("不支持的 Gemini 接口：{action}")),
    }
}

async fn list_models(State(g): State<Gateway>, headers: HeaderMap, RawQuery(query): RawQuery) -> Response {
    let cfg = g.config();
    // Claude Code 的模型发现只保留名字里带 claude / anthropic 的模型
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let for_claude = cfg.routing.claude_alias
        && (headers.contains_key("anthropic-version") || headers.contains_key("x-api-key") || ua.contains("claude") || query.as_deref().is_some_and(|q| q.contains("limit=")));
    let created = crate::ir::now_secs();
    let data: Vec<Value> = router::list_models(&cfg)
        .into_iter()
        .map(|m| {
            let lower = m.id.to_lowercase();
            let mut id = if for_claude && !lower.contains("claude") && !lower.contains("anthropic") {
                format!("{CLAUDE_ALIAS}{}", m.id)
            } else {
                m.id.clone()
            };
            // Claude Code 看到 [1m] 后缀就按 1M 上下文使用（发请求前会去掉后缀）
            if for_claude && m.context_window >= 1_000_000 {
                id.push_str("[1m]");
            }
            json!({
                "id": id, "object": "model", "type": "model", "created": created,
                "created_at": "2025-01-01T00:00:00Z", "owned_by": m.endpoint,
                "display_name": m.display_name, "description": format!("{} · kiki-proxy", m.endpoint),
                "max_input_tokens": if m.context_window > 0 { json!(m.context_window) } else { Value::Null },
                "supports_1m": m.context_window >= 1_000_000,
            })
        })
        .collect();
    let first = data.first().and_then(|d| d.get("id")).cloned().unwrap_or(Value::Null);
    let last = data.last().and_then(|d| d.get("id")).cloned().unwrap_or(Value::Null);
    Json(json!({ "object": "list", "data": data, "has_more": false, "first_id": first, "last_id": last })).into_response()
}

async fn list_gemini_models(State(g): State<Gateway>) -> Response {
    let models: Vec<Value> = router::list_models(&g.config())
        .into_iter()
        .map(|m| {
            json!({
                "name": format!("models/{}", m.id), "displayName": m.display_name,
                "supportedGenerationMethods": ["generateContent", "streamGenerateContent", "countTokens"],
            })
        })
        .collect();
    Json(json!({ "models": models })).into_response()
}

// ------------------------------------------------------------------ 小工具

#[allow(clippy::result_large_err)]
fn check_access(cfg: &Config, headers: &HeaderMap, query: Option<&str>, client: Protocol) -> Result<(), Response> {
    let key = cfg.server.access_key.trim();
    if key.is_empty() {
        return Ok(());
    }
    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    let provided = h("x-api-key")
        .or_else(|| h("authorization").map(|a| a.trim_start_matches("Bearer ").trim().to_string()))
        .or_else(|| h("x-goog-api-key"))
        .or_else(|| {
            query.and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "key").map(|(_, v)| v.to_string()))
        });
    if provided.as_deref() == Some(key) {
        Ok(())
    } else {
        Err(error_response(client, 401, "kiki-proxy 访问密钥不正确"))
    }
}

pub fn error_response(client: Protocol, status: u16, message: &str) -> Response {
    let body = protocol::error_body(client, status, message);
    raw_response(status, serde_json::to_vec(&body).unwrap_or_default(), "application/json")
}

fn raw_response(status: u16, body: Vec<u8>, content_type: &str) -> Response {
    let mut r = Response::new(Body::from(body));
    *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    r.headers_mut().insert("content-type", HeaderValue::from_str(content_type).unwrap_or(HeaderValue::from_static("application/json")));
    r
}

fn sse_response(body: Body) -> Response {
    let mut r = Response::new(body);
    let h = r.headers_mut();
    h.insert("content-type", HeaderValue::from_static("text/event-stream; charset=utf-8"));
    h.insert("cache-control", HeaderValue::from_static("no-cache"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    r
}

const CONTEXT_1M_BETA: &str = "context-1m-2025-08-07";

/// 各家「上下文超长」错误的常见说法
fn is_context_overflow(msg: &str) -> bool {
    let m = msg.to_lowercase();
    [
        "context_length_exceeded",
        "maximum context length",
        "context length",
        "context window",
        "prompt is too long",
        "input is too long",
        "too many tokens",
        "token limit",
        "exceeds the maximum number of tokens",
        "input token count",
    ]
    .iter()
    .any(|p| m.contains(p))
}

fn describe_reqwest_error(e: &reqwest::Error) -> String {
    use std::error::Error as _;
    let mut msg = if e.is_connect() {
        "连接失败".to_string()
    } else if e.is_timeout() {
        "超时".to_string()
    } else if e.is_body() || e.is_decode() {
        "读取响应失败".to_string()
    } else {
        "请求失败".to_string()
    };
    let mut src = e.source();
    let mut detail = Vec::new();
    while let Some(s) = src {
        detail.push(s.to_string());
        src = s.source();
    }
    if let Some(d) = detail.last() {
        msg.push('：');
        msg.push_str(d);
    }
    msg
}

fn cap(bytes: &[u8], max_kb: u32) -> String {
    truncate_str(String::from_utf8_lossy(bytes).to_string(), max_kb as usize * 1024)
}

fn truncate_str(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        let total = s.len();
        s.truncate(cut);
        s.push_str(&format!("\n…（已截断，原始大小 {} KB）", total / 1024));
    }
    s
}
