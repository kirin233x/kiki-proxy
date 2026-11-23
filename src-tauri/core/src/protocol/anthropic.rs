//! Anthropic Messages API（/v1/messages）

use super::{opt_str, u64_at, EncodeCtx, StreamDecoder, StreamEncoder};
use crate::ir::*;
use crate::sse::SseEvent;
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

// ------------------------------------------------------------------ 请求

fn parse_image(v: &Value) -> Option<ImageSource> {
    let src = v.get("source")?;
    match src.get("type").and_then(Value::as_str) {
        Some("base64") => Some(ImageSource::Base64 {
            media_type: opt_str(src, "media_type").unwrap_or_else(|| "image/png".into()),
            data: opt_str(src, "data").unwrap_or_default(),
        }),
        Some("url") => opt_str(src, "url").map(ImageSource::Url),
        _ => None,
    }
}

fn parse_blocks(content: &Value) -> Vec<Part> {
    if let Some(s) = content.as_str() {
        return vec![Part::Text(s.to_string())];
    }
    let mut out = Vec::new();
    for b in content.as_array().into_iter().flatten() {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => out.push(Part::Text(opt_str(b, "text").unwrap_or_default())),
            Some("image") => {
                if let Some(img) = parse_image(b) {
                    out.push(Part::Image(img));
                }
            }
            Some("thinking") => out.push(Part::Thinking {
                text: opt_str(b, "thinking").unwrap_or_default(),
                signature: opt_str(b, "signature"),
            }),
            Some("tool_use") => out.push(Part::ToolUse {
                id: opt_str(b, "id").unwrap_or_default(),
                name: opt_str(b, "name").unwrap_or_default(),
                input: b.get("input").cloned().unwrap_or_else(|| json!({})),
            }),
            Some("tool_result") => out.push(Part::ToolResult {
                tool_use_id: opt_str(b, "tool_use_id").unwrap_or_default(),
                content: b.get("content").map(parse_blocks).unwrap_or_default(),
                is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            }),
            Some("document") => {
                // PDF 等文档：只能保留文本型的
                if let Some(t) = b.pointer("/source/data").and_then(Value::as_str) {
                    if b.pointer("/source/type").and_then(Value::as_str) == Some("text") {
                        out.push(Part::Text(t.to_string()));
                    }
                }
            }
            _ => {} // redacted_thinking、server_tool_use 等无法跨协议表达的块
        }
    }
    out
}

/// Claude Code 新版会在 messages 里插入 `role: "system"` 的消息（插件提示、子任务通知等），
/// subagent 的请求经常以这样一条消息结尾。官方 API 认识它，但大多数兼容服务（以及转 OpenAI 格式的程序）
/// 不认识，常被当成助手消息，模型以为已经答完了，subagent 就直接结束。
/// 这里把它们并进相邻的用户消息（包在 <system-reminder> 里），保证对话仍以用户消息结尾。返回是否改动过
pub fn fold_system_messages(messages: &mut Vec<Value>) -> bool {
    if !messages.iter().any(|m| m.get("role").and_then(Value::as_str) == Some("system")) {
        return false;
    }
    let blocks_of = |m: &Value| -> Vec<Value> {
        match m.get("content") {
            Some(Value::String(s)) => vec![json!({"type":"text","text":s})],
            Some(Value::Array(a)) => a.clone(),
            _ => vec![],
        }
    };
    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    let mut pending: Vec<Value> = Vec::new();
    for m in messages.drain(..) {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("user").to_string();
        if role == "system" {
            let text: Vec<String> = blocks_of(&m).iter().filter_map(|b| b.get("text").and_then(Value::as_str).map(String::from)).collect();
            let text = text.join("\n\n");
            if text.trim().is_empty() {
                continue;
            }
            let block = json!({"type":"text","text":format!("<system-reminder>\n{text}\n</system-reminder>")});
            // 前一条是用户消息就接在它后面
            match out.last_mut() {
                Some(prev) if prev.get("role").and_then(Value::as_str) == Some("user") => {
                    let mut b = blocks_of(prev);
                    b.push(block);
                    prev["content"] = json!(b);
                }
                _ => pending.push(block),
            }
            continue;
        }
        if !pending.is_empty() {
            if role == "user" {
                // 并进下一条用户消息；tool_result 必须排在最前面，所以放到末尾
                let mut b = blocks_of(&m);
                let has_result = b.iter().any(|x| x.get("type").and_then(Value::as_str) == Some("tool_result"));
                let mut m = m;
                if has_result {
                    b.append(&mut pending);
                } else {
                    pending.append(&mut b);
                    b = std::mem::take(&mut pending);
                }
                m["content"] = json!(b);
                out.push(m);
                continue;
            }
            out.push(json!({"role":"user","content": std::mem::take(&mut pending)}));
        }
        out.push(m);
    }
    if !pending.is_empty() {
        out.push(json!({"role":"user","content": pending}));
    }
    *messages = out;
    true
}

