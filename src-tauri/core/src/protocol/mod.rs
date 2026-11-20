//! 4 种协议的适配器，以及按协议分发的入口。

pub mod anthropic;
pub mod gemini;
pub mod openai_chat;
pub mod openai_responses;

use crate::ir::{new_id, BlockKind, Event, Protocol, Request, Response, StopReason, Usage};
use crate::sse::SseEvent;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

/// 端点级的兼容开关：不同厂商对「OpenAI 兼容」的理解不完全一样
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Compat {
    /// OpenAI Chat：把思考内容放回 assistant 消息的 `reasoning_content`（DeepSeek 工具调用时需要）
    pub send_reasoning_content: bool,
    /// OpenAI Chat：用 `max_completion_tokens` 代替 `max_tokens`（OpenAI o 系列 / GPT-5 需要）
    pub use_max_completion_tokens: bool,
    /// OpenAI Chat：发送 `reasoning_effort`
    pub send_reasoning_effort: bool,
    /// 不发送 temperature / top_p（部分推理模型不接受）
    pub drop_sampling_params: bool,
}

/// 生成客户端响应时需要的上下文
#[derive(Debug, Clone, Default)]
pub struct EncodeCtx {
    /// 回给客户端的模型名（客户端请求的那个名字）
    pub model: String,
    /// OpenAI Responses 的 custom tool 名字（要还原成 custom_tool_call）
    pub custom_tools: HashSet<String>,
}

impl EncodeCtx {
    pub fn new(model: &str, req: Option<&Request>) -> Self {
        let custom_tools = req
            .map(|r| {
                r.tools
                    .iter()
                    .filter(|t| t.kind == crate::ir::ToolKind::Custom)
                    .map(|t| t.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        EncodeCtx { model: model.to_string(), custom_tools }
    }
}

pub trait StreamDecoder: Send {
    /// 上游的一个 SSE 事件 → IR 事件
    fn feed(&mut self, ev: &SseEvent) -> Vec<Event>;
    /// 上游流结束
    fn finish(&mut self) -> Vec<Event> {
        Vec::new()
    }
}

pub trait StreamEncoder: Send {
    /// IR 事件 → 发给客户端的 SSE 事件
    fn encode(&mut self, ev: &Event) -> Vec<SseEvent>;
}

pub fn parse_request(p: Protocol, body: &Value) -> Result<Request> {
    match p {
        Protocol::Anthropic => anthropic::parse_request(body),
        Protocol::OpenaiChat => openai_chat::parse_request(body),
        Protocol::OpenaiResponses => openai_responses::parse_request(body),
        Protocol::Gemini => gemini::parse_request(body),
    }
}

pub fn build_request(p: Protocol, req: &Request, model: &str, compat: &Compat) -> Value {
    match p {
        Protocol::Anthropic => anthropic::build_request(req, model),
        Protocol::OpenaiChat => openai_chat::build_request(req, model, compat),
        Protocol::OpenaiResponses => openai_responses::build_request(req, model, compat),
        Protocol::Gemini => gemini::build_request(req, compat),
    }
}

pub fn parse_response(p: Protocol, body: &Value) -> Result<Response> {
    match p {
        Protocol::Anthropic => anthropic::parse_response(body),
        Protocol::OpenaiChat => openai_chat::parse_response(body),
        Protocol::OpenaiResponses => openai_responses::parse_response(body),
        Protocol::Gemini => gemini::parse_response(body),
    }
}

pub fn build_response(p: Protocol, resp: &Response, ctx: &EncodeCtx) -> Value {
    match p {
        Protocol::Anthropic => anthropic::build_response(resp, ctx),
        Protocol::OpenaiChat => openai_chat::build_response(resp, ctx),
        Protocol::OpenaiResponses => openai_responses::build_response(resp, ctx),
        Protocol::Gemini => gemini::build_response(resp, ctx),
    }
}

pub fn decoder(p: Protocol) -> Box<dyn StreamDecoder> {
    match p {
        Protocol::Anthropic => Box::new(anthropic::Decoder::default()),
        Protocol::OpenaiChat => Box::new(openai_chat::Decoder::default()),
        Protocol::OpenaiResponses => Box::new(openai_responses::Decoder::default()),
        Protocol::Gemini => Box::new(gemini::Decoder::default()),
    }
}

pub fn encoder(p: Protocol, ctx: EncodeCtx) -> Box<dyn StreamEncoder> {
    match p {
        Protocol::Anthropic => Box::new(anthropic::Encoder::new(ctx)),
        Protocol::OpenaiChat => Box::new(openai_chat::Encoder::new(ctx)),
        Protocol::OpenaiResponses => Box::new(openai_responses::Encoder::new(ctx)),
        Protocol::Gemini => Box::new(gemini::Encoder::new(ctx)),
    }
}

/// 按客户端协议的格式生成错误响应体
pub fn error_body(p: Protocol, status: u16, message: &str) -> Value {
    match p {
        Protocol::Anthropic => serde_json::json!({
            "type": "error",
            "error": { "type": anthropic_error_type(status), "message": message }
        }),
        Protocol::OpenaiChat | Protocol::OpenaiResponses => serde_json::json!({
            "error": { "message": message, "type": "kiki_proxy_error", "code": status }
        }),
        Protocol::Gemini => serde_json::json!({
            "error": { "code": status, "message": message, "status": gemini_status(status) }
        }),
    }
}

fn anthropic_error_type(status: u16) -> &'static str {
    match status {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        529 => "overloaded_error",
        _ => "api_error",
    }
}

fn gemini_status(status: u16) -> &'static str {
    match status {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        429 => "RESOURCE_EXHAUSTED",
        503 => "UNAVAILABLE",
        _ => "INTERNAL",
    }
}

/// 尽量从各种格式的错误体里抽出一句人话
pub fn error_message(body: &[u8]) -> String {
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        let candidates = [
            v.pointer("/error/message"),
            v.pointer("/message"),
            v.pointer("/error"),
            v.pointer("/detail"),
            v.pointer("/msg"),
        ];
        for c in candidates.into_iter().flatten() {
            if let Some(s) = c.as_str() {
                return s.to_string();
            }
        }
        return v.to_string();
    }
    let s = String::from_utf8_lossy(body);
    s.chars().take(2000).collect()
}

