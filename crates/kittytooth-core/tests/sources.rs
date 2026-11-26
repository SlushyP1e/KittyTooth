//! Heart rate sources end to end: a mock WebSocket server stands in for Pulsoid and HypeRate
//! (data, reconnect, auth failure, stale detection), a UDP sender feeds the OSC input source,
//! and two mock sources exercise fallback switching. Readings are checked on the OSC output.
#![allow(clippy::result_large_err)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use kittytooth_core::config::{Config, OscTarget, Preset, RememberedDevice};
use kittytooth_core::engine::{self, ConnState, EngineHandle, EngineOptions, Snapshot};
use kittytooth_core::sensor::mock::{MockBackend, MockDevice, MockStep};
use kittytooth_core::source::{
    DefaultFactory, Endpoints, MemorySecrets, SecretKind, SecretStore, SourceKind,
};
use rosc::{OscPacket, OscType};
use tokio::net::{TcpListener, UdpSocket};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::{http, Message};

const KT: &str = "/avatar/parameters/KittyTooth/";

// ---------- OSC output listener ----------

type Msgs = Arc<Mutex<Vec<(String, OscType)>>>;

struct Listener {
    port: u16,
    msgs: Msgs,
    _task: tokio::task::JoinHandle<()>,
}

fn flatten(p: OscPacket, out: &mut Vec<rosc::OscMessage>) {
    match p {
        OscPacket::Message(m) => out.push(m),
        OscPacket::Bundle(b) => b.content.into_iter().for_each(|c| flatten(c, out)),
    }
}

impl Listener {
    async fn new() -> Self {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = sock.local_addr().unwrap().port();
        let msgs: Msgs = Arc::default();
        let m2 = msgs.clone();
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                let Ok((n, _)) = sock.recv_from(&mut buf).await else {
                    continue;
                };
                let (_, pkt) = rosc::decoder::decode_udp(&buf[..n]).expect("valid OSC");
                let mut v = Vec::new();
                flatten(pkt, &mut v);
                let mut g = m2.lock().unwrap();
                for m in v {
                    if let Some(a) = m.args.into_iter().next() {
                        g.push((m.addr, a));
                    }
                }
            }
        });
        Self {
            port,
            msgs,
            _task: task,
        }
    }

    fn last(&self, addr: &str) -> Option<OscType> {
        self.msgs
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(a, _)| a == addr)
            .map(|(_, v)| v.clone())
    }

    fn hr_values(&self) -> Vec<i32> {
        let hr = format!("{KT}HR");
        self.msgs
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(a, v)| match v {
                OscType::Int(i) if *a == hr => Some(*i),
                _ => None,
            })
            .collect()
    }
}

async fn until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    f()
}

async fn until_snap(h: &EngineHandle, timeout: Duration, f: impl Fn(&Snapshot) -> bool) -> bool {
    until(timeout, || f(&h.snapshot())).await
}

// ---------- mock WebSocket server ----------
// (tungstenite fixes the handshake callback signature, including its large error type.)

#[derive(Clone)]
enum Step {
    Send(String),
    Wait(u64),
    /// Read one client frame and record it.
    Expect,
    Close,
    /// Keep the connection open without sending anything.
    Silent,
}

enum Reply {
    Reject(u16, String),
    Script(Vec<Step>),
}

type Behavior = Arc<dyn Fn(usize, &str) -> Reply + Send + Sync>;

struct WsServer {
    port: u16,
    conns: Arc<AtomicUsize>,
    uris: Arc<Mutex<Vec<String>>>,
    frames: Arc<Mutex<Vec<String>>>,
}

