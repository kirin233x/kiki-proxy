//! Google Gemini API（/v1beta/models/{model}:generateContent），Gemini CLI 使用这个协议。

use super::{opt_str, u64_at, Compat, EncodeCtx, StreamDecoder, StreamEncoder};
use crate::ir::*;
use crate::sse::SseEvent;
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};

/// Gemini 3 要求回传函数调用的思考签名；从其他协议转过来时没有签名，用官方文档给的占位值跳过校验
const SKIP_SIGNATURE: &str = "skip_thought_signature_validator";
/// 经过网关转换的 Gemini 签名都加上这个前缀，避免被误发给 Anthropic（反之亦然）
pub const SIG_PREFIX: &str = "gemini:";

fn tag(sig: String) -> String {
    format!("{SIG_PREFIX}{sig}")
}

// ------------------------------------------------------------------ 请求

pub fn parse_request(body: &Value) -> Result<Request> {
    // Gemini 的模型名在 URL 里，由网关在调用前写进 body["model"]
    let model = opt_str(body, "model").unwrap_or_default();
    let system = body.get("systemInstruction").or_else(|| body.get("system_instruction")).map(|s| {
        s.get("parts").and_then(Value::as_array).into_iter().flatten().filter_map(|p| opt_str(p, "text")).collect::<Vec<_>>().join("\n")
    });
    // Gemini 的函数调用没有 id（新版本有时有），按名字排队配对 functionResponse
    let mut pending: HashMap<String, VecDeque<String>> = HashMap::new();
    let mut counter = 0usize;
    let mut messages = Vec::new();
    for c in body.get("contents").and_then(Value::as_array).into_iter().flatten() {
        let role = if c.get("role").and_then(Value::as_str) == Some("model") { Role::Assistant } else { Role::User };
        let mut parts = Vec::new();
        for p in c.get("parts").and_then(Value::as_array).into_iter().flatten() {
            if let Some(fc) = p.get("functionCall") {
                let name = opt_str(fc, "name").unwrap_or_default();
                let id = opt_str(fc, "id").unwrap_or_else(|| {
                    counter += 1;
                    format!("call_{name}_{counter}")
                });
                pending.entry(name.clone()).or_default().push_back(id.clone());
                parts.push(Part::ToolUse { id, name, input: fc.get("args").cloned().unwrap_or_else(|| json!({})) });
            } else if let Some(fr) = p.get("functionResponse") {
                let name = opt_str(fr, "name").unwrap_or_default();
                let id = opt_str(fr, "id")
                    .or_else(|| pending.get_mut(&name).and_then(VecDeque::pop_front))
                    .unwrap_or_else(|| format!("call_{name}"));
                let resp = fr.get("response").cloned().unwrap_or(Value::Null);
                let text = match resp.get("output").or_else(|| resp.get("content")) {
                    Some(Value::String(s)) => s.clone(),
                    _ => resp.to_string(),
                };
                let is_error = resp.get("error").is_some();
                parts.push(Part::ToolResult { tool_use_id: id, content: vec![Part::Text(text)], is_error });
            } else if let Some(d) = p.get("inlineData").or_else(|| p.get("inline_data")) {
                parts.push(Part::Image(ImageSource::Base64 {
                    media_type: opt_str(d, "mimeType").or_else(|| opt_str(d, "mime_type")).unwrap_or_else(|| "image/png".into()),
                    data: opt_str(d, "data").unwrap_or_default(),
                }));
            } else if let Some(d) = p.get("fileData").or_else(|| p.get("file_data")) {
                if let Some(u) = opt_str(d, "fileUri").or_else(|| opt_str(d, "file_uri")) {
                    parts.push(Part::Image(ImageSource::Url(u)));
                }
            } else if let Some(t) = p.get("text").and_then(Value::as_str) {
                if p.get("thought").and_then(Value::as_bool) == Some(true) {
                    parts.push(Part::Thinking { text: t.to_string(), signature: opt_str(p, "thoughtSignature").map(tag) });
                } else {
                    parts.push(Part::Text(t.to_string()));
                }
            }
        }
        messages.push(Message { role, content: parts });
    }
    let mut tools = Vec::new();
    for t in body.get("tools").and_then(Value::as_array).into_iter().flatten() {
        let decls = t.get("functionDeclarations").or_else(|| t.get("function_declarations"));
        for f in decls.and_then(Value::as_array).into_iter().flatten() {
            tools.push(Tool {
                name: opt_str(f, "name").unwrap_or_default(),
                description: opt_str(f, "description"),
                schema: f
                    .get("parametersJsonSchema")
                    .or_else(|| f.get("parameters"))
                    .cloned()
                    .map(lowercase_types)
                    .unwrap_or_else(|| json!({"type":"object","properties":{}})),
                kind: ToolKind::Function,
            });
        }
    }
    let tool_choice = body.pointer("/toolConfig/functionCallingConfig").and_then(|c| {
        match c.get("mode").and_then(Value::as_str) {
            Some("NONE") => Some(ToolChoice::None),
            Some("ANY") => match c.get("allowedFunctionNames").and_then(Value::as_array) {
                Some(a) if a.len() == 1 => a[0].as_str().map(|n| ToolChoice::Tool(n.into())),
                _ => Some(ToolChoice::Any),
            },
            Some("AUTO") => Some(ToolChoice::Auto),
            _ => None,
        }
    });
    let gc = body.get("generationConfig").cloned().unwrap_or_default();
    let reasoning = gc.get("thinkingConfig").and_then(|t| {
        let budget = t.get("thinkingBudget").and_then(Value::as_i64);
        let level = opt_str(t, "thinkingLevel");
        match (budget, level) {
            (Some(b), _) if b > 0 => Some(Reasoning { budget_tokens: Some(b as u32), effort: None }),
            (_, Some(l)) => Some(Reasoning { budget_tokens: None, effort: Some(l.to_lowercase()) }),
            _ => None,
        }
    });
    Ok(Request {
        model,
        system: system.filter(|s| !s.is_empty()),
        messages,
        tools,
        tool_choice,
        max_tokens: gc.get("maxOutputTokens").and_then(Value::as_u64).map(|v| v as u32),
        temperature: gc.get("temperature").and_then(Value::as_f64),
        top_p: gc.get("topP").and_then(Value::as_f64),
        stop: gc
            .get("stopSequences")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        stream: false, // 由 URL（streamGenerateContent）决定，网关会覆盖
        reasoning,
    })
}

