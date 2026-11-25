//! OSC input: listens on a UDP port for a heart rate value sent by another app (int or float on
//! a configurable address) so KittyTooth can relay it to its own outputs.

use std::net::{Ipv4Addr, SocketAddr};

use async_trait::async_trait;
use rosc::{OscPacket, OscType};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{HeartRateSource, HrSample, SourceKind};
use crate::config::OscInputConfig;
use crate::error::{KtError, KtResult};
use crate::sensor::{DeviceInfo, SensorEvent, SensorSession};

pub struct OscInputSource {
    cfg: OscInputConfig,
}

impl OscInputSource {
    pub fn new(cfg: OscInputConfig) -> Self {
        Self { cfg }
    }
}

/// Extracts a heart rate from a message if it is on `address` and carries a number.
/// Floats are taken as BPM (rounded); out-of-range values are rejected later by the processor.
pub fn heart_rate_from(packet: &OscPacket, address: &str) -> Option<u16> {
    match packet {
        OscPacket::Message(m) if m.addr == address => m.args.first().and_then(|a| {
            let v = match a {
                OscType::Int(i) => *i as f64,
                OscType::Long(i) => *i as f64,
                OscType::Float(f) => *f as f64,
                OscType::Double(d) => *d,
                _ => return None,
            };
            (v.is_finite() && v >= 0.0).then(|| v.round().min(u16::MAX as f64) as u16)
        }),
        OscPacket::Message(_) => None,
        // Use the last matching value in a bundle.
        OscPacket::Bundle(b) => b.content.iter().rev().find_map(|p| heart_rate_from(p, address)),
    }
}

#[async_trait]
impl HeartRateSource for OscInputSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Osc
    }

    fn label(&self) -> String {
        format!("OSC input :{}", self.cfg.port)
    }

    async fn connect(&self, events: mpsc::Sender<SensorEvent>) -> KtResult<Box<dyn SensorSession>> {
        let address = crate::osc::normalize_address(&self.cfg.address)?;
        let ip = if self.cfg.lan {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        let sock = UdpSocket::bind(SocketAddr::from((ip, self.cfg.port)))
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::AddrInUse => KtError::PortInUse(self.cfg.port),
                _ => KtError::OscSocket(e.to_string()),
            })?;
        let port = sock.local_addr().map(|a| a.port()).unwrap_or(self.cfg.port);
        tracing::info!(port, address = %address, lan = self.cfg.lan, "listening for OSC heart rate");
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                let Ok((n, _from)) = sock.recv_from(&mut buf).await else {
                    continue;
                };
                let Ok((_, pkt)) = rosc::decoder::decode_udp(&buf[..n]) else {
                    continue;
                };
                if let Some(bpm) = heart_rate_from(&pkt, &address) {
                    if events
                        .send(SensorEvent::Sample(HrSample::new(bpm, SourceKind::Osc)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
        Ok(Box::new(OscInputSession {
            info: DeviceInfo {
                id: format!("osc:{port}"),
                name: format!("OSC input :{port}"),
                location: None,
                demo: false,
            },
            task: Some(task),
        }))
    }
}

struct OscInputSession {
    info: DeviceInfo,
    task: Option<JoinHandle<()>>,
}

#[async_trait]
impl SensorSession for OscInputSession {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    async fn disconnect(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
            let _ = t.await;
        }
    }
}

impl Drop for OscInputSession {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rosc::{OscBundle, OscMessage, OscTime};

    fn msg(addr: &str, arg: OscType) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: addr.into(),
            args: vec![arg],
        })
    }

    #[test]
    fn values() {
        let a = "/avatar/parameters/HR";
        assert_eq!(heart_rate_from(&msg(a, OscType::Int(88)), a), Some(88));
        assert_eq!(heart_rate_from(&msg(a, OscType::Float(91.6)), a), Some(92));
        assert_eq!(heart_rate_from(&msg(a, OscType::Double(70.0)), a), Some(70));
        assert_eq!(heart_rate_from(&msg(a, OscType::Bool(true)), a), None);
        assert_eq!(heart_rate_from(&msg(a, OscType::Float(f32::NAN)), a), None);
        assert_eq!(heart_rate_from(&msg(a, OscType::Int(-5)), a), None);
        assert_eq!(heart_rate_from(&msg("/other", OscType::Int(88)), a), None);
        let b = OscPacket::Bundle(OscBundle {
            timetag: OscTime {
                seconds: 0,
                fractional: 1,
            },
            content: vec![
                msg(a, OscType::Int(80)),
                msg("/x", OscType::Int(1)),
                msg(a, OscType::Int(81)),
            ],
        });
        assert_eq!(heart_rate_from(&b, a), Some(81));
    }
}
