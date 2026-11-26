//! ANT+ end to end: a scripted ANT stick (speaking the real ANT message protocol) feeds the
//! engine through `DefaultFactory`, and heart rate plus HRV derived from beat event times come
//! out the other side. An unplugged stick is reported and retried.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kittytooth_core::config::Config;
use kittytooth_core::engine::{self, EngineHandle, EngineOptions, Snapshot};
use kittytooth_core::error::{KtError, KtResult};
use kittytooth_core::sensor::mock::MockBackend;
use kittytooth_core::source::ant::protocol::*;
use kittytooth_core::source::ant::{StickOpener, Transport};
use kittytooth_core::source::{DefaultFactory, MemorySecrets, SourceKind};

/// Answers every command and, once the channel is open, plays HRM pages at 20x real speed with
/// beats alternating 800 / 860 ms apart (so RMSSD is 60 ms).
struct ScriptedStick {
    out: VecDeque<u8>,
    open: bool,
    beat: u8,
    event: u16,
    repeats: u8,
    unplugged: Arc<AtomicBool>,
}

impl Transport for ScriptedStick {
    fn write(&mut self, frame: &[u8]) -> KtResult<()> {
        let id = frame[2];
        let reply = match id {
            MSG_RESET => encode(MSG_STARTUP, &[0x20]),
            MSG_REQUEST => encode(MSG_CHANNEL_ID, &[0, 0x39, 0x30, HRM_DEVICE_TYPE, 1]),
            _ => {
                self.open |= id == MSG_OPEN_CHANNEL;
                encode(MSG_CHANNEL_EVENT, &[0, id, RESPONSE_NO_ERROR])
            }
        };
        self.out.extend(reply);
        Ok(())
    }

    fn read(&mut self, _timeout: Duration) -> KtResult<Vec<u8>> {
        if self.unplugged.load(Ordering::Relaxed) {
            return Err(KtError::AntStickMissing);
        }
        if !self.out.is_empty() {
            return Ok(self.out.drain(..).collect());
        }
        if !self.open {
            return Ok(vec![]);
        }
        std::thread::sleep(Duration::from_millis(12));
        // Each beat is repeated on 4 pages, as a real monitor does at ~4 Hz.
        self.repeats += 1;
        if self.repeats == 4 {
            self.repeats = 0;
            self.beat = self.beat.wrapping_add(1);
            let rr_ms: u32 = if self.beat.is_multiple_of(2) { 800 } else { 860 };
            self.event = self.event.wrapping_add((rr_ms * 1024 / 1000) as u16);
        }
        let [e0, e1] = self.event.to_le_bytes();
        let page = [0x00, 0xFF, 0xFF, 0xFF, e0, e1, self.beat, 72];
        let mut data = vec![0];
        data.extend_from_slice(&page);
        Ok(encode(MSG_BROADCAST_DATA, &data))
    }
}

struct Opener {
    present: Arc<AtomicBool>,
    unplugged: Arc<AtomicBool>,
    opens: AtomicUsize,
}

impl StickOpener for Opener {
    fn open(&self) -> KtResult<(Box<dyn Transport>, String)> {
        self.opens.fetch_add(1, Ordering::Relaxed);
        if !self.present.load(Ordering::Relaxed) {
            return Err(KtError::AntStickMissing);
        }
        self.unplugged.store(false, Ordering::Relaxed);
        Ok((
            Box::new(ScriptedStick {
                out: VecDeque::new(),
                open: false,
                beat: 0,
                event: 0xF000, // wraps within the first minute
                repeats: 0,
                unplugged: self.unplugged.clone(),
            }),
            "ANT USB-m Stick".into(),
        ))
    }
}

async fn until_snap(h: &EngineHandle, timeout: Duration, f: impl Fn(&Snapshot) -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f(&h.snapshot()) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ant_stick_feeds_heart_rate_and_hrv() {
    let present = Arc::new(AtomicBool::new(true));
    let unplugged = Arc::new(AtomicBool::new(false));
    let opener = Arc::new(Opener {
        present: present.clone(),
        unplugged: unplugged.clone(),
        opens: AtomicUsize::new(0),
    });
    let mut factory = DefaultFactory::new(
        Arc::new(MockBackend::new(vec![])),
        Arc::new(MemorySecrets::default()),
    );
    factory.ant = opener.clone();

    let mut cfg = Config::default();
    cfg.osc.targets.clear();
    cfg.osc.oscquery = false;
    cfg.device.demo_device = false;
    cfg.device.stale_timeout_secs = 2;
    cfg.source.active = SourceKind::Ant;
    let h = engine::start_with_factory(
        cfg,
        None,
        Arc::new(factory),
        EngineOptions {
            mdns: false,
            network_link_floor: Duration::ZERO,
        },
    );

    assert!(until_snap(&h, Duration::from_secs(5), |s| s.bpm == Some(72)).await);
    let s = h.snapshot();
    assert_eq!(s.live_source, Some(SourceKind::Ant));
    assert_eq!(s.device.as_ref().unwrap().name, "ANT+ HRM 12345");
    // HRV from ANT+ beat event times: alternating 800/860 ms beats give an RMSSD near 60 ms.
    assert!(
        until_snap(&h, Duration::from_secs(5), |s| s
            .hrv_ms
            .is_some_and(|v| (v - 60.0).abs() < 2.0))
        .await,
        "hrv was {:?}",
        h.snapshot().hrv_ms
    );

    // Unplug: the source reports the missing stick, keeps retrying, and recovers on replug.
    present.store(false, Ordering::Relaxed);
    unplugged.store(true, Ordering::Relaxed);
    assert!(
        until_snap(&h, Duration::from_secs(5), |s| s
            .error
            .as_ref()
            .is_some_and(|e| e.code == "ant_stick_missing"))
        .await
    );
    let before = opener.opens.load(Ordering::Relaxed);
    present.store(true, Ordering::Relaxed);
    assert!(
        until_snap(&h, Duration::from_secs(15), |s| s.bpm == Some(72)
            && s.error.is_none())
        .await
    );
    assert!(opener.opens.load(Ordering::Relaxed) > before);
}