impl WsServer {
    async fn start(behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let conns = Arc::new(AtomicUsize::new(0));
        let uris: Arc<Mutex<Vec<String>>> = Arc::default();
        let frames: Arc<Mutex<Vec<String>>> = Arc::default();
        let (c2, u2, f2) = (conns.clone(), uris.clone(), frames.clone());
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    continue;
                };
                let n = c2.fetch_add(1, Ordering::SeqCst);
                let (behavior, uris, frames) = (behavior.clone(), u2.clone(), f2.clone());
                tokio::spawn(async move {
                    let mut script = None;
                    let cb = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
                        let uri = req.uri().to_string();
                        uris.lock().unwrap().push(uri.clone());
                        match behavior(n, &uri) {
                            Reply::Reject(status, body) => {
                                Err(http::Response::builder().status(status).body(Some(body)).unwrap())
                            }
                            Reply::Script(s) => {
                                script = Some(s);
                                Ok(resp)
                            }
                        }
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(tcp, cb).await else {
                        return;
                    };
                    for step in script.unwrap_or_default() {
                        match step {
                            Step::Send(t) => {
                                if ws.send(Message::text(t)).await.is_err() {
                                    return;
                                }
                            }
                            Step::Wait(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                            Step::Expect => {
                                if let Some(Ok(Message::Text(t))) = ws.next().await {
                                    frames.lock().unwrap().push(t.to_string());
                                }
                            }
                            Step::Close => {
                                let _ = ws.close(None).await;
                                return;
                            }
                            Step::Silent => {
                                // Drain whatever the client sends until it goes away.
                                while let Some(Ok(m)) = ws.next().await {
                                    if let Message::Text(t) = m {
                                        frames.lock().unwrap().push(t.to_string());
                                    }
                                }
                                return;
                            }
                        }
                    }
                    let _ = ws.close(None).await;
                });
            }
        });
        Self {
            port,
            conns,
            uris,
            frames,
        }
    }

    fn connections(&self) -> usize {
        self.conns.load(Ordering::SeqCst)
    }
}

fn pulsoid_hr(bpm: u16) -> Step {
    Step::Send(format!(
        r#"{{"type":"heart_rate","timestamp":"2026-10-17T12:00:00.123Z","heartRate":{{"bpm":{bpm}}}}}"#
    ))
}

fn hyperate_hr(id: &str, bpm: u16) -> Step {
    Step::Send(format!(
        r#"{{"topic":"hr:{id}","event":"hr_update","payload":{{"hr":{bpm}}},"ref":null}}"#
    ))
}

fn stream(mut head: Vec<Step>, msg: impl Fn() -> Step, n: usize, gap_ms: u64) -> Vec<Step> {
    for _ in 0..n {
        head.push(msg());
        head.push(Step::Wait(gap_ms));
    }
    head
}

// ---------- engine setup ----------

fn target(port: u16) -> OscTarget {
    OscTarget {
        id: "t".into(),
        name: "t".into(),
        host: "127.0.0.1".into(),
        port,
        enabled: true,
        preset: Preset::KittyTooth,
        use_oscquery: false,
        chatbox: false,
        ..OscTarget::default()
    }
}

fn config(kind: SourceKind, out_port: u16) -> Config {
    let mut c = Config::default();
    c.osc.targets = vec![target(out_port)];
    c.osc.oscquery = false;
    c.device.demo_device = false;
    c.device.auto_connect = true;
    c.device.stale_timeout_secs = 2;
    c.source.active = kind;
    c
}

fn opts() -> EngineOptions {
    EngineOptions {
        mdns: false,
        network_link_floor: Duration::ZERO,
    }
}

fn net_engine(cfg: Config, ws_port: u16, secrets: Arc<MemorySecrets>) -> EngineHandle {
    let backend = Arc::new(MockBackend::new(vec![]));
    let factory = DefaultFactory {
        backend,
        secrets,
        endpoints: Endpoints {
            pulsoid_ws: format!("ws://127.0.0.1:{ws_port}/api/v2/data/real_time"),
            hyperate_ws: format!("ws://127.0.0.1:{ws_port}/ws"),
        },
        ant: kittytooth_core::source::ant::default_opener(),
    };
    engine::start_with_factory(cfg, None, Arc::new(factory), opts())
}

fn secrets_with(kind: SecretKind, value: &str) -> Arc<MemorySecrets> {
    let s = Arc::new(MemorySecrets::default());
    s.set(kind, value).unwrap();
    s
}

