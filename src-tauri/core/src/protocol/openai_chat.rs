//! OpenAI Chat Completions（/v1/chat/completions），也覆盖 DeepSeek 等兼容服务。

use super::{opt_str, parse_args, u64_at, Compat, EncodeCtx, StreamDecoder, StreamEncoder};
use crate::ir::*;
use crate::sse::SseEvent;
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

// ------------------------------------------------------------------ 请求

fn parse_content(content: &Value) -> Vec<Part> {
    match content {
        Value::String(s) => vec![Part::Text(s.clone())],
        Value::Array(a) => a
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") | Some("input_text") => Some(Part::Text(opt_str(p, "text").unwrap_or_default())),
                Some("image_url") => {
                    let url = p
                        .pointer("/image_url/url")
                        .and_then(Value::as_str)
                        .or_else(|| p.get("image_url").and_then(Value::as_str))?;
                    Some(Part::Image(ImageSource::from_url(url)))
                }
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

pub fn parse_request(body: &Value) -> Result<Request> {
    let model = opt_str(body, "model").ok_or_else(|| anyhow!("请求缺少 model 字段"))?;
    let mut system: Vec<String> = Vec::new();
    let mut messages: Vec<Message> = Vec::new();
    for m in body.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let content = m.get("content").cloned().unwrap_or(Value::Null);
        match m.get("role").and_then(Value::as_str).unwrap_or("user") {
            "system" | "developer" => system.push(parts_text(&parse_content(&content))),
            "assistant" => {
                let mut parts = Vec::new();
                let reasoning = opt_str(m, "reasoning_content").or_else(|| opt_str(m, "reasoning"));
                if let Some(r) = reasoning.filter(|r| !r.is_empty()) {
                    parts.push(Part::Thinking { text: r, signature: None });
                }
                parts.extend(parse_content(&content));
                for tc in m.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
                    parts.push(Part::ToolUse {
                        id: opt_str(tc, "id").unwrap_or_else(|| new_id("call_")),
                        name: tc.pointer("/function/name").and_then(Value::as_str).unwrap_or("").to_string(),
                        input: parse_args(tc.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("")),
                    });
                }
                messages.push(Message { role: Role::Assistant, content: parts });
            }
            "tool" | "function" => {
                let part = Part::ToolResult {
                    tool_use_id: opt_str(m, "tool_call_id").or_else(|| opt_str(m, "name")).unwrap_or_default(),
                    content: parse_content(&content),
                    is_error: false,
                };
                messages.push(Message { role: Role::User, content: vec![part] });
            }
            _ => messages.push(Message { role: Role::User, content: parse_content(&content) }),
        }
    }
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let f = t.get("function")?;
            Some(Tool {
                name: opt_str(f, "name")?,
                description: opt_str(f, "description"),
                schema: f.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
                kind: ToolKind::Function,
            })
        })
        .collect();
    let tool_choice = body.get("tool_choice").and_then(|tc| match tc {
        Value::String(s) => match s.as_str() {
            "auto" => Some(ToolChoice::Auto),
            "none" => Some(ToolChoice::None),
            "required" => Some(ToolChoice::Any),
            _ => None,
        },
        Value::Object(_) => tc.pointer("/function/name").and_then(Value::as_str).map(|n| ToolChoice::Tool(n.into())),
        _ => None,
    });
    let stop = match body.get("stop") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect(),
        _ => vec![],
    };
    let reasoning = opt_str(body, "reasoning_effort").map(|e| Reasoning { budget_tokens: None, effort: Some(e) });
    Ok(Request {
        model,
        system: (!system.is_empty()).then(|| system.join("\n\n")),
        messages,
        tools,
        tool_choice,
        max_tokens: body
            .get("max_completion_tokens")
            .or_else(|| body.get("max_tokens"))
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        stop,
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        reasoning,
    })
}

