//! ANT+ heart rate: reads an ANT+ Heart Rate Monitor (chest strap, or a watch broadcasting HR)
//! through a USB ANT stick.
//!
//! The stick is driven on a dedicated thread: reset, set the ANT+ network key, open one
//! wildcard slave channel for device type 120 on RF 57 at period 8070, and decode broadcast
//! pages into [`HrSample`]s. RR intervals come from beat event times (see [`hrm`]), so HRV
//! works even for watches that send no RR over Bluetooth.

pub mod hrm;
pub mod protocol;
#[cfg(feature = "ant")]
pub mod usb;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use super::{HeartRateSource, HrSample, SourceKind};
use crate::error::{KtError, KtResult};
use crate::sensor::{DeviceInfo, SensorEvent, SensorSession};
use hrm::HrmDecoder;
use protocol::{Deframer, Message};

/// Raw byte pipe to an ANT stick.
pub trait Transport: Send {
    fn write(&mut self, frame: &[u8]) -> KtResult<()>;
    /// Bytes received within `timeout`; empty when nothing arrived.
    fn read(&mut self, timeout: Duration) -> KtResult<Vec<u8>>;
}

/// Finds and opens an ANT stick.
pub trait StickOpener: Send + Sync {
    /// Returns the open stick and its product name.
    fn open(&self) -> KtResult<(Box<dyn Transport>, String)>;
}

/// The channel KittyTooth uses on the stick.
const CHANNEL: u8 = 0;
const NETWORK: u8 = 0;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(200);
/// How long `connect` waits for a monitor before reporting it isn't broadcasting. Kept well
/// under the engine's connect-attempt timeout.
pub const DEFAULT_SEARCH: Duration = Duration::from_secs(20);

pub struct AntSource {
    opener: Arc<dyn StickOpener>,
    search: Duration,
}

impl AntSource {
    pub fn new(opener: Arc<dyn StickOpener>) -> Self {
        Self {
            opener,
            search: DEFAULT_SEARCH,
        }
    }

    pub fn with_search(mut self, search: Duration) -> Self {
        self.search = search;
        self
    }
}

/// The USB stick opener for this build, or one that always reports "no stick" when ANT
/// support is compiled out.
pub fn default_opener() -> Arc<dyn StickOpener> {
    #[cfg(feature = "ant")]
    {
        Arc::new(usb::UsbOpener)
    }
    #[cfg(not(feature = "ant"))]
    {
        struct Unsupported;
        impl StickOpener for Unsupported {
            fn open(&self) -> KtResult<(Box<dyn Transport>, String)> {
                Err(KtError::AntStickMissing)
            }
        }
        Arc::new(Unsupported)
    }
}

#[async_trait]
impl HeartRateSource for AntSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Ant
    }

    fn label(&self) -> String {
        "ANT+ heart rate".into()
    }

    async fn connect(&self, events: mpsc::Sender<SensorEvent>) -> KtResult<Box<dyn SensorSession>> {
        let (ready_tx, ready_rx) = oneshot::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let opener = self.opener.clone();
        let search = self.search;
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("kittytooth-ant".into())
            .spawn(move || run(opener, search, events, thread_stop, ready_tx))
            .map_err(|e| KtError::AntStick(e.to_string()))?;
        match ready_rx.await {
            Ok(Ok(info)) => Ok(Box::new(AntSession {
                info,
                stop,
                thread: Some(thread),
            })),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(KtError::AntStick("the ANT thread stopped".into())),
        }
    }
}

struct AntSession {
    info: DeviceInfo,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[async_trait]
impl SensorSession for AntSession {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    async fn disconnect(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || t.join()).await;
        }
    }
}

