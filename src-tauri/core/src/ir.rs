//! 协议无关的中间表示（IR）。
//!
//! 所有协议转换都是「客户端协议 → IR → 上游协议」，响应方向反过来。
//! 这样 4 种协议只需要各写一对解析/生成函数，而不是 4×3 个两两转换。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 网关支持的 4 种 API 协议
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// Anthropic Messages（Claude Code）
    Anthropic,
    /// OpenAI Chat Completions（大部分 OpenAI 兼容工具）
    OpenaiChat,
    /// OpenAI Responses（Codex CLI）
    OpenaiResponses,
    /// Google Gemini（Gemini CLI）
    Gemini,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Protocol::Anthropic => "Anthropic",
            Protocol::OpenaiChat => "OpenAI Chat",
            Protocol::OpenaiResponses => "OpenAI Responses",
            Protocol::Gemini => "Gemini",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    Url(String),
}

impl ImageSource {
    /// 把 `data:image/png;base64,xxx` 形式的 URL 拆成 base64，其余保持 URL
    pub fn from_url(url: &str) -> Self {
        if let Some(rest) = url.strip_prefix("data:") {
            if let Some((meta, data)) = rest.split_once(',') {
                if let Some(mt) = meta.strip_suffix(";base64") {
                    return ImageSource::Base64 { media_type: mt.to_string(), data: data.to_string() };
                }
            }
        }
        ImageSource::Url(url.to_string())
    }

    pub fn to_url(&self) -> String {
        match self {
            ImageSource::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
            ImageSource::Url(u) => u.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Image(ImageSource),
    Thinking { text: String, signature: Option<String> },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: Vec<Part>, is_error: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Part>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// 普通的 JSON Schema 函数
    Function,
    /// OpenAI Responses 的 custom tool（输入是一段自由文本，Codex 的 apply_patch 用它）。
    /// 转到其他协议时，映射成只有一个 `input: string` 参数的函数。
    Custom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: Option<String>,
    pub schema: Value,
    pub kind: ToolKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolChoice {
    Auto,
    None,
    /// 必须调用某个工具
    Any,
    Tool(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reasoning {
    pub budget_tokens: Option<u32>,
    /// low / medium / high
    pub effort: Option<String>,
}

impl Reasoning {
    pub fn effort_or_from_budget(&self) -> Option<String> {
        if let Some(e) = &self.effort {
            return Some(e.clone());
        }
        self.budget_tokens.map(|b| {
            if b < 4096 {
                "low".into()
            } else if b < 16384 {
                "medium".into()
            } else {
                "high".into()
            }
        })
    }

    pub fn budget_or_from_effort(&self) -> Option<u32> {
        if let Some(b) = self.budget_tokens {
            return Some(b);
        }
        self.effort.as_deref().map(|e| match e {
            "minimal" | "low" => 2048,
            "medium" => 8192,
            _ => 24576,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Request {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub tool_choice: Option<ToolChoice>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
    pub stream: bool,
    pub reasoning: Option<Reasoning>,
}

impl Request {
    pub fn is_custom_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name == name && t.kind == ToolKind::Custom)
    }

    /// 根据 tool_use id 找到工具名（Gemini 的 functionResponse 需要名字而不是 id）
    pub fn tool_name_for_id(&self, id: &str) -> Option<String> {
        for m in &self.messages {
            for p in &m.content {
                if let Part::ToolUse { id: i, name, .. } = p {
                    if i == id {
                        return Some(name.clone());
                    }
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopReason {
    #[default]
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
    ContentFilter,
}

/// token 用量。`input` 是**不含缓存**的输入 token（Anthropic 的口径），
/// 转成 OpenAI/Gemini 时再把缓存加回去。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    pub fn total_input(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }

    /// 合并：后到的非零值覆盖前面的值（流式里 usage 往往分几次给）
    pub fn merge(&mut self, o: &Usage) {
        if o.input > 0 {
            self.input = o.input;
        }
        if o.output > 0 {
            self.output = o.output;
        }
        if o.cache_read > 0 {
            self.cache_read = o.cache_read;
        }
        if o.cache_write > 0 {
            self.cache_write = o.cache_write;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Response {
    pub id: String,
    pub model: String,
    pub content: Vec<Part>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

impl Response {
    pub fn has_tool_use(&self) -> bool {
        self.content.iter().any(|p| matches!(p, Part::ToolUse { .. }))
    }
}

// ------------------------------------------------------------------ 流式事件

#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    Text,
    Thinking,
    ToolUse { id: String, name: String },
}

/// 块状的流式事件（和 Anthropic 的 SSE 结构类似，是表达能力最强的一种）
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Start { id: String, model: String, usage: Usage },
    BlockStart { index: usize, kind: BlockKind },
    TextDelta { index: usize, text: String },
    ThinkingDelta { index: usize, text: String },
    SignatureDelta { index: usize, signature: String },
    ToolArgsDelta { index: usize, json: String },
    BlockStop { index: usize },
    Finish { stop_reason: StopReason, usage: Usage },
    Error { message: String },
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}{}", uuid::Uuid::new_v4().simple())
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 把同一角色的连续消息合并成一条（Anthropic / Gemini 要求角色交替）
pub fn merge_consecutive(messages: Vec<Message>) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    for m in messages {
        if m.content.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.role == m.role => last.content.extend(m.content),
            _ => out.push(m),
        }
    }
    out
}

/// 工具结果里的纯文本拼接（OpenAI 的 tool 消息只支持文本）
pub fn parts_text(parts: &[Part]) -> String {
    let mut s = String::new();
    for p in parts {
        if let Part::Text(t) = p {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(t);
        }
    }
    s
}

/// 粗略估算 token 数（没有上游 count_tokens 接口时用）
pub fn estimate_tokens(req: &Request) -> u64 {
    let mut chars = req.system.as_deref().map(|s| s.chars().count()).unwrap_or(0);
    fn walk(parts: &[Part], chars: &mut usize) {
        for p in parts {
            match p {
                Part::Text(t) => *chars += t.chars().count(),
                Part::Thinking { text, .. } => *chars += text.chars().count(),
                Part::ToolUse { input, .. } => *chars += input.to_string().len(),
                Part::ToolResult { content, .. } => walk(content, chars),
                Part::Image(_) => *chars += 1600 * 4,
            }
        }
    }
    for m in &req.messages {
        walk(&m.content, &mut chars);
    }
    for t in &req.tools {
        chars += t.name.len() + t.description.as_deref().map(str::len).unwrap_or(0) + t.schema.to_string().len();
    }
    (chars as u64 / 4).max(1)
}
