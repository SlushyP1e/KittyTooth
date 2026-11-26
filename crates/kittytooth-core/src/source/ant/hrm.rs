//! ANT+ Heart Rate Monitor data pages and RR-interval derivation.
//!
//! Every HRM page carries the same last four bytes:
//!
//! | byte | meaning                                          |
//! |------|--------------------------------------------------|
//! | 0    | page number (bits 0-6) and page-change toggle (7)|
//! | 1-3  | page-specific                                    |
//! | 4-5  | heart beat event time, LSB first, 1/1024 s       |
//! | 6    | heart beat count (wraps at 256)                  |
//! | 7    | computed heart rate, bpm (0 = invalid)           |
//!
//! The monitor repeats each beat's data at ~4 Hz. An RR interval is the difference between the
//! event times of two consecutive beats (count +1), so it works on any monitor, including Garmin
//! watches that send no RR over Bluetooth. The 16-bit event time wraps every 64 s.

/// Page 4: bytes 2-3 hold the previous beat's event time.
pub const PAGE_PREVIOUS_BEAT: u8 = 4;
/// Page 7: byte 1 is the battery level in percent (0xFF = not reported).
pub const PAGE_BATTERY: u8 = 7;

/// The common fields of one HRM data page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HrmPage {
    pub page: u8,
    pub toggle: bool,
    /// Last heart beat event time in 1/1024 s.
    pub event_time: u16,
    pub beat_count: u8,
    /// Computed heart rate; 0 means the monitor has no valid reading.
    pub bpm: u8,
    pub raw: [u8; 8],
}

impl HrmPage {
    pub fn parse(raw: &[u8; 8]) -> Self {
        Self {
            page: raw[0] & 0x7F,
            toggle: raw[0] & 0x80 != 0,
            event_time: u16::from_le_bytes([raw[4], raw[5]]),
            beat_count: raw[6],
            bpm: raw[7],
            raw: *raw,
        }
    }

    /// Previous beat event time (page 4 only).
    pub fn previous_event_time(&self) -> Option<u16> {
        (self.page == PAGE_PREVIOUS_BEAT).then(|| u16::from_le_bytes([self.raw[2], self.raw[3]]))
    }

    /// Battery level in percent (page 7 only, when reported).
    pub fn battery_percent(&self) -> Option<u8> {
        (self.page == PAGE_BATTERY && self.raw[1] <= 100).then_some(self.raw[1])
    }
}

/// Converts an event-time difference (1/1024 s, wrapping) to milliseconds.
pub fn event_delta_ms(later: u16, earlier: u16) -> f64 {
    later.wrapping_sub(earlier) as f64 * 1000.0 / 1024.0
}

/// What one page contributed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decoded {
    /// Computed heart rate, `None` when the monitor reports 0 (no reading).
    pub bpm: Option<u16>,
    /// RR intervals completed by this page, in milliseconds.
    pub rr_ms: Vec<f64>,
    pub battery: Option<u8>,
    /// Whether this page should become a reading: a new beat, or ~1 s since the last reading
    /// (so a steady value keeps flowing even if the beat count stalls).
    pub emit: bool,
}

/// Pages per second at the HRM message period; used to emit at least once a second.
const PAGES_PER_READING: u32 = 4;

/// Turns a stream of HRM pages into readings with RR intervals.
#[derive(Debug, Default)]
pub struct HrmDecoder {
    last_beat: Option<(u8, u16)>,
    last_toggle: Option<bool>,
    /// The page-change toggle has flipped, so page numbers are meaningful. Legacy monitors
    /// never flip it and only the common fields can be trusted.
    paged: bool,
    since_emit: u32,
}

