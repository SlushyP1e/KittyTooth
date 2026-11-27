//! Main window behaviour and the compact always-on-top mini window.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};

use crate::state::AppState;

/// Whether the main webview was last told that nobody can see it.
static MAIN_ASLEEP: AtomicBool = AtomicBool::new(false);

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        set_main_awake(app, true);
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Hiding or minimising the native window leaves WebView2 rendering as if it were on screen. This
/// hides the webview itself (so `document.hidden` is true and animations stop) and asks it to give
/// back memory. Scripts keep running, which alert sounds rely on.
pub fn set_main_awake(app: &AppHandle, awake: bool) {
    if !cfg!(windows) || MAIN_ASLEEP.swap(!awake, Ordering::SeqCst) != awake {
        return;
    }
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let result = if awake {
        w.as_ref().show()
    } else {
        w.as_ref().hide()
    };
    if let Err(err) = result {
        tracing::warn!(error = %err, awake, "could not change webview visibility");
    }
    #[cfg(windows)]
    set_memory_target(&w, awake);
}

#[cfg(windows)]
fn set_memory_target(w: &tauri::WebviewWindow, awake: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows_core::Interface;

    let level = if awake {
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
    } else {
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
    };
    let _ = w.with_webview(move |webview| {
        // Older WebView2 runtimes lack this interface; the cast then fails and nothing changes.
        // SAFETY: COM calls on the live controller, on the thread that owns it.
        let result = unsafe {
            webview
                .controller()
                .CoreWebView2()
                .and_then(|core| core.cast::<ICoreWebView2_19>())
                .and_then(|core| core.SetMemoryUsageTargetLevel(level))
        };
        if let Err(err) = result {
            tracing::debug!(error = %err, "memory usage target not set");
        }
    });
}

/// Opens (or focuses) the mini window. Must not be called synchronously from an IPC command or a
/// menu callback on Windows; call it from an async command or a spawned task.
pub fn open_mini(app: &AppHandle, on_top: bool) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("mini") {
        // A window left half-built by an earlier failure can refuse to show: replace it.
        if w.show().and_then(|_| w.set_focus()).is_ok() {
            return Ok(());
        }
        tracing::warn!("mini window unresponsive; recreating");
        let _ = w.destroy();
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

/// Closing the main window hides it to the tray (unless disabled or quitting). The webview sleeps
/// while the window is in the tray or minimised.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    // Minimising and restoring both arrive as a resize; focus covers any restore that does not.
    if matches!(event, WindowEvent::Resized(_) | WindowEvent::Focused(true)) {
        let shown = window.is_visible().unwrap_or(true) && !window.is_minimized().unwrap_or(false);
        set_main_awake(window.app_handle(), shown);
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
            set_main_awake(app, false);
        } else {
            state.quitting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
    }
}
