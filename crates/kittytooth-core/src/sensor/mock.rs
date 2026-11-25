//! Mock sensors: a realistic demo strap/watch for the UI, and scripted sensors for tests.
//!
//! The demo strap follows a 6-minute loop: rest, exercise ramp, peak, recovery. Along the way
//! it injects the real-world problems KittyTooth must survive: a single-sample spike, 6 s of
//! lost skin contact, and an 8 s dropout (longer than the default stale timeout, so the
//! engine's stale detection and auto-reconnect run). RR intervals carry realistic
//! respiratory sinus arrhythmia so HRV is meaningful. All payloads are real `0x2A37` bytes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rand::{Rng, SeedableRng};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::{KtError, KtResult};
use crate::hrm::{encode, ms_to_rr, BodySensorLocation, HeartRateMeasurement, SensorContact};
use crate::osc::sender::epoch_ms;
use crate::sensor::{
    AdapterState, DeviceInfo, DiscoveredDevice, HeartRateBackend, SensorEvent, SensorSession, DEMO_PREFIX,
};

/// One step of a scripted sensor.
#[derive(Debug, Clone, PartialEq)]
pub enum MockStep {
    /// Send a measurement.
    Hr {
        bpm: u16,
        rr_ms: Vec<f64>,
        contact: SensorContact,
    },
    /// Send raw bytes as a measurement payload.
    Raw(Vec<u8>),
    Battery(u8),
    Wait(Duration),
    /// Simulate the link dropping.
    LinkLost,
}

impl MockStep {
    pub fn hr(bpm: u16) -> Self {
        MockStep::Hr {
            bpm,
            rr_ms: vec![],
            contact: SensorContact::Detected,
        }
    }

    pub fn hr_rr(bpm: u16, rr_ms: &[f64]) -> Self {
        MockStep::Hr {
            bpm,
            rr_ms: rr_ms.to_vec(),
            contact: SensorContact::Detected,
        }
    }

