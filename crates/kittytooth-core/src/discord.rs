//! Optional Discord Rich Presence: shows "Playing KittyTooth" on your Discord profile with your
//! heart rate, a link to the project, and how long the app has been open.
//!
//! It talks to the Discord desktop app over its local IPC socket (a named pipe on Windows, a Unix
//! socket elsewhere) on its own OS thread, and only reads the engine's watch channels, so it can
//! never slow the OSC path. Nothing goes over the network from KittyTooth itself; with Discord
//! closed it just checks again every few seconds.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::config::{Config, DiscordConfig};
use crate::engine::{EngineHandle, Snapshot};

/// KittyTooth's Discord application ID (the "KittyTooth" in "Playing KittyTooth"). Settings can
/// override it; with both empty Rich Presence reports that it needs one.
pub const DEFAULT_APP_ID: &str = "1575689702080647230";
pub const REPO_URL: &str = "https://github.com/SlushyP1e/KittyTooth";
/// Shown as the large image. Discord proxies external image URLs, so no asset upload is needed.
pub const ICON_URL: &str = "https://raw.githubusercontent.com/SlushyP1e/KittyTooth/main/assets/icon.png";

/// How often to look for Discord while it is closed.
const PROBE_EVERY: Duration = Duration::from_secs(10);
/// Discord allows about one presence update every few seconds; stay well under its limit.
const MIN_UPDATE: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(250);
/// Largest frame accepted from Discord.
const MAX_FRAME: u32 = 64 * 1024;

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiscordState {
    /// Turned off in settings.
    #[default]
    Off,
    /// No Discord application ID configured.
    NoAppId,
    /// The Discord desktop app is not running (or not reachable).
    NotRunning,
    Connected,
    /// Discord refused the connection; `message` says why. Retries automatically.
    Error,
}

/// Rich Presence status for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct DiscordStatus {
    pub state: DiscordState,
    /// The Discord account the presence shows on, while connected.
    pub user: Option<String>,
    pub message: Option<String>,
}

impl DiscordStatus {
    fn of(state: DiscordState) -> Self {
        Self {
            state,
            ..Default::default()
        }
    }
}

/// Handle to the Rich Presence thread.
pub struct DiscordHandle {
    status: watch::Receiver<DiscordStatus>,
    stop: Arc<AtomicBool>,
    done: Mutex<Option<mpsc::Receiver<()>>>,
}

impl DiscordHandle {
    pub fn status(&self) -> DiscordStatus {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<DiscordStatus> {
        self.status.clone()
    }

    /// Clears the presence and disconnects. Waits at most a second.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(rx) = self.done.lock().ok().and_then(|mut d| d.take()) {
            let _ = rx.recv_timeout(Duration::from_secs(1));
        }
    }
}

/// Starts the Rich Presence thread for an engine.
pub fn spawn(engine: &EngineHandle) -> DiscordHandle {
    spawn_with(
        engine.subscribe_config(),
        engine.subscribe_snapshot(),
        Box::new(connect_ipc),
    )
}

type Connector = Box<dyn Fn() -> std::io::Result<Box<dyn Pipe>> + Send>;

/// The IPC byte stream (a named pipe or Unix socket; a fake in tests).
pub trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

