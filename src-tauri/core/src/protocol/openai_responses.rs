//! OpenAI Responses API（/v1/responses），Codex CLI 使用这个协议。

use super::{custom_tool_schema, json_escape_fragment, opt_str, parse_args, u64_at, Compat, EncodeCtx, StreamDecoder, StreamEncoder};
use crate::ir::*;
use crate::sse::SseEvent;
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

// ------------------------------------------------------------------ 请求

fn parse_content(content: &Value) -> Vec<Part> {
    match content {
        Value::String(s) => vec![Part::Text(s.clone())],
        Value::Array(a) => a
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("input_text") | Some("output_text") | Some("text") => Some(Part::Text(opt_str(p, "text").unwrap_or_default())),
                Some("input_image") => {
                    let url = p
                        .get("image_url")
                        .and_then(|u| u.as_str().map(str::to_string).or_else(|| opt_str(u, "url")))?;
                    Some(Part::Image(ImageSource::from_url(&url)))
                }
                Some("refusal") => Some(Part::Text(opt_str(p, "refusal").unwrap_or_default())),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

fn push(messages: &mut Vec<Message>, role: Role, part: Part) {
    match messages.last_mut() {
        Some(m) if m.role == role => m.content.push(part),
        _ => messages.push(Message { role, content: vec![part] }),
    }
}

pub fn parse_request(body: &Value) -> Result<Request> {
    let model = opt_str(body, "model").ok_or_else(|| anyhow!("请求缺少 model 字段"))?;
    let mut system: Vec<String> = Vec::new();
    if let Some(i) = opt_str(body, "instructions").filter(|s| !s.is_empty()) {
        system.push(i);
    }
    let mut messages: Vec<Message> = Vec::new();
    match body.get("input") {
        Some(Value::String(s)) => messages.push(Message { role: Role::User, content: vec![Part::Text(s.clone())] }),
        Some(Value::Array(items)) => {
            for it in items {
                let ty = it.get("type").and_then(Value::as_str).unwrap_or("message");
                match ty {
                    "message" => {
                        let parts = parse_content(it.get("content").unwrap_or(&Value::Null));
                        match it.get("role").and_then(Value::as_str).unwrap_or("user") {
                            "system" | "developer" => system.push(parts_text(&parts)),
                            "assistant" => parts.into_iter().for_each(|p| push(&mut messages, Role::Assistant, p)),
                            _ => parts.into_iter().for_each(|p| push(&mut messages, Role::User, p)),
                        }
                    }
                    "function_call" => push(
                        &mut messages,
                        Role::Assistant,
                        Part::ToolUse {
                            id: opt_str(it, "call_id").unwrap_or_default(),
                            name: opt_str(it, "name").unwrap_or_default(),
                            input: parse_args(it.get("arguments").and_then(Value::as_str).unwrap_or("")),
                        },
                    ),
                    "custom_tool_call" => push(
                        &mut messages,
                        Role::Assistant,
                        Part::ToolUse {
                            id: opt_str(it, "call_id").unwrap_or_default(),
                            name: opt_str(it, "name").unwrap_or_default(),
                            input: json!({ "input": opt_str(it, "input").unwrap_or_default() }),
                        },
                    ),
                    "function_call_output" | "custom_tool_call_output" => {
                        let content = match it.get("output") {
                            Some(Value::String(s)) => vec![Part::Text(s.clone())],
                            Some(v @ Value::Array(_)) => parse_content(v),
                            Some(v) => vec![Part::Text(v.to_string())],
                            None => vec![],
                        };
                        push(
                            &mut messages,
                            Role::User,
                            Part::ToolResult { tool_use_id: opt_str(it, "call_id").unwrap_or_default(), content, is_error: false },
                        );
                    }
                    "reasoning" => {
                        let text: Vec<String> = it
                            .get("summary")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|s| opt_str(s, "text"))
                            .collect();
                        if !text.is_empty() {
                            push(&mut messages, Role::Assistant, Part::Thinking { text: text.join("\n\n"), signature: None });
                        }
                    }
                    _ => {} // local_shell_call、web_search_call 等内置工具，无法跨协议
                }
            }
        }
        _ => {}
    }
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| match t.get("type").and_then(Value::as_str) {
            Some("function") => Some(Tool {
                name: opt_str(t, "name")?,
                description: opt_str(t, "description"),
                schema: t.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
                kind: ToolKind::Function,
            }),
            Some("custom") => Some(Tool {
                name: opt_str(t, "name")?,
                description: opt_str(t, "description"),
                schema: custom_tool_schema(),
                kind: ToolKind::Custom,
            }),
            _ => None,
        })
        .collect();
    let tool_choice = body.get("tool_choice").and_then(|tc| match tc {
        Value::String(s) => match s.as_str() {
            "auto" => Some(ToolChoice::Auto),
            "none" => Some(ToolChoice::None),
            "required" => Some(ToolChoice::Any),
            _ => None,
        },
        Value::Object(_) => opt_str(tc, "name").map(ToolChoice::Tool),
        _ => None,
    });
    let reasoning = body
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .map(|e| Reasoning { budget_tokens: None, effort: Some(e.to_string()) });
    Ok(Request {
        model,
        system: (!system.is_empty()).then(|| system.join("\n\n")),
        messages,
        tools,
        tool_choice,
        max_tokens: body.get("max_output_tokens").and_then(Value::as_u64).map(|v| v as u32),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        stop: vec![],
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        reasoning,
    })
}

