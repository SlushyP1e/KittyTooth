//! Optional SteamVR integration: a small BPM overlay in the headset (on a wrist or in view) and
//! a heartbeat-like controller buzz when a heart rate alert fires.
//!
//! It runs on its own OS thread and only reads the engine's watch/broadcast channels, so it can
//! never block or slow the OSC path. SteamVR is loaded at runtime (see [`ffi`]); with no SteamVR
//! installed or running it just reports that and checks again every few seconds. It never
//! launches SteamVR.

pub mod fade;
mod ffi;
pub mod render;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::{broadcast, watch};

use crate::config::{Config, SteamVrConfig};
use crate::engine::{EngineEvent, EngineHandle, Snapshot};
use ffi::{ConnectError, Matrix34, OverlayHandle, Session, TrackedDeviceIndex};
use render::View;

/// How often to look for SteamVR while it is not running.
const PROBE_EVERY: Duration = Duration::from_secs(5);
/// Slower when it is not installed at all (only a file check, but no need to hurry).
const PROBE_NOT_INSTALLED: Duration = Duration::from_secs(15);
/// Main loop tick while connected.
const TICK: Duration = Duration::from_millis(50);
/// Tick while disabled or disconnected.
const IDLE_TICK: Duration = Duration::from_millis(200);
/// Re-render at most this often (only when the content changed).
const MIN_REDRAW: Duration = Duration::from_millis(250);
/// Controller indices can change when a controller reconnects; look them up this often.
const REATTACH_EVERY: Duration = Duration::from_secs(1);

/// Main loop tick while the wrist fade is running (smooth fades, ~40 Hz).
const FADE_TICK: Duration = Duration::from_millis(25);
/// A full fade in or out takes this long.
const FADE_SECS: f32 = 0.2;
/// How long the old image stays under a freshly uploaded one (a few compositor frames).
const SWAP_OVERLAP: Duration = Duration::from_millis(60);

const OVERLAY_KEYS: [&str; 2] = ["kittytooth.heartrate.a", "kittytooth.heartrate.b"];
const OVERLAY_NAME: &str = "KittyTooth heart rate";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SteamVrState {
    /// Turned off in settings.
    #[default]
    Off,
    /// No SteamVR (OpenVR runtime) on this PC.
    NotInstalled,
    /// Installed but not running; connects automatically when it starts.
    NotRunning,
    Connected,
    /// Something went wrong; `message` says what. Retries automatically.
    Error,
}

/// SteamVR connection status for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct SteamVrStatus {
    pub state: SteamVrState,
    /// SteamVR version while connected.
    pub version: Option<String>,
    pub message: Option<String>,
}

impl SteamVrStatus {
    fn of(state: SteamVrState) -> Self {
        Self {
            state,
            ..Default::default()
        }
    }
}

/// Handle to the SteamVR thread.
pub struct SteamVrHandle {
    status: watch::Receiver<SteamVrStatus>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SteamVrHandle {
    pub fn status(&self) -> SteamVrStatus {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<SteamVrStatus> {
        self.status.clone()
    }

    /// Removes the overlay and disconnects from SteamVR. Returns within about half a second.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.lock().ok().and_then(|mut t| t.take()) {
            let _ = t.join();
        }
    }
}

/// Starts the SteamVR thread for an engine.
pub fn spawn(engine: &EngineHandle) -> SteamVrHandle {
    spawn_with(
        engine.subscribe_config(),
        engine.subscribe_snapshot(),
        engine.subscribe_events(),
        ffi::vrpath_file(),
    )
}

/// Starts the SteamVR thread on raw channels. `vrpath` is the OpenVR path registry file.
pub fn spawn_with(
    config: watch::Receiver<Config>,
    snapshot: watch::Receiver<Snapshot>,
    events: broadcast::Receiver<EngineEvent>,
    vrpath: Option<std::path::PathBuf>,
) -> SteamVrHandle {
    let (tx, rx) = watch::channel(SteamVrStatus::default());
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Worker {
        config,
        snapshot,
        events,
        vrpath,
        status: tx,
        stop: stop.clone(),
    };
    let thread = std::thread::Builder::new()
        .name("steamvr".into())
        .spawn(move || worker.run())
        .map_err(|e| tracing::warn!(error = %e, "could not start the SteamVR thread"))
        .ok();
    SteamVrHandle {
        status: rx,
        stop,
        thread: Mutex::new(thread),
    }
}

