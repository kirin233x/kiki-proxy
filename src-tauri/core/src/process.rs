//! 本地程序托管：由 kiki-proxy 启动端点背后的命令行程序（比如公司内部服务的代理 cc-proxy），
//! 收集它的输出，挂掉了自动重启，暂停网关或退出 App 时一起结束。
//!
//! 环境变量：GUI App 拿到的环境和终端里不一样（不会读 ~/.zshrc），所以第一次启动程序前，
//! 先用交互式登录 shell 跑一次 `env`，拿到和终端里一样的环境变量（代理、证书、PATH……），
//! 再加上端点里额外填写的变量，用它们执行命令。

use crate::config::Config;
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

const MAX_LOG_LINES: usize = 1000;
/// 启动后这么短时间内就退出，算一次「很快失败」
const QUICK_EXIT: Duration = Duration::from_secs(60);
/// 连续很快失败这么多次，就不再自动重启
const MAX_QUICK_FAILURES: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
struct Spec {
    command: String,
    workdir: String,
    env: Vec<(String, String)>,
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
    /// 上一次退出 / 停止的原因
    pub last_exit: String,
    pub command: String,
    /// 是否被手动停止（或连续失败后放弃），需要手动「启动」
    pub held: bool,
    /// 网关是否已暂停
    pub paused: bool,
    /// 最近的输出（stdout + stderr）
    pub logs: Vec<String>,
}

struct Entry {
    spec: Spec,
    status: ProcessStatus,
    logs: VecDeque<String>,
    /// 当前守护任务的停止开关；None 表示没有在守护
    stop: Option<watch::Sender<bool>>,
    /// 守护任务的代号，旧任务结束时不覆盖新任务的状态
    gen: u64,
}

#[derive(Default)]
pub struct Processes {
    map: Mutex<HashMap<String, Entry>>,
    /// 最近一次配置里需要运行的程序
    want: Mutex<HashMap<String, Spec>>,
    /// 手动停止的
    held: Mutex<HashSet<String>>,
    /// 连续失败后放弃自动重启的（命令改了会自动恢复）
    gave_up: Mutex<HashSet<String>>,
    /// 网关暂停时所有程序都停
    paused: AtomicBool,
    gen: AtomicU64,
    /// 异步运行时。界面上的操作可能来自没有运行时的线程（比如 macOS 主线程），
    /// 所以在运行时里被调用时记下来，之后都用它启动后台任务
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    /// 终端里的环境变量（第一次用时读取；手动启动 / 重启时重新读取）
    shell_env: tokio::sync::Mutex<Option<Result<HashMap<String, String>, String>>>,
    /// 自动重启等待时间的单位（毫秒），0 表示默认的 1 秒；测试里调小
    backoff_unit_ms: AtomicU64,
}