pub fn parse_request(body: &Value) -> Result<Request> {
    let model = opt_str(body, "model").ok_or_else(|| anyhow!("请求缺少 model 字段"))?;
    let system = match body.get("system") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(a)) => {
            let s: Vec<String> = a.iter().filter_map(|b| opt_str(b, "text")).collect();
            (!s.is_empty()).then(|| s.join("\n\n"))
        }
        _ => None,
    };
    let mut raw = body.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
    fold_system_messages(&mut raw);
    let mut messages = Vec::new();
    for m in &raw {
        let role = if m.get("role").and_then(Value::as_str) == Some("assistant") { Role::Assistant } else { Role::User };
        messages.push(Message { role, content: m.get("content").map(parse_blocks).unwrap_or_default() });
    }
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("input_schema").is_some())
        .map(|t| Tool {
            name: opt_str(t, "name").unwrap_or_default(),
            description: opt_str(t, "description"),
            schema: t.get("input_schema").cloned().unwrap_or_else(|| json!({"type":"object"})),
            kind: ToolKind::Function,
        })
        .collect();
    let tool_choice = body.get("tool_choice").and_then(|tc| match tc.get("type").and_then(Value::as_str) {
        Some("auto") => Some(ToolChoice::Auto),
        Some("any") => Some(ToolChoice::Any),
        Some("none") => Some(ToolChoice::None),
        Some("tool") => opt_str(tc, "name").map(ToolChoice::Tool),
        _ => None,
    });
    let reasoning = body.get("thinking").and_then(|t| {
        (t.get("type").and_then(Value::as_str) == Some("enabled")).then(|| Reasoning {
            budget_tokens: t.get("budget_tokens").and_then(Value::as_u64).map(|b| b as u32),
            effort: None,
        })
    });
    Ok(Request {
        model,
        system,
        messages,
        tools,
        tool_choice,
        max_tokens: body.get("max_tokens").and_then(Value::as_u64).map(|v| v as u32),
        temperature: body.get("temperature").and_then(Value::as_f64),
        top_p: body.get("top_p").and_then(Value::as_f64),
        stop: body
            .get("stop_sequences")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        reasoning,
    })
}

fn image_block(img: &ImageSource) -> Value {
    match img {
        ImageSource::Base64 { media_type, data } => {
            json!({"type":"image","source":{"type":"base64","media_type":media_type,"data":data}})
        }
        ImageSource::Url(u) => json!({"type":"image","source":{"type":"url","url":u}}),
    }
}

fn build_blocks(parts: &[Part]) -> Vec<Value> {
    let mut out = Vec::new();
    for p in parts {
        match p {
            Part::Text(t) => {
                if !t.is_empty() {
                    out.push(json!({"type":"text","text":t}));
                }
            }
            Part::Image(img) => out.push(image_block(img)),
            Part::Thinking { text, signature } => {
                // 没有签名的思考块（来自其他厂商）Anthropic 会拒绝，只能丢掉
                let foreign = |s: &String| s.is_empty() || s.starts_with(super::gemini::SIG_PREFIX);
                if let Some(sig) = signature.as_ref().filter(|s| !foreign(s)) {
                    out.push(json!({"type":"thinking","thinking":text,"signature":sig}));
                }
            }
            Part::ToolUse { id, name, input } => {
                let input = if input.is_object() { input.clone() } else { json!({ "input": input }) };
                out.push(json!({"type":"tool_use","id":id,"name":name,"input":input}));
            }
            Part::ToolResult { tool_use_id, content, is_error } => {
                let mut b = json!({"type":"tool_result","tool_use_id":tool_use_id,"content":build_blocks(content)});
                if *is_error {
                    b["is_error"] = json!(true);
                }
                out.push(b);
            }
        }
    }
    out
}

