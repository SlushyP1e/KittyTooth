//! Real Bluetooth LE backend built on btleplug (WinRT on Windows, BlueZ on Linux,
//! CoreBluetooth on macOS).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use btleplug::api::{
    Central, CentralEvent, CentralState, CharPropFlags, Manager as _, Peripheral as _, ScanFilter,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::error::{KtError, KtResult};
use crate::hrm::{parse_battery, BodySensorLocation};
use crate::osc::sender::epoch_ms;
use crate::sensor::{
    AdapterState, DeviceInfo, DiscoveredDevice, HeartRateBackend, SensorEvent, SensorSession, BATTERY_LEVEL,
    BODY_SENSOR_LOCATION, HEART_RATE_MEASUREMENT, HEART_RATE_SERVICE,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DISCOVER_TIMEOUT: Duration = Duration::from_secs(10);
const FIND_TIMEOUT: Duration = Duration::from_secs(12);

/// Maps btleplug errors (including WinRT HRESULT strings) to human-friendly errors.
pub fn map_error(e: &btleplug::Error, name: &str) -> KtError {
    use btleplug::Error as E;
    match e {
        E::PermissionDenied => KtError::PermissionDenied,
        E::DeviceNotFound => KtError::DeviceNotFound(name.to_string()),
        E::TimedOut(_) => KtError::ConnectTimeout(name.to_string()),
        E::NoAdapterAvailable => KtError::AdapterMissing,
        E::NotConnected => KtError::Ble("the sensor is not connected".into()),
        other => classify_message(&other.to_string(), name),
    }
}

/// Classifies free-form platform error text. Exposed for tests.
pub fn classify_message(msg: &str, name: &str) -> KtError {
    let m = msg.to_ascii_lowercase();
    if (m.contains("radio") && m.contains("off")) || m.contains("powered off") || m.contains("poweredoff") {
        KtError::RadioOff
    } else if m.contains("no adapter") || m.contains("adapter not found") || m.contains("0x8007048f") {
        KtError::AdapterMissing
    } else if m.contains("access is denied")
        || m.contains("accessdenied")
        || m.contains("0x80070005")
        || m.contains("in use")
        || m.contains("busy")
    {
        KtError::DeviceBusy(name.to_string())
    } else if m.contains("unreachable")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("0x80070079")
    {
        KtError::ConnectTimeout(name.to_string())
    } else if m.contains("permission") {
        KtError::PermissionDenied
    } else {
        KtError::Ble(msg.to_string())
    }
}

pub struct BleBackend {
    manager: Manager,
    adapter: Mutex<Option<Adapter>>,
    scan_task: Mutex<Option<JoinHandle<()>>>,
    user_scanning: AtomicBool,
}

impl BleBackend {
    pub async fn new() -> KtResult<Self> {
        let manager = Manager::new().await.map_err(|e| map_error(&e, ""))?;
        Ok(Self {
            manager,
            adapter: Mutex::new(None),
            scan_task: Mutex::new(None),
            user_scanning: AtomicBool::new(false),
        })
    }

    /// The first adapter, re-queried until one exists (so a dongle plugged in later works).
    async fn adapter(&self) -> KtResult<Adapter> {
        let mut guard = self.adapter.lock().await;
        if let Some(a) = guard.as_ref() {
            return Ok(a.clone());
        }
        let adapters = self.manager.adapters().await.map_err(|e| map_error(&e, ""))?;
        let a = adapters.into_iter().next().ok_or(KtError::AdapterMissing)?;
        *guard = Some(a.clone());
        Ok(a)
    }

    async fn ready_adapter(&self) -> KtResult<Adapter> {
        let a = self.adapter().await?;
        match a.adapter_state().await {
            Ok(CentralState::PoweredOff) => Err(KtError::RadioOff),
            _ => Ok(a),
        }
    }

    async fn find_peripheral(&self, adapter: &Adapter, id: &str) -> KtResult<Option<Peripheral>> {
        let list = adapter.peripherals().await.map_err(|e| map_error(&e, id))?;
        Ok(list.into_iter().find(|p| p.id().to_string() == id))
    }
}

async fn describe(p: &Peripheral) -> Option<DiscoveredDevice> {
    let props = p.properties().await.ok().flatten()?;
    let name = props
        .local_name
        .clone()
        .or(props.advertisement_name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Unknown device ({})", props.address));
    Some(DiscoveredDevice {
        id: p.id().to_string(),
        name,
        rssi: props.rssi,
        has_hr_service: props.services.contains(&HEART_RATE_SERVICE)
            || props.service_data.contains_key(&HEART_RATE_SERVICE),
        last_seen_ms: epoch_ms(),
        demo: false,
    })
}

#[async_trait]
impl HeartRateBackend for BleBackend {
    async fn adapter_state(&self) -> AdapterState {
        match self.adapter().await {
            Err(KtError::AdapterMissing) => AdapterState::Missing,
            Err(_) => AdapterState::Unknown,
            Ok(a) => match a.adapter_state().await {
                Ok(CentralState::PoweredOn) => AdapterState::Ready,
                Ok(CentralState::PoweredOff) => AdapterState::Off,
                // Some stacks report Unknown while working fine.
                _ => AdapterState::Ready,
            },
        }
    }

    async fn start_scan(&self, show_all: bool, found: mpsc::Sender<DiscoveredDevice>) -> KtResult<()> {
        let adapter = self.ready_adapter().await?;
        let mut events = adapter.events().await.map_err(|e| map_error(&e, ""))?;
        // No OS-level filter: some sensors only list 0x180D in the scan response, which
        // service filters can miss. We filter ourselves using the merged properties.
        adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| map_error(&e, ""))?;
        self.user_scanning.store(true, Ordering::SeqCst);
        let a = adapter.clone();
        let task = tokio::spawn(async move {
            // Report already known peripherals first.
            if let Ok(list) = a.peripherals().await {
                for p in list {
                    if let Some(d) = describe(&p).await {
                        if (show_all || d.has_hr_service) && found.send(d).await.is_err() {
                            return;
                        }
                    }
                }
            }
            while let Some(ev) = events.next().await {
                let id = match ev {
                    CentralEvent::DeviceDiscovered(id)
                    | CentralEvent::DeviceUpdated(id)
                    | CentralEvent::RssiUpdate { id, .. }
                    | CentralEvent::ServicesAdvertisement { id, .. }
                    | CentralEvent::ServiceDataAdvertisement { id, .. } => id,
                    _ => continue,
                };
                if let Ok(p) = a.peripheral(&id).await {
                    if let Some(d) = describe(&p).await {
                        if (show_all || d.has_hr_service) && found.send(d).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        if let Some(old) = self.scan_task.lock().await.replace(task) {
            old.abort();
        }
        Ok(())
    }

    async fn stop_scan(&self) -> KtResult<()> {
        self.user_scanning.store(false, Ordering::SeqCst);
        if let Some(t) = self.scan_task.lock().await.take() {
            t.abort();
        }
        if let Ok(a) = self.adapter().await {
            let _ = a.stop_scan().await;
        }
        Ok(())
    }

    async fn connect(
        &self,
        id: &str,
        name_hint: &str,
        events: mpsc::Sender<SensorEvent>,
    ) -> KtResult<Box<dyn SensorSession>> {
        let name = if name_hint.is_empty() { id } else { name_hint };
        let adapter = self.ready_adapter().await?;

        // Windows only connects to peripherals it has seen advertising: scan briefly if needed.
        let mut peripheral = self.find_peripheral(&adapter, id).await?;
        if peripheral.is_none() {
            adapter
                .start_scan(ScanFilter::default())
                .await
                .map_err(|e| map_error(&e, name))?;
            let deadline = Instant::now() + FIND_TIMEOUT;
            while peripheral.is_none() && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(300)).await;
                peripheral = self.find_peripheral(&adapter, id).await?;
            }
            if !self.user_scanning.load(Ordering::SeqCst) {
                let _ = adapter.stop_scan().await;
            }
        }
        let p = peripheral.ok_or_else(|| KtError::DeviceNotFound(name.to_string()))?;
        let name = describe(&p)
            .await
            .map(|d| d.name)
            .filter(|n| !n.starts_with("Unknown device"))
            .unwrap_or_else(|| name.to_string());

        if !p.is_connected().await.unwrap_or(false) {
            match tokio::time::timeout(CONNECT_TIMEOUT, p.connect()).await {
                Err(_) => return Err(KtError::ConnectTimeout(name)),
                Ok(Err(e)) => return Err(map_error(&e, &name)),
                Ok(Ok(())) => {}
            }
        }
        let setup = async {
            match tokio::time::timeout(DISCOVER_TIMEOUT, p.discover_services()).await {
                Err(_) => return Err(KtError::ConnectTimeout(name.clone())),
                Ok(Err(e)) => return Err(map_error(&e, &name)),
                Ok(Ok(())) => {}
            }
            let chars = p.characteristics();
            let hrm = chars
                .iter()
                .find(|c| c.uuid == HEART_RATE_MEASUREMENT)
                .cloned()
                .ok_or_else(|| KtError::NoHeartRateService(name.clone()))?;
            let mut notifications = p.notifications().await.map_err(|e| map_error(&e, &name))?;
            p.subscribe(&hrm).await.map_err(|e| map_error(&e, &name))?;

            let location = match chars.iter().find(|c| c.uuid == BODY_SENSOR_LOCATION) {
                Some(c) if c.properties.contains(CharPropFlags::READ) => p
                    .read(c)
                    .await
                    .ok()
                    .and_then(|v| v.first().map(|&b| BodySensorLocation::from_byte(b))),
                _ => None,
            };
            if let Some(c) = chars.iter().find(|c| c.uuid == BATTERY_LEVEL) {
                if c.properties.contains(CharPropFlags::READ) {
                    if let Some(b) = p.read(c).await.ok().and_then(|v| parse_battery(&v)) {
                        let _ = events.send(SensorEvent::Battery(b)).await;
                    }
                }
                if c.properties.contains(CharPropFlags::NOTIFY) {
                    let _ = p.subscribe(c).await;
                }
            }

            let tx = events.clone();
            let notify_task = tokio::spawn(async move {
                while let Some(n) = notifications.next().await {
                    let ev = if n.uuid == HEART_RATE_MEASUREMENT {
                        SensorEvent::Measurement {
                            data: n.value,
                            received: Instant::now(),
                        }
                    } else if n.uuid == BATTERY_LEVEL {
                        match parse_battery(&n.value) {
                            Some(b) => SensorEvent::Battery(b),
                            None => continue,
                        }
                    } else {
                        continue;
                    };
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }
                let _ = tx
                    .send(SensorEvent::Disconnected(Some(KtError::Ble(
                        "connection lost".into(),
                    ))))
                    .await;
            });

            // Watch for the OS reporting the disconnect (the notification stream doesn't
            // always end on Windows).
            let pid = p.id();
            let tx = events.clone();
            let mut central = adapter.events().await.map_err(|e| map_error(&e, &name))?;
            let watch_task = tokio::spawn(async move {
                while let Some(ev) = central.next().await {
                    match ev {
                        CentralEvent::DeviceDisconnected(id) if id == pid => {
                            let _ = tx.send(SensorEvent::Disconnected(None)).await;
                            return;
                        }
                        CentralEvent::StateUpdate(CentralState::PoweredOff) => {
                            let _ = tx.send(SensorEvent::Disconnected(Some(KtError::RadioOff))).await;
                            return;
                        }
                        _ => {}
                    }
                }
            });
            Ok((location, vec![notify_task, watch_task], hrm))
        };
        match setup.await {
            Ok((location, tasks, hrm)) => Ok(Box::new(BleSession {
                info: DeviceInfo {
                    id: id.to_string(),
                    name,
                    location,
                    demo: false,
                },
                peripheral: Arc::new(p),
                hrm,
                tasks,
            })),
            Err(e) => {
                let _ = tokio::time::timeout(Duration::from_secs(3), p.disconnect()).await;
                Err(e)
            }
        }
    }
}

struct BleSession {
    info: DeviceInfo,
    peripheral: Arc<Peripheral>,
    hrm: btleplug::api::Characteristic,
    tasks: Vec<JoinHandle<()>>,
}

#[async_trait]
impl SensorSession for BleSession {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    async fn disconnect(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
        let p = self.peripheral.clone();
        let hrm = self.hrm.clone();
        let _ = tokio::time::timeout(Duration::from_secs(3), async move {
            let _ = p.unsubscribe(&hrm).await;
            let _ = p.disconnect().await;
        })
        .await;
    }
}

impl Drop for BleSession {
    fn drop(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_windows_errors() {
        assert_eq!(
            classify_message("The Bluetooth radio is off", "x"),
            KtError::RadioOff
        );
        assert_eq!(
            classify_message("Adapter state: PoweredOff", "x"),
            KtError::RadioOff
        );
        assert_eq!(
            classify_message("Access is denied. (0x80070005)", "H10"),
            KtError::DeviceBusy("H10".into())
        );
        assert_eq!(
            classify_message("GattCommunicationStatus Unreachable", "H10"),
            KtError::ConnectTimeout("H10".into())
        );
        assert_eq!(classify_message("no adapter found", "x"), KtError::AdapterMissing);
        assert!(matches!(classify_message("weird", "x"), KtError::Ble(_)));
    }

    #[test]
    fn maps_typed_errors() {
        assert_eq!(
            map_error(&btleplug::Error::PermissionDenied, "x"),
            KtError::PermissionDenied
        );
        assert_eq!(
            map_error(&btleplug::Error::NoAdapterAvailable, "x"),
            KtError::AdapterMissing
        );
        assert_eq!(
            map_error(&btleplug::Error::TimedOut(Duration::from_secs(1)), "H9"),
            KtError::ConnectTimeout("H9".into())
        );
        assert_eq!(
            map_error(&btleplug::Error::DeviceNotFound, "H9"),
            KtError::DeviceNotFound("H9".into())
        );
    }
}