fn content_parts(parts: &[Part]) -> Value {
    let has_image = parts.iter().any(|p| matches!(p, Part::Image(_)));
    if !has_image {
        let text: Vec<&str> = parts.iter().filter_map(|p| if let Part::Text(t) = p { Some(t.as_str()) } else { None }).collect();
        return json!(text.join(""));
    }
    Value::Array(
        parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(json!({"type":"text","text":t})),
                Part::Image(img) => Some(json!({"type":"image_url","image_url":{"url":img.to_url()}})),
                _ => None,
            })
            .collect(),
    )
}

pub fn build_request(req: &Request, model: &str, compat: &Compat) -> Value {
    let mut msgs = Vec::new();
    if let Some(s) = req.system.as_ref().filter(|s| !s.is_empty()) {
        msgs.push(json!({"role":"system","content":s}));
    }
    for m in &req.messages {
        match m.role {
            Role::Assistant => {
                let mut o = json!({"role":"assistant"});
                let text: String = m.content.iter().filter_map(|p| if let Part::Text(t) = p { Some(t.as_str()) } else { None }).collect();
                o["content"] = if text.is_empty() { Value::Null } else { json!(text) };
                let calls: Vec<Value> = m
                    .content
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolUse { id, name, input } => Some(json!({
                            "id": id, "type": "function",
                            "function": { "name": name, "arguments": input.to_string() }
                        })),
                        _ => None,
                    })
                    .collect();
                if !calls.is_empty() {
                    o["tool_calls"] = Value::Array(calls);
                }
                if compat.send_reasoning_content {
                    let r: String = m
                        .content
                        .iter()
                        .filter_map(|p| if let Part::Thinking { text, .. } = p { Some(text.as_str()) } else { None })
                        .collect();
                    if !r.is_empty() {
                        o["reasoning_content"] = json!(r);
                    }
                }
                if o["content"].is_null() && o.get("tool_calls").is_none() {
                    continue;
                }
                msgs.push(o);
            }
            Role::User => {
                // 工具结果要拆成独立的 tool 消息，而且必须紧跟在 assistant 后面
                let mut rest = Vec::new();
                let mut tool_images = Vec::new();
                for p in &m.content {
                    match p {
                        Part::ToolResult { tool_use_id, content, is_error } => {
                            let mut text = parts_text(content);
                            if *is_error && !text.starts_with("Error") {
                                text = format!("Error: {text}");
                            }
                            msgs.push(json!({"role":"tool","tool_call_id":tool_use_id,"content":text}));
                            tool_images.extend(content.iter().filter(|c| matches!(c, Part::Image(_))).cloned());
                        }
                        other => rest.push(other.clone()),
                    }
                }
                // tool 消息不支持图片，把截图之类的放进随后的 user 消息
                let mut user_parts = tool_images;
                user_parts.extend(rest);
                if !user_parts.is_empty() {
                    msgs.push(json!({"role":"user","content":content_parts(&user_parts)}));
                }
            }
        }
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), Value::Array(msgs));
    if let Some(mt) = req.max_tokens {
        let key = if compat.use_max_completion_tokens { "max_completion_tokens" } else { "max_tokens" };
        body.insert(key.into(), json!(mt));
    }
    if !compat.drop_sampling_params {
        if let Some(t) = req.temperature {
            body.insert("temperature".into(), json!(t));
        }
        if let Some(t) = req.top_p {
            body.insert("top_p".into(), json!(t));
        }
    }
    if !req.stop.is_empty() {
        body.insert("stop".into(), json!(req.stop));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                let mut f = json!({"name":t.name,"parameters":t.schema});
                if let Some(d) = &t.description {
                    f["description"] = json!(d);
                }
                json!({"type":"function","function":f})
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        if let Some(tc) = &req.tool_choice {
            body.insert(
                "tool_choice".into(),
                match tc {
                    ToolChoice::Auto => json!("auto"),
                    ToolChoice::None => json!("none"),
                    ToolChoice::Any => json!("required"),
                    ToolChoice::Tool(n) => json!({"type":"function","function":{"name":n}}),
                },
            );
        }
    }
    if compat.send_reasoning_effort {
        if let Some(e) = req.reasoning.as_ref().and_then(Reasoning::effort_or_from_budget) {
            body.insert("reasoning_effort".into(), json!(e));
        }
    }
    if req.stream {
        body.insert("stream".into(), json!(true));
        body.insert("stream_options".into(), json!({"include_usage": true}));
    }
    Value::Object(body)
}