pub fn build_request(req: &Request, model: &str, compat: &Compat) -> Value {
    let mut input = Vec::new();
    for m in &req.messages {
        let mut buf: Vec<Value> = Vec::new();
        let role = if m.role == Role::User { "user" } else { "assistant" };
        let flush = |buf: &mut Vec<Value>, input: &mut Vec<Value>| {
            if !buf.is_empty() {
                input.push(json!({"type":"message","role":role,"content":std::mem::take(buf)}));
            }
        };
        for p in &m.content {
            match p {
                Part::Text(t) => buf.push(if m.role == Role::User {
                    json!({"type":"input_text","text":t})
                } else {
                    json!({"type":"output_text","text":t})
                }),
                Part::Image(img) => {
                    if m.role == Role::User {
                        buf.push(json!({"type":"input_image","image_url":img.to_url()}));
                    }
                }
                Part::ToolUse { id, name, input: args } => {
                    flush(&mut buf, &mut input);
                    if req.is_custom_tool(name) {
                        let raw = args.get("input").and_then(Value::as_str).unwrap_or("").to_string();
                        input.push(json!({"type":"custom_tool_call","call_id":id,"name":name,"input":raw}));
                    } else {
                        input.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":args.to_string()}));
                    }
                }
                Part::ToolResult { tool_use_id, content, .. } => {
                    flush(&mut buf, &mut input);
                    let custom = req.tool_name_for_id(tool_use_id).is_some_and(|n| req.is_custom_tool(&n));
                    let ty = if custom { "custom_tool_call_output" } else { "function_call_output" };
                    input.push(json!({"type":ty,"call_id":tool_use_id,"output":parts_text(content)}));
                    let imgs: Vec<Value> = content
                        .iter()
                        .filter_map(|c| if let Part::Image(i) = c { Some(json!({"type":"input_image","image_url":i.to_url()})) } else { None })
                        .collect();
                    if !imgs.is_empty() {
                        input.push(json!({"type":"message","role":"user","content":imgs}));
                    }
                }
                Part::Thinking { .. } => {} // 没有加密内容的推理项上游不接受
            }
        }
        flush(&mut buf, &mut input);
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    if let Some(s) = req.system.as_ref().filter(|s| !s.is_empty()) {
        body.insert("instructions".into(), json!(s));
    }
    body.insert("input".into(), Value::Array(input));
    body.insert("store".into(), json!(false));
    if let Some(mt) = req.max_tokens {
        body.insert("max_output_tokens".into(), json!(mt));
    }
    if !compat.drop_sampling_params && req.reasoning.is_none() {
        if let Some(t) = req.temperature {
            body.insert("temperature".into(), json!(t));
        }
        if let Some(t) = req.top_p {
            body.insert("top_p".into(), json!(t));
        }
    }
    if let Some(e) = req.reasoning.as_ref().and_then(Reasoning::effort_or_from_budget) {
        body.insert("reasoning".into(), json!({"effort": e, "summary": "auto"}));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| match t.kind {
                ToolKind::Custom => {
                    let mut o = json!({"type":"custom","name":t.name});
                    if let Some(d) = &t.description {
                        o["description"] = json!(d);
                    }
                    o
                }
                ToolKind::Function => {
                    let mut o = json!({"type":"function","name":t.name,"parameters":t.schema});
                    if let Some(d) = &t.description {
                        o["description"] = json!(d);
                    }
                    o
                }
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
                    ToolChoice::Tool(n) => json!({"type":"function","name":n}),
                },
            );
        }
    }
    if req.stream {
        body.insert("stream".into(), json!(true));
    }
    Value::Object(body)
}

