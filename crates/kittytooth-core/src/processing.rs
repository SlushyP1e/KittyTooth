//! Signal processing: validation, spike rejection, smoothing, session statistics, HRV and zones.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Lowest physiologically plausible reading accepted.
pub const MIN_VALID_BPM: u16 = 25;
/// Highest physiologically plausible reading accepted.
pub const MAX_VALID_BPM: u16 = 250;
/// Rolling window for RMSSD.
pub const HRV_WINDOW: Duration = Duration::from_secs(30);

/// Exponential moving average strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Smoothing {
    #[default]
    Off,
    Light,
    Medium,
}

impl Smoothing {
    /// EMA weight of the newest sample (1.0 = no smoothing).
    pub fn alpha(self) -> f64 {
        match self {
            Smoothing::Off => 1.0,
            Smoothing::Light => 0.5,
            Smoothing::Medium => 0.25,
        }
    }
}

/// Exponential moving average.
#[derive(Debug, Clone, Default)]
pub struct Ema {
    value: Option<f64>,
}

impl Ema {
    pub fn update(&mut self, sample: f64, alpha: f64) -> f64 {
        let v = match self.value {
            Some(prev) => alpha * sample + (1.0 - alpha) * prev,
            None => sample,
        };
        self.value = Some(v);
        v
    }

    pub fn value(&self) -> Option<f64> {
        self.value
    }

    pub fn reset(&mut self) {
        self.value = None;
    }
}

/// Why a sample was not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// Outside 25..=250 bpm.
    OutOfRange,
    /// Sensor reports no skin contact.
    NoContact,
    /// A sudden jump not (yet) confirmed by the next sample.
    SuspectedSpike,
}

/// Result of feeding a sample to the [`SpikeFilter`]/[`Processor`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SampleOutcome {
    Accepted { raw: u16, smoothed: f64 },
    Rejected(RejectReason),
}

/// Rejects single-sample spikes without delaying normal samples.
///
/// A sample that jumps more than `max(25 bpm, 20 %)` from the last accepted value is held
/// back. If the next sample confirms the new level it is accepted; if the next sample is
/// back near the old level, the held sample is discarded as a spike. After a gap of more than
/// [`SpikeFilter::RESET_AFTER`] any in-range value is accepted directly.
#[derive(Debug, Clone, Default)]
pub struct SpikeFilter {
    last: Option<(u16, Instant)>,
    pending: Option<u16>,
    rejected: u64,
}

impl SpikeFilter {
    pub const RESET_AFTER: Duration = Duration::from_secs(5);

    pub fn threshold(reference: u16) -> u16 {
        25u16.max((reference as f64 * 0.2).round() as u16)
    }

    /// Returns `Some(bpm)` if the sample should be used.
    pub fn push(&mut self, bpm: u16, now: Instant) -> Option<u16> {
        let last = match self.last {
            Some((v, t)) if now.saturating_duration_since(t) <= Self::RESET_AFTER => v,
            _ => {
                self.pending = None;
                self.last = Some((bpm, now));
                return Some(bpm);
            }
        };
        let thr = Self::threshold(last);
        if let Some(p) = self.pending.take() {
            if bpm.abs_diff(p) <= Self::threshold(p) {
                // The jump was real: new level confirmed by a second sample.
                self.last = Some((bpm, now));
                return Some(bpm);
            }
            // Pending sample was a lone spike.
            self.rejected += 1;
        }
        if bpm.abs_diff(last) <= thr {
            self.last = Some((bpm, now));
            Some(bpm)
        } else {
            self.pending = Some(bpm);
            None
        }
    }

    pub fn rejected_count(&self) -> u64 {
        self.rejected
    }

    pub fn reset(&mut self) {
        self.last = None;
        self.pending = None;
    }
}

/// Session min / average / max of accepted raw samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionStats {
    pub min: Option<u16>,
    pub max: Option<u16>,
    pub avg: Option<f64>,
    pub samples: u64,
}

impl SessionStats {
    pub fn push(&mut self, bpm: u16) {
        self.min = Some(self.min.map_or(bpm, |m| m.min(bpm)));
        self.max = Some(self.max.map_or(bpm, |m| m.max(bpm)));
        let n = self.samples as f64;
        self.avg = Some(self.avg.map_or(bpm as f64, |a| (a * n + bpm as f64) / (n + 1.0)));
        self.samples += 1;
    }
}