struct Worker {
    config: watch::Receiver<Config>,
    snapshot: watch::Receiver<Snapshot>,
    events: broadcast::Receiver<EngineEvent>,
    vrpath: Option<std::path::PathBuf>,
    status: watch::Sender<SteamVrStatus>,
    stop: Arc<AtomicBool>,
}

impl Worker {
    fn run(mut self) {
        let stop = self.stop.clone();
        let status = self.status.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.main_loop()));
        if r.is_err() && !stop.load(Ordering::SeqCst) {
            tracing::error!("SteamVR integration stopped after an internal error");
            status.send_replace(SteamVrStatus {
                state: SteamVrState::Error,
                version: None,
                message: Some("The SteamVR integration stopped. Restart KittyTooth to try again.".into()),
            });
        }
    }

    fn set_status(&self, s: SteamVrStatus) {
        self.status.send_if_modified(|cur| {
            if *cur == s {
                false
            } else {
                *cur = s;
                true
            }
        });
    }

    /// Sleeps up to `d`, waking early on shutdown.
    fn nap(&self, d: Duration) -> bool {
        let end = Instant::now() + d;
        while Instant::now() < end {
            if self.stop.load(Ordering::SeqCst) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25).min(end - Instant::now()));
        }
        !self.stop.load(Ordering::SeqCst)
    }

    /// Alerts that should buzz, since the last call. Drops everything else.
    fn take_alerts(&mut self) -> usize {
        let mut n = 0;
        loop {
            match self.events.try_recv() {
                Ok(EngineEvent::Alert { .. }) => n += 1,
                Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => return n,
            }
        }
    }

    fn main_loop(&mut self) {
        let mut conn: Option<Connected> = None;
        let mut next_probe = Instant::now();
        let mut last_logged: Option<SteamVrState> = None;
        while !self.stop.load(Ordering::SeqCst) {
            let cfg = self.config.borrow().steamvr.clone();
            if !cfg.enabled {
                if conn.take().is_some() {
                    tracing::info!("SteamVR integration turned off");
                }
                self.take_alerts();
                self.set_status(SteamVrStatus::of(SteamVrState::Off));
                next_probe = Instant::now();
                last_logged = None;
                if !self.nap(IDLE_TICK) {
                    break;
                }
                continue;
            }

            let Some(c) = conn.as_mut() else {
                // Alerts while SteamVR is away are not saved up for later.
                self.take_alerts();
                if Instant::now() >= next_probe {
                    let (status, wait) = match self.connect() {
                        Ok(c) => {
                            let s = SteamVrStatus {
                                state: SteamVrState::Connected,
                                version: c.session.runtime_version.clone(),
                                message: None,
                            };
                            tracing::info!(version = ?s.version, "connected to SteamVR");
                            conn = Some(c);
                            (s, PROBE_EVERY)
                        }
                        Err(ConnectError::NotInstalled) => {
                            (SteamVrStatus::of(SteamVrState::NotInstalled), PROBE_NOT_INSTALLED)
                        }
                        Err(ConnectError::NotRunning) => {
                            (SteamVrStatus::of(SteamVrState::NotRunning), PROBE_EVERY)
                        }
                        Err(ConnectError::Failed(msg)) => {
                            if last_logged != Some(SteamVrState::Error) {
                                tracing::warn!(error = %msg, "could not connect to SteamVR");
                            }
                            let s = SteamVrStatus {
                                state: SteamVrState::Error,
                                version: None,
                                message: Some(msg),
                            };
                            (s, PROBE_EVERY * 2)
                        }
                    };
                    if last_logged != Some(status.state) && status.state != SteamVrState::Connected {
                        tracing::debug!(state = ?status.state, "SteamVR status");
                    }
                    last_logged = Some(status.state);
                    self.set_status(status);
                    next_probe = Instant::now() + wait;
                }
                if !self.nap(IDLE_TICK) {
                    break;
                }
                continue;
            };

            if c.session.poll_quit() {
                tracing::info!("SteamVR is closing; disconnecting");
                conn = None; // removes the overlay and unloads vrclient
                self.set_status(SteamVrStatus::of(SteamVrState::NotRunning));
                last_logged = Some(SteamVrState::NotRunning);
                next_probe = Instant::now() + PROBE_EVERY;
                continue;
            }

            let view = view_of(&self.snapshot.borrow());
            if let Err(msg) = c.sync_overlay(&cfg, view) {
                tracing::warn!(error = %msg, "SteamVR overlay problem");
                c.error = Some(format!("The overlay could not be shown: {msg}"));
            }
            self.set_status(SteamVrStatus {
                state: SteamVrState::Connected,
                version: c.session.runtime_version.clone(),
                message: c.error.clone(),
            });
            if self.take_alerts() > 0 && cfg.haptics {
                c.heartbeat(&cfg.haptics_hand, &self.stop);
            }
            let fading = cfg.overlay && cfg.fade && cfg.attach != "hud";
            if !self.nap(if fading { FADE_TICK } else { TICK }) {
                break;
            }
        }
        drop(conn);
    }

    fn connect(&self) -> Result<Connected, ConnectError> {
        let client = ffi::find_client(self.vrpath.as_deref())?;
        let session = Session::connect(&client)?;
        Ok(Connected::new(session))
    }
}

