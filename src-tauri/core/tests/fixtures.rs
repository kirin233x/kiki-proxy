//! 用手写的「真实世界」报文测试解析器，避免只测自己的编码器和解码器能互相读懂。

use kiki_core::ir::*;
use kiki_core::protocol::{self, Compat, EncodeCtx, Normalizer};
use kiki_core::sse::SseParser;
use serde_json::{json, Value};

fn decode_stream(p: Protocol, raw: &str) -> Response {
    let mut parser = SseParser::new();
    let mut dec = protocol::decoder(p);
    let mut norm = Normalizer::new("m");
    let mut evs = Vec::new();
    for e in parser.feed(raw.as_bytes()).into_iter().chain(parser.finish()) {
        for ie in dec.feed(&e) {
            evs.extend(norm.push(ie));
        }
    }
    for ie in dec.finish() {
        evs.extend(norm.push(ie));
    }
    evs.extend(norm.end());
    protocol::aggregate(&evs)
}

#[test]
fn anthropic_real_stream() {
    let raw = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"usage":{"input_tokens":25,"cache_creation_input_tokens":0,"cache_read_input_tokens":100,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me check"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQB"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: ping
data: {"type": "ping"}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"Bash","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\": \"l"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"s\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":89}}

event: message_stop
data: {"type":"message_stop"}

"#;
    let r = decode_stream(Protocol::Anthropic, raw);
    assert_eq!(r.content[0], Part::Thinking { text: "Let me check".into(), signature: Some("EqQB".into()) });
    assert_eq!(r.content[1], Part::ToolUse { id: "toolu_01".into(), name: "Bash".into(), input: json!({"command":"ls"}) });
    assert_eq!(r.stop_reason, StopReason::ToolUse);
    assert_eq!(r.usage, Usage { input: 25, output: 89, cache_read: 100, cache_write: 0 });
}

#[test]
fn deepseek_reasoning_stream() {
    let raw = r#"data: {"id":"a","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"role":"assistant","content":null,"reasoning_content":""},"finish_reason":null}]}

data: {"id":"a","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"content":null,"reasoning_content":"嗯，"},"finish_reason":null}]}

data: {"id":"a","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"content":null,"reasoning_content":"用户问好"},"finish_reason":null}]}

data: {"id":"a","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"content":"你好","reasoning_content":null},"finish_reason":null}]}

data: {"id":"a","object":"chat.completion.chunk","created":1,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"content":"！"},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":30,"total_tokens":50,"prompt_cache_hit_tokens":8,"prompt_cache_miss_tokens":12}}

data: [DONE]

"#;
    let r = decode_stream(Protocol::OpenaiChat, raw);
    assert_eq!(r.content[0], Part::Thinking { text: "嗯，用户问好".into(), signature: None });
    assert_eq!(r.content[1], Part::Text("你好！".into()));
    assert_eq!(r.stop_reason, StopReason::EndTurn);
    assert_eq!(r.usage, Usage { input: 12, output: 30, cache_read: 8, cache_write: 0 });
}

#[test]
fn openai_parallel_tool_calls_stream() {
    let raw = r#"data: {"id":"c","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read","arguments":""}}]}}]}

data: {"id":"c","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"p\":1}"}}]}}]}

data: {"id":"c","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"write","arguments":"{\"p\":"}}]}}]}

data: {"id":"c","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"2}"}}]}}]}

data: {"id":"c","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: {"id":"c","choices":[],"usage":{"prompt_tokens":5,"completion_tokens":7,"prompt_tokens_details":{"cached_tokens":0}}}

data: [DONE]
"#;
    let r = decode_stream(Protocol::OpenaiChat, raw);
    assert_eq!(r.content.len(), 2);
    assert_eq!(r.content[0], Part::ToolUse { id: "call_a".into(), name: "read".into(), input: json!({"p":1}) });
    assert_eq!(r.content[1], Part::ToolUse { id: "call_b".into(), name: "write".into(), input: json!({"p":2}) });
    assert_eq!(r.usage.output, 7);
}

