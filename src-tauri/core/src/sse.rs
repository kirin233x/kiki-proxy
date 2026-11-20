//! Server-Sent Events 的增量解析与序列化。

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SseEvent {
    /// `event:` 字段，没有时为 None
    pub event: Option<String>,
    /// 多行 `data:` 用 `\n` 拼起来
    pub data: String,
}

impl SseEvent {
    pub fn data(data: impl Into<String>) -> Self {
        SseEvent { event: None, data: data.into() }
    }

    pub fn named(event: impl Into<String>, data: impl Into<String>) -> Self {
        SseEvent { event: Some(event.into()), data: data.into() }
    }

    pub fn to_bytes(&self) -> String {
        let mut s = String::new();
        if let Some(e) = &self.event {
            s.push_str("event: ");
            s.push_str(e);
            s.push('\n');
        }
        for line in self.data.split('\n') {
            s.push_str("data: ");
            s.push_str(line);
            s.push('\n');
        }
        s.push('\n');
        s
    }

    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.data).ok()
    }
}

/// 增量解析器：喂任意切分的字节，吐出完整的事件
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    cur: SseEvent,
    has_data: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line(&String::from_utf8_lossy(&line), &mut out);
        }
        out
    }

    /// 流结束时把没有以空行结尾的最后一个事件也吐出来
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let s = String::from_utf8_lossy(&rest).to_string();
            for l in s.split('\n') {
                self.line(l.trim_end_matches('\r'), &mut out);
            }
        }
        self.dispatch(&mut out);
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line.starts_with(':') {
            return; // 注释 / 心跳
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.cur.event = Some(value.to_string()),
            "data" => {
                if self.has_data {
                    self.cur.data.push('\n');
                }
                self.cur.data.push_str(value);
                self.has_data = true;
            }
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        if self.has_data || self.cur.event.is_some() {
            out.push(std::mem::take(&mut self.cur));
        }
        self.cur = SseEvent::default();
        self.has_data = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_split_chunks() {
        let mut p = SseParser::new();
        let mut evs = p.feed(b"event: a\nda");
        assert!(evs.is_empty());
        evs.extend(p.feed(b"ta: {\"x\":1}\r\n\r\ndata: [DONE]\n"));
        evs.extend(p.finish());
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].event.as_deref(), Some("a"));
        assert_eq!(evs[0].data, "{\"x\":1}");
        assert_eq!(evs[1].data, "[DONE]");
    }
}
