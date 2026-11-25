//! The engine: one task owning connection management, processing and OSC output.
//!
//! BLE notification → parse → process → OSC encode → UDP send happens synchronously inside
//! this task, so OSC never depends on the UI. The UI observes the engine through a `watch`
//! snapshot, a broadcast event stream and a few shared buffers (history, OSC log).

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::beat::{BeatScheduler, BeatSource};
use crate::config::{Config, ConfigStore, DisconnectBehavior, RememberedDevice};
use crate::error::{KtError, KtResult, UserError};
use crate::hrm::{self, SensorContact};
use crate::osc::chatbox;
use crate::osc::oscquery::{self, OscQueryEvent, OscQueryHandle, VrchatEndpoint};
use crate::osc::presets::{self, Metrics, Role};
use crate::osc::sender::{epoch_ms, OscLog, OscLogEntry, Outgoing, TargetSender, TargetStats};
use crate::processing::{Processor, SampleOutcome, SessionStats};
use crate::sensor::{
    AdapterState, DeviceInfo, DiscoveredDevice, HeartRateBackend, SensorEvent, SensorSession,
};

/// Reconnect delay for the n-th consecutive failure (n >= 1): 1, 2, 4, 8, 10, 10, ... s.
pub fn backoff(attempt: u32) -> Duration {
    let secs = 1u64 << attempt.saturating_sub(1).min(4);
    Duration::from_secs(secs.min(10))
}

const TICK: Duration = Duration::from_millis(250);
const PULSE: Duration = Duration::from_millis(100);
const SWEEP_LEN: Duration = Duration::from_secs(16);
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(45);
const HISTORY_CAP: usize = 6 * 60 * 60 * 4; // 6 h at 4 samples/s

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnState {
    /// Not trying to connect.
    #[default]
    Idle,
    Connecting,
    /// Link up and data flowing.
    Connected,
    /// Link up but no valid reading (no contact / waiting for first packet).
    NoData,
    /// Waiting to retry after a failure or dropout.
    Reconnecting,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LatencyStats {
    pub last_us: u64,
    pub avg_us: u64,
    pub max_us: u64,
    pub samples: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TargetStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub dest: Option<String>,
    pub via_oscquery: bool,
    pub stats: TargetStats,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct OscQueryStatus {
    pub active: bool,
    pub mdns: bool,
    pub http_port: Option<u16>,
    pub osc_port: Option<u16>,
    pub error: Option<String>,
}

/// Everything the UI needs to render, published on every change.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Snapshot {
    pub state: ConnState,
    pub attempt: u32,
    /// Unix ms of the next reconnect attempt.
    pub retry_at_ms: Option<u64>,
    pub device: Option<DeviceInfo>,
    /// The device we are connected or trying to connect to.
    pub target: Option<RememberedDevice>,
    /// Displayed BPM (smoothed if enabled).
    pub bpm: Option<u16>,
    pub raw_bpm: Option<u16>,
    /// OSC "Connected": valid data within the stale timeout.
    pub data_valid: bool,
    pub contact: SensorContact,
    pub stats: SessionStats,
    pub hrv_ms: Option<f64>,
    pub zone: u8,
    pub max_hr: u16,
    pub energy_kj: Option<u16>,
    pub battery: Option<u8>,
    pub last_packet_ms: Option<u64>,
    pub adapter: AdapterState,
    pub error: Option<UserError>,
    pub scanning: bool,
    pub test_sweep: bool,
    pub targets: Vec<TargetStatus>,
    pub vrchat: Option<VrchatEndpoint>,
    pub oscquery: OscQueryStatus,
    pub latency: LatencyStats,
    pub rejected_spikes: u64,
    pub rejected_out_of_range: u64,
    pub packets: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct HistoryPoint {
    pub t_ms: u64,
    pub bpm: f32,
    pub raw: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    Beat {
        t_ms: u64,
        interval_ms: u64,
        source: BeatSource,
        test: bool,
    },
    Sample(HistoryPoint),
    /// Config changed (by the engine itself, e.g. remembering a device).
    ConfigChanged,
}

#[derive(Debug)]
pub enum Command {
    SetConfig(Box<Config>, oneshot::Sender<KtResult<()>>),
    StartScan,
    StopScan,
    Connect(RememberedDevice),
    Disconnect,
    Forget,
    TestSweep(bool),
    ResetSession,
    Shutdown(oneshot::Sender<()>),
}

/// Shared buffers readable without going through the engine task.
#[derive(Debug, Default)]
pub struct Shared {
    pub osc_log: Mutex<OscLog>,
    pub history: Mutex<VecDeque<HistoryPoint>>,
}

#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// Allow OSCQuery mDNS (tests disable it).
    pub mdns: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self { mdns: true }
    }
}

/// Cheap, cloneable handle to the engine.
#[derive(Clone)]
pub struct EngineHandle {
    cmd: mpsc::Sender<Command>,
    snapshot: watch::Receiver<Snapshot>,
    devices: watch::Receiver<Vec<DiscoveredDevice>>,
    config: watch::Receiver<Config>,
    events: broadcast::Sender<EngineEvent>,
    shared: Arc<Shared>,
}

impl EngineHandle {
    async fn send(&self, c: Command) -> KtResult<()> {
        self.cmd.send(c).await.map_err(|_| KtError::EngineStopped)
    }

    pub async fn set_config(&self, cfg: Config) -> KtResult<()> {
        let (tx, rx) = oneshot::channel();
        self.send(Command::SetConfig(Box::new(cfg), tx)).await?;
        rx.await.map_err(|_| KtError::EngineStopped)?
    }

    pub async fn start_scan(&self) -> KtResult<()> {
        self.send(Command::StartScan).await
    }