// ------------------------------------------------------------------ 响应

fn parse_stop(s: Option<&str>) -> StopReason {
    match s {
        Some("length") => StopReason::MaxTokens,
        Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
        Some("content_filter") => StopReason::ContentFilter,
        _ => StopReason::EndTurn,
    }
}

fn stop_str(s: StopReason) -> &'static str {
    match s {
        StopReason::MaxTokens => "length",
        StopReason::ToolUse => "tool_calls",
        StopReason::ContentFilter => "content_filter",
        _ => "stop",
    }
}

pub fn parse_usage(u: &Value) -> Usage {
    let prompt = u64_at(u, "/prompt_tokens");
    // OpenAI 写在 prompt_tokens_details.cached_tokens，DeepSeek 写在 prompt_cache_hit_tokens
    let cached = u64_at(u, "/prompt_tokens_details/cached_tokens").max(u64_at(u, "/prompt_cache_hit_tokens"));
    Usage { input: prompt.saturating_sub(cached), output: u64_at(u, "/completion_tokens"), cache_read: cached, cache_write: 0 }
}

fn usage_json(u: &Usage) -> Value {
    json!({
        "prompt_tokens": u.total_input(),
        "completion_tokens": u.output,
        "total_tokens": u.total_input() + u.output,
        "prompt_tokens_details": { "cached_tokens": u.cache_read },
    })
}

pub fn parse_response(body: &Value) -> Result<Response> {
    if body.get("error").is_some() && body.get("choices").is_none() {
        return Err(anyhow!(super::error_message(body.to_string().as_bytes())));
    }
    let choice = body.pointer("/choices/0").cloned().unwrap_or_default();
    let msg = choice.get("message").cloned().unwrap_or_default();
    let mut content = Vec::new();
    if let Some(r) = opt_str(&msg, "reasoning_content").or_else(|| opt_str(&msg, "reasoning")).filter(|r| !r.is_empty()) {
        content.push(Part::Thinking { text: r, signature: None });
    }
    if let Some(t) = opt_str(&msg, "content").filter(|t| !t.is_empty()) {
        content.push(Part::Text(t));
    }
    for tc in msg.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
        content.push(Part::ToolUse {
            id: opt_str(tc, "id").unwrap_or_else(|| new_id("call_")),
            name: tc.pointer("/function/name").and_then(Value::as_str).unwrap_or("").to_string(),
            input: parse_args(tc.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("")),
        });
    }
    let mut stop_reason = parse_stop(choice.get("finish_reason").and_then(Value::as_str));
    if content.iter().any(|p| matches!(p, Part::ToolUse { .. })) {
        stop_reason = StopReason::ToolUse;
    }
    Ok(Response {
        id: opt_str(body, "id").unwrap_or_else(|| new_id("chatcmpl-")),
        model: opt_str(body, "model").unwrap_or_default(),
        content,
        stop_reason,
        usage: body.get("usage").map(parse_usage).unwrap_or_default(),
    })
}

