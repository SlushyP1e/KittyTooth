//! VRChat chatbox template rendering.

use crate::osc::presets::Metrics;

/// VRChat truncates chatbox text at 144 characters.
pub const MAX_CHATBOX_CHARS: usize = 144;

/// Supported tokens, for the UI help text.
pub const TOKENS: &[(&str, &str)] = &[
    ("{hr}", "current BPM"),
    ("{heart}", "heart emoji that alternates each send"),
    ("{min}", "session minimum"),
    ("{avg}", "session average"),
    ("{max}", "session maximum"),
    ("{hrv}", "HRV (RMSSD, ms)"),
    ("{zone}", "heart rate zone 0-5"),
    ("{battery}", "sensor battery %"),
];

/// Renders `template`. `phase` alternates the heart glyph so repeated sends look alive.
pub fn render(template: &str, m: &Metrics, phase: bool) -> String {
    let dash = || "--".to_string();
    let heart = if phase { "\u{2764}\u{fe0f}" } else { "\u{1f497}" };
    let out = template
        .replace("{hr}", &m.hr.to_string())
        .replace("{heart}", heart)
        .replace("{min}", &m.session_min.map_or_else(dash, |v| v.to_string()))
        .replace("{avg}", &m.session_avg.map_or_else(dash, |v| format!("{v:.0}")))
        .replace("{max}", &m.session_max.map_or_else(dash, |v| v.to_string()))
        .replace("{hrv}", &m.hrv_ms.map_or_else(dash, |v| format!("{v:.0}")))
        .replace("{zone}", &m.zone.to_string())
        .replace("{battery}", &m.battery.map_or_else(dash, |v| v.to_string()));
    out.chars().take(MAX_CHATBOX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_tokens() {
        let m = Metrics {
            connected: true,
            hr: 72,
            session_min: Some(60),
            session_avg: Some(80.4),
            session_max: Some(120),
            hrv_ms: Some(42.6),
            zone: 1,
            battery: Some(90),
            ..Default::default()
        };
        assert_eq!(render("{hr} bpm {heart}", &m, true), "72 bpm \u{2764}\u{fe0f}");
        assert_eq!(render("{hr} bpm {heart}", &m, false), "72 bpm \u{1f497}");
        assert_eq!(
            render(
                "{min}/{avg}/{max} hrv {hrv} z{zone} {battery}% {unknown}",
                &m,
                true
            ),
            "60/80/120 hrv 43 z1 90% {unknown}"
        );
    }

    #[test]
    fn missing_values_dash_and_truncate() {
        let m = Metrics::default();
        assert_eq!(render("{min} {hrv} {battery}", &m, true), "-- -- --");
        assert_eq!(render(&"x".repeat(300), &m, true).chars().count(), 144);
    }
}