/// RMSSD over a rolling time window of RR intervals.
///
/// Artefacts are filtered: intervals outside 240-2400 ms (25-250 bpm) are dropped, as are
/// intervals deviating more than 25 % from the previous accepted interval (ectopic beats /
/// missed detections). After three consecutive deviations the reference resets so a genuine
/// rate change is not rejected forever. Differences are never taken across a gap in the
/// data (no RR for > 3 s), since those intervals are not successive beats.
#[derive(Debug, Clone, Default)]
pub struct Rmssd {
    rr: VecDeque<RrEntry>,
    segment: u64,
    last_arrival: Option<Instant>,
    consecutive_rejects: u8,
}

#[derive(Debug, Clone, Copy)]
struct RrEntry {
    at: Instant,
    ms: f64,
    segment: u64,
}

impl Rmssd {
    const GAP: Duration = Duration::from_secs(3);
    pub const MIN_INTERVALS: usize = 5;

    pub fn push(&mut self, rr_ms: f64, now: Instant) {
        if let Some(t) = self.last_arrival {
            if now.saturating_duration_since(t) > Self::GAP {
                self.segment += 1;
                self.consecutive_rejects = 0;
            }
        }
        self.last_arrival = Some(now);
        if !(240.0..=2400.0).contains(&rr_ms) {
            return;
        }
        if let Some(prev) = self.rr.back().filter(|p| p.segment == self.segment) {
            if (rr_ms - prev.ms).abs() > prev.ms * 0.25 {
                self.consecutive_rejects += 1;
                if self.consecutive_rejects < 3 {
                    return;
                }
                // Persistent change: start a new segment from here.
                self.segment += 1;
            }
        }
        self.consecutive_rejects = 0;
        self.rr.push_back(RrEntry {
            at: now,
            ms: rr_ms,
            segment: self.segment,
        });
        self.prune(now);
    }

    fn prune(&mut self, now: Instant) {
        while let Some(f) = self.rr.front() {
            if now.saturating_duration_since(f.at) > HRV_WINDOW {
                self.rr.pop_front();
            } else {
                break;
            }
        }
    }

    /// RMSSD in milliseconds, or `None` with too little data in the window.
    pub fn value(&mut self, now: Instant) -> Option<f64> {
        self.prune(now);
        if self.rr.len() < Self::MIN_INTERVALS {
            return None;
        }
        let mut sum = 0.0;
        let mut n = 0usize;
        let mut prev: Option<&RrEntry> = None;
        for e in &self.rr {
            if let Some(p) = prev {
                if p.segment == e.segment {
                    let d = e.ms - p.ms;
                    sum += d * d;
                    n += 1;
                }
            }
            prev = Some(e);
        }
        if n < Self::MIN_INTERVALS - 1 {
            return None;
        }
        Some((sum / n as f64).sqrt())
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Plain RMSSD of a sequence of successive RR intervals (ms). Exposed for tests/tools.
pub fn rmssd(rr_ms: &[f64]) -> Option<f64> {
    if rr_ms.len() < 2 {
        return None;
    }
    let sum: f64 = rr_ms.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    Some((sum / (rr_ms.len() - 1) as f64).sqrt())
}

/// Heart rate zone settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoneConfig {
    /// If set, max HR = 220 - age.
    pub age: Option<u8>,
    /// Max HR used when no age is set.
    pub manual_max_hr: u16,
    /// Lower bounds of zones 1..=5 as % of max HR. Below the first bound is zone 0.
    pub thresholds: [u8; 5],
}

impl Default for ZoneConfig {
    fn default() -> Self {
        Self {
            age: None,
            manual_max_hr: 190,
            thresholds: [50, 60, 70, 80, 90],
        }
    }
}

impl ZoneConfig {
    pub fn max_hr(&self) -> u16 {
        match self.age {
            Some(age) if (1..=120).contains(&age) => 220 - age as u16,
            _ => self.manual_max_hr.clamp(100, 250),
        }
    }

    /// Zone 0 (below zone 1) to 5.
    pub fn zone(&self, bpm: u16) -> u8 {
        let pct = bpm as f64 * 100.0 / self.max_hr() as f64;
        self.thresholds.iter().filter(|&&t| pct >= t as f64).count() as u8
    }

