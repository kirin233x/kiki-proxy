//! 请求日志与用量统计（SQLite）。
//!
//! - `requests`：每个请求一行摘要（模型、端点、状态、耗时、token、费用），统计用，默认保留 90 天
//! - `bodies`：完整的请求/响应内容，调试用，默认保留 24 小时

use crate::ir::{now_millis, Usage};
use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub gateway_model: String,
    pub endpoint: String,
    pub status: u16,
    pub error: String,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    pub id: String,
    /// 毫秒时间戳
    pub ts: i64,
    pub client_protocol: String,
    pub path: String,
    pub requested_model: String,
    pub gateway_model: String,
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub upstream_model: String,
    pub upstream_protocol: String,
    pub proxy: String,
    pub rule: String,
    pub status: u16,
    pub error: String,
    pub duration_ms: i64,
    pub ttfb_ms: i64,
    pub stream: bool,
    pub usage: Usage,
    pub cost: f64,
    pub attempts: Vec<Attempt>,
    /// 正在进行中（流式还没结束）
    #[serde(default)]
    pub pending: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogBodies {
    pub client_request: String,
    pub client_headers: String,
    pub upstream_url: String,
    pub upstream_request: String,
    pub upstream_headers: String,
    pub upstream_status: u16,
    pub upstream_response: String,
    pub client_response: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LogQuery {
    pub q: String,
    pub endpoint: String,
    /// all / ok / error
    pub status: String,
    pub before: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub key: String,
    pub label: String,
    pub requests: u64,
    pub errors: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
    pub avg_ms: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyRow {
    /// YYYY-MM-DD（本地时区由前端按 ts 换算，这里用 UTC 偏移参数）
    pub day: String,
    pub requests: u64,
    pub errors: u64,
    pub input: u64,
    pub output: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total: UsageRow,
    pub by_model: Vec<UsageRow>,
    pub by_endpoint: Vec<UsageRow>,
    pub daily: Vec<DailyRow>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS requests (
                id TEXT PRIMARY KEY, ts INTEGER NOT NULL,
                client_protocol TEXT, path TEXT, requested_model TEXT, gateway_model TEXT,
                endpoint_id TEXT, endpoint_name TEXT, upstream_model TEXT, upstream_protocol TEXT,
                proxy TEXT, rule TEXT, status INTEGER, error TEXT,
                duration_ms INTEGER, ttfb_ms INTEGER, stream INTEGER,
                input_tokens INTEGER, output_tokens INTEGER, cache_read INTEGER, cache_write INTEGER,
                cost REAL, attempts TEXT
             );
             CREATE INDEX IF NOT EXISTS requests_ts ON requests(ts);
             CREATE TABLE IF NOT EXISTS bodies (
                id TEXT PRIMARY KEY, ts INTEGER NOT NULL,
                client_request TEXT, client_headers TEXT, upstream_url TEXT,
                upstream_request TEXT, upstream_headers TEXT, upstream_status INTEGER,
                upstream_response TEXT, client_response TEXT
             );
             CREATE INDEX IF NOT EXISTS bodies_ts ON bodies(ts);",
        )?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn insert(&self, r: &LogRecord, b: Option<&LogBodies>) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO requests VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
            params![
                r.id,
                r.ts,
                r.client_protocol,
                r.path,
                r.requested_model,
                r.gateway_model,
                r.endpoint_id,
                r.endpoint_name,
                r.upstream_model,
                r.upstream_protocol,
                r.proxy,
                r.rule,
                r.status,
                r.error,
                r.duration_ms,
                r.ttfb_ms,
                r.stream,
                r.usage.input as i64,
                r.usage.output as i64,
                r.usage.cache_read as i64,
                r.usage.cache_write as i64,
                r.cost,
                serde_json::to_string(&r.attempts)?,
            ],
        )?;
        if let Some(b) = b {
            conn.execute(
                "INSERT OR REPLACE INTO bodies VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    r.id,
                    r.ts,
                    b.client_request,
                    b.client_headers,
                    b.upstream_url,
                    b.upstream_request,
                    b.upstream_headers,
                    b.upstream_status,
                    b.upstream_response,
                    b.client_response
                ],
            )?;
        }
        Ok(())
    }

    fn row(r: &rusqlite::Row) -> rusqlite::Result<LogRecord> {
        let attempts: String = r.get(22)?;
        Ok(LogRecord {
            id: r.get(0)?,
            ts: r.get(1)?,
            client_protocol: r.get(2)?,
            path: r.get(3)?,
            requested_model: r.get(4)?,
            gateway_model: r.get(5)?,
            endpoint_id: r.get(6)?,
            endpoint_name: r.get(7)?,
            upstream_model: r.get(8)?,
            upstream_protocol: r.get(9)?,
            proxy: r.get(10)?,
            rule: r.get(11)?,
            status: r.get(12)?,
            error: r.get(13)?,
            duration_ms: r.get(14)?,
            ttfb_ms: r.get(15)?,
            stream: r.get(16)?,
            usage: Usage {
                input: r.get::<_, i64>(17)? as u64,
                output: r.get::<_, i64>(18)? as u64,
                cache_read: r.get::<_, i64>(19)? as u64,
                cache_write: r.get::<_, i64>(20)? as u64,
            },
            cost: r.get(21)?,
            attempts: serde_json::from_str(&attempts).unwrap_or_default(),
            pending: false,
        })
    }

    pub fn list(&self, q: &LogQuery) -> Result<Vec<LogRecord>> {
        let conn = self.conn.lock();
        let mut sql = String::from("SELECT * FROM requests WHERE ts < ?1");
        let like = format!("%{}%", q.q.trim());
        if !q.q.trim().is_empty() {
            sql.push_str(" AND (requested_model LIKE ?2 OR gateway_model LIKE ?2 OR error LIKE ?2 OR id LIKE ?2)");
        } else {
            sql.push_str(" AND ?2 = ?2");
        }
        if !q.endpoint.is_empty() {
            sql.push_str(" AND endpoint_id = ?3");
        } else {
            sql.push_str(" AND ?3 = ?3");
        }
        match q.status.as_str() {
            "ok" => sql.push_str(" AND status >= 200 AND status < 300"),
            "error" => sql.push_str(" AND (status < 200 OR status >= 300)"),
            _ => {}
        }
        sql.push_str(" ORDER BY ts DESC LIMIT ?4");
        let mut st = conn.prepare(&sql)?;
        let rows = st
            .query_map(
                params![q.before.unwrap_or(i64::MAX), like, q.endpoint, q.limit.unwrap_or(100).min(500)],
                Self::row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get(&self, id: &str) -> Result<Option<(LogRecord, Option<LogBodies>)>> {
        let conn = self.conn.lock();
        let rec = conn.query_row("SELECT * FROM requests WHERE id = ?1", [id], Self::row).optional()?;
        let Some(rec) = rec else { return Ok(None) };
        let bodies = conn
            .query_row(
                "SELECT client_request, client_headers, upstream_url, upstream_request, upstream_headers, upstream_status, upstream_response, client_response FROM bodies WHERE id = ?1",
                [id],
                |r| {
                    Ok(LogBodies {
                        client_request: r.get(0)?,
                        client_headers: r.get(1)?,
                        upstream_url: r.get(2)?,
                        upstream_request: r.get(3)?,
                        upstream_headers: r.get(4)?,
                        upstream_status: r.get(5)?,
                        upstream_response: r.get(6)?,
                        client_response: r.get(7)?,
                    })
                },
            )
            .optional()?;
        Ok(Some((rec, bodies)))
    }

    /// 统计最近 `days` 天；`tz_offset_min` 是本地时区相对 UTC 的分钟数（东八区 = 480）
    pub fn stats(&self, days: u32, tz_offset_min: i32) -> Result<Stats> {
        let since = now_millis() - days as i64 * 86_400_000;
        let conn = self.conn.lock();
        let agg = "COUNT(*), SUM(CASE WHEN status < 200 OR status >= 300 THEN 1 ELSE 0 END),
                   SUM(input_tokens), SUM(output_tokens), SUM(cache_read), SUM(cache_write), SUM(cost), AVG(duration_ms)";
        let map_row = |r: &rusqlite::Row| -> rusqlite::Result<UsageRow> {
            Ok(UsageRow {
                key: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                label: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                requests: r.get::<_, i64>(2)? as u64,
                errors: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                input: r.get::<_, Option<i64>>(4)?.unwrap_or(0) as u64,
                output: r.get::<_, Option<i64>>(5)?.unwrap_or(0) as u64,
                cache_read: r.get::<_, Option<i64>>(6)?.unwrap_or(0) as u64,
                cache_write: r.get::<_, Option<i64>>(7)?.unwrap_or(0) as u64,
                cost: r.get::<_, Option<f64>>(8)?.unwrap_or(0.0),
                avg_ms: r.get::<_, Option<f64>>(9)?.unwrap_or(0.0),
            })
        };
        let total = conn.query_row(&format!("SELECT '', '', {agg} FROM requests WHERE ts >= ?1"), [since], map_row)?;
        let mut st = conn.prepare(&format!(
            "SELECT gateway_model, MAX(endpoint_name), {agg} FROM requests WHERE ts >= ?1 AND gateway_model != '' GROUP BY gateway_model ORDER BY COUNT(*) DESC"
        ))?;
        let by_model = st.query_map([since], map_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut st = conn.prepare(&format!(
            "SELECT endpoint_id, MAX(endpoint_name), {agg} FROM requests WHERE ts >= ?1 AND endpoint_id != '' GROUP BY endpoint_id ORDER BY COUNT(*) DESC"
        ))?;
        let by_endpoint = st.query_map([since], map_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let offset_ms = tz_offset_min as i64 * 60_000;
        let mut st = conn.prepare(
            "SELECT date((ts + ?2) / 1000, 'unixepoch') AS d, COUNT(*),
                    SUM(CASE WHEN status < 200 OR status >= 300 THEN 1 ELSE 0 END),
                    SUM(input_tokens + cache_read + cache_write), SUM(output_tokens), SUM(cost)
             FROM requests WHERE ts >= ?1 GROUP BY d ORDER BY d",
        )?;
        let daily = st
            .query_map(params![since, offset_ms], |r| {
                Ok(DailyRow {
                    day: r.get(0)?,
                    requests: r.get::<_, i64>(1)? as u64,
                    errors: r.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
                    input: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                    output: r.get::<_, Option<i64>>(4)?.unwrap_or(0) as u64,
                    cost: r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Stats { total, by_model, by_endpoint, daily })
    }

    /// 按保留期限清理
    pub fn cleanup(&self, body_hours: u32, stats_days: u32) -> Result<()> {
        let now = now_millis();
        let conn = self.conn.lock();
        conn.execute("DELETE FROM bodies WHERE ts < ?1", [now - body_hours as i64 * 3_600_000])?;
        conn.execute("DELETE FROM requests WHERE ts < ?1", [now - stats_days as i64 * 86_400_000])?;
        Ok(())
    }

    /// 只删失败的请求（状态码不是 2xx，或者带了错误信息），返回删了几条
    pub fn clear_failed(&self) -> Result<usize> {
        let conn = self.conn.lock();
        let cond = "status < 200 OR status >= 300 OR (error IS NOT NULL AND error != '')";
        conn.execute(&format!("DELETE FROM bodies WHERE id IN (SELECT id FROM requests WHERE {cond})"), [])?;
        let n = conn.execute(&format!("DELETE FROM requests WHERE {cond}"), [])?;
        Ok(n)
    }

    pub fn clear(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute_batch("DELETE FROM bodies; DELETE FROM requests; VACUUM;")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_only_failed() {
        let s = Store::memory().unwrap();
        for (id, status, error) in [("ok", 200, ""), ("bad", 502, "上游错误"), ("odd", 200, "流中断")] {
            let r = LogRecord { id: id.into(), ts: now_millis(), status, error: error.into(), ..Default::default() };
            s.insert(&r, Some(&LogBodies::default())).unwrap();
        }
        assert_eq!(s.clear_failed().unwrap(), 2);
        let left: Vec<String> = s.list(&LogQuery::default()).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(left, ["ok"]);
    }

    #[test]
    fn insert_list_stats() {
        let s = Store::memory().unwrap();
        let r = LogRecord {
            id: "a".into(),
            ts: now_millis() - 1000,
            gateway_model: "ds/x".into(),
            endpoint_id: "ds".into(),
            status: 200,
            usage: Usage { input: 10, output: 5, ..Default::default() },
            cost: 0.5,
            ..Default::default()
        };
        s.insert(&r, Some(&LogBodies { client_request: "{}".into(), ..Default::default() })).unwrap();
        assert_eq!(s.list(&LogQuery::default()).unwrap().len(), 1);
        assert!(s.get("a").unwrap().unwrap().1.is_some());
        let st = s.stats(7, 480).unwrap();
        assert_eq!(st.total.requests, 1);
        assert_eq!(st.by_model[0].input, 10);
        assert_eq!(st.daily.len(), 1);
        s.cleanup(0, 90).unwrap();
        assert!(s.get("a").unwrap().unwrap().1.is_none());
    }
}
