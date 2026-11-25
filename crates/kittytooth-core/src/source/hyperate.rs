//! HypeRate real-time heart rate over WebSocket (Phoenix channels).
//!
//! Implemented to HypeRate's official WebSocket API document
//! (<https://github.com/HypeRate/HypeRate-Websocket-API/blob/main/docs/WEBSOCKET_API.md>, linked
//! from <https://developers.hyperate.io/>):
//! connect to `wss://app.hyperate.io/ws/:deviceId?token=API_KEY`, send
//! `{"topic":"hr:ID","event":"phx_join","payload":{},"ref":"1"}`, receive
//! `{"topic":"hr:ID","event":"hr_update","payload":{"hr":75},"ref":null}`, send
//! `{"event":"ping","payload":{"timestamp":MS}}` every 15 s, and `phx_leave` before closing.
//! The docs don't specify how an invalid key or device ID is reported; see `upgrade_error` and
//! `parse_message` for how KittyTooth interprets HTTP rejections and error replies.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::ws::{self, OpenError, Protocol, WsSession};
use super::{HeartRateSource, HrSample, SourceKind};
use crate::error::{KtError, KtResult};
use crate::osc::sender::epoch_ms;
use crate::sensor::{DeviceInfo, SensorEvent, SensorSession};

pub const WS_BASE: &str = "wss://app.hyperate.io/ws";
pub const HEARTBEAT: Duration = Duration::from_secs(15);
const JOIN_REF: &str = "1";

/// Device IDs are shown in the HypeRate app. The format isn't documented, so KittyTooth only
/// accepts what can safely go into a URL path: 2-32 letters, digits, `-` or `_`.
pub fn validate_device_id(id: &str) -> KtResult<String> {
    let id = id.trim();
    let ok = (2..=32).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(id.to_string())
    } else {
        Err(KtError::InvalidDeviceId(id.chars().take(40).collect()))
    }
}

pub struct HypeRateSource {
    base: String,
    key: String,
    device_id: String,
}

impl HypeRateSource {
    pub fn new(base: String, key: String, device_id: String) -> Self {
        Self { base, key, device_id }
    }

    fn url(&self) -> String {
        format!(
            "{}/{}?token={}",
            self.base.trim_end_matches('/'),
            ws::encode_component(&self.device_id),
            ws::encode_component(&self.key)
        )
    }
}

pub(crate) fn upgrade_error(e: OpenError, device_id: &str) -> KtError {
    match e {
        OpenError::Unreachable(d) => {
            tracing::info!(detail = %d, "hyperate unreachable");
            KtError::Offline("HypeRate")
        }
        OpenError::Http { status, body } => {
            tracing::info!(status, body = %body, "hyperate rejected the connection");
            match status {
                401 | 403 => KtError::BadApiKey,
                404 => KtError::InvalidDeviceId(device_id.into()),
                429 => KtError::RateLimited("HypeRate"),
                _ => KtError::Service("HypeRate", format!("HTTP {status}")),
            }
        }
    }
}

#[async_trait]
impl HeartRateSource for HypeRateSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Hyperate
    }

    fn label(&self) -> String {
        format!("HypeRate {}", self.device_id)
    }

    async fn connect(&self, events: mpsc::Sender<SensorEvent>) -> KtResult<Box<dyn SensorSession>> {
        let socket = ws::open(&self.url())
            .await
            .map_err(|e| upgrade_error(e, &self.device_id))?;
        tracing::info!(device = %self.device_id, "hyperate connected");
        let proto = HypeRateProtocol {
            device_id: self.device_id.clone(),
        };
        let (task, stop) = ws::spawn(socket, proto, events);
        Ok(Box::new(WsSession {
            info: DeviceInfo {
                id: format!("hyperate:{}", self.device_id),
                name: self.label(),
                location: None,
                demo: false,
            },
            task: Some(task),
            stop: Some(stop),
        }))
    }
}

struct HypeRateProtocol {
    device_id: String,
}

impl HypeRateProtocol {
    fn topic(&self) -> String {
        format!("hr:{}", self.device_id)
    }
}

pub fn join_message(device_id: &str) -> String {
    json!({ "topic": format!("hr:{device_id}"), "event": "phx_join", "payload": {}, "ref": JOIN_REF })
        .to_string()
}

pub fn ping_message(now_ms: u64) -> String {
    json!({ "event": "ping", "payload": { "timestamp": now_ms } }).to_string()
}

impl Protocol for HypeRateProtocol {
    fn on_open(&mut self) -> Vec<String> {
        vec![join_message(&self.device_id)]
    }

    fn on_text(&mut self, text: &str) -> KtResult<Vec<SensorEvent>> {
        parse_message(text, &self.device_id)
    }

    fn heartbeat(&mut self) -> Option<(Duration, String)> {
        Some((HEARTBEAT, ping_message(epoch_ms())))
    }

