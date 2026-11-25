//! System tray: an icon that shows the live BPM, a tooltip, and quick actions.

use std::sync::Mutex;

use kittytooth_core::config::Config;
use kittytooth_core::engine::{ConnState, EngineHandle, Snapshot};
use kittytooth_core::tray_icon::{parse_hex, render_bpm_icon, ICON_SIZE};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;
use crate::windows;

const TRAY_ID: &str = "main";

pub struct TrayItems {
    toggle: MenuItem<Wry>,
    last: Mutex<Option<String>>,
}

fn icon(bpm: Option<u16>, accent: &str) -> Image<'static> {
    let rgba = render_bpm_icon(bpm, parse_hex(accent).unwrap_or([255, 111, 168]));
    Image::new_owned(rgba, ICON_SIZE, ICON_SIZE)
}

pub fn create(app: &AppHandle, cfg: &Config) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show KittyTooth", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Connect", true, None::<&str>)?;
    let mini = MenuItem::with_id(app, "mini", "Mini window", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit KittyTooth", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &toggle, &mini, &sep, &quit])?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(None, &cfg.ui.accent))
        .tooltip("KittyTooth")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => windows::show_main(app),
            "mini" => {
                let on_top = app
                    .try_state::<AppState>()
                    .map(|s| s.engine.config().ui.mini_always_on_top)
                    .unwrap_or(true);
                if let Err(e) = windows::open_mini(app, on_top) {
                    tracing::warn!(error = %e, "could not open mini window");
                }
            }
            "toggle" => toggle_connection(app),
            "quit" => {
                if let Some(s) = app.try_state::<AppState>() {
                    s.quitting.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                windows::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(TrayItems {
        toggle,
        last: Mutex::new(None),
    });
    Ok(())
}

fn toggle_connection(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let engine = state.engine.clone();
    let snap = engine.snapshot();
    let cfg = engine.config();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if snap.target.is_some() && snap.state != ConnState::Idle {
            let _ = engine.disconnect().await;
        } else if let Some(d) = cfg.device.last_device {
            let _ = engine.connect(&d.id, &d.name).await;
        } else {
            windows::show_main(&app);
        }
    });
}

fn apply(app: &AppHandle, snap: &Snapshot, cfg: &Config, force: bool) {
    let shown = (snap.data_valid || snap.test_sweep).then_some(snap.bpm).flatten();
    let toggle_text = if snap.target.is_some() && snap.state != ConnState::Idle {
        "Disconnect".to_string()
    } else if let Some(d) = &cfg.device.last_device {
        format!("Connect to {}", d.name)
    } else {
        "Choose a sensor...".to_string()
    };
    let tooltip = match (shown, &snap.device) {
        (Some(b), Some(d)) => format!("KittyTooth: {b} bpm ({})", d.name),
        (Some(b), None) => format!("KittyTooth: {b} bpm"),
        _ => match snap.state {
            ConnState::Connecting | ConnState::Reconnecting => "KittyTooth: connecting...".into(),
            ConnState::NoData => "KittyTooth: waiting for heart rate".into(),
            _ => "KittyTooth: not connected".into(),
        },
    };
    let key = format!("{shown:?}|{toggle_text}|{tooltip}|{}", cfg.ui.accent);
    let Some(items) = app.try_state::<TrayItems>() else {
        return;
    };
    if let Ok(mut last) = items.last.lock() {
        if !force && last.as_deref() == Some(key.as_str()) {
            return;
        }
        *last = Some(key);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(icon(shown, &cfg.ui.accent)));
        let _ = tray.set_tooltip(Some(&tooltip));
    }
    let _ = items.toggle.set_text(toggle_text);
}

/// Re-renders the tray with the current state (e.g. after an accent change).
pub fn refresh(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        apply(app, &state.engine.snapshot(), &state.engine.config(), true);
    }
}

/// Keeps the tray in sync with the engine. Only re-renders when something visible changed.
pub fn spawn_updater(app: AppHandle, engine: EngineHandle) {
    tauri::async_runtime::spawn(async move {
        let mut snap = engine.subscribe_snapshot();
        loop {
            let s = snap.borrow_and_update().clone();
            apply(&app, &s, &engine.config(), false);
            if snap.changed().await.is_err() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}
