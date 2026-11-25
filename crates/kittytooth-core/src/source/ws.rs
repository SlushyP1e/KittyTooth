//! Shared WebSocket plumbing for the network sources (Pulsoid, HypeRate).
//!
//! A source supplies a [`Protocol`] (what to send after connecting, how to read a text frame,
//! an optional app-level heartbeat); this module owns the socket task, error mapping and the
//! clean shutdown. URLs carry secrets, so they are never logged.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::error::{KtError, KtResult};
use crate::sensor::{DeviceInfo, SensorEvent, SensorSession};

pub(crate) type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// A failed WebSocket upgrade, reduced to what the sources need to pick an error.
#[derive(Debug)]
pub(crate) enum OpenError {
    /// The server answered the upgrade with an HTTP error.
    Http { status: u16, body: String },
    /// DNS, TCP, TLS or timeout: the service can't be reached.
    Unreachable(String),
}

pub(crate) async fn open(url: &str) -> Result<Ws, OpenError> {
    match tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(url)).await {
        Err(_) => Err(OpenError::Unreachable("timed out".into())),
        Ok(Ok((ws, _resp))) => Ok(ws),
        Ok(Err(tungstenite::Error::Http(resp))) => {
            let status = resp.status().as_u16();
            let body = resp
                .body()
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).chars().take(300).collect())
                .unwrap_or_default();
            Err(OpenError::Http { status, body })
        }
        Ok(Err(e)) => Err(OpenError::Unreachable(e.to_string())),
    }
}

/// Per-service message handling.
pub(crate) trait Protocol: Send + 'static {
    /// Text frames to send right after the connection opens (e.g. a channel join).
    fn on_open(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// Handles one text frame. An `Err` closes the session with that error.
    fn on_text(&mut self, text: &str) -> KtResult<Vec<SensorEvent>>;
    /// App-level keepalive: interval and the frame to send.
    fn heartbeat(&mut self) -> Option<(Duration, String)> {
        None
    }
    /// Frames to send before a user-initiated close (e.g. a channel leave).
    fn on_leave(&mut self) -> Vec<String> {
        Vec::new()
    }
    /// The error to report when the server closes the connection.
    fn closed(&self, code: Option<u16>, reason: &str) -> KtError;
}

/// Runs the socket until it closes, the receiver goes away, or the session is stopped.
pub(crate) fn spawn<P: Protocol>(
    mut ws: Ws,
    mut proto: P,
    events: mpsc::Sender<SensorEvent>,
) -> (JoinHandle<()>, oneshot::Sender<()>) {
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        for m in proto.on_open() {
            if ws.send(Message::text(m)).await.is_err() {
                let _ = events
                    .send(SensorEvent::Disconnected(Some(proto.closed(None, "send failed"))))
                    .await;
                return;
            }
        }
        let hb = proto.heartbeat();
        let period = hb.as_ref().map(|(d, _)| *d).unwrap_or(Duration::from_secs(3600));
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            tokio::select! {
                _ = &mut stop_rx => {
                    for m in proto.on_leave() {
                        let _ = ws.send(Message::text(m)).await;
                    }
                    let _ = tokio::time::timeout(Duration::from_millis(300), ws.close(None)).await;
                    return;
                }
                _ = ticker.tick(), if hb.is_some() => {
                    if let Some((_, frame)) = proto.heartbeat() {
                        if ws.send(Message::text(frame)).await.is_err() {
                            let _ = events.send(SensorEvent::Disconnected(Some(proto.closed(None, "heartbeat failed")))).await;
                            return;
                        }
                    }
                }
                msg = ws.next() => {
                    let err = match msg {
                        Some(Ok(Message::Text(t))) => match proto.on_text(t.as_str()) {
                            Ok(evs) => {
                                for ev in evs {
                                    if events.send(ev).await.is_err() {
                                        return;
                                    }
                                }
                                continue;
                            }
                            Err(e) => {
                                let _ = tokio::time::timeout(Duration::from_millis(300), ws.close(None)).await;
                                e
                            }
                        },
                        Some(Ok(Message::Binary(b))) => match std::str::from_utf8(&b).map(|s| proto.on_text(s)) {
                            Ok(Ok(evs)) => {
                                for ev in evs {
                                    if events.send(ev).await.is_err() {
                                        return;
                                    }
                                }
                                continue;
                            }
                            Ok(Err(e)) => e,
                            Err(_) => continue,
                        },
                        Some(Ok(Message::Close(frame))) => {
                            let (code, reason) = frame
                                .map(|f| (Some(u16::from(f.code)), f.reason.to_string()))
                                .unwrap_or((None, String::new()));
                            proto.closed(code, &reason)
                        }
                        // Pings are answered by tungstenite on the next read/write.
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => proto.closed(None, &e.to_string()),
                        None => proto.closed(None, "connection closed"),
                    };
                    let _ = events.send(SensorEvent::Disconnected(Some(err))).await;
                    return;
                }
            }
        }
    });
    (task, stop_tx)
}

pub(crate) struct WsSession {
    pub info: DeviceInfo,
    pub task: Option<JoinHandle<()>>,
    pub stop: Option<oneshot::Sender<()>>,
}

#[async_trait]
impl SensorSession for WsSession {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    async fn disconnect(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.task.take() {
            if tokio::time::timeout(Duration::from_millis(600), &mut *Box::pin(t))
                .await
                .is_err()
            {
                // Task is aborted when the handle is dropped below.
            }
        }
    }
}

impl Drop for WsSession {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

/// Percent-encodes a query/path component (RFC 3986 unreserved characters pass through).
pub(crate) fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Parses an RFC 3339 / ISO 8601 UTC timestamp like `2026-10-17T12:00:00.123Z` to epoch ms.
pub(crate) fn parse_iso_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let time = time.strip_suffix('Z').or_else(|| time.strip_suffix("+00:00"))?;
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let mut t = hms.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi, se) = (t.next()??, t.next()??, t.next()??);
    let ms: i64 = if frac.is_empty() {
        0
    } else {
        let f: String = frac.chars().chain("000".chars()).take(3).collect();
        f.parse().ok()?
    };
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let total = ((days * 24 + h) * 60 + mi) * 60 + se;
    u64::try_from(total * 1000 + ms).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_timestamps() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("2021-07-03T11:10:55Z"), Some(1_625_310_655_000));
        assert_eq!(parse_iso_ms("2026-10-17T12:00:00.123Z"), Some(1_792_238_400_123));
        assert_eq!(parse_iso_ms("nonsense"), None);
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(encode_component("abc-123_~."), "abc-123_~.");
        assert_eq!(encode_component("a b&c/d"), "a%20b%26c%2Fd");
    }
}
