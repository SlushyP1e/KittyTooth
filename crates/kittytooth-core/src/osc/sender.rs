//! Per-target OSC sender: change detection, keepalive, prioritised coalescing queue and a
//! strict sliding-window rate limit (never more than `max_rate` messages in any 1 s window).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::config::OscTarget;
use crate::osc::presets::{ParamMsg, Role};
use crate::osc::{encode_message, OscValue};

/// Strict sliding-window limiter.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    max: u32,
    window: Duration,
    sent: VecDeque<Instant>,
}

impl RateLimiter {
    pub fn new(max_per_sec: u32) -> Self {
        Self {
            max: max_per_sec.max(1),
            window: Duration::from_secs(1),
            sent: VecDeque::new(),
        }
    }

    pub fn set_max(&mut self, max_per_sec: u32) {
        self.max = max_per_sec.max(1);
    }

    fn prune(&mut self, now: Instant) {
        while let Some(&t) = self.sent.front() {
            if now.saturating_duration_since(t) >= self.window {
                self.sent.pop_front();
            } else {
                break;
            }
        }
    }

    /// Consumes a slot if one is free.
    pub fn try_acquire(&mut self, now: Instant) -> bool {
        self.prune(now);
        if (self.sent.len() as u32) < self.max {
            self.sent.push_back(now);
            true
        } else {
            false
        }
    }

    /// Earliest instant a slot frees up (now if one is free).
    pub fn next_available(&mut self, now: Instant) -> Instant {
        self.prune(now);
        if (self.sent.len() as u32) < self.max {
            now
        } else {
            self.sent.front().map_or(now, |&t| t + self.window)
        }
    }
}

/// One entry in the live OSC log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OscLogEntry {
    pub seq: u64,
    /// Unix epoch milliseconds.
    pub t_ms: u64,
    pub target: String,
    pub address: String,
    pub value: String,
    pub kind: String,
    pub test: bool,
}

pub fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TargetStats {
    pub sent: u64,
    /// Messages replaced in the queue by a newer value before being sent.
    pub coalesced: u64,
    pub errors: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct Pending {
    address: String,
    value: OscValue,
    prio: u8,
    seq: u64,
}

/// A queued outgoing datagram (address + encoded bytes) ready for the socket.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub dest: SocketAddr,
    pub bytes: Vec<u8>,
    pub log: OscLogEntry,
}

/// Sender state for one OSC target.
#[derive(Debug)]
pub struct TargetSender {
    pub target: OscTarget,
    pub dest: Option<SocketAddr>,
    last: HashMap<String, (OscValue, Instant)>,
    pending: Vec<Pending>,
    limiter: RateLimiter,
    seq: u64,
    chatbox_last: Option<(Instant, String)>,
    pub stats: TargetStats,
}

impl TargetSender {
    pub fn new(target: OscTarget, dest: Option<SocketAddr>, max_rate: u32) -> Self {
        Self {
            target,
            dest,
            last: HashMap::new(),
            pending: Vec::new(),
            limiter: RateLimiter::new(max_rate),
            seq: 0,
            chatbox_last: None,
            stats: TargetStats::default(),
        }
    }

    pub fn set_max_rate(&mut self, r: u32) {
        self.limiter.set_max(r);
    }