pub fn build_response(resp: &Response, ctx: &EncodeCtx) -> Value {
    let text: String = resp.content.iter().filter_map(|p| if let Part::Text(t) = p { Some(t.as_str()) } else { None }).collect();
    let reasoning: String =
        resp.content.iter().filter_map(|p| if let Part::Thinking { text, .. } = p { Some(text.as_str()) } else { None }).collect();
    let calls: Vec<Value> = resp
        .content
        .iter()
        .filter_map(|p| match p {
            Part::ToolUse { id, name, input } => Some(json!({
                "id": id, "type": "function", "function": { "name": name, "arguments": input.to_string() }
            })),
            _ => None,
        })
        .collect();
    let mut msg = json!({"role":"assistant","content": if text.is_empty() && !calls.is_empty() { Value::Null } else { json!(text) }});
    if !reasoning.is_empty() {
        msg["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        msg["tool_calls"] = Value::Array(calls);
    }
    json!({
        "id": new_id("chatcmpl-"),
        "object": "chat.completion",
        "created": now_secs(),
        "model": ctx.model,
        "choices": [{ "index": 0, "message": msg, "finish_reason": stop_str(resp.stop_reason) }],
        "usage": usage_json(&resp.usage),
    })
}

// ------------------------------------------------------------------ 流式解码

#[derive(Default)]
pub struct Decoder {
    started: bool,
    /// 当前打开的文本 / 思考块的序号
    text_block: Option<usize>,
    think_block: Option<usize>,
    /// OpenAI 的 tool_calls[].index → IR 块序号
    tools: HashMap<u64, usize>,
    next: usize,
    stop: Option<StopReason>,
    usage: Usage,
    done: bool,
}

impl Decoder {
    fn close_text(&mut self, out: &mut Vec<Event>) {
        if let Some(i) = self.text_block.take() {
            out.push(Event::BlockStop { index: i });
        }
    }
    fn close_think(&mut self, out: &mut Vec<Event>) {
        if let Some(i) = self.think_block.take() {
            out.push(Event::BlockStop { index: i });
        }
    }
    fn alloc(&mut self) -> usize {
        let i = self.next;
        self.next += 1;
        i
    }
}

impl StreamDecoder for Decoder {
    fn feed(&mut self, ev: &SseEvent) -> Vec<Event> {
        let mut out = Vec::new();
        if ev.data.trim() == "[DONE]" {
            return self.finish();
        }
        let Some(v) = ev.json() else { return out };
        if v.get("error").is_some() && v.get("choices").is_none() {
            out.push(Event::Error { message: super::error_message(ev.data.as_bytes()) });
            return out;
        }
        if !self.started {
            self.started = true;
            out.push(Event::Start {
                id: opt_str(&v, "id").unwrap_or_else(|| new_id("chatcmpl-")),
                model: opt_str(&v, "model").unwrap_or_default(),
                usage: Usage::default(),
            });
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage.merge(&parse_usage(u));
        }
        let Some(choice) = v.pointer("/choices/0") else { return out };
        let delta = choice.get("delta").cloned().unwrap_or_default();
        let reasoning = opt_str(&delta, "reasoning_content").or_else(|| opt_str(&delta, "reasoning"));
        if let Some(r) = reasoning.filter(|r| !r.is_empty()) {
            self.close_text(&mut out);
            let i = match self.think_block {
                Some(i) => i,
                None => {
                    let i = self.alloc();
                    self.think_block = Some(i);
                    out.push(Event::BlockStart { index: i, kind: BlockKind::Thinking });
                    i
                }
            };
            out.push(Event::ThinkingDelta { index: i, text: r });
        }
        if let Some(t) = opt_str(&delta, "content").filter(|t| !t.is_empty()) {
            self.close_think(&mut out);
            let i = match self.text_block {
                Some(i) => i,
                None => {
                    let i = self.alloc();
                    self.text_block = Some(i);
                    out.push(Event::BlockStart { index: i, kind: BlockKind::Text });
                    i
                }
            };
            out.push(Event::TextDelta { index: i, text: t });
        }
        for tc in delta.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
            self.close_text(&mut out);
            self.close_think(&mut out);
            let k = tc.get("index").and_then(Value::as_u64).unwrap_or(self.tools.len() as u64);
            let i = match self.tools.get(&k) {
                Some(i) => *i,
                None => {
                    // 新的工具调用开始：先关掉上一个
                    if let Some(prev) = self.tools.values().max().copied() {
                        out.push(Event::BlockStop { index: prev });
                    }
                    let i = self.alloc();
                    self.tools.insert(k, i);
                    out.push(Event::BlockStart {
                        index: i,
                        kind: BlockKind::ToolUse {
                            id: opt_str(tc, "id").unwrap_or_else(|| new_id("call_")),
                            name: tc.pointer("/function/name").and_then(Value::as_str).unwrap_or("").to_string(),
                        },
                    });
                    i
                }
            };
            if let Some(a) = tc.pointer("/function/arguments").and_then(Value::as_str).filter(|a| !a.is_empty()) {
                out.push(Event::ToolArgsDelta { index: i, json: a.to_string() });
            }
        }
        if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop = Some(parse_stop(Some(fr)));
        }
        out
    }

    fn finish(&mut self) -> Vec<Event> {
        if self.done {
            return vec![];
        }
        self.done = true;
        let mut out = Vec::new();
        self.close_text(&mut out);
        self.close_think(&mut out);
        let mut stop = self.stop.unwrap_or_default();
        if !self.tools.is_empty() && stop == StopReason::EndTurn {
            stop = StopReason::ToolUse;
        }
        out.push(Event::Finish { stop_reason: stop, usage: self.usage });
        out
    }
}