/// Gemini 旧式 schema 的类型是大写的（OBJECT/STRING），转成标准 JSON Schema
fn lowercase_types(mut v: Value) -> Value {
    match &mut v {
        Value::Object(m) => {
            for (k, val) in m.iter_mut() {
                if k == "type" {
                    if let Some(s) = val.as_str() {
                        *val = json!(s.to_lowercase());
                    }
                } else {
                    *val = lowercase_types(val.take());
                }
            }
        }
        Value::Array(a) => {
            for x in a.iter_mut() {
                *x = lowercase_types(x.take());
            }
        }
        _ => {}
    }
    v
}

/// 去掉 Gemini 不认识的 JSON Schema 字段
fn clean_schema(mut v: Value) -> Value {
    if let Value::Object(m) = &mut v {
        m.remove("$schema");
        m.remove("$id");
        for (_, val) in m.iter_mut() {
            *val = clean_schema(val.take());
        }
    } else if let Value::Array(a) = &mut v {
        for x in a.iter_mut() {
            *x = clean_schema(x.take());
        }
    }
    v
}

fn image_part(img: &ImageSource) -> Value {
    match img {
        ImageSource::Base64 { media_type, data } => json!({"inlineData":{"mimeType":media_type,"data":data}}),
        ImageSource::Url(u) => json!({"fileData":{"mimeType":"image/*","fileUri":u}}),
    }
}