/// What the overlay should show for a snapshot.
pub fn view_of(s: &Snapshot) -> View {
    match &s.alert {
        Some(a) => View {
            bpm: Some(a.bpm),
            alert: Some(a.kind),
            test: a.test,
        },
        None => View {
            bpm: s.bpm.filter(|_| s.data_valid),
            alert: None,
            test: false,
        },
    }
}

/// Overlay width in metres for a placement and size setting.
pub fn overlay_width(attach: &str, size: &str) -> f32 {
    let wrist = attach != "hud";
    match (wrist, size) {
        (true, "small") => 0.07,
        (true, "large") => 0.14,
        (true, _) => 0.10,
        (false, "small") => 0.18,
        (false, "large") => 0.36,
        (false, _) => 0.26,
    }
}

/// Rotation about X by `deg` with a translation, as a row-major 3x4 matrix.
fn rot_x(deg: f32, t: [f32; 3]) -> Matrix34 {
    let (s, c) = deg.to_radians().sin_cos();
    [[1.0, 0.0, 0.0, t[0]], [0.0, c, -s, t[1]], [0.0, s, c, t[2]]]
}

/// Overlay pose relative to its device. A wrist overlay lies on top of the controller, just
/// behind it, tilted back towards your eyes (text up = towards the controller tip). The HUD sits
/// a metre ahead, a little below eye level, tilted up to face you.
pub fn attach_transform(attach: &str) -> Matrix34 {
    if attach == "hud" {
        rot_x(-15.0, [0.0, -0.28, -1.0])
    } else {
        rot_x(-60.0, [0.0, 0.03, 0.12])
    }
}

/// Heartbeat buzz: a strong "lub", a pause, a softer "dub". (start ms, end ms, pulse µs)
pub const HEARTBEAT: [(u64, u64, u16); 2] = [(0, 90, 3999), (210, 280, 2600)];

