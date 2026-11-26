//! Saved session history: recording, local storage, retention and CSV export.
//!
//! A session starts with the first valid reading and ends once the source has been quiet for
//! [`QUIET_END_MS`], or on Reset session or quit. About one sample per second is kept (displayed
//! BPM, raw BPM and any RR intervals that arrived since the last sample), plus every alert that
//! fired. Nothing here touches the network: sessions live only in the app data folder.
//!
//! Storage is one small text file per session (`<id>.kts`), appended line by line so a crash
//! loses at most the line being written, plus a summary sidecar (`<id>.json`) written when the
//! session ends. Files without a sidecar (the app crashed) are finished on the next start.
//!
//! ```text
//! #{"v":1,"id":"1764100000000","start_ms":1764100000000,"source":"ble",...}
//! 0 72.4 73 812,820        offset ms, displayed bpm, raw bpm, RR ms (optional)
//! !61000 high 165 160      alert: offset ms, kind, bpm, threshold
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::alerts::AlertKind;
use crate::error::{KtError, KtResult};
use crate::processing::ZoneConfig;
use crate::source::SourceKind;

/// A session ends after the source has delivered nothing valid for this long.
pub const QUIET_END_MS: u64 = 3 * 60 * 1000;
/// Roughly one stored sample per second.
pub const SAMPLE_INTERVAL_MS: u64 = 1000;
/// Sessions shorter than this (a strap put on and taken off) are not kept.
pub const MIN_SESSION_MS: u64 = 30 * 1000;
/// A gap longer than this between samples is not counted towards zone time.
const MAX_SAMPLE_SPAN_MS: u64 = 5000;
/// HRV is computed over this trailing window...
const HRV_WINDOW_MS: u64 = 60 * 1000;
/// ...every this often.
const HRV_STEP_MS: u64 = 15 * 1000;
const FORMAT_VERSION: u32 = 1;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

// ---------- settings ----------

/// How long saved sessions are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    Days30,
    #[default]
    Days90,
    Days365,
    Forever,
}

impl Retention {
    pub fn max_age_ms(self) -> Option<u64> {
        match self {
            Retention::Days30 => Some(30 * DAY_MS),
            Retention::Days90 => Some(90 * DAY_MS),
            Retention::Days365 => Some(365 * DAY_MS),
            Retention::Forever => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// Save sessions at all.
    pub enabled: bool,
    pub retention: Retention,
    /// Oldest sessions are removed once all sessions together take more than this.
    pub max_mb: u32,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention: Retention::default(),
            max_mb: 200,
        }
    }
}

impl HistoryConfig {
    pub fn sanitize(&mut self) {
        self.max_mb = self.max_mb.clamp(10, 10_000);
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_mb as u64 * 1024 * 1024
    }
}

// ---------- data ----------

/// Written as the first line of a session file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub v: u32,
    pub id: String,
    pub start_ms: u64,
    pub source: SourceKind,
    /// Device or service label, if known.
    pub device: Option<String>,
    /// Zone settings at the start, so zone times don't change when settings do later.
    pub zones: ZoneConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSample {
    pub t_ms: u64,
    pub bpm: f32,
    pub raw: u16,
    /// RR intervals (ms) received since the previous sample.
    pub rr_ms: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAlert {
    pub t_ms: u64,
    pub kind: AlertKind,
    pub bpm: u16,
    pub threshold: u16,
}

/// One row of the History list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub start_ms: u64,
    /// Time of the last valid reading.
    pub end_ms: u64,
    pub source: SourceKind,
    pub device: Option<String>,
    pub min_bpm: u16,
    pub avg_bpm: u16,
    pub max_bpm: u16,
    pub samples: u32,
    pub alerts: u32,
    pub has_rr: bool,
    /// Still being recorded.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub bytes: u64,
}

impl SessionSummary {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct HrvPoint {
    pub t_ms: u64,
    pub rmssd_ms: f32,
}

/// A whole session for the detail view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Session {
    pub summary: SessionSummary,
    pub zones: ZoneConfig,
    pub max_hr: u16,
    pub samples: Vec<SessionSample>,
    pub alerts: Vec<SessionAlert>,
    /// Seconds spent in zones 0..=5.
    pub zone_secs: [u64; 6],
    /// RMSSD over time; empty when the source sends no RR intervals.
    pub hrv: Vec<HrvPoint>,
}

// ---------- recorder (pure state machine) ----------

/// What the recorder asks the writer to do.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordOp {
    Begin(Box<SessionMeta>),
    Sample(SessionSample),
    Alert(SessionAlert),
    /// Finish the session: write its summary.
    End,
    /// The session was too short to keep: delete it.
    Discard,
}

#[derive(Debug)]
struct OpenSession {
    start_ms: u64,
    last_valid_ms: u64,
    last_written_ms: Option<u64>,
    pending_rr: Vec<u16>,
}

