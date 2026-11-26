//! The engine: one task owning connection management, processing and OSC output.
//!
//! BLE notification → parse → process → OSC encode → UDP send happens synchronously inside
//! this task, so OSC never depends on the UI. The UI observes the engine through a `watch`
//! snapshot, a broadcast event stream and a few shared buffers (history, OSC log).

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use std::net::UdpSocket;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::alerts::{AlertAction, AlertKind, AlertMonitor, AlertStatus};
use crate::beat::{BeatScheduler, BeatSource};
use crate::config::{Config, ConfigStore, DisconnectBehavior, RememberedDevice};
use crate::error::{KtError, KtResult, UserError};
use crate::history::{self, RecordOp, Recorder, SessionMeta, SessionStore, WriterMsg};
use crate::hrm::SensorContact;
use crate::osc::chatbox;
use crate::osc::oscquery::{self, OscQueryEvent, OscQueryHandle, VrchatEndpoint};
use crate::osc::presets::{self, Metrics, Role};
use crate::osc::sender::{epoch_ms, OscLog, OscLogEntry, Outgoing, TargetSender, TargetStats};
use crate::processing::{Processor, SampleOutcome, SessionStats, MAX_VALID_BPM, MIN_VALID_BPM};
use crate::sensor::{
    AdapterState, DeviceInfo, DiscoveredDevice, HeartRateBackend, SensorEvent, SensorSession,
};
use crate::source::{
    Capabilities, DefaultFactory, HeartRateSource, HrSample, MemorySecrets, SourceFactory, SourceKind,
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
/// The main source must deliver this long before the engine switches back from the backup.
pub const FALLBACK_RECOVER: Duration = Duration::from_secs(3);
/// Rate-limited services are retried no sooner than this.
const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(30);
const PRIMARY: usize = 0;
const BACKUP: usize = 1;

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
    /// Last scan failure (adapter problems are reported via `adapter`).
    pub scan_error: Option<UserError>,
    pub scanning: bool,
    pub test_sweep: bool,
    pub targets: Vec<TargetStatus>,
    pub vrchat: Option<VrchatEndpoint>,
    pub oscquery: OscQueryStatus,
    pub latency: LatencyStats,
    pub rejected_spikes: u64,
    pub rejected_out_of_range: u64,
    pub packets: u64,
    /// The chosen main source.
    pub source: SourceKind,
    /// The source feeding readings right now (`None` while there is no valid data).
    pub live_source: Option<SourceKind>,
    /// The backup source is standing in for a stale main source.
    pub fallback_active: bool,
    /// What the live (or main) source can report; missing items show as "not available".
    pub capabilities: Capabilities,
    /// Main and backup source connections.
    pub sources: Vec<SlotStatus>,
    /// High or low heart rate alert showing right now.
    pub alert: Option<AlertStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SlotRole {
    #[default]
    Primary,
    Backup,
}

/// Connection status of the main or backup source.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SlotStatus {
    pub role: SlotRole,
    pub kind: SourceKind,
    pub label: Option<String>,
    pub state: ConnState,
    pub error: Option<UserError>,
    pub last_packet_ms: Option<u64>,
    /// This source is the one feeding readings right now.
    pub live: bool,
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
    /// A heart rate alert fired (or a reminder for one that is still showing).
    Alert {
        alert_kind: AlertKind,
        bpm: u16,
        threshold: u16,
        repeat: bool,
        test: bool,
    },
    /// A saved session began, ended or was removed by retention.
    SessionsChanged,
}