    /// Sorts and clamps thresholds into a valid ascending set.
    pub fn sanitize(&mut self) {
        for t in &mut self.thresholds {
            *t = (*t).clamp(1, 100);
        }
        self.thresholds.sort_unstable();
        self.manual_max_hr = self.manual_max_hr.clamp(100, 250);
        if let Some(a) = self.age {
            if !(1..=120).contains(&a) {
                self.age = None;
            }
        }
    }
}

/// Splits a BPM value into (ones, tens, hundreds) digits; values above 999 clamp to 999.
pub fn split_digits(bpm: u16) -> (u8, u8, u8) {
    let v = bpm.min(999);
    ((v % 10) as u8, ((v / 10) % 10) as u8, (v / 100) as u8)
}

/// Linear map of `v` from `[min, max]` to `[0, 1]`, clamped.
pub fn normalize(v: f64, min: f64, max: f64) -> f64 {
    if max <= min {
        return if v >= max { 1.0 } else { 0.0 };
    }
    ((v - min) / (max - min)).clamp(0.0, 1.0)
}

/// Complete per-session processing pipeline.
#[derive(Debug, Clone, Default)]
pub struct Processor {
    pub smoothing: Smoothing,
    spike: SpikeFilter,
    ema: Ema,
    stats: SessionStats,
    rmssd: Rmssd,
    last_raw: Option<u16>,
    last_accepted_at: Option<Instant>,
    out_of_range: u64,
}

impl Processor {
    pub fn new(smoothing: Smoothing) -> Self {
        Self {
            smoothing,
            ..Default::default()
        }
    }

    /// Feeds a heart rate sample.
    pub fn push_bpm(&mut self, bpm: u16, has_contact: bool, now: Instant) -> SampleOutcome {
        if !has_contact {
            return SampleOutcome::Rejected(RejectReason::NoContact);
        }
        if !(MIN_VALID_BPM..=MAX_VALID_BPM).contains(&bpm) {
            self.out_of_range += 1;
            return SampleOutcome::Rejected(RejectReason::OutOfRange);
        }
        // A long gap invalidates the smoothing history.
        if let Some(t) = self.last_accepted_at {
            if now.saturating_duration_since(t) > Duration::from_secs(10) {
                self.ema.reset();
            }
        }
        match self.spike.push(bpm, now) {
            Some(v) => {
                let smoothed = self.ema.update(v as f64, self.smoothing.alpha());
                self.stats.push(v);
                self.last_raw = Some(v);
                self.last_accepted_at = Some(now);
                SampleOutcome::Accepted { raw: v, smoothed }
            }
            None => SampleOutcome::Rejected(RejectReason::SuspectedSpike),
        }
    }

    pub fn push_rr(&mut self, rr_ms: f64, now: Instant) {
        self.rmssd.push(rr_ms, now);
    }

    pub fn hrv_ms(&mut self, now: Instant) -> Option<f64> {
        self.rmssd.value(now)
    }

    pub fn stats(&self) -> SessionStats {
        self.stats
    }

    pub fn last_raw(&self) -> Option<u16> {
        self.last_raw
    }

    pub fn smoothed(&self) -> Option<f64> {
        self.ema.value()
    }

    pub fn last_accepted_at(&self) -> Option<Instant> {
        self.last_accepted_at
    }

    pub fn rejected_spikes(&self) -> u64 {
        self.spike.rejected_count()
    }

    pub fn rejected_out_of_range(&self) -> u64 {
        self.out_of_range
    }

    /// Called on (re)connect: keeps session stats, drops short-term history.
    pub fn on_reconnect(&mut self) {
        self.spike.reset();
        self.ema.reset();
        self.rmssd.reset();
    }