/// Decides when sessions start and end and which samples are kept. It does no I/O, so the
/// rules are unit tested with plain timestamps.
#[derive(Debug, Default)]
pub struct Recorder {
    open: Option<OpenSession>,
}

impl Recorder {
    pub fn is_recording(&self) -> bool {
        self.open.is_some()
    }

    /// A valid reading arrived at `t_ms`. `meta` describes the source if a session starts.
    pub fn sample(
        &mut self,
        t_ms: u64,
        bpm: f64,
        raw: u16,
        rr_ms: &[f64],
        meta: impl FnOnce(u64) -> SessionMeta,
    ) -> Vec<RecordOp> {
        let mut ops = self.tick(t_ms);
        let open = match &mut self.open {
            Some(o) => o,
            None => {
                ops.push(RecordOp::Begin(Box::new(meta(t_ms))));
                self.open.insert(OpenSession {
                    start_ms: t_ms,
                    last_valid_ms: t_ms,
                    last_written_ms: None,
                    pending_rr: Vec::new(),
                })
            }
        };
        open.last_valid_ms = t_ms;
        open.pending_rr.extend(
            rr_ms
                .iter()
                .filter(|r| r.is_finite() && **r >= 1.0)
                .map(|r| r.round().min(u16::MAX as f64) as u16),
        );
        let due = open
            .last_written_ms
            .is_none_or(|w| t_ms.saturating_sub(w) >= SAMPLE_INTERVAL_MS);
        if due {
            open.last_written_ms = Some(t_ms);
            ops.push(RecordOp::Sample(SessionSample {
                t_ms,
                bpm: (bpm * 10.0).round() as f32 / 10.0,
                raw,
                rr_ms: std::mem::take(&mut open.pending_rr),
            }));
        }
        ops
    }

    /// A real (not test) alert fired. Ignored when no session is open.
    pub fn alert(&mut self, t_ms: u64, kind: AlertKind, bpm: u16, threshold: u16) -> Vec<RecordOp> {
        if self.open.is_none() {
            return Vec::new();
        }
        vec![RecordOp::Alert(SessionAlert {
            t_ms,
            kind,
            bpm,
            threshold,
        })]
    }

    /// Called periodically; ends the session once the source has been quiet long enough.
    pub fn tick(&mut self, t_ms: u64) -> Vec<RecordOp> {
        match &self.open {
            Some(o) if t_ms.saturating_sub(o.last_valid_ms) >= QUIET_END_MS => self.end(),
            _ => Vec::new(),
        }
    }

    /// Ends the session now (Reset session, quit, recording turned off).
    pub fn end(&mut self) -> Vec<RecordOp> {
        let Some(o) = self.open.take() else {
            return Vec::new();
        };
        if o.last_valid_ms.saturating_sub(o.start_ms) < MIN_SESSION_MS {
            vec![RecordOp::Discard]
        } else {
            vec![RecordOp::End]
        }
    }
}

// ---------- file format ----------

fn sample_line(start_ms: u64, s: &SessionSample) -> String {
    let mut line = format!("{} {:.1} {}", s.t_ms.saturating_sub(start_ms), s.bpm, s.raw);
    if !s.rr_ms.is_empty() {
        line.push(' ');
        let rr: Vec<String> = s.rr_ms.iter().map(|r| r.to_string()).collect();
        line.push_str(&rr.join(","));
    }
    line.push('\n');
    line
}

fn alert_line(start_ms: u64, a: &SessionAlert) -> String {
    let kind = match a.kind {
        AlertKind::High => "high",
        AlertKind::Low => "low",
    };
    format!(
        "!{} {} {} {}\n",
        a.t_ms.saturating_sub(start_ms),
        kind,
        a.bpm,
        a.threshold
    )
}