impl HrmDecoder {
    pub fn push(&mut self, raw: &[u8; 8]) -> Decoded {
        let p = HrmPage::parse(raw);
        if self.last_toggle.is_some_and(|t| t != p.toggle) {
            self.paged = true;
        }
        self.last_toggle = Some(p.toggle);

        let mut out = Decoded {
            bpm: (p.bpm != 0).then_some(p.bpm as u16),
            ..Decoded::default()
        };
        if self.paged {
            out.battery = p.battery_percent();
        }

        let new_beat = match self.last_beat {
            None => true,
            Some((count, event)) => match p.beat_count.wrapping_sub(count) {
                0 => false,
                1 => {
                    out.rr_ms.push(event_delta_ms(p.event_time, event));
                    true
                }
                // Beats were missed (radio dropouts). The gap can't be split into intervals,
                // but page 4 still gives the interval that ended at this beat.
                _ => {
                    if let Some(prev) = self.paged.then(|| p.previous_event_time()).flatten() {
                        out.rr_ms.push(event_delta_ms(p.event_time, prev));
                    }
                    true
                }
            },
        };
        out.rr_ms.retain(|&rr| rr > 0.0);
        if new_beat {
            self.last_beat = Some((p.beat_count, p.event_time));
        }

        self.since_emit += 1;
        out.emit = new_beat || self.since_emit >= PAGES_PER_READING;
        if out.emit {
            self.since_emit = 0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a page: `page | toggle`, page-specific bytes, event time, count, bpm.
    fn page(no: u8, toggle: bool, b1: u8, b23: u16, event: u16, count: u8, bpm: u8) -> [u8; 8] {
        let [e0, e1] = event.to_le_bytes();
        let [p0, p1] = b23.to_le_bytes();
        [no | if toggle { 0x80 } else { 0 }, b1, p0, p1, e0, e1, count, bpm]
    }

    #[test]
    fn parses_fixture_pages() {
        // Page 4 from a chest strap: previous beat 0x1E00, this beat 0x2190 (= 912 / 1024 s
        // later), beat 0x2A, 70 bpm.
        let raw = [0x84, 0xFF, 0x00, 0x1E, 0x90, 0x21, 0x2A, 0x46];
        let p = HrmPage::parse(&raw);
        assert_eq!(p.page, 4);
        assert!(p.toggle);
        assert_eq!(p.event_time, 0x2190);
        assert_eq!(p.beat_count, 42);
        assert_eq!(p.bpm, 70);
        assert_eq!(p.previous_event_time(), Some(0x1E00));
        assert_eq!(p.battery_percent(), None);
        assert_eq!(event_delta_ms(0x2190, 0x1E00), 890.625);

        // Page 7: 85 % battery, 2.9 V, status "good".
        let raw = [0x07, 0x55, 0xE6, 0x22, 0x90, 0x21, 0x2A, 0x46];
        let p = HrmPage::parse(&raw);
        assert_eq!(p.battery_percent(), Some(85));
        assert_eq!(p.previous_event_time(), None);
        // Level not reported.
        let raw = [0x07, 0xFF, 0xE6, 0x22, 0x90, 0x21, 0x2A, 0x46];
        assert_eq!(HrmPage::parse(&raw).battery_percent(), None);
    }

    #[test]
    fn rr_from_consecutive_beats() {
        let mut d = HrmDecoder::default();
        // First page: a reading, but no interval yet.
        let r = d.push(&page(0, false, 0xFF, 0xFFFF, 1024, 10, 60));
        assert_eq!((r.bpm, r.emit, r.rr_ms.len()), (Some(60), true, 0));
        // The same beat repeated at 4 Hz: no reading, no interval.
        for _ in 0..2 {
            let r = d.push(&page(0, false, 0xFF, 0xFFFF, 1024, 10, 60));
            assert!(!r.emit && r.rr_ms.is_empty());
        }
        // Next beat 1024 ticks later: one 1000 ms interval.
        let r = d.push(&page(0, false, 0xFF, 0xFFFF, 2048, 11, 60));
        assert!(r.emit);
        assert_eq!(r.rr_ms, vec![1000.0]);
        // Next beat 820 ticks later.
        let r = d.push(&page(0, false, 0xFF, 0xFFFF, 2868, 12, 62));
        assert_eq!(r.rr_ms, vec![820.0 * 1000.0 / 1024.0]);
    }

    #[test]
    fn rr_across_event_time_wraparound() {
        let mut d = HrmDecoder::default();
        d.push(&page(0, false, 0xFF, 0xFFFF, 0xFE00, 255, 75));
        // 0xFE00 + 800 wraps to 0x0120; the beat count wraps 255 -> 0 at the same time.
        let r = d.push(&page(0, false, 0xFF, 0xFFFF, 0x0120, 0, 75));
        assert_eq!(r.rr_ms, vec![800.0 * 1000.0 / 1024.0]);
    }

    #[test]
    fn beat_count_gaps_drop_the_interval() {
        let mut d = HrmDecoder::default();
        d.push(&page(0, false, 0xFF, 0xFFFF, 1000, 20, 70));
        // Two beats missed: the 2.6 s span holds three intervals, so none is reported.
        let r = d.push(&page(0, false, 0xFF, 0xFFFF, 3700, 23, 70));
        assert!(r.emit);
        assert!(r.rr_ms.is_empty());
        // Back to consecutive beats: intervals resume from the new reference.
        let r = d.push(&page(0, false, 0xFF, 0xFFFF, 4600, 24, 70));
        assert_eq!(r.rr_ms, vec![900.0 * 1000.0 / 1024.0]);
    }

    #[test]
    fn page_4_recovers_the_interval_after_a_gap() {
        let mut d = HrmDecoder::default();
        // Toggle flips after the first page, so pages are trusted.
        d.push(&page(4, false, 0xFF, 100, 1000, 20, 70));
        d.push(&page(4, true, 0xFF, 100, 1000, 20, 70));
        // Gap of 3 beats, but page 4 says the last beat came 870 ticks after the one before.
        let r = d.push(&page(4, true, 0xFF, 3000, 3870, 23, 70));
        assert_eq!(r.rr_ms, vec![870.0 * 1000.0 / 1024.0]);
    }

    #[test]
    fn legacy_monitors_ignore_page_bytes() {
        let mut d = HrmDecoder::default();
        // A legacy monitor never flips the toggle: byte 0 is junk, so neither "page 7" battery
        // nor "page 4" previous event times are believed.
        let r = d.push(&page(7, false, 50, 0, 1000, 1, 70));
        assert_eq!(r.battery, None);
        let r = d.push(&page(4, false, 0, 3000, 3870, 5, 70));
        assert!(r.rr_ms.is_empty());
    }

    #[test]
    fn battery_once_paged() {
        let mut d = HrmDecoder::default();
        d.push(&page(0, false, 0xFF, 0xFFFF, 1000, 1, 70));
        let r = d.push(&page(PAGE_BATTERY, true, 64, 0x22E6, 1000, 1, 70));
        assert_eq!(r.battery, Some(64));
    }

    #[test]
    fn steady_readings_keep_flowing_and_zero_is_invalid() {
        let mut d = HrmDecoder::default();
        assert!(d.push(&page(0, false, 0xFF, 0xFFFF, 1000, 1, 0)).emit);
        let emits: Vec<bool> = (0..8)
            .map(|_| d.push(&page(0, false, 0xFF, 0xFFFF, 1000, 1, 0)).emit)
            .collect();
        assert_eq!(emits, [false, false, false, true, false, false, false, true]);
        assert_eq!(d.push(&page(0, false, 0xFF, 0xFFFF, 1000, 1, 0)).bpm, None);
    }
}