/// Starts the thread on raw channels with a custom IPC connector.
pub fn spawn_with(
    config: watch::Receiver<Config>,
    snapshot: watch::Receiver<Snapshot>,
    connect: Connector,
) -> DiscordHandle {
    let (tx, rx) = watch::channel(DiscordStatus::default());
    let stop = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel();
    let worker = Worker {
        config,
        snapshot,
        connect,
        status: tx,
        stop: stop.clone(),
        started: unix_secs(),
    };
    let spawned = std::thread::Builder::new().name("discord".into()).spawn(move || {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker.run()));
        let _ = done_tx.send(());
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the Discord thread");
    }
    DiscordHandle {
        status: rx,
        stop,
        done: Mutex::new(Some(done_rx)),
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The application ID in use: the setting, else [`DEFAULT_APP_ID`].
pub fn app_id(cfg: &DiscordConfig) -> Option<String> {
    let id = cfg.app_id.trim();
    let id = if id.is_empty() { DEFAULT_APP_ID } else { id };
    (!id.is_empty()).then(|| id.to_string())
}

/// The activity Discord shows for a snapshot. `started` is when KittyTooth opened (unix secs).
pub fn activity(s: &Snapshot, cfg: &DiscordConfig, started: u64) -> Value {
    let bpm = s.bpm.filter(|_| s.data_valid);
    let details = match bpm {
        Some(b) => format!("\u{2764}\u{fe0f} {b} bpm"),
        None => "Waiting for heart rate".to_string(),
    };
    let mut state = Vec::new();
    if bpm.is_some() && cfg.show_zone && s.zone > 0 {
        state.push(format!("Zone {}", s.zone));
    }
    if cfg.show_device {
        if let Some(d) = s.device.as_ref().filter(|d| !d.demo && !d.name.trim().is_empty()) {
            state.push(d.name.trim().chars().take(60).collect());
        }
    }
    let mut a = json!({
        "details": details,
        "timestamps": { "start": started },
        "assets": {
            "large_image": ICON_URL,
            "large_text": format!("KittyTooth {}", env!("CARGO_PKG_VERSION")),
        },
    });
    let state = state.join(" \u{b7} ");
    // Discord rejects a state shorter than 2 characters.
    if state.chars().count() >= 2 {
        a["state"] = json!(state);
    }
    if cfg.button {
        a["buttons"] = json!([{ "label": "Get KittyTooth", "url": REPO_URL }]);
    }
    a
}

/// Encodes one IPC frame: opcode and length (little-endian u32s), then the JSON payload.
pub fn encode_frame(op: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Reads one IPC frame.
pub fn read_frame(r: &mut dyn Read) -> std::io::Result<(u32, Value)> {
    let mut head = [0u8; 8];
    r.read_exact(&mut head)?;
    let op = u32::from_le_bytes(head[..4].try_into().unwrap_or_default());
    let len = u32::from_le_bytes(head[4..].try_into().unwrap_or_default());
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    let v = serde_json::from_slice(&body).unwrap_or(Value::Null);
    Ok((op, v))
}

enum IpcError {
    /// The pipe broke or Discord went away.
    Io(std::io::Error),
    /// Discord closed the connection or answered with an error.
    Refused(String),
}

impl From<std::io::Error> for IpcError {
    fn from(e: std::io::Error) -> Self {
        IpcError::Io(e)
    }
}

struct Ipc {
    pipe: Box<dyn Pipe>,
}

impl Ipc {
    fn send(&mut self, op: u32, payload: &Value) -> Result<(), IpcError> {
        self.pipe.write_all(&encode_frame(op, payload))?;
        self.pipe.flush()?;
        Ok(())
    }

    /// Waits for Discord's answer, replying to pings on the way.
    fn reply(&mut self) -> Result<Value, IpcError> {
        loop {
            let (op, v) = read_frame(&mut self.pipe)?;
            match op {
                OP_PING => self.send(OP_PONG, &v)?,
                OP_CLOSE => {
                    let msg = v["message"].as_str().unwrap_or("Discord closed the connection");
                    return Err(IpcError::Refused(msg.to_string()));
                }
                OP_FRAME => {
                    if v["evt"] == "ERROR" {
                        let msg = v["data"]["message"]
                            .as_str()
                            .unwrap_or("Discord returned an error");
                        return Err(IpcError::Refused(msg.to_string()));
                    }
                    return Ok(v);
                }
                _ => {}
            }
        }
    }

    /// Handshake; returns the Discord username from the READY event.
    fn handshake(&mut self, app_id: &str) -> Result<Option<String>, IpcError> {
        self.send(OP_HANDSHAKE, &json!({ "v": 1, "client_id": app_id }))?;
        let ready = self.reply()?;
        let user = &ready["data"]["user"];
        let name = user["global_name"].as_str().or_else(|| user["username"].as_str());
        Ok(name.map(str::to_string))
    }

    /// Sets (or with `None`, clears) the presence.
    fn set_activity(&mut self, activity: Option<&Value>) -> Result<(), IpcError> {
        let mut args = json!({ "pid": std::process::id() });
        if let Some(a) = activity {
            args["activity"] = a.clone();
        }
        let cmd = json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": uuid::Uuid::new_v4().to_string() });
        self.send(OP_FRAME, &cmd)?;
        self.reply().map(|_| ())
    }
}

/// Opens Discord's IPC pipe (`discord-ipc-0` .. `discord-ipc-9`).
fn connect_ipc() -> std::io::Result<Box<dyn Pipe>> {
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "Discord is not running");
    #[cfg(windows)]
    for i in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{i}");
        match std::fs::OpenOptions::new().read(true).write(true).open(path) {
            Ok(f) => return Ok(Box::new(f)),
            Err(e) => last = e,
        }
    }
    #[cfg(unix)]
    {
        let mut dirs: Vec<std::path::PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .iter()
            .filter_map(std::env::var_os)
            .map(Into::into)
            .collect();
        dirs.push("/tmp".into());
        for dir in dirs {
            // Flatpak and Snap Discord put the socket in a subfolder.
            for sub in ["", "app/com.discordapp.Discord", "snap.discord"] {
                for i in 0..10 {
                    let p = dir.join(sub).join(format!("discord-ipc-{i}"));
                    match std::os::unix::net::UnixStream::connect(&p) {
                        Ok(s) => {
                            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                            return Ok(Box::new(s));
                        }
                        Err(e) => last = e,
                    }
                }
            }
        }
    }
    Err(last)
}

