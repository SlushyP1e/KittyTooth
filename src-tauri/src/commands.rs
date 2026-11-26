//! Tauri commands. Every command returns typed, human-readable errors (`UserError`).

use kittytooth_core::config::{Config, ConfigStore};
use kittytooth_core::engine::{HistoryPoint, Snapshot};
use kittytooth_core::history::{self, Session, SessionStore, SessionSummary};
use kittytooth_core::osc::sender::OscLogEntry;
use kittytooth_core::sensor::DiscoveredDevice;
use kittytooth_core::source::{self, SecretKind, SecretStatus, SourceKind};
use kittytooth_core::steamvr::SteamVrStatus;
use kittytooth_core::{KtError, UserError};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

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
    /// Folder holding saved sessions.
    pub history_dir: Option<String>,
    pub notice: Option<UserError>,
    /// Stored tokens/keys, masked.
    pub secrets: Vec<SecretStatus>,
    /// This build ships a HypeRate API key, so users only need their device ID.
    pub hyperate_builtin_key: bool,
}

#[derive(Serialize)]
pub struct InitialState {
    pub snapshot: Snapshot,
    pub config: Config,
    pub devices: Vec<DiscoveredDevice>,
    pub info: AppInfo,
    pub steamvr: SteamVrStatus,
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
        history_dir: state.engine.sessions().map(|s| s.dir().display().to_string()),
        notice: state.startup_notice.lock().ok().and_then(|mut n| n.take()),
        secrets: secret_statuses(state),
        hyperate_builtin_key: source::builtin_hyperate_key().is_some(),
    }
}

fn secret_statuses(state: &AppState) -> Vec<SecretStatus> {
    SecretKind::ALL
        .iter()
        .map(|&k| source::secret_status(state.secrets.as_ref(), k))
        .collect()
}

#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> CmdResult<InitialState> {
    Ok(InitialState {
        snapshot: state.engine.snapshot(),
        config: state.engine.config(),
        devices: state.engine.devices(),
        info: app_info(&state).await,
        steamvr: state.steamvr.status(),
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
pub async fn select_source(state: State<'_, AppState>, kind: SourceKind) -> CmdResult<()> {
    state.engine.select_source(kind).await.map_err(err)
}

#[tauri::command]
pub fn get_secrets(state: State<'_, AppState>) -> Vec<SecretStatus> {
    secret_statuses(&state)
}

/// Stores a token or key in the OS credential store and reconnects sources that use it.
#[tauri::command]
pub async fn set_secret(
    state: State<'_, AppState>,
    kind: SecretKind,
    value: String,
) -> CmdResult<Vec<SecretStatus>> {
    let v = value.trim();
    if v.is_empty() || v.len() > 512 || v.chars().any(char::is_control) {
        return Err(err(KtError::InvalidInput(
            "that doesn't look like a valid token or key".into(),
        )));
    }
    state.secrets.set(kind, v).map_err(err)?;
    tracing::info!(account = kind.account(), value = %source::mask(v), "credential saved");
    state.engine.restart_sources().await.map_err(err)?;
    Ok(secret_statuses(&state))
}

#[tauri::command]
pub async fn clear_secret(state: State<'_, AppState>, kind: SecretKind) -> CmdResult<Vec<SecretStatus>> {
    state.secrets.delete(kind).map_err(err)?;
    tracing::info!(account = kind.account(), "credential removed");
    state.engine.restart_sources().await.map_err(err)?;
    Ok(secret_statuses(&state))
}

/// Opens one of a fixed set of help pages in the browser.
#[tauri::command]
pub fn open_link(app: AppHandle, link: String) -> CmdResult<()> {
    let url = match link.as_str() {
        "pulsoid_token" => source::pulsoid::TOKEN_PAGE,
        "hyperate_api" => "https://www.hyperate.io/api",
        _ => return Err(err(KtError::InvalidInput("unknown link".into()))),
    };
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| err(KtError::Io(e.to_string())))
}

#[tauri::command]
pub async fn test_sweep(state: State<'_, AppState>, on: bool) -> CmdResult<()> {
    state.engine.test_sweep(on).await.map_err(err)
}

#[tauri::command]
pub async fn test_alert(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.test_alert().await.map_err(err)
}

#[tauri::command]
pub async fn reset_session(state: State<'_, AppState>) -> CmdResult<()> {
    state.engine.reset_session().await.map_err(err)
}

#[tauri::command]
pub fn get_history(state: State<'_, AppState>, since_ms: u64) -> Vec<HistoryPoint> {
    state.engine.history(since_ms)
}

fn sessions(state: &AppState) -> CmdResult<SessionStore> {
    state
        .engine
        .sessions()
        .cloned()
        .ok_or_else(|| err(KtError::Io("session history is not available".into())))
}

/// File work runs on the blocking pool so a big history never stalls the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| err(KtError::Io(e.to_string())))?
}