pub fn build_request(req: &Request, model: &str) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    let mut max_tokens = req.max_tokens.unwrap_or(8192);
    if let Some(r) = &req.reasoning {
        if let Some(budget) = r.budget_or_from_effort() {
            let budget = budget.max(1024);
            if max_tokens <= budget {
                max_tokens = budget + 4096;
            }
            body.insert("thinking".into(), json!({"type":"enabled","budget_tokens":budget}));
        }
    }
    body.insert("max_tokens".into(), json!(max_tokens));
    if let Some(s) = req.system.as_ref().filter(|s| !s.is_empty()) {
        body.insert("system".into(), json!(s));
    }
    let msgs: Vec<Value> = merge_consecutive(req.messages.clone())
        .iter()
        .map(|m| {
            json!({
                "role": if m.role == Role::User { "user" } else { "assistant" },
                "content": build_blocks(&m.content),
            })
        })
        .filter(|m| m["content"].as_array().is_some_and(|a| !a.is_empty()))
        .collect();
    body.insert("messages".into(), Value::Array(msgs));
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                let mut o = json!({"name":t.name,"input_schema":t.schema});
                if let Some(d) = &t.description {
                    o["description"] = json!(d);
                }
                o
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        if let Some(tc) = &req.tool_choice {
            body.insert(
                "tool_choice".into(),
                match tc {
                    ToolChoice::Auto => json!({"type":"auto"}),
                    ToolChoice::None => json!({"type":"none"}),
                    ToolChoice::Any => json!({"type":"any"}),
                    ToolChoice::Tool(n) => json!({"type":"tool","name":n}),
                },
            );
        }
    }
    // 开启思考时 Anthropic 不允许改 temperature
    if req.reasoning.is_none() {
        if let Some(t) = req.temperature {
            body.insert("temperature".into(), json!(t));
        }
        if let Some(t) = req.top_p {
            body.insert("top_p".into(), json!(t));
        }
    }
    if !req.stop.is_empty() {
        body.insert("stop_sequences".into(), json!(req.stop));
    }
    if req.stream {
        body.insert("stream".into(), json!(true));
    }
    Value::Object(body)
}

// ------------------------------------------------------------------ 响应

pub fn parse_stop(s: Option<&str>) -> StopReason {
    match s {
        Some("max_tokens") | Some("model_context_window_exceeded") => StopReason::MaxTokens,
        Some("tool_use") => StopReason::ToolUse,
        Some("stop_sequence") => StopReason::StopSequence,
        Some("refusal") => StopReason::ContentFilter,
        _ => StopReason::EndTurn,
    }
}

pub fn stop_str(s: StopReason) -> &'static str {
    match s {
        StopReason::EndTurn | StopReason::ContentFilter => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::ToolUse => "tool_use",
        StopReason::StopSequence => "stop_sequence",
    }
}

pub fn parse_usage(u: &Value) -> Usage {
    Usage {
        input: u64_at(u, "/input_tokens"),
        output: u64_at(u, "/output_tokens"),
        cache_read: u64_at(u, "/cache_read_input_tokens"),
        cache_write: u64_at(u, "/cache_creation_input_tokens"),
    }
}

fn usage_json(u: &Usage) -> Value {
    json!({
        "input_tokens": u.input,
        "output_tokens": u.output,
        "cache_read_input_tokens": u.cache_read,
        "cache_creation_input_tokens": u.cache_write,
    })
}

pub fn parse_response(body: &Value) -> Result<Response> {
    if body.get("type").and_then(Value::as_str) == Some("error") {
        return Err(anyhow!(super::error_message(body.to_string().as_bytes())));
    }
    Ok(Response {
        id: opt_str(body, "id").unwrap_or_else(|| new_id("msg_")),
        model: opt_str(body, "model").unwrap_or_default(),
        content: body.get("content").map(parse_blocks).unwrap_or_default(),
        stop_reason: parse_stop(body.get("stop_reason").and_then(Value::as_str)),
        usage: body.get("usage").map(parse_usage).unwrap_or_default(),
    })
}

pub fn build_response(resp: &Response, ctx: &EncodeCtx) -> Value {
    json!({
        "id": if resp.id.starts_with("msg_") { resp.id.clone() } else { new_id("msg_") },
        "type": "message",
        "role": "assistant",
        "model": ctx.model,
        "content": build_blocks(&resp.content),
        "stop_reason": stop_str(resp.stop_reason),
        "stop_sequence": Value::Null,
        "usage": usage_json(&resp.usage),
    })
}

