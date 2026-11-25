//! OSCQuery: advertises KittyTooth over mDNS and discovers VRChat's OSC endpoint.
//!
//! * Advertising: a tiny HTTP server (loopback only, so Windows Firewall never prompts)
//!   serves `/?HOST_INFO` and the node tree. We register `_oscjson._tcp` and `_osc._udp`.
//!   Our tree exposes `/avatar/change`, so VRChat tells us when the avatar changes and the
//!   engine can resend every parameter immediately.
//! * Discovery: browse `_oscjson._tcp` for `VRChat-Client-*` services, fetch their
//!   `HOST_INFO`, and report `OSC_IP`/`OSC_PORT`. Targets with `use_oscquery` then send to
//!   that port; otherwise (or if nothing is found) the manual port is used.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::{KtError, KtResult};
use crate::http;

pub const OSCJSON_SERVICE: &str = "_oscjson._tcp.local.";
pub const OSC_SERVICE: &str = "_osc._udp.local.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VrchatEndpoint {
    /// mDNS instance full name.
    pub service: String,
    pub ip: IpAddr,
    pub port: u16,
    /// Reachable on this machine via loopback.
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscQueryEvent {
    VrchatFound(VrchatEndpoint),
    VrchatLost(String),
    AvatarChanged,
}

/// Running OSCQuery services. Dropping it stops everything.
pub struct OscQueryHandle {
    pub http_port: u16,
    pub osc_port: u16,
    pub mdns_active: bool,
    pub mdns_error: Option<String>,
    daemon: Option<mdns_sd::ServiceDaemon>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for OscQueryHandle {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
        if let Some(d) = self.daemon.take() {
            let _ = d.shutdown();
        }
    }
}

/// Parses a HOST_INFO document into (ip, port).
pub fn parse_host_info(body: &[u8]) -> Option<(IpAddr, u16)> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let port = u16::try_from(v.get("OSC_PORT")?.as_u64()?).ok()?;
    let ip = v
        .get("OSC_IP")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    Some((ip, port))
}

pub fn host_info(name: &str, osc_port: u16) -> Value {
    json!({
        "NAME": name,
        "OSC_IP": "127.0.0.1",
        "OSC_PORT": osc_port,
        "OSC_TRANSPORT": "UDP",
        "EXTENSIONS": { "ACCESS": true, "VALUE": true, "DESCRIPTION": true }
    })
}

pub fn node_tree() -> Value {
    json!({
        "DESCRIPTION": "KittyTooth",
        "FULL_PATH": "/",
        "ACCESS": 0,
        "CONTENTS": {
            "avatar": {
                "FULL_PATH": "/avatar",
                "ACCESS": 0,
                "CONTENTS": {
                    "change": {
                        "FULL_PATH": "/avatar/change",
                        "ACCESS": 2,
                        "TYPE": "s",
                        "DESCRIPTION": "Avatar id; KittyTooth resends all parameters on change"
                    }
                }
            }
        }
    })
}

fn find_node<'a>(tree: &'a Value, path: &str) -> Option<&'a Value> {
    let mut node = tree;
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        node = node.get("CONTENTS")?.get(seg)?;
    }
    Some(node)
}

async fn serve_http(listener: TcpListener, name: String, osc_port: u16) {
    let tree = node_tree();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let tree = tree.clone();
        let name = name.clone();
        tokio::spawn(async move {
            let Ok(req) = http::read_request(&mut stream).await else {
                return;
            };
            let resp = if req.method != "GET" {
                http::response(405, "text/plain", b"method not allowed")
            } else if req.query.as_deref() == Some("HOST_INFO") {
                http::response(
                    200,
                    "application/json",
                    host_info(&name, osc_port).to_string().as_bytes(),
                )
            } else {
                match find_node(&tree, &req.path) {
                    Some(n) => http::response(200, "application/json", n.to_string().as_bytes()),
                    None => http::response(404, "text/plain", b"not found"),
                }
            };
            let _ = stream.write_all(&resp).await;
            let _ = stream.shutdown().await;
        });
    }
}

