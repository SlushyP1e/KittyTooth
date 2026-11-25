//! Pulsoid real-time heart rate over WebSocket (v2 API).
//!
//! Implemented to <https://docs.pulsoid.net/heart-rate-and-sensor-data/read-data-via-websocket-v2>:
//! `wss://dev.pulsoid.net/api/v2/data/real_time?access_token=TOKEN&types=heart_rate,battery_level`.
//! Messages are JSON objects `{"type", "timestamp", "heartRate": {"bpm"}}` /
//! `{"batteryLevel": {"percent"}}` / `{"ingestionPause": {...}}`. Upgrade status codes: 400 when
//! the token lacks a scope for a requested type (error 6003), 401 for a missing or invalid token.
//! The v1 message shape (`{"measured_at", "data": {"heart_rate"}}`) is also accepted.

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use super::ws::{self, OpenError, Protocol, WsSession};
use super::{HeartRateSource, HrSample, SourceKind};
use crate::error::{KtError, KtResult};
use crate::sensor::{DeviceInfo, SensorEvent, SensorSession};

pub const REAL_TIME_V2: &str = "wss://dev.pulsoid.net/api/v2/data/real_time";
/// Where users create a manual token (docs: Manual Token Issuing).
pub const TOKEN_PAGE: &str = "https://pulsoid.net/ui/keys";

pub struct PulsoidSource {
    base: String,
    token: String,
}

impl PulsoidSource {
    pub fn new(base: String, token: String) -> Self {
        Self { base, token }
    }

    fn url(&self, types: &str) -> String {
        format!(
            "{}?access_token={}&types={}",
            self.base,
            ws::encode_component(&self.token),
            types
        )
    }
}

/// Maps a failed upgrade to a plain-language error.
pub(crate) fn upgrade_error(e: OpenError) -> KtError {
    match e {
        OpenError::Unreachable(d) => {
            tracing::info!(detail = %d, "pulsoid unreachable");
            KtError::Offline("Pulsoid")
        }
        OpenError::Http { status, body } => {
            tracing::info!(status, body = %body, "pulsoid rejected the connection");
            match status {
                401 | 403 => {
                    KtError::BadToken(error_message(&body).unwrap_or_else(|| "token_not_found".into()))
                }
                402 => KtError::PremiumRequired,
                429 => KtError::RateLimited("Pulsoid"),
                _ => KtError::Service("Pulsoid", format!("HTTP {status}")),
            }
        }
    }
}

fn error_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("error_message").and_then(Value::as_str).map(str::to_string)
}

#[async_trait]
impl HeartRateSource for PulsoidSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Pulsoid
    }

    fn label(&self) -> String {
        "Pulsoid".into()
    }

    async fn connect(&self, events: mpsc::Sender<SensorEvent>) -> KtResult<Box<dyn SensorSession>> {
        // Ask for the battery too; tokens without `data:battery_level:read` get HTTP 400, then
        // we fall back to heart rate only.
        let socket = match ws::open(&self.url("heart_rate,battery_level")).await {
            Ok(s) => s,
            Err(OpenError::Http { status: 400, body }) => {
                tracing::info!(body = %body, "pulsoid token lacks battery scope; heart rate only");
                ws::open(&self.url("heart_rate")).await.map_err(upgrade_error)?
            }
            Err(e) => return Err(upgrade_error(e)),
        };
        tracing::info!("pulsoid connected");
        let (task, stop) = ws::spawn(socket, PulsoidProtocol, events);
        Ok(Box::new(WsSession {
            info: DeviceInfo {
                id: "pulsoid".into(),
                name: "Pulsoid".into(),
                location: None,
                demo: false,
            },
            task: Some(task),
            stop: Some(stop),
        }))
    }
}

struct PulsoidProtocol;

impl Protocol for PulsoidProtocol {
    fn on_text(&mut self, text: &str) -> KtResult<Vec<SensorEvent>> {
        Ok(parse_message(text))
    }

    fn closed(&self, code: Option<u16>, reason: &str) -> KtError {
        tracing::info!(code = ?code, reason, "pulsoid closed the connection");
        // The server closes the socket when the token is revoked or expires; the reconnect
        // then gets a 401 and reports the bad token.
        KtError::Service("Pulsoid", "connection closed".into())
    }
}

