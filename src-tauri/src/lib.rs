//! Tauri 外壳：钥匙串、插件弹窗、菜单栏图标，以及给前端调用的命令。

use kiki_core::clients::{self, ApplyOptions, ClientInfo};
use kiki_core::config::Config;
use kiki_core::gateway::{ServerStatus, TestResult};
use kiki_core::process::ProcessStatus;
use kiki_core::secrets::SecretStore;
use kiki_core::router::ListedModel;
use kiki_core::store::{LogBodies, LogQuery, LogRecord, Stats};
use kiki_core::Gateway;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ------------------------------------------------------------------ 钥匙串

const KEYCHAIN_SERVICE: &str = "kiki-proxy";

/// macOS 钥匙串（其他平台上 keyring 用各自的系统存储）
struct Keychain;

impl SecretStore for Keychain {
    fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        match keyring::Entry::new(KEYCHAIN_SERVICE, key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        keyring::Entry::new(KEYCHAIN_SERVICE, key)?.set_password(value)?;
        Ok(())
    }
    fn delete(&self, key: &str) -> anyhow::Result<()> {
        match keyring::Entry::new(KEYCHAIN_SERVICE, key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

struct AppState {
    gateway: Gateway,
}

// ------------------------------------------------------------------ 配置与服务

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    (*state.gateway.config()).clone()
}

#[tauri::command]
async fn save_config(app: AppHandle, state: State<'_, AppState>, config: Config) -> CmdResult<()> {
    state.gateway.save_config(config).await.map_err(err)?;
    refresh_tray(&app);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    git_hash: &'static str,
    /// 构建时间（Unix 秒）
    build_time: u64,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        git_hash: env!("KIKI_GIT_HASH"),
        build_time: env!("KIKI_BUILD_TIME").parse().unwrap_or(0),
    }
}

#[tauri::command]
fn get_status(state: State<AppState>) -> ServerStatus {
    state.gateway.status()
}

#[tauri::command]
async fn start_server(app: AppHandle, state: State<'_, AppState>) -> CmdResult<ServerStatus> {
    let r = state.gateway.start().await;
    refresh_tray(&app);
    r.map_err(err)?;
    Ok(state.gateway.status())
}

#[tauri::command]
async fn stop_server(app: AppHandle, state: State<'_, AppState>) -> CmdResult<ServerStatus> {
    state.gateway.stop().await;
    refresh_tray(&app);
    Ok(state.gateway.status())
}

#[tauri::command]
fn export_config(state: State<AppState>) -> CmdResult<String> {
    state.gateway.config().to_toml().map_err(err)
}

#[tauri::command]
async fn import_config(state: State<'_, AppState>, toml: String) -> CmdResult<Config> {
    let cfg = Config::from_toml(&toml).map_err(err)?;
    state.gateway.save_config(cfg.clone()).await.map_err(err)?;
    Ok(cfg)
}

#[tauri::command]
fn reveal_config(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    let p = state.gateway.config_path().to_path_buf();
    app.opener().reveal_item_in_dir(p).map_err(err)
}

#[tauri::command]
fn explain_route(state: State<AppState>, model: String, client: kiki_core::ir::Protocol) -> CmdResult<Vec<kiki_core::router::RouteHop>> {
    kiki_core::router::explain(&state.gateway.config(), &model, client).map_err(err)
}

#[tauri::command]
fn list_models(state: State<AppState>) -> Vec<ListedModel> {
    kiki_core::router::list_models(&state.gateway.config())
}

// ------------------------------------------------------------------ 密钥（只写不读，界面上只显示「已设置」）

#[tauri::command]
fn secret_flags(state: State<AppState>, keys: Vec<String>) -> HashMap<String, bool> {
    keys.into_iter()
        .map(|k| {
            let has = state.gateway.secrets().get(&k).ok().flatten().is_some_and(|v| !v.is_empty());
            (k, has)
        })
        .collect()
}

#[tauri::command]
fn set_secret(state: State<AppState>, key: String, value: String) -> CmdResult<()> {
    let s = state.gateway.secrets();
    if value.is_empty() {
        s.delete(&key).map_err(err)?;
    } else {
        s.set(&key, &value).map_err(err)?;
    }
    state.gateway.secrets_changed();
    Ok(())
}

// ------------------------------------------------------------------ 本地程序

#[tauri::command]
fn process_status(state: State<AppState>, id: String) -> Option<ProcessStatus> {
    state.gateway.process_status(&id)
}

// 异步命令：在异步运行时里执行（同步命令跑在 macOS 主线程上，那里没有运行时）
#[tauri::command]
async fn restart_process(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.gateway.restart_process(&id).map_err(err)
}

#[tauri::command]
async fn start_process(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.gateway.start_process(&id).map_err(err)
}

#[tauri::command]
async fn stop_process(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.gateway.stop_process(&id);
    Ok(())
}

// ------------------------------------------------------------------ 价格表与上游模型

#[tauri::command]
fn pricing_status(state: State<AppState>) -> kiki_core::pricing::PricingStatus {
    state.gateway.pricing_status()
}

#[tauri::command]
async fn refresh_pricing(state: State<'_, AppState>) -> CmdResult<kiki_core::pricing::PricingStatus> {
    state.gateway.refresh_pricing().await.map_err(err)
}

#[tauri::command]
async fn lookup_models(
    state: State<'_, AppState>,
    models: Vec<String>,
    base_url: String,
    protocol: kiki_core::ir::Protocol,
) -> CmdResult<Vec<Option<kiki_core::pricing::ModelInfo>>> {
    state.gateway.lookup_models(&models, &base_url, protocol).await.map_err(err)
}

#[tauri::command]
async fn list_upstream_models(state: State<'_, AppState>, id: String) -> CmdResult<Vec<String>> {
    state.gateway.list_upstream_models(&id).await.map_err(err)
}

// ------------------------------------------------------------------ 测试

#[tauri::command]
async fn test_endpoint(state: State<'_, AppState>, id: String, model: Option<String>) -> CmdResult<TestResult> {
    Ok(state.gateway.test_endpoint(&id, model).await)
}

#[tauri::command]
async fn test_endpoint_report(state: State<'_, AppState>, id: String) -> CmdResult<kiki_core::gateway::TestReport> {
    Ok(state.gateway.test_endpoint_report(&id).await)
}

#[tauri::command]
async fn test_step(state: State<'_, AppState>, id: String, step: String, model: Option<String>) -> CmdResult<kiki_core::gateway::TestStep> {
    let g = &state.gateway;
    Ok(match step.as_str() {
        "network" => g.test_step_network(&id).await,
        "auth" => g.test_step_auth(&id).await,
        _ => g.test_step_model(&id, model.as_deref().unwrap_or("")).await,
    })
}

#[tauri::command]
async fn test_proxy(state: State<'_, AppState>, id: String) -> CmdResult<TestResult> {
    Ok(state.gateway.test_proxy(&id).await)
}

// ------------------------------------------------------------------ 日志与统计

#[tauri::command]
async fn list_logs(state: State<'_, AppState>, query: LogQuery) -> CmdResult<Vec<LogRecord>> {
    let g = state.gateway.clone();
    tokio::task::spawn_blocking(move || g.store().list(&query)).await.map_err(err)?.map_err(err)
}

#[derive(Serialize)]
struct LogDetail {
    record: LogRecord,
    bodies: Option<LogBodies>,
}

#[tauri::command]
async fn get_log(state: State<'_, AppState>, id: String) -> CmdResult<Option<LogDetail>> {
    let g = state.gateway.clone();
    let r = tokio::task::spawn_blocking(move || g.store().get(&id)).await.map_err(err)?.map_err(err)?;
    Ok(r.map(|(record, bodies)| LogDetail { record, bodies }))
}

#[tauri::command]
async fn clear_logs(state: State<'_, AppState>) -> CmdResult<()> {
    let g = state.gateway.clone();
    tokio::task::spawn_blocking(move || g.store().clear()).await.map_err(err)?.map_err(err)
}

#[tauri::command]
async fn get_stats(state: State<'_, AppState>, days: u32, tz_offset_min: i32) -> CmdResult<Stats> {
    let g = state.gateway.clone();
    tokio::task::spawn_blocking(move || g.store().stats(days, tz_offset_min)).await.map_err(err)?.map_err(err)
}

// ------------------------------------------------------------------ 客户端接入

#[tauri::command]
fn list_clients(state: State<AppState>) -> Vec<ClientInfo> {
    clients::list(state.gateway.config().server.port)
}

#[tauri::command]
fn apply_client(state: State<AppState>, client: String, options: ApplyOptions) -> CmdResult<clients::ApplyReport> {
    clients::apply(&client, &state.gateway.config(), &options).map_err(err)
}

#[tauri::command]
fn export_desktop_config(app: AppHandle, state: State<AppState>, models: Vec<clients::DesktopModel>) -> CmdResult<String> {
    let cfg = state.gateway.config();
    let p = clients::write_desktop_config(cfg.server.port, &cfg.server.access_key, &models).map_err(err)?;
    let _ = app.opener().reveal_item_in_dir(&p);
    Ok(p.display().to_string())
}

#[tauri::command]
fn inspect_claude(state: State<AppState>) -> clients::ClaudeInspection {
    clients::inspect_claude(&state.gateway.config())
}

#[tauri::command]
fn desktop_connected(state: State<AppState>) -> bool {
    clients::desktop_connected(state.gateway.config().server.port)
}

#[tauri::command]
fn restore_client(client: String) -> CmdResult<()> {
    clients::restore(&client).map_err(err)
}

// ------------------------------------------------------------------ 开机启动

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<()> {
    let m = app.autolaunch();
    if enabled {
        m.enable().map_err(err)
    } else {
        m.disable().map_err(err)
    }
}

// ------------------------------------------------------------------ 菜单栏

const TRAY_ID: &str = "kiki-tray";

fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let status = app.try_state::<AppState>().map(|s| s.gateway.status()).unwrap_or_default();
    let label = if status.running {
        format!("● 运行中 · 127.0.0.1:{}", status.port)
    } else if let Some(e) = &status.error {
        format!("○ 未运行：{e}")
    } else {
        "○ 已暂停".to_string()
    };
    let status_item = MenuItem::with_id(app, "status", label, false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", if status.running { "暂停网关" } else { "启动网关" }, true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "打开 kiki-proxy", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, Some("CmdOrCtrl+Q"))?;
    Menu::with_items(app, &[&status_item, &PredefinedMenuItem::separator(app)?, &open, &toggle, &PredefinedMenuItem::separator(app)?, &quit])
}

fn refresh_tray(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), tray_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = app.emit("status", app.try_state::<AppState>().map(|s| s.gateway.status()));
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip("kiki-proxy")
        .menu(&tray_menu(app)?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => show_main(app),
            "quit" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(s) = app.try_state::<AppState>() {
                        s.gateway.shutdown_processes();
                        s.gateway.stop().await;
                    }
                    app.exit(0);
                });
            }
            "toggle" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(s) = app.try_state::<AppState>() {
                        if s.gateway.status().running {
                            s.gateway.stop().await;
                        } else {
                            let _ = s.gateway.start().await;
                        }
                    }
                    refresh_tray(&app);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = ev {
                show_main(tray.app_handle());
            }
            if let TrayIconEvent::Click { button: MouseButton::Right, button_state: MouseButtonState::Up, .. } = ev {
                refresh_tray(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

// ------------------------------------------------------------------ 入口

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .setup(|app| {
            let handle = app.handle().clone();
            let dir = app.path().app_data_dir()?;
            let gateway = Gateway::new(&dir, Arc::new(Keychain)).map_err(|e| e.to_string())?;
            app.manage(AppState { gateway: gateway.clone() });

            // 实时日志推给前端
            let mut rx = gateway.subscribe();
            let h = handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(rec) => {
                            let _ = h.emit("log", rec);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });

            let h = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = gateway.start().await {
                    eprintln!("网关启动失败：{e}");
                }
                refresh_tray(&h);
            });

            setup_tray(&handle)?;

            // 开机自启时不弹窗口，安静地待在菜单栏
            if std::env::args().any(|a| a == "--background") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关窗口只是隐藏，网关继续在菜单栏运行
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_status,
            app_info,
            start_server,
            stop_server,
            export_config,
            import_config,
            reveal_config,
            list_models,
            explain_route,
            secret_flags,
            set_secret,
            test_endpoint,
            test_endpoint_report,
            test_step,
            test_proxy,
            pricing_status,
            refresh_pricing,
            lookup_models,
            list_upstream_models,
            list_logs,
            get_log,
            clear_logs,
            get_stats,
            list_clients,
            apply_client,
            restore_client,
            export_desktop_config,
            desktop_connected,
            inspect_claude,
            process_status,
            restart_process,
            start_process,
            stop_process,
            get_autostart,
            set_autostart,
        ])
        .build(tauri::generate_context!())
        .expect("启动 kiki-proxy 失败")
        .run(|app, event| {
            // macOS 上点 Dock 图标时重新显示窗口
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app);
            }
            // 不管怎么退出（菜单、Cmd+Q、系统关机），都把托管的本地程序一起结束
            if let tauri::RunEvent::Exit = event {
                if let Some(s) = app.try_state::<AppState>() {
                    s.gateway.shutdown_processes();
                }
            }
            let _ = (app, event);
        });
}
