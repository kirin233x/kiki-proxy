//! 本地程序托管：由 kiki-proxy 启动端点背后的命令行程序（比如公司内部服务的代理 cc-proxy），
//! 收集它的输出，挂掉了自动重启，App 退出时一起结束。
//!
//! 命令通过登录 shell（`$SHELL -lc "<命令>"`）执行，这样能拿到你在终端里配置的 PATH 和环境变量，
//! 写法和在终端里敲的一样，可以带参数、`cd`、`export`。

use crate::config::Config;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

const MAX_LOG_LINES: usize = 1000;

#[derive(Debug, Clone, PartialEq)]
struct Spec {
    command: String,
    workdir: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStatus {
    /// running / starting / restarting / stopped / failed
    pub state: String,
    pub pid: Option<u32>,
    /// 本次启动时间（毫秒）
    pub started_at: i64,
    /// 自动重启过几次
    pub restarts: u32,
    /// 上一次退出的原因
    pub last_exit: String,
    pub command: String,
    /// 最近的输出（stdout + stderr）
    pub logs: Vec<String>,
}

struct Entry {
    spec: Spec,
    status: ProcessStatus,
    logs: VecDeque<String>,
    stop: watch::Sender<bool>,
}

#[derive(Default)]
pub struct Processes {
    map: Mutex<HashMap<String, Entry>>,
}

fn spec_of(cfg: &Config) -> HashMap<String, Spec> {
    cfg.endpoints
        .iter()
        .filter(|e| e.enabled && e.process.enabled && !e.process.command.trim().is_empty())
        .map(|e| (e.id.clone(), Spec { command: e.process.command.trim().to_string(), workdir: e.process.workdir.trim().to_string() }))
        .collect()
}

fn shell() -> String {
    std::env::var("SHELL").ok().filter(|s| std::path::Path::new(s).exists()).unwrap_or_else(|| {
        if std::path::Path::new("/bin/zsh").exists() {
            "/bin/zsh".into()
        } else {
            "/bin/sh".into()
        }
    })
}

fn expand_home(p: &str) -> String {
    match (p.strip_prefix('~'), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}{rest}"),
        _ => p.to_string(),
    }
}