fn contains_avatar_change(p: &rosc::OscPacket) -> bool {
    match p {
        rosc::OscPacket::Message(m) => m.addr == "/avatar/change",
        rosc::OscPacket::Bundle(b) => b.content.iter().any(contains_avatar_change),
    }
}

async fn serve_osc(sock: UdpSocket, events: mpsc::Sender<OscQueryEvent>) {
    let mut buf = vec![0u8; rosc::decoder::MTU];
    loop {
        let Ok((n, _)) = sock.recv_from(&mut buf).await else {
            continue;
        };
        if let Ok((_, pkt)) = rosc::decoder::decode_udp(&buf[..n]) {
            if contains_avatar_change(&pkt) && events.send(OscQueryEvent::AvatarChanged).await.is_err() {
                return;
            }
        }
    }
}

/// Fetches HOST_INFO, preferring loopback (VRChat on this PC).
pub async fn resolve_vrchat(service: &str, addrs: &[IpAddr], http_port: u16) -> Option<VrchatEndpoint> {
    let mut candidates = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    candidates.extend(addrs.iter().copied().filter(|a| a.is_ipv4() && !a.is_loopback()));
    for (i, ip) in candidates.into_iter().enumerate() {
        let addr = SocketAddr::new(ip, http_port);
        if let Ok((200, body)) = http::get(addr, "/?HOST_INFO", Duration::from_secs(2)).await {
            if let Some((osc_ip, port)) = parse_host_info(&body) {
                let local = i == 0;
                return Some(VrchatEndpoint {
                    service: service.to_string(),
                    ip: if local { osc_ip } else { ip },
                    port,
                    local,
                });
            }
        }
    }
    None
}