    pub async fn stop_scan(&self) -> KtResult<()> {
        self.send(Command::StopScan).await
    }

    pub async fn connect(&self, id: &str, name: &str) -> KtResult<()> {
        self.send(Command::Connect(RememberedDevice {
            id: id.to_string(),
            name: name.to_string(),
        }))
        .await
    }

    pub async fn disconnect(&self) -> KtResult<()> {
        self.send(Command::Disconnect).await
    }

    pub async fn forget(&self) -> KtResult<()> {
        self.send(Command::Forget).await
    }

    pub async fn test_sweep(&self, on: bool) -> KtResult<()> {
        self.send(Command::TestSweep(on)).await
    }

    pub async fn reset_session(&self) -> KtResult<()> {
        self.send(Command::ResetSession).await
    }

    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Command::Shutdown(tx)).await.is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(3), rx).await;
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.borrow().clone()
    }

    pub fn subscribe_snapshot(&self) -> watch::Receiver<Snapshot> {
        self.snapshot.clone()
    }

    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        self.devices.borrow().clone()
    }

    pub fn subscribe_devices(&self) -> watch::Receiver<Vec<DiscoveredDevice>> {
        self.devices.clone()
    }

    pub fn config(&self) -> Config {
        self.config.borrow().clone()
    }

    pub fn subscribe_config(&self) -> watch::Receiver<Config> {
        self.config.clone()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    pub fn osc_log(&self) -> Vec<OscLogEntry> {
        self.shared
            .osc_log
            .lock()
            .map(|l| l.entries())
            .unwrap_or_default()
    }

    /// History points newer than `since_ms` (0 = everything).
    pub fn history(&self, since_ms: u64) -> Vec<HistoryPoint> {
        self.shared
            .history
            .lock()
            .map(|h| h.iter().filter(|p| p.t_ms > since_ms).copied().collect())
            .unwrap_or_default()
    }
}

/// Starts the engine on the current tokio runtime.
pub fn start(
    config: Config,
    store: Option<ConfigStore>,
    backend: Arc<dyn HeartRateBackend>,
    opts: EngineOptions,
) -> EngineHandle {
    let config = config.sanitized();
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (snap_tx, snap_rx) = watch::channel(Snapshot::default());
    let (dev_tx, dev_rx) = watch::channel(Vec::new());
    let (cfg_tx, cfg_rx) = watch::channel(config.clone());
    let (ev_tx, _) = broadcast::channel(256);
    let shared = Arc::new(Shared::default());
    let handle = EngineHandle {
        cmd: cmd_tx,
        snapshot: snap_rx,
        devices: dev_rx,
        config: cfg_rx,
        events: ev_tx.clone(),
        shared: shared.clone(),
    };
    backend.set_demo_enabled(config.device.demo_device);
    let engine = Engine::new(
        config, store, backend, opts, snap_tx, dev_tx, cfg_tx, ev_tx, shared,
    );
    tokio::spawn(engine.run(cmd_rx));
    handle
}

type AttemptResult = (u64, KtResult<Box<dyn SensorSession>>, mpsc::Receiver<SensorEvent>);

struct Sweep {
    started: Instant,
}

struct Engine {
    config: Config,
    store: Option<ConfigStore>,
    backend: Arc<dyn HeartRateBackend>,
    opts: EngineOptions,
    snap_tx: watch::Sender<Snapshot>,
    dev_tx: watch::Sender<Vec<DiscoveredDevice>>,
    cfg_tx: watch::Sender<Config>,
    ev_tx: broadcast::Sender<EngineEvent>,
    shared: Arc<Shared>,

    snap: Snapshot,
    dirty: bool,

    // connection
    desired: Option<RememberedDevice>,
    session: Option<Box<dyn SensorSession>>,
    session_rx: Option<mpsc::Receiver<SensorEvent>>,
    attempt_gen: u64,
    attempt_inflight: bool,
    attempt_count: u32,
    next_attempt_at: Option<Instant>,
    attempt_tx: mpsc::Sender<AttemptResult>,
    attempt_rx: mpsc::Receiver<AttemptResult>,
    last_packet: Option<Instant>,

    // scanning
    scan_tx: mpsc::Sender<DiscoveredDevice>,
    scan_rx: mpsc::Receiver<DiscoveredDevice>,
    devices: HashMap<String, DiscoveredDevice>,
    devices_dirty: bool,
    last_devices_publish: Instant,

    // adapter polling
    adapter_tx: mpsc::Sender<AdapterState>,
    adapter_rx: mpsc::Receiver<AdapterState>,
    last_adapter_poll: Option<Instant>,

    // processing
    processor: Processor,
    beat: BeatScheduler,
    beat_toggle: bool,
    beat_pulse: bool,
    pulse_off_at: Option<Instant>,
    recent: VecDeque<(Instant, u16)>,
    last_valid_at: Option<Instant>,

    // OSC
    socket_v4: Option<UdpSocket>,
    socket_v6: Option<UdpSocket>,
    senders: Vec<TargetSender>,
    disconnect_sent: bool,
    chatbox_phase: bool,
    resolve_tx: mpsc::Sender<(u64, String, Option<SocketAddr>)>,
    resolve_rx: mpsc::Receiver<(u64, String, Option<SocketAddr>)>,
    targets_gen: u64,
    oscq: Option<OscQueryHandle>,
    oscq_tx: mpsc::Sender<OscQueryEvent>,
    oscq_rx: mpsc::Receiver<OscQueryEvent>,
    sweep: Option<Sweep>,
    latency_sum_us: u64,
}

async fn recv_opt<T>(rx: &mut Option<mpsc::Receiver<T>>) -> Option<T> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