fn parse_sample(start_ms: u64, line: &str) -> Option<SessionSample> {
    let mut it = line.split_ascii_whitespace();
    let dt: u64 = it.next()?.parse().ok()?;
    let bpm: f32 = it.next()?.parse().ok()?;
    let raw: u16 = it.next()?.parse().ok()?;
    let rr_ms = match it.next() {
        Some(rr) => rr
            .split(',')
            .map(|r| r.parse::<u16>().ok())
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
    if !bpm.is_finite() {
        return None;
    }
    Some(SessionSample {
        t_ms: start_ms + dt,
        bpm,
        raw,
        rr_ms,
    })
}

fn parse_alert(start_ms: u64, line: &str) -> Option<SessionAlert> {
    let mut it = line.split_ascii_whitespace();
    let dt: u64 = it.next()?.parse().ok()?;
    let kind = match it.next()? {
        "high" => AlertKind::High,
        "low" => AlertKind::Low,
        _ => return None,
    };
    Some(SessionAlert {
        t_ms: start_ms + dt,
        kind,
        bpm: it.next()?.parse().ok()?,
        threshold: it.next()?.parse().ok()?,
    })
}

struct Parsed {
    meta: SessionMeta,
    samples: Vec<SessionSample>,
    alerts: Vec<SessionAlert>,
}

/// Reads a session file. Lines that don't parse (a crash mid-write) are skipped.
fn parse_session(text: &str) -> KtResult<Parsed> {
    let mut lines = text.lines();
    let header = lines
        .next()
        .and_then(|l| l.strip_prefix('#'))
        .ok_or_else(|| KtError::Io("not a KittyTooth session file".into()))?;
    let meta: SessionMeta =
        serde_json::from_str(header).map_err(|e| KtError::Io(format!("unreadable session header: {e}")))?;
    let mut samples = Vec::new();
    let mut alerts = Vec::new();
    for line in lines {
        if let Some(a) = line.strip_prefix('!') {
            alerts.extend(parse_alert(meta.start_ms, a));
        } else {
            samples.extend(parse_sample(meta.start_ms, line));
        }
    }
    Ok(Parsed {
        meta,
        samples,
        alerts,
    })
}

fn summarize(p: &Parsed) -> SessionSummary {
    let n = p.samples.len();
    let (mut min, mut max, mut sum) = (u16::MAX, 0u16, 0u64);
    for s in &p.samples {
        min = min.min(s.raw);
        max = max.max(s.raw);
        sum += s.raw as u64;
    }
    SessionSummary {
        id: p.meta.id.clone(),
        start_ms: p.meta.start_ms,
        end_ms: p.samples.last().map_or(p.meta.start_ms, |s| s.t_ms),
        source: p.meta.source,
        device: p.meta.device.clone(),
        min_bpm: if n == 0 { 0 } else { min },
        avg_bpm: if n == 0 {
            0
        } else {
            (sum as f64 / n as f64).round() as u16
        },
        max_bpm: max,
        samples: n as u32,
        alerts: p.alerts.len() as u32,
        has_rr: p.samples.iter().any(|s| !s.rr_ms.is_empty()),
        active: false,
        bytes: 0,
    }
}

/// Seconds in zones 0..=5. Each sample counts until the next one, but no more than a few
/// seconds, so a dropout in the middle isn't credited to any zone.
pub fn zone_seconds(samples: &[SessionSample], zones: &ZoneConfig) -> [u64; 6] {
    let mut ms = [0u64; 6];
    for (i, s) in samples.iter().enumerate() {
        let span = samples
            .get(i + 1)
            .map_or(SAMPLE_INTERVAL_MS, |n| n.t_ms.saturating_sub(s.t_ms))
            .min(MAX_SAMPLE_SPAN_MS);
        let z = zones.zone(s.bpm.round() as u16).min(5) as usize;
        ms[z] += span;
    }
    ms.map(|v| (v + 500) / 1000)
}

/// RMSSD over a trailing window, stepped through the session. Implausible intervals and
/// beat-to-beat jumps (missed or extra beats) are dropped first.
pub fn hrv_series(samples: &[SessionSample]) -> Vec<HrvPoint> {
    // Spread each sample's intervals evenly back over the second before it.
    let mut beats: Vec<(u64, f64)> = Vec::new();
    for s in samples {
        let n = s.rr_ms.len() as u64;
        for (k, &rr) in s.rr_ms.iter().enumerate() {
            let back = (n - 1 - k as u64) * SAMPLE_INTERVAL_MS / n.max(1);
            if (300..=2000).contains(&rr) {
                beats.push((s.t_ms.saturating_sub(back), rr as f64));
            }
        }
    }
    let (Some(first), Some(last)) = (beats.first(), beats.last()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut t = first.0 + HRV_WINDOW_MS;
    let mut lo = 0;
    while t < last.0 + HRV_STEP_MS {
        while lo < beats.len() && beats[lo].0 + HRV_WINDOW_MS <= t {
            lo += 1;
        }
        let hi = beats.partition_point(|b| b.0 <= t);
        let win = &beats[lo..hi];
        let diffs: Vec<f64> = win
            .windows(2)
            .map(|w| w[1].1 - w[0].1)
            .zip(win.iter())
            .filter(|(d, prev)| d.abs() <= 0.2 * prev.1)
            .map(|(d, _)| d)
            .collect();
        if diffs.len() >= 10 {
            let rmssd = (diffs.iter().map(|d| d * d).sum::<f64>() / diffs.len() as f64).sqrt();
            out.push(HrvPoint {
                t_ms: t,
                rmssd_ms: (rmssd * 10.0).round() as f32 / 10.0,
            });
        }
        t += HRV_STEP_MS;
    }
    out
}

// ---------- CSV ----------

/// Unix ms as an ISO 8601 UTC timestamp, e.g. `2026-11-26T14:03:05.123Z`.
pub fn iso_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60,
        ms % 1000
    )
}