#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> CmdResult<Vec<SessionSummary>> {
    let store = sessions(&state)?;
    blocking(move || Ok(store.list())).await
}

#[tauri::command]
pub async fn get_session(state: State<'_, AppState>, id: String) -> CmdResult<Session> {
    let store = sessions(&state)?;
    blocking(move || store.load(&id).map_err(err)).await
}

#[tauri::command]
pub async fn delete_session(state: State<'_, AppState>, id: String) -> CmdResult<Vec<SessionSummary>> {
    let store = sessions(&state)?;
    blocking(move || {
        store.delete(&id).map_err(err)?;
        tracing::info!(id, "session deleted");
        Ok(store.list())
    })
    .await
}

/// Saves a session as CSV where the user picks. Returns the path, or `None` if cancelled.
#[tauri::command]
pub async fn export_session(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Option<String>> {
    let store = sessions(&state)?;
    let s2 = store.clone();
    let id2 = id.clone();
    let session = blocking(move || s2.load(&id2).map_err(err)).await?;
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("CSV", &["csv"])
        .set_file_name(history::csv_file_name(session.summary.start_ms))
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| err(KtError::Io(e.to_string())))?;
    let text = history::to_csv(&session);
    std::fs::write(&path, text).map_err(|e| err(e.into()))?;
    tracing::info!(id, path = %path.display(), "session exported");
    Ok(Some(path.display().to_string()))
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
    let creds: Vec<String> = secret_statuses(&state)
        .iter()
        .map(|s| {
            format!(
                "{}: {}",
                s.kind.account(),
                if s.present { "saved" } else { "not set" }
            )
        })
        .collect();
    let text = format!(
        "KittyTooth diagnostics\n\
         ======================\n\
         version: {}\nplatform: {}\nconfig: {}\nlogs: {}\noverlay: {}\nsteamvr: {}\ncredentials (values never included): {}\n\n\
         -- engine snapshot --\n{}\n\n-- config --\n{}\n\n-- recent log --\n{}\n",
        info.version,
        info.platform,
        info.config_path,
        info.log_dir,
        info.overlay_url.as_deref().unwrap_or("off"),
        serde_json::to_string(&state.steamvr.status()).unwrap_or_default(),
        creds.join(", "),
        serde_json::to_string_pretty(&snap).unwrap_or_default(),
        serde_json::to_string_pretty(&cfg).unwrap_or_default(),
        state.logging.tail(150),
    );
    // Belt and braces: no stored secret may leave through the clipboard.
    let text = crate::secrets::scrub(&text, state.secrets.as_ref());
    app.clipboard()
        .write_text(text.clone())
        .map_err(|e| err(KtError::Io(e.to_string())))?;
    Ok(text)
}

/// Async on purpose: creating a webview window inside a synchronous command runs on the main
/// thread while WebView2 is still inside the IPC callback, which deadlocks or half-creates the
/// window on Windows (tauri docs, wry#583). Async commands run off the main thread.
#[tauri::command]
pub async fn open_mini(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
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