    pub fn reset_session(&mut self) {
        let smoothing = self.smoothing;
        *self = Self::new(smoothing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn ema_math() {
        let mut e = Ema::default();
        assert_eq!(e.update(100.0, 0.5), 100.0);
        assert_eq!(e.update(80.0, 0.5), 90.0);
        assert_eq!(e.update(80.0, 0.25), 87.5);
        e.reset();
        assert_eq!(e.value(), None);
        assert_eq!(e.update(60.0, 1.0), 60.0);
    }

    #[test]
    fn smoothing_levels() {
        let start = t0();
        for (s, expect) in [
            (Smoothing::Off, 80.0),
            (Smoothing::Light, 70.0),
            (Smoothing::Medium, 65.0),
        ] {
            let mut p = Processor::new(s);
            p.push_bpm(60, true, start);
            match p.push_bpm(80, true, start + Duration::from_secs(1)) {
                SampleOutcome::Accepted { raw, smoothed } => {
                    assert_eq!(raw, 80);
                    assert!((smoothed - expect).abs() < 1e-9, "{s:?}");
                }
                o => panic!("{o:?}"),
            }
        }
    }

    #[test]
    fn rejects_out_of_range() {
        let mut p = Processor::new(Smoothing::Off);
        let n = t0();
        assert_eq!(
            p.push_bpm(24, true, n),
            SampleOutcome::Rejected(RejectReason::OutOfRange)
        );
        assert_eq!(
            p.push_bpm(251, true, n),
            SampleOutcome::Rejected(RejectReason::OutOfRange)
        );
        assert_eq!(
            p.push_bpm(0, true, n),
            SampleOutcome::Rejected(RejectReason::OutOfRange)
        );
        assert!(matches!(p.push_bpm(25, true, n), SampleOutcome::Accepted { .. }));
        let mut p = Processor::new(Smoothing::Off);
        assert!(matches!(p.push_bpm(250, true, n), SampleOutcome::Accepted { .. }));
        assert_eq!(p.rejected_out_of_range(), 0);
    }

    #[test]
    fn rejects_no_contact() {
        let mut p = Processor::new(Smoothing::Off);
        assert_eq!(
            p.push_bpm(70, false, t0()),
            SampleOutcome::Rejected(RejectReason::NoContact)
        );
    }

    #[test]
    fn single_sample_spike_rejected() {
        let mut f = SpikeFilter::default();
        let s = t0();
        let sec = |n: u64| s + Duration::from_secs(n);
        assert_eq!(f.push(70, sec(0)), Some(70));
        assert_eq!(f.push(72, sec(1)), Some(72));
        assert_eq!(f.push(180, sec(2)), None); // spike held
        assert_eq!(f.push(73, sec(3)), Some(73)); // back to normal: spike discarded
        assert_eq!(f.rejected_count(), 1);
    }

    #[test]
    fn confirmed_jump_accepted() {
        let mut f = SpikeFilter::default();
        let s = t0();
        let sec = |n: u64| s + Duration::from_secs(n);
        assert_eq!(f.push(70, sec(0)), Some(70));
        assert_eq!(f.push(130, sec(1)), None);
        assert_eq!(f.push(128, sec(2)), Some(128)); // confirmed
        assert_eq!(f.push(126, sec(3)), Some(126));
        assert_eq!(f.rejected_count(), 0);
    }

    #[test]
    fn gradual_change_never_held() {
        let mut f = SpikeFilter::default();
        let s = t0();
        for i in 0..100u16 {
            let bpm = 60 + i; // +1 bpm/s ramp
            assert_eq!(f.push(bpm, s + Duration::from_secs(i as u64)), Some(bpm));
        }
    }

    #[test]
    fn spike_filter_resets_after_gap() {
        let mut f = SpikeFilter::default();
        let s = t0();
        f.push(60, s);
        assert_eq!(f.push(150, s + Duration::from_secs(10)), Some(150));
    }

    #[test]
    fn spike_threshold() {
        assert_eq!(SpikeFilter::threshold(60), 25);
        assert_eq!(SpikeFilter::threshold(200), 40);
    }

    #[test]
    fn session_stats() {
        let mut p = Processor::new(Smoothing::Medium);
        let s = t0();
        for (i, v) in [60u16, 70, 80].into_iter().enumerate() {
            p.push_bpm(v, true, s + Duration::from_secs(i as u64));
        }
        let st = p.stats();
        assert_eq!(st.min, Some(60));
        assert_eq!(st.max, Some(80));
        assert_eq!(st.avg, Some(70.0));
        assert_eq!(st.samples, 3);
        p.reset_session();
        assert_eq!(p.stats().samples, 0);
        assert_eq!(p.smoothing, Smoothing::Medium);
    }

    #[test]
    fn rmssd_plain_math() {
        // diffs: 10, -20, 30 -> squares 100, 400, 900 -> mean 466.67 -> sqrt 21.602
        let v = rmssd(&[800.0, 810.0, 790.0, 820.0]).unwrap();
        assert!((v - 21.602_469).abs() < 1e-5);
        assert_eq!(rmssd(&[800.0]), None);
        assert_eq!(rmssd(&[800.0, 800.0]), Some(0.0));
    }

    #[test]
    fn rolling_rmssd_matches_plain() {
        let mut r = Rmssd::default();
        let s = t0();
        let seq = [800.0, 810.0, 790.0, 820.0, 805.0, 815.0];
        for (i, ms) in seq.iter().enumerate() {
            r.push(*ms, s + Duration::from_millis(800 * i as u64));
        }
        let now = s + Duration::from_secs(5);
        let got = r.value(now).unwrap();
        assert!((got - rmssd(&seq).unwrap()).abs() < 1e-9);
    }

    #[test]
    fn rolling_rmssd_window_expires() {
        let mut r = Rmssd::default();
        let s = t0();
        for i in 0..10 {
            r.push(800.0 + (i % 2) as f64 * 20.0, s + Duration::from_millis(800 * i));
        }
        assert!(r.value(s + Duration::from_secs(8)).is_some());
        assert_eq!(r.value(s + Duration::from_secs(60)), None);
    }

    #[test]
    fn rmssd_filters_artefacts() {
        let mut r = Rmssd::default();
        let s = t0();
        let seq = [800.0, 810.0, 3000.0, 790.0, 100.0, 820.0, 1300.0, 805.0, 815.0];
        for (i, ms) in seq.iter().enumerate() {
            r.push(*ms, s + Duration::from_millis(800 * i as u64));
        }
        let clean = [800.0, 810.0, 790.0, 820.0, 805.0, 815.0];
        let got = r.value(s + Duration::from_secs(8)).unwrap();
        assert!((got - rmssd(&clean).unwrap()).abs() < 1e-9);
    }

    #[test]
    fn rmssd_accepts_persistent_rate_change() {
        let mut r = Rmssd::default();
        let s = t0();
        let mut t = 0u64;
        for _ in 0..3 {
            r.push(1000.0, s + Duration::from_millis(t));
            t += 1000;
        }
        // Sustained jump to 600 ms is eventually accepted.
        for _ in 0..8 {
            r.push(600.0, s + Duration::from_millis(t));
            t += 600;
        }
        assert_eq!(r.value(s + Duration::from_millis(t)), Some(0.0));
    }

    #[test]
    fn rmssd_does_not_diff_across_gaps() {
        let mut r = Rmssd::default();
        let s = t0();
        for i in 0..5 {
            r.push(800.0, s + Duration::from_millis(800 * i));
        }
        // 10 s gap, then a different but steady rhythm (within 25 %).
        for i in 0..5 {
            r.push(
                900.0,
                s + Duration::from_secs(14) + Duration::from_millis(900 * i),
            );
        }
        assert_eq!(r.value(s + Duration::from_secs(19)), Some(0.0));
    }

    #[test]
    fn zones() {
        let z = ZoneConfig {
            age: Some(30),
            ..Default::default()
        };
        assert_eq!(z.max_hr(), 190);
        assert_eq!(z.zone(80), 0); // 42 %
        assert_eq!(z.zone(95), 1); // 50 %
        assert_eq!(z.zone(114), 2); // 60 %
        assert_eq!(z.zone(133), 3); // 70 %
        assert_eq!(z.zone(152), 4); // 80 %
        assert_eq!(z.zone(171), 5); // 90 %
        assert_eq!(z.zone(250), 5);
        let m = ZoneConfig {
            age: None,
            manual_max_hr: 200,
            ..Default::default()
        };
        assert_eq!(m.max_hr(), 200);
        assert_eq!(m.zone(100), 1);
    }

    #[test]
    fn zone_sanitize() {
        let mut z = ZoneConfig {
            age: Some(0),
            manual_max_hr: 20,
            thresholds: [90, 50, 70, 60, 200],
        };
        z.sanitize();
        assert_eq!(z.thresholds, [50, 60, 70, 90, 100]);
        assert_eq!(z.age, None);
        assert_eq!(z.manual_max_hr, 100);
    }

    #[test]
    fn digits() {
        assert_eq!(split_digits(0), (0, 0, 0));
        assert_eq!(split_digits(7), (7, 0, 0));
        assert_eq!(split_digits(65), (5, 6, 0));
        assert_eq!(split_digits(100), (0, 0, 1));
        assert_eq!(split_digits(187), (7, 8, 1));
        assert_eq!(split_digits(250), (0, 5, 2));
        assert_eq!(split_digits(1234), (9, 9, 9));
    }

    #[test]
    fn normalization() {
        assert_eq!(normalize(40.0, 40.0, 200.0), 0.0);
        assert_eq!(normalize(200.0, 40.0, 200.0), 1.0);
        assert_eq!(normalize(120.0, 40.0, 200.0), 0.5);
        assert_eq!(normalize(10.0, 40.0, 200.0), 0.0);
        assert_eq!(normalize(300.0, 40.0, 200.0), 1.0);
        assert_eq!(normalize(5.0, 10.0, 10.0), 0.0);
    }

    #[test]
    fn reconnect_keeps_stats() {
        let mut p = Processor::new(Smoothing::Light);
        let s = t0();
        p.push_bpm(70, true, s);
        p.on_reconnect();
        assert_eq!(p.stats().samples, 1);
        assert_eq!(p.smoothed(), None);
    }
}