/// CSV with one row per stored sample. RR intervals in a row are separated by spaces; an alert
/// is shown on the first row at or after the moment it fired.
pub fn to_csv(session: &Session) -> String {
    let mut out = String::from("timestamp,elapsed_s,bpm,raw,rr_ms,zone,alert\n");
    let start = session.summary.start_ms;
    let mut alerts = session.alerts.iter().peekable();
    for s in &session.samples {
        let mut fired = Vec::new();
        while let Some(a) = alerts.next_if(|a| a.t_ms <= s.t_ms) {
            fired.push(format!(
                "{} {}",
                match a.kind {
                    AlertKind::High => "high",
                    AlertKind::Low => "low",
                },
                a.bpm
            ));
        }
        let rr: Vec<String> = s.rr_ms.iter().map(|r| r.to_string()).collect();
        out.push_str(&format!(
            "{},{:.1},{:.1},{},{},{},{}\n",
            iso_utc(s.t_ms),
            s.t_ms.saturating_sub(start) as f64 / 1000.0,
            s.bpm,
            s.raw,
            rr.join(" "),
            session.zones.zone(s.bpm.round() as u16),
            fired.join(" ")
        ));
    }
    out
}

/// A safe file name for an exported session, e.g. `kittytooth-2026-11-26-1403.csv`.
pub fn csv_file_name(start_ms: u64) -> String {
    let iso = iso_utc(start_ms);
    format!("kittytooth-{}-{}{}.csv", &iso[..10], &iso[11..13], &iso[14..16])
}

// ---------- store ----------

/// Session ids are the start time in Unix ms; anything else is refused (no path tricks).
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// Read access to the sessions folder. Cheap to clone.
#[derive(Debug, Clone)]
pub struct SessionStore {
    dir: PathBuf,
    active: Arc<Mutex<Option<String>>>,
}

impl SessionStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            active: Arc::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn data_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.kts"))
    }

    fn summary_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn check_id(id: &str) -> KtResult<()> {
        if valid_id(id) {
            Ok(())
        } else {
            Err(KtError::InvalidInput("unknown session".into()))
        }
    }

    /// The session being recorded right now, if any.
    pub fn active_id(&self) -> Option<String> {
        self.active.lock().ok().and_then(|a| a.clone())
    }

    fn set_active(&self, id: Option<String>) {
        if let Ok(mut a) = self.active.lock() {
            *a = id;
        }
    }

    fn ids(&self) -> Vec<String> {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        rd.filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            let id = name.strip_suffix(".kts")?;
            valid_id(id).then(|| id.to_string())
        })
        .collect()
    }

    fn read_parsed(&self, id: &str) -> KtResult<Parsed> {
        Self::check_id(id)?;
        let text = fs::read_to_string(self.data_path(id)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => KtError::InvalidInput("that session no longer exists".into()),
            _ => e.into(),
        })?;
        parse_session(&text)
    }

    fn bytes(&self, id: &str) -> u64 {
        let size = |p: PathBuf| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        size(self.data_path(id)) + size(self.summary_path(id))
    }

    fn summary(&self, id: &str) -> KtResult<SessionSummary> {
        let active = self.active_id().as_deref() == Some(id);
        let cached = if active {
            None
        } else {
            fs::read_to_string(self.summary_path(id))
                .ok()
                .and_then(|t| serde_json::from_str::<SessionSummary>(&t).ok())
        };
        let mut s = match cached {
            Some(s) => s,
            None => summarize(&self.read_parsed(id)?),
        };
        s.active = active;
        s.bytes = self.bytes(id);
        Ok(s)
    }

    /// All sessions, newest first.
    pub fn list(&self) -> Vec<SessionSummary> {
        let mut out: Vec<SessionSummary> = self
            .ids()
            .iter()
            .filter_map(|id| match self.summary(id) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!(id, error = %e, "skipping unreadable session");
                    None
                }
            })
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.start_ms));
        out
    }

    pub fn load(&self, id: &str) -> KtResult<Session> {
        let parsed = self.read_parsed(id)?;
        let mut summary = summarize(&parsed);
        summary.active = self.active_id().as_deref() == Some(id);
        summary.bytes = self.bytes(id);
        let zones = parsed.meta.zones.clone();
        Ok(Session {
            zone_secs: zone_seconds(&parsed.samples, &zones),
            hrv: hrv_series(&parsed.samples),
            max_hr: zones.max_hr(),
            zones,
            summary,
            samples: parsed.samples,
            alerts: parsed.alerts,
        })
    }

    pub fn csv(&self, id: &str) -> KtResult<String> {
        Ok(to_csv(&self.load(id)?))
    }

    fn remove(&self, id: &str) {
        let _ = fs::remove_file(self.summary_path(id));
        let _ = fs::remove_file(self.data_path(id));
    }

    pub fn delete(&self, id: &str) -> KtResult<()> {
        Self::check_id(id)?;
        if self.active_id().as_deref() == Some(id) {
            return Err(KtError::InvalidInput(
                "this session is still recording; reset the session first".into(),
            ));
        }
        self.remove(id);
        if self.data_path(id).exists() {
            return Err(KtError::Io("the session file could not be deleted".into()));
        }
        Ok(())
    }

    /// Writes the summary sidecar (marks the session finished), or deletes a session that is
    /// too short to keep. Returns whether it was kept.
    fn finish(&self, id: &str) -> bool {
        let Ok(parsed) = self.read_parsed(id) else {
            self.remove(id);
            return false;
        };
        let s = summarize(&parsed);
        if s.samples == 0 || s.duration_ms() < MIN_SESSION_MS {
            self.remove(id);
            return false;
        }
        if let Ok(json) = serde_json::to_string(&s) {
            if let Err(e) = fs::write(self.summary_path(id), json) {
                tracing::warn!(id, error = %e, "could not write the session summary");
            }
        }
        true
    }

    /// Finishes sessions left open by a crash.
    pub fn recover(&self) {
        let active = self.active_id();
        for id in self.ids() {
            if Some(&id) != active.as_ref() && !self.summary_path(&id).exists() {
                let kept = self.finish(&id);
                tracing::info!(id, kept, "finished a session left open");
            }
        }
    }

    /// Removes sessions older than the retention period, then the oldest ones until all fit in
    /// the size cap. The session being recorded is never removed. Returns removed ids.
    pub fn prune(&self, cfg: &HistoryConfig, now_ms: u64) -> Vec<String> {
        self.prune_limits(cfg.retention.max_age_ms(), cfg.max_bytes(), now_ms)
    }

    fn prune_limits(&self, max_age_ms: Option<u64>, max_bytes: u64, now_ms: u64) -> Vec<String> {
        let active = self.active_id();
        let mut all = self.list();
        all.sort_by_key(|s| s.start_ms); // oldest first
        let mut removed = Vec::new();
        let mut total: u64 = all.iter().map(|s| s.bytes).sum();
        for s in &all {
            if Some(&s.id) == active.as_ref() {
                continue;
            }
            let too_old = max_age_ms.is_some_and(|age| s.end_ms.saturating_add(age) < now_ms);
            if too_old || total > max_bytes {
                self.remove(&s.id);
                total = total.saturating_sub(s.bytes);
                removed.push(s.id.clone());
            }
        }
        if !removed.is_empty() {
            tracing::info!(count = removed.len(), "old sessions removed");
        }
        removed
    }
}

