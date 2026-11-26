//! End-to-end: mock sensor → engine → OSC over real UDP → local listener.
//! Asserts exact addresses, OSC types and values for every preset, plus beat events,
//! disconnect behaviour, stale detection, reconnect backoff, rate limiting, chatbox,
//! OSCQuery avatar-change resend, latency and the OBS overlay.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kittytooth_core::config::{
    ChatboxConfig, Config, CustomMapping, DisconnectBehavior, Metric, OscTarget, Preset, RememberedDevice,
    ValueKind,
};
use kittytooth_core::engine::{self, ConnState, EngineHandle, EngineOptions};
use kittytooth_core::hrm::SensorContact;
use kittytooth_core::sensor::mock::{MockBackend, MockDevice, MockStep};
use rosc::{OscPacket, OscType};
use tokio::net::UdpSocket;

type Msgs = Arc<Mutex<Vec<(Instant, String, Vec<OscType>)>>>;

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
                let now = Instant::now();
                let mut g = m2.lock().unwrap();
                for m in v {
                    g.push((now, m.addr, m.args));
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
            .find(|(_, a, _)| a == addr)
            .map(|(_, _, v)| v[0].clone())
    }

    fn values(&self, addr: &str) -> Vec<OscType> {
        self.msgs
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, a, _)| a == addr)
            .map(|(_, _, v)| v[0].clone())
            .collect()
    }

    fn all(&self) -> Vec<(Instant, String, Vec<OscType>)> {
        self.msgs.lock().unwrap().clone()
    }

    fn len(&self) -> usize {
        self.msgs.lock().unwrap().len()
    }

    async fn wait_for(&self, timeout: Duration, f: impl Fn(&Listener) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if f(self) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        f(self)
    }
}

fn target(id: &str, port: u16, preset: Preset) -> OscTarget {
    OscTarget {
        id: id.into(),
        name: id.into(),
        host: "127.0.0.1".into(),
        port,
        enabled: true,
        preset,
        custom: vec![],
        use_oscquery: false,
        chatbox: true,
        range: None,
    }
}

fn base_config(targets: Vec<OscTarget>) -> Config {
    let mut c = Config::default();
    c.osc.targets = targets;
    c.osc.oscquery = false;
    c.device.demo_device = false;
    c.device.last_device = Some(RememberedDevice {
        id: "test:1".into(),
        name: "Test Strap".into(),
    });
    c.device.auto_connect = true;
    c
}

fn steady(n: usize, bpm: u16, rr: f64, gap_ms: u64) -> Vec<MockStep> {
    let mut v = vec![MockStep::Battery(80)];
    for _ in 0..n {
        v.push(MockStep::hr_rr(bpm, &[rr]));
        v.push(MockStep::wait_ms(gap_ms));
    }
    v
}

fn start(cfg: Config, steps: Vec<MockStep>) -> (EngineHandle, Arc<MockBackend>) {
    let backend = Arc::new(MockBackend::new(vec![MockDevice::scripted(
        "test:1",
        "Test Strap",
        steps,
    )]));
    let h = engine::start(
        cfg,
        None,
        backend.clone(),
        EngineOptions {
            mdns: false,
            ..Default::default()
        },
    );
    (h, backend)
}

fn f(v: f32) -> OscType {
    OscType::Float(v)
}