// ------------------------------------------------------------------ 流规范化

/// 夹在上游解码器和客户端编码器之间，保证事件序列总是合法的：
/// 先 Start；块先开后关；结束前关掉所有块；只 Finish 一次。
/// 这样每个编码器都不用操心上游的各种不规范行为。
pub struct Normalizer {
    model: String,
    started: bool,
    finished: bool,
    open: BTreeSet<usize>,
    kinds: std::collections::HashMap<usize, BlockKind>,
    /// 汇总后的 usage（日志和统计用）
    pub usage: Usage,
    pub stop_reason: Option<StopReason>,
    pub error: Option<String>,
}

impl Normalizer {
    pub fn new(model: &str) -> Self {
        Normalizer {
            model: model.to_string(),
            started: false,
            finished: false,
            open: BTreeSet::new(),
            kinds: Default::default(),
            usage: Usage::default(),
            stop_reason: None,
            error: None,
        }
    }

    fn ensure_started(&mut self, out: &mut Vec<Event>) {
        if !self.started {
            self.started = true;
            out.push(Event::Start { id: new_id("msg_"), model: self.model.clone(), usage: Usage::default() });
        }
    }

    fn ensure_block(&mut self, index: usize, kind: BlockKind, out: &mut Vec<Event>) -> bool {
        if self.open.contains(&index) {
            return true;
        }
        if matches!(kind, BlockKind::ToolUse { .. }) {
            return false; // 没有 id/name 的工具参数无法补救，丢掉
        }
        self.open.insert(index);
        self.kinds.insert(index, kind.clone());
        out.push(Event::BlockStart { index, kind });
        true
    }

    pub fn push(&mut self, ev: Event) -> Vec<Event> {
        let mut out = Vec::new();
        if self.finished {
            return out;
        }
        match ev {
            Event::Start { id, model, usage } => {
                self.usage.merge(&usage);
                if !self.started {
                    self.started = true;
                    out.push(Event::Start { id, model, usage });
                }
            }
            Event::BlockStart { index, kind } => {
                self.ensure_started(&mut out);
                if !self.open.contains(&index) {
                    self.open.insert(index);
                    self.kinds.insert(index, kind.clone());
                    out.push(Event::BlockStart { index, kind });
                }
            }
            Event::TextDelta { index, text } => {
                self.ensure_started(&mut out);
                if self.ensure_block(index, BlockKind::Text, &mut out) {
                    out.push(Event::TextDelta { index, text });
                }
            }
            Event::ThinkingDelta { index, text } => {
                self.ensure_started(&mut out);
                if self.ensure_block(index, BlockKind::Thinking, &mut out) {
                    out.push(Event::ThinkingDelta { index, text });
                }
            }
            Event::SignatureDelta { index, signature } => {
                self.ensure_started(&mut out);
                if self.ensure_block(index, BlockKind::Thinking, &mut out) {
                    out.push(Event::SignatureDelta { index, signature });
                }
            }
            Event::ToolArgsDelta { index, json } => {
                self.ensure_started(&mut out);
                if self.ensure_block(index, BlockKind::ToolUse { id: String::new(), name: String::new() }, &mut out) {
                    out.push(Event::ToolArgsDelta { index, json });
                }
            }
            Event::BlockStop { index } => {
                if self.open.remove(&index) {
                    out.push(Event::BlockStop { index });
                }
            }
            Event::Finish { stop_reason, usage } => {
                self.ensure_started(&mut out);
                self.close_all(&mut out);
                self.usage.merge(&usage);
                // 有工具调用但上游给的停止原因不对时纠正（部分兼容服务会这样）
                let has_tool = self.kinds.values().any(|k| matches!(k, BlockKind::ToolUse { .. }));
                let stop_reason = if has_tool && stop_reason == StopReason::EndTurn { StopReason::ToolUse } else { stop_reason };
                self.stop_reason = Some(stop_reason);
                self.finished = true;
                out.push(Event::Finish { stop_reason, usage: self.usage });
            }
            Event::Error { message } => {
                self.ensure_started(&mut out);
                self.close_all(&mut out);
                self.error = Some(message.clone());
                self.finished = true;
                out.push(Event::Error { message });
            }
        }
        out
    }

