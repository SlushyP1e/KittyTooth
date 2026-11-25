//! Main window behaviour and the compact always-on-top mini window.

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};

use crate::state::AppState;

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn open_mini(app: &AppHandle, on_top: bool) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("mini") {
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "mini", WebviewUrl::App("index.html#/mini".into()))
        .title("KittyTooth Mini")
        .inner_size(232.0, 112.0)
        .min_inner_size(160.0, 80.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(on_top)
        .resizable(true)
        .build()?;
    Ok(())
}

/// Closing the main window hides it to the tray (unless disabled or quitting).
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        let app = window.app_handle();
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        if state.quitting.load(Ordering::SeqCst) {
            return;
        }
        if state.engine.config().ui.close_to_tray {
            api.prevent_close();
            let _ = window.hide();
        } else {
            state.quitting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
    }
}
