//! Heart rate sources. Every source (Bluetooth LE, ANT+, Pulsoid, HypeRate, OSC input, demo) implements
//! [`HeartRateSource`] and emits the same [`HrSample`] through [`SensorEvent`], so the engine's
//! processing and OSC output never care where a reading came from.

pub mod ant;
pub mod hyperate;
pub mod osc_input;
pub mod pulsoid;
mod ws;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::config::{Config, RememberedDevice};
use crate::error::{KtError, KtResult};
use crate::hrm::{self, SensorContact};
use crate::osc::sender::epoch_ms;
use crate::sensor::{HeartRateBackend, SensorEvent, SensorSession, DEMO_PREFIX};

/// Which kind of source produced a reading. Serialized as the short tag the UI shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    #[default]
    Ble,
    Pulsoid,
    Hyperate,
    Osc,
    Demo,
    /// ANT+ heart rate monitor through a USB ANT stick.
    Ant,
}

impl SourceKind {
    pub const ALL: [SourceKind; 6] = [
        SourceKind::Ble,
        SourceKind::Ant,
        SourceKind::Pulsoid,
        SourceKind::Hyperate,
        SourceKind::Osc,
        SourceKind::Demo,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            SourceKind::Ble => "ble",
            SourceKind::Pulsoid => "pulsoid",
            SourceKind::Hyperate => "hyperate",
            SourceKind::Osc => "osc",
            SourceKind::Demo => "demo",
            SourceKind::Ant => "ant",
        }
    }

    /// The kind a remembered Bluetooth device belongs to (demo ids are `demo:*`).
    pub fn of_device(id: &str) -> SourceKind {
        if id.starts_with(DEMO_PREFIX) {
            SourceKind::Demo
        } else {
            SourceKind::Ble
        }
    }

    /// What this kind of source can report at most.
    pub fn capabilities(self) -> Capabilities {
        match self {
            SourceKind::Ble | SourceKind::Demo => Capabilities {
                rr: true,
                battery: true,
                contact: true,
            },
            // ANT+ HRM pages carry beat event times (RR intervals) and, on most straps, battery,
            // but no skin-contact flag.
            SourceKind::Ant => Capabilities {
                rr: true,
                battery: true,
                contact: false,
            },
            // Pulsoid's v2 real-time API streams heart rate and the monitor's battery level.
            SourceKind::Pulsoid => Capabilities {
                rr: false,
                battery: true,
                contact: false,
            },
            SourceKind::Hyperate | SourceKind::Osc => Capabilities::default(),
        }
    }

    /// Whether a silent link should be torn down and reconnected. A UDP listener has no link.
    pub fn reconnects_on_silence(self) -> bool {
        !matches!(self, SourceKind::Osc)
    }

    pub fn is_network(self) -> bool {
        matches!(self, SourceKind::Pulsoid | SourceKind::Hyperate)
    }
}

/// Optional data a source can provide. Missing data is shown as "not available from this
/// source", never as 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capabilities {
    /// RR intervals (needed for HRV and exact beat timing).
    pub rr: bool,
    pub battery: bool,
    pub contact: bool,
}

/// One heart rate reading, whatever the source.
#[derive(Debug, Clone, PartialEq)]
pub struct HrSample {
    pub bpm: u16,
    /// RR intervals in milliseconds, oldest first.
    pub rr_ms: Vec<f64>,
    pub battery: Option<u8>,
    /// `None` when the source can't tell.
    pub contact: Option<bool>,
    pub energy_kj: Option<u16>,
    /// Unix epoch ms when the reading was taken (source time if it reports one).
    pub timestamp_ms: u64,
    /// When the reading reached KittyTooth (for latency measurement).
    pub received: Instant,
    pub source: SourceKind,
}

impl HrSample {
    pub fn new(bpm: u16, source: SourceKind) -> Self {
        Self {
            bpm,
            rr_ms: Vec::new(),
            battery: None,
            contact: None,
            energy_kj: None,
            timestamp_ms: epoch_ms(),
            received: Instant::now(),
            source,
        }
    }