    /// Forget what was sent so the next offer resends everything (e.g. avatar change).
    pub fn invalidate(&mut self) {
        self.last.clear();
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Queues messages that changed, or whose last send is older than `keepalive`.
    /// With `force`, everything is queued.
    pub fn offer(&mut self, msgs: &[ParamMsg], now: Instant, keepalive: Option<Duration>, force: bool) {
        for m in msgs {
            let last = self.last.get(&m.address);
            let changed = last.is_none_or(|(v, _)| *v != m.value);
            let stale =
                keepalive.is_some_and(|k| last.is_none_or(|(_, t)| now.saturating_duration_since(*t) >= k));
            if !(changed || stale || force) {
                continue;
            }
            let prio = match (m.role, changed) {
                (Role::Beat, _) => 0,
                (Role::Connection, true) => 1,
                (Role::Value, true) => 2,
                _ => 3, // keepalive refresh
            };
            self.enqueue(&m.address, m.value, prio);
        }
    }

    fn enqueue(&mut self, address: &str, value: OscValue, prio: u8) {
        if let Some(p) = self.pending.iter_mut().find(|p| p.address == address) {
            if p.value != value {
                self.stats.coalesced += 1;
            }
            p.value = value;
            p.prio = p.prio.min(prio);
            return;
        }
        self.seq += 1;
        self.pending.push(Pending {
            address: address.to_string(),
            value,
            prio,
            seq: self.seq,
        });
    }

    /// Takes as many queued messages as the rate limit allows, highest priority first.
    pub fn drain(&mut self, now: Instant, test: bool) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let Some(dest) = self.dest else {
            return out;
        };
        if self.pending.is_empty() {
            return out;
        }
        self.pending.sort_by_key(|p| (p.prio, p.seq));
        let mut taken = 0;
        for p in &self.pending {
            if !self.limiter.try_acquire(now) {
                break;
            }
            taken += 1;
            match encode_message(&p.address, vec![p.value.to_rosc()]) {
                Ok(bytes) => {
                    self.last.insert(p.address.clone(), (p.value, now));
                    out.push(Outgoing {
                        dest,
                        bytes,
                        log: OscLogEntry {
                            seq: 0,
                            t_ms: epoch_ms(),
                            target: self.target.name.clone(),
                            address: p.address.clone(),
                            value: p.value.display(),
                            kind: p.value.kind().into(),
                            test,
                        },
                    });
                }
                Err(e) => {
                    self.stats.errors += 1;
                    self.stats.last_error = Some(e.to_string());
                }
            }
        }
        self.pending.drain(..taken);
        out
    }

    /// When the next queued message may be sent, if any are queued.
    pub fn next_flush_at(&mut self, now: Instant) -> Option<Instant> {
        if self.pending.is_empty() || self.dest.is_none() {
            None
        } else {
            Some(self.limiter.next_available(now))
        }
    }

    /// Chatbox: sends `text` if `interval` passed since the last send and the text changed
    /// (or 20 s passed, to keep it visible). Respects the shared rate limit.
    pub fn chatbox(
        &mut self,
        text: &str,
        silent: bool,
        interval: Duration,
        now: Instant,
        test: bool,
    ) -> Option<Outgoing> {
        let dest = self.dest?;
        if let Some((t, last)) = &self.chatbox_last {
            let elapsed = now.saturating_duration_since(*t);
            if elapsed < interval || (last == text && elapsed < Duration::from_secs(20)) {
                return None;
            }
        }
        if !self.limiter.try_acquire(now) {
            return None;
        }
        let args = vec![
            rosc::OscType::String(text.to_string()),
            rosc::OscType::Bool(true),    // send immediately, bypass the keyboard
            rosc::OscType::Bool(!silent), // play notification sound
        ];
        let bytes = encode_message("/chatbox/input", args).ok()?;
        self.chatbox_last = Some((now, text.to_string()));
        Some(Outgoing {
            dest,
            bytes,
            log: OscLogEntry {
                seq: 0,
                t_ms: epoch_ms(),
                target: self.target.name.clone(),
                address: "/chatbox/input".into(),
                value: format!("\"{text}\""),
                kind: "string".into(),
                test,
            },
        })
    }

    pub fn reset_chatbox(&mut self) {
        self.chatbox_last = None;
    }

    pub fn record_result(&mut self, ok: Result<(), String>) {
        match ok {
            Ok(()) => self.stats.sent += 1,
            Err(e) => {
                self.stats.errors += 1;
                self.stats.last_error = Some(e);
            }
        }
    }
}

/// Bounded ring buffer of the most recent OSC messages.
#[derive(Debug, Clone)]
pub struct OscLog {
    cap: usize,
    next_seq: u64,
    entries: VecDeque<OscLogEntry>,
}