async fn browse(receiver: mdns_sd::Receiver<mdns_sd::ServiceEvent>, events: mpsc::Sender<OscQueryEvent>) {
    while let Ok(ev) = receiver.recv_async().await {
        match ev {
            mdns_sd::ServiceEvent::ServiceResolved(info) => {
                let name = info.get_fullname().to_string();
                if !name.starts_with("VRChat-Client-") {
                    continue;
                }
                let addrs: Vec<IpAddr> = info.get_addresses().iter().map(|a| a.to_ip_addr()).collect();
                if let Some(ep) = resolve_vrchat(&name, &addrs, info.get_port()).await {
                    tracing::info!(ip = %ep.ip, port = ep.port, local = ep.local, "discovered VRChat via OSCQuery");
                    if events.send(OscQueryEvent::VrchatFound(ep)).await.is_err() {
                        return;
                    }
                }
            }
            mdns_sd::ServiceEvent::ServiceRemoved(_, name) if name.starts_with("VRChat-Client-") => {
                if events.send(OscQueryEvent::VrchatLost(name)).await.is_err() {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Starts the OSCQuery HTTP/OSC servers and, if `mdns` is true, mDNS advertise + discovery.
/// mDNS failure (no multicast, blocked) is not fatal: it's reported in `mdns_error`.
pub async fn start(mdns: bool, events: mpsc::Sender<OscQueryEvent>) -> KtResult<OscQueryHandle> {
    let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|e| KtError::OscSocket(e.to_string()))?;
    let osc_port = udp
        .local_addr()
        .map_err(|e| KtError::OscSocket(e.to_string()))?
        .port();
    let tcp = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let http_port = tcp.local_addr()?.port();
    let suffix: String = uuid::Uuid::new_v4().simple().to_string()[..4].to_uppercase();
    let name = format!("KittyTooth-{suffix}");

    let mut tasks = vec![
        tokio::spawn(serve_http(tcp, name.clone(), osc_port)),
        tokio::spawn(serve_osc(udp, events.clone())),
    ];
    let mut handle = OscQueryHandle {
        http_port,
        osc_port,
        mdns_active: false,
        mdns_error: None,
        daemon: None,
        tasks: Vec::new(),
    };
    if mdns {
        match start_mdns(&name, http_port, osc_port) {
            Ok((daemon, receiver)) => {
                tasks.push(tokio::spawn(browse(receiver, events)));
                handle.daemon = Some(daemon);
                handle.mdns_active = true;
            }
            Err(e) => {
                tracing::warn!(error = %e, "mDNS unavailable; OSCQuery discovery disabled");
                handle.mdns_error = Some(e);
            }
        }
    }
    handle.tasks = tasks;
    Ok(handle)
}

type MdnsStart = (mdns_sd::ServiceDaemon, mdns_sd::Receiver<mdns_sd::ServiceEvent>);

fn start_mdns(name: &str, http_port: u16, osc_port: u16) -> Result<MdnsStart, String> {
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let host = format!("{}.local.", name.to_lowercase());
    for (ty, port) in [(OSCJSON_SERVICE, http_port), (OSC_SERVICE, osc_port)] {
        let info = mdns_sd::ServiceInfo::new(
            ty,
            name,
            &host,
            "127.0.0.1",
            port,
            None::<std::collections::HashMap<String, String>>,
        )
        .map_err(|e| e.to_string())?;
        daemon.register(info).map_err(|e| e.to_string())?;
    }
    let receiver = daemon.browse(OSCJSON_SERVICE).map_err(|e| e.to_string())?;
    Ok((daemon, receiver))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_info_parsing() {
        let body =
            br#"{"NAME":"VRChat-Client-ABC123","OSC_IP":"127.0.0.1","OSC_PORT":9010,"OSC_TRANSPORT":"UDP"}"#;
        assert_eq!(parse_host_info(body), Some(("127.0.0.1".parse().unwrap(), 9010)));
        assert_eq!(
            parse_host_info(br#"{"OSC_PORT":9000}"#),
            Some(("127.0.0.1".parse().unwrap(), 9000))
        );
        assert_eq!(parse_host_info(br#"{"NAME":"x"}"#), None);
        assert_eq!(parse_host_info(b"nope"), None);
        let ours = host_info("KittyTooth-TEST", 1234).to_string();
        assert_eq!(
            parse_host_info(ours.as_bytes()),
            Some(("127.0.0.1".parse().unwrap(), 1234))
        );
    }

    #[test]
    fn tree_lookup() {
        let t = node_tree();
        assert_eq!(find_node(&t, "/").unwrap()["FULL_PATH"], "/");
        assert_eq!(find_node(&t, "/avatar/change").unwrap()["TYPE"], "s");
        assert!(find_node(&t, "/nope").is_none());
    }

    #[tokio::test]
    async fn serves_host_info_tree_and_avatar_change() {
        let (tx, mut rx) = mpsc::channel(8);
        let h = start(false, tx).await.unwrap();
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, h.http_port));
        let (s, body) = http::get(addr, "/?HOST_INFO", Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(s, 200);
        let (_, port) = parse_host_info(&body).unwrap();
        assert_eq!(port, h.osc_port);
        let (s, body) = http::get(addr, "/avatar/change", Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(s, 200);
        assert!(String::from_utf8_lossy(&body).contains("/avatar/change"));
        let (s, _) = http::get(addr, "/missing", Duration::from_secs(2)).await.unwrap();
        assert_eq!(s, 404);

        // VRChat sends /avatar/change to our OSC port.
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let pkt = crate::osc::encode_message("/avatar/change", vec![rosc::OscType::String("avtr_x".into())])
            .unwrap();
        sock.send_to(&pkt, ("127.0.0.1", h.osc_port)).await.unwrap();
        let ev = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap();
        assert_eq!(ev, Some(OscQueryEvent::AvatarChanged));
    }

    #[tokio::test]
    async fn resolves_local_vrchat_host_info() {
        // A fake VRChat OSCQuery server on loopback.
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let _ = http::read_request(&mut s).await;
            let body = br#"{"NAME":"VRChat-Client-F00D","OSC_IP":"127.0.0.1","OSC_PORT":9555}"#;
            s.write_all(&http::response(200, "application/json", body))
                .await
                .unwrap();
        });
        let ep = resolve_vrchat("VRChat-Client-F00D._oscjson._tcp.local.", &[], port)
            .await
            .unwrap();
        assert_eq!(ep.port, 9555);
        assert!(ep.local);
    }
}