    fn on_leave(&mut self) -> Vec<String> {
        vec![json!({ "topic": self.topic(), "event": "phx_leave", "payload": {}, "ref": epoch_ms().to_string() })
            .to_string()]
    }

    fn closed(&self, code: Option<u16>, reason: &str) -> KtError {
        tracing::info!(code = ?code, reason, "hyperate closed the connection");
        KtError::Service("HypeRate", "connection closed".into())
    }
}

/// Handles one HypeRate frame: `hr_update` gives a sample; an error reply to our join means
/// the device ID was refused; everything else (join confirmation, ping replies) is ignored.
pub fn parse_message(text: &str, device_id: &str) -> KtResult<Vec<SensorEvent>> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Ok(Vec::new());
    };
    let event = v.get("event").and_then(Value::as_str).unwrap_or("");
    match event {
        "hr_update" => {
            let hr = v.pointer("/payload/hr").and_then(|h| {
                h.as_u64().or_else(|| {
                    h.as_f64()
                        .filter(|f| f.is_finite() && *f >= 0.0)
                        .map(|f| f.round() as u64)
                })
            });
            Ok(hr
                .map(|bpm| {
                    vec![SensorEvent::Sample(HrSample::new(
                        bpm.min(u16::MAX as u64) as u16,
                        SourceKind::Hyperate,
                    ))]
                })
                .unwrap_or_default())
        }
        "phx_reply" => {
            let is_join = v.get("ref").and_then(Value::as_str) == Some(JOIN_REF);
            let status = v.pointer("/payload/status").and_then(Value::as_str);
            if is_join && status == Some("error") {
                let reason = v
                    .pointer("/payload/response/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("join refused");
                tracing::info!(reason, "hyperate refused the channel join");
                Err(KtError::InvalidDeviceId(device_id.into()))
            } else {
                Ok(Vec::new())
            }
        }
        "phx_error" => Err(KtError::Service("HypeRate", "channel error".into())),
        _ => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPDATE: &str = include_str!("../../tests/fixtures/hyperate_hr_update.json");
    const JOIN_OK: &str = include_str!("../../tests/fixtures/hyperate_phx_reply_ok.json");
    const JOIN_ERR: &str = include_str!("../../tests/fixtures/hyperate_phx_reply_error.json");

    #[test]
    fn hr_update() {
        let ev = parse_message(UPDATE, "abc123").unwrap();
        let [SensorEvent::Sample(s)] = ev.as_slice() else {
            panic!("{ev:?}")
        };
        assert_eq!(s.bpm, 75);
        assert_eq!(s.source, SourceKind::Hyperate);
        assert!(s.rr_ms.is_empty() && s.battery.is_none() && s.contact.is_none());
    }

    #[test]
    fn replies() {
        assert!(parse_message(JOIN_OK, "abc123").unwrap().is_empty());
        assert_eq!(
            parse_message(JOIN_ERR, "abc123").unwrap_err(),
            KtError::InvalidDeviceId("abc123".into())
        );
        assert!(parse_message("garbage", "x").unwrap().is_empty());
        assert!(parse_message(r#"{"event":"hr_update","payload":{}}"#, "x")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn protocol_frames_match_docs() {
        let j: Value = serde_json::from_str(&join_message("abc123")).unwrap();
        assert_eq!(
            j,
            json!({"topic":"hr:abc123","event":"phx_join","payload":{},"ref":"1"})
        );
        let p: Value = serde_json::from_str(&ping_message(42)).unwrap();
        assert_eq!(p, json!({"event":"ping","payload":{"timestamp":42}}));
        let s = HypeRateSource::new(WS_BASE.into(), "k y".into(), "abc123".into());
        assert_eq!(s.url(), "wss://app.hyperate.io/ws/abc123?token=k%20y");
    }

    #[test]
    fn device_ids() {
        assert_eq!(validate_device_id(" 1A2B3C ").unwrap(), "1A2B3C");
        assert!(validate_device_id("").is_err());
        assert!(validate_device_id("a").is_err());
        assert!(validate_device_id("abc/../x").is_err());
        assert!(validate_device_id("abc def").is_err());
        assert_eq!(validate_device_id("x").unwrap_err().code(), "invalid_device_id");
    }

    #[test]
    fn upgrade_errors() {
        let http = |status| OpenError::Http {
            status,
            body: String::new(),
        };
        assert_eq!(upgrade_error(http(403), "x"), KtError::BadApiKey);
        assert_eq!(
            upgrade_error(http(404), "x"),
            KtError::InvalidDeviceId("x".into())
        );
        assert_eq!(upgrade_error(http(429), "x").code(), "rate_limited");
        assert_eq!(
            upgrade_error(OpenError::Unreachable("x".into()), "x").code(),
            "offline"
        );
    }
}