impl Drop for AntSession {
    fn drop(&mut self) {
        // The thread notices within one poll interval, closes the channel and releases the stick.
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Message-level access to the stick.
struct Stick {
    io: Box<dyn Transport>,
    deframer: Deframer,
    /// Messages that arrived while waiting for a command response.
    pending: VecDeque<Message>,
}

impl Stick {
    fn new(io: Box<dyn Transport>) -> Self {
        Self {
            io,
            deframer: Deframer::default(),
            pending: VecDeque::new(),
        }
    }

    /// The next message, waiting up to `timeout`.
    fn recv(&mut self, timeout: Duration) -> KtResult<Option<Message>> {
        if let Some(m) = self.pending.pop_front() {
            return Ok(Some(m));
        }
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(m) = self.deframer.next_message() {
                return Ok(Some(m));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let bytes = self.io.read(left.min(POLL))?;
            self.deframer.push(&bytes);
        }
    }

    /// Sends a configuration command and waits for its response.
    fn command(&mut self, frame: &[u8]) -> KtResult<()> {
        let id = frame[2];
        self.io.write(frame)?;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let mut held = Vec::new();
        let result = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.recv_fresh(left)? {
                Some(Message::Response { msg_id, code, .. }) if msg_id == id => {
                    break if code == protocol::RESPONSE_NO_ERROR {
                        Ok(())
                    } else {
                        Err(KtError::AntStick(format!(
                            "command 0x{id:02X} was refused (code 0x{code:02X})"
                        )))
                    };
                }
                Some(other) => held.push(other),
                None => break Err(KtError::AntStick(format!("no answer to command 0x{id:02X}"))),
            }
        };
        self.pending.extend(held);
        result
    }

    /// Like [`recv`](Self::recv) but skips the pending queue (used while collecting a response).
    fn recv_fresh(&mut self, timeout: Duration) -> KtResult<Option<Message>> {
        let pending = std::mem::take(&mut self.pending);
        let r = self.recv(timeout);
        self.pending = pending;
        r
    }

    fn reset(&mut self) -> KtResult<()> {
        self.io.write(&protocol::reset())?;
        // Sticks answer with a startup message within ~500 ms; some clones stay silent, so a
        // missing one isn't an error (the next command will tell).
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.recv(left)? {
                Some(Message::Startup(_)) | None => break,
                Some(_) => {}
            }
        }
        self.pending.clear();
        Ok(())
    }

    /// Resets the stick and opens the heart rate channel in search mode.
    fn open_hrm_channel(&mut self) -> KtResult<()> {
        use protocol::*;
        self.reset()?;
        self.command(&set_network_key(NETWORK, &ANT_PLUS_NETWORK_KEY))?;
        self.command(&assign_channel(CHANNEL, CHANNEL_TYPE_SLAVE, NETWORK))?;
        self.command(&set_channel_id(CHANNEL, 0, HRM_DEVICE_TYPE, 0))?;
        self.command(&set_channel_period(CHANNEL, HRM_PERIOD))?;
        self.command(&set_rf_frequency(CHANNEL, ANT_PLUS_RF))?;
        self.command(&set_search_timeout(CHANNEL, SEARCH_FOREVER))?;
        self.command(&open_channel(CHANNEL))
    }

    fn close(&mut self) {
        let _ = self.io.write(&protocol::close_channel(CHANNEL));
        let _ = self.io.write(&protocol::reset());
    }
}

/// Opens the stick, waits for a monitor, reports readiness, then streams readings until told
/// to stop or the stick goes away.
fn run(
    opener: Arc<dyn StickOpener>,
    search: Duration,
    events: mpsc::Sender<SensorEvent>,
    stop: Arc<AtomicBool>,
    ready: oneshot::Sender<KtResult<DeviceInfo>>,
) {
    let (mut stick, first_page, info) = match connect_blocking(&*opener, search, &stop) {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    tracing::info!(device = %info.name, "ANT+ heart rate monitor found");
    if ready.send(Ok(info)).is_err() {
        // The connect attempt was abandoned (timed out or the source changed).
        stick.close();
        return;
    }
    let reason = stream(&mut stick, first_page, &events, &stop);
    stick.close();
    if let Some(e) = reason {
        let _ = events.blocking_send(SensorEvent::Disconnected(Some(e)));
    }
}

fn connect_blocking(
    opener: &dyn StickOpener,
    search: Duration,
    stop: &AtomicBool,
) -> KtResult<(Stick, [u8; 8], DeviceInfo)> {
    let (io, stick_name) = opener.open()?;
    tracing::info!(stick = %stick_name, "ANT stick opened");
    let mut stick = Stick::new(io);
    stick.open_hrm_channel()?;

    let deadline = Instant::now() + search;
    let first = loop {
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            stick.close();
            return Err(KtError::DeviceNotFound("Your ANT+ heart rate monitor".into()));
        }
        match stick.recv(POLL)? {
            Some(Message::Data {
                channel: CHANNEL,
                page,
            }) => break page,
            Some(Message::Event {
                channel: CHANNEL,
                code: protocol::EVENT_CHANNEL_CLOSED,
            }) => stick.io.write(&protocol::open_channel(CHANNEL))?,
            _ => {}
        }
    };

    // Ask which monitor we paired with, for the device label.
    stick.io.write(&protocol::request_channel_id(CHANNEL))?;
    let mut device_number = None;
    let mut held = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        match stick.recv_fresh(POLL)? {
            Some(Message::ChannelId { device_number: n, .. }) => {
                device_number = Some(n);
                break;
            }
            Some(other) => held.push(other),
            None => {}
        }
    }
    stick.pending.extend(held);

    let info = match device_number {
        Some(n) => DeviceInfo {
            id: format!("ant:{n}"),
            name: format!("ANT+ HRM {n}"),
            location: None,
            demo: false,
        },
        None => DeviceInfo {
            id: "ant:hrm".into(),
            name: "ANT+ heart rate monitor".into(),
            location: None,
            demo: false,
        },
    };
    Ok((stick, first, info))
}