pub fn build_request(req: &Request, compat: &Compat) -> Value {
    let mut contents = Vec::new();
    for m in merge_consecutive(req.messages.clone()) {
        let mut parts = Vec::new();
        let mut last_sig: Option<String> = None;
        for p in &m.content {
            match p {
                Part::Text(t) => {
                    if !t.is_empty() {
                        parts.push(json!({"text":t}));
                    }
                }
                Part::Image(img) => parts.push(image_part(img)),
                Part::Thinking { signature, .. } => {
                    // 思考文本不回传，但 Gemini 自己的签名要挂到紧随其后的函数调用上
                    last_sig = signature.as_deref().and_then(|s| s.strip_prefix(SIG_PREFIX)).map(str::to_string);
                }
                Part::ToolUse { id: _, name, input } => {
                    let sig = last_sig.take().unwrap_or_else(|| SKIP_SIGNATURE.to_string());
                    parts.push(json!({"functionCall":{"name":name,"args":input},"thoughtSignature":sig}));
                }
                Part::ToolResult { tool_use_id, content, is_error } => {
                    let name = req.tool_name_for_id(tool_use_id).unwrap_or_else(|| tool_use_id.clone());
                    let text = parts_text(content);
                    let response = if *is_error { json!({"error": text}) } else { json!({"output": text}) };
                    parts.push(json!({"functionResponse":{"name":name,"response":response}}));
                    for c in content {
                        if let Part::Image(img) = c {
                            parts.push(image_part(img));
                        }
                    }
                }
            }
        }
        if parts.is_empty() {
            continue;
        }
        contents.push(json!({"role": if m.role == Role::User { "user" } else { "model" }, "parts": parts}));
    }
    let mut body = Map::new();
    body.insert("contents".into(), Value::Array(contents));
    if let Some(s) = req.system.as_ref().filter(|s| !s.is_empty()) {
        body.insert("systemInstruction".into(), json!({"parts":[{"text":s}]}));
    }
    if !req.tools.is_empty() {
        let decls: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                let mut o = json!({"name":t.name,"parametersJsonSchema":clean_schema(t.schema.clone())});
                if let Some(d) = &t.description {
                    o["description"] = json!(d);
                }
                o
            })
            .collect();
        body.insert("tools".into(), json!([{ "functionDeclarations": decls }]));
        if let Some(tc) = &req.tool_choice {
            let cfg = match tc {
                ToolChoice::Auto => json!({"mode":"AUTO"}),
                ToolChoice::None => json!({"mode":"NONE"}),
                ToolChoice::Any => json!({"mode":"ANY"}),
                ToolChoice::Tool(n) => json!({"mode":"ANY","allowedFunctionNames":[n]}),
            };
            body.insert("toolConfig".into(), json!({ "functionCallingConfig": cfg }));
        }
    }
    let mut gc = Map::new();
    if let Some(mt) = req.max_tokens {
        gc.insert("maxOutputTokens".into(), json!(mt));
    }
    if !compat.drop_sampling_params {
        if let Some(t) = req.temperature {
            gc.insert("temperature".into(), json!(t));
        }
        if let Some(t) = req.top_p {
            gc.insert("topP".into(), json!(t));
        }
    }
    if !req.stop.is_empty() {
        gc.insert("stopSequences".into(), json!(req.stop));
    }
    if let Some(r) = &req.reasoning {
        let mut tc = json!({"includeThoughts": true});
        if let Some(b) = r.budget_or_from_effort() {
            tc["thinkingBudget"] = json!(b);
        }
        gc.insert("thinkingConfig".into(), tc);
    }
    if !gc.is_empty() {
        body.insert("generationConfig".into(), Value::Object(gc));
    }
    Value::Object(body)
}

