//! Local OBS browser source: `http://127.0.0.1:<port>/overlay` (transparent background),
//! fed live by Server-Sent Events from `/events`. `/api/state` returns a JSON snapshot.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use crate::engine::{EngineEvent, EngineHandle, Snapshot};
use crate::error::{KtError, KtResult};
use crate::http;

pub const OVERLAY_HTML: &str = include_str!("overlay.html");

/// Running overlay server; dropping it stops the server.
pub struct OverlayServer {
    pub addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Drop for OverlayServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl OverlayServer {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/overlay", self.addr.port())
    }
}

fn state_json(s: &Snapshot) -> String {
    json!({
        "bpm": s.bpm,
        "connected": s.data_valid || s.test_sweep,
        "zone": s.zone,
        "state": s.state,
    })
    .to_string()
}

/// Starts the server on 127.0.0.1:`port` (0 = any free port).
pub async fn start(engine: EngineHandle, port: u16) -> KtResult<OverlayServer> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                KtError::PortInUse(port)
            } else {
                KtError::Io(e.to_string())
            }
        })?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            let engine = engine.clone();
            tokio::spawn(async move {
                let _ = handle(stream, engine).await;
            });
        }
    });
    Ok(OverlayServer { addr, task })
}

async fn handle(mut stream: TcpStream, engine: EngineHandle) -> std::io::Result<()> {
    let req = http::read_request(&mut stream).await?;
    if req.method != "GET" {
        stream
            .write_all(&http::response(405, "text/plain", b"method not allowed"))
            .await?;
        return Ok(());
    }
    match req.path.as_str() {
        "/" | "/overlay" | "/overlay/" => {
            stream
                .write_all(&http::response(
                    200,
                    "text/html; charset=utf-8",
                    OVERLAY_HTML.as_bytes(),
                ))
                .await?;
        }
        "/api/state" => {
            let body = state_json(&engine.snapshot());
            stream
                .write_all(&http::response(200, "application/json", body.as_bytes()))
                .await?;
        }
        "/events" => return sse(stream, engine).await,
        _ => {
            stream
                .write_all(&http::response(404, "text/plain", b"not found"))
                .await?;
        }
    }
    stream.shutdown().await
}

async fn sse(mut stream: TcpStream, engine: EngineHandle) -> std::io::Result<()> {
    stream.write_all(http::sse_head()).await?;
    let mut snap = engine.subscribe_snapshot();
    let mut events = engine.subscribe_events();
    let mut last = state_json(&snap.borrow_and_update());
    stream
        .write_all(format!("event: state\ndata: {last}\n\n").as_bytes())
        .await?;
    let mut keepalive = tokio::time::interval(Duration::from_secs(15));
    keepalive.tick().await;
    loop {
        tokio::select! {
            r = snap.changed() => {
                if r.is_err() { return Ok(()); }
                let s = state_json(&snap.borrow_and_update());
                if s != last {
                    stream.write_all(format!("event: state\ndata: {s}\n\n").as_bytes()).await?;
                    last = s;
                }
            }
            e = events.recv() => match e {
                Ok(EngineEvent::Beat { interval_ms, .. }) => {
                    stream.write_all(format!("event: beat\ndata: {{\"interval_ms\":{interval_ms}}}\n\n").as_bytes()).await?;
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return Ok(()),
            },
            _ = keepalive.tick() => {
                stream.write_all(b": keepalive\n\n").await?;
            }
        }
    }
}
