//! KittyTooth desktop shell: wires the core engine to Tauri windows, tray, events and
//! OS integration. All heart-rate/OSC logic lives in `kittytooth-core`.

mod commands;
mod logging;
mod secrets;
mod state;
mod tray;
mod updates;
mod windows;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use kittytooth_core::config::ConfigStore;
use kittytooth_core::engine::{self, EngineEvent, EngineHandle, EngineOptions};
use kittytooth_core::sensor::mock::MockBackend;
use kittytooth_core::sensor::{CompositeBackend, HeartRateBackend};
use kittytooth_core::source::{DefaultFactory, SecretStore};
use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;

use crate::state::AppState;

/// UI updates are throttled to this rate; the OSC path is unaffected.
const UI_FRAME: Duration = Duration::from_millis(33);

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::show_main(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::PendingUpdate::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::set_config,
            commands::start_scan,
            commands::stop_scan,
            commands::connect,
            commands::disconnect,
            commands::forget,
            commands::select_source,
            commands::get_secrets,
            commands::set_secret,
            commands::clear_secret,
            commands::open_link,
            commands::test_sweep,
            commands::test_alert,
            commands::reset_session,
            commands::get_history,
            commands::get_osc_log,
            commands::list_sessions,
            commands::get_session,
            commands::delete_session,
            commands::export_session,
            commands::export_config,
            commands::import_config,
            commands::reset_config,
            commands::copy_diagnostics,
            commands::open_mini,
            commands::close_mini,
            commands::show_main,
            updates::check_update,
            updates::install_update,
            updates::open_release,
        ])
        .setup(|app| {
            setup(app)?;
            Ok(())
        })
        .on_window_event(windows::on_window_event)
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(state) = app.try_state::<AppState>() {
                state.quitting.store(true, Ordering::SeqCst);
                // Removes the in-headset overlay before the engine goes.
                state.steamvr.shutdown();
                let engine = state.engine.clone();
                // Sends Connected=false to every OSC target before quitting.
                tauri::async_runtime::block_on(engine.shutdown());
            }
        }
    });
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let data_dir = app.path().app_config_dir()?;
    let log_dir = app.path().app_log_dir().unwrap_or_else(|_| data_dir.join("logs"));
    // Saved sessions stay on this computer (local, not roaming, app data).
    let history_dir = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| data_dir.clone())
        .join("sessions");
    let store = ConfigStore::new(&data_dir);
    let report = store.load();
    let logging = logging::Logging::init(&log_dir, report.config.debug.log_latency);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        "KittyTooth starting"
    );
    if let Some(v) = report.migrated_from {
        tracing::info!(from = v, "config migrated");
    }
    if let Some(e) = &report.recovered {
        tracing::warn!(error = %e, "config unreadable; defaults used and a backup kept");
    }
    let config = report.config.clone();
    let secrets: Arc<dyn SecretStore> = Arc::new(secrets::KeyringSecrets);

    let engine: EngineHandle = tauri::async_runtime::block_on(async {
        let real: Option<Arc<dyn HeartRateBackend>> =
            match kittytooth_core::sensor::ble::BleBackend::new().await {
                Ok(b) => Some(Arc::new(b)),
                Err(e) => {
                    tracing::warn!(error = %e, "Bluetooth unavailable");
                    None
                }
            };
        let backend = Arc::new(CompositeBackend::new(
            real,
            Arc::new(MockBackend::demo()),
            config.device.demo_device,
        ));
        let factory = Arc::new(DefaultFactory::new(backend, secrets.clone()));
        engine::start_with_factory(
            config.clone(),
            Some(store.clone()),
            factory,
            EngineOptions {
                history_dir: Some(history_dir),
                ..Default::default()
            },
        )
    });

    // Optional SteamVR overlay + haptics on its own thread; it only reads engine channels.
    let steamvr = kittytooth_core::steamvr::spawn(&engine);
    let steamvr_status = steamvr.subscribe();
    let state = AppState::new(
        engine.clone(),
        steamvr,
        store,
        logging,
        secrets,
        report.recovered.map(|e| e.user()),
    );
    app.manage(state);
    tauri::async_runtime::block_on(state::apply_overlay(&handle, &config));
    state::sync_autostart(&handle, config.ui.autostart);

    tray::create(&handle, &config)?;
    spawn_bridges(&handle, &engine);
    spawn_steamvr_bridge(&handle, steamvr_status);
    tray::spawn_updater(handle.clone(), engine.clone());

    // "Start minimized to tray" applies to every launch, including start-with-Windows
    // (which passes --autostart).
    if !config.ui.start_minimized {
        windows::show_main(&handle);
    } else {
        tracing::info!(
            autostart = std::env::args().any(|a| a == "--autostart"),
            "starting minimized to tray"
        );
    }
    Ok(())
}