// ---------- Pulsoid ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pulsoid_streams_heart_rate_and_battery() {
    let l = Listener::new().await;
    let srv = WsServer::start(Arc::new(|_, _| {
        Reply::Script(stream(
            vec![Step::Send(r#"{"type":"battery_level","timestamp":"2026-10-17T12:00:00Z","batteryLevel":{"percent":67}}"#.into())],
            || pulsoid_hr(85),
            200,
            100,
        ))
    }))
    .await;
    let h = net_engine(
        config(SourceKind::Pulsoid, l.port),
        srv.port,
        secrets_with(SecretKind::PulsoidToken, "tok-1234"),
    );
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(85)))
        .await
    );
    assert_eq!(l.last(&format!("{KT}Connected")), Some(OscType::Bool(true)));
    assert!(
        until_snap(&h, Duration::from_secs(1), |s| s.live_source.is_some()
            && s.battery.is_some())
        .await
    );
    let s = h.snapshot();
    assert_eq!(s.state, ConnState::Connected);
    assert_eq!(s.live_source, Some(SourceKind::Pulsoid));
    assert_eq!(s.battery, Some(67));
    assert!(s.capabilities.battery && !s.capabilities.rr && !s.capabilities.contact);
    assert_eq!(s.hrv_ms, None, "no RR from Pulsoid, so no HRV");
    let uri = srv.uris.lock().unwrap()[0].clone();
    assert_eq!(
        uri,
        "/api/v2/data/real_time?access_token=tok-1234&types=heart_rate,battery_level"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pulsoid_token_without_battery_scope_falls_back_to_heart_rate_only() {
    let l = Listener::new().await;
    let srv = WsServer::start(Arc::new(|_, uri: &str| {
        if uri.contains("battery_level") {
            Reply::Reject(
                400,
                r#"{"error_code":6003,"error_message":"error_invalid_scope"}"#.into(),
            )
        } else {
            Reply::Script(stream(vec![], || pulsoid_hr(77), 100, 100))
        }
    }))
    .await;
    let h = net_engine(
        config(SourceKind::Pulsoid, l.port),
        srv.port,
        secrets_with(SecretKind::PulsoidToken, "tok"),
    );
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(77)))
        .await
    );
    assert!(srv
        .uris
        .lock()
        .unwrap()
        .iter()
        .any(|u| u.ends_with("types=heart_rate")));
    assert!(h.snapshot().error.is_none());
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pulsoid_bad_token_stops_retrying_with_clear_error() {
    let l = Listener::new().await;
    let srv = WsServer::start(Arc::new(|_, _| {
        Reply::Reject(
            401,
            r#"{"error_code":"7005","error_message":"token_not_found"}"#.into(),
        )
    }))
    .await;
    let h = net_engine(
        config(SourceKind::Pulsoid, l.port),
        srv.port,
        secrets_with(SecretKind::PulsoidToken, "revoked-token"),
    );
    assert!(until_snap(&h, Duration::from_secs(5), |s| s.error.is_some()).await);
    let s = h.snapshot();
    let e = s.error.unwrap();
    assert_eq!(e.code, "bad_token");
    assert_eq!(e.title, "Pulsoid token rejected");
    assert!(
        !format!("{e:?}").contains("revoked-token"),
        "never echo the token"
    );
    assert_eq!(s.state, ConnState::Idle);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(srv.connections(), 1, "a rejected token isn't retried");
    // A new token (RestartSources) tries again.
    h.restart_sources().await.unwrap();
    assert!(until(Duration::from_secs(3), || srv.connections() == 2).await);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pulsoid_reconnects_with_backoff_after_close() {
    let l = Listener::new().await;
    let srv = WsServer::start(Arc::new(|n, _| {
        let bpm = if n == 0 { 70 } else { 90 };
        let mut s = stream(vec![], move || pulsoid_hr(bpm), 5, 100);
        if n == 0 {
            s.push(Step::Close);
        } else {
            s = stream(s, move || pulsoid_hr(bpm), 100, 100);
        }
        Reply::Script(s)
    }))
    .await;
    let h = net_engine(
        config(SourceKind::Pulsoid, l.port),
        srv.port,
        secrets_with(SecretKind::PulsoidToken, "tok"),
    );
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(70)))
        .await
    );
    assert!(until_snap(&h, Duration::from_secs(3), |s| s.state == ConnState::Reconnecting).await);
    assert_eq!(h.snapshot().error.map(|e| e.code), Some("service_error".into()));
    // First retry after 1 s.
    assert!(
        until(Duration::from_secs(4), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(90)))
        .await
    );
    assert_eq!(srv.connections(), 2);
    assert!(
        until_snap(&h, Duration::from_secs(2), |s| s.state == ConnState::Connected
            && s.error.is_none())
        .await
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pulsoid_silence_marks_data_stale_and_reconnects() {
    let l = Listener::new().await;
    let srv = WsServer::start(Arc::new(|_, _| {
        let mut s = stream(vec![], || pulsoid_hr(80), 5, 100);
        s.push(Step::Silent);
        Reply::Script(s)
    }))
    .await;
    let h = net_engine(
        config(SourceKind::Pulsoid, l.port),
        srv.port,
        secrets_with(SecretKind::PulsoidToken, "tok"),
    );
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(80)))
        .await
    );
    let t0 = Instant::now();
    // Stale timeout is 2 s: Connected=false goes out, then the link is torn down and retried.
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}Connected"))
            == Some(OscType::Bool(false)))
        .await
    );
    assert!(t0.elapsed() >= Duration::from_millis(1500), "{:?}", t0.elapsed());
    assert!(until(Duration::from_secs(6), || srv.connections() >= 2).await);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pulsoid_offline_is_reported_and_retried() {
    let l = Listener::new().await;
    // Nothing listens on this port.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let h = net_engine(
        config(SourceKind::Pulsoid, l.port),
        port,
        secrets_with(SecretKind::PulsoidToken, "tok"),
    );
    assert!(until_snap(&h, Duration::from_secs(5), |s| s.error.is_some()).await);
    let s = h.snapshot();
    assert_eq!(s.error.unwrap().code, "offline");
    assert_eq!(s.state, ConnState::Reconnecting);
    assert!(s.retry_at_ms.is_some());
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pulsoid_missing_token_explains_what_to_do() {
    let l = Listener::new().await;
    let h = net_engine(
        config(SourceKind::Pulsoid, l.port),
        1,
        Arc::new(MemorySecrets::default()),
    );
    assert!(until_snap(&h, Duration::from_secs(2), |s| s.error.is_some()).await);
    let e = h.snapshot().error.unwrap();
    assert_eq!(e.code, "missing_credential");
    assert!(e.fix.contains("Get token"));
    h.shutdown().await;
}