const KT: &str = "/avatar/parameters/KittyTooth/";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kittytooth_preset_exact_values() {
    let l = Listener::new().await;
    let (h, _) = start(
        base_config(vec![target("t", l.port, Preset::KittyTooth)]),
        steady(20, 72, 833.0, 100),
    );
    assert!(
        l.wait_for(Duration::from_secs(5), |l| l.last(&format!("{KT}HR"))
            == Some(OscType::Int(72))
            && l.last(&format!("{KT}Battery")).is_some())
            .await
    );
    let k = |n: &str| {
        l.last(&format!("{KT}{n}"))
            .unwrap_or_else(|| panic!("missing {n}"))
    };
    assert_eq!(k("HR"), OscType::Int(72));
    assert_eq!(k("Normalized"), f(((72.0f64 - 40.0) / 160.0) as f32));
    assert_eq!(k("Percent"), f((72.0f64 / 190.0) as f32));
    assert_eq!(k("Ones"), OscType::Int(2));
    assert_eq!(k("Tens"), OscType::Int(7));
    assert_eq!(k("Hundreds"), OscType::Int(0));
    assert_eq!(k("Connected"), OscType::Bool(true));
    assert_eq!(k("Zone"), OscType::Int(0));
    assert_eq!(k("Battery"), f(0.8));
    assert!(matches!(k("HRV"), OscType::Float(v) if (0.0..=1.0).contains(&v)));
    // Beats from RR intervals: Beat toggles, BeatPulse goes true then false.
    assert!(
        l.wait_for(Duration::from_secs(3), |l| {
            let b = l.values(&format!("{KT}Beat"));
            let p = l.values(&format!("{KT}BeatPulse"));
            b.contains(&OscType::Bool(true))
                && b.contains(&OscType::Bool(false))
                && p.contains(&OscType::Bool(true))
                && p.contains(&OscType::Bool(false))
        })
        .await,
        "beat toggles and pulses"
    );
    // Every address is in the KittyTooth namespace and has the documented type.
    for (_, addr, args) in l.all() {
        assert!(addr.starts_with(KT), "{addr}");
        let name = &addr[KT.len()..];
        let ok = match name {
            "HR" | "Ones" | "Tens" | "Hundreds" | "Zone" => matches!(args[0], OscType::Int(_)),
            "Normalized" | "Percent" | "HRV" | "Battery" => matches!(args[0], OscType::Float(_)),
            "Connected" | "Beat" | "BeatPulse" => matches!(args[0], OscType::Bool(_)),
            other => panic!("unexpected parameter {other}"),
        };
        assert!(ok, "{name} has wrong type {:?}", args[0]);
    }
    let s = h.snapshot();
    assert_eq!(s.state, ConnState::Connected);
    assert!(s.latency.samples > 0);
    assert!(s.latency.avg_us < 10_000, "avg latency {} us", s.latency.avg_us);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hrtovrchat_preset_exact_values() {
    let l = Listener::new().await;
    let (h, _) = start(
        base_config(vec![target("t", l.port, Preset::HrToVrchat)]),
        steady(20, 128, 469.0, 100),
    );
    let p = |n: &str| format!("/avatar/parameters/{n}");
    assert!(
        l.wait_for(Duration::from_secs(5), |l| l.last(&p("HR"))
            == Some(OscType::Int(128)))
            .await
    );
    let pct = (128.0f64 / 255.0) as f32;
    let k = |n: &str| l.last(&p(n)).unwrap_or_else(|| panic!("missing {n}"));
    assert_eq!(k("HR"), OscType::Int(128));
    assert_eq!(k("onesHR"), OscType::Int(8));
    assert_eq!(k("tensHR"), OscType::Int(2));
    assert_eq!(k("hundredsHR"), OscType::Int(1));
    assert_eq!(k("isHRConnected"), OscType::Bool(true));
    assert_eq!(k("isHRActive"), OscType::Bool(true));
    assert_eq!(k("HRPercent"), f(pct));
    assert_eq!(k("FullHRPercent"), f(((2.0 * 128.0f64 / 255.0) - 1.0) as f32));
    assert!(
        l.wait_for(Duration::from_secs(3), |l| {
            l.values(&p("isHRBeat")).contains(&OscType::Bool(true))
                && l.values(&p("HeartBeatToggle")).len() >= 2
        })
        .await
    );
    let allowed = [
        "HR",
        "onesHR",
        "tensHR",
        "hundredsHR",
        "isHRConnected",
        "isHRActive",
        "isHRBeat",
        "HeartBeatToggle",
        "HRPercent",
        "FullHRPercent",
    ];
    for (_, addr, _) in l.all() {
        let name = addr.strip_prefix("/avatar/parameters/").expect("avatar param");
        assert!(allowed.contains(&name), "unexpected {name}");
    }
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vrcosc_preset_exact_values() {
    let l = Listener::new().await;
    let (h, _) = start(
        base_config(vec![target("t", l.port, Preset::Vrcosc)]),
        steady(20, 96, 625.0, 100),
    );
    let p = |n: &str| format!("/avatar/parameters/VRCOSC/Heartrate/{n}");
    assert!(
        l.wait_for(Duration::from_secs(5), |l| l.last(&p("Value"))
            == Some(OscType::Int(96)))
            .await
    );
    let k = |n: &str| l.last(&p(n)).unwrap_or_else(|| panic!("missing {n}"));
    assert_eq!(k("Enabled"), OscType::Bool(true));
    assert_eq!(k("Connected"), OscType::Bool(true));
    assert_eq!(k("Value"), OscType::Int(96));
    assert_eq!(k("Average"), OscType::Int(96));
    assert_eq!(k("Normalised"), f((96.0f64 / 240.0) as f32));
    assert_eq!(k("Units"), f(0.6));
    assert_eq!(k("Tens"), f(0.9));
    assert_eq!(k("Hundreds"), f(0.0));
    assert!(
        l.wait_for(Duration::from_secs(3), |l| l.values(&p("Beat")).len() >= 2)
            .await
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_preset_exact_values() {
    let l = Listener::new().await;
    let mut t = target("t", l.port, Preset::Custom);
    t.custom = vec![
        CustomMapping {
            address: "MyHeart".into(),
            metric: Metric::Hr,
            kind: ValueKind::Int,
        },
        CustomMapping {
            address: "/custom/hr_float".into(),
            metric: Metric::Hr,
            kind: ValueKind::Float,
        },
        CustomMapping {
            address: "Alive".into(),
            metric: Metric::Connected,
            kind: ValueKind::Bool,
        },
        CustomMapping {
            address: "Zone".into(),
            metric: Metric::Zone,
            kind: ValueKind::Int,
        },
    ];
    t.range = Some((50, 150));
    let (h, _) = start(base_config(vec![t]), steady(20, 100, 600.0, 100));
    assert!(
        l.wait_for(Duration::from_secs(5), |l| l.last("/avatar/parameters/MyHeart")
            == Some(OscType::Int(100)))
            .await
    );
    assert_eq!(l.last("/custom/hr_float"), Some(f(100.0)));
    assert_eq!(l.last("/avatar/parameters/Alive"), Some(OscType::Bool(true)));
    assert_eq!(l.last("/avatar/parameters/Zone"), Some(OscType::Int(1))); // 100/190 = 52.6 % of max HR
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiple_targets_receive_their_own_preset() {
    let a = Listener::new().await;
    let b = Listener::new().await;
    let (h, _) = start(
        base_config(vec![
            target("a", a.port, Preset::KittyTooth),
            target("b", b.port, Preset::Vrcosc),
        ]),
        steady(10, 80, 750.0, 100),
    );
    assert!(
        a.wait_for(Duration::from_secs(5), |l| l.last(&format!("{KT}HR"))
            == Some(OscType::Int(80)))
            .await
    );
    assert!(
        b.wait_for(Duration::from_secs(5), |l| l
            .last("/avatar/parameters/VRCOSC/Heartrate/Value")
            == Some(OscType::Int(80)))
            .await
    );
    assert!(a.all().iter().all(|(_, addr, _)| addr.starts_with(KT)));
    assert!(b.all().iter().all(|(_, addr, _)| addr.contains("VRCOSC")));
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_sends_connected_false_once_and_holds() {
    let l = Listener::new().await;
    let (h, _) = start(
        base_config(vec![target("t", l.port, Preset::KittyTooth)]),
        steady(200, 70, 857.0, 50),
    );
    assert!(
        l.wait_for(Duration::from_secs(5), |l| l.last(&format!("{KT}HR"))
            == Some(OscType::Int(70)))
            .await
    );
    h.disconnect().await.unwrap();
    assert!(
        l.wait_for(Duration::from_secs(2), |l| l.last(&format!("{KT}Connected"))
            == Some(OscType::Bool(false)))
            .await
    );
    tokio::time::sleep(Duration::from_millis(2500)).await; // > 2 keepalive periods
    let falses = l
        .values(&format!("{KT}Connected"))
        .iter()
        .rev()
        .take_while(|v| **v == OscType::Bool(false))
        .count();
    assert_eq!(falses, 1, "Connected=false is sent exactly once");
    assert_eq!(
        l.last(&format!("{KT}HR")),
        Some(OscType::Int(70)),
        "hold last value"
    );
    assert_eq!(h.snapshot().state, ConnState::Idle);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_send_zero_mode() {
    let l = Listener::new().await;
    let mut cfg = base_config(vec![target("t", l.port, Preset::HrToVrchat)]);
    cfg.osc.on_disconnect = DisconnectBehavior::SendZero;
    let (h, _) = start(cfg, steady(200, 70, 857.0, 50));
    let p = |n: &str| format!("/avatar/parameters/{n}");
    assert!(
        l.wait_for(Duration::from_secs(5), |l| l.last(&p("HR"))
            == Some(OscType::Int(70)))
            .await
    );
    h.disconnect().await.unwrap();
    assert!(
        l.wait_for(Duration::from_secs(2), |l| l.last(&p("HR"))
            == Some(OscType::Int(0))
            && l.last(&p("isHRConnected")) == Some(OscType::Bool(false))
            && l.last(&p("onesHR")) == Some(OscType::Int(0)))
            .await
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_data_marks_disconnected_and_reconnects() {
    let l = Listener::new().await;
    let mut cfg = base_config(vec![target("t", l.port, Preset::KittyTooth)]);
    cfg.device.stale_timeout_secs = 2;
    // Two packets then silence.
    let (h, backend) = start(
        cfg,
        vec![MockStep::hr(75), MockStep::wait_ms(200), MockStep::hr(75)],
    );
    assert!(
        l.wait_for(Duration::from_secs(3), |l| l.last(&format!("{KT}HR"))
            == Some(OscType::Int(75)))
            .await
    );
    let t0 = Instant::now();
    assert!(
        l.wait_for(Duration::from_secs(4), |l| l.last(&format!("{KT}Connected"))
            == Some(OscType::Bool(false)))
            .await
    );
    let waited = t0.elapsed();
    assert!(
        waited >= Duration::from_millis(1500),
        "stale too early: {waited:?}"
    );
    // Reconnect happens automatically (backoff 1 s), and data flows again.
    let reconnected = async {
        while backend.connect_calls() < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(4), reconnected)
        .await
        .expect("reconnect attempt");
    assert!(
        l.wait_for(Duration::from_secs(3), |l| l.last(&format!("{KT}Connected"))
            == Some(OscType::Bool(true)))
            .await
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_backoff_after_failures() {
    let l = Listener::new().await;
    let backend = Arc::new(MockBackend::new(vec![MockDevice::scripted(
        "test:1",
        "Test Strap",
        steady(100, 70, 857.0, 100),
    )]));
    backend.fail_next_connects(3);
    let t0 = Instant::now();
    let h = engine::start(
        base_config(vec![target("t", l.port, Preset::KittyTooth)]),
        None,
        backend.clone(),
        EngineOptions {
            mdns: false,
            ..Default::default()
        },
    );
    // Attempts at ~0 s, +1 s, +2 s, +4 s → success at ~7 s.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(backend.connect_calls(), 1);
    let s = h.snapshot();
    assert_eq!(s.state, ConnState::Reconnecting);
    assert_eq!(s.error.as_ref().map(|e| e.code.as_str()), Some("connect_timeout"));
    assert!(
        l.wait_for(Duration::from_secs(12), |l| l.last(&format!("{KT}HR"))
            == Some(OscType::Int(70)))
            .await
    );
    let took = t0.elapsed();
    assert_eq!(backend.connect_calls(), 4);
    assert!(
        took >= Duration::from_millis(6500) && took < Duration::from_secs(12),
        "{took:?}"
    );
    assert_eq!(h.snapshot().state, ConnState::Connected);
    assert!(h.snapshot().error.is_none());
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn link_loss_reconnects_without_user_action() {
    let l = Listener::new().await;
    let steps = vec![
        MockStep::hr(70),
        MockStep::wait_ms(100),
        MockStep::hr(70),
        MockStep::LinkLost,
    ];
    let (h, backend) = start(base_config(vec![target("t", l.port, Preset::KittyTooth)]), steps);
    assert!(
        l.wait_for(Duration::from_secs(3), |l| l
            .values(&format!("{KT}Connected"))
            .contains(&OscType::Bool(false)))
            .await
    );
    let r = async {
        while backend.connect_calls() < 3 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(6), r)
        .await
        .expect("keeps reconnecting");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rate_limit_never_exceeded_under_flood() {
    let l = Listener::new().await;
    let mut steps = vec![];
    for i in 0..300u16 {
        steps.push(MockStep::hr_rr(60 + (i % 100), &[400.0]));
        steps.push(MockStep::wait_ms(10));
    }
    let (h, _) = start(base_config(vec![target("t", l.port, Preset::KittyTooth)]), steps);
    tokio::time::sleep(Duration::from_secs(4)).await;
    let all = l.all();
    assert!(all.len() > 40, "got {} messages", all.len());
    for (i, (t, _, _)) in all.iter().enumerate() {
        let in_window = all[i..]
            .iter()
            .take_while(|(x, _, _)| *x < *t + Duration::from_millis(950))
            .count();
        assert!(in_window <= 20, "{in_window} messages within one second");
    }
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keepalive_resends_every_second() {
    let l = Listener::new().await;
    // One packet per 300 ms with identical values; keepalive should still refresh HR.
    let (h, _) = start(
        base_config(vec![target("t", l.port, Preset::KittyTooth)]),
        steady(30, 70, 857.0, 300),
    );
    tokio::time::sleep(Duration::from_millis(3600)).await;
    let n = l.values(&format!("{KT}HR")).len();
    assert!((3..=6).contains(&n), "HR sent {n} times in 3.6 s");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spikes_out_of_range_and_contact_loss_are_filtered() {
    let l = Listener::new().await;
    let mut cfg = base_config(vec![target("t", l.port, Preset::KittyTooth)]);
    cfg.device.stale_timeout_secs = 2;
    let mut steps = vec![
        MockStep::hr(70),
        MockStep::wait_ms(100),
        MockStep::hr(71),
        MockStep::wait_ms(100),
        MockStep::hr(200), // single-sample spike
        MockStep::wait_ms(100),
        MockStep::hr(72),
        MockStep::wait_ms(100),
        MockStep::hr(20), // impossible
        MockStep::wait_ms(100),
        MockStep::Raw(vec![0x10, 0xFA]), // 250, valid edge but a spike vs 72
        MockStep::wait_ms(100),
        MockStep::hr(73),
        MockStep::wait_ms(100),
    ];
    // Then lose skin contact for longer than the stale timeout, while packets keep flowing.
    for _ in 0..30 {
        steps.push(MockStep::Hr {
            bpm: 0,
            rr_ms: vec![],
            contact: SensorContact::NotDetected,
        });
        steps.push(MockStep::wait_ms(100));
    }
    let (h, backend) = start(cfg, steps);
    assert!(
        l.wait_for(Duration::from_secs(3), |l| l.last(&format!("{KT}HR"))
            == Some(OscType::Int(73)))
            .await
    );
    let hrs = l.values(&format!("{KT}HR"));
    assert!(
        !hrs.contains(&OscType::Int(200))
            && !hrs.contains(&OscType::Int(20))
            && !hrs.contains(&OscType::Int(250))
    );
    assert!(
        l.wait_for(Duration::from_secs(4), |l| l.last(&format!("{KT}Connected"))
            == Some(OscType::Bool(false)))
            .await,
        "sustained contact loss marks disconnected"
    );
    assert_eq!(backend.connect_calls(), 1, "packets still flowing: no reconnect");
    assert_eq!(h.snapshot().state, ConnState::NoData);
    assert!(h.snapshot().rejected_spikes >= 1);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chatbox_template_and_min_interval() {
    let l = Listener::new().await;
    let mut cfg = base_config(vec![target("t", l.port, Preset::KittyTooth)]);
    cfg.osc.chatbox = ChatboxConfig {
        enabled: true,
        template: "{hr} bpm".into(),
        interval_secs: 2.0,
        silent: true,
    };
    let mut steps = vec![];
    for i in 0..60u16 {
        steps.push(MockStep::hr(70 + (i / 10)));
        steps.push(MockStep::wait_ms(100));
    }
    let (h, _) = start(cfg, steps);
    tokio::time::sleep(Duration::from_millis(4600)).await;
    let chats: Vec<_> = l
        .all()
        .into_iter()
        .filter(|(_, a, _)| a == "/chatbox/input")
        .collect();
    assert!(chats.len() >= 2, "got {} chatbox messages", chats.len());
    for w in chats.windows(2) {
        assert!(
            w[1].0 - w[0].0 >= Duration::from_millis(1950),
            "chatbox sent too fast"
        );
    }
    let (_, _, args) = &chats[0];
    assert_eq!(args.len(), 3);
    assert!(matches!(&args[0], OscType::String(s) if s.ends_with(" bpm")));
    assert_eq!(args[1], OscType::Bool(true));
    assert_eq!(args[2], OscType::Bool(false), "silent: no notification sound");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_sweep_works_without_sensor() {
    let l = Listener::new().await;
    let mut cfg = base_config(vec![target("t", l.port, Preset::KittyTooth)]);
    cfg.device.last_device = None;
    let backend = Arc::new(MockBackend::new(vec![]));
    let h = engine::start(
        cfg,
        None,
        backend,
        EngineOptions {
            mdns: false,
            ..Default::default()
        },
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.test_sweep(true).await.unwrap();
    assert!(
        l.wait_for(Duration::from_secs(6), |l| l
            .values(&format!("{KT}HR"))
            .iter()
            .any(|v| matches!(v, OscType::Int(x) if *x > 100)))
            .await
    );
    assert_eq!(l.last(&format!("{KT}Connected")), Some(OscType::Bool(true)));
    assert!(l.values(&format!("{KT}Beat")).len() >= 2);
    assert!(h.osc_log().iter().any(|e| e.test));
    assert!(h.snapshot().test_sweep);
    h.test_sweep(false).await.unwrap();
    assert!(
        l.wait_for(Duration::from_secs(2), |l| l.last(&format!("{KT}Connected"))
            == Some(OscType::Bool(false)))
            .await
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn avatar_change_via_oscquery_resends_everything() {
    let l = Listener::new().await;
    let mut cfg = base_config(vec![target("t", l.port, Preset::KittyTooth)]);
    cfg.osc.oscquery = true;
    cfg.osc.keepalive_ms = 0; // so only the avatar change can cause a resend
    let (h, _) = start(cfg, steady(100, 70, 857.0, 400));
    assert!(
        l.wait_for(Duration::from_secs(3), |l| l.last(&format!("{KT}HR"))
            == Some(OscType::Int(70)))
            .await
    );
    let osc_port = h.snapshot().oscquery.osc_port.expect("oscquery osc port");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = l.values(&format!("{KT}Zone")).len();
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pkt = rosc::encoder::encode(&OscPacket::Message(rosc::OscMessage {
        addr: "/avatar/change".into(),
        args: vec![OscType::String("avtr_123".into())],
    }))
    .unwrap();
    sock.send_to(&pkt, SocketAddr::from(([127, 0, 0, 1], osc_port)))
        .await
        .unwrap();
    assert!(
        l.wait_for(Duration::from_millis(1500), |l| l
            .values(&format!("{KT}Zone"))
            .len()
            > before)
            .await,
        "unchanged params resent immediately on avatar change"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlay_serves_html_and_state() {
    let l = Listener::new().await;
    let (h, _) = start(
        base_config(vec![target("t", l.port, Preset::KittyTooth)]),
        steady(50, 66, 909.0, 100),
    );
    assert!(l.wait_for(Duration::from_secs(3), |l| l.len() > 0).await);
    let srv = kittytooth_core::overlay::start(h.clone(), 0).await.unwrap();
    assert!(srv.url().ends_with("/overlay"));
    let (s, body) = kittytooth_core::http::get(srv.addr, "/overlay", Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(s, 200);
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("EventSource") && html.contains("background: transparent"));
    let (s, font) =
        kittytooth_core::http::get(srv.addr, "/fonts/bricolage-800.woff2", Duration::from_secs(2))
            .await
            .unwrap();
    assert_eq!(s, 200);
    assert_eq!(&font[..4], b"wOF2");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (s, body) = kittytooth_core::http::get(srv.addr, "/api/state", Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(s, 200);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["bpm"], 66);
    assert_eq!(v["connected"], true);
    // A second server on the same port reports a friendly error.
    let err = kittytooth_core::overlay::start(h.clone(), srv.addr.port())
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), "port_in_use");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readings_are_recorded_to_session_history() {
    let dir = std::env::temp_dir().join(format!("kittytooth-pipeline-history-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let backend = Arc::new(MockBackend::new(vec![MockDevice::scripted(
        "test:1",
        "Test Strap",
        steady(60, 72, 833.0, 100),
    )]));
    let h = engine::start(
        base_config(vec![]),
        None,
        backend,
        EngineOptions {
            mdns: false,
            history_dir: Some(dir.clone()),
            ..Default::default()
        },
    );
    let store = h.sessions().expect("history is set up").clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    while store.list().first().is_none_or(|s| s.samples < 2) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let live = store.list();
    assert_eq!(live.len(), 1);
    assert!(live[0].active);
    assert_eq!(live[0].device.as_deref(), Some("Test Strap"));
    assert_eq!(live[0].max_bpm, 72);
    let session = store.load(&live[0].id).unwrap();
    assert!(session.samples.iter().any(|s| s.rr_ms.contains(&833)));
    // Reset session ends it; a few seconds is too short to keep.
    h.reset_session().await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while store.active_id().is_some() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The next reading starts a new session unless recording is off.
    let mut cfg = h.config();
    cfg.history.enabled = false;
    h.set_config(cfg).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(store.list().is_empty(), "{:?}", store.list());
    h.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
