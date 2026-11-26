//! High and low heart rate alerts.
//!
//! An alert fires once the heart rate has stayed at or past a threshold for `sustain_secs`, so a
//! single spike or a strap slipping does not trigger it. While it lasts, a reminder fires every
//! `repeat_secs`. It clears once the rate is back inside the range by a small margin
//! (hysteresis), or when readings stop.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The rate must come back this many BPM inside the threshold before an alert clears.
pub const HYSTERESIS_BPM: u16 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertConfig {
    pub enabled: bool,
    /// Alert at or above this BPM.
    pub high_bpm: u16,
    /// Alert at or below this BPM.
    pub low_bpm: u16,
    /// Seconds the rate must stay out of range before alerting (0 = immediately).
    pub sustain_secs: u32,
    /// Seconds between reminders while it stays out of range (0 = alert once).
    pub repeat_secs: u32,
    /// Show a desktop notification.
    pub notify: bool,
    /// Play a sound in the app.
    pub sound: bool,
    /// Post the alert to the VRChat chatbox so people nearby see it.
    pub chatbox: bool,
    /// Chatbox text for a high alert. Same tokens as the chatbox template.
    pub high_text: String,
    /// Chatbox text for a low alert.
    pub low_text: String,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            high_bpm: 150,
            low_bpm: 45,
            sustain_secs: 10,
            repeat_secs: 120,
            notify: true,
            sound: true,
            chatbox: false,
            high_text: DEFAULT_HIGH_TEXT.into(),
            low_text: DEFAULT_LOW_TEXT.into(),
        }
    }
}

pub const DEFAULT_HIGH_TEXT: &str = "\u{26a0}\u{fe0f} High heart rate: {hr} bpm";
pub const DEFAULT_LOW_TEXT: &str = "\u{26a0}\u{fe0f} Low heart rate: {hr} bpm";

impl AlertConfig {
    pub fn sanitize(&mut self) {
        self.low_bpm = self.low_bpm.clamp(25, 200);
        self.high_bpm = self.high_bpm.clamp(40, 250);
        if self.low_bpm + 2 * HYSTERESIS_BPM >= self.high_bpm {
            let d = AlertConfig::default();
            self.low_bpm = d.low_bpm;
            self.high_bpm = d.high_bpm;
        }
        self.sustain_secs = self.sustain_secs.min(300);
        if self.repeat_secs != 0 {
            self.repeat_secs = self.repeat_secs.clamp(30, 3600);
        }
        for (text, default) in [
            (&mut self.high_text, DEFAULT_HIGH_TEXT),
            (&mut self.low_text, DEFAULT_LOW_TEXT),
        ] {
            if text.trim().is_empty() {
                *text = default.into();
            }
            if text.chars().count() > 144 {
                *text = text.chars().take(144).collect();
            }
        }
    }

    pub fn threshold(&self, kind: AlertKind) -> u16 {
        match kind {
            AlertKind::High => self.high_bpm,
            AlertKind::Low => self.low_bpm,
        }
    }

    pub fn text(&self, kind: AlertKind) -> &str {
        match kind {
            AlertKind::High => &self.high_text,
            AlertKind::Low => &self.low_text,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    High,
    Low,
}

impl AlertKind {
    pub fn label(self) -> &'static str {
        match self {
            AlertKind::High => "High heart rate",
            AlertKind::Low => "Low heart rate",
        }
    }
}

/// The alert currently showing, published in the snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertStatus {
    pub kind: AlertKind,
    /// Latest BPM while the alert lasts.
    pub bpm: u16,
    pub threshold: u16,
    /// Unix ms when the alert first fired.
    pub since_ms: u64,
    /// Fired by the "Test alert" button.
    pub test: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertAction {
    /// Notify now. `repeat` is a reminder for an alert that is already showing.
    Fire {
        kind: AlertKind,
        bpm: u16,
        threshold: u16,
        repeat: bool,
    },
    /// The rate is back in range (or readings stopped).
    Clear { kind: AlertKind },
}

#[derive(Debug, Clone, Copy)]
struct Active {
    kind: AlertKind,
    last_fired: Instant,
}

#[derive(Debug, Default)]
pub struct AlertMonitor {
    pending: Option<(AlertKind, Instant)>,
    active: Option<Active>,
}

impl AlertMonitor {
    pub fn active(&self) -> Option<AlertKind> {
        self.active.map(|a| a.kind)
    }