// ------------------------------------------------------------------ 响应

fn parse_stop(s: Option<&str>) -> StopReason {
    match s {
        Some("MAX_TOKENS") => StopReason::MaxTokens,
        Some("SAFETY") | Some("RECITATION") | Some("BLOCKLIST") | Some("PROHIBITED_CONTENT") | Some("SPII") => {
            StopReason::ContentFilter
        }
        _ => StopReason::EndTurn,
    }
}

fn stop_str(s: StopReason) -> &'static str {
    match s {
        StopReason::MaxTokens => "MAX_TOKENS",
        StopReason::ContentFilter => "SAFETY",
        _ => "STOP",
    }
}

pub fn parse_usage(u: &Value) -> Usage {
    let prompt = u64_at(u, "/promptTokenCount");
    let cached = u64_at(u, "/cachedContentTokenCount");
    Usage {
        input: prompt.saturating_sub(cached),
        output: u64_at(u, "/candidatesTokenCount") + u64_at(u, "/thoughtsTokenCount"),
        cache_read: cached,
        cache_write: 0,
    }
}

fn usage_json(u: &Usage) -> Value {
    json!({
        "promptTokenCount": u.total_input(),
        "candidatesTokenCount": u.output,
        "totalTokenCount": u.total_input() + u.output,
        "cachedContentTokenCount": u.cache_read,
    })
}

fn candidate_parts(v: &Value, counter: &mut usize) -> Vec<Part> {
    let mut out = Vec::new();
    for p in v.pointer("/candidates/0/content/parts").and_then(Value::as_array).into_iter().flatten() {
        if let Some(fc) = p.get("functionCall") {
            let name = opt_str(fc, "name").unwrap_or_default();
            *counter += 1;
            let id = opt_str(fc, "id").unwrap_or_else(|| format!("call_{}", new_id("")));
            // 把签名放在一个空的思考块里带回去，下一轮转回 Gemini 时挂回函数调用
            if let Some(sig) = opt_str(p, "thoughtSignature") {
                out.push(Part::Thinking { text: String::new(), signature: Some(tag(sig)) });
            }
            out.push(Part::ToolUse { id, name, input: fc.get("args").cloned().unwrap_or_else(|| json!({})) });
        } else if let Some(t) = p.get("text").and_then(Value::as_str) {
            if p.get("thought").and_then(Value::as_bool) == Some(true) {
                out.push(Part::Thinking { text: t.to_string(), signature: opt_str(p, "thoughtSignature").map(tag) });
            } else {
                out.push(Part::Text(t.to_string()));
            }
        }
    }
    out
}

pub fn parse_response(body: &Value) -> Result<Response> {
    if body.get("error").is_some() && body.get("candidates").is_none() {
        return Err(anyhow!(super::error_message(body.to_string().as_bytes())));
    }
    let mut counter = 0;
    let content = candidate_parts(body, &mut counter);
    let mut stop_reason = parse_stop(body.pointer("/candidates/0/finishReason").and_then(Value::as_str));
    if content.iter().any(|p| matches!(p, Part::ToolUse { .. })) {
        stop_reason = StopReason::ToolUse;
    }
    Ok(Response {
        id: opt_str(body, "responseId").unwrap_or_else(|| new_id("gem_")),
        model: opt_str(body, "modelVersion").unwrap_or_default(),
        content,
        stop_reason,
        usage: body.get("usageMetadata").map(parse_usage).unwrap_or_default(),
    })
}

fn build_parts(parts: &[Part]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut sig: Option<String> = None;
    for p in parts {
        match p {
            Part::Text(t) => out.push(json!({"text":t})),
            Part::Thinking { text, signature } => {
                if !text.is_empty() {
                    out.push(json!({"text":text,"thought":true}));
                }
                sig = signature.as_deref().and_then(|s| s.strip_prefix(SIG_PREFIX)).map(str::to_string);
            }
            Part::ToolUse { id, name, input } => {
                let mut o = json!({"functionCall":{"name":name,"args":input,"id":id}});
                if let Some(s) = sig.take() {
                    o["thoughtSignature"] = json!(s);
                }
                out.push(o);
            }
            _ => {}
        }
    }
    out
}