// ---------- writer ----------

/// Applies [`RecordOp`]s to the files of a [`SessionStore`].
pub struct SessionWriter {
    store: SessionStore,
    file: Option<(String, u64, File)>,
}

impl SessionWriter {
    pub fn new(store: SessionStore) -> Self {
        Self { store, file: None }
    }

    pub fn apply(&mut self, op: RecordOp) -> KtResult<()> {
        match op {
            RecordOp::Begin(meta) => {
                self.close(true);
                fs::create_dir_all(&self.store.dir)?;
                let mut meta = *meta;
                meta.v = FORMAT_VERSION;
                // Ids must be unique; nudge if two sessions start in the same millisecond.
                while self.store.data_path(&meta.id).exists() {
                    meta.start_ms += 1;
                    meta.id = meta.start_ms.to_string();
                }
                let mut f = OpenOptions::new()
                    .create_new(true)
                    .append(true)
                    .open(self.store.data_path(&meta.id))?;
                let header = serde_json::to_string(&meta).map_err(|e| KtError::Io(e.to_string()))?;
                f.write_all(format!("#{header}\n").as_bytes())?;
                self.store.set_active(Some(meta.id.clone()));
                self.file = Some((meta.id, meta.start_ms, f));
            }
            RecordOp::Sample(s) => self.write(|start| sample_line(start, &s))?,
            RecordOp::Alert(a) => self.write(|start| alert_line(start, &a))?,
            RecordOp::End => self.close(true),
            RecordOp::Discard => self.close(false),
        }
        Ok(())
    }

    fn write(&mut self, line: impl FnOnce(u64) -> String) -> KtResult<()> {
        if let Some((_, start, f)) = &mut self.file {
            f.write_all(line(*start).as_bytes())?;
        }
        Ok(())
    }

    fn close(&mut self, keep: bool) {
        let Some((id, _, f)) = self.file.take() else {
            return;
        };
        let _ = f.sync_all();
        drop(f);
        self.store.set_active(None);
        if keep {
            self.store.finish(&id);
        } else {
            self.store.remove(&id);
        }
    }
}