impl Engine {
    #[allow(clippy::too_many_arguments)]
    fn new(
        config: Config,
        store: Option<ConfigStore>,
        backend: Arc<dyn HeartRateBackend>,
        opts: EngineOptions,
        snap_tx: watch::Sender<Snapshot>,
        dev_tx: watch::Sender<Vec<DiscoveredDevice>>,
        cfg_tx: watch::Sender<Config>,
        ev_tx: broadcast::Sender<EngineEvent>,
        shared: Arc<Shared>,
    ) -> Self {
        let (attempt_tx, attempt_rx) = mpsc::channel(4);
        let (scan_tx, scan_rx) = mpsc::channel(256);
        let (adapter_tx, adapter_rx) = mpsc::channel(4);
        let (resolve_tx, resolve_rx) = mpsc::channel(16);
        let (oscq_tx, oscq_rx) = mpsc::channel(16);
        let desired = if config.device.auto_connect {
            config.device.last_device.clone()
        } else {
            None
        };
        let processor = Processor::new(config.processing.smoothing);
        Self {
            config,
            store,
            backend,
            opts,
            snap_tx,
            dev_tx,
            cfg_tx,
            ev_tx,
            shared,
            snap: Snapshot::default(),
            dirty: true,
            desired,
            session: None,
            session_rx: None,
            attempt_gen: 0,
            attempt_inflight: false,
            attempt_count: 0,
            next_attempt_at: Some(Instant::now()),
            attempt_tx,
            attempt_rx,
            last_packet: None,
            scan_tx,
            scan_rx,
            devices: HashMap::new(),
            devices_dirty: false,
            last_devices_publish: Instant::now(),
            adapter_tx,
            adapter_rx,
            last_adapter_poll: None,
            processor,
            beat: BeatScheduler::default(),
            beat_toggle: false,
            beat_pulse: false,
            pulse_off_at: None,
            recent: VecDeque::new(),
            last_valid_at: None,
            socket_v4: None,
            socket_v6: None,
            senders: Vec::new(),
            disconnect_sent: false,
            chatbox_phase: false,
            resolve_tx,
            resolve_rx,
            targets_gen: 0,
            oscq: None,
            oscq_tx,
            oscq_rx,
            sweep: None,
            latency_sum_us: 0,
        }
    }