// ---------- HypeRate ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hyperate_joins_channel_and_streams() {
    let l = Listener::new().await;
    let srv = WsServer::start(Arc::new(|_, _| {
        let mut s = vec![
            Step::Expect,
            Step::Send(r#"{"topic":"hr:abc123","event":"phx_reply","payload":{"status":"ok","response":{}},"ref":"1"}"#.into()),
        ];
        s = stream(s, || hyperate_hr("abc123", 75), 100, 100);
        Reply::Script(s)
    }))
    .await;
    let mut cfg = config(SourceKind::Hyperate, l.port);
    cfg.source.hyperate.device_id = "abc123".into();
    let h = net_engine(cfg, srv.port, secrets_with(SecretKind::HyperateKey, "dev-key"));
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(75)))
        .await
    );
    assert_eq!(srv.uris.lock().unwrap()[0], "/ws/abc123?token=dev-key");
    let join: serde_json::Value = serde_json::from_str(&srv.frames.lock().unwrap()[0]).unwrap();
    assert_eq!(join["topic"], "hr:abc123");
    assert_eq!(join["event"], "phx_join");
    assert_eq!(join["ref"], "1");
    assert!(until_snap(&h, Duration::from_secs(1), |s| s.live_source.is_some()).await);
    let s = h.snapshot();
    assert_eq!(s.live_source, Some(SourceKind::Hyperate));
    assert_eq!(s.device.unwrap().name, "HypeRate abc123");
    assert_eq!(s.battery, None);
    assert!(!s.capabilities.battery && !s.capabilities.rr);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hyperate_rejected_key_and_unknown_device() {
    let l = Listener::new().await;
    let srv = WsServer::start(Arc::new(|_, uri: &str| {
        if uri.contains("token=bad") {
            Reply::Reject(403, String::new())
        } else {
            Reply::Script(vec![
                Step::Expect,
                Step::Send(
                    r#"{"topic":"hr:nope99","event":"phx_reply","payload":{"status":"error","response":{"reason":"unmatched topic"}},"ref":"1"}"#
                        .into(),
                ),
                Step::Silent,
            ])
        }
    }))
    .await;
    let mut cfg = config(SourceKind::Hyperate, l.port);
    cfg.source.hyperate.device_id = "nope99".into();
    let h = net_engine(
        cfg.clone(),
        srv.port,
        secrets_with(SecretKind::HyperateKey, "bad"),
    );
    assert!(until_snap(&h, Duration::from_secs(5), |s| s.error.is_some()).await);
    assert_eq!(h.snapshot().error.unwrap().code, "bad_api_key");
    assert_eq!(h.snapshot().state, ConnState::Idle);
    h.shutdown().await;

    let h = net_engine(cfg, srv.port, secrets_with(SecretKind::HyperateKey, "good"));
    assert!(until_snap(&h, Duration::from_secs(5), |s| s.error.is_some()).await);
    let s = h.snapshot();
    assert_eq!(s.error.unwrap().code, "invalid_device_id");
    assert_eq!(s.state, ConnState::Idle);
    let before = srv.connections();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(srv.connections(), before, "an unknown device ID isn't retried");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hyperate_reconnects_and_detects_stale_data() {
    let l = Listener::new().await;
    let srv = WsServer::start(Arc::new(|n, _| {
        let mut s = stream(vec![Step::Expect], || hyperate_hr("abc123", 66), 5, 100);
        s.push(if n == 0 { Step::Close } else { Step::Silent });
        Reply::Script(s)
    }))
    .await;
    let mut cfg = config(SourceKind::Hyperate, l.port);
    cfg.source.hyperate.device_id = "abc123".into();
    let h = net_engine(cfg, srv.port, secrets_with(SecretKind::HyperateKey, "k"));
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(66)))
        .await
    );
    assert!(
        until(Duration::from_secs(5), || srv.connections() >= 2).await,
        "reconnects after close"
    );
    // Second connection goes silent: data stale -> Connected=false.
    assert!(until_snap(&h, Duration::from_secs(4), |s| s.state == ConnState::Connected).await);
    assert!(
        until(Duration::from_secs(6), || l.last(&format!("{KT}Connected"))
            == Some(OscType::Bool(false)))
        .await
    );
    h.shutdown().await;
}