impl Default for OscLog {
    fn default() -> Self {
        Self::new(50)
    }
}

impl OscLog {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            next_seq: 1,
            entries: VecDeque::with_capacity(cap),
        }
    }

    pub fn push(&mut self, mut e: OscLogEntry) {
        e.seq = self.next_seq;
        self.next_seq += 1;
        if self.entries.len() == self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(e);
    }

    pub fn entries(&self) -> Vec<OscLogEntry> {
        self.entries.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(addr: &str, v: OscValue, role: Role) -> ParamMsg {
        ParamMsg {
            address: addr.into(),
            value: v,
            role,
        }
    }

    fn sender(rate: u32) -> TargetSender {
        TargetSender::new(
            OscTarget::default(),
            Some("127.0.0.1:9000".parse().unwrap()),
            rate,
        )
    }

    #[test]
    fn limiter_strict_window() {
        let mut l = RateLimiter::new(20);
        let t = Instant::now();
        for _ in 0..20 {
            assert!(l.try_acquire(t));
        }
        assert!(!l.try_acquire(t));
        assert!(!l.try_acquire(t + Duration::from_millis(999)));
        assert_eq!(l.next_available(t), t + Duration::from_secs(1));
        assert!(l.try_acquire(t + Duration::from_secs(1)));
    }

    #[test]
    fn limiter_never_exceeds_cap_in_any_window() {
        let mut l = RateLimiter::new(20);
        let start = Instant::now();
        let mut sends = Vec::new();
        for ms in 0..5000u64 {
            let now = start + Duration::from_millis(ms);
            // try 3 times per ms: a flood
            for _ in 0..3 {
                if l.try_acquire(now) {
                    sends.push(ms);
                }
            }
        }
        for (i, &s) in sends.iter().enumerate() {
            let in_window = sends[i..].iter().take_while(|&&x| x < s + 1000).count();
            assert!(in_window <= 20, "window starting {s}ms had {in_window}");
        }
        assert!(sends.len() >= 100); // 5 s * 20/s
    }

    #[test]
    fn only_changes_are_sent() {
        let mut s = sender(20);
        let t = Instant::now();
        let v = vec![
            msg("/a", OscValue::Int(1), Role::Value),
            msg("/b", OscValue::Int(2), Role::Value),
        ];
        s.offer(&v, t, None, false);
        assert_eq!(s.drain(t, false).len(), 2);
        s.offer(&v, t + Duration::from_millis(100), None, false);
        assert_eq!(s.drain(t, false).len(), 0);
        let v2 = vec![
            msg("/a", OscValue::Int(5), Role::Value),
            msg("/b", OscValue::Int(2), Role::Value),
        ];
        s.offer(&v2, t + Duration::from_millis(200), None, false);
        let out = s.drain(t + Duration::from_millis(200), false);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].log.address, "/a");
    }

    #[test]
    fn keepalive_resends_unchanged() {
        let mut s = sender(20);
        let t = Instant::now();
        let v = vec![msg("/a", OscValue::Int(1), Role::Value)];
        let ka = Some(Duration::from_secs(1));
        s.offer(&v, t, ka, false);
        assert_eq!(s.drain(t, false).len(), 1);
        s.offer(&v, t + Duration::from_millis(500), ka, false);
        assert_eq!(s.drain(t + Duration::from_millis(500), false).len(), 0);
        s.offer(&v, t + Duration::from_millis(1000), ka, false);
        assert_eq!(s.drain(t + Duration::from_millis(1000), false).len(), 1);
    }

    #[test]
    fn beats_jump_the_queue_and_rate_cap_holds() {
        let mut s = sender(3);
        let t = Instant::now();
        let v: Vec<ParamMsg> = (0..5)
            .map(|i| msg(&format!("/v{i}"), OscValue::Int(i), Role::Value))
            .chain([msg("/beat", OscValue::Bool(true), Role::Beat)])
            .collect();
        s.offer(&v, t, None, false);
        let out = s.drain(t, false);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].log.address, "/beat");
        assert_eq!(s.pending_len(), 3);
        assert_eq!(s.next_flush_at(t), Some(t + Duration::from_secs(1)));
        assert!(s.drain(t + Duration::from_millis(500), false).is_empty());
        assert_eq!(s.drain(t + Duration::from_secs(1), false).len(), 3);
        assert_eq!(s.pending_len(), 0);
    }

    #[test]
    fn queue_coalesces_to_latest_value() {
        let mut s = sender(1);
        let t = Instant::now();
        s.offer(&[msg("/x", OscValue::Int(0), Role::Value)], t, None, false);
        s.drain(t, false); // uses the only slot
        for i in 1..10 {
            s.offer(&[msg("/hr", OscValue::Int(i), Role::Value)], t, None, false);
        }
        assert_eq!(s.pending_len(), 1);
        assert_eq!(s.stats.coalesced, 8);
        let out = s.drain(t + Duration::from_secs(1), false);
        assert_eq!(out[0].log.value, "9");
    }

    #[test]
    fn force_and_invalidate_resend() {
        let mut s = sender(20);
        let t = Instant::now();
        let v = vec![msg("/a", OscValue::Int(1), Role::Value)];
        s.offer(&v, t, None, false);
        s.drain(t, false);
        s.offer(&v, t, None, true);
        assert_eq!(s.drain(t, false).len(), 1);
        s.invalidate();
        s.offer(&v, t, None, false);
        assert_eq!(s.drain(t, false).len(), 1);
    }

    #[test]
    fn no_dest_sends_nothing() {
        let mut s = TargetSender::new(OscTarget::default(), None, 20);
        let t = Instant::now();
        s.offer(&[msg("/a", OscValue::Int(1), Role::Value)], t, None, false);
        assert!(s.drain(t, false).is_empty());
        assert_eq!(s.next_flush_at(t), None);
    }

    #[test]
    fn chatbox_respects_min_interval() {
        let mut s = sender(20);
        let t = Instant::now();
        let iv = Duration::from_secs(2);
        assert!(s.chatbox("70 bpm", true, iv, t, false).is_some());
        assert!(s
            .chatbox("71 bpm", true, iv, t + Duration::from_millis(1999), false)
            .is_none());
        let o = s
            .chatbox("71 bpm", true, iv, t + Duration::from_secs(2), false)
            .unwrap();
        let (_, pkt) = rosc::decoder::decode_udp(&o.bytes).unwrap();
        match pkt {
            rosc::OscPacket::Message(m) => {
                assert_eq!(m.addr, "/chatbox/input");
                assert_eq!(
                    m.args,
                    vec![
                        rosc::OscType::String("71 bpm".into()),
                        rosc::OscType::Bool(true),
                        rosc::OscType::Bool(false)
                    ]
                );
            }
            _ => panic!(),
        }
        // unchanged text is not resent until 20 s pass
        assert!(s
            .chatbox("71 bpm", true, iv, t + Duration::from_secs(10), false)
            .is_none());
        assert!(s
            .chatbox("71 bpm", true, iv, t + Duration::from_secs(23), false)
            .is_some());
    }

    #[test]
    fn chatbox_with_sound() {
        let mut s = sender(20);
        let o = s
            .chatbox("hi", false, Duration::from_secs(2), Instant::now(), false)
            .unwrap();
        let (_, pkt) = rosc::decoder::decode_udp(&o.bytes).unwrap();
        let rosc::OscPacket::Message(m) = pkt else {
            panic!()
        };
        assert_eq!(m.args[2], rosc::OscType::Bool(true));
    }

    #[test]
    fn log_ring_buffer() {
        let mut l = OscLog::new(50);
        for i in 0..60 {
            l.push(OscLogEntry {
                seq: 0,
                t_ms: i,
                target: "t".into(),
                address: "/a".into(),
                value: i.to_string(),
                kind: "int".into(),
                test: false,
            });
        }
        let e = l.entries();
        assert_eq!(e.len(), 50);
        assert_eq!(e[0].value, "10");
        assert_eq!(e[49].seq, 60);
    }
}