    async fn run(mut self, mut cmd_rx: mpsc::Receiver<Command>) {
        self.socket_v4 = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok();
        if self.socket_v4.is_none() {
            self.snap.error = Some(KtError::OscSocket("could not open a UDP socket".into()).user());
        }
        self.snap.max_hr = self.config.zones.max_hr();
        self.snap.target = self.desired.clone();
        self.apply_oscquery().await;
        self.rebuild_senders();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let now = Instant::now();
            let wake = self.next_wake(now);
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(Command::Shutdown(done)) => {
                            self.shutdown().await;
                            let _ = done.send(());
                            return;
                        }
                        Some(c) => self.on_command(c).await,
                        None => { self.shutdown().await; return; }
                    }
                }
                ev = recv_opt(&mut self.session_rx) => self.on_sensor_event(ev).await,
                Some(res) = self.attempt_rx.recv() => self.on_attempt(res).await,
                Some(d) = self.scan_rx.recv() => self.on_discovered(d),
                Some(s) = self.adapter_rx.recv() => self.on_adapter(s),
                Some(r) = self.resolve_rx.recv() => self.on_resolved(r),
                Some(e) = self.oscq_rx.recv() => self.on_oscquery(e),
                _ = tokio::time::sleep_until(wake.into()) => self.on_timers(Instant::now()),
                _ = tick.tick() => self.on_tick(Instant::now()),
            }
            self.publish();
        }
    }

    // ---------- timing ----------

    fn next_wake(&mut self, now: Instant) -> Instant {
        let far = now + Duration::from_secs(3600);
        let mut w = far;
        if self.beats_active(now) {
            if let Some(b) = self.beat.next_beat_at(now) {
                w = w.min(b);
            }
        }
        if let Some(p) = self.pulse_off_at {
            w = w.min(p);
        }
        for s in &mut self.senders {
            if let Some(f) = s.next_flush_at(now) {
                w = w.min(f);
            }
        }
        w.max(now)
    }

    fn beats_active(&self, now: Instant) -> bool {
        self.sweep.is_some() || self.data_valid(now)
    }

    fn data_valid(&self, now: Instant) -> bool {
        self.last_valid_at
            .is_some_and(|t| now.saturating_duration_since(t) <= self.stale_timeout())
    }

    fn stale_timeout(&self) -> Duration {
        Duration::from_secs(self.config.device.stale_timeout_secs as u64)
    }

    /// Tracks the valid/invalid transition (e.g. sustained contact loss).
    fn update_validity(&mut self, now: Instant) {
        let valid = self.data_valid(now);
        if valid != self.snap.data_valid {
            self.snap.data_valid = valid;
            self.dirty = true;
            if !valid {
                self.beat.reset();
                if self.session.is_some() {
                    self.set_state(ConnState::NoData);
                }
            }
        }
    }

    fn on_timers(&mut self, now: Instant) {
        let mut emit = false;
        if let Some(p) = self.pulse_off_at {
            if now >= p {
                self.pulse_off_at = None;
                self.beat_pulse = false;
                emit = true;
            }
        }
        if self.beats_active(now) {
            if let Some(b) = self.beat.poll(now) {
                self.beat_toggle = !self.beat_toggle;
                self.beat_pulse = true;
                self.pulse_off_at = Some(now + PULSE);
                let _ = self.ev_tx.send(EngineEvent::Beat {
                    t_ms: epoch_ms(),
                    interval_ms: b.interval.as_millis() as u64,
                    source: b.source,
                    test: self.sweep.is_some(),
                });
                emit = true;
            }
        }
        if emit {
            self.emit_osc(now, false, false);
        } else {
            self.flush(now);
        }
    }

    fn on_tick(&mut self, now: Instant) {
        // Test sweep progression.
        if let Some(s) = &self.sweep {
            if now.saturating_duration_since(s.started) >= SWEEP_LEN {
                self.stop_sweep();
            } else {
                let bpm = sweep_bpm(now.saturating_duration_since(s.started));
                self.beat.set_bpm(Some(bpm as f64));
            }
        }

        // Stale detection: no packets at all → link is dead; reconnect.
        if self.session.is_some() {
            if let Some(t) = self.last_packet {
                if now.saturating_duration_since(t) > self.stale_timeout() {
                    tracing::warn!(
                        timeout_s = self.config.device.stale_timeout_secs,
                        "sensor data stale; reconnecting"
                    );
                    self.on_link_lost(None, now);
                }
            }
        }

        // Data validity (e.g. sustained contact loss) → OSC disconnected frame.
        self.update_validity(now);

        // Reconnect scheduling.
        if self.desired.is_some() && self.session.is_none() && !self.attempt_inflight {
            if let Some(at) = self.next_attempt_at {
                if now >= at {
                    self.spawn_attempt();
                }
            }
        }

        // Adapter polling every 3 s while not connected (cheap, async, off-loop).
        if self.session.is_none()
            && self
                .last_adapter_poll
                .is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(3))
        {
            self.last_adapter_poll = Some(now);
            let b = self.backend.clone();
            let tx = self.adapter_tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(b.adapter_state().await).await;
            });
        }

        // Device list: prune and publish at most 2 Hz.
        let before = self.devices.len();
        let cutoff = epoch_ms().saturating_sub(60_000);
        self.devices.retain(|_, d| d.last_seen_ms >= cutoff);
        if self.devices.len() != before {
            self.devices_dirty = true;
        }
        if self.devices_dirty
            && now.saturating_duration_since(self.last_devices_publish) >= Duration::from_millis(500)
        {
            let mut list: Vec<DiscoveredDevice> = self.devices.values().cloned().collect();
            list.sort_by(|a, b| {
                b.has_hr_service
                    .cmp(&a.has_hr_service)
                    .then(b.rssi.cmp(&a.rssi))
                    .then(a.name.cmp(&b.name))
            });
            let _ = self.dev_tx.send(list);
            self.devices_dirty = false;
            self.last_devices_publish = now;
        }

        // Keepalive / sweep values / chatbox.
        self.emit_osc(now, false, true);
        self.chatbox(now);

        if self.data_valid(now) {
            let hrv = self.processor.hrv_ms(now);
            if hrv != self.snap.hrv_ms {
                self.snap.hrv_ms = hrv;
                self.dirty = true;
            }
        }
    }

    // ---------- commands ----------

    async fn on_command(&mut self, c: Command) {
        match c {
            Command::SetConfig(cfg, reply) => {
                let r = self.set_config(*cfg).await;
                let _ = reply.send(r);
            }
            Command::StartScan => {
                self.snap.scanning = true;
                self.dirty = true;
                if let Err(e) = self
                    .backend
                    .start_scan(self.config.device.show_all_devices, self.scan_tx.clone())
                    .await
                {
                    tracing::info!(error = %e, "scan start failed");
                    self.snap.error = Some(e.user());
                } else if matches!(
                    self.snap.error.as_ref().map(|e| e.code.as_str()),
                    Some("radio_off" | "adapter_missing")
                ) {
                    self.snap.error = None;
                }
            }
            Command::StopScan => {
                let _ = self.backend.stop_scan().await;
                self.snap.scanning = false;
                self.dirty = true;
            }
            Command::Connect(dev) => {
                tracing::info!(id = %dev.id, name = %dev.name, "connect requested");
                self.teardown_session().await;
                self.desired = Some(dev.clone());
                self.attempt_count = 0;
                self.next_attempt_at = Some(Instant::now());
                self.snap.error = None;
                self.snap.target = Some(dev.clone());
                if self.config.device.last_device.as_ref() != Some(&dev) {
                    self.config.device.last_device = Some(dev);
                    self.persist_config();
                }
                self.set_state(ConnState::Connecting);
                self.spawn_attempt();
            }
            Command::Disconnect => {
                tracing::info!("disconnect requested");
                self.desired = None;
                self.teardown_session().await;
                self.mark_invalid(Instant::now());
                self.snap.target = None;
                self.snap.error = None;
                self.snap.retry_at_ms = None;
                self.snap.attempt = 0;
                self.set_state(ConnState::Idle);
            }
            Command::Forget => {
                self.desired = None;
                self.teardown_session().await;
                self.mark_invalid(Instant::now());
                self.snap.target = None;
                self.snap.error = None;
                self.snap.retry_at_ms = None;
                self.set_state(ConnState::Idle);
                if self.config.device.last_device.take().is_some() {
                    self.persist_config();
                }
            }
            Command::TestSweep(on) => {
                if on {
                    self.sweep = Some(Sweep {
                        started: Instant::now(),
                    });
                    self.beat.reset();
                    self.beat.set_bpm(Some(60.0));
                    for s in &mut self.senders {
                        s.reset_chatbox();
                    }
                    self.snap.test_sweep = true;
                    self.dirty = true;
                } else {
                    self.stop_sweep();
                }
            }
            Command::ResetSession => {
                self.processor.reset_session();
                self.recent.clear();
                if let Ok(mut h) = self.shared.history.lock() {
                    h.clear();
                }
                self.snap.stats = SessionStats::default();
                self.dirty = true;
            }
            Command::Shutdown(_) => {}
        }
    }

    fn stop_sweep(&mut self) {
        if self.sweep.take().is_some() {
            self.beat.reset();
            self.snap.test_sweep = false;
            self.dirty = true;
            // Restore the real state on the avatar.
            for s in &mut self.senders {
                s.invalidate();
            }
            self.disconnect_sent = false;
            let now = Instant::now();
            if let Some(b) = self.snap.bpm {
                if self.data_valid(now) {
                    self.beat.set_bpm(Some(b as f64));
                }
            }
            self.emit_osc(now, true, false);
        }
    }

    async fn set_config(&mut self, cfg: Config) -> KtResult<()> {
        let mut cfg = cfg.sanitized();
        for t in &cfg.osc.targets {
            for m in &t.custom {
                crate::osc::normalize_address(&m.address)?;
            }
        }
        // The remembered device is owned by the engine.
        if cfg.device.last_device.is_none()
            && self.config.device.last_device.is_some()
            && self.desired.is_some()
        {
            cfg.device.last_device = self.config.device.last_device.clone();
        }
        let old = std::mem::replace(&mut self.config, cfg);
        self.processor.smoothing = self.config.processing.smoothing;
        self.backend.set_demo_enabled(self.config.device.demo_device);
        if old.osc.targets != self.config.osc.targets || old.osc.max_rate != self.config.osc.max_rate {
            self.rebuild_senders();
        }
        if old.osc.on_disconnect != self.config.osc.on_disconnect && !self.data_valid(Instant::now()) {
            self.disconnect_sent = false;
        }
        if old.osc.oscquery != self.config.osc.oscquery {
            self.apply_oscquery().await;
        }
        if old.device.show_all_devices != self.config.device.show_all_devices && self.snap.scanning {
            self.devices.clear();
            self.devices_dirty = true;
            let _ = self.backend.stop_scan().await;
            if let Err(e) = self
                .backend
                .start_scan(self.config.device.show_all_devices, self.scan_tx.clone())
                .await
            {
                self.snap.error = Some(e.user());
            }
        }
        self.snap.max_hr = self.config.zones.max_hr();
        self.dirty = true;
        let r = match &self.store {
            Some(s) => s.save(&self.config),
            None => Ok(()),
        };
        let _ = self.cfg_tx.send(self.config.clone());
        r
    }

    fn persist_config(&mut self) {
        if let Some(s) = &self.store {
            if let Err(e) = s.save(&self.config) {
                tracing::warn!(error = %e, "could not save config");
            }
        }
        let _ = self.cfg_tx.send(self.config.clone());
        let _ = self.ev_tx.send(EngineEvent::ConfigChanged);
    }

    // ---------- connection ----------

    fn spawn_attempt(&mut self) {
        let Some(dev) = self.desired.clone() else {
            return;
        };
        self.attempt_gen += 1;
        self.attempt_inflight = true;
        self.next_attempt_at = None;
        self.snap.retry_at_ms = None;
        self.set_state(if self.attempt_count == 0 {
            ConnState::Connecting
        } else {
            ConnState::Reconnecting
        });
        let gen = self.attempt_gen;
        let backend = self.backend.clone();
        let tx = self.attempt_tx.clone();
        tokio::spawn(async move {
            let (ev_tx, ev_rx) = mpsc::channel(64);
            let res = match tokio::time::timeout(
                CONNECT_ATTEMPT_TIMEOUT,
                backend.connect(&dev.id, &dev.name, ev_tx),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => Err(KtError::ConnectTimeout(dev.name.clone())),
            };
            let _ = tx.send((gen, res, ev_rx)).await;
        });
    }

    async fn on_attempt(&mut self, (gen, res, rx): AttemptResult) {
        if gen != self.attempt_gen || self.desired.is_none() {
            if let Ok(mut s) = res {
                s.disconnect().await;
            }
            return;
        }
        self.attempt_inflight = false;
        let now = Instant::now();
        match res {
            Ok(session) => {
                let info = session.info();
                tracing::info!(name = %info.name, "sensor connected");
                self.session = Some(session);
                self.session_rx = Some(rx);
                self.attempt_count = 0;
                self.last_packet = Some(now);
                self.processor.on_reconnect();
                self.beat.reset();
                self.snap.device = Some(info);
                self.snap.error = None;
                self.snap.attempt = 0;
                self.snap.retry_at_ms = None;
                self.snap.battery = None;
                self.set_state(ConnState::NoData);
            }
            Err(e) => {
                tracing::info!(error = %e, attempt = self.attempt_count + 1, "connect failed");
                self.snap.error = Some(e.user());
                self.schedule_retry(now);
            }
        }
    }

    fn schedule_retry(&mut self, now: Instant) {
        self.attempt_count = self.attempt_count.saturating_add(1);
        let delay = backoff(self.attempt_count);
        self.next_attempt_at = Some(now + delay);
        self.snap.attempt = self.attempt_count;
        self.snap.retry_at_ms = Some(epoch_ms() + delay.as_millis() as u64);
        self.set_state(ConnState::Reconnecting);
    }

    fn on_link_lost(&mut self, reason: Option<KtError>, now: Instant) {
        if let Some(mut s) = self.session.take() {
            tokio::spawn(async move { s.disconnect().await });
        }
        self.session_rx = None;
        self.last_packet = None;
        self.mark_invalid(now);
        if let Some(e) = reason {
            self.snap.error = Some(e.user());
        }
        if self.desired.is_some() {
            self.schedule_retry(now);
        } else {
            self.set_state(ConnState::Idle);
        }
    }

    async fn teardown_session(&mut self) {
        self.attempt_gen += 1; // invalidates in-flight attempts
        self.attempt_inflight = false;
        self.next_attempt_at = None;
        if let Some(mut s) = self.session.take() {
            s.disconnect().await;
        }
        self.session_rx = None;
        self.last_packet = None;
        self.snap.device = None;
        self.snap.battery = None;
        self.dirty = true;
    }

    fn mark_invalid(&mut self, now: Instant) {
        self.last_valid_at = None;
        self.beat.reset();
        self.beat_pulse = false;
        self.pulse_off_at = None;
        self.snap.data_valid = false;
        self.dirty = true;
        self.emit_osc(now, false, false);
    }

    fn set_state(&mut self, s: ConnState) {
        if self.snap.state != s {
            self.snap.state = s;
            self.dirty = true;
        }
    }

    // ---------- sensor data ----------

    async fn on_sensor_event(&mut self, ev: Option<SensorEvent>) {
        let now = Instant::now();
        match ev {
            Some(SensorEvent::Measurement { data, received }) => self.on_measurement(&data, received, now),
            Some(SensorEvent::Battery(b)) => {
                self.snap.battery = Some(b);
                self.dirty = true;
            }
            Some(SensorEvent::Disconnected(reason)) => {
                tracing::info!(reason = ?reason, "sensor link lost");
                self.on_link_lost(reason, now);
            }
            None => self.on_link_lost(Some(KtError::Ble("connection closed".into())), now),
        }
    }

    fn on_measurement(&mut self, data: &[u8], received: Instant, now: Instant) {
        self.last_packet = Some(now);
        self.snap.packets += 1;
        self.snap.last_packet_ms = Some(epoch_ms());
        let m = match hrm::parse(data) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(error = %e, bytes = ?data, "unparseable measurement");
                return;
            }
        };
        self.snap.contact = m.contact;
        if m.energy_expended_kj.is_some() {
            self.snap.energy_kj = m.energy_expended_kj;
        }
        let outcome = self.processor.push_bpm(m.bpm, m.contact.has_contact(), now);
        if m.contact.has_contact() {
            for rr in m.rr_ms() {
                self.processor.push_rr(rr, now);
                self.beat.push_rr(Duration::from_secs_f64(rr / 1000.0), now);
            }
        }
        if let SampleOutcome::Accepted { raw, smoothed } = outcome {
            self.last_valid_at = Some(now);
            let shown = smoothed.round() as u16;
            self.snap.bpm = Some(shown);
            self.snap.raw_bpm = Some(raw);
            self.snap.data_valid = true;
            self.snap.stats = self.processor.stats();
            self.snap.zone = self.config.zones.zone(shown);
            self.snap.max_hr = self.config.zones.max_hr();
            self.beat.set_bpm(Some(smoothed));
            self.recent.push_back((now, raw));
            while self
                .recent
                .front()
                .is_some_and(|(t, _)| now.saturating_duration_since(*t) > Duration::from_secs(10))
            {
                self.recent.pop_front();
            }
            let point = HistoryPoint {
                t_ms: epoch_ms(),
                bpm: smoothed as f32,
                raw,
            };
            if let Ok(mut h) = self.shared.history.lock() {
                if h.len() >= HISTORY_CAP {
                    h.pop_front();
                }
                h.push_back(point);
            }
            let _ = self.ev_tx.send(EngineEvent::Sample(point));
            if self.session.is_some() {
                self.set_state(ConnState::Connected);
            }
        }
        self.snap.rejected_spikes = self.processor.rejected_spikes();
        self.snap.rejected_out_of_range = self.processor.rejected_out_of_range();
        self.dirty = true;

        // Hot path: send changed values right away.
        self.emit_osc(now, false, false);
        let us = received.elapsed().as_micros() as u64;
        let l = &mut self.snap.latency;
        l.last_us = us;
        l.max_us = l.max_us.max(us);
        l.samples += 1;
        self.latency_sum_us += us;
        l.avg_us = self.latency_sum_us / l.samples;
        if self.config.debug.log_latency {
            tracing::debug!(latency_us = us, bpm = m.bpm, "ble->osc latency");
        }
    }

    fn metrics(&mut self, now: Instant) -> Metrics {
        if let Some(s) = &self.sweep {
            let bpm = sweep_bpm(now.saturating_duration_since(s.started)).round() as u16;
            return Metrics {
                connected: true,
                hr: bpm,
                hr_raw: bpm,
                max_hr: self.config.zones.max_hr(),
                zone: self.config.zones.zone(bpm),
                hrv_ms: Some(45.0),
                battery: Some(100),
                beat_toggle: self.beat_toggle,
                beat_pulse: self.beat_pulse,
                session_min: Some(60),
                session_avg: Some(120.0),
                session_max: Some(180),
                avg_10s: Some(bpm as f64),
            };
        }
        let avg_10s = if self.recent.is_empty() {
            None
        } else {
            Some(self.recent.iter().map(|(_, v)| *v as f64).sum::<f64>() / self.recent.len() as f64)
        };
        let stats = self.processor.stats();
        let hr = self.snap.bpm.unwrap_or(0);
        Metrics {
            connected: self.data_valid(now),
            hr,
            hr_raw: self.snap.raw_bpm.unwrap_or(0),
            max_hr: self.config.zones.max_hr(),
            zone: self.config.zones.zone(hr),
            hrv_ms: self.processor.hrv_ms(now),
            battery: self.snap.battery,
            beat_toggle: self.beat_toggle,
            beat_pulse: self.beat_pulse,
            session_min: stats.min,
            session_avg: stats.avg,
            session_max: stats.max,
            avg_10s,
        }
    }

    // ---------- OSC ----------

    fn rebuild_senders(&mut self) {
        self.targets_gen += 1;
        let gen = self.targets_gen;
        self.senders = self
            .config
            .osc
            .targets
            .iter()
            .filter(|t| t.enabled)
            .map(|t| TargetSender::new(t.clone(), None, self.config.osc.max_rate))
            .collect();
        for s in &mut self.senders {
            let host = s.target.host.clone();
            let port = s.target.port;
            match host.parse::<IpAddr>() {
                Ok(ip) => s.dest = Some(SocketAddr::new(ip, port)),
                Err(_) => {
                    let tx = self.resolve_tx.clone();
                    let id = s.target.id.clone();
                    tokio::spawn(async move {
                        let lookup = tokio::time::timeout(
                            Duration::from_secs(3),
                            tokio::net::lookup_host((host.as_str(), port)),
                        )
                        .await;
                        let addr = match lookup {
                            Ok(Ok(mut it)) => it.find(|a| a.is_ipv4()),
                            _ => None,
                        };
                        let _ = tx.send((gen, id, addr)).await;
                    });
                }
            }
        }
        self.apply_vrchat_override();
        self.disconnect_sent = false;
        self.update_target_status();
        let now = Instant::now();
        self.emit_osc(now, true, false);
    }

    fn on_resolved(&mut self, (gen, id, addr): (u64, String, Option<SocketAddr>)) {
        if gen != self.targets_gen {
            return;
        }
        if let Some(s) = self.senders.iter_mut().find(|s| s.target.id == id) {
            match addr {
                Some(a) => s.dest = Some(a),
                None => {
                    s.stats.last_error = Some(
                        KtError::InvalidOscTarget(format!("{}:{}", s.target.host, s.target.port))
                            .user()
                            .message,
                    );
                }
            }
        }
        self.apply_vrchat_override();
        self.update_target_status();
        self.emit_osc(Instant::now(), true, false);
    }

    fn apply_vrchat_override(&mut self) {
        let vr = self.snap.vrchat.clone();
        for s in &mut self.senders {
            let manual = s
                .target
                .host
                .parse::<IpAddr>()
                .ok()
                .map(|ip| SocketAddr::new(ip, s.target.port));
            let host_is_local = s.target.host == "localhost"
                || s.target.host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
            let over = match (&vr, s.target.use_oscquery) {
                (Some(v), true) if v.local && host_is_local => {
                    Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), v.port))
                }
                (Some(v), true) if !v.local && s.target.host.parse::<IpAddr>().ok() == Some(v.ip) => {
                    Some(SocketAddr::new(v.ip, v.port))
                }
                _ => None,
            };
            if let Some(o) = over {
                if s.dest != Some(o) {
                    s.dest = Some(o);
                    s.invalidate();
                }
            } else if manual.is_some() && s.dest != manual {
                s.dest = manual;
                s.invalidate();
            }
        }
    }

    fn update_target_status(&mut self) {
        let vr = self.snap.vrchat.as_ref().map(|v| v.port);
        let enabled: HashMap<&str, &TargetSender> =
            self.senders.iter().map(|s| (s.target.id.as_str(), s)).collect();
        let status: Vec<TargetStatus> = self
            .config
            .osc
            .targets
            .iter()
            .map(|t| {
                let s = enabled.get(t.id.as_str());
                TargetStatus {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    enabled: t.enabled,
                    dest: s.and_then(|s| s.dest).map(|d| d.to_string()),
                    via_oscquery: s
                        .and_then(|s| s.dest)
                        .is_some_and(|d| t.use_oscquery && Some(d.port()) == vr && d.port() != t.port),
                    stats: s.map(|s| s.stats.clone()).unwrap_or_default(),
                }
            })
            .collect();
        if status != self.snap.targets {
            self.snap.targets = status;
            self.dirty = true;
        }
    }

    /// Offers current values to every sender and flushes.
    /// `force`: resend everything. `keepalive`: refresh values older than the keepalive.
    fn emit_osc(&mut self, now: Instant, force: bool, keepalive: bool) {
        self.update_validity(now);
        let metrics = self.metrics(now);
        if metrics.connected {
            self.disconnect_sent = false;
            let ka = (keepalive && self.config.osc.keepalive_ms > 0)
                .then(|| Duration::from_millis(self.config.osc.keepalive_ms as u64));
            for s in &mut self.senders {
                let msgs = presets::messages(&s.target, &metrics);
                s.offer(&msgs, now, ka, force);
            }
        } else if !self.disconnect_sent || force {
            // Send Connected=false (and zeros if configured) once, then stay quiet.
            self.disconnect_sent = true;
            let zero = self.config.osc.on_disconnect == DisconnectBehavior::SendZero;
            let m = if zero { metrics.zeroed() } else { metrics };
            for s in &mut self.senders {
                let msgs: Vec<_> = presets::messages(&s.target, &m)
                    .into_iter()
                    .filter(|p| zero || p.role == Role::Connection)
                    .collect();
                s.offer(&msgs, now, None, true);
            }
        }
        self.flush(now);
    }

    fn flush(&mut self, now: Instant) {
        let test = self.sweep.is_some();
        let mut out: Vec<(usize, Outgoing)> = Vec::new();
        for (i, s) in self.senders.iter_mut().enumerate() {
            for o in s.drain(now, test) {
                out.push((i, o));
            }
        }
        if out.is_empty() {
            return;
        }
        self.send_all(out);
    }

    fn send_all(&mut self, out: Vec<(usize, Outgoing)>) {
        let shared = self.shared.clone();
        let mut log = shared.osc_log.lock().ok();
        for (i, o) in out {
            let res = self.send_datagram(&o);
            if let Some(s) = self.senders.get_mut(i) {
                s.record_result(res);
            }
            if let Some(l) = log.as_mut() {
                l.push(o.log);
            }
        }
        drop(log);
        self.update_target_status();
    }

    fn send_datagram(&mut self, o: &Outgoing) -> Result<(), String> {
        let sock = if o.dest.is_ipv4() {
            self.socket_v4.as_ref()
        } else {
            if self.socket_v6.is_none() {
                self.socket_v6 = std::net::UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0))
                    .and_then(|s| {
                        s.set_nonblocking(true)?;
                        Ok(s)
                    })
                    .ok()
                    .and_then(|s| UdpSocket::from_std(s).ok());
            }
            self.socket_v6.as_ref()
        };
        let Some(sock) = sock else {
            return Err("no UDP socket".into());
        };
        sock.try_send_to(&o.bytes, o.dest).map(|_| ()).map_err(|e| {
            tracing::debug!(error = %e, dest = %o.dest, "OSC send failed");
            KtError::OscSocket(e.to_string()).user().message
        })
    }

    fn chatbox(&mut self, now: Instant) {
        let cb = &self.config.osc.chatbox;
        if !cb.enabled {
            return;
        }
        let metrics = self.metrics(now);
        if !metrics.connected {
            return;
        }
        let text = chatbox::render(&self.config.osc.chatbox.template, &metrics, self.chatbox_phase);
        let interval = Duration::from_secs_f64(self.config.osc.chatbox.interval_secs.max(2.0));
        let silent = self.config.osc.chatbox.silent;
        let test = self.sweep.is_some();
        let mut out = Vec::new();
        for (i, s) in self.senders.iter_mut().enumerate() {
            if s.target.chatbox {
                if let Some(o) = s.chatbox(&text, silent, interval, now, test) {
                    out.push((i, o));
                }
            }
        }
        if !out.is_empty() {
            self.chatbox_phase = !self.chatbox_phase;
            self.send_all(out);
        }
    }

    async fn apply_oscquery(&mut self) {
        if self.config.osc.oscquery {
            if self.oscq.is_none() {
                match oscquery::start(self.opts.mdns, self.oscq_tx.clone()).await {
                    Ok(h) => {
                        self.snap.oscquery = OscQueryStatus {
                            active: true,
                            mdns: h.mdns_active,
                            http_port: Some(h.http_port),
                            osc_port: Some(h.osc_port),
                            error: h.mdns_error.clone(),
                        };
                        self.oscq = Some(h);
                    }
                    Err(e) => {
                        self.snap.oscquery = OscQueryStatus {
                            error: Some(e.user().message),
                            ..Default::default()
                        };
                    }
                }
            }
        } else {
            self.oscq = None;
            self.snap.oscquery = OscQueryStatus::default();
            if self.snap.vrchat.take().is_some() {
                self.apply_vrchat_override();
            }
        }
        self.dirty = true;
    }

    fn on_oscquery(&mut self, e: OscQueryEvent) {
        match e {
            OscQueryEvent::VrchatFound(ep) => {
                self.snap.vrchat = Some(ep);
                self.apply_vrchat_override();
            }
            OscQueryEvent::VrchatLost(name) => {
                if self.snap.vrchat.as_ref().is_some_and(|v| v.service == name) {
                    self.snap.vrchat = None;
                    self.apply_vrchat_override();
                }
            }
            OscQueryEvent::AvatarChanged => {
                tracing::info!("avatar changed; resending all parameters");
                for s in &mut self.senders {
                    s.invalidate();
                }
                self.disconnect_sent = false;
                self.emit_osc(Instant::now(), true, false);
            }
        }
        self.update_target_status();
        self.dirty = true;
    }

    // ---------- scanning & adapter ----------

    fn on_discovered(&mut self, d: DiscoveredDevice) {
        if !self.snap.scanning {
            return;
        }
        if !self.config.device.show_all_devices && !d.has_hr_service {
            return;
        }
        self.devices.insert(d.id.clone(), d);
        self.devices_dirty = true;
    }

    fn on_adapter(&mut self, s: AdapterState) {
        if self.snap.adapter != s {
            tracing::info!(state = ?s, "adapter state");
            self.snap.adapter = s;
            self.dirty = true;
            // Clear a stale radio error once the radio is back; reconnect promptly.
            if s == AdapterState::Ready {
                if matches!(
                    self.snap.error.as_ref().map(|e| e.code.as_str()),
                    Some("radio_off" | "adapter_missing")
                ) {
                    self.snap.error = None;
                }
                if self.desired.is_some() && self.session.is_none() && !self.attempt_inflight {
                    self.next_attempt_at = Some(Instant::now());
                }
            }
        }
    }

    // ---------- output ----------

    fn publish(&mut self) {
        if self.dirty {
            self.dirty = false;
            let snap = self.snap.clone();
            self.snap_tx.send_if_modified(|s| {
                if *s != snap {
                    *s = snap;
                    true
                } else {
                    false
                }
            });
        }
    }

    async fn shutdown(&mut self) {
        self.desired = None;
        self.teardown_session().await;
        self.mark_invalid(Instant::now());
        let _ = self.backend.stop_scan().await;
        self.oscq = None;
    }
}

/// Test sweep: 60 → 180 → 60 bpm over [`SWEEP_LEN`].
pub fn sweep_bpm(elapsed: Duration) -> f32 {
    let x = (elapsed.as_secs_f32() / SWEEP_LEN.as_secs_f32()).clamp(0.0, 1.0);
    let tri = 1.0 - (2.0 * x - 1.0).abs();
    60.0 + 120.0 * tri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_sequence() {
        let s: Vec<u64> = (1..=7).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(s, vec![1, 2, 4, 8, 10, 10, 10]);
        assert_eq!(backoff(1000).as_secs(), 10);
    }

    #[test]
    fn sweep_shape() {
        assert_eq!(sweep_bpm(Duration::ZERO), 60.0);
        assert_eq!(sweep_bpm(SWEEP_LEN / 2), 180.0);
        assert_eq!(sweep_bpm(SWEEP_LEN), 60.0);
    }
}