// ------------------------------------------------------------------ 响应

pub fn parse_usage(u: &Value) -> Usage {
    let input = u64_at(u, "/input_tokens");
    let cached = u64_at(u, "/input_tokens_details/cached_tokens");
    Usage { input: input.saturating_sub(cached), output: u64_at(u, "/output_tokens"), cache_read: cached, cache_write: 0 }
}

fn usage_json(u: &Usage) -> Value {
    json!({
        "input_tokens": u.total_input(),
        "input_tokens_details": { "cached_tokens": u.cache_read },
        "output_tokens": u.output,
        "output_tokens_details": { "reasoning_tokens": 0 },
        "total_tokens": u.total_input() + u.output,
    })
}

pub fn parse_response(body: &Value) -> Result<Response> {
    if body.get("error").is_some_and(|e| !e.is_null()) && body.get("output").is_none() {
        return Err(anyhow!(super::error_message(body.to_string().as_bytes())));
    }
    let mut content = Vec::new();
    for it in body.get("output").and_then(Value::as_array).into_iter().flatten() {
        match it.get("type").and_then(Value::as_str) {
            Some("message") => {
                let t = parts_text(&parse_content(it.get("content").unwrap_or(&Value::Null)));
                if !t.is_empty() {
                    content.push(Part::Text(t));
                }
            }
            Some("function_call") => content.push(Part::ToolUse {
                id: opt_str(it, "call_id").unwrap_or_default(),
                name: opt_str(it, "name").unwrap_or_default(),
                input: parse_args(it.get("arguments").and_then(Value::as_str).unwrap_or("")),
            }),
            Some("custom_tool_call") => content.push(Part::ToolUse {
                id: opt_str(it, "call_id").unwrap_or_default(),
                name: opt_str(it, "name").unwrap_or_default(),
                input: json!({ "input": opt_str(it, "input").unwrap_or_default() }),
            }),
            Some("reasoning") => {
                let text: Vec<String> =
                    it.get("summary").and_then(Value::as_array).into_iter().flatten().filter_map(|s| opt_str(s, "text")).collect();
                if !text.is_empty() {
                    content.push(Part::Thinking { text: text.join("\n\n"), signature: None });
                }
            }
            _ => {}
        }
    }
    let stop_reason = if content.iter().any(|p| matches!(p, Part::ToolUse { .. })) {
        StopReason::ToolUse
    } else if body.get("status").and_then(Value::as_str) == Some("incomplete") {
        StopReason::MaxTokens
    } else {
        StopReason::EndTurn
    };
    Ok(Response {
        id: opt_str(body, "id").unwrap_or_else(|| new_id("resp_")),
        model: opt_str(body, "model").unwrap_or_default(),
        content,
        stop_reason,
        usage: body.get("usage").map(parse_usage).unwrap_or_default(),
    })
}