// ------------------------------------------------------------------ 流式解码

#[derive(Default)]
pub struct Decoder {
    usage: Usage,
    stop: Option<StopReason>,
    /// 上游块序号 → 是否是我们认识的块（不认识的块的后续事件要丢掉）
    known: HashMap<usize, bool>,
}

impl StreamDecoder for Decoder {
    fn feed(&mut self, ev: &SseEvent) -> Vec<Event> {
        let Some(v) = ev.json() else { return vec![] };
        let ty = v.get("type").and_then(Value::as_str).or(ev.event.as_deref()).unwrap_or("");
        let index = v.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        match ty {
            "message_start" => {
                let m = v.get("message").cloned().unwrap_or_default();
                self.usage.merge(&m.get("usage").map(parse_usage).unwrap_or_default());
                vec![Event::Start {
                    id: opt_str(&m, "id").unwrap_or_else(|| new_id("msg_")),
                    model: opt_str(&m, "model").unwrap_or_default(),
                    usage: self.usage,
                }]
            }
            "content_block_start" => {
                let b = v.get("content_block").cloned().unwrap_or_default();
                let kind = match b.get("type").and_then(Value::as_str) {
                    Some("text") => Some(BlockKind::Text),
                    Some("thinking") => Some(BlockKind::Thinking),
                    Some("tool_use") => Some(BlockKind::ToolUse {
                        id: opt_str(&b, "id").unwrap_or_default(),
                        name: opt_str(&b, "name").unwrap_or_default(),
                    }),
                    _ => None,
                };
                self.known.insert(index, kind.is_some());
                let mut out = Vec::new();
                if let Some(kind) = kind {
                    out.push(Event::BlockStart { index, kind });
                    // 极少数情况下起始块里就带了文本
                    if let Some(t) = b.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                        out.push(Event::TextDelta { index, text: t.to_string() });
                    }
                }
                out
            }
            "content_block_delta" => {
                if self.known.get(&index) == Some(&false) {
                    return vec![];
                }
                let d = v.get("delta").cloned().unwrap_or_default();
                match d.get("type").and_then(Value::as_str) {
                    Some("text_delta") => vec![Event::TextDelta { index, text: opt_str(&d, "text").unwrap_or_default() }],
                    Some("thinking_delta") => {
                        vec![Event::ThinkingDelta { index, text: opt_str(&d, "thinking").unwrap_or_default() }]
                    }
                    Some("signature_delta") => {
                        vec![Event::SignatureDelta { index, signature: opt_str(&d, "signature").unwrap_or_default() }]
                    }
                    Some("input_json_delta") => {
                        vec![Event::ToolArgsDelta { index, json: opt_str(&d, "partial_json").unwrap_or_default() }]
                    }
                    _ => vec![],
                }
            }
            "content_block_stop" => {
                if self.known.get(&index) == Some(&false) {
                    return vec![];
                }
                vec![Event::BlockStop { index }]
            }
            "message_delta" => {
                if let Some(u) = v.get("usage") {
                    self.usage.merge(&parse_usage(u));
                }
                if let Some(s) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop = Some(parse_stop(Some(s)));
                }
                vec![]
            }
            "message_stop" => vec![Event::Finish { stop_reason: self.stop.unwrap_or_default(), usage: self.usage }],
            "error" => vec![Event::Error { message: super::error_message(ev.data.as_bytes()) }],
            _ => vec![],
        }
    }

    fn finish(&mut self) -> Vec<Event> {
        // 没收到 message_stop 但收到了 message_delta
        match self.stop {
            Some(s) => vec![Event::Finish { stop_reason: s, usage: self.usage }],
            None => vec![],
        }
    }
}

// ------------------------------------------------------------------ 流式编码

pub struct Encoder {
    ctx: EncodeCtx,
    /// IR 块序号 → 输出的块序号（保证连续从 0 开始）
    map: HashMap<usize, usize>,
    next: usize,
}

impl Encoder {
    pub fn new(ctx: EncodeCtx) -> Self {
        Encoder { ctx, map: HashMap::new(), next: 0 }
    }

    fn ev(name: &str, data: Value) -> SseEvent {
        SseEvent::named(name, data.to_string())
    }
}