// ---------- OSC input ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn osc_input_relays_heart_rate() {
    let l = Listener::new().await;
    let free = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let in_port = free.local_addr().unwrap().port();
    drop(free);
    let mut cfg = config(SourceKind::Osc, l.port);
    cfg.source.osc_input.port = in_port;
    cfg.source.osc_input.address = "HR".into(); // bare name -> /avatar/parameters/HR
    let h = engine::start(cfg, None, Arc::new(MockBackend::new(vec![])), opts());
    assert!(until_snap(&h, Duration::from_secs(3), |s| s.state == ConnState::NoData).await);
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let to = SocketAddr::from(([127, 0, 0, 1], in_port));
    let send = |addr: &'static str, v: OscType| {
        rosc::encoder::encode(&OscPacket::Message(rosc::OscMessage {
            addr: addr.into(),
            args: vec![v],
        }))
        .unwrap()
    };
    for _ in 0..10 {
        sender
            .send_to(&send("/avatar/parameters/HR", OscType::Int(88)), to)
            .await
            .unwrap();
        sender
            .send_to(&send("/avatar/parameters/Other", OscType::Int(5)), to)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        until(Duration::from_secs(3), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(88)))
        .await
    );
    assert!(until_snap(&h, Duration::from_secs(1), |s| s.live_source.is_some()).await);
    let s = h.snapshot();
    assert_eq!(s.live_source, Some(SourceKind::Osc));
    assert_eq!(s.device.unwrap().name, format!("OSC input :{in_port}"));
    for _ in 0..10 {
        sender
            .send_to(&send("/avatar/parameters/HR", OscType::Float(101.4)), to)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        until(Duration::from_secs(3), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(101)))
        .await
    );
    assert!(!l.hr_values().contains(&5));
    // The listener has no link: silence makes data stale but never tears it down.
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}Connected"))
            == Some(OscType::Bool(false)))
        .await
    );
    assert_eq!(h.snapshot().state, ConnState::NoData);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn osc_input_port_in_use_is_explained() {
    let l = Listener::new().await;
    let taken = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut cfg = config(SourceKind::Osc, l.port);
    cfg.source.osc_input.port = taken.local_addr().unwrap().port();
    let h = engine::start(cfg, None, Arc::new(MockBackend::new(vec![])), opts());
    assert!(until_snap(&h, Duration::from_secs(3), |s| s.error.is_some()).await);
    assert_eq!(h.snapshot().error.unwrap().code, "port_in_use");
    h.shutdown().await;
}