    /// Feeds the current BPM (`None` while there is no valid reading).
    pub fn update(&mut self, cfg: &AlertConfig, bpm: Option<u16>, now: Instant) -> Option<AlertAction> {
        let Some(bpm) = bpm.filter(|_| cfg.enabled) else {
            self.pending = None;
            return self.active.take().map(|a| AlertAction::Clear { kind: a.kind });
        };

        if let Some(a) = self.active {
            let still = match a.kind {
                AlertKind::High => bpm + HYSTERESIS_BPM > cfg.high_bpm,
                AlertKind::Low => bpm < cfg.low_bpm + HYSTERESIS_BPM,
            };
            if !still {
                self.active = None;
                self.pending = None;
                return Some(AlertAction::Clear { kind: a.kind });
            }
            let repeat = Duration::from_secs(cfg.repeat_secs as u64);
            if cfg.repeat_secs > 0 && now.saturating_duration_since(a.last_fired) >= repeat {
                self.active = Some(Active { last_fired: now, ..a });
                return Some(AlertAction::Fire {
                    kind: a.kind,
                    bpm,
                    threshold: cfg.threshold(a.kind),
                    repeat: true,
                });
            }
            return None;
        }

        let out = if bpm >= cfg.high_bpm {
            Some(AlertKind::High)
        } else if bpm <= cfg.low_bpm {
            Some(AlertKind::Low)
        } else {
            None
        };
        let Some(kind) = out else {
            self.pending = None;
            return None;
        };
        let since = match self.pending {
            Some((k, t)) if k == kind => t,
            _ => {
                self.pending = Some((kind, now));
                now
            }
        };
        if now.saturating_duration_since(since) >= Duration::from_secs(cfg.sustain_secs as u64) {
            self.pending = None;
            self.active = Some(Active {
                kind,
                last_fired: now,
            });
            return Some(AlertAction::Fire {
                kind,
                bpm,
                threshold: cfg.threshold(kind),
                repeat: false,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AlertConfig {
        AlertConfig {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn disabled_never_fires() {
        let mut m = AlertMonitor::default();
        let c = AlertConfig::default();
        assert_eq!(m.update(&c, Some(200), Instant::now()), None);
    }

    #[test]
    fn high_needs_sustain_then_fires_once_and_repeats() {
        let mut m = AlertMonitor::default();
        let c = cfg();
        let t = Instant::now();
        assert_eq!(m.update(&c, Some(160), t), None);
        assert_eq!(m.update(&c, Some(158), t + Duration::from_secs(9)), None);
        assert_eq!(
            m.update(&c, Some(161), t + Duration::from_secs(10)),
            Some(AlertAction::Fire {
                kind: AlertKind::High,
                bpm: 161,
                threshold: 150,
                repeat: false
            })
        );
        assert_eq!(m.update(&c, Some(165), t + Duration::from_secs(60)), None);
        assert!(matches!(
            m.update(&c, Some(165), t + Duration::from_secs(130)),
            Some(AlertAction::Fire { repeat: true, .. })
        ));
    }

    #[test]
    fn dipping_back_resets_sustain() {
        let mut m = AlertMonitor::default();
        let c = cfg();
        let t = Instant::now();
        m.update(&c, Some(160), t);
        m.update(&c, Some(120), t + Duration::from_secs(5));
        assert_eq!(m.update(&c, Some(160), t + Duration::from_secs(11)), None);
        assert!(m.update(&c, Some(160), t + Duration::from_secs(21)).is_some());
    }

    #[test]
    fn clears_with_hysteresis_and_on_signal_loss() {
        let mut m = AlertMonitor::default();
        let c = AlertConfig {
            sustain_secs: 0,
            ..cfg()
        };
        let t = Instant::now();
        assert!(m.update(&c, Some(40), t).is_some());
        assert_eq!(m.active(), Some(AlertKind::Low));
        // 46 is above the threshold but within the margin: still low.
        assert_eq!(m.update(&c, Some(46), t), None);
        assert_eq!(
            m.update(&c, Some(48), t),
            Some(AlertAction::Clear { kind: AlertKind::Low })
        );
        assert!(m.update(&c, Some(40), t).is_some());
        assert_eq!(
            m.update(&c, None, t),
            Some(AlertAction::Clear { kind: AlertKind::Low })
        );
    }

    #[test]
    fn sanitize_fixes_bad_values() {
        let mut c = AlertConfig {
            high_bpm: 60,
            low_bpm: 70,
            repeat_secs: 5,
            high_text: "  ".into(),
            ..cfg()
        };
        c.sanitize();
        assert_eq!((c.low_bpm, c.high_bpm), (45, 150));
        assert_eq!(c.repeat_secs, 30);
        assert_eq!(c.high_text, DEFAULT_HIGH_TEXT);
    }
}