// ------------------------------------------------------------------ 流式编码

pub struct Encoder {
    ctx: EncodeCtx,
    id: String,
    created: i64,
    /// IR 块序号 → tool_calls 的 index
    tools: HashMap<usize, usize>,
}

impl Encoder {
    pub fn new(ctx: EncodeCtx) -> Self {
        Encoder { ctx, id: new_id("chatcmpl-"), created: now_secs(), tools: HashMap::new() }
    }

    fn chunk(&self, delta: Value, finish: Option<&str>) -> SseEvent {
        SseEvent::data(
            json!({
                "id": self.id, "object": "chat.completion.chunk", "created": self.created, "model": self.ctx.model,
                "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
            })
            .to_string(),
        )
    }
}

impl StreamEncoder for Encoder {
    fn encode(&mut self, ev: &Event) -> Vec<SseEvent> {
        match ev {
            Event::Start { .. } => vec![self.chunk(json!({"role":"assistant","content":""}), None)],
            Event::BlockStart { index, kind: BlockKind::ToolUse { id, name } } => {
                let k = self.tools.len();
                self.tools.insert(*index, k);
                vec![self.chunk(
                    json!({"tool_calls":[{"index":k,"id":id,"type":"function","function":{"name":name,"arguments":""}}]}),
                    None,
                )]
            }
            Event::TextDelta { text, .. } => vec![self.chunk(json!({"content":text}), None)],
            Event::ThinkingDelta { text, .. } => vec![self.chunk(json!({"reasoning_content":text}), None)],
            Event::ToolArgsDelta { index, json: j } => match self.tools.get(index) {
                Some(k) => vec![self.chunk(json!({"tool_calls":[{"index":k,"function":{"arguments":j}}]}), None)],
                None => vec![],
            },
            Event::Finish { stop_reason, usage } => {
                let usage_chunk = json!({
                    "id": self.id, "object": "chat.completion.chunk", "created": self.created, "model": self.ctx.model,
                    "choices": [], "usage": usage_json(usage),
                });
                vec![
                    self.chunk(json!({}), Some(stop_str(*stop_reason))),
                    SseEvent::data(usage_chunk.to_string()),
                    SseEvent::data("[DONE]"),
                ]
            }
            Event::Error { message } => vec![
                SseEvent::data(json!({"error":{"message":message,"type":"upstream_error"}}).to_string()),
                SseEvent::data("[DONE]"),
            ],
            _ => vec![],
        }
    }
}