pub fn build_response(resp: &Response, ctx: &EncodeCtx) -> Value {
    json!({
        "candidates": [{
            "content": { "role": "model", "parts": build_parts(&resp.content) },
            "finishReason": stop_str(resp.stop_reason),
            "index": 0,
        }],
        "usageMetadata": usage_json(&resp.usage),
        "modelVersion": ctx.model,
        "responseId": new_id(""),
    })
}

// ------------------------------------------------------------------ 流式解码

#[derive(Default)]
pub struct Decoder {
    started: bool,
    text_block: Option<usize>,
    think_block: Option<usize>,
    next: usize,
    usage: Usage,
    stop: Option<StopReason>,
    has_tool: bool,
    counter: usize,
    done: bool,
}

impl Decoder {
    fn alloc(&mut self) -> usize {
        let i = self.next;
        self.next += 1;
        i
    }
    fn close(&mut self, out: &mut Vec<Event>) {
        if let Some(i) = self.text_block.take() {
            out.push(Event::BlockStop { index: i });
        }
        if let Some(i) = self.think_block.take() {
            out.push(Event::BlockStop { index: i });
        }
    }
}

impl StreamDecoder for Decoder {
    fn feed(&mut self, ev: &SseEvent) -> Vec<Event> {
        let mut out = Vec::new();
        let Some(v) = ev.json() else { return out };
        if v.get("error").is_some() && v.get("candidates").is_none() {
            out.push(Event::Error { message: super::error_message(ev.data.as_bytes()) });
            return out;
        }
        if !self.started {
            self.started = true;
            out.push(Event::Start {
                id: opt_str(&v, "responseId").unwrap_or_else(|| new_id("gem_")),
                model: opt_str(&v, "modelVersion").unwrap_or_default(),
                usage: Usage::default(),
            });
        }
        if let Some(u) = v.get("usageMetadata") {
            self.usage.merge(&parse_usage(u));
        }
        for p in v.pointer("/candidates/0/content/parts").and_then(Value::as_array).into_iter().flatten() {
            if let Some(fc) = p.get("functionCall") {
                self.close(&mut out);
                self.has_tool = true;
                self.counter += 1;
                if let Some(sig) = opt_str(p, "thoughtSignature") {
                    let i = self.alloc();
                    out.push(Event::BlockStart { index: i, kind: BlockKind::Thinking });
                    out.push(Event::SignatureDelta { index: i, signature: tag(sig) });
                    out.push(Event::BlockStop { index: i });
                }
                let i = self.alloc();
                let name = opt_str(fc, "name").unwrap_or_default();
                let id = opt_str(fc, "id").unwrap_or_else(|| format!("call_{}", new_id("")));
                out.push(Event::BlockStart { index: i, kind: BlockKind::ToolUse { id, name } });
                out.push(Event::ToolArgsDelta { index: i, json: fc.get("args").cloned().unwrap_or_else(|| json!({})).to_string() });
                out.push(Event::BlockStop { index: i });
            } else if let Some(t) = p.get("text").and_then(Value::as_str) {
                let thought = p.get("thought").and_then(Value::as_bool) == Some(true);
                if thought {
                    if let Some(i) = self.text_block.take() {
                        out.push(Event::BlockStop { index: i });
                    }
                    let i = match self.think_block {
                        Some(i) => i,
                        None => {
                            let i = self.alloc();
                            self.think_block = Some(i);
                            out.push(Event::BlockStart { index: i, kind: BlockKind::Thinking });
                            i
                        }
                    };
                    out.push(Event::ThinkingDelta { index: i, text: t.to_string() });
                    if let Some(sig) = opt_str(p, "thoughtSignature") {
                        out.push(Event::SignatureDelta { index: i, signature: tag(sig) });
                    }
                } else {
                    if let Some(i) = self.think_block.take() {
                        out.push(Event::BlockStop { index: i });
                    }
                    let i = match self.text_block {
                        Some(i) => i,
                        None => {
                            let i = self.alloc();
                            self.text_block = Some(i);
                            out.push(Event::BlockStart { index: i, kind: BlockKind::Text });
                            i
                        }
                    };
                    if !t.is_empty() {
                        out.push(Event::TextDelta { index: i, text: t.to_string() });
                    }
                }
            }
        }
        if let Some(fr) = v.pointer("/candidates/0/finishReason").and_then(Value::as_str) {
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
        self.close(&mut out);
        let mut stop = self.stop.unwrap_or_default();
        if self.has_tool && stop == StopReason::EndTurn {
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
    /// 工具调用的参数要攒齐了才能一次发出（Gemini 的 functionCall 不支持增量）
    tools: HashMap<usize, (String, String, String)>,
    sigs: HashMap<usize, String>,
    pending_sig: Option<String>,
}

impl Encoder {
    pub fn new(ctx: EncodeCtx) -> Self {
        Encoder { ctx, id: new_id(""), tools: HashMap::new(), sigs: HashMap::new(), pending_sig: None }
    }

    fn chunk(&self, parts: Vec<Value>, finish: Option<&str>, usage: Option<&Usage>) -> SseEvent {
        let mut cand = json!({"content":{"role":"model","parts":parts},"index":0});
        if let Some(f) = finish {
            cand["finishReason"] = json!(f);
        }
        let mut o = json!({"candidates":[cand],"modelVersion":self.ctx.model,"responseId":self.id});
        if let Some(u) = usage {
            o["usageMetadata"] = usage_json(u);
        }
        SseEvent::data(o.to_string())
    }
}

impl StreamEncoder for Encoder {
    fn encode(&mut self, ev: &Event) -> Vec<SseEvent> {
        match ev {
            Event::BlockStart { index, kind: BlockKind::ToolUse { id, name } } => {
                self.tools.insert(*index, (id.clone(), name.clone(), String::new()));
                vec![]
            }
            Event::TextDelta { text, .. } => vec![self.chunk(vec![json!({"text":text})], None, None)],
            Event::ThinkingDelta { text, .. } => {
                if text.is_empty() {
                    vec![]
                } else {
                    vec![self.chunk(vec![json!({"text":text,"thought":true})], None, None)]
                }
            }
            Event::SignatureDelta { index, signature } => {
                if let Some(s) = signature.strip_prefix(SIG_PREFIX) {
                    self.sigs.insert(*index, s.to_string());
                }
                vec![]
            }
            Event::ToolArgsDelta { index, json: j } => {
                if let Some(t) = self.tools.get_mut(index) {
                    t.2.push_str(j);
                }
                vec![]
            }
            Event::BlockStop { index } => {
                if let Some(sig) = self.sigs.remove(index) {
                    self.pending_sig = Some(sig);
                }
                match self.tools.remove(index) {
                    Some((id, name, args)) => {
                        let mut p = json!({"functionCall":{"name":name,"args":super::parse_args(&args),"id":id}});
                        if let Some(s) = self.pending_sig.take() {
                            p["thoughtSignature"] = json!(s);
                        }
                        vec![self.chunk(vec![p], None, None)]
                    }
                    None => vec![],
                }
            }
            Event::Finish { stop_reason, usage } => vec![self.chunk(vec![], Some(stop_str(*stop_reason)), Some(usage))],
            Event::Error { message } => {
                vec![SseEvent::data(json!({"error":{"code":500,"message":message,"status":"INTERNAL"}}).to_string())]
            }
            _ => vec![],
        }
    }
}