// ---------- source switching and fallback ----------

fn hr_steps(bpm: u16, n: usize, gap_ms: u64) -> Vec<MockStep> {
    let mut v = Vec::new();
    for _ in 0..n {
        v.push(MockStep::hr(bpm));
        v.push(MockStep::wait_ms(gap_ms));
    }
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fallback_switches_to_backup_and_back() {
    let l = Listener::new().await;
    // Main: 70 bpm for 2 s, silent for 5 s (link stays up), then 70 bpm again.
    let mut main = hr_steps(70, 10, 200);
    main.push(MockStep::wait_ms(5000));
    main.extend(hr_steps(70, 60, 200));
    let backend = Arc::new(MockBackend::new(vec![
        MockDevice::scripted("test:1", "Main Strap", main),
        // The demo source resolves to `demo:strap`; script it as the backup.
        MockDevice::scripted("demo:strap", "Backup Strap", hr_steps(120, 100, 200)),
    ]));
    let mut cfg = config(SourceKind::Ble, l.port);
    cfg.device.stale_timeout_secs = 10; // keep the silent main link up; fallback uses after_secs
    cfg.device.last_device = Some(RememberedDevice {
        id: "test:1".into(),
        name: "Main Strap".into(),
    });
    cfg.source.fallback.enabled = true;
    cfg.source.fallback.backup = SourceKind::Demo;
    cfg.source.fallback.after_secs = 2;
    let h = engine::start(cfg, None, backend, opts());

    assert!(
        until(Duration::from_secs(3), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(70)))
        .await
    );
    assert!(until_snap(&h, Duration::from_secs(1), |s| s.sources.len() == 2).await);
    let s = h.snapshot();
    assert!(!s.fallback_active);
    assert_eq!(s.live_source, Some(SourceKind::Ble));

    // Main goes quiet at ~2 s; after 2 more seconds the backup takes over.
    assert!(
        until(Duration::from_secs(5), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(120)))
        .await
    );
    assert!(until_snap(&h, Duration::from_secs(1), |s| s.fallback_active).await);
    let s = h.snapshot();
    assert_eq!(s.live_source, Some(SourceKind::Demo));
    assert_eq!(s.device.as_ref().unwrap().name, "Backup Strap");
    assert!(s.sources.iter().any(|x| x.live && x.kind == SourceKind::Demo));

    // Main resumes at ~7 s and is trusted again after 3 s of steady data.
    assert!(
        until(Duration::from_secs(9), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(70)))
        .await
    );
    assert!(until_snap(&h, Duration::from_secs(1), |s| !s.fallback_active).await);
    let s = h.snapshot();
    assert_eq!(s.live_source, Some(SourceKind::Ble));
    // Never a gap of zeros while switching.
    assert!(!l.hr_values().contains(&0));
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selecting_a_source_switches_instantly() {
    let l = Listener::new().await;
    let backend = Arc::new(MockBackend::new(vec![
        MockDevice::scripted("test:1", "Main Strap", hr_steps(70, 200, 100)),
        MockDevice::scripted("demo:strap", "Demo", hr_steps(110, 200, 100)),
    ]));
    let mut cfg = config(SourceKind::Ble, l.port);
    cfg.device.last_device = Some(RememberedDevice {
        id: "test:1".into(),
        name: "Main Strap".into(),
    });
    let h = engine::start(cfg, None, backend, opts());
    assert!(
        until(Duration::from_secs(3), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(70)))
        .await
    );
    let t0 = Instant::now();
    h.select_source(SourceKind::Demo).await.unwrap();
    assert!(
        until(Duration::from_secs(3), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(110)))
        .await
    );
    assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
    assert!(until_snap(&h, Duration::from_secs(1), |s| s.source == SourceKind::Demo).await);
    assert_eq!(h.config().source.active, SourceKind::Demo);
    assert_eq!(
        h.config().device.last_device.unwrap().id,
        "test:1",
        "BLE device stays remembered"
    );
    h.select_source(SourceKind::Ble).await.unwrap();
    assert!(
        until(Duration::from_secs(3), || l.last(&format!("{KT}HR"))
            == Some(OscType::Int(70)))
        .await
    );
    h.shutdown().await;
}