fn output_items(resp: &Response, ctx: &EncodeCtx) -> Vec<Value> {
    let mut out = Vec::new();
    for p in &resp.content {
        match p {
            Part::Text(t) => out.push(json!({
                "type":"message","id":new_id("msg_"),"status":"completed","role":"assistant",
                "content":[{"type":"output_text","text":t,"annotations":[]}]
            })),
            Part::Thinking { text, .. } => out.push(json!({
                "type":"reasoning","id":new_id("rs_"),"summary":[{"type":"summary_text","text":text}]
            })),
            Part::ToolUse { id, name, input } => out.push(tool_item(ctx, id, name, input, "completed")),
            _ => {}
        }
    }
    out
}

fn tool_item(ctx: &EncodeCtx, call_id: &str, name: &str, input: &Value, status: &str) -> Value {
    if ctx.custom_tools.contains(name) {
        let raw = input.get("input").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| input.to_string());
        json!({"type":"custom_tool_call","id":new_id("ctc_"),"status":status,"call_id":call_id,"name":name,"input":raw})
    } else {
        json!({"type":"function_call","id":new_id("fc_"),"status":status,"call_id":call_id,"name":name,"arguments":input.to_string()})
    }
}

fn response_obj(id: &str, ctx: &EncodeCtx, status: &str, output: Vec<Value>, usage: Option<&Usage>) -> Value {
    let mut o = json!({
        "id": id, "object": "response", "created_at": now_secs(), "status": status,
        "model": ctx.model, "output": output, "parallel_tool_calls": true, "tool_choice": "auto", "tools": [],
    });
    if let Some(u) = usage {
        o["usage"] = usage_json(u);
    }
    if status == "incomplete" {
        o["incomplete_details"] = json!({"reason":"max_output_tokens"});
    }
    o
}

pub fn build_response(resp: &Response, ctx: &EncodeCtx) -> Value {
    let status = if resp.stop_reason == StopReason::MaxTokens { "incomplete" } else { "completed" };
    response_obj(&new_id("resp_"), ctx, status, output_items(resp, ctx), Some(&resp.usage))
}

// ------------------------------------------------------------------ 流式解码

#[derive(Default)]
pub struct Decoder {
    custom: HashSet<usize>,
    /// 每个输出项是否已经收到过增量（没收到的话，在 done 时补发完整内容）
    got_delta: HashSet<usize>,
    kinds: HashMap<usize, &'static str>,
    has_tool: bool,
    finished: bool,
}