fn now_hms() -> String {
    let secs = crate::ir::now_secs();
    // 只用来在日志里看个大概，按本机时区换算麻烦，直接用 UTC 偏移 +0
    let s = secs % 86400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// 结束整个进程组（shell 和它拉起的子进程）
fn kill_group(pid: u32, force: bool) {
    #[cfg(unix)]
    unsafe {
        let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
        libc::kill(-(pid as i32), sig);
    }
    #[cfg(not(unix))]
    let _ = (pid, force);
}

impl Processes {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 按配置启动 / 停止 / 重启程序（命令改了会重启）
    pub fn sync(self: &Arc<Self>, cfg: &Config) {
        let want = spec_of(cfg);
        let mut to_start = Vec::new();
        {
            let mut map = self.map.lock();
            // 不再需要的、命令变了的：停掉
            let ids: Vec<String> = map.keys().cloned().collect();
            for id in ids {
                let keep = want.get(&id).is_some_and(|s| *s == map[&id].spec);
                if !keep {
                    if let Some(e) = map.remove(&id) {
                        let _ = e.stop.send(true);
                    }
                }
            }
            for (id, spec) in want {
                if !map.contains_key(&id) {
                    to_start.push((id, spec));
                }
            }
        }
        for (id, spec) in to_start {
            self.spawn_supervisor(id, spec);
        }
    }

    /// 手动重启
    pub fn restart(self: &Arc<Self>, id: &str, cfg: &Config) {
        if let Some(e) = self.map.lock().remove(id) {
            let _ = e.stop.send(true);
        }
        if let Some(spec) = spec_of(cfg).remove(id) {
            self.spawn_supervisor(id.to_string(), spec);
        }
    }

    pub fn status(&self, id: &str) -> Option<ProcessStatus> {
        let map = self.map.lock();
        let e = map.get(id)?;
        let mut st = e.status.clone();
        st.logs = e.logs.iter().cloned().collect();
        Some(st)
    }

    pub fn is_running(&self, id: &str) -> Option<bool> {
        self.map.lock().get(id).map(|e| e.status.state == "running")
    }

    /// App 退出时调用：立刻结束所有程序
    pub fn stop_all(&self) {
        let mut map = self.map.lock();
        for (_, e) in map.drain() {
            let _ = e.stop.send(true);
            if let Some(pid) = e.status.pid {
                kill_group(pid, false);
            }
        }
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.map.lock().get_mut(id) {
            f(e);
        }
    }

    fn log(&self, id: &str, line: String) {
        self.update(id, |e| {
            e.logs.push_back(line);
            while e.logs.len() > MAX_LOG_LINES {
                e.logs.pop_front();
            }
        });
    }

    fn spawn_supervisor(self: &Arc<Self>, id: String, spec: Spec) {
        let (tx, mut rx) = watch::channel(false);
        self.map.lock().insert(
            id.clone(),
            Entry {
                spec: spec.clone(),
                status: ProcessStatus { state: "starting".into(), command: spec.command.clone(), ..Default::default() },
                logs: VecDeque::new(),
                stop: tx,
            },
        );
        let me = self.clone();
        tokio::spawn(async move {
            let mut backoff = 1u64;
            loop {
                if *rx.borrow() {
                    break;
                }
                me.log(&id, format!("[kiki-proxy {}] 启动：{}", now_hms(), spec.command));
                let mut cmd = tokio::process::Command::new(shell());
                cmd.arg("-lc").arg(&spec.command).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
                if !spec.workdir.is_empty() {
                    cmd.current_dir(expand_home(&spec.workdir));
                }
                #[cfg(unix)]
                cmd.process_group(0);
                let started = Instant::now();
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let msg = format!("无法启动：{e}");
                        me.log(&id, format!("[kiki-proxy {}] {msg}", now_hms()));
                        me.update(&id, |en| {
                            en.status.state = "failed".into();
                            en.status.last_exit = msg;
                            en.status.pid = None;
                        });
                        if wait_or_stop(&mut rx, backoff).await {
                            break;
                        }
                        backoff = (backoff * 2).min(30);
                        continue;
                    }
                };
                let pid = child.id();
                me.update(&id, |en| {
                    en.status.state = "running".into();
                    en.status.pid = pid;
                    en.status.started_at = crate::ir::now_millis();
                });
                for out in [child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|s| Box::new(s) as _)]
                    .into_iter()
                    .flatten()
                {
                    let me2 = me.clone();
                    let id2 = id.clone();
                    tokio::spawn(async move {
                        let mut lines = BufReader::new(out).lines();
                        while let Ok(Some(l)) = lines.next_line().await {
                            me2.log(&id2, l);
                        }
                    });
                }

                tokio::select! {
                    r = child.wait() => {
                        let why = match r {
                            Ok(s) if s.success() => "程序自己退出了（退出码 0）".to_string(),
                            Ok(s) => match s.code() {
                                Some(c) => format!("程序退出，退出码 {c}"),
                                None => "程序被信号结束".to_string(),
                            },
                            Err(e) => format!("等待程序时出错：{e}"),
                        };
                        me.log(&id, format!("[kiki-proxy {}] {why}", now_hms()));
                        // 跑了一段时间才挂的，从头开始退避
                        if started.elapsed() > Duration::from_secs(60) {
                            backoff = 1;
                        }
                        me.update(&id, |en| {
                            en.status.state = "restarting".into();
                            en.status.pid = None;
                            en.status.last_exit = why;
                            en.status.restarts += 1;
                        });
                        me.log(&id, format!("[kiki-proxy {}] {backoff} 秒后自动重启", now_hms()));
                        if wait_or_stop(&mut rx, backoff).await {
                            break;
                        }
                        backoff = (backoff * 2).min(30);
                    }
                    _ = rx.changed() => {
                        if let Some(pid) = pid {
                            kill_group(pid, false);
                            if tokio::time::timeout(Duration::from_secs(3), child.wait()).await.is_err() {
                                kill_group(pid, true);
                                let _ = child.wait().await;
                            }
                        }
                        break;
                    }
                }
            }
            me.update(&id, |en| {
                en.status.state = "stopped".into();
                en.status.pid = None;
            });
        });
    }
}

/// 等待 `secs` 秒；期间收到停止信号返回 true
async fn wait_or_stop(rx: &mut watch::Receiver<bool>, secs: u64) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(secs)) => *rx.borrow(),
        _ = rx.changed() => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    fn cfg(cmd: &str) -> Config {
        let mut c = Config::default();
        c.endpoints.push(EndpointConfig {
            id: "local".into(),
            name: "本地".into(),
            base_url: "http://127.0.0.1:1".into(),
            process: ProcessConfig { enabled: true, command: cmd.into(), workdir: String::new() },
            ..Default::default()
        });
        c
    }

    #[tokio::test]
    async fn runs_logs_restarts_and_stops() {
        let p = Processes::new();
        // 输出一行后退出 → 应该被自动重启
        p.sync(&cfg("echo hello-from-child; exit 3"));
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let st = p.status("local").unwrap();
        assert!(st.logs.iter().any(|l| l == "hello-from-child"), "{:?}", st.logs);
        assert!(st.restarts >= 1, "{st:?}");
        assert!(st.last_exit.contains('3'), "{st:?}");

        // 换成长期运行的命令：命令变了会重启成新的
        p.sync(&cfg("echo started; sleep 30"));
        tokio::time::sleep(Duration::from_millis(700)).await;
        let st = p.status("local").unwrap();
        assert_eq!(st.state, "running", "{st:?}");
        let pid = st.pid.unwrap();

        // 配置里去掉 → 停止并结束进程
        p.sync(&Config::default());
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(p.status("local").is_none());
        #[cfg(unix)]
        assert_ne!(unsafe { libc::kill(pid as i32, 0) }, 0, "进程应该已经结束");
    }
}