struct Connected {
    session: Session,
    /// Two overlays at the same spot: a new image goes to the hidden one, which is then shown on
    /// top before the old one is hidden, so an update never blanks the card.
    overlays: Option<[OverlayHandle; 2]>,
    /// Index of the overlay holding the latest image.
    front: usize,
    /// The previous front, still shown underneath until this time.
    retire: Option<(usize, Instant)>,
    /// Attached to this device and shown.
    shown_on: Option<TrackedDeviceIndex>,
    applied: Option<(String, String)>,
    last_pixels: Option<Vec<u8>>,
    last_view: Option<View>,
    last_draw: Option<Instant>,
    last_attach: Option<Instant>,
    last_tick: Option<Instant>,
    /// Gaze fade, 0..1 (eased).
    fade: f32,
    /// Alpha last sent to SteamVR.
    sent_alpha: f32,
    /// Creating or drawing the overlay failed; not retried until it is toggled or reconnected.
    failed: bool,
    /// Shown in the status while `failed`.
    error: Option<String>,
}

impl Connected {
    fn new(session: Session) -> Self {
        Self {
            session,
            overlays: None,
            front: 0,
            retire: None,
            shown_on: None,
            applied: None,
            last_pixels: None,
            last_view: None,
            last_draw: None,
            last_attach: None,
            last_tick: None,
            fade: 0.0,
            sent_alpha: -1.0,
            failed: false,
            error: None,
        }
    }

    fn destroy_overlays(&mut self) {
        if let Some(hs) = self.overlays.take() {
            for h in hs {
                self.session.destroy_overlay(h);
            }
        }
        self.front = 0;
        self.retire = None;
        self.shown_on = None;
        self.applied = None;
        self.last_pixels = None;
        self.last_view = None;
        self.fade = 0.0;
        self.sent_alpha = -1.0;
    }

    fn sync_overlay(&mut self, cfg: &SteamVrConfig, view: View) -> Result<(), String> {
        let now = Instant::now();
        let dt = self.last_tick.map_or(0.0, |t| (now - t).as_secs_f32().min(0.1));
        self.last_tick = Some(now);
        if !cfg.overlay {
            self.destroy_overlays();
            self.failed = false;
            self.error = None;
            return Ok(());
        }
        if self.failed {
            return Ok(()); // reported once; retried after a reconnect or toggling it
        }
        let hs = match self.overlays {
            Some(hs) => hs,
            None => {
                let mut hs = [0; 2];
                for (i, key) in OVERLAY_KEYS.iter().enumerate() {
                    match self.session.create_overlay(key, OVERLAY_NAME) {
                        Ok(h) => hs[i] = h,
                        Err(e) => {
                            for &h in &hs[..i] {
                                self.session.destroy_overlay(h);
                            }
                            self.failed = true;
                            return Err(e);
                        }
                    }
                }
                self.session.set_sort_order(hs[0], 11);
                self.session.set_sort_order(hs[1], 10);
                self.overlays = Some(hs);
                hs
            }
        };

        let want = (cfg.attach.clone(), cfg.size.clone());
        let changed = self.applied.as_ref() != Some(&want);
        if changed {
            for h in hs {
                self.session.set_width(h, overlay_width(&cfg.attach, &cfg.size));
            }
            self.applied = Some(want);
        }

        if changed || self.last_attach.is_none_or(|t| now - t >= REATTACH_EVERY) {
            self.last_attach = Some(now);
            let device = match cfg.attach.as_str() {
                "hud" => Some(ffi::TRACKED_DEVICE_HMD),
                "right_wrist" => self.session.controller(ffi::ROLE_RIGHT_HAND),
                _ => self.session.controller(ffi::ROLE_LEFT_HAND),
            };
            match device {
                Some(d) => {
                    if changed || self.shown_on != Some(d) {
                        let m = attach_transform(&cfg.attach);
                        for h in hs {
                            self.session.set_transform_relative(h, d, &m);
                        }
                        if self.shown_on.is_none() && self.last_pixels.is_some() {
                            self.session.show(hs[self.front]);
                        }
                        self.shown_on = Some(d);
                    }
                }
                None => {
                    // That controller is off; hide until it comes back.
                    if self.shown_on.take().is_some() {
                        for h in hs {
                            self.session.hide(h);
                        }
                        self.retire = None;
                    }
                }
            }
        }

        // The old image has been covered long enough for the new one to be on screen.
        if let Some((old, at)) = self.retire {
            if now >= at {
                self.session.hide(hs[old]);
                self.retire = None;
            }
        }

        let due = self.last_draw.is_none_or(|t| now - t >= MIN_REDRAW);
        if self.last_view != Some(view) && due {
            self.last_view = Some(view);
            let mut px = render::render(&view);
            // Only upload when the image really changed.
            if self.last_pixels.as_deref() != Some(&px[..]) {
                self.upload(hs, &mut px, now)?;
                self.last_pixels = Some(px);
                self.last_draw = Some(now);
            }
        }

        self.apply_fade(cfg, view, hs, dt);
        Ok(())
    }

