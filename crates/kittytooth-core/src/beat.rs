//! Beat scheduler.
//!
//! Sensors deliver RR intervals in batches (typically once per second) describing beats that
//! already happened. To produce a beat event stream with the *real* rhythm, the scheduler
//! queues those intervals and replays them: each beat fires one RR interval after the
//! previous beat. When no RR data is available (sensor doesn't send it, or it stopped), the
//! interval is derived from the current BPM instead.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Where a beat's timing came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BeatSource {
    Rr,
    Bpm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Beat {
    pub at: Instant,
    pub interval: Duration,
    pub source: BeatSource,
}

#[derive(Debug, Clone, Default)]
pub struct BeatScheduler {
    queue: VecDeque<Duration>,
    last_beat: Option<Instant>,
    last_rr_at: Option<Instant>,
    bpm: Option<f64>,
}

impl BeatScheduler {
    /// Max queued intervals; more than this means we are lagging, so old ones are dropped.
    const MAX_QUEUE: usize = 4;
    /// Fall back to BPM timing if no RR arrived for this long.
    const RR_TIMEOUT: Duration = Duration::from_secs(3);

    pub fn push_rr(&mut self, rr: Duration, now: Instant) {
        if !(Duration::from_millis(240)..=Duration::from_millis(2400)).contains(&rr) {
            return;
        }
        self.last_rr_at = Some(now);
        self.queue.push_back(rr);
        while self.queue.len() > Self::MAX_QUEUE {
            self.queue.pop_front();
        }
    }

    pub fn set_bpm(&mut self, bpm: Option<f64>) {
        self.bpm = bpm.filter(|b| (25.0..=250.0).contains(b));
    }

    fn rr_active(&self, now: Instant) -> bool {
        self.last_rr_at
            .is_some_and(|t| now.saturating_duration_since(t) <= Self::RR_TIMEOUT)
    }

    fn next_interval(&self, now: Instant) -> Option<(Duration, BeatSource)> {
        if let Some(rr) = self.queue.front() {
            return Some((*rr, BeatSource::Rr));
        }
        if self.rr_active(now) {
            // RR stream is live but momentarily drained: keep rhythm with the last BPM.
            return self
                .bpm
                .map(|b| (Duration::from_secs_f64(60.0 / b), BeatSource::Rr));
        }
        self.bpm
            .map(|b| (Duration::from_secs_f64(60.0 / b), BeatSource::Bpm))
    }

    /// When the next beat is due, or `None` if there is nothing to beat to.
    pub fn next_beat_at(&self, now: Instant) -> Option<Instant> {
        let (interval, _) = self.next_interval(now)?;
        match self.last_beat {
            Some(last) => {
                let due = last + interval;
                // After a pause, don't fire a burst of overdue beats: beat now.
                Some(if due + interval < now { now } else { due })
            }
            None => Some(now),
        }
    }

    /// Fires the beat if due. Returns the beat when one fired.
    pub fn poll(&mut self, now: Instant) -> Option<Beat> {
        let due = self.next_beat_at(now)?;
        if due > now {
            return None;
        }
        let (interval, source) = self.next_interval(now)?;
        if source == BeatSource::Rr && !self.queue.is_empty() {
            self.queue.pop_front();
        }
        self.last_beat = Some(now);
        Some(Beat {
            at: now,
            interval,
            source,
        })
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_without_data() {
        let s = BeatScheduler::default();
        assert_eq!(s.next_beat_at(Instant::now()), None);
    }

    #[test]
    fn bpm_fallback_timing() {
        let mut s = BeatScheduler::default();
        let t = Instant::now();
        s.set_bpm(Some(60.0));
        let b = s.poll(t).unwrap();
        assert_eq!(b.source, BeatSource::Bpm);
        assert_eq!(s.next_beat_at(t), Some(t + Duration::from_secs(1)));
        assert!(s.poll(t + Duration::from_millis(500)).is_none());
        assert!(s.poll(t + Duration::from_secs(1)).is_some());
    }

    #[test]
    fn rr_replay_follows_real_intervals() {
        let mut s = BeatScheduler::default();
        let t = Instant::now();
        s.set_bpm(Some(60.0));
        s.push_rr(Duration::from_millis(800), t);
        s.push_rr(Duration::from_millis(900), t);
        let b1 = s.poll(t).unwrap();
        assert_eq!(b1.source, BeatSource::Rr);
        assert_eq!(b1.interval, Duration::from_millis(800));
        // next interval is the second RR value
        assert_eq!(s.next_beat_at(t), Some(t + Duration::from_millis(900)));
        let b2 = s.poll(t + Duration::from_millis(900)).unwrap();
        assert_eq!(b2.interval, Duration::from_millis(900));
        assert_eq!(s.queued(), 0);
    }

    #[test]
    fn rejects_implausible_rr_and_bounds_queue() {
        let mut s = BeatScheduler::default();
        let t = Instant::now();
        s.push_rr(Duration::from_millis(100), t);
        s.push_rr(Duration::from_millis(5000), t);
        assert_eq!(s.queued(), 0);
        for _ in 0..10 {
            s.push_rr(Duration::from_millis(700), t);
        }
        assert_eq!(s.queued(), 4);
    }

    #[test]
    fn no_burst_after_pause() {
        let mut s = BeatScheduler::default();
        let t = Instant::now();
        s.set_bpm(Some(120.0));
        s.poll(t).unwrap();
        let later = t + Duration::from_secs(10);
        assert_eq!(s.next_beat_at(later), Some(later));
        s.poll(later).unwrap();
        assert!(s.poll(later + Duration::from_millis(10)).is_none());
    }

    #[test]
    fn out_of_range_bpm_ignored() {
        let mut s = BeatScheduler::default();
        s.set_bpm(Some(0.0));
        assert_eq!(s.next_beat_at(Instant::now()), None);
    }
}