impl StreamEncoder for Encoder {
    fn encode(&mut self, ev: &Event) -> Vec<SseEvent> {
        match ev {
            Event::Start { id, usage, .. } => {
                let id = if id.starts_with("msg_") { id.clone() } else { new_id("msg_") };
                vec![Self::ev(
                    "message_start",
                    json!({"type":"message_start","message":{
                        "id": id, "type":"message","role":"assistant","model": self.ctx.model,
                        "content": [], "stop_reason": Value::Null, "stop_sequence": Value::Null,
                        "usage": usage_json(&Usage { output: 0, ..*usage }),
                    }}),
                )]
            }
            Event::BlockStart { index, kind } => {
                let i = self.next;
                self.next += 1;
                self.map.insert(*index, i);
                let block = match kind {
                    BlockKind::Text => json!({"type":"text","text":""}),
                    BlockKind::Thinking => json!({"type":"thinking","thinking":"","signature":""}),
                    BlockKind::ToolUse { id, name } => json!({"type":"tool_use","id":id,"name":name,"input":{}}),
                };
                vec![Self::ev("content_block_start", json!({"type":"content_block_start","index":i,"content_block":block}))]
            }
            Event::TextDelta { index, text } => self.delta(*index, json!({"type":"text_delta","text":text})),
            Event::ThinkingDelta { index, text } => self.delta(*index, json!({"type":"thinking_delta","thinking":text})),
            Event::SignatureDelta { index, signature } => {
                self.delta(*index, json!({"type":"signature_delta","signature":signature}))
            }
            Event::ToolArgsDelta { index, json: j } => {
                self.delta(*index, json!({"type":"input_json_delta","partial_json":j}))
            }
            Event::BlockStop { index } => match self.map.get(index) {
                Some(i) => vec![Self::ev("content_block_stop", json!({"type":"content_block_stop","index":i}))],
                None => vec![],
            },
            Event::Finish { stop_reason, usage } => vec![
                Self::ev(
                    "message_delta",
                    json!({"type":"message_delta",
                        "delta":{"stop_reason":stop_str(*stop_reason),"stop_sequence":Value::Null},
                        "usage": usage_json(usage)}),
                ),
                Self::ev("message_stop", json!({"type":"message_stop"})),
            ],
            Event::Error { message } => vec![Self::ev(
                "error",
                json!({"type":"error","error":{"type":"api_error","message":message}}),
            )],
        }
    }
}

impl Encoder {
    fn delta(&self, index: usize, delta: Value) -> Vec<SseEvent> {
        match self.map.get(&index) {
            Some(i) => vec![Self::ev("content_block_delta", json!({"type":"content_block_delta","index":i,"delta":delta}))],
            None => vec![],
        }
    }
}

#[cfg(test)]
mod fold_tests {
    use super::fold_system_messages;
    use serde_json::json;

    /// Claude Code subagent 的真实请求结构：user, system, assistant(tool_use), user(tool_result), system
    #[test]
    fn system_messages_fold_into_user() {
        let mut m = vec![
            json!({"role":"user","content":[{"type":"text","text":"任务"}]}),
            json!({"role":"system","content":"插件提示"}),
            json!({"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}),
            json!({"role":"system","content":[{"type":"text","text":"后台通知"}]}),
        ];
        assert!(fold_system_messages(&mut m));
        let roles: Vec<&str> = m.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        assert_eq!(m[0]["content"][1]["text"], json!("<system-reminder>\n插件提示\n</system-reminder>"));
        assert_eq!(m[2]["content"][0]["type"], json!("tool_result"));
        assert_eq!(m[2]["content"][1]["text"], json!("<system-reminder>\n后台通知\n</system-reminder>"));

        // 开头就是 system、后面跟 tool_result 以外的用户消息：并到用户消息前面
        let mut m = vec![json!({"role":"system","content":"a"}), json!({"role":"user","content":"b"})];
        fold_system_messages(&mut m);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0]["content"][1]["text"], json!("b"));
        // 夹在两条助手消息之间：变成一条用户消息
        let mut m = vec![
            json!({"role":"user","content":"q"}),
            json!({"role":"assistant","content":"x"}),
            json!({"role":"system","content":"s"}),
        ];
        fold_system_messages(&mut m);
        assert_eq!(m.last().unwrap()["role"], json!("user"));
        // 没有 system 消息时不动
        let mut m = vec![json!({"role":"user","content":"q"})];
        assert!(!fold_system_messages(&mut m));
    }
}