#[test]
fn responses_real_stream_with_custom_tool() {
    let raw = r#"event: response.created
data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_1","object":"response","status":"in_progress","model":"gpt-5-codex","output":[]}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}

event: response.reasoning_summary_part.added
data: {"type":"response.reasoning_summary_part.added","sequence_number":2,"item_id":"rs_1","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}

event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","sequence_number":3,"item_id":"rs_1","output_index":0,"summary_index":0,"delta":"**Planning**"}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":4,"output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"**Planning**"}]}}

event: response.output_item.added
data: {"type":"response.output_item.added","sequence_number":5,"output_index":1,"item":{"id":"ctc_1","type":"custom_tool_call","status":"in_progress","call_id":"call_x","name":"apply_patch","input":""}}

event: response.custom_tool_call_input.delta
data: {"type":"response.custom_tool_call_input.delta","sequence_number":6,"item_id":"ctc_1","output_index":1,"delta":"*** Begin Patch\n*** Add File: \"a.txt\"\n"}

event: response.custom_tool_call_input.delta
data: {"type":"response.custom_tool_call_input.delta","sequence_number":7,"item_id":"ctc_1","output_index":1,"delta":"+hi\n*** End Patch"}

event: response.output_item.done
data: {"type":"response.output_item.done","sequence_number":8,"output_index":1,"item":{"id":"ctc_1","type":"custom_tool_call","status":"completed","call_id":"call_x","name":"apply_patch","input":"*** Begin Patch\n*** Add File: \"a.txt\"\n+hi\n*** End Patch"}}

event: response.completed
data: {"type":"response.completed","sequence_number":9,"response":{"id":"resp_1","status":"completed","output":[],"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":40},"output_tokens":20,"output_tokens_details":{"reasoning_tokens":5},"total_tokens":120}}}

"#;
    let r = decode_stream(Protocol::OpenaiResponses, raw);
    assert_eq!(r.content[0], Part::Thinking { text: "**Planning**".into(), signature: None });
    assert_eq!(
        r.content[1],
        Part::ToolUse {
            id: "call_x".into(),
            name: "apply_patch".into(),
            input: json!({"input":"*** Begin Patch\n*** Add File: \"a.txt\"\n+hi\n*** End Patch"})
        }
    );
    assert_eq!(r.stop_reason, StopReason::ToolUse);
    assert_eq!(r.usage, Usage { input: 60, output: 20, cache_read: 40, cache_write: 0 });
}