/// Turns one Pulsoid message into events. Unknown or malformed messages yield nothing.
pub fn parse_message(text: &str) -> Vec<SensorEvent> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let ts = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(ws::parse_iso_ms)
        .or_else(|| v.get("measured_at").and_then(Value::as_u64));
    let bpm = v
        .pointer("/heartRate/bpm")
        .or_else(|| v.pointer("/data/heart_rate"))
        .and_then(Value::as_u64);
    if let Some(bpm) = bpm {
        let mut s = HrSample::new(bpm.min(u16::MAX as u64) as u16, SourceKind::Pulsoid);
        if let Some(t) = ts {
            s.timestamp_ms = t;
        }
        out.push(SensorEvent::Sample(s));
    }
    if let Some(p) = v.pointer("/batteryLevel/percent").and_then(Value::as_u64) {
        out.push(SensorEvent::Battery(p.min(100) as u8));
    }
    if let Some(ends) = v.pointer("/ingestionPause/ends_at").and_then(Value::as_str) {
        // No data until `ends_at`; staleness handling shows it as "no data" meanwhile.
        tracing::info!(ends_at = ends, "pulsoid paused data ingestion");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HR: &str = include_str!("../../tests/fixtures/pulsoid_v2_heart_rate.json");
    const BATTERY: &str = include_str!("../../tests/fixtures/pulsoid_v2_battery_level.json");
    const PAUSE: &str = include_str!("../../tests/fixtures/pulsoid_v2_ingestion_pause.json");
    const V1: &str = include_str!("../../tests/fixtures/pulsoid_v1_heart_rate.json");
    const ERR: &str = include_str!("../../tests/fixtures/pulsoid_error_token_not_found.json");

    #[test]
    fn v2_heart_rate() {
        let ev = parse_message(HR);
        let [SensorEvent::Sample(s)] = ev.as_slice() else {
            panic!("{ev:?}")
        };
        assert_eq!(s.bpm, 85);
        assert_eq!(s.source, SourceKind::Pulsoid);
        assert_eq!(s.timestamp_ms, 1_792_238_400_123);
        assert!(s.rr_ms.is_empty() && s.contact.is_none() && s.battery.is_none());
    }

    #[test]
    fn v2_battery_and_pause() {
        assert_eq!(parse_message(BATTERY), vec![SensorEvent::Battery(67)]);
        assert!(parse_message(PAUSE).is_empty());
    }

    #[test]
    fn v1_legacy_shape() {
        let ev = parse_message(V1);
        let [SensorEvent::Sample(s)] = ev.as_slice() else {
            panic!("{ev:?}")
        };
        assert_eq!(s.bpm, 40);
        assert_eq!(s.timestamp_ms, 1_625_310_655_000);
    }

    #[test]
    fn garbage_is_ignored() {
        assert!(parse_message("not json").is_empty());
        assert!(parse_message("{}").is_empty());
        assert!(parse_message(r#"{"heartRate":{"bpm":"fast"}}"#).is_empty());
    }

    #[test]
    fn upgrade_errors() {
        let e = upgrade_error(OpenError::Http {
            status: 401,
            body: ERR.into(),
        });
        assert_eq!(e, KtError::BadToken("token_not_found".into()));
        assert_eq!(e.code(), "bad_token");
        assert!(!e.retriable());
        assert_eq!(
            upgrade_error(OpenError::Http {
                status: 402,
                body: String::new()
            }),
            KtError::PremiumRequired
        );
        assert_eq!(
            upgrade_error(OpenError::Http {
                status: 429,
                body: String::new()
            })
            .code(),
            "rate_limited"
        );
        assert_eq!(
            upgrade_error(OpenError::Unreachable("dns".into())).code(),
            "offline"
        );
        assert!(upgrade_error(OpenError::Http {
            status: 503,
            body: String::new()
        })
        .retriable());
    }

    #[test]
    fn url_never_logs_token() {
        let s = PulsoidSource::new(REAL_TIME_V2.into(), "8c4da3ce-7ed7".into());
        assert_eq!(
            s.url("heart_rate"),
            "wss://dev.pulsoid.net/api/v2/data/real_time?access_token=8c4da3ce-7ed7&types=heart_rate"
        );
    }
}