    /// Double-buffered upload: draw into the hidden overlay, show it above the current one, and
    /// hide the current one a moment later.
    fn upload(&mut self, hs: [OverlayHandle; 2], px: &mut [u8], now: Instant) -> Result<(), String> {
        let first = self.last_pixels.is_none();
        let target = if first { self.front } else { 1 - self.front };
        if let Some((old, _)) = self.retire.take() {
            if old != target {
                self.session.hide(hs[old]);
            }
        }
        if let Err(e) = self
            .session
            .set_raw(hs[target], px, render::WIDTH, render::HEIGHT)
        {
            self.failed = true;
            return Err(e);
        }
        if first {
            if self.shown_on.is_some() {
                self.session.show(hs[target]);
            }
            return Ok(());
        }
        let old = self.front;
        self.session.set_sort_order(hs[target], 11);
        self.session.set_sort_order(hs[old], 10);
        if self.shown_on.is_some() {
            self.session.show(hs[target]);
            self.retire = Some((old, now + SWAP_OVERLAP));
        }
        self.front = target;
        Ok(())
    }

    /// Eases the gaze fade and sends the alpha (user opacity x fade) when it changed.
    fn apply_fade(&mut self, cfg: &SteamVrConfig, view: View, hs: [OverlayHandle; 2], dt: f32) {
        let target = match self.shown_on {
            // Alerts always show; the HUD is always in view.
            Some(d) if cfg.fade && cfg.attach != "hud" && view.alert.is_none() => {
                let poses = self.session.poses(d + 1);
                let (hmd, dev) = (&poses[ffi::TRACKED_DEVICE_HMD as usize], &poses[d as usize]);
                if hmd.pose_is_valid && dev.pose_is_valid {
                    let overlay = fade::mul(&dev.device_to_absolute, &attach_transform(&cfg.attach));
                    let gaze = fade::gaze_angle(&hmd.device_to_absolute, &overlay);
                    let facing = fade::facing_angle(&hmd.device_to_absolute, &overlay);
                    fade::fade_target(gaze, facing, fade::fade_angles(&cfg.fade_angle))
                } else {
                    self.fade // tracking lost for a moment: hold
                }
            }
            _ => 1.0,
        };
        self.fade = fade::ease_toward(self.fade, target, dt, FADE_SECS);
        let alpha = cfg.opacity as f32 / 100.0 * self.fade;
        if (alpha - self.sent_alpha).abs() > 0.004 || (alpha == 0.0) != (self.sent_alpha == 0.0) {
            for h in hs {
                self.session.set_alpha(h, alpha);
            }
            self.sent_alpha = alpha;
        }
    }