    /// Decodes a Bluetooth Heart Rate Measurement (`0x2A37`) payload.
    pub fn from_hrm(data: &[u8], received: Instant, source: SourceKind) -> KtResult<Self> {
        let m = hrm::parse(data)?;
        Ok(Self {
            bpm: m.bpm,
            rr_ms: m.rr_ms().collect(),
            battery: None,
            contact: match m.contact {
                SensorContact::NotSupported => None,
                SensorContact::NotDetected => Some(false),
                SensorContact::Detected => Some(true),
            },
            energy_kj: m.energy_expended_kj,
            timestamp_ms: epoch_ms(),
            received,
            source,
        })
    }

    pub fn sensor_contact(&self) -> SensorContact {
        match self.contact {
            None => SensorContact::NotSupported,
            Some(false) => SensorContact::NotDetected,
            Some(true) => SensorContact::Detected,
        }
    }

    pub fn has_contact(&self) -> bool {
        self.contact != Some(false)
    }
}

/// A heart rate source. `connect` opens a session whose readings flow into `events` as
/// [`SensorEvent::Sample`] until the session is disconnected or dropped.
#[async_trait]
pub trait HeartRateSource: Send + Sync {
    fn kind(&self) -> SourceKind;
    fn capabilities(&self) -> Capabilities {
        self.kind().capabilities()
    }
    /// Short human label for the UI ("Pulsoid", "HypeRate 1A2B3C", ...).
    fn label(&self) -> String;
    async fn connect(&self, events: mpsc::Sender<SensorEvent>) -> KtResult<Box<dyn SensorSession>>;
}

/// Bluetooth LE (or the demo strap, which goes through the same backend).
pub struct BleSource {
    backend: Arc<dyn HeartRateBackend>,
    device: RememberedDevice,
}

impl BleSource {
    pub fn new(backend: Arc<dyn HeartRateBackend>, device: RememberedDevice) -> Self {
        Self { backend, device }
    }
}

#[async_trait]
impl HeartRateSource for BleSource {
    fn kind(&self) -> SourceKind {
        SourceKind::of_device(&self.device.id)
    }

    fn label(&self) -> String {
        self.device.name.clone()
    }

    async fn connect(&self, events: mpsc::Sender<SensorEvent>) -> KtResult<Box<dyn SensorSession>> {
        self.backend
            .connect(&self.device.id, &self.device.name, events)
            .await
    }
}

// ---------- secrets ----------

/// Secrets kept out of the JSON config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    PulsoidToken,
    /// User override of the HypeRate developer API key.
    HyperateKey,
}

impl SecretKind {
    pub const ALL: [SecretKind; 2] = [SecretKind::PulsoidToken, SecretKind::HyperateKey];

    /// Account name in the OS credential store.
    pub fn account(self) -> &'static str {
        match self {
            SecretKind::PulsoidToken => "pulsoid-token",
            SecretKind::HyperateKey => "hyperate-api-key",
        }
    }
}

/// Storage for tokens and keys (the OS credential store in the app, memory in tests).
pub trait SecretStore: Send + Sync {
    fn get(&self, kind: SecretKind) -> Option<String>;
    fn set(&self, kind: SecretKind, value: &str) -> KtResult<()>;
    fn delete(&self, kind: SecretKind) -> KtResult<()>;
}

#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<SecretKind, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, kind: SecretKind) -> Option<String> {
        self.0.lock().ok()?.get(&kind).cloned()
    }
    fn set(&self, kind: SecretKind, value: &str) -> KtResult<()> {
        if let Ok(mut m) = self.0.lock() {
            m.insert(kind, value.to_string());
        }
        Ok(())
    }
    fn delete(&self, kind: SecretKind) -> KtResult<()> {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&kind);
        }
        Ok(())
    }
}

/// Masks a secret for display and logs: only the last four characters stay visible.
pub fn mask(secret: &str) -> String {
    let n = secret.chars().count();
    if n <= 4 {
        return "••••".into();
    }
    let tail: String = secret.chars().skip(n - 4).collect();
    format!("••••{tail}")
}

/// HypeRate developer API key baked in at build time (`KITTYTOOTH_HYPERATE_KEY`). Never committed.
pub fn builtin_hyperate_key() -> Option<&'static str> {
    option_env!("KITTYTOOTH_HYPERATE_KEY").filter(|k| !k.trim().is_empty())
}

