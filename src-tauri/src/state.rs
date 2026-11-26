//! Shared application state and config side effects (overlay server, autostart, logging).

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use kittytooth_core::config::{Config, ConfigStore};
use kittytooth_core::engine::EngineHandle;
use kittytooth_core::overlay::{self, OverlayServer};
use kittytooth_core::source::SecretStore;
use kittytooth_core::steamvr::SteamVrHandle;
use kittytooth_core::UserError;
use tauri::{AppHandle, Manager};

use crate::logging::Logging;

#[derive(Default)]
pub struct OverlayState {
    pub server: Option<OverlayServer>,
    pub error: Option<UserError>,
    pub port: u16,
    pub enabled: bool,
}

pub struct AppState {
    pub engine: EngineHandle,
    pub store: ConfigStore,
    pub logging: Logging,
    pub secrets: Arc<dyn SecretStore>,
    pub overlay: tokio::sync::Mutex<OverlayState>,
    /// Optional SteamVR overlay and haptics (its own thread; idle unless turned on).
    pub steamvr: SteamVrHandle,
    /// Shown once in the UI (e.g. config was unreadable and defaults were used).
    pub startup_notice: Mutex<Option<UserError>>,
    pub quitting: AtomicBool,
}

impl AppState {
    pub fn new(
        engine: EngineHandle,
        steamvr: SteamVrHandle,
        store: ConfigStore,
        logging: Logging,
        secrets: Arc<dyn SecretStore>,
        notice: Option<UserError>,
    ) -> Self {
        Self {
            engine,
            steamvr,
            store,
            logging,
            secrets,
            overlay: tokio::sync::Mutex::new(OverlayState::default()),
            startup_notice: Mutex::new(notice),
            quitting: AtomicBool::new(false),
        }
    }
}

/// Starts/stops/restarts the OBS overlay server to match the config.
pub async fn apply_overlay(app: &AppHandle, cfg: &Config) {
    let state = app.state::<AppState>();
    let mut o = state.overlay.lock().await;
    let want = cfg.overlay.enabled;
    if o.enabled == want && o.port == cfg.overlay.port && (o.server.is_some() || o.error.is_some() || !want) {
        return;
    }
    o.server = None;
    o.error = None;
    o.enabled = want;
    o.port = cfg.overlay.port;
    if want {
        match overlay::start(state.engine.clone(), cfg.overlay.port).await {
            Ok(s) => {
                tracing::info!(url = %s.url(), "overlay server started");
                o.server = Some(s);
            }
            Err(e) => {
                tracing::warn!(error = %e, "overlay server failed");
                o.error = Some(e.user());
            }
        }
    }
}

pub fn sync_autostart(app: &AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    let current = al.is_enabled().unwrap_or(false);
    if current != enabled {
        let r = if enabled { al.enable() } else { al.disable() };
        if let Err(e) = r {
            tracing::warn!(error = %e, "could not change start-with-system setting");
        }
    }
}