    /// Plays [`HEARTBEAT`] on the chosen controller(s). Blocks this thread for ~0.3 s.
    fn heartbeat(&self, hand: &str, stop: &AtomicBool) {
        let mut devices = Vec::new();
        if hand != "right" {
            devices.extend(self.session.controller(ffi::ROLE_LEFT_HAND));
        }
        if hand != "left" {
            devices.extend(self.session.controller(ffi::ROLE_RIGHT_HAND));
        }
        if devices.is_empty() {
            return;
        }
        let start = Instant::now();
        let end = HEARTBEAT.iter().map(|s| s.1).max().unwrap_or(0);
        loop {
            let ms = start.elapsed().as_millis() as u64;
            if ms >= end || stop.load(Ordering::SeqCst) {
                break;
            }
            if let Some(&(_, _, us)) = HEARTBEAT.iter().find(|(a, b, _)| (*a..*b).contains(&ms)) {
                for &d in &devices {
                    self.session.haptic_pulse(d, us);
                }
            }
            // SteamVR takes one legacy pulse per controller every 5 ms.
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Connected {
    fn drop(&mut self) {
        self.destroy_overlays();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::{AlertKind, AlertStatus};

    fn wait_for(rx: &watch::Receiver<SteamVrStatus>, want: SteamVrState) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if rx.borrow().state == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Against this PC's real SteamVR install (never launches it): with SteamVR closed the
    /// status must settle on "not running". Run by hand with `-- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_runtime_status() {
        let mut cfg = Config::default();
        cfg.steamvr.enabled = true;
        let (_cfg_tx, cfg_rx) = watch::channel(cfg);
        let (_snap_tx, snap_rx) = watch::channel(Snapshot::default());
        let (_ev_tx, ev_rx) = broadcast::channel(8);
        let h = spawn_with(cfg_rx, snap_rx, ev_rx, ffi::vrpath_file());
        let rx = h.subscribe();
        let end = Instant::now() + Duration::from_secs(5);
        while rx.borrow().state == SteamVrState::Off && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        println!("status: {:?}", h.status());
        assert_ne!(h.status().state, SteamVrState::Off);
        h.shutdown();
    }

    #[test]
    fn view_follows_snapshot_and_alert() {
        let mut s = Snapshot {
            bpm: Some(72),
            data_valid: true,
            ..Default::default()
        };
        assert_eq!(view_of(&s).bpm, Some(72));
        s.data_valid = false;
        assert_eq!(view_of(&s).bpm, None, "stale data shows --");
        s.alert = Some(AlertStatus {
            kind: AlertKind::High,
            bpm: 162,
            threshold: 150,
            since_ms: 0,
            test: true,
        });
        let v = view_of(&s);
        assert_eq!((v.bpm, v.alert, v.test), (Some(162), Some(AlertKind::High), true));
    }

    #[test]
    fn placement_values() {
        assert!(overlay_width("hud", "large") > overlay_width("hud", "small"));
        assert!(overlay_width("left_wrist", "medium") < overlay_width("hud", "medium"));
        // Rotations stay orthonormal.
        for a in ["hud", "left_wrist"] {
            let m = attach_transform(a);
            for r in &m {
                let n: f32 = r[..3].iter().map(|v| v * v).sum();
                assert!((n - 1.0).abs() < 1e-5);
            }
        }
        // The wrist overlay faces up and back (its +Z goes towards +Y and +Z of the controller).
        let m = attach_transform("left_wrist");
        assert!(m[1][2] > 0.5 && m[2][2] > 0.0);
        assert!(HEARTBEAT.iter().all(|&(a, b, us)| a < b && us <= 3999));
    }

    #[test]
    fn reports_off_then_not_installed_and_shuts_down() {
        if std::env::var_os("VR_OVERRIDE").is_some() {
            return;
        }
        let mut cfg = Config::default();
        let (cfg_tx, cfg_rx) = watch::channel(cfg.clone());
        let (_snap_tx, snap_rx) = watch::channel(Snapshot::default());
        let (ev_tx, ev_rx) = broadcast::channel(8);
        let missing = std::env::temp_dir().join(format!("kt-none-{}.vrpath", uuid::Uuid::new_v4()));
        let h = spawn_with(cfg_rx, snap_rx, ev_rx, Some(missing));
        let rx = h.subscribe();
        assert!(wait_for(&rx, SteamVrState::Off));
        cfg.steamvr.enabled = true;
        cfg_tx.send_replace(cfg);
        assert!(wait_for(&rx, SteamVrState::NotInstalled));
        // Alerts with no SteamVR are dropped quietly.
        let _ = ev_tx.send(EngineEvent::ConfigChanged);
        let t = Instant::now();
        h.shutdown();
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