#[test]
fn codex_request_custom_tool_roundtrip() {
    // Codex 发来的请求：custom tool + 历史里的 custom_tool_call
    let body = json!({
        "model": "ds/deepseek-chat",
        "instructions": "You are Codex.",
        "input": [
            {"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions>"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"add a file"}]},
            {"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"plan"}],"encrypted_content":"gAAA"},
            {"type":"custom_tool_call","call_id":"c1","name":"apply_patch","input":"*** Begin Patch"},
            {"type":"custom_tool_call_output","call_id":"c1","output":"Done!"},
            {"type":"function_call","call_id":"c2","name":"shell","arguments":"{\"command\":[\"ls\"]}"},
            {"type":"function_call_output","call_id":"c2","output":"a.txt"}
        ],
        "tools": [
            {"type":"function","name":"shell","description":"run","parameters":{"type":"object","properties":{"command":{"type":"array","items":{"type":"string"}}}},"strict":false},
            {"type":"custom","name":"apply_patch","description":"patch","format":{"type":"grammar","syntax":"lark","definition":"start: x"}},
            {"type":"web_search"}
        ],
        "tool_choice": "auto",
        "reasoning": {"effort":"medium","summary":"auto"},
        "stream": true,
        "store": false,
        "include": ["reasoning.encrypted_content"]
    });
    let ir = protocol::parse_request(Protocol::OpenaiResponses, &body).unwrap();
    assert_eq!(ir.system.as_deref(), Some("You are Codex.\n\n<permissions>"));
    assert_eq!(ir.tools.len(), 2);
    assert!(ir.is_custom_tool("apply_patch"));
    // 转成 OpenAI Chat 发给 DeepSeek
    let chat = protocol::build_request(Protocol::OpenaiChat, &ir, "deepseek-chat", &Compat::default());
    let msgs = chat["messages"].as_array().unwrap();
    let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, vec!["system", "user", "assistant", "tool", "assistant", "tool"]);
    assert_eq!(msgs[2]["tool_calls"][0]["function"]["arguments"], json!("{\"input\":\"*** Begin Patch\"}"));
    assert_eq!(chat["tools"][1]["function"]["parameters"]["required"], json!(["input"]));

    // DeepSeek 返回 apply_patch 的函数调用 → 还原成 Codex 的 custom_tool_call
    let ctx = EncodeCtx::new("ds/deepseek-chat", Some(&ir));
    let resp = Response {
        content: vec![Part::ToolUse { id: "c3".into(), name: "apply_patch".into(), input: json!({"input":"*** Begin Patch\n+x"}) }],
        stop_reason: StopReason::ToolUse,
        ..Default::default()
    };
    let mut enc = protocol::encoder(Protocol::OpenaiResponses, ctx);
    let mut out = String::new();
    let mut norm = Normalizer::new("m");
    for ev in protocol::response_to_events(&resp) {
        for e in norm.push(ev) {
            enc.encode(&e).iter().for_each(|s| out.push_str(&s.to_bytes()));
        }
    }
    assert!(out.contains(r#""type":"custom_tool_call""#), "{out}");
    assert!(out.contains(r#""input":"*** Begin Patch\n+x""#), "{out}");
    assert!(out.contains("response.completed"));
}

#[test]
fn gemini_real_stream() {
    let raw = r#"data: {"candidates": [{"content": {"parts": [{"text": "**Thinking**","thought": true}],"role": "model"},"index": 0}],"usageMetadata": {"promptTokenCount": 9,"totalTokenCount": 9},"modelVersion": "gemini-2.5-pro","responseId": "abc"}

data: {"candidates": [{"content": {"parts": [{"text": "Hel"}],"role": "model"},"index": 0}],"modelVersion": "gemini-2.5-pro"}

data: {"candidates": [{"content": {"parts": [{"text": "lo"}],"role": "model"},"index": 0}]}

data: {"candidates": [{"content": {"parts": [{"functionCall": {"name": "list_dir","args": {"path": "."}},"thoughtSignature": "CsYB"}],"role": "model"},"finishReason": "STOP","index": 0}],"usageMetadata": {"promptTokenCount": 9,"candidatesTokenCount": 4,"thoughtsTokenCount": 30,"totalTokenCount": 43}}

"#;
    let r = decode_stream(Protocol::Gemini, raw);
    assert_eq!(r.content[0], Part::Thinking { text: "**Thinking**".into(), signature: None });
    assert_eq!(r.content[1], Part::Text("Hello".into()));
    assert!(matches!(&r.content[2], Part::Thinking { signature: Some(s), .. } if s == "gemini:CsYB"));
    assert!(matches!(&r.content[3], Part::ToolUse { name, input, .. } if name == "list_dir" && input == &json!({"path":"."})));
    assert_eq!(r.stop_reason, StopReason::ToolUse);
    assert_eq!(r.usage, Usage { input: 9, output: 34, cache_read: 0, cache_write: 0 });

    // 签名挂回下一轮请求的函数调用上；发给 Anthropic 时丢掉
    let req = Request {
        messages: vec![
            Message { role: Role::User, content: vec![Part::Text("hi".into())] },
            Message { role: Role::Assistant, content: r.content.clone() },
        ],
        ..Default::default()
    };
    let g = protocol::build_request(Protocol::Gemini, &req, "x", &Compat::default());
    let parts = g["contents"][1]["parts"].as_array().unwrap();
    assert_eq!(parts.last().unwrap()["thoughtSignature"], json!("CsYB"));
    let a = protocol::build_request(Protocol::Anthropic, &req, "x", &Compat::default());
    assert!(!a.to_string().contains("CsYB"), "Gemini 签名不应发给 Anthropic");
}

#[test]
fn claude_code_request_to_openai() {
    // Claude Code 风格：system 数组 + cache_control、图片、tool_result 里带图片
    let body = json!({
        "model": "claude@ds/deepseek-chat",
        "max_tokens": 32000,
        "system": [{"type":"text","text":"You are Claude Code.","cache_control":{"type":"ephemeral"}}],
        "messages": [
            {"role":"user","content":[
                {"type":"text","text":"看看这张图"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBOR"}}
            ]},
            {"role":"assistant","content":[
                {"type":"thinking","thinking":"嗯","signature":"sig"},
                {"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"/a.png"}}
            ]},
            {"role":"user","content":[
                {"type":"tool_result","tool_use_id":"toolu_1","content":[
                    {"type":"text","text":"图片内容如下"},
                    {"type":"image","source":{"type":"base64","media_type":"image/png","data":"SCREEN"}}
                ]},
                {"type":"text","text":"继续","cache_control":{"type":"ephemeral"}}
            ]}
        ],
        "tools": [{"name":"Read","description":"read","input_schema":{"type":"object","properties":{"file_path":{"type":"string"}}}},
                  {"type":"web_search_20250305","name":"web_search","max_uses":5}],
        "thinking": {"type":"enabled","budget_tokens":10000},
        "stream": true
    });
    let ir = protocol::parse_request(Protocol::Anthropic, &body).unwrap();
    assert_eq!(ir.tools.len(), 1, "服务端工具应被忽略");
    let chat = protocol::build_request(
        Protocol::OpenaiChat,
        &ir,
        "deepseek-chat",
        &Compat { send_reasoning_content: true, send_reasoning_effort: true, ..Default::default() },
    );
    let msgs = chat["messages"].as_array().unwrap();
    assert_eq!(msgs[0], json!({"role":"system","content":"You are Claude Code."}));
    assert_eq!(msgs[1]["content"][1]["image_url"]["url"], json!("data:image/png;base64,iVBOR"));
    assert_eq!(msgs[2]["reasoning_content"], json!("嗯"));
    assert_eq!(msgs[3], json!({"role":"tool","tool_call_id":"toolu_1","content":"图片内容如下"}));
    // tool 消息不能带图片，截图被放进紧随其后的 user 消息
    assert_eq!(msgs[4]["role"], json!("user"));
    assert_eq!(msgs[4]["content"][0]["image_url"]["url"], json!("data:image/png;base64,SCREEN"));
    assert_eq!(msgs[4]["content"][1]["text"], json!("继续"));
    assert_eq!(chat["reasoning_effort"], json!("medium"));
    assert_eq!(chat["stream_options"], json!({"include_usage":true}));

    // 转成 Gemini：functionResponse 用工具名而不是 id
    let g = protocol::build_request(Protocol::Gemini, &ir, "gemini-2.5-pro", &Compat::default());
    let s = g.to_string();
    assert!(s.contains(r#""functionResponse":{"name":"Read""#), "{s}");
    assert_eq!(g["generationConfig"]["thinkingConfig"]["thinkingBudget"], json!(10000));
}

#[test]
fn error_bodies_are_readable() {
    assert_eq!(protocol::error_message(br#"{"error":{"message":"bad key","type":"x"}}"#), "bad key");
    assert_eq!(protocol::error_message(br#"{"detail":"nope"}"#), "nope");
    assert_eq!(protocol::error_message(b"<html>502 Bad Gateway</html>"), "<html>502 Bad Gateway</html>");
    let v: Value = protocol::error_body(Protocol::Anthropic, 429, "slow down");
    assert_eq!(v["error"]["type"], json!("rate_limit_error"));
}