fn parse_env(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let l = l.strip_prefix("export ").unwrap_or(l);
            let (k, v) = l.split_once('=')?;
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
            let v = v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')).unwrap_or(v);
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

fn spec_of(cfg: &Config) -> HashMap<String, Spec> {
    cfg.endpoints
        .iter()
        .filter(|e| e.enabled && e.process.enabled && !e.process.command.trim().is_empty())
        .map(|e| {
            (
                e.id.clone(),
                Spec {
                    command: e.process.command.trim().to_string(),
                    workdir: e.process.workdir.trim().to_string(),
                    env: parse_env(&e.process.env),
                },
            )
        })
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
    // 日志里看个大概时间就行，按 UTC
    let s = crate::ir::now_secs() % 86400;
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

/// 用交互式登录 shell 读取终端里的环境变量（会读 ~/.zshrc）
async fn capture_shell_env() -> Result<HashMap<String, String>, String> {
    const MARK: &str = "__KIKI_PROXY_ENV__";
    let out = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(shell())
            .arg("-ilc")
            .arg(format!("printf '\\n{MARK}\\n'; env -0"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "读取超时（10 秒）".to_string())?
    .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let body = text.split_once(&format!("{MARK}\n")).map(|(_, b)| b).ok_or("没有拿到输出")?;
    let skip = ["PWD", "OLDPWD", "SHLVL", "_"];
    Ok(body
        .split('\0')
        .filter_map(|kv| kv.split_once('='))
        .filter(|(k, _)| !k.is_empty() && !skip.contains(k))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect())
}

/// 和网络 / 证书有关的变量名，用来在日志里提示
fn network_vars<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut v: Vec<String> = keys
        .filter(|k| {
            let l = k.to_lowercase();
            l.contains("proxy") || l.contains("ssl") || l.contains("cert") || l == "godebug" || l.contains("ca_bundle") || l.contains("extra_ca")
        })
        .cloned()
        .collect();
    v.sort();
    v.dedup();
    v
}

impl Processes {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    #[cfg(test)]
    fn with_backoff_unit(ms: u64) -> Arc<Self> {
        let p = Self::default();
        p.backoff_unit_ms.store(ms, Ordering::SeqCst);
        Arc::new(p)
    }

    fn unit(&self) -> Duration {
        match self.backoff_unit_ms.load(Ordering::SeqCst) {
            0 => Duration::from_secs(1),
            ms => Duration::from_millis(ms),
        }
    }

    fn runtime(&self) -> Option<tokio::runtime::Handle> {
        let mut rt = self.runtime.lock();
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            *rt = Some(h);
        }
        rt.clone()
    }

    /// 配置变了：记下需要运行的程序，然后对齐
    pub fn sync(self: &Arc<Self>, cfg: &Config) {
        let want = spec_of(cfg);
        {
            // 命令改了：之前因为连续失败放弃的，给一次新机会
            let old = self.want.lock();
            let mut gave_up = self.gave_up.lock();
            gave_up.retain(|id| old.get(id) == want.get(id));
        }
        *self.want.lock() = want;
        self.apply();
    }

    /// 让实际运行的程序和「应该运行的」一致
    fn apply(self: &Arc<Self>) {
        let want = self.want.lock().clone();
        let paused = self.paused.load(Ordering::SeqCst);
        let held = self.held.lock().clone();
        let gave_up = self.gave_up.lock().clone();
        let mut to_start = Vec::new();
        {
            let mut map = self.map.lock();
            // 不再需要的：停掉并移除
            map.retain(|id, e| {
                if want.contains_key(id) {
                    return true;
                }
                if let Some(s) = e.stop.take() {
                    let _ = s.send(true);
                }
                false
            });
            for (id, spec) in &want {
                let blocked = if paused {
                    Some("网关已暂停")
                } else if held.contains(id) {
                    Some("已手动停止")
                } else if gave_up.contains(id) {
                    Some("")
                } else {
                    None
                };
                let entry = map.get_mut(id);
                match (blocked, entry) {
                    // 不该运行：正在运行的停掉
                    (Some(why), Some(e)) => {
                        if let Some(s) = e.stop.take() {
                            let _ = s.send(true);
                            if !why.is_empty() {
                                e.status.last_exit = why.to_string();
                            }
                        }
                    }
                    (Some(why), None) => {
                        map.insert(id.clone(), Entry::idle(spec.clone(), why));
                    }
                    // 该运行：没在守护，或者命令变了，就（重新）启动
                    (None, Some(e)) => {
                        if e.stop.is_none() || e.spec != *spec {
                            if let Some(s) = e.stop.take() {
                                let _ = s.send(true);
                            }
                            to_start.push((id.clone(), spec.clone()));
                        }
                    }
                    (None, None) => to_start.push((id.clone(), spec.clone())),
                }
            }
        }
        for (id, spec) in to_start {
            self.spawn_supervisor(id, spec);
        }
    }

    /// 手动停止，直到手动启动
    pub fn stop(self: &Arc<Self>, id: &str) {
        self.held.lock().insert(id.to_string());
        if let Some(e) = self.map.lock().get_mut(id) {
            if let Some(s) = e.stop.take() {
                let _ = s.send(true);
            }
            e.status.last_exit = "已手动停止".into();
            e.logs.push_back(format!("[kiki-proxy {}] 手动停止", now_hms()));
        }
    }

    /// 手动启动（也会清除「连续失败后放弃」的状态，并重新读取终端环境变量）
    pub fn start(self: &Arc<Self>, id: &str) -> Result<()> {
        if self.paused.load(Ordering::SeqCst) {
            return Err(anyhow!("网关已暂停，请先启动网关"));
        }
        if !self.want.lock().contains_key(id) {
            return Err(anyhow!("这个端点没有启用本地程序，或者启动命令为空"));
        }
        self.held.lock().remove(id);
        self.gave_up.lock().remove(id);
        self.forget_shell_env();
        self.apply();
        Ok(())
    }

    /// 手动重启
    pub fn restart(self: &Arc<Self>, id: &str) -> Result<()> {
        if let Some(e) = self.map.lock().get_mut(id) {
            if let Some(s) = e.stop.take() {
                let _ = s.send(true);
            }
        }
        self.start(id)
    }

    /// 网关暂停：所有程序都停（保留日志）
    pub fn pause(self: &Arc<Self>) {
        self.paused.store(true, Ordering::SeqCst);
        self.apply();
    }

    /// 网关启动：恢复运行（手动停止的除外）
    pub fn resume(self: &Arc<Self>) {
        self.paused.store(false, Ordering::SeqCst);
        self.apply();
    }

    /// App 退出时调用：立刻结束所有程序（不依赖异步运行时）
    pub fn stop_all(&self) {
        self.paused.store(true, Ordering::SeqCst);
        let mut map = self.map.lock();
        for (_, e) in map.drain() {
            if let Some(s) = e.stop {
                let _ = s.send(true);
            }
            if let Some(pid) = e.status.pid {
                kill_group(pid, false);
            }
        }
    }

    pub fn status(&self, id: &str) -> Option<ProcessStatus> {
        let map = self.map.lock();
        let e = map.get(id)?;
        let mut st = e.status.clone();
        st.held = self.held.lock().contains(id) || self.gave_up.lock().contains(id);
        st.paused = self.paused.load(Ordering::SeqCst);
        st.logs = e.logs.iter().cloned().collect();
        Some(st)
    }

    fn forget_shell_env(&self) {
        if let Ok(mut g) = self.shell_env.try_lock() {
            *g = None;
        }
    }

    async fn shell_env(&self) -> Result<HashMap<String, String>, String> {
        let mut g = self.shell_env.lock().await;
        if g.is_none() {
            *g = Some(capture_shell_env().await);
        }
        g.clone().unwrap()
    }

    /// 只更新属于这一代守护任务的状态
    fn update(&self, id: &str, gen: u64, f: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.map.lock().get_mut(id) {
            if e.gen == gen {
                f(e);
            }
        }
    }

    fn log(&self, id: &str, line: String) {
        if let Some(e) = self.map.lock().get_mut(id) {
            e.logs.push_back(line);
            while e.logs.len() > MAX_LOG_LINES {
                e.logs.pop_front();
            }
        }
    }

    fn spawn_supervisor(self: &Arc<Self>, id: String, spec: Spec) {
        let Some(rt) = self.runtime() else {
            tracing::error!("没有可用的异步运行时，无法启动本地程序「{id}」");
            return;
        };
        let (tx, mut rx) = watch::channel(false);
        let gen = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut map = self.map.lock();
            let e = map.entry(id.clone()).or_insert_with(|| Entry::idle(spec.clone(), ""));
            e.spec = spec.clone();
            e.stop = Some(tx);
            e.gen = gen;
            e.status = ProcessStatus { state: "starting".into(), command: spec.command.clone(), ..Default::default() };
        }
        let me = self.clone();
        rt.spawn(async move {
            let mut backoff = 1u64;
            let mut quick_failures = 0u32;
            let mut restarts = 0u32;
            loop {
                if *rx.borrow() {
                    break;
                }
                // 和终端里一样的环境变量 + 端点里额外填写的
                let mut envs: HashMap<String, String> = HashMap::new();
                match me.shell_env().await {
                    Ok(env) => {
                        envs.extend(env);
                        envs.extend(spec.env.iter().cloned());
                        let net = network_vars(envs.keys());
                        me.log(
                            &id,
                            format!(
                                "[kiki-proxy {}] 已载入终端环境变量 {} 个；{}",
                                now_hms(),
                                envs.len(),
                                if net.is_empty() { "没有代理 / 证书相关的变量".to_string() } else { format!("代理 / 证书相关：{}", net.join("、")) }
                            ),
                        );
                    }
                    Err(e) => {
                        envs.extend(spec.env.iter().cloned());
                        me.log(&id, format!("[kiki-proxy {}] 读取终端环境变量失败（{e}），只用登录 shell 的环境", now_hms()));
                    }
                }
                me.log(&id, format!("[kiki-proxy {}] 启动：{}", now_hms(), spec.command));
                let mut cmd = tokio::process::Command::new(shell());
                cmd.arg("-lc")
                    .arg(&spec.command)
                    .envs(&envs)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                if !spec.workdir.is_empty() {
                    cmd.current_dir(expand_home(&spec.workdir));
                } else if let Ok(h) = std::env::var("HOME") {
                    cmd.current_dir(h);
                }
                #[cfg(unix)]
                cmd.process_group(0);
                let started = Instant::now();
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let msg = format!("无法启动：{e}");
                        me.log(&id, format!("[kiki-proxy {}] {msg}", now_hms()));
                        me.update(&id, gen, |en| {
                            en.status.state = "failed".into();
                            en.status.last_exit = msg;
                            en.status.pid = None;
                        });
                        if wait_or_stop(&mut rx, me.unit() * backoff as u32).await {
                            break;
                        }
                        backoff = (backoff * 2).min(30);
                        continue;
                    }
                };
                let pid = child.id();
                me.update(&id, gen, |en| {
                    en.status.state = "running".into();
                    en.status.pid = pid;
                    en.status.started_at = crate::ir::now_millis();
                    en.status.restarts = restarts;
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
                        if started.elapsed() > QUICK_EXIT {
                            backoff = 1;
                            quick_failures = 0;
                        } else {
                            quick_failures += 1;
                        }
                        // 一直启动就挂：不再反复重启，等用户处理
                        if quick_failures >= MAX_QUICK_FAILURES {
                            let msg = format!("连续 {quick_failures} 次启动后很快退出，已停止自动重启（{why}）。看看上面的输出，修好后点「启动」");
                            me.log(&id, format!("[kiki-proxy {}] {msg}", now_hms()));
                            me.gave_up.lock().insert(id.clone());
                            me.update(&id, gen, |en| {
                                en.status.state = "failed".into();
                                en.status.pid = None;
                                en.status.last_exit = msg;
                                en.stop = None;
                            });
                            return;
                        }
                        restarts += 1;
                        me.update(&id, gen, |en| {
                            en.status.state = "restarting".into();
                            en.status.pid = None;
                            en.status.last_exit = why;
                            en.status.restarts = restarts;
                        });
                        me.log(&id, format!("[kiki-proxy {}] {backoff} 秒后自动重启", now_hms()));
                        if wait_or_stop(&mut rx, me.unit() * backoff as u32).await {
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
            me.update(&id, gen, |en| {
                en.status.state = "stopped".into();
                en.status.pid = None;
                en.stop = None;
            });
        });
    }
}

impl Entry {
    fn idle(spec: Spec, why: &str) -> Self {
        Entry {
            status: ProcessStatus { state: "stopped".into(), command: spec.command.clone(), last_exit: why.into(), ..Default::default() },
            spec,
            logs: VecDeque::new(),
            stop: None,
            gen: 0,
        }
    }
}

/// 等待一段时间；期间收到停止信号返回 true
async fn wait_or_stop(rx: &mut watch::Receiver<bool>, d: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => *rx.borrow(),
        _ = rx.changed() => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    fn cfg(cmd: &str) -> Config {
        cfg_env(cmd, "")
    }

    fn cfg_env(cmd: &str, env: &str) -> Config {
        let mut c = Config::default();
        c.endpoints.push(EndpointConfig {
            id: "local".into(),
            name: "本地".into(),
            base_url: "http://127.0.0.1:1".into(),
            process: ProcessConfig { enabled: true, command: cmd.into(), workdir: String::new(), env: env.into() },
            ..Default::default()
        });
        c
    }

    async fn wait_state(p: &Processes, state: &str) -> ProcessStatus {
        for _ in 0..50 {
            if let Some(st) = p.status("local") {
                if st.state == state {
                    return st;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("等不到状态 {state}：{:?}", p.status("local"));
    }

    fn alive(pid: u32) -> bool {
        #[cfg(unix)]
        return unsafe { libc::kill(pid as i32, 0) } == 0;
        #[cfg(not(unix))]
        return { let _ = pid; false };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runs_logs_restarts_and_stops() {
        let p = Processes::new();
        // 输出一行后退出 → 应该被自动重启
        p.sync(&cfg("echo hello-from-child; exit 3"));
        tokio::time::sleep(Duration::from_millis(1800)).await;
        let st = p.status("local").unwrap();
        assert!(st.logs.iter().any(|l| l == "hello-from-child"), "{:?}", st.logs);
        assert!(st.restarts >= 1, "{st:?}");
        assert!(st.last_exit.contains('3'), "{st:?}");

        // 换成长期运行的命令：命令变了会重启成新的
        p.sync(&cfg("echo started; sleep 30"));
        let st = wait_state(&p, "running").await;
        let pid = st.pid.unwrap();

        // 配置里去掉 → 停止并结束进程
        p.sync(&Config::default());
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(p.status("local").is_none());
        assert!(!alive(pid), "进程应该已经结束");
    }

    /// 回归测试：界面上的操作可能在没有异步运行时的线程上调用（macOS 主线程），不能 panic
    #[tokio::test(flavor = "multi_thread")]
    async fn restart_from_plain_thread() {
        let p = Processes::new();
        p.sync(&cfg("echo started; sleep 30"));
        wait_state(&p, "running").await;
        let p2 = p.clone();
        std::thread::spawn(move || p2.restart("local").unwrap()).join().expect("在普通线程上重启不应该 panic");
        tokio::time::sleep(Duration::from_millis(300)).await;
        wait_state(&p, "running").await;
        p.stop_all();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn manual_stop_survives_config_save_until_started() {
        let p = Processes::new();
        let c = cfg("sleep 30");
        p.sync(&c);
        let pid = wait_state(&p, "running").await.pid.unwrap();
        p.stop("local");
        let st = wait_state(&p, "stopped").await;
        assert!(st.held);
        assert!(!alive(pid));
        // 保存配置不会把手动停止的程序拉起来
        p.sync(&c);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(p.status("local").unwrap().state, "stopped");
        // 手动启动
        p.start("local").unwrap();
        let st = wait_state(&p, "running").await;
        assert!(!st.held);
        p.stop_all();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pause_and_resume_with_gateway() {
        let p = Processes::new();
        p.sync(&cfg("sleep 30"));
        let pid = wait_state(&p, "running").await.pid.unwrap();
        p.pause();
        let st = wait_state(&p, "stopped").await;
        assert!(st.paused);
        assert!(!alive(pid));
        assert!(p.start("local").is_err(), "网关暂停时不能单独启动");
        p.resume();
        wait_state(&p, "running").await;
        p.stop_all();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gives_up_after_quick_failures() {
        let p = Processes::with_backoff_unit(20);
        p.sync(&cfg("exit 7"));
        let st = wait_state(&p, "failed").await;
        assert!(st.held);
        assert!(st.last_exit.contains("已停止自动重启"), "{st:?}");
        // 命令改了：重新尝试
        p.sync(&cfg("sleep 30"));
        wait_state(&p, "running").await;
        p.stop_all();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn extra_env_is_passed() {
        let p = Processes::new();
        p.sync(&cfg_env("echo value=$KIKI_TEST_VAR; sleep 30", "export KIKI_TEST_VAR=\"hello world\"\n# 注释\n"));
        wait_state(&p, "running").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let st = p.status("local").unwrap();
        assert!(st.logs.iter().any(|l| l == "value=hello world"), "{:?}", st.logs);
        assert!(st.logs.iter().any(|l| l.contains("已载入终端环境变量")), "{:?}", st.logs);
        p.stop_all();
    }
}