/// Messages for the background writer thread.
pub enum WriterMsg {
    Op(RecordOp),
    Prune(HistoryConfig, u64),
    /// Replies once everything before it is on disk.
    Sync(tokio::sync::oneshot::Sender<()>),
}

/// Runs a [`SessionWriter`] on its own thread so disk writes never touch the OSC hot path.
/// `changed` is called after a session begins or ends, or sessions were pruned.
pub fn spawn_writer(store: SessionStore, changed: impl Fn() + Send + 'static) -> mpsc::Sender<WriterMsg> {
    let (tx, rx) = mpsc::channel::<WriterMsg>();
    let spawned = std::thread::Builder::new()
        .name("kittytooth-history".into())
        .spawn(move || {
            store.recover();
            let mut w = SessionWriter::new(store.clone());
            for msg in rx {
                match msg {
                    WriterMsg::Op(op) => {
                        let notify = matches!(op, RecordOp::Begin(_) | RecordOp::End | RecordOp::Discard);
                        if let Err(e) = w.apply(op) {
                            tracing::warn!(error = %e, "could not save session history");
                        }
                        if notify {
                            changed();
                        }
                    }
                    WriterMsg::Prune(cfg, now) => {
                        if !store.prune(&cfg, now).is_empty() {
                            changed();
                        }
                    }
                    WriterMsg::Sync(done) => {
                        let _ = done.send(());
                    }
                }
            }
            w.close(true);
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the history writer");
    }
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(t: u64) -> SessionMeta {
        SessionMeta {
            v: 1,
            id: t.to_string(),
            start_ms: t,
            source: SourceKind::Ble,
            device: Some("Polar H10".into()),
            zones: ZoneConfig::default(),
        }
    }

    fn feed(r: &mut Recorder, t: u64, bpm: u16) -> Vec<RecordOp> {
        r.sample(t, bpm as f64, bpm, &[], meta)
    }

    fn temp_store(name: &str) -> SessionStore {
        let d = std::env::temp_dir().join(format!(
            "kittytooth-history-{name}-{}-{}",
            std::process::id(),
            crate::osc::sender::epoch_ms()
        ));
        let _ = fs::remove_dir_all(&d);
        SessionStore::new(d)
    }

    /// Records one sample per second from `start` for `secs` seconds.
    fn record(w: &mut SessionWriter, r: &mut Recorder, start: u64, secs: u64, bpm: u16) {
        for i in 0..secs {
            let rr = [60_000.0 / bpm as f64 + (i % 3) as f64 * 10.0];
            for op in r.sample(start + i * 1000, bpm as f64, bpm, &rr, meta) {
                w.apply(op).unwrap();
            }
        }
    }

    #[test]
    fn first_valid_sample_starts_a_session() {
        let mut r = Recorder::default();
        assert!(r.tick(0).is_empty());
        let ops = feed(&mut r, 1_000, 70);
        assert!(matches!(&ops[0], RecordOp::Begin(m) if m.start_ms == 1_000));
        assert!(matches!(&ops[1], RecordOp::Sample(s) if s.raw == 70));
        assert!(r.is_recording());
    }

    #[test]
    fn keeps_about_one_sample_per_second_and_collects_rr() {
        let mut r = Recorder::default();
        let mut samples = Vec::new();
        for i in 0..20u64 {
            // 4 readings a second, each with one RR interval.
            for op in r.sample(i * 250, 60.0, 60, &[1000.0], meta) {
                if let RecordOp::Sample(s) = op {
                    samples.push(s);
                }
            }
        }
        assert_eq!(samples.len(), 5);
        assert_eq!(samples[0].rr_ms, vec![1000]);
        // Later samples carry the intervals that arrived since the previous one.
        assert_eq!(samples[1].rr_ms.len(), 4);
        assert_eq!(samples[1].t_ms, 1000);
    }

    #[test]
    fn quiet_source_ends_the_session() {
        let mut r = Recorder::default();
        for i in 0..60 {
            feed(&mut r, i * 1000, 80);
        }
        assert!(r.tick(59_000 + QUIET_END_MS - 1).is_empty());
        assert_eq!(r.tick(59_000 + QUIET_END_MS), vec![RecordOp::End]);
        assert!(!r.is_recording());
    }

    #[test]
    fn a_short_gap_continues_the_session_and_a_long_one_starts_another() {
        let mut r = Recorder::default();
        for i in 0..40 {
            feed(&mut r, i * 1000, 80);
        }
        let ops = feed(&mut r, 39_000 + 60_000, 80);
        assert!(!ops.iter().any(|o| matches!(o, RecordOp::Begin(_))));
        let late = 99_000 + QUIET_END_MS + 5_000;
        let ops = feed(&mut r, late, 80);
        assert_eq!(ops[0], RecordOp::End);
        assert!(matches!(&ops[1], RecordOp::Begin(m) if m.start_ms == late));
    }

    #[test]
    fn short_sessions_are_discarded_and_reset_ends() {
        let mut r = Recorder::default();
        for i in 0..10 {
            feed(&mut r, i * 1000, 80);
        }
        assert_eq!(r.end(), vec![RecordOp::Discard]);
        assert!(r.end().is_empty());
        for i in 0..40 {
            feed(&mut r, 100_000 + i * 1000, 80);
        }
        assert_eq!(r.end(), vec![RecordOp::End]);
    }

    #[test]
    fn alerts_only_recorded_during_a_session() {
        let mut r = Recorder::default();
        assert!(r.alert(0, AlertKind::High, 170, 160).is_empty());
        feed(&mut r, 0, 165);
        assert_eq!(r.alert(500, AlertKind::High, 170, 160).len(), 1);
    }

    #[test]
    fn lines_roundtrip_and_bad_lines_are_skipped() {
        let s = SessionSample {
            t_ms: 1_500,
            bpm: 72.4,
            raw: 73,
            rr_ms: vec![812, 820],
        };
        let line = sample_line(1_000, &s);
        assert_eq!(line, "500 72.4 73 812,820\n");
        assert_eq!(parse_sample(1_000, line.trim()), Some(s));
        let text = format!(
            "#{}\n0 70.0 70\n!1000 high 170 160\n2000 71.0 7",
            serde_json::to_string(&meta(5)).unwrap()
        );
        let p = parse_session(&text).unwrap();
        assert_eq!(p.samples.len(), 2); // the truncated last line still parses as raw 7
        assert_eq!(p.alerts[0].t_ms, 1_005);
        assert!(parse_session("garbage").is_err());
        assert_eq!(parse_sample(0, "12 x 70"), None);
    }

    #[test]
    fn writer_store_roundtrip() {
        let store = temp_store("roundtrip");
        let mut w = SessionWriter::new(store.clone());
        let mut r = Recorder::default();
        let start = 1_764_000_000_000;
        record(&mut w, &mut r, start, 90, 75);
        for op in r.alert(start + 10_000, AlertKind::High, 170, 160) {
            w.apply(op).unwrap();
        }
        assert_eq!(store.active_id(), Some(start.to_string()));
        let live = store.list();
        assert!(live[0].active);
        assert!(store.delete(&live[0].id).is_err(), "can't delete while recording");
        for op in r.end() {
            w.apply(op).unwrap();
        }
        let list = store.list();
        assert_eq!(list.len(), 1);
        let s = &list[0];
        assert!(!s.active);
        assert_eq!((s.min_bpm, s.avg_bpm, s.max_bpm), (75, 75, 75));
        assert_eq!(s.samples, 90);
        assert_eq!(s.alerts, 1);
        assert!(s.has_rr);
        assert_eq!(s.device.as_deref(), Some("Polar H10"));
        assert_eq!(s.duration_ms(), 89_000);
        let full = store.load(&s.id).unwrap();
        assert_eq!(full.samples.len(), 90);
        assert_eq!(full.zone_secs.iter().sum::<u64>(), 90);
        assert!(!full.hrv.is_empty());
        store.delete(&s.id).unwrap();
        assert!(store.list().is_empty());
        assert!(store.delete("../config").is_err());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn discarded_sessions_leave_no_files() {
        let store = temp_store("discard");
        let mut w = SessionWriter::new(store.clone());
        let mut r = Recorder::default();
        record(&mut w, &mut r, 1_000_000, 5, 70);
        for op in r.end() {
            w.apply(op).unwrap();
        }
        assert!(store.list().is_empty());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn crash_left_sessions_are_recovered() {
        let store = temp_store("recover");
        // "Crash": the writers are dropped without ending their sessions.
        for (start, secs) in [(2_000_000, 45), (9_000_000, 3)] {
            let mut w = SessionWriter::new(store.clone());
            let mut r = Recorder::default();
            record(&mut w, &mut r, start, secs, 70);
            assert!(!store.summary_path(&start.to_string()).exists());
        }
        store.set_active(None);
        store.recover();
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "2000000");
        assert!(store.summary_path("2000000").exists());
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn retention_prunes_old_sessions_but_not_the_active_one() {
        let store = temp_store("prune");
        let mut w = SessionWriter::new(store.clone());
        let day = DAY_MS;
        let now = 400 * day;
        for (i, start) in [now - 100 * day, now - 40 * day, now - 10 * day, now - day]
            .into_iter()
            .enumerate()
        {
            let mut r = Recorder::default();
            record(&mut w, &mut r, start + i as u64, 40, 70);
            for op in r.end() {
                w.apply(op).unwrap();
            }
        }
        assert_eq!(store.list().len(), 4);
        let mut cfg = HistoryConfig {
            retention: Retention::Days90,
            ..Default::default()
        };
        assert_eq!(store.prune(&cfg, now).len(), 1);
        cfg.retention = Retention::Days30;
        assert_eq!(store.prune(&cfg, now).len(), 1);
        cfg.retention = Retention::Forever;
        assert!(store.prune(&cfg, now).is_empty());
        let left = store.list().len();
        // The active session is never pruned.
        let mut r = Recorder::default();
        record(&mut w, &mut r, now, 40, 70);
        let all = HistoryConfig {
            retention: Retention::Days30,
            ..Default::default()
        };
        assert_eq!(store.prune(&all, now + 31 * day).len(), left);
        assert_eq!(store.list().len(), 1);
        assert!(store.list()[0].active);
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn size_cap_removes_oldest() {
        let store = temp_store("cap");
        let mut w = SessionWriter::new(store.clone());
        for k in 0..3u64 {
            let mut r = Recorder::default();
            record(&mut w, &mut r, 10_000_000 * (k + 1), 40, 70);
            for op in r.end() {
                w.apply(op).unwrap();
            }
        }
        let sizes: u64 = store.list().iter().map(|s| s.bytes).sum();
        assert!(sizes > 0);
        let removed = store.prune_limits(None, sizes - 1, 0);
        assert_eq!(removed, vec!["10000000".to_string()]);
        assert_eq!(store.list().len(), 2);
        let _ = fs::remove_dir_all(store.dir());
    }

    #[test]
    fn zone_time_ignores_gaps() {
        let zones = ZoneConfig::default(); // max 190: zone 1 from 95, zone 3 from 133
        let mk = |t, bpm: f32| SessionSample {
            t_ms: t,
            bpm,
            raw: bpm as u16,
            rr_ms: vec![],
        };
        let samples = vec![mk(0, 100.0), mk(1000, 100.0), mk(2000, 140.0), mk(60_000, 140.0)];
        let z = zone_seconds(&samples, &zones);
        assert_eq!(z[1], 2);
        assert_eq!(z[3], 6); // 5 s cap for the gap + 1 s for the last sample
    }

    #[test]
    fn hrv_needs_rr_and_drops_artifacts() {
        let mk = |t, rr: Vec<u16>| SessionSample {
            t_ms: t,
            bpm: 60.0,
            raw: 60,
            rr_ms: rr,
        };
        assert!(hrv_series(&[mk(0, vec![]), mk(1000, vec![])]).is_empty());
        let samples: Vec<SessionSample> = (0..120u64)
            .map(|i| {
                let rr = if i == 50 {
                    2500
                } else if i % 2 == 0 {
                    1000
                } else {
                    1040
                };
                mk(i * 1000, vec![rr])
            })
            .collect();
        let hrv = hrv_series(&samples);
        assert!(!hrv.is_empty());
        for p in &hrv {
            assert!((p.rmssd_ms - 40.0).abs() < 0.5, "{p:?}");
        }
    }

    #[test]
    fn iso_timestamps() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_utc(1_795_701_785_123), "2026-11-26T14:03:05.123Z");
        assert_eq!(csv_file_name(1_795_701_785_123), "kittytooth-2026-11-26-1403.csv");
    }

    #[test]
    fn csv_format() {
        let zones = ZoneConfig::default();
        let start = 1_795_701_785_000;
        let session = Session {
            summary: SessionSummary {
                id: start.to_string(),
                start_ms: start,
                end_ms: start + 2000,
                source: SourceKind::Ble,
                device: None,
                min_bpm: 70,
                avg_bpm: 120,
                max_bpm: 171,
                samples: 3,
                alerts: 1,
                has_rr: true,
                active: false,
                bytes: 0,
            },
            max_hr: zones.max_hr(),
            zones,
            samples: vec![
                SessionSample {
                    t_ms: start,
                    bpm: 70.0,
                    raw: 70,
                    rr_ms: vec![857],
                },
                SessionSample {
                    t_ms: start + 1000,
                    bpm: 120.5,
                    raw: 121,
                    rr_ms: vec![500, 498],
                },
                SessionSample {
                    t_ms: start + 2000,
                    bpm: 171.0,
                    raw: 171,
                    rr_ms: vec![],
                },
            ],
            alerts: vec![SessionAlert {
                t_ms: start + 1500,
                kind: AlertKind::High,
                bpm: 171,
                threshold: 160,
            }],
            zone_secs: [0; 6],
            hrv: vec![],
        };
        let csv = to_csv(&session);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "timestamp,elapsed_s,bpm,raw,rr_ms,zone,alert");
        assert_eq!(lines[1], "2026-11-26T14:03:05.000Z,0.0,70.0,70,857,0,");
        assert_eq!(lines[2], "2026-11-26T14:03:06.000Z,1.0,120.5,121,500 498,2,");
        assert_eq!(lines[3], "2026-11-26T14:03:07.000Z,2.0,171.0,171,,5,high 171");
        assert_eq!(lines.len(), 4);
    }
}