impl StreamDecoder for Decoder {
    fn feed(&mut self, ev: &SseEvent) -> Vec<Event> {
        let Some(v) = ev.json() else { return vec![] };
        let ty = v.get("type").and_then(Value::as_str).or(ev.event.as_deref()).unwrap_or("");
        let idx = v.get("output_index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let delta = || opt_str(&v, "delta").unwrap_or_default();
        match ty {
            "response.created" => {
                let r = v.get("response").cloned().unwrap_or_default();
                vec![Event::Start {
                    id: opt_str(&r, "id").unwrap_or_else(|| new_id("resp_")),
                    model: opt_str(&r, "model").unwrap_or_default(),
                    usage: Usage::default(),
                }]
            }
            "response.output_item.added" => {
                let it = v.get("item").cloned().unwrap_or_default();
                match it.get("type").and_then(Value::as_str) {
                    Some("message") => {
                        self.kinds.insert(idx, "message");
                        vec![Event::BlockStart { index: idx, kind: BlockKind::Text }]
                    }
                    Some("reasoning") => {
                        self.kinds.insert(idx, "reasoning");
                        vec![Event::BlockStart { index: idx, kind: BlockKind::Thinking }]
                    }
                    Some("function_call") => {
                        self.kinds.insert(idx, "function_call");
                        self.has_tool = true;
                        vec![Event::BlockStart {
                            index: idx,
                            kind: BlockKind::ToolUse {
                                id: opt_str(&it, "call_id").unwrap_or_default(),
                                name: opt_str(&it, "name").unwrap_or_default(),
                            },
                        }]
                    }
                    Some("custom_tool_call") => {
                        self.kinds.insert(idx, "custom_tool_call");
                        self.custom.insert(idx);
                        self.has_tool = true;
                        vec![
                            Event::BlockStart {
                                index: idx,
                                kind: BlockKind::ToolUse {
                                    id: opt_str(&it, "call_id").unwrap_or_default(),
                                    name: opt_str(&it, "name").unwrap_or_default(),
                                },
                            },
                            Event::ToolArgsDelta { index: idx, json: "{\"input\":\"".into() },
                        ]
                    }
                    _ => vec![],
                }
            }
            "response.output_text.delta" => {
                self.got_delta.insert(idx);
                vec![Event::TextDelta { index: idx, text: delta() }]
            }
            "response.reasoning_summary_part.added" => {
                // 多段摘要之间空一行
                if v.get("summary_index").and_then(Value::as_u64).unwrap_or(0) > 0 {
                    vec![Event::ThinkingDelta { index: idx, text: "\n\n".into() }]
                } else {
                    vec![]
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                self.got_delta.insert(idx);
                vec![Event::ThinkingDelta { index: idx, text: delta() }]
            }
            "response.function_call_arguments.delta" => {
                self.got_delta.insert(idx);
                vec![Event::ToolArgsDelta { index: idx, json: delta() }]
            }
            "response.custom_tool_call_input.delta" => {
                self.got_delta.insert(idx);
                vec![Event::ToolArgsDelta { index: idx, json: json_escape_fragment(&delta()) }]
            }
            "response.output_item.done" => {
                let it = v.get("item").cloned().unwrap_or_default();
                let mut out = Vec::new();
                let Some(kind) = self.kinds.get(&idx).copied() else { return out };
                if !self.got_delta.contains(&idx) {
                    match kind {
                        "message" => {
                            let t = parts_text(&parse_content(it.get("content").unwrap_or(&Value::Null)));
                            if !t.is_empty() {
                                out.push(Event::TextDelta { index: idx, text: t });
                            }
                        }
                        "function_call" => {
                            if let Some(a) = opt_str(&it, "arguments") {
                                out.push(Event::ToolArgsDelta { index: idx, json: a });
                            }
                        }
                        "custom_tool_call" => {
                            if let Some(a) = opt_str(&it, "input") {
                                out.push(Event::ToolArgsDelta { index: idx, json: json_escape_fragment(&a) });
                            }
                        }
                        _ => {}
                    }
                }
                if self.custom.contains(&idx) {
                    out.push(Event::ToolArgsDelta { index: idx, json: "\"}".into() });
                }
                out.push(Event::BlockStop { index: idx });
                out
            }
            "response.completed" | "response.incomplete" | "response.done" => {
                self.finished = true;
                let r = v.get("response").cloned().unwrap_or_default();
                let usage = r.get("usage").map(parse_usage).unwrap_or_default();
                let stop = if self.has_tool {
                    StopReason::ToolUse
                } else if ty == "response.incomplete" || r.get("status").and_then(Value::as_str) == Some("incomplete") {
                    StopReason::MaxTokens
                } else {
                    StopReason::EndTurn
                };
                vec![Event::Finish { stop_reason: stop, usage }]
            }
            "response.failed" | "error" => {
                self.finished = true;
                let msg = v
                    .pointer("/response/error/message")
                    .or_else(|| v.pointer("/error/message"))
                    .or_else(|| v.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| ev.data.clone());
                vec![Event::Error { message: msg }]
            }
            _ => vec![],
        }
    }
}

// ------------------------------------------------------------------ 流式编码

struct ItemState {
    output_index: usize,
    item_id: String,
    kind: BlockKind,
    buf: String,
}

pub struct Encoder {
    ctx: EncodeCtx,
    id: String,
    seq: u64,
    items: HashMap<usize, ItemState>,
    next_output: usize,
    done_items: Vec<(usize, Value)>,
}

impl Encoder {
    pub fn new(ctx: EncodeCtx) -> Self {
        Encoder { ctx, id: new_id("resp_"), seq: 0, items: HashMap::new(), next_output: 0, done_items: Vec::new() }
    }

    fn ev(&mut self, ty: &str, mut data: Value) -> SseEvent {
        data["type"] = json!(ty);
        data["sequence_number"] = json!(self.seq);
        self.seq += 1;
        SseEvent::named(ty, data.to_string())
    }

    fn is_custom(&self, kind: &BlockKind) -> bool {
        matches!(kind, BlockKind::ToolUse { name, .. } if self.ctx.custom_tools.contains(name))
    }
}

impl StreamEncoder for Encoder {
    fn encode(&mut self, ev: &Event) -> Vec<SseEvent> {
        let mut out = Vec::new();
        match ev {
            Event::Start { .. } => {
                let r = response_obj(&self.id, &self.ctx, "in_progress", vec![], None);
                out.push(self.ev("response.created", json!({ "response": r.clone() })));
                out.push(self.ev("response.in_progress", json!({ "response": r })));
            }
            Event::BlockStart { index, kind } => {
                let oi = self.next_output;
                self.next_output += 1;
                let (item_id, item) = match kind {
                    BlockKind::Text => {
                        let id = new_id("msg_");
                        (id.clone(), json!({"type":"message","id":id,"status":"in_progress","role":"assistant","content":[]}))
                    }
                    BlockKind::Thinking => {
                        let id = new_id("rs_");
                        (id.clone(), json!({"type":"reasoning","id":id,"summary":[]}))
                    }
                    BlockKind::ToolUse { id: call_id, name } => {
                        if self.is_custom(kind) {
                            let id = new_id("ctc_");
                            (id.clone(), json!({"type":"custom_tool_call","id":id,"status":"in_progress","call_id":call_id,"name":name,"input":""}))
                        } else {
                            let id = new_id("fc_");
                            (id.clone(), json!({"type":"function_call","id":id,"status":"in_progress","call_id":call_id,"name":name,"arguments":""}))
                        }
                    }
                };
                out.push(self.ev("response.output_item.added", json!({"output_index":oi,"item":item})));
                match kind {
                    BlockKind::Text => out.push(self.ev(
                        "response.content_part.added",
                        json!({"item_id":item_id,"output_index":oi,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
                    )),
                    BlockKind::Thinking => out.push(self.ev(
                        "response.reasoning_summary_part.added",
                        json!({"item_id":item_id,"output_index":oi,"summary_index":0,"part":{"type":"summary_text","text":""}}),
                    )),
                    _ => {}
                }
                self.items.insert(*index, ItemState { output_index: oi, item_id, kind: kind.clone(), buf: String::new() });
            }
            Event::TextDelta { index, text } => {
                if let Some(st) = self.items.get_mut(index) {
                    st.buf.push_str(text);
                    let (id, oi) = (st.item_id.clone(), st.output_index);
                    out.push(self.ev("response.output_text.delta", json!({"item_id":id,"output_index":oi,"content_index":0,"delta":text})));
                }
            }
            Event::ThinkingDelta { index, text } => {
                if let Some(st) = self.items.get_mut(index) {
                    st.buf.push_str(text);
                    let (id, oi) = (st.item_id.clone(), st.output_index);
                    out.push(self.ev(
                        "response.reasoning_summary_text.delta",
                        json!({"item_id":id,"output_index":oi,"summary_index":0,"delta":text}),
                    ));
                }
            }
            Event::ToolArgsDelta { index, json: j } => {
                let custom = self.items.get(index).map(|st| self.is_custom(&st.kind));
                if let Some(st) = self.items.get_mut(index) {
                    st.buf.push_str(j);
                    // custom tool 的参数要等完整后才能取出 input 字符串，先只缓存
                    if custom == Some(false) {
                        let (id, oi) = (st.item_id.clone(), st.output_index);
                        out.push(self.ev("response.function_call_arguments.delta", json!({"item_id":id,"output_index":oi,"delta":j})));
                    }
                }
            }
            Event::BlockStop { index } => {
                let Some(st) = self.items.remove(index) else { return out };
                let (id, oi) = (st.item_id.clone(), st.output_index);
                let item = match &st.kind {
                    BlockKind::Text => {
                        out.push(self.ev("response.output_text.done", json!({"item_id":id,"output_index":oi,"content_index":0,"text":st.buf})));
                        let part = json!({"type":"output_text","text":st.buf,"annotations":[]});
                        out.push(self.ev("response.content_part.done", json!({"item_id":id,"output_index":oi,"content_index":0,"part":part})));
                        json!({"type":"message","id":id,"status":"completed","role":"assistant","content":[part]})
                    }
                    BlockKind::Thinking => {
                        out.push(self.ev(
                            "response.reasoning_summary_text.done",
                            json!({"item_id":id,"output_index":oi,"summary_index":0,"text":st.buf}),
                        ));
                        let part = json!({"type":"summary_text","text":st.buf});
                        out.push(self.ev(
                            "response.reasoning_summary_part.done",
                            json!({"item_id":id,"output_index":oi,"summary_index":0,"part":part}),
                        ));
                        json!({"type":"reasoning","id":id,"summary":[part]})
                    }
                    BlockKind::ToolUse { id: call_id, name } => {
                        if self.is_custom(&st.kind) {
                            let input = parse_args(&st.buf);
                            let raw = input.get("input").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| st.buf.clone());
                            out.push(self.ev("response.custom_tool_call_input.delta", json!({"item_id":id,"output_index":oi,"delta":raw})));
                            out.push(self.ev("response.custom_tool_call_input.done", json!({"item_id":id,"output_index":oi,"input":raw})));
                            json!({"type":"custom_tool_call","id":id,"status":"completed","call_id":call_id,"name":name,"input":raw})
                        } else {
                            let args = if st.buf.trim().is_empty() { "{}".to_string() } else { st.buf.clone() };
                            out.push(self.ev(
                                "response.function_call_arguments.done",
                                json!({"item_id":id,"output_index":oi,"arguments":args}),
                            ));
                            json!({"type":"function_call","id":id,"status":"completed","call_id":call_id,"name":name,"arguments":args})
                        }
                    }
                };
                out.push(self.ev("response.output_item.done", json!({"output_index":oi,"item":item.clone()})));
                self.done_items.push((oi, item));
            }
            Event::Finish { stop_reason, usage } => {
                self.done_items.sort_by_key(|(i, _)| *i);
                let output: Vec<Value> = self.done_items.iter().map(|(_, v)| v.clone()).collect();
                let (ty, status) = if *stop_reason == StopReason::MaxTokens {
                    ("response.incomplete", "incomplete")
                } else {
                    ("response.completed", "completed")
                };
                let r = response_obj(&self.id, &self.ctx, status, output, Some(usage));
                out.push(self.ev(ty, json!({ "response": r })));
            }
            Event::Error { message } => {
                let mut r = response_obj(&self.id, &self.ctx, "failed", vec![], None);
                r["error"] = json!({"code":"server_error","message":message});
                out.push(self.ev("response.failed", json!({ "response": r })));
            }
            Event::SignatureDelta { .. } => {} // Responses 没有可见的签名字段
        }
        out
    }
}