    fn close_all(&mut self, out: &mut Vec<Event>) {
        for i in std::mem::take(&mut self.open) {
            out.push(Event::BlockStop { index: i });
        }
    }

    /// 上游流结束（可能没有正常的结束事件）
    pub fn end(&mut self) -> Vec<Event> {
        if self.finished {
            return Vec::new();
        }
        self.push(Event::Finish { stop_reason: StopReason::EndTurn, usage: Usage::default() })
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

/// 把一组 IR 事件聚合成完整的响应（测试和日志用）
pub fn aggregate(events: &[Event]) -> Response {
    use crate::ir::Part;
    let mut resp = Response::default();
    let mut blocks: std::collections::BTreeMap<usize, (BlockKind, String, Option<String>)> = Default::default();
    for ev in events {
        match ev {
            Event::Start { id, model, usage } => {
                resp.id = id.clone();
                resp.model = model.clone();
                resp.usage.merge(usage);
            }
            Event::BlockStart { index, kind } => {
                blocks.insert(*index, (kind.clone(), String::new(), None));
            }
            Event::TextDelta { index, text } | Event::ThinkingDelta { index, text } | Event::ToolArgsDelta { index, json: text } => {
                if let Some(b) = blocks.get_mut(index) {
                    b.1.push_str(text);
                }
            }
            Event::SignatureDelta { index, signature } => {
                if let Some(b) = blocks.get_mut(index) {
                    b.2 = Some(signature.clone());
                }
            }
            Event::Finish { stop_reason, usage } => {
                resp.stop_reason = *stop_reason;
                resp.usage.merge(usage);
            }
            _ => {}
        }
    }
    for (_, (kind, buf, sig)) in blocks {
        resp.content.push(match kind {
            BlockKind::Text => Part::Text(buf),
            BlockKind::Thinking => Part::Thinking { text: buf, signature: sig },
            BlockKind::ToolUse { id, name } => Part::ToolUse {
                id,
                name,
                input: serde_json::from_str(&buf).unwrap_or_else(|_| serde_json::json!({})),
            },
        });
    }
    resp
}

/// 把完整响应拆成事件（上游非流式、客户端要流式时用；目前用于测试）
pub fn response_to_events(resp: &Response) -> Vec<Event> {
    use crate::ir::Part;
    let mut evs = vec![Event::Start { id: resp.id.clone(), model: resp.model.clone(), usage: Usage { output: 0, ..resp.usage } }];
    for (i, p) in resp.content.iter().enumerate() {
        match p {
            Part::Text(t) => {
                evs.push(Event::BlockStart { index: i, kind: BlockKind::Text });
                evs.push(Event::TextDelta { index: i, text: t.clone() });
            }
            Part::Thinking { text, signature } => {
                evs.push(Event::BlockStart { index: i, kind: BlockKind::Thinking });
                evs.push(Event::ThinkingDelta { index: i, text: text.clone() });
                if let Some(s) = signature {
                    evs.push(Event::SignatureDelta { index: i, signature: s.clone() });
                }
            }
            Part::ToolUse { id, name, input } => {
                evs.push(Event::BlockStart { index: i, kind: BlockKind::ToolUse { id: id.clone(), name: name.clone() } });
                evs.push(Event::ToolArgsDelta { index: i, json: input.to_string() });
            }
            _ => continue,
        }
        evs.push(Event::BlockStop { index: i });
    }
    evs.push(Event::Finish { stop_reason: resp.stop_reason, usage: resp.usage });
    evs
}

// ------------------------------------------------------------------ 小工具

pub(crate) fn opt_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn u64_at(v: &Value, ptr: &str) -> u64 {
    v.pointer(ptr).and_then(Value::as_u64).unwrap_or(0)
}

/// 把工具参数的 JSON 字符串解析成对象；解析失败时包一层，避免丢数据
pub(crate) fn parse_args(s: &str) -> Value {
    if s.trim().is_empty() {
        return serde_json::json!({});
    }
    serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({ "_raw": s }))
}

/// JSON 字符串转义，但不带两边的引号（流式拼接 custom tool 的输入时用）
pub(crate) fn json_escape_fragment(s: &str) -> String {
    let q = serde_json::to_string(s).unwrap_or_default();
    q[1..q.len() - 1].to_string()
}

/// custom tool（自由文本输入）映射成函数时用的参数 schema
pub(crate) fn custom_tool_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": { "input": { "type": "string", "description": "工具的原始文本输入" } },
        "required": ["input"]
    })
}
