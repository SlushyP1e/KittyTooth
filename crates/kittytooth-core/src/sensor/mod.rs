//! Bluetooth sensor abstraction. [`HeartRateBackend`] scans and connects BLE sensors; the
//! btleplug backend (feature `ble`), the mock/demo backend and a composite of both implement it.
//! The engine reaches them through [`crate::source::BleSource`].

#[cfg(feature = "ble")]
pub mod ble;
pub mod mock;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::{KtError, KtResult};
use crate::hrm::BodySensorLocation;

/// Heart Rate Service.
pub const HEART_RATE_SERVICE: uuid::Uuid = uuid::Uuid::from_u128(0x0000180d_0000_1000_8000_00805f9b34fb);
/// Heart Rate Measurement characteristic.
pub const HEART_RATE_MEASUREMENT: uuid::Uuid = uuid::Uuid::from_u128(0x00002a37_0000_1000_8000_00805f9b34fb);
/// Body Sensor Location characteristic.
pub const BODY_SENSOR_LOCATION: uuid::Uuid = uuid::Uuid::from_u128(0x00002a38_0000_1000_8000_00805f9b34fb);
/// Battery Service.
pub const BATTERY_SERVICE: uuid::Uuid = uuid::Uuid::from_u128(0x0000180f_0000_1000_8000_00805f9b34fb);
/// Battery Level characteristic.
pub const BATTERY_LEVEL: uuid::Uuid = uuid::Uuid::from_u128(0x00002a19_0000_1000_8000_00805f9b34fb);

/// Prefix of demo device ids.
pub const DEMO_PREFIX: &str = "demo:";

/// A device seen while scanning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub rssi: Option<i16>,
    /// Advertises the Heart Rate Service.
    pub has_hr_service: bool,
    /// Unix epoch ms when last seen.
    pub last_seen_ms: u64,
    pub demo: bool,
}

/// The connected device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub location: Option<BodySensorLocation>,
    pub demo: bool,
}

/// Bluetooth adapter availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdapterState {
    Ready,
    Off,
    Missing,
    #[default]
    Unknown,
}

impl AdapterState {
    pub fn error(self) -> Option<KtError> {
        match self {
            AdapterState::Off => Some(KtError::RadioOff),
            AdapterState::Missing => Some(KtError::AdapterMissing),
            _ => None,
        }
    }
}

/// Events a connected source sends to the engine. Every source emits the same
/// [`HrSample`](crate::source::HrSample).
#[derive(Debug, Clone, PartialEq)]
pub enum SensorEvent {
    Sample(crate::source::HrSample),
    /// A packet arrived but couldn't be decoded (keeps the link alive, carries no reading).
    Unreadable(String),
    Battery(u8),
    /// The link dropped. `None` means a clean disconnect.
    Disconnected(Option<KtError>),
}

/// An open sensor connection.
#[async_trait]
pub trait SensorSession: Send + Sync {
    fn info(&self) -> DeviceInfo;
    async fn disconnect(&mut self);
}

/// Something that can find and connect heart rate sensors.
#[async_trait]
pub trait HeartRateBackend: Send + Sync {
    async fn adapter_state(&self) -> AdapterState;
    /// Starts (or keeps) scanning. Discovered devices are sent repeatedly with fresh RSSI.
    async fn start_scan(&self, show_all: bool, found: mpsc::Sender<DiscoveredDevice>) -> KtResult<()>;
    async fn stop_scan(&self) -> KtResult<()>;
    /// Connects, subscribes to notifications and reads battery/location. Events flow into
    /// `events` until the session is disconnected or dropped.
    async fn connect(
        &self,
        id: &str,
        name_hint: &str,
        events: mpsc::Sender<SensorEvent>,
    ) -> KtResult<Box<dyn SensorSession>>;
    /// Enables/disables the demo device (composite backend only).
    fn set_demo_enabled(&self, _enabled: bool) {}
}

/// Routes `demo:` ids to the mock backend and everything else to the real one.
pub struct CompositeBackend {
    real: Option<Arc<dyn HeartRateBackend>>,
    demo: Arc<mock::MockBackend>,
    demo_enabled: AtomicBool,
}

impl CompositeBackend {
    pub fn new(
        real: Option<Arc<dyn HeartRateBackend>>,
        demo: Arc<mock::MockBackend>,
        demo_enabled: bool,
    ) -> Self {
        Self {
            real,
            demo,
            demo_enabled: AtomicBool::new(demo_enabled),
        }
    }
}

#[async_trait]
impl HeartRateBackend for CompositeBackend {
    async fn adapter_state(&self) -> AdapterState {
        match &self.real {
            Some(r) => r.adapter_state().await,
            None => AdapterState::Missing,
        }
    }

    async fn start_scan(&self, show_all: bool, found: mpsc::Sender<DiscoveredDevice>) -> KtResult<()> {
        if self.demo_enabled.load(Ordering::Relaxed) {
            self.demo.start_scan(show_all, found.clone()).await?;
        }
        match &self.real {
            Some(r) => r.start_scan(show_all, found).await,
            None => Err(KtError::AdapterMissing),
        }
    }

    async fn stop_scan(&self) -> KtResult<()> {
        let _ = self.demo.stop_scan().await;
        match &self.real {
            Some(r) => r.stop_scan().await,
            None => Ok(()),
        }
    }

    async fn connect(
        &self,
        id: &str,
        name_hint: &str,
        events: mpsc::Sender<SensorEvent>,
    ) -> KtResult<Box<dyn SensorSession>> {
        if id.starts_with(DEMO_PREFIX) {
            return self.demo.connect(id, name_hint, events).await;
        }
        match &self.real {
            Some(r) => r.connect(id, name_hint, events).await,
            None => Err(KtError::AdapterMissing),
        }
    }

    fn set_demo_enabled(&self, enabled: bool) {
        self.demo_enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            let demo = self.demo.clone();
            tokio::spawn(async move {
                let _ = demo.stop_scan().await;
            });
        }
    }
}