#[derive(Debug)]
pub enum Command {
    SetConfig(Box<Config>, oneshot::Sender<KtResult<()>>),
    StartScan,
    StopScan,
    Connect(RememberedDevice),
    Disconnect,
    Forget,
    /// Makes a source kind the main source and starts it.
    SelectSource(SourceKind),
    /// Reconnects running sources (after a token or key changed).
    RestartSources,
    TestSweep(bool),
    /// Fires a sample high alert so the user can check notifications and the chatbox.
    TestAlert,
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
    /// Network sources may send only when the value changes, so their link is declared dead
    /// after `max(stale timeout, this)` of silence rather than the plain stale timeout.
    pub network_link_floor: Duration,
    /// Bluetooth links are only declared dead after `max(learned stale timeout, this)` of
    /// silence. Windows reports real disconnects; silence alone usually means a slow sender
    /// (phone apps relaying a watch send every 5-25 s), and reconnecting loses readings.
    pub ble_link_floor: Duration,
    /// Folder for saved sessions; `None` records nothing.
    pub history_dir: Option<PathBuf>,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            mdns: true,
            network_link_floor: Duration::from_secs(20),
            ble_link_floor: Duration::from_secs(30),
            history_dir: None,
        }
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
    sessions: Option<SessionStore>,
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

    pub async fn select_source(&self, kind: SourceKind) -> KtResult<()> {
        self.send(Command::SelectSource(kind)).await
    }

    pub async fn restart_sources(&self) -> KtResult<()> {
        self.send(Command::RestartSources).await
    }

    pub async fn test_sweep(&self, on: bool) -> KtResult<()> {
        self.send(Command::TestSweep(on)).await
    }

    pub async fn test_alert(&self) -> KtResult<()> {
        self.send(Command::TestAlert).await
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

    /// Saved sessions, when history is set up (`EngineOptions::history_dir`).
    pub fn sessions(&self) -> Option<&SessionStore> {
        self.sessions.as_ref()
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

/// Starts the engine with the default source factory and in-memory secrets.
pub fn start(
    config: Config,
    store: Option<ConfigStore>,
    backend: Arc<dyn HeartRateBackend>,
    opts: EngineOptions,
) -> EngineHandle {
    let factory = Arc::new(DefaultFactory::new(backend, Arc::new(MemorySecrets::default())));
    start_with_factory(config, store, factory, opts)
}

/// Starts the engine on the current tokio runtime.
pub fn start_with_factory(
    config: Config,
    store: Option<ConfigStore>,
    factory: Arc<dyn SourceFactory>,
    opts: EngineOptions,
) -> EngineHandle {
    let backend = factory.backend();
    let config = config.sanitized();
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (snap_tx, snap_rx) = watch::channel(Snapshot::default());
    let (dev_tx, dev_rx) = watch::channel(Vec::new());
    let (cfg_tx, cfg_rx) = watch::channel(config.clone());
    let (ev_tx, _) = broadcast::channel(256);
    let shared = Arc::new(Shared::default());
    let sessions = opts.history_dir.as_ref().map(SessionStore::new);
    let history_tx = sessions.as_ref().map(|store| {
        let ev = ev_tx.clone();
        let tx = history::spawn_writer(store.clone(), move || {
            let _ = ev.send(EngineEvent::SessionsChanged);
        });
        let _ = tx.send(WriterMsg::Prune(config.history.clone(), epoch_ms()));
        tx
    });
    let handle = EngineHandle {
        cmd: cmd_tx,
        snapshot: snap_rx,
        devices: dev_rx,
        config: cfg_rx,
        events: ev_tx.clone(),
        shared: shared.clone(),
        sessions,
    };
    backend.set_demo_enabled(config.device.demo_device);
    let mut engine = Engine::new(
        config, store, factory, backend, opts, snap_tx, dev_tx, cfg_tx, ev_tx, shared,
    );
    engine.history_tx = history_tx;
    tokio::spawn(engine.run(cmd_rx));
    handle
}

type AttemptResult = (
    usize,
    u64,
    KtResult<Box<dyn SensorSession>>,
    mpsc::Receiver<SensorEvent>,
);

/// One source connection (main or backup) with its own reconnect loop.
#[derive(Default)]
struct Slot {
    kind: Option<SourceKind>,
    source: Option<Arc<dyn HeartRateSource>>,
    session: Option<Box<dyn SensorSession>>,
    gen: u64,
    inflight: bool,
    attempts: u32,
    next_at: Option<Instant>,
    retry_at_ms: Option<u64>,
    /// Stopped retrying (rejected credentials, missing setup) until the user changes something.
    halted: bool,
    started: Option<Instant>,
    last_packet: Option<Instant>,
    last_packet_ms: Option<u64>,
    last_valid: Option<Instant>,
    valid_since: Option<Instant>,
    state: ConnState,
    error: Option<UserError>,
    device: Option<DeviceInfo>,
    battery: Option<u8>,
    /// Packets this connection, and how many carried RR intervals. Some sensors (Garmin
    /// watches in broadcast mode, many optical bands) never send RR, so HRV is unavailable.
    packets: u32,
    rr_packets: u32,
    /// Recent gaps between valid readings, to learn how often this source sends.
    gaps: VecDeque<Duration>,
}

/// Gaps longer than this are dropouts, not the source's pace.
const MAX_LEARNED_GAP: Duration = Duration::from_secs(60);
/// How many recent gaps the learned pace looks at.
const GAP_WINDOW: usize = 12;

/// After this many packets without a single RR interval, HRV is reported as unavailable.
const NO_RR_AFTER_PACKETS: u32 = 15;

impl Slot {
    /// The connected device has shown it sends no RR intervals.
    fn lacks_rr(&self) -> bool {
        self.packets >= NO_RR_AFTER_PACKETS && self.rr_packets == 0
    }

    /// Records a valid reading and the gap since the previous one.
    fn mark_valid(&mut self, now: Instant) {
        if let Some(prev) = self.last_valid {
            let gap = now.saturating_duration_since(prev);
            if gap <= MAX_LEARNED_GAP {
                if self.gaps.len() >= GAP_WINDOW {
                    self.gaps.pop_front();
                }
                self.gaps.push_back(gap);
            }
        }
        self.last_valid = Some(now);
    }

    /// Stale timeout adapted to this source's pace: never below `base`, and for sources that
    /// send slowly (every 10-20 s), long enough to cover their usual gaps (capped at 60 s).
    fn learned_stale(&self, base: Duration) -> Duration {
        learned_stale(&self.gaps, base)
    }

    fn fresh(&self, now: Instant, within: Duration) -> bool {
        self.last_valid
            .is_some_and(|t| now.saturating_duration_since(t) <= within)
    }
}

struct Sweep {
    started: Instant,
}

struct Engine {
    config: Config,
    store: Option<ConfigStore>,
    factory: Arc<dyn SourceFactory>,
    backend: Arc<dyn HeartRateBackend>,
    opts: EngineOptions,
    snap_tx: watch::Sender<Snapshot>,
    dev_tx: watch::Sender<Vec<DiscoveredDevice>>,
    cfg_tx: watch::Sender<Config>,
    ev_tx: broadcast::Sender<EngineEvent>,
    shared: Arc<Shared>,

    snap: Snapshot,
    dirty: bool,

    // sources
    /// The user wants sources running (auto-connect, or Connect/Select pressed).
    running: bool,
    slots: [Slot; 2],

    // saved session history
    recorder: Recorder,
    history_tx: Option<std::sync::mpsc::Sender<WriterMsg>>,
    /// Event receivers of the slots' sessions (separate fields so `select!` can poll both).
    primary_rx: Option<mpsc::Receiver<SensorEvent>>,
    backup_rx: Option<mpsc::Receiver<SensorEvent>>,
    /// Index of the slot feeding readings.
    live: usize,
    engine_error: Option<UserError>,
    attempt_tx: mpsc::Sender<AttemptResult>,
    attempt_rx: mpsc::Receiver<AttemptResult>,

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

    // alerts
    alerts: AlertMonitor,
    /// A test alert shows until this time.
    test_alert_until: Option<Instant>,
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
        factory: Arc<dyn SourceFactory>,
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
        let running = config.device.auto_connect;
        let processor = Processor::new(config.processing.smoothing);
        Self {
            config,
            store,
            factory,
            backend,
            opts,
            snap_tx,
            dev_tx,
            cfg_tx,
            ev_tx,
            shared,
            snap: Snapshot::default(),
            dirty: true,
            running,
            slots: Default::default(),
            primary_rx: None,
            backup_rx: None,
            live: PRIMARY,
            engine_error: None,
            attempt_tx,
            attempt_rx,
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
            alerts: AlertMonitor::default(),
            recorder: Recorder::default(),
            history_tx: None,
            test_alert_until: None,
            latency_sum_us: 0,
        }
    }

    async fn run(mut self, mut cmd_rx: mpsc::Receiver<Command>) {
        // Plain non-blocking std sockets: UDP sends complete synchronously on the hot path
        // (tokio's `try_send_to` can report WouldBlock before readiness is first observed).
        self.socket_v4 = nonblocking_udp((Ipv4Addr::UNSPECIFIED, 0).into());
        if self.socket_v4.is_none() {
            self.engine_error = Some(KtError::OscSocket("could not open a UDP socket".into()).user());
        }
        self.snap.max_hr = self.config.zones.max_hr();
        self.apply_oscquery().await;
        self.rebuild_senders();
        self.reconcile(false, false).await;
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
                ev = recv_opt(&mut self.primary_rx) => self.on_sensor_event(PRIMARY, ev).await,
                ev = recv_opt(&mut self.backup_rx) => self.on_sensor_event(BACKUP, ev).await,
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
            .is_some_and(|t| now.saturating_duration_since(t) <= self.live_stale())
    }

    /// Stale timeout for the live source, adapted to how often it sends.
    fn live_stale(&self) -> Duration {
        self.slots[self.live].learned_stale(self.stale_timeout())
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
                let live = &mut self.slots[self.live];
                live.valid_since = None;
                if live.session.is_some() {
                    live.state = ConnState::NoData;
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
        let ops = self.recorder.tick(epoch_ms());
        self.record(ops);
        // Test sweep progression.
        if let Some(s) = &self.sweep {
            if now.saturating_duration_since(s.started) >= SWEEP_LEN {
                self.stop_sweep();
            } else {
                let bpm = sweep_bpm(now.saturating_duration_since(s.started));
                self.beat.set_bpm(Some(bpm as f64));
            }
        }

        for i in [PRIMARY, BACKUP] {
            // Stale detection: no packets at all → link is dead; reconnect.
            let link_timeout = self.link_timeout(i);
            let slot = &self.slots[i];
            if let (Some(_), Some(t), Some(limit)) = (&slot.session, slot.last_packet, link_timeout) {
                if now.saturating_duration_since(t) > limit {
                    tracing::warn!(
                        slot = i,
                        timeout_s = limit.as_secs(),
                        "source data stale; reconnecting"
                    );
                    self.on_link_lost(i, None, now);
                }
            }
            // A standby source with a link but no recent reading.
            if i != self.live {
                let stale = self.slots[i].learned_stale(self.stale_timeout());
                let slot = &mut self.slots[i];
                if slot.session.is_some() && slot.state == ConnState::Connected && !slot.fresh(now, stale) {
                    slot.state = ConnState::NoData;
                    slot.valid_since = None;
                }
            }
            // Reconnect scheduling.
            let slot = &self.slots[i];
            if slot.source.is_some()
                && slot.session.is_none()
                && !slot.inflight
                && !slot.halted
                && slot.next_at.is_some_and(|at| now >= at)
            {
                self.spawn_attempt(i);
            }
        }

        // Data validity (e.g. sustained contact loss) → OSC disconnected frame.
        self.update_validity(now);
        self.evaluate_live(now);

        // Adapter polling every 3 s while no Bluetooth link is up (cheap, async, off-loop).
        let ble_connected = self
            .slots
            .iter()
            .any(|s| s.session.is_some() && matches!(s.kind, Some(SourceKind::Ble | SourceKind::Demo)));
        if !ble_connected
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

        self.check_alerts(now);

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
                self.snap.scan_error = match self
                    .backend
                    .start_scan(self.config.device.show_all_devices, self.scan_tx.clone())
                    .await
                {
                    Ok(()) => None,
                    Err(e) => {
                        tracing::info!(error = %e, "scan start failed");
                        Some(e.user())
                    }
                };
            }
            Command::StopScan => {
                let _ = self.backend.stop_scan().await;
                self.snap.scanning = false;
                self.dirty = true;
            }
            Command::Connect(dev) => {
                tracing::info!(id = %dev.id, name = %dev.name, "connect requested");
                let kind = SourceKind::of_device(&dev.id);
                let changed = self.config.device.last_device.as_ref() != Some(&dev)
                    || self.config.source.active != kind;
                self.config.device.last_device = Some(dev);
                self.config.source.active = kind;
                if changed {
                    self.persist_config();
                }
                self.running = true;
                self.reconcile(true, false).await;
            }
            Command::SelectSource(kind) => {
                tracing::info!(source = kind.tag(), "source selected");
                if self.config.source.active != kind {
                    self.config.source.active = kind;
                    self.persist_config();
                }
                self.running = true;
                self.reconcile(true, false).await;
            }
            Command::RestartSources => {
                if self.running {
                    self.reconcile(true, true).await;
                }
            }
            Command::Disconnect => {
                tracing::info!("disconnect requested");
                self.running = false;
                self.reconcile(false, false).await;
            }
            Command::Forget => {
                self.running = false;
                self.reconcile(false, false).await;
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
            Command::TestAlert => self.test_alert(Instant::now()),
            Command::ResetSession => {
                let ops = self.recorder.end();
                self.record(ops);
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
        if cfg.device.last_device.is_none() && self.config.device.last_device.is_some() && self.running {
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
            self.snap.scan_error = self
                .backend
                .start_scan(self.config.device.show_all_devices, self.scan_tx.clone())
                .await
                .err()
                .map(|e| e.user());
        }
        if old.source != self.config.source
            || old.device.stale_timeout_secs != self.config.device.stale_timeout_secs
        {
            let changed = |k: SourceKind| match k {
                SourceKind::Hyperate => old.source.hyperate != self.config.source.hyperate,
                SourceKind::Osc => old.source.osc_input != self.config.source.osc_input,
                _ => false,
            };
            let (p, b) = (
                changed(self.config.source.active),
                changed(self.config.source.fallback.backup),
            );
            self.reconcile(p, b).await;
        }
        if old.history != self.config.history {
            if !self.config.history.enabled {
                let ops = self.recorder.end();
                self.record(ops);
            }
            if let Some(tx) = &self.history_tx {
                let _ = tx.send(WriterMsg::Prune(self.config.history.clone(), epoch_ms()));
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

    // ---------- sources ----------

    fn rx_mut(&mut self, i: usize) -> &mut Option<mpsc::Receiver<SensorEvent>> {
        if i == PRIMARY {
            &mut self.primary_rx
        } else {
            &mut self.backup_rx
        }
    }

    /// Link timeout for a source kind: `None` for sources without a link (UDP input).
    fn link_timeout(&self, i: usize) -> Option<Duration> {
        let kind = self.slots[i].kind?;
        if !kind.reconnects_on_silence() {
            return None;
        }
        let stale = self.slots[i].learned_stale(self.stale_timeout());
        Some(if kind.is_network() {
            stale.max(self.opts.network_link_floor)
        } else if matches!(kind, SourceKind::Ble | SourceKind::Demo) {
            stale.max(self.opts.ble_link_floor)
        } else {
            stale
        })
    }

    /// Brings both slots in line with what should run. `restart_*` forces a reconnect even if
    /// the kind is unchanged (its settings or credentials changed).
    async fn reconcile(&mut self, restart_primary: bool, restart_backup: bool) {
        let fb = self.config.source.fallback.clone();
        let active = self.config.source.active;
        let want = [
            self.running.then_some(active),
            (self.running && fb.enabled && fb.backup != active).then_some(fb.backup),
        ];
        for (i, restart) in [(PRIMARY, restart_primary), (BACKUP, restart_backup)] {
            if self.slots[i].kind != want[i] || (restart && want[i].is_some()) {
                self.stop_slot(i).await;
                if let Some(k) = want[i] {
                    self.start_slot(i, k);
                }
            }
        }
        if self.live != PRIMARY && self.slots[self.live].kind.is_none() {
            self.switch_live(PRIMARY, Instant::now());
        }
        self.dirty = true;
    }

    fn start_slot(&mut self, i: usize, kind: SourceKind) {
        let now = Instant::now();
        let built = self.factory.build(kind, &self.config);
        let slot = &mut self.slots[i];
        *slot = Slot {
            gen: slot.gen + 1,
            kind: Some(kind),
            started: Some(now),
            ..Default::default()
        };
        match built {
            Ok(src) => {
                tracing::info!(slot = i, source = kind.tag(), label = %src.label(), "starting source");
                slot.source = Some(src);
                self.spawn_attempt(i);
            }
            Err(e) => {
                tracing::info!(slot = i, source = kind.tag(), error = %e, "source not ready");
                slot.halted = true;
                // Bluetooth without a chosen sensor simply waits for the user.
                if e != KtError::NoSensorChosen {
                    slot.error = Some(e.user());
                }
            }
        }
    }

    async fn stop_slot(&mut self, i: usize) {
        let was_live_with_data = i == self.live && self.slots[i].kind.is_some();
        let slot = &mut self.slots[i];
        let gen = slot.gen + 1; // invalidates in-flight attempts
        let session = slot.session.take();
        *slot = Slot {
            gen,
            ..Default::default()
        };
        *self.rx_mut(i) = None;
        if let Some(mut s) = session {
            s.disconnect().await;
        }
        if was_live_with_data {
            self.mark_invalid(Instant::now());
        }
        self.dirty = true;
    }

    fn spawn_attempt(&mut self, i: usize) {
        let slot = &mut self.slots[i];
        let Some(src) = slot.source.clone() else {
            return;
        };
        slot.gen += 1;
        slot.inflight = true;
        slot.next_at = None;
        slot.retry_at_ms = None;
        slot.state = if slot.attempts == 0 {
            ConnState::Connecting
        } else {
            ConnState::Reconnecting
        };
        self.dirty = true;
        let gen = slot.gen;
        let tx = self.attempt_tx.clone();
        tokio::spawn(async move {
            let (ev_tx, ev_rx) = mpsc::channel(64);
            let res = match tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, src.connect(ev_tx)).await {
                Ok(r) => r,
                Err(_) => Err(KtError::ConnectTimeout(src.label())),
            };
            let _ = tx.send((i, gen, res, ev_rx)).await;
        });
    }

    async fn on_attempt(&mut self, (i, gen, res, rx): AttemptResult) {
        let slot = &mut self.slots[i];
        if gen != slot.gen || slot.source.is_none() {
            if let Ok(mut s) = res {
                s.disconnect().await;
            }
            return;
        }
        slot.inflight = false;
        let now = Instant::now();
        match res {
            Ok(session) => {
                let info = session.info();
                tracing::info!(slot = i, name = %info.name, "source connected");
                slot.session = Some(session);
                slot.attempts = 0;
                slot.last_packet = Some(now);
                slot.device = Some(info);
                slot.error = None;
                slot.retry_at_ms = None;
                slot.battery = None;
                slot.packets = 0;
                slot.rr_packets = 0;
                slot.gaps.clear();
                slot.state = ConnState::NoData;
                *self.rx_mut(i) = Some(rx);
                if i == self.live {
                    self.processor.on_reconnect();
                    self.beat.reset();
                }
            }
            Err(e) => {
                tracing::info!(slot = i, error = %e, attempt = slot.attempts + 1, "connect failed");
                self.fail_slot(i, e, now);
            }
        }
        self.dirty = true;
    }

    /// Records an error and either schedules a retry or, for errors retrying can't fix, stops.
    fn fail_slot(&mut self, i: usize, e: KtError, now: Instant) {
        let slot = &mut self.slots[i];
        slot.error = Some(e.user());
        if !e.retriable() {
            slot.halted = true;
            slot.next_at = None;
            slot.retry_at_ms = None;
            slot.state = ConnState::Idle;
            return;
        }
        slot.attempts = slot.attempts.saturating_add(1);
        let mut delay = backoff(slot.attempts);
        if matches!(e, KtError::RateLimited(_)) {
            delay = delay.max(RATE_LIMIT_BACKOFF);
        }
        slot.next_at = Some(now + delay);
        slot.retry_at_ms = Some(epoch_ms() + delay.as_millis() as u64);
        slot.state = ConnState::Reconnecting;
    }

    fn on_link_lost(&mut self, i: usize, reason: Option<KtError>, now: Instant) {
        let slot = &mut self.slots[i];
        if let Some(mut s) = slot.session.take() {
            tokio::spawn(async move { s.disconnect().await });
        }
        slot.last_packet = None;
        slot.last_valid = None;
        slot.valid_since = None;
        *self.rx_mut(i) = None;
        if i == self.live {
            self.mark_invalid(now);
        }
        let slot = &mut self.slots[i];
        if slot.source.is_none() {
            slot.state = ConnState::Idle;
            return;
        }
        match reason {
            Some(e) => self.fail_slot(i, e, now),
            None => {
                // Silent link: keep the last error (if any) and retry.
                let slot = &mut self.slots[i];
                slot.attempts = slot.attempts.saturating_add(1);
                let delay = backoff(slot.attempts);
                slot.next_at = Some(now + delay);
                slot.retry_at_ms = Some(epoch_ms() + delay.as_millis() as u64);
                slot.state = ConnState::Reconnecting;
            }
        }
        self.dirty = true;
    }

    /// Picks the slot that feeds readings: the main source unless fallback is on and the main
    /// source has been stale for `after_secs` while the backup is delivering. Switching back
    /// waits until the main source has delivered for [`FALLBACK_RECOVER`].
    fn evaluate_live(&mut self, now: Instant) {
        let fb = &self.config.source.fallback;
        if !fb.enabled || self.slots[BACKUP].kind.is_none() {
            if self.live != PRIMARY {
                self.switch_live(PRIMARY, now);
            }
            return;
        }
        let after = Duration::from_secs(fb.after_secs as u64);
        let stale = self.stale_timeout();
        let p = &self.slots[PRIMARY];
        let primary_stale = p.kind.is_none()
            || p.last_valid
                .or(p.started)
                .is_none_or(|t| now.saturating_duration_since(t) > after);
        let primary_recovered = p.fresh(now, stale)
            && p.valid_since
                .is_some_and(|t| now.saturating_duration_since(t) >= FALLBACK_RECOVER);
        let backup_fresh = self.slots[BACKUP].fresh(now, stale);
        let want = if self.live == PRIMARY {
            if primary_stale && backup_fresh {
                BACKUP
            } else {
                PRIMARY
            }
        } else if primary_recovered {
            PRIMARY
        } else {
            BACKUP
        };
        if want != self.live {
            self.switch_live(want, now);
        }
    }

    fn switch_live(&mut self, to: usize, now: Instant) {
        if to == self.live {
            return;
        }
        tracing::info!(
            from = ?self.slots[self.live].kind.map(SourceKind::tag),
            to = ?self.slots[to].kind.map(SourceKind::tag),
            "switching live source"
        );
        self.live = to;
        self.processor.on_reconnect();
        self.beat.reset();
        self.recent.clear();
        self.last_valid_at = self.slots[to].last_valid;
        if !self.data_valid(now) {
            self.mark_invalid(now);
        }
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

    // ---------- sensor data ----------

    async fn on_sensor_event(&mut self, i: usize, ev: Option<SensorEvent>) {
        let now = Instant::now();
        match ev {
            Some(SensorEvent::Sample(s)) => self.on_sample(i, s, now),
            Some(SensorEvent::Unreadable(detail)) => {
                tracing::debug!(slot = i, detail = %detail, "unreadable packet");
                self.slots[i].last_packet = Some(now);
                if i == self.live {
                    self.snap.packets += 1;
                    self.dirty = true;
                }
            }
            Some(SensorEvent::Battery(b)) => {
                self.slots[i].battery = Some(b);
                self.dirty = true;
            }
            Some(SensorEvent::Disconnected(reason)) => {
                tracing::info!(slot = i, reason = ?reason, "source link lost");
                self.on_link_lost(i, reason, now);
            }
            None => {
                let reason = match self.slots[i].kind {
                    Some(SourceKind::Ble | SourceKind::Demo) => KtError::Ble("connection closed".into()),
                    _ => KtError::Service("The source", "connection closed".into()),
                };
                self.on_link_lost(i, Some(reason), now)
            }
        }
    }

    fn on_sample(&mut self, i: usize, s: HrSample, now: Instant) {
        let slot = &mut self.slots[i];
        slot.last_packet = Some(now);
        slot.last_packet_ms = Some(epoch_ms());
        slot.packets = slot.packets.saturating_add(1);
        if !s.rr_ms.is_empty() {
            slot.rr_packets = slot.rr_packets.saturating_add(1);
        }
        if let Some(b) = s.battery {
            slot.battery = Some(b);
        }
        if i != self.live {
            // Standby source: only track whether it is delivering.
            let ok = (MIN_VALID_BPM..=MAX_VALID_BPM).contains(&s.bpm) && s.has_contact();
            if ok {
                slot.mark_valid(now);
                slot.valid_since.get_or_insert(now);
                slot.state = ConnState::Connected;
            } else {
                slot.valid_since = None;
                slot.state = ConnState::NoData;
            }
            self.dirty = true;
            self.evaluate_live(now);
            if i != self.live {
                return;
            }
        }
        self.on_measurement(i, s, now);
    }

    // ---------- saved history ----------

    fn record(&mut self, ops: Vec<RecordOp>) {
        if let Some(tx) = &self.history_tx {
            for op in ops {
                let _ = tx.send(WriterMsg::Op(op));
            }
        }
    }

    fn record_sample(&mut self, i: usize, t_ms: u64, bpm: f64, raw: u16, rr: &[f64]) {
        if self.history_tx.is_none() || !self.config.history.enabled {
            return;
        }
        let slot = &self.slots[i];
        let source = slot.kind.unwrap_or(self.config.source.active);
        let device = slot
            .device
            .as_ref()
            .map(|d| d.name.clone())
            .or_else(|| slot.source.as_ref().map(|s| s.label()))
            .filter(|l| !l.trim().is_empty());
        let zones = self.config.zones.clone();
        let ops = self.recorder.sample(t_ms, bpm, raw, rr, |start_ms| SessionMeta {
            v: 1,
            id: start_ms.to_string(),
            start_ms,
            source,
            device,
            zones,
        });
        self.record(ops);
    }

    fn on_measurement(&mut self, i: usize, s: HrSample, now: Instant) {
        self.snap.packets += 1;
        self.snap.last_packet_ms = Some(epoch_ms());
        self.snap.contact = s.sensor_contact();
        if s.energy_kj.is_some() {
            self.snap.energy_kj = s.energy_kj;
        }
        let outcome = self.processor.push_bpm(s.bpm, s.has_contact(), now);
        if s.has_contact() {
            for &rr in &s.rr_ms {
                self.processor.push_rr(rr, now);
                self.beat.push_rr(Duration::from_secs_f64(rr / 1000.0), now);
            }
        }
        if let SampleOutcome::Accepted { raw, smoothed } = outcome {
            self.last_valid_at = Some(now);
            let slot = &mut self.slots[i];
            slot.mark_valid(now);
            slot.valid_since.get_or_insert(now);
            if slot.session.is_some() {
                slot.state = ConnState::Connected;
            }
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
            let rr: &[f64] = if s.has_contact() { &s.rr_ms } else { &[] };
            self.record_sample(i, point.t_ms, smoothed, raw, rr);
        }
        self.snap.rejected_spikes = self.processor.rejected_spikes();
        self.snap.rejected_out_of_range = self.processor.rejected_out_of_range();
        self.dirty = true;

        // Hot path: send changed values right away.
        self.emit_osc(now, false, false);
        let us = s.received.elapsed().as_micros() as u64;
        let l = &mut self.snap.latency;
        l.last_us = us;
        l.max_us = l.max_us.max(us);
        l.samples += 1;
        self.latency_sum_us += us;
        l.avg_us = self.latency_sum_us / l.samples;
        if self.config.debug.log_latency {
            tracing::debug!(
                latency_us = us,
                bpm = s.bpm,
                source = s.source.tag(),
                "source->osc latency"
            );
        }
        self.evaluate_live(now);
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
                alert: self.alert_code(),
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
            battery: self.slots[self.live].battery,
            beat_toggle: self.beat_toggle,
            beat_pulse: self.beat_pulse,
            session_min: stats.min,
            session_avg: stats.avg,
            session_max: stats.max,
            avg_10s,
            alert: self.alert_code(),
        }
    }

    /// The `Alert` avatar parameter: 0 none, 1 high, 2 low (test alerts included, so the
    /// Test alert button can be used to check an avatar's alert animation).
    fn alert_code(&self) -> u8 {
        match self.snap.alert.as_ref().map(|a| a.kind) {
            None => 0,
            Some(AlertKind::High) => 1,
            Some(AlertKind::Low) => 2,
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
                self.socket_v6 = nonblocking_udp((Ipv6Addr::UNSPECIFIED, 0).into());
            }
            self.socket_v6.as_ref()
        };
        let Some(sock) = sock else {
            return Err("no UDP socket".into());
        };
        sock.send_to(&o.bytes, o.dest).map(|_| ()).map_err(|e| {
            tracing::debug!(error = %e, dest = %o.dest, "OSC send failed");
            KtError::OscSocket(e.to_string()).user().message
        })
    }

    fn chatbox(&mut self, now: Instant) {
        // While a real alert is showing (and alerts go to the chatbox), its text replaces the
        // normal template so the regular message does not overwrite the warning.
        let alert_template = self
            .alerts
            .active()
            .filter(|_| self.config.alerts.chatbox)
            .map(|k| self.config.alerts.text(k).to_string());
        if !self.config.osc.chatbox.enabled && alert_template.is_none() {
            return;
        }
        let metrics = self.metrics(now);
        if !metrics.connected {
            return;
        }
        let template = alert_template.unwrap_or_else(|| self.config.osc.chatbox.template.clone());
        let text = chatbox::render(&template, &metrics, self.chatbox_phase);
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

    // ---------- alerts ----------

    fn check_alerts(&mut self, now: Instant) {
        if self.test_alert_until.is_some_and(|t| now >= t) {
            self.test_alert_until = None;
            if self.snap.alert.as_ref().is_some_and(|a| a.test) {
                self.snap.alert = None;
                self.dirty = true;
                self.clear_alert_chatbox(now);
            }
        }
        // The test sweep's made-up values never raise alerts.
        let bpm = self
            .snap
            .bpm
            .filter(|_| self.sweep.is_none() && self.data_valid(now));
        match self.alerts.update(&self.config.alerts, bpm, now) {
            Some(AlertAction::Fire {
                kind,
                bpm,
                threshold,
                repeat,
            }) => {
                tracing::info!(kind = ?kind, bpm, threshold, repeat, "heart rate alert");
                let since_ms = match &self.snap.alert {
                    Some(a) if repeat && !a.test => a.since_ms,
                    _ => epoch_ms(),
                };
                self.test_alert_until = None;
                self.snap.alert = Some(AlertStatus {
                    kind,
                    bpm,
                    threshold,
                    since_ms,
                    test: false,
                });
                self.dirty = true;
                self.fire_alert(kind, bpm, threshold, repeat, false, now);
            }
            Some(AlertAction::Clear { kind }) => {
                tracing::info!(kind = ?kind, "heart rate alert cleared");
                if self.snap.alert.as_ref().is_some_and(|a| !a.test) {
                    self.snap.alert = None;
                    self.dirty = true;
                }
                self.clear_alert_chatbox(now);
            }
            None => {
                if let (Some(a), Some(b)) = (self.snap.alert.as_mut(), bpm) {
                    if !a.test && a.bpm != b {
                        a.bpm = b;
                        self.dirty = true;
                    }
                }
            }
        }
    }

    fn test_alert(&mut self, now: Instant) {
        if self.alerts.active().is_some() {
            return; // a real alert is showing; do not cover it
        }
        let threshold = self.config.alerts.high_bpm;
        let bpm = threshold.saturating_add(12);
        self.snap.alert = Some(AlertStatus {
            kind: AlertKind::High,
            bpm,
            threshold,
            since_ms: epoch_ms(),
            test: true,
        });
        self.dirty = true;
        self.test_alert_until = Some(now + Duration::from_secs(8));
        self.fire_alert(AlertKind::High, bpm, threshold, false, true, now);
    }

    /// Tells the UI (notification, sound) and, if chosen, VRChat's chatbox.
    fn fire_alert(
        &mut self,
        kind: AlertKind,
        bpm: u16,
        threshold: u16,
        repeat: bool,
        test: bool,
        now: Instant,
    ) {
        if !test {
            let ops = self.recorder.alert(epoch_ms(), kind, bpm, threshold);
            self.record(ops);
        }
        let _ = self.ev_tx.send(EngineEvent::Alert {
            alert_kind: kind,
            bpm,
            threshold,
            repeat,
            test,
        });
        if !self.config.alerts.chatbox {
            return;
        }
        let mut metrics = self.metrics(now);
        metrics.hr = bpm;
        let text = chatbox::render(self.config.alerts.text(kind), &metrics, self.chatbox_phase);
        // The first alert plays VRChat's chatbox sound so it gets noticed; reminders stay quiet.
        self.send_chatbox_now(&text, repeat, test, now);
    }

    /// Clears the alert from the chatbox when the normal chatbox output is off (when it is on,
    /// the next regular message replaces it).
    fn clear_alert_chatbox(&mut self, now: Instant) {
        if self.config.alerts.chatbox && !self.config.osc.chatbox.enabled {
            self.send_chatbox_now("", true, false, now);
        } else {
            for s in &mut self.senders {
                s.reset_chatbox();
            }
        }
    }

    /// Sends chatbox text to every chatbox target right away, ignoring the send interval.
    fn send_chatbox_now(&mut self, text: &str, silent: bool, test: bool, now: Instant) {
        let mut out = Vec::new();
        for (i, s) in self.senders.iter_mut().enumerate() {
            if s.target.enabled && s.target.chatbox {
                s.reset_chatbox();
                if let Some(o) = s.chatbox(text, silent, Duration::ZERO, now, test) {
                    out.push((i, o));
                }
            }
        }
        if !out.is_empty() {
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
            let recovered = matches!(self.snap.adapter, AdapterState::Off | AdapterState::Missing);
            self.snap.adapter = s;
            self.dirty = true;
            // Clear a stale radio error once the radio is back; reconnect promptly.
            if s == AdapterState::Ready {
                for slot in &mut self.slots {
                    if !matches!(slot.kind, Some(SourceKind::Ble | SourceKind::Demo)) {
                        continue;
                    }
                    if matches!(
                        slot.error.as_ref().map(|e| e.code.as_str()),
                        Some("radio_off" | "adapter_missing")
                    ) {
                        slot.error = None;
                    }
                    if recovered
                        && slot.source.is_some()
                        && slot.session.is_none()
                        && !slot.inflight
                        && !slot.halted
                    {
                        slot.next_at = Some(Instant::now());
                    }
                }
            }
        }
    }

    // ---------- output ----------

    /// Copies slot state into the snapshot: top-level connection fields describe the live slot.
    fn sync_snapshot(&mut self) {
        let now = Instant::now();
        let live = &self.slots[self.live];
        let active = self.config.source.active;
        let mut capabilities = live
            .source
            .as_ref()
            .map(|s| s.capabilities())
            .unwrap_or_else(|| active.capabilities());
        if live.lacks_rr() {
            capabilities.rr = false;
        }
        let target = (self.running && matches!(active, SourceKind::Ble | SourceKind::Demo))
            .then(|| self.config.device.last_device.clone())
            .flatten();
        let sources: Vec<SlotStatus> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, sl)| {
                Some(SlotStatus {
                    role: if i == PRIMARY {
                        SlotRole::Primary
                    } else {
                        SlotRole::Backup
                    },
                    kind: sl.kind?,
                    label: sl
                        .device
                        .as_ref()
                        .map(|d| d.name.clone())
                        .or_else(|| sl.source.as_ref().map(|s| s.label())),
                    state: sl.state,
                    error: sl.error.clone(),
                    last_packet_ms: sl.last_packet_ms,
                    live: i == self.live,
                })
            })
            .collect();
        let live_kind = if self.snap_valid(now) { live.kind } else { None };
        let fallback_active = self.live == BACKUP;
        let error = live.error.clone().or_else(|| self.engine_error.clone());
        let (state, attempts, retry_at_ms, device, battery) = (
            live.state,
            live.attempts,
            live.retry_at_ms,
            live.device.clone(),
            live.battery,
        );
        let snap = &mut self.snap;
        let mut changed = false;
        macro_rules! set {
            ($field:ident, $v:expr) => {{
                let v = $v;
                if snap.$field != v {
                    snap.$field = v;
                    changed = true;
                }
            }};
        }
        set!(state, state);
        set!(attempt, attempts);
        set!(retry_at_ms, retry_at_ms);
        set!(device, device);
        set!(error, error);
        set!(battery, battery);
        set!(source, active);
        set!(live_source, live_kind);
        set!(fallback_active, fallback_active);
        set!(capabilities, capabilities);
        set!(target, target);
        set!(sources, sources);
        if changed {
            self.dirty = true;
        }
    }

    fn snap_valid(&self, now: Instant) -> bool {
        self.data_valid(now) || self.sweep.is_some()
    }

    fn publish(&mut self) {
        self.sync_snapshot();
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
        let ops = self.recorder.end();
        self.record(ops);
        if let Some(tx) = &self.history_tx {
            let (done_tx, done_rx) = oneshot::channel();
            if tx.send(WriterMsg::Sync(done_tx)).is_ok() {
                let _ = tokio::time::timeout(Duration::from_secs(1), done_rx).await;
            }
        }
        self.running = false;
        self.stop_slot(PRIMARY).await;
        self.stop_slot(BACKUP).await;
        self.mark_invalid(Instant::now());
        let _ = self.backend.stop_scan().await;
        self.oscq = None;
    }
}

fn nonblocking_udp(addr: SocketAddr) -> Option<UdpSocket> {
    let s = UdpSocket::bind(addr).ok()?;
    s.set_nonblocking(true).ok()?;
    Some(s)
}

/// Test sweep: 60 → 180 → 60 bpm over [`SWEEP_LEN`].
/// Stale timeout learned from recent gaps between readings: `base` for fast senders, else 1.5x
/// the longest recent gap, capped at [`MAX_LEARNED_GAP`]. Needs 3 gaps before it adapts.
pub fn learned_stale(gaps: &VecDeque<Duration>, base: Duration) -> Duration {
    if gaps.len() < 3 {
        return base;
    }
    let longest = gaps.iter().copied().max().unwrap_or_default();
    base.max(longest.mul_f64(1.5)).min(MAX_LEARNED_GAP)
}

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
    fn learned_stale_adapts_to_slow_senders() {
        let base = Duration::from_secs(5);
        let gaps = |v: &[u64]| v.iter().map(|&s| Duration::from_secs(s)).collect::<VecDeque<_>>();
        // Too few samples, or a fast strap: the configured timeout.
        assert_eq!(learned_stale(&gaps(&[20, 20]), base), base);
        assert_eq!(learned_stale(&gaps(&[1, 1, 1, 1]), base), base);
        // A phone app relaying a watch every 5-15 s.
        assert_eq!(
            learned_stale(&gaps(&[5, 15, 8]), base),
            Duration::from_millis(22_500)
        );
        // Never more than a minute.
        assert_eq!(learned_stale(&gaps(&[50, 55, 58]), base), Duration::from_secs(60));
    }

    #[test]
    fn sweep_shape() {
        assert_eq!(sweep_bpm(Duration::ZERO), 60.0);
        assert_eq!(sweep_bpm(SWEEP_LEN / 2), 180.0);
        assert_eq!(sweep_bpm(SWEEP_LEN), 60.0);
    }
}