/// Shows a desktop notification for a heart rate alert.
fn notify_alert(
    app: &tauri::AppHandle,
    kind: kittytooth_core::alerts::AlertKind,
    bpm: u16,
    threshold: u16,
    test: bool,
) {
    use kittytooth_core::alerts::AlertKind;
    let side = match kind {
        AlertKind::High => "above",
        AlertKind::Low => "below",
    };
    let title = if test {
        format!("{} (test)", kind.label())
    } else {
        kind.label().to_string()
    };
    let body = format!("{bpm} bpm, {side} your {threshold} bpm alert.");
    if let Err(err) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %err, "could not show the alert notification");
    }
}

/// Forwards SteamVR connection status to the UI.
fn spawn_steamvr_bridge(
    app: &tauri::AppHandle,
    mut status: tokio::sync::watch::Receiver<kittytooth_core::steamvr::SteamVrStatus>,
) {
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        while status.changed().await.is_ok() {
            let s = status.borrow_and_update().clone();
            let _ = a.emit("steamvr", &s);
        }
    });
}

/// Forwards engine state to the UI. Snapshots are throttled to display rate; beats and
/// samples go out immediately (a few per second at most).
fn spawn_bridges(app: &tauri::AppHandle, engine: &EngineHandle) {
    let a = app.clone();
    let mut snap = engine.subscribe_snapshot();
    tauri::async_runtime::spawn(async move {
        while snap.changed().await.is_ok() {
            let s = snap.borrow_and_update().clone();
            let _ = a.emit("snapshot", &s);
            tokio::time::sleep(UI_FRAME).await;
        }
    });

    let a = app.clone();
    let mut devices = engine.subscribe_devices();
    tauri::async_runtime::spawn(async move {
        while devices.changed().await.is_ok() {
            let d = devices.borrow_and_update().clone();
            let _ = a.emit("devices", &d);
        }
    });

    let a = app.clone();
    let e = engine.clone();
    let mut events = engine.subscribe_events();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(EngineEvent::Beat {
                    interval_ms, test, ..
                }) => {
                    let _ = a.emit(
                        "beat",
                        serde_json::json!({ "interval_ms": interval_ms, "test": test }),
                    );
                }
                Ok(EngineEvent::Sample(p)) => {
                    let _ = a.emit("sample", &p);
                }
                Ok(EngineEvent::SessionsChanged) => {
                    let _ = a.emit("sessions", ());
                }
                Ok(EngineEvent::ConfigChanged) => {
                    let _ = a.emit("config", &e.config());
                }
                Ok(EngineEvent::Alert {
                    alert_kind,
                    bpm,
                    threshold,
                    repeat,
                    test,
                }) => {
                    let _ = a.emit(
                        "alert",
                        serde_json::json!({
                            "kind": alert_kind,
                            "bpm": bpm,
                            "threshold": threshold,
                            "repeat": repeat,
                            "test": test,
                        }),
                    );
                    if e.config().alerts.notify {
                        notify_alert(&a, alert_kind, bpm, threshold, test);
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
        }
    });
}
