//! Tauri commands. Every command returns typed, human-readable errors (`UserError`).

use kittytooth_core::config::{Config, ConfigStore};
use kittytooth_core::engine::{HistoryPoint, Snapshot};
use kittytooth_core::osc::sender::OscLogEntry;
use kittytooth_core::sensor::DiscoveredDevice;
use kittytooth_core::{KtError, UserError};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

use crate::state::{self, AppState};
use crate::{tray, windows};

type CmdResult<T> = Result<T, UserError>;

fn err(e: KtError) -> UserError {
    e.user()
}

#[derive(Serialize)]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub overlay_url: Option<String>,
    pub overlay_error: Option<UserError>,
    pub log_dir: String,
    pub config_path: String,
    pub notice: Option<UserError>,
}

#[derive(Serialize)]
pub struct InitialState {
    pub snapshot: Snapshot,
    pub config: Config,
    pub devices: Vec<DiscoveredDevice>,
    pub info: AppInfo,
}

async fn app_info(state: &AppState) -> AppInfo {
    let o = state.overlay.lock().await;
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        overlay_url: o.server.as_ref().map(|s| s.url()),
        overlay_error: o.error.clone(),
        log_dir: state.logging.dir.display().to_string(),
        config_path: state.store.path().display().to_string(),
        notice: state.startup_notice.lock().ok().and_then(|mut n| n.take()),
    }
}

#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> CmdResult<InitialState> {
    Ok(InitialState {
        snapshot: state.engine.snapshot(),
        config: state.engine.config(),
        devices: state.engine.devices(),
        info: app_info(&state).await,
    })
}

/// Applies a config and its OS side effects. Returns the sanitized config and fresh info.
async fn apply_config(app: &AppHandle, state: &AppState, cfg: Config) -> CmdResult<(Config, AppInfo)> {
    let old = state.engine.config();
    state.engine.set_config(cfg).await.map_err(err)?;
    let new = state.engine.config();
    if old.debug.log_latency != new.debug.log_latency {
        state.logging.set_debug(new.debug.log_latency);
    }
    if old.ui.autostart != new.ui.autostart {
        state::sync_autostart(app, new.ui.autostart);
    }
    if old.overlay != new.overlay {
        state::apply_overlay(app, &new).await;
    }
    if old.ui.mini_always_on_top != new.ui.mini_always_on_top {
        if let Some(w) = app.get_webview_window("mini") {
            let _ = w.set_always_on_top(new.ui.mini_always_on_top);
        }
    }
    if old.ui.accent != new.ui.accent {
        tray::refresh(app);
    }
    Ok((new, app_info(state).await))
}

#[derive(Serialize)]
pub struct ConfigResult {
    pub config: Config,
    pub info: AppInfo,
}

#[tauri::command]
pub async fn set_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Config,
) -> CmdResult<ConfigResult> {
    let (config, info) = apply_config(&app, &state, config).await?;
    Ok(ConfigResult { config, info })
}

#[tauri::command]
pub async fn start_scan(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.start_scan().await.map_err(err)
}

#[tauri::command]
pub async fn stop_scan(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.stop_scan().await.map_err(err)
}

#[tauri::command]
pub async fn connect(state: State<'_, AppState>, id: String, name: String) -> CmdResult<()> {
    if id.trim().is_empty() {
        return Err(err(KtError::InvalidInput("no device selected".into())));
    }
    state.engine.connect(&id, &name).await.map_err(err)
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.disconnect().await.map_err(err)
}

#[tauri::command]
pub async fn forget(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.forget().await.map_err(err)
}

#[tauri::command]
pub async fn test_sweep(state: State<'_, AppState>, on: bool) -> CmdResult<()> {
    state.engine.test_sweep(on).await.map_err(err)
}

#[tauri::command]
pub async fn reset_session(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.reset_session().await.map_err(err)
}

#[tauri::command]
pub fn get_history(state: State<'_, AppState>, since_ms: u64) -> Vec<HistoryPoint> {
    state.engine.history(since_ms)
}

#[tauri::command]
pub fn get_osc_log(state: State<'_, AppState>) -> Vec<OscLogEntry> {
    state.engine.osc_log()
}

#[tauri::command]
pub async fn export_config(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<String>> {
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("KittyTooth settings", &["json"])
        .set_file_name("kittytooth-settings.json")
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| err(KtError::Io(e.to_string())))?;
    let text = serde_json::to_string_pretty(&state.engine.config()).map_err(|e| err(e.into()))?;
    std::fs::write(&path, text).map_err(|e| err(e.into()))?;
    tracing::info!(path = %path.display(), "settings exported");
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
pub async fn import_config(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<ConfigResult>> {
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("KittyTooth settings", &["json"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| err(KtError::Io(e.to_string())))?;
    let text = std::fs::read_to_string(&path).map_err(|e| err(e.into()))?;
    let (cfg, _) = ConfigStore::parse(&text).map_err(err)?;
    let (config, info) = apply_config(&app, &state, cfg).await?;
    tracing::info!(path = %path.display(), "settings imported");
    Ok(Some(ConfigResult { config, info }))
}

#[tauri::command]
pub async fn reset_config(app: AppHandle, state: State<'_, AppState>) -> CmdResult<ConfigResult> {
    state.engine.forget().await.map_err(err)?;
    let (config, info) = apply_config(&app, &state, Config::default()).await?;
    tracing::info!("settings reset to defaults");
    Ok(ConfigResult { config, info })
}

#[tauri::command]
pub async fn copy_diagnostics(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let snap = state.engine.snapshot();
    let cfg = state.engine.config();
    let info = app_info(&state).await;
    let text = format!(
        "KittyTooth diagnostics\n\
         ======================\n\
         version: {}\nplatform: {}\nconfig: {}\nlogs: {}\noverlay: {}\n\n\
         -- engine snapshot --\n{}\n\n-- config --\n{}\n\n-- recent log --\n{}\n",
        info.version,
        info.platform,
        info.config_path,
        info.log_dir,
        info.overlay_url.as_deref().unwrap_or("off"),
        serde_json::to_string_pretty(&snap).unwrap_or_default(),
        serde_json::to_string_pretty(&cfg).unwrap_or_default(),
        state.logging.tail(150),
    );
    app.clipboard()
        .write_text(text.clone())
        .map_err(|e| err(KtError::Io(e.to_string())))?;
    Ok(text)
}

#[tauri::command]
pub fn open_mini(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    windows::open_mini(&app, state.engine.config().ui.mini_always_on_top)
        .map_err(|e| err(KtError::Io(e.to_string())))
}

#[tauri::command]
pub fn close_mini(app: AppHandle) {
    if let Some(w) = app.get_webview_window("mini") {
        let _ = w.close();
    }
}

#[tauri::command]
pub fn show_main(app: AppHandle) {
    windows::show_main(&app);
}
