//! In-app updates via `tauri-plugin-updater` and the `latest.json` asset on GitHub Releases.
//!
//! Checking never installs anything. Installing happens only from the "Install and restart"
//! button, and only for copies the updater can replace (the NSIS install and the AppImage).
//! Portable copies get the release page instead; see `kittytooth_core::update`.

use std::sync::atomic::Ordering;

use kittytooth_core::update::{self, InstallKind, UpdateInfo};
use kittytooth_core::UserError;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

use crate::state::AppState;

type CmdResult<T> = Result<T, UserError>;

/// The update found by the last check, kept for "Install and restart".
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

fn user_error(code: &str, title: &str, message: &str, fix: &str) -> UserError {
    UserError {
        code: code.into(),
        title: title.into(),
        message: message.into(),
        fix: fix.into(),
    }
}

fn check_failed(e: tauri_plugin_updater::Error) -> UserError {
    tracing::warn!(error = %e, "update check failed");
    user_error(
        "update_check_failed",
        "Couldn't check for updates",
        "GitHub didn't answer, or the release has no update information yet.",
        "Check your internet connection and try again later.",
    )
}

fn install_failed(e: tauri_plugin_updater::Error) -> UserError {
    tracing::warn!(error = %e, "update install failed");
    user_error(
        "update_install_failed",
        "The update didn't install",
        "KittyTooth couldn't download or verify the new version.",
        "Try again later, or download it from the release page.",
    )
}

/// How this copy installs updates. Until the release signing key is set up (the public key
/// in tauri.conf.json is still the placeholder) nothing can be verified, so every copy
/// behaves like the portable one and links to the release page.
fn install_kind(app: &AppHandle) -> InstallKind {
    let configured = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .is_some_and(update::is_real_pubkey);
    if configured {
        update::detect_install_kind()
    } else {
        InstallKind::Portable
    }
}

/// Asks GitHub for `latest.json`. Returns the update on offer, or `None` when up to date.
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> CmdResult<Option<UpdateInfo>> {
    let updater = app
        .updater_builder()
        .version_comparator(|current, remote| update::is_newer(&current, &remote.version))
        .build()
        .map_err(check_failed)?;
    let found = updater.check().await.map_err(check_failed)?;
    let kind = install_kind(&app);
    let info = found.as_ref().map(|u| {
        UpdateInfo::new(
            &u.current_version,
            &u.version,
            u.body.clone(),
            u.raw_json
                .get("pub_date")
                .and_then(|d| d.as_str())
                .map(str::to_string),
            kind,
        )
    });
    if let Some(i) = &info {
        tracing::info!(version = %i.version, kind = ?kind, "update available");
    }
    *pending.0.lock().await = found;
    Ok(info)
}

/// Downloads and verifies the pending update, stops the engine cleanly (so VRChat sees
/// Connected=false), installs it and restarts. On Windows the installer closes the app itself.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, PendingUpdate>,
) -> CmdResult<()> {
    if !install_kind(&app).can_self_update() {
        return Err(user_error(
            "update_portable",
            "This copy can't update itself",
            "The portable version can't be replaced while it runs.",
            "Download the new version from the release page.",
        ));
    }
    let Some(found) = pending.0.lock().await.clone() else {
        return Err(user_error(
            "update_none",
            "No update to install",
            "There is no newer version waiting.",
            "Use Check now in Settings first.",
        ));
    };
    tracing::info!(version = %found.version, "downloading update");
    let mut downloaded = 0u64;
    let bytes = found
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                let _ = app.emit("update_progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(install_failed)?;

    tracing::info!(version = %found.version, "installing update");
    // On Windows `install` launches the installer and exits right away, so stop first.
    #[cfg(windows)]
    stop_engine(&state).await;
    if let Err(e) = found.install(bytes) {
        let mut err = install_failed(e);
        if cfg!(windows) {
            err.fix =
                "Restart KittyTooth to reconnect, then download the update from the release page.".into();
        }
        return Err(err);
    }
    #[cfg(not(windows))]
    stop_engine(&state).await;
    app.restart()
}

async fn stop_engine(state: &AppState) {
    state.quitting.store(true, Ordering::SeqCst);
    state.engine.shutdown().await;
}

/// Opens the GitHub release page for `version` (portable copies update by hand).
#[tauri::command]
pub fn open_release(app: AppHandle, version: String) -> CmdResult<()> {
    app.opener()
        .open_url(update::release_page_url(&version), None::<&str>)
        .map_err(|e| {
            tracing::warn!(error = %e, "could not open the release page");
            user_error(
                "open_failed",
                "Couldn't open your browser",
                "KittyTooth couldn't open the release page.",
                &format!("Open {} yourself.", update::RELEASES_URL),
            )
        })
}