struct Worker {
    config: watch::Receiver<Config>,
    snapshot: watch::Receiver<Snapshot>,
    connect: Connector,
    status: watch::Sender<DiscordStatus>,
    stop: Arc<AtomicBool>,
    started: u64,
}

struct Session {
    ipc: Ipc,
    app_id: String,
    user: Option<String>,
    sent: Option<Value>,
    sent_at: Option<Instant>,
}

impl Worker {
    fn set_status(&self, s: DiscordStatus) {
        self.status.send_if_modified(|cur| {
            if *cur == s {
                false
            } else {
                *cur = s;
                true
            }
        });
    }

    fn run(self) {
        let mut session: Option<Session> = None;
        let mut next_probe = Instant::now();
        let mut logged_error: Option<String> = None;
        while !self.stop.load(Ordering::SeqCst) {
            let cfg = self.config.borrow().discord.clone();
            let id = app_id(&cfg);
            let changed_id = session.as_ref().is_some_and(|s| Some(&s.app_id) != id.as_ref());
            if !cfg.enabled || id.is_none() || changed_id {
                if let Some(mut s) = session.take() {
                    let _ = s.ipc.set_activity(None);
                    tracing::info!("Discord Rich Presence stopped");
                }
                next_probe = Instant::now();
                logged_error = None;
                if cfg.enabled && id.is_some() {
                    continue; // the app ID changed: reconnect with the new one now
                }
                self.set_status(DiscordStatus::of(if cfg.enabled {
                    DiscordState::NoAppId
                } else {
                    DiscordState::Off
                }));
                std::thread::sleep(TICK);
                continue;
            }
            let id = id.unwrap_or_default();

            if session.is_none() {
                if Instant::now() >= next_probe {
                    next_probe = Instant::now() + PROBE_EVERY;
                    match (self.connect)() {
                        Err(_) => self.set_status(DiscordStatus::of(DiscordState::NotRunning)),
                        Ok(pipe) => {
                            let mut ipc = Ipc { pipe };
                            match ipc.handshake(&id) {
                                Ok(user) => {
                                    tracing::info!("connected to Discord for Rich Presence");
                                    logged_error = None;
                                    session = Some(Session {
                                        ipc,
                                        app_id: id.clone(),
                                        user,
                                        sent: None,
                                        sent_at: None,
                                    });
                                }
                                Err(IpcError::Io(_)) => {
                                    self.set_status(DiscordStatus::of(DiscordState::NotRunning))
                                }
                                Err(IpcError::Refused(msg)) => {
                                    if logged_error.as_ref() != Some(&msg) {
                                        tracing::warn!(error = %msg, "Discord refused Rich Presence");
                                        logged_error = Some(msg.clone());
                                    }
                                    next_probe = Instant::now() + PROBE_EVERY * 3;
                                    self.set_status(DiscordStatus {
                                        state: DiscordState::Error,
                                        user: None,
                                        message: Some(msg),
                                    });
                                }
                            }
                        }
                    }
                }
                if session.is_none() {
                    std::thread::sleep(TICK);
                    continue;
                }
            }

            let s = session.as_mut().expect("connected");
            let want = activity(&self.snapshot.borrow(), &cfg, self.started);
            let due = s.sent_at.is_none_or(|t| t.elapsed() >= MIN_UPDATE);
            if s.sent.as_ref() != Some(&want) && due {
                match s.ipc.set_activity(Some(&want)) {
                    Ok(()) => {
                        s.sent = Some(want);
                        s.sent_at = Some(Instant::now());
                    }
                    Err(IpcError::Refused(msg)) => {
                        // Bad activity data (e.g. a button Discord dislikes): report, keep the link.
                        if logged_error.as_ref() != Some(&msg) {
                            tracing::warn!(error = %msg, "Discord rejected the presence");
                            logged_error = Some(msg);
                        }
                        s.sent = Some(want);
                        s.sent_at = Some(Instant::now());
                    }
                    Err(IpcError::Io(e)) => {
                        tracing::info!(error = %e, "lost the connection to Discord");
                        session = None;
                        next_probe = Instant::now() + PROBE_EVERY;
                        self.set_status(DiscordStatus::of(DiscordState::NotRunning));
                        continue;
                    }
                }
            }
            let user = s.user.clone();
            self.set_status(DiscordStatus {
                state: DiscordState::Connected,
                user,
                message: logged_error.clone(),
            });
            std::thread::sleep(TICK);
        }
        if let Some(mut s) = session.take() {
            let _ = s.ipc.set_activity(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::DeviceInfo;
    use std::collections::VecDeque;

    fn cfg() -> DiscordConfig {
        DiscordConfig {
            enabled: true,
            app_id: "123".into(),
            ..Default::default()
        }
    }

    #[test]
    fn activity_shows_bpm_zone_device_and_link() {
        let mut s = Snapshot {
            bpm: Some(88),
            data_valid: true,
            zone: 2,
            device: Some(DeviceInfo {
                id: "x".into(),
                name: "Polar H10".into(),
                location: None,
                demo: false,
            }),
            ..Default::default()
        };
        let a = activity(&s, &cfg(), 1000);
        assert_eq!(a["details"], "\u{2764}\u{fe0f} 88 bpm");
        assert_eq!(a["state"], "Zone 2 \u{b7} Polar H10");
        assert_eq!(a["timestamps"]["start"], 1000);
        assert_eq!(a["buttons"][0]["url"], REPO_URL);
        assert_eq!(a["assets"]["large_image"], ICON_URL);

        s.data_valid = false;
        let a = activity(&s, &cfg(), 1000);
        assert_eq!(a["details"], "Waiting for heart rate");
        assert_eq!(a["state"], "Polar H10", "no zone without a reading");

        let off = DiscordConfig {
            show_zone: false,
            show_device: false,
            button: false,
            ..cfg()
        };
        let a = activity(&s, &off, 1000);
        assert!(a.get("state").is_none() && a.get("buttons").is_none());
    }

    #[test]
    fn frames_round_trip() {
        let f = encode_frame(OP_FRAME, &json!({"a": 1}));
        assert_eq!(&f[..4], &1u32.to_le_bytes());
        assert_eq!(&f[4..8], &7u32.to_le_bytes());
        let (op, v) = read_frame(&mut &f[..]).unwrap();
        assert_eq!((op, v), (OP_FRAME, json!({"a": 1})));
        let mut huge = 1u32.to_le_bytes().to_vec();
        huge.extend_from_slice(&(MAX_FRAME + 1).to_le_bytes());
        assert!(read_frame(&mut &huge[..]).is_err());
    }

    #[test]
    fn app_id_falls_back_to_default() {
        assert_eq!(app_id(&cfg()).as_deref(), Some("123"));
        let none = DiscordConfig::default();
        assert_eq!(
            app_id(&none),
            (!DEFAULT_APP_ID.is_empty()).then(|| DEFAULT_APP_ID.to_string())
        );
    }

    /// A fake Discord: answers the handshake with READY and every command with an ack, and
    /// records what KittyTooth sent.
    struct FakeDiscord {
        inbox: Arc<Mutex<Vec<(u32, Value)>>>,
        out: VecDeque<u8>,
        buf: Vec<u8>,
    }

    impl Read for FakeDiscord {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            let n = b.len().min(self.out.len());
            for x in b.iter_mut().take(n) {
                *x = self.out.pop_front().unwrap_or_default();
            }
            Ok(n)
        }
    }

    impl Write for FakeDiscord {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.buf.extend_from_slice(b);
            while let Ok((op, v)) = read_frame(&mut &self.buf[..]) {
                let len = u32::from_le_bytes(self.buf[4..8].try_into().unwrap()) as usize;
                self.buf.drain(..8 + len);
                let reply = if op == OP_HANDSHAKE {
                    json!({"cmd": "DISPATCH", "evt": "READY", "data": {"user": {"username": "kitty"}}})
                } else {
                    json!({"cmd": v["cmd"], "nonce": v["nonce"], "evt": null})
                };
                self.out.extend(encode_frame(OP_FRAME, &reply));
                self.inbox.lock().unwrap().push((op, v));
            }
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn wait_for(rx: &watch::Receiver<DiscordStatus>, want: DiscordState) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if rx.borrow().state == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn connects_sets_presence_and_clears_on_shutdown() {
        let config = Config {
            discord: cfg(),
            ..Default::default()
        };
        let (_cfg_tx, cfg_rx) = watch::channel(config);
        let snap = Snapshot {
            bpm: Some(70),
            data_valid: true,
            ..Default::default()
        };
        let (_snap_tx, snap_rx) = watch::channel(snap);
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let i2 = inbox.clone();
        let h = spawn_with(
            cfg_rx,
            snap_rx,
            Box::new(move || {
                Ok(Box::new(FakeDiscord {
                    inbox: i2.clone(),
                    out: VecDeque::new(),
                    buf: Vec::new(),
                }) as Box<dyn Pipe>)
            }),
        );
        let rx = h.subscribe();
        assert!(wait_for(&rx, DiscordState::Connected));
        assert_eq!(h.status().user.as_deref(), Some("kitty"));
        h.shutdown();
        let got = inbox.lock().unwrap().clone();
        assert_eq!(got[0].1["client_id"], "123");
        assert_eq!(got[1].1["cmd"], "SET_ACTIVITY");
        assert_eq!(got[1].1["args"]["activity"]["details"], "\u{2764}\u{fe0f} 70 bpm");
        let last = &got.last().unwrap().1;
        assert!(last["args"].get("activity").is_none(), "presence cleared on quit");
    }

    #[test]
    fn reports_missing_app_id_and_closed_discord() {
        let mut config = Config::default();
        config.discord.enabled = true;
        config.discord.app_id = String::new();
        let (cfg_tx, cfg_rx) = watch::channel(config.clone());
        let (_snap_tx, snap_rx) = watch::channel(Snapshot::default());
        let h = spawn_with(
            cfg_rx,
            snap_rx,
            Box::new(|| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "closed"))),
        );
        let rx = h.subscribe();
        if DEFAULT_APP_ID.is_empty() {
            assert!(wait_for(&rx, DiscordState::NoAppId));
        }
        config.discord.app_id = "42".into();
        cfg_tx.send_replace(config);
        assert!(wait_for(&rx, DiscordState::NotRunning));
        h.shutdown();
    }
}