/// What the UI shows about a stored secret (never the value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretStatus {
    pub kind: SecretKind,
    pub present: bool,
    pub masked: Option<String>,
}

pub fn secret_status(store: &dyn SecretStore, kind: SecretKind) -> SecretStatus {
    let v = store.get(kind).filter(|s| !s.is_empty());
    SecretStatus {
        kind,
        present: v.is_some(),
        masked: v.as_deref().map(mask),
    }
}

// ---------- factory ----------

/// Network endpoints (overridable so tests can point at a local mock server).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub pulsoid_ws: String,
    pub hyperate_ws: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            pulsoid_ws: pulsoid::REAL_TIME_V2.into(),
            hyperate_ws: hyperate::WS_BASE.into(),
        }
    }
}

/// Builds the source for a kind from the current config and secrets.
pub trait SourceFactory: Send + Sync {
    fn build(&self, kind: SourceKind, config: &Config) -> KtResult<Arc<dyn HeartRateSource>>;
    /// The Bluetooth backend (scanning lives outside the source abstraction).
    fn backend(&self) -> Arc<dyn HeartRateBackend>;
}

pub struct DefaultFactory {
    pub backend: Arc<dyn HeartRateBackend>,
    pub secrets: Arc<dyn SecretStore>,
    pub endpoints: Endpoints,
    /// Opens the ANT stick (the USB opener, or a fake in tests).
    pub ant: Arc<dyn ant::StickOpener>,
}

impl DefaultFactory {
    pub fn new(backend: Arc<dyn HeartRateBackend>, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            backend,
            secrets,
            endpoints: Endpoints::default(),
            ant: ant::default_opener(),
        }
    }
}

/// The demo strap used when the demo source is chosen without a remembered demo device.
pub const DEMO_STRAP: &str = "demo:strap";

impl SourceFactory for DefaultFactory {
    fn build(&self, kind: SourceKind, config: &Config) -> KtResult<Arc<dyn HeartRateSource>> {
        match kind {
            SourceKind::Ble => match &config.device.last_device {
                Some(d) if SourceKind::of_device(&d.id) == SourceKind::Ble => {
                    Ok(Arc::new(BleSource::new(self.backend.clone(), d.clone())))
                }
                _ => Err(KtError::NoSensorChosen),
            },
            SourceKind::Demo => {
                let d = config
                    .device
                    .last_device
                    .clone()
                    .filter(|d| SourceKind::of_device(&d.id) == SourceKind::Demo)
                    .unwrap_or_else(|| RememberedDevice {
                        id: DEMO_STRAP.into(),
                        name: "KittyTooth Demo Strap".into(),
                    });
                Ok(Arc::new(BleSource::new(self.backend.clone(), d)))
            }
            SourceKind::Pulsoid => {
                let token = self
                    .secrets
                    .get(SecretKind::PulsoidToken)
                    .filter(|t| !t.trim().is_empty())
                    .ok_or(KtError::MissingCredential("pulsoid_token"))?;
                Ok(Arc::new(pulsoid::PulsoidSource::new(
                    self.endpoints.pulsoid_ws.clone(),
                    token.trim().to_string(),
                )))
            }
            SourceKind::Hyperate => {
                let id = hyperate::validate_device_id(&config.source.hyperate.device_id)?;
                let key = self
                    .secrets
                    .get(SecretKind::HyperateKey)
                    .filter(|k| !k.trim().is_empty())
                    .or_else(|| builtin_hyperate_key().map(str::to_string))
                    .ok_or(KtError::MissingCredential("hyperate_key"))?;
                Ok(Arc::new(hyperate::HypeRateSource::new(
                    self.endpoints.hyperate_ws.clone(),
                    key.trim().to_string(),
                    id,
                )))
            }
            SourceKind::Osc => Ok(Arc::new(osc_input::OscInputSource::new(
                config.source.osc_input.clone(),
            ))),
            SourceKind::Ant => Ok(Arc::new(ant::AntSource::new(self.ant.clone()))),
        }
    }