    pub fn wait_ms(ms: u64) -> Self {
        MockStep::Wait(Duration::from_millis(ms))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MockScript {
    /// Realistic chest strap with RR intervals, contact detection and battery.
    DemoStrap,
    /// Watch in HR-broadcast mode: 8-bit HR only, no RR, no contact, no battery.
    DemoWatch,
    /// Deterministic steps (tests). After the last step the sensor stays silent.
    Steps(Vec<MockStep>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MockDevice {
    pub id: String,
    pub name: String,
    pub rssi: i16,
    pub has_hr_service: bool,
    pub location: Option<BodySensorLocation>,
    pub script: MockScript,
}

impl MockDevice {
    pub fn scripted(id: &str, name: &str, steps: Vec<MockStep>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            rssi: -55,
            has_hr_service: true,
            location: Some(BodySensorLocation::Chest),
            script: MockScript::Steps(steps),
        }
    }
}

pub struct MockBackend {
    devices: Vec<MockDevice>,
    started: Instant,
    scan: Mutex<Option<JoinHandle<()>>>,
    adapter: Mutex<AdapterState>,
    fail_next: AtomicU32,
    connects: AtomicU32,
}

impl MockBackend {
    /// The demo devices shown in the app.
    pub fn demo() -> Self {
        Self::new(vec![
            MockDevice {
                id: format!("{DEMO_PREFIX}strap"),
                name: "KittyTooth Demo Strap".into(),
                rssi: -52,
                has_hr_service: true,
                location: Some(BodySensorLocation::Chest),
                script: MockScript::DemoStrap,
            },
            MockDevice {
                id: format!("{DEMO_PREFIX}watch"),
                name: "KittyTooth Demo Watch".into(),
                rssi: -71,
                has_hr_service: true,
                location: Some(BodySensorLocation::Wrist),
                script: MockScript::DemoWatch,
            },
            MockDevice {
                id: format!("{DEMO_PREFIX}mouse"),
                name: "Demo Mouse (no heart rate)".into(),
                rssi: -80,
                has_hr_service: false,
                location: None,
                script: MockScript::Steps(vec![]),
            },
        ])
    }

    pub fn new(devices: Vec<MockDevice>) -> Self {
        Self {
            devices,
            started: Instant::now(),
            scan: Mutex::new(None),
            adapter: Mutex::new(AdapterState::Ready),
            fail_next: AtomicU32::new(0),
            connects: AtomicU32::new(0),
        }
    }

    /// Makes the next `n` connection attempts fail with a timeout (tests).
    pub fn fail_next_connects(&self, n: u32) {
        self.fail_next.store(n, Ordering::SeqCst);
    }

    pub fn set_adapter_state(&self, s: AdapterState) {
        if let Ok(mut a) = self.adapter.lock() {
            *a = s;
        }
    }

    /// Number of successful + failed connect calls.
    pub fn connect_calls(&self) -> u32 {
        self.connects.load(Ordering::SeqCst)
    }

    fn adapter(&self) -> AdapterState {
        self.adapter.lock().map(|a| *a).unwrap_or(AdapterState::Unknown)
    }
}

#[async_trait]
impl HeartRateBackend for MockBackend {
    async fn adapter_state(&self) -> AdapterState {
        self.adapter()
    }

    async fn start_scan(&self, show_all: bool, found: mpsc::Sender<DiscoveredDevice>) -> KtResult<()> {
        if let Some(e) = self.adapter().error() {
            return Err(e);
        }
        let devices: Vec<MockDevice> = self
            .devices
            .iter()
            .filter(|d| show_all || d.has_hr_service)
            .cloned()
            .collect();
        let task = tokio::spawn(async move {
            let mut rng = rand::rngs::StdRng::seed_from_u64(7);
            loop {
                for d in &devices {
                    let dev = DiscoveredDevice {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        rssi: Some(d.rssi + rng.random_range(-4..=4)),
                        has_hr_service: d.has_hr_service,
                        last_seen_ms: epoch_ms(),
                        demo: d.id.starts_with(DEMO_PREFIX),
                    };
                    if found.send(dev).await.is_err() {
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        });
        if let Ok(mut s) = self.scan.lock() {
            if let Some(old) = s.replace(task) {
                old.abort();
            }
        }
        Ok(())
    }

    async fn stop_scan(&self) -> KtResult<()> {
        if let Ok(mut s) = self.scan.lock() {
            if let Some(t) = s.take() {
                t.abort();
            }
        }
        Ok(())
    }

    async fn connect(
        &self,
        id: &str,
        _name_hint: &str,
        events: mpsc::Sender<SensorEvent>,
    ) -> KtResult<Box<dyn SensorSession>> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        if let Some(e) = self.adapter().error() {
            return Err(e);
        }
        let dev = self
            .devices
            .iter()
            .find(|d| d.id == id)
            .cloned()
            .ok_or_else(|| KtError::DeviceNotFound(id.to_string()))?;
        if take_one(&self.fail_next) {
            return Err(KtError::ConnectTimeout(dev.name.clone()));
        }
        if !dev.has_hr_service {
            return Err(KtError::NoHeartRateService(dev.name.clone()));
        }
        let started = self.started;
        let script = dev.script.clone();
        let task = tokio::spawn(async move {
            match script {
                MockScript::Steps(steps) => run_steps(steps, events).await,
                MockScript::DemoStrap => run_demo(started, true, events).await,
                MockScript::DemoWatch => run_demo(started, false, events).await,
            }
        });
        Ok(Box::new(MockSession {
            info: DeviceInfo {
                id: dev.id.clone(),
                name: dev.name.clone(),
                location: dev.location,
                demo: dev.id.starts_with(DEMO_PREFIX),
            },
            task: Some(task),
        }))
    }
}

struct MockSession {
    info: DeviceInfo,
    task: Option<JoinHandle<()>>,
}

#[async_trait]
impl SensorSession for MockSession {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    async fn disconnect(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

impl Drop for MockSession {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

async fn run_steps(steps: Vec<MockStep>, events: mpsc::Sender<SensorEvent>) {
    for s in steps {
        let ev = match s {
            MockStep::Hr { bpm, rr_ms, contact } => {
                let m = HeartRateMeasurement {
                    bpm,
                    u16_format: false,
                    contact,
                    energy_expended_kj: None,
                    rr_intervals: rr_ms.iter().map(|&ms| ms_to_rr(ms)).collect(),
                };
                SensorEvent::Measurement {
                    data: encode(&m),
                    received: Instant::now(),
                }
            }
            MockStep::Raw(data) => SensorEvent::Measurement {
                data,
                received: Instant::now(),
            },
            MockStep::Battery(b) => SensorEvent::Battery(b),
            MockStep::Wait(d) => {
                tokio::time::sleep(d).await;
                continue;
            }
            MockStep::LinkLost => {
                let _ = events
                    .send(SensorEvent::Disconnected(Some(KtError::Ble("link lost".into()))))
                    .await;
                return;
            }
        };
        if events.send(ev).await.is_err() {
            return;
        }
    }
    // Stay "connected" but silent; the session is closed by the engine.
    std::future::pending::<()>().await;
}

/// Demo loop length in seconds.
pub const DEMO_LOOP_SECS: f64 = 360.0;

/// Target heart rate of the demo at loop time `t` (seconds).
pub fn demo_target_hr(t: f64) -> f64 {
    let t = t.rem_euclid(DEMO_LOOP_SECS);
    let ease = |x: f64| x * x * (3.0 - 2.0 * x);
    match t {
        t if t < 60.0 => 64.0,
        t if t < 180.0 => 64.0 + (158.0 - 64.0) * ease((t - 60.0) / 120.0),
        t if t < 240.0 => 158.0 + 6.0 * ((t - 180.0) / 9.0).sin(),
        t if t < 330.0 => 70.0 + (158.0 - 70.0) * (-(t - 240.0) / 25.0).exp(),
        _ => 66.0,
    }
}

/// Scripted problems in the demo loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoEvent {
    Normal,
    Spike,
    ContactLost,
    Dropout,
}

pub fn demo_event(t: f64) -> DemoEvent {
    let t = t.rem_euclid(DEMO_LOOP_SECS);
    if (100.0..101.0).contains(&t) {
        DemoEvent::Spike
    } else if (150.0..156.0).contains(&t) {
        DemoEvent::ContactLost
    } else if (280.0..288.0).contains(&t) {
        DemoEvent::Dropout
    } else {
        DemoEvent::Normal
    }
}

fn gaussian(rng: &mut impl Rng) -> f64 {
    // Box-Muller
    let u1: f64 = rng.random_range(1e-9..1.0);
    let u2: f64 = rng.random_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

async fn run_demo(started: Instant, strap: bool, events: mpsc::Sender<SensorEvent>) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(if strap { 42 } else { 43 });
    let t_now = || started.elapsed().as_secs_f64();
    let mut next_beat = t_now();
    let mut last_rr_ms = 900.0;
    let mut energy: u16 = 0;
    let mut ticks: u64 = 0;
    let battery = |t: f64| (92.0 - t / 600.0).clamp(5.0, 100.0) as u8;
    if strap && events.send(SensorEvent::Battery(battery(t_now()))).await.is_err() {
        return;
    }
    let mut interval = tokio::time::interval(Duration::from_millis(1000));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        ticks += 1;
        let t = t_now();
        // Generate the beats that happened since the last packet.
        let mut rr = Vec::new();
        while next_beat <= t {
            let hr = demo_target_hr(next_beat);
            // Respiratory sinus arrhythmia: stronger at rest.
            let rsa = (1.0 - ((hr - 60.0) / 110.0).clamp(0.0, 1.0)) * 0.06;
            let breath = (2.0 * std::f64::consts::PI * next_beat / 4.8).sin();
            let ms = 60_000.0 / hr * (1.0 + rsa * breath + gaussian(&mut rng) * 0.012);
            let ms = ms.clamp(280.0, 2000.0);
            rr.push(ms);
            last_rr_ms = ms;
            next_beat += ms / 1000.0;
        }
        let event = demo_event(t);
        if event == DemoEvent::Dropout {
            continue; // radio silence
        }
        let mean_rr = if rr.is_empty() {
            last_rr_ms
        } else {
            rr.iter().sum::<f64>() / rr.len() as f64
        };
        let mut bpm = (60_000.0 / mean_rr).round() as u16;
        let mut contact = if strap {
            SensorContact::Detected
        } else {
            SensorContact::NotSupported
        };
        match event {
            DemoEvent::Spike => bpm = 232,
            DemoEvent::ContactLost if strap => {
                contact = SensorContact::NotDetected;
                bpm = 0;
                rr.clear();
            }
            _ => {}
        }
        let m = if strap {
            energy = energy.saturating_add((bpm as f64 / 40.0) as u16);
            HeartRateMeasurement {
                bpm,
                u16_format: false,
                contact,
                energy_expended_kj: ticks.is_multiple_of(10).then_some(energy),
                rr_intervals: rr.iter().map(|&ms| ms_to_rr(ms)).collect(),
            }
        } else {
            HeartRateMeasurement::simple((bpm as i32 + rng.random_range(-1..=1)).clamp(0, 255) as u8)
        };
        let ev = SensorEvent::Measurement {
            data: encode(&m),
            received: Instant::now(),
        };
        if events.send(ev).await.is_err() {
            return;
        }
        if strap && ticks.is_multiple_of(60) && events.send(SensorEvent::Battery(battery(t))).await.is_err() {
            return;
        }
    }
}

/// Atomically decrements `n` if it is above zero; true when a unit was taken.
/// (A CAS loop rather than `fetch_update`, which is deprecated on newer toolchains.)
fn take_one(n: &std::sync::atomic::AtomicU32) -> bool {
    let mut cur = n.load(Ordering::SeqCst);
    while cur > 0 {
        match n.compare_exchange_weak(cur, cur - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(actual) => cur = actual,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hrm::parse;

    #[test]
    fn demo_profile_is_plausible() {
        for i in 0..3600 {
            let hr = demo_target_hr(i as f64 / 10.0);
            assert!((55.0..=170.0).contains(&hr), "t={} hr={hr}", i as f64 / 10.0);
        }
        assert!(demo_target_hr(30.0) < 70.0);
        assert!(demo_target_hr(200.0) > 145.0);
        assert_eq!(demo_event(100.5), DemoEvent::Spike);
        assert_eq!(demo_event(152.0), DemoEvent::ContactLost);
        assert_eq!(demo_event(285.0), DemoEvent::Dropout);
        assert_eq!(demo_event(285.0 + DEMO_LOOP_SECS), DemoEvent::Dropout);
        assert_eq!(demo_event(10.0), DemoEvent::Normal);
    }

    #[tokio::test]
    async fn scripted_sensor_emits_valid_packets() {
        let b = MockBackend::new(vec![MockDevice::scripted(
            "t:1",
            "Test",
            vec![
                MockStep::Battery(77),
                MockStep::hr_rr(70, &[857.0]),
                MockStep::LinkLost,
            ],
        )]);
        let (tx, mut rx) = mpsc::channel(8);
        let s = b.connect("t:1", "", tx).await.unwrap();
        assert_eq!(s.info().name, "Test");
        assert_eq!(rx.recv().await, Some(SensorEvent::Battery(77)));
        match rx.recv().await {
            Some(SensorEvent::Measurement { data, .. }) => {
                let m = parse(&data).unwrap();
                assert_eq!(m.bpm, 70);
                assert_eq!(m.contact, SensorContact::Detected);
                assert_eq!(m.rr_intervals.len(), 1);
            }
            o => panic!("{o:?}"),
        }
        assert!(matches!(
            rx.recv().await,
            Some(SensorEvent::Disconnected(Some(_)))
        ));
    }

    #[tokio::test]
    async fn failure_injection_and_errors() {
        let b = MockBackend::demo();
        let (tx, _rx) = mpsc::channel(8);
        b.fail_next_connects(1);
        assert!(matches!(
            b.connect("demo:strap", "", tx.clone()).await.err(),
            Some(KtError::ConnectTimeout(_))
        ));
        assert!(b.connect("demo:strap", "", tx.clone()).await.is_ok());
        assert!(matches!(
            b.connect("demo:mouse", "", tx.clone()).await.err(),
            Some(KtError::NoHeartRateService(_))
        ));
        assert!(matches!(
            b.connect("nope", "", tx.clone()).await.err(),
            Some(KtError::DeviceNotFound(_))
        ));
        b.set_adapter_state(AdapterState::Off);
        assert_eq!(
            b.connect("demo:strap", "", tx).await.err(),
            Some(KtError::RadioOff)
        );
        assert_eq!(b.connect_calls(), 5);
    }

    #[tokio::test]
    async fn scan_filters_non_hr_unless_show_all() {
        let b = MockBackend::demo();
        let (tx, mut rx) = mpsc::channel(16);
        b.start_scan(false, tx).await.unwrap();
        let mut names = vec![];
        for _ in 0..2 {
            names.push(rx.recv().await.unwrap().name);
        }
        b.stop_scan().await.unwrap();
        assert!(names.iter().all(|n| !n.contains("Mouse")));
        let (tx, mut rx) = mpsc::channel(16);
        b.start_scan(true, tx).await.unwrap();
        let mut all = vec![];
        for _ in 0..3 {
            all.push(rx.recv().await.unwrap());
        }
        b.stop_scan().await.unwrap();
        assert!(all.iter().any(|d| !d.has_hr_service));
    }

    #[tokio::test]
    async fn demo_strap_streams_rr() {
        let b = MockBackend::demo();
        let (tx, mut rx) = mpsc::channel(8);
        let _s = b.connect("demo:strap", "", tx).await.unwrap();
        assert!(matches!(rx.recv().await, Some(SensorEvent::Battery(_))));
        // First interval tick fires immediately.
        let Some(SensorEvent::Measurement { data, .. }) = rx.recv().await else {
            panic!()
        };
        let m = parse(&data).unwrap();
        assert!((55..=80).contains(&m.bpm), "{}", m.bpm);
        assert_eq!(m.contact, SensorContact::Detected);
    }
}