/// Streams readings; returns why the link ended (`None` when asked to stop or nobody listens).
fn stream(
    stick: &mut Stick,
    first_page: [u8; 8],
    events: &mpsc::Sender<SensorEvent>,
    stop: &AtomicBool,
) -> Option<KtError> {
    let mut decoder = HrmDecoder::default();
    let mut battery = None;
    let mut page = Some(first_page);
    loop {
        if let Some(raw) = page.take() {
            for ev in readings(&mut decoder, &mut battery, &raw) {
                if events.blocking_send(ev).is_err() {
                    return None;
                }
            }
        }
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        match stick.recv(POLL) {
            Ok(Some(Message::Data {
                channel: CHANNEL,
                page: p,
            })) => page = Some(p),
            Ok(Some(Message::Event {
                channel: CHANNEL,
                code: protocol::EVENT_CHANNEL_CLOSED,
            })) => {
                if let Err(e) = stick.io.write(&protocol::open_channel(CHANNEL)) {
                    return Some(e);
                }
            }
            Ok(_) => {}
            Err(e) => return Some(e),
        }
        if events.is_closed() {
            return None;
        }
    }
}

/// Engine events for one data page.
fn readings(decoder: &mut HrmDecoder, battery: &mut Option<u8>, raw: &[u8; 8]) -> Vec<SensorEvent> {
    let d = decoder.push(raw);
    let mut out = Vec::new();
    if d.battery.is_some() && d.battery != *battery {
        *battery = d.battery;
        out.extend(d.battery.map(SensorEvent::Battery));
    }
    if d.emit {
        out.push(match d.bpm {
            Some(bpm) => {
                let mut s = HrSample::new(bpm, SourceKind::Ant);
                s.rr_ms = d.rr_ms;
                s.battery = *battery;
                SensorEvent::Sample(s)
            }
            None => SensorEvent::Unreadable("the monitor has no heart rate yet".into()),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::*;
    use std::sync::Mutex;

    type Written = Arc<Mutex<Vec<Vec<u8>>>>;

    /// A scripted ANT stick: answers every command, then plays data pages after the channel
    /// opens, then optionally "unplugs".
    struct FakeStick {
        out: VecDeque<u8>,
        pages: VecDeque<[u8; 8]>,
        opened: bool,
        unplug_after_pages: bool,
        written: Written,
    }

    impl Transport for FakeStick {
        fn write(&mut self, frame: &[u8]) -> KtResult<()> {
            self.written.lock().unwrap().push(frame.to_vec());
            let id = frame[2];
            let reply = match id {
                MSG_RESET => encode(MSG_STARTUP, &[0x20]),
                MSG_REQUEST => encode(MSG_CHANNEL_ID, &[CHANNEL, 0x39, 0x30, HRM_DEVICE_TYPE, 0x01]),
                MSG_CLOSE_CHANNEL => encode(MSG_CHANNEL_EVENT, &[CHANNEL, id, 0]),
                _ => {
                    if id == MSG_OPEN_CHANNEL {
                        self.opened = true;
                    }
                    encode(MSG_CHANNEL_EVENT, &[CHANNEL, id, RESPONSE_NO_ERROR])
                }
            };
            self.out.extend(reply);
            Ok(())
        }

        fn read(&mut self, _timeout: Duration) -> KtResult<Vec<u8>> {
            if !self.out.is_empty() {
                // Hand bytes out in small, uneven chunks like USB transfers do.
                let n = self.out.len().min(7);
                return Ok(self.out.drain(..n).collect());
            }
            if self.opened {
                if let Some(p) = self.pages.pop_front() {
                    let mut data = vec![CHANNEL];
                    data.extend_from_slice(&p);
                    return Ok(encode(MSG_BROADCAST_DATA, &data));
                }
                if self.unplug_after_pages {
                    return Err(KtError::AntStickMissing);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
            Ok(vec![])
        }
    }

    struct FakeOpener {
        pages: Vec<[u8; 8]>,
        unplug: bool,
        result: Option<KtError>,
        written: Written,
    }

    impl StickOpener for FakeOpener {
        fn open(&self) -> KtResult<(Box<dyn Transport>, String)> {
            if let Some(e) = &self.result {
                return Err(e.clone());
            }
            Ok((
                Box::new(FakeStick {
                    out: VecDeque::new(),
                    pages: self.pages.iter().copied().collect(),
                    opened: false,
                    unplug_after_pages: self.unplug,
                    written: self.written.clone(),
                }),
                "ANT USBStick2".into(),
            ))
        }
    }

    fn opener(pages: Vec<[u8; 8]>, unplug: bool) -> (Arc<FakeOpener>, Written) {
        let written = Arc::new(Mutex::new(Vec::new()));
        (
            Arc::new(FakeOpener {
                pages,
                unplug,
                result: None,
                written: written.clone(),
            }),
            written,
        )
    }

    fn hr_page(event: u16, count: u8, bpm: u8) -> [u8; 8] {
        let [e0, e1] = event.to_le_bytes();
        [0x00, 0xFF, 0xFF, 0xFF, e0, e1, count, bpm]
    }

    #[tokio::test]
    async fn streams_samples_with_rr_then_reports_unplug() {
        let pages = vec![
            hr_page(1000, 1, 64),
            hr_page(1000, 1, 64),
            hr_page(1960, 2, 64),
            hr_page(2900, 3, 65),
        ];
        let (op, written) = opener(pages, true);
        let src = AntSource::new(op).with_search(Duration::from_secs(2));
        assert_eq!(src.kind(), SourceKind::Ant);
        let (tx, mut rx) = mpsc::channel(16);
        let mut session = src.connect(tx).await.unwrap();
        assert_eq!(session.info().name, "ANT+ HRM 12345");
        assert_eq!(session.info().id, "ant:12345");

        let mut samples = Vec::new();
        let reason = loop {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
            {
                Some(SensorEvent::Sample(s)) => samples.push(s),
                Some(SensorEvent::Disconnected(r)) => break r,
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(reason, Some(KtError::AntStickMissing));
        assert_eq!(samples.len(), 3);
        assert!(samples.iter().all(|s| s.source == SourceKind::Ant));
        assert_eq!(samples[0].rr_ms, Vec::<f64>::new());
        assert_eq!(samples[1].rr_ms, vec![960.0 * 1000.0 / 1024.0]);
        assert_eq!(samples[2].bpm, 65);
        assert_eq!(samples[2].rr_ms, vec![940.0 * 1000.0 / 1024.0]);
        session.disconnect().await;

        // The channel was configured as an ANT+ HRM wildcard search.
        let w = written.lock().unwrap();
        assert!(w.contains(&set_network_key(NETWORK, &ANT_PLUS_NETWORK_KEY)));
        assert!(w.contains(&set_channel_id(CHANNEL, 0, HRM_DEVICE_TYPE, 0)));
        assert!(w.contains(&set_channel_period(CHANNEL, HRM_PERIOD)));
        assert!(w.contains(&set_rf_frequency(CHANNEL, ANT_PLUS_RF)));
    }

    #[tokio::test]
    async fn no_monitor_is_reported_after_the_search_window() {
        let (op, _) = opener(vec![], false);
        let src = AntSource::new(op).with_search(Duration::from_millis(300));
        let (tx, _rx) = mpsc::channel(16);
        let err = src.connect(tx).await.err().unwrap();
        assert!(matches!(err, KtError::DeviceNotFound(_)));
    }

    #[tokio::test]
    async fn stick_errors_pass_through() {
        for e in [
            KtError::AntStickMissing,
            KtError::AntStickBusy,
            KtError::AntDriver("libusb0".into()),
        ] {
            let op = Arc::new(FakeOpener {
                pages: vec![],
                unplug: false,
                result: Some(e.clone()),
                written: Arc::default(),
            });
            let (tx, _rx) = mpsc::channel(16);
            assert_eq!(AntSource::new(op).connect(tx).await.err(), Some(e));
        }
    }

    #[tokio::test]
    async fn disconnect_stops_the_thread_and_closes_the_channel() {
        let (op, written) = opener(vec![hr_page(1000, 1, 70)], false);
        let (tx, _rx) = mpsc::channel(16);
        let mut s = AntSource::new(op).connect(tx).await.unwrap();
        s.disconnect().await;
        assert!(written.lock().unwrap().contains(&close_channel(CHANNEL)));
    }

    #[test]
    fn battery_is_sent_once_and_zero_bpm_is_unreadable() {
        let mut d = HrmDecoder::default();
        let mut b = None;
        readings(&mut d, &mut b, &[0x00, 0xFF, 0xFF, 0xFF, 0, 4, 1, 0]);
        let ev = readings(&mut d, &mut b, &[0x87, 90, 0, 0, 0, 4, 2, 70]);
        assert_eq!(ev[0], SensorEvent::Battery(90));
        assert!(matches!(&ev[1], SensorEvent::Sample(s) if s.battery == Some(90) && s.bpm == 70));
        let ev = readings(&mut d, &mut b, &[0x87, 90, 0, 0, 0, 8, 3, 0]);
        assert_eq!(ev.len(), 1);
        assert!(matches!(ev[0], SensorEvent::Unreadable(_)));
    }
}