    fn backend(&self) -> Arc<dyn HeartRateBackend> {
        self.backend.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hrm_to_sample() {
        // flags: contact detected + RR present; 72 bpm; one RR of 1024/1024 s.
        let s = HrSample::from_hrm(&[0b0001_0110, 72, 0x00, 0x04], Instant::now(), SourceKind::Ble).unwrap();
        assert_eq!(s.bpm, 72);
        assert_eq!(s.contact, Some(true));
        assert_eq!(s.rr_ms, vec![1000.0]);
        assert_eq!(s.source, SourceKind::Ble);
        let s = HrSample::from_hrm(&[0, 60], Instant::now(), SourceKind::Demo).unwrap();
        assert_eq!(s.contact, None);
        assert!(s.has_contact());
        assert!(HrSample::from_hrm(&[0x01, 60], Instant::now(), SourceKind::Ble).is_err());
    }

    #[test]
    fn masking() {
        assert_eq!(mask("8c4da3ce-7ed7-4a19-a1f1-058498661e45"), "••••1e45");
        assert_eq!(mask("abc"), "••••");
        let m = MemorySecrets::default();
        assert!(!secret_status(&m, SecretKind::PulsoidToken).present);
        m.set(SecretKind::PulsoidToken, "secret-token-1234").unwrap();
        let st = secret_status(&m, SecretKind::PulsoidToken);
        assert!(st.present);
        assert_eq!(st.masked.as_deref(), Some("••••1234"));
        assert!(!format!("{st:?}").contains("secret-token"));
    }

    #[test]
    fn capabilities_per_kind() {
        assert!(SourceKind::Ble.capabilities().rr);
        assert!(SourceKind::Pulsoid.capabilities().battery);
        assert!(!SourceKind::Pulsoid.capabilities().rr);
        assert_eq!(SourceKind::Hyperate.capabilities(), Capabilities::default());
        assert!(!SourceKind::Osc.reconnects_on_silence());
        assert!(SourceKind::Ant.capabilities().rr);
        assert!(!SourceKind::Ant.capabilities().contact);
        assert!(SourceKind::Ant.reconnects_on_silence());
        assert!(!SourceKind::Ant.is_network());
        assert_eq!(serde_json::to_string(&SourceKind::Ant).unwrap(), "\"ant\"");
        assert_eq!(SourceKind::Ant.tag(), "ant");
        assert_eq!(SourceKind::of_device("demo:strap"), SourceKind::Demo);
        assert_eq!(
            serde_json::to_string(&SourceKind::Hyperate).unwrap(),
            "\"hyperate\""
        );
    }

    #[test]
    fn factory_requirements() {
        let backend: Arc<dyn HeartRateBackend> = Arc::new(crate::sensor::mock::MockBackend::new(vec![]));
        let secrets = Arc::new(MemorySecrets::default());
        let f = DefaultFactory::new(backend, secrets.clone());
        let mut c = Config::default();
        assert_eq!(f.build(SourceKind::Ble, &c).err(), Some(KtError::NoSensorChosen));
        assert_eq!(f.build(SourceKind::Demo, &c).unwrap().kind(), SourceKind::Demo);
        assert_eq!(
            f.build(SourceKind::Pulsoid, &c).err(),
            Some(KtError::MissingCredential("pulsoid_token"))
        );
        secrets.set(SecretKind::PulsoidToken, "tok").unwrap();
        assert_eq!(
            f.build(SourceKind::Pulsoid, &c).unwrap().kind(),
            SourceKind::Pulsoid
        );
        assert!(matches!(
            f.build(SourceKind::Hyperate, &c),
            Err(KtError::InvalidDeviceId(_))
        ));
        c.source.hyperate.device_id = "1A2B3C".into();
        if builtin_hyperate_key().is_none() {
            assert_eq!(
                f.build(SourceKind::Hyperate, &c).err(),
                Some(KtError::MissingCredential("hyperate_key"))
            );
        }
        secrets.set(SecretKind::HyperateKey, "key").unwrap();
        assert_eq!(
            f.build(SourceKind::Hyperate, &c).unwrap().label(),
            "HypeRate 1A2B3C"
        );
        assert_eq!(f.build(SourceKind::Osc, &c).unwrap().kind(), SourceKind::Osc);
        assert_eq!(f.build(SourceKind::Ant, &c).unwrap().kind(), SourceKind::Ant);
    }
}
