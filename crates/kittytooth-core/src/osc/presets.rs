//! Parameter presets: maps live [`Metrics`] to OSC addresses and typed values.
//!
//! Names are verified against upstream sources (see DECISIONS.md for commits):
//! * HRtoVRChat_OSC — `HRtoVRChat_OSC/ParamsManager.cs`, `ConfigManager.cs`
//! * VRCOSC — `VRCOSC.App/SDK/Modules/Heartrate/HeartrateModule.cs`

use serde::Serialize;

use crate::config::{CustomMapping, Metric, OscTarget, Preset, ValueKind};
use crate::osc::{normalize_address, OscValue, AVATAR_PREFIX};
use crate::processing::{normalize, split_digits};

/// RMSSD (ms) that maps to 1.0 in normalised HRV parameters.
pub const HRV_FULL_SCALE_MS: f64 = 200.0;

/// A snapshot of every value a preset may send.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Metrics {
    pub connected: bool,
    /// Current BPM (smoothed if smoothing is on), rounded.
    pub hr: u16,
    /// Raw BPM.
    pub hr_raw: u16,
    /// Max heart rate used for zones/percent.
    pub max_hr: u16,
    pub zone: u8,
    pub hrv_ms: Option<f64>,
    /// Battery percent 0..=100.
    pub battery: Option<u8>,
    pub beat_toggle: bool,
    pub beat_pulse: bool,
    pub session_min: Option<u16>,
    pub session_avg: Option<f64>,
    pub session_max: Option<u16>,
    /// Rolling 10 s average (VRCOSC "Average").
    pub avg_10s: Option<f64>,
    /// Heart rate alert showing: 0 none, 1 high, 2 low.
    pub alert: u8,
}

impl Metrics {
    /// The zeroed metrics sent on disconnect in "send 0" mode.
    pub fn zeroed(&self) -> Metrics {
        Metrics {
            connected: false,
            max_hr: self.max_hr,
            ..Default::default()
        }
    }
}

/// What a message represents; drives priority and disconnect behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Beat toggles/pulses: highest priority, timing-sensitive.
    Beat,
    /// Connected / enabled flags.
    Connection,
    /// Everything else.
    Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParamMsg {
    pub address: String,
    pub value: OscValue,
    pub role: Role,
}

fn p(prefix: &str, name: &str, value: OscValue, role: Role) -> ParamMsg {
    ParamMsg {
        address: format!("{prefix}{name}"),
        value,
        role,
    }
}

fn int(v: impl Into<i64>) -> OscValue {
    OscValue::Int(v.into().clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

fn float(v: f64) -> OscValue {
    OscValue::Float(v as f32)
}

/// HRV mapped to 0..1 (0..200 ms).
pub fn hrv_normalized(hrv_ms: Option<f64>) -> f64 {
    hrv_ms.map_or(0.0, |v| (v / HRV_FULL_SCALE_MS).clamp(0.0, 1.0))
}

/// Builds every message for `target`'s preset from `m`.
pub fn messages(target: &OscTarget, m: &Metrics) -> Vec<ParamMsg> {
    let (lo, hi) = target.range();
    let norm = normalize(m.hr as f64, lo as f64, hi as f64);
    match target.preset {
        Preset::KittyTooth => kittytooth(m, norm),
        Preset::HrToVrchat => hrtovrchat(m, norm),
        Preset::Vrcosc => vrcosc(m, norm),
        Preset::Custom => custom(&target.custom, m, norm),
    }
}

/// Native preset under `/avatar/parameters/KittyTooth/`.
fn kittytooth(m: &Metrics, norm: f64) -> Vec<ParamMsg> {
    let pre = format!("{AVATAR_PREFIX}KittyTooth/");
    let (ones, tens, hundreds) = split_digits(m.hr);
    let percent = if m.max_hr > 0 {
        (m.hr as f64 / m.max_hr as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut v = vec![
        p(&pre, "Connected", OscValue::Bool(m.connected), Role::Connection),
        p(&pre, "HR", int(m.hr), Role::Value),
        p(&pre, "Normalized", float(norm), Role::Value),
        p(&pre, "Percent", float(percent), Role::Value),
        p(&pre, "Ones", int(ones), Role::Value),
        p(&pre, "Tens", int(tens), Role::Value),
        p(&pre, "Hundreds", int(hundreds), Role::Value),
        p(&pre, "Zone", int(m.zone), Role::Value),
        p(&pre, "HRV", float(hrv_normalized(m.hrv_ms)), Role::Value),
        p(&pre, "Beat", OscValue::Bool(m.beat_toggle), Role::Beat),
        p(&pre, "BeatPulse", OscValue::Bool(m.beat_pulse), Role::Beat),
        p(&pre, "Alert", int(m.alert), Role::Value),
    ];
    if let Some(b) = m.battery {
        v.push(p(&pre, "Battery", float(b as f64 / 100.0), Role::Value));
    }
    v
}

/// HRtoVRChat_OSC compatible parameters (root of `/avatar/parameters/`).
fn hrtovrchat(m: &Metrics, norm: f64) -> Vec<ParamMsg> {
    let pre = AVATAR_PREFIX;
    let (ones, tens, hundreds) = split_digits(m.hr);
    vec![
        p(
            pre,
            "isHRConnected",
            OscValue::Bool(m.connected),
            Role::Connection,
        ),
        p(pre, "isHRActive", OscValue::Bool(m.connected), Role::Connection),
        // Upstream clamps HR to 0..255.
        p(pre, "HR", int(m.hr.min(255)), Role::Value),
        p(pre, "onesHR", int(ones), Role::Value),
        p(pre, "tensHR", int(tens), Role::Value),
        p(pre, "hundredsHR", int(hundreds), Role::Value),
        p(pre, "HRPercent", float(norm), Role::Value),
        p(pre, "FullHRPercent", float(2.0 * norm - 1.0), Role::Value),
        // Upstream isHRBeat: true for ~100 ms on each beat.
        p(pre, "isHRBeat", OscValue::Bool(m.beat_pulse), Role::Beat),
        p(pre, "HeartBeatToggle", OscValue::Bool(m.beat_toggle), Role::Beat),
    ]
}

/// VRCOSC Heartrate module compatible parameters under `/avatar/parameters/VRCOSC/Heartrate/`.
fn vrcosc(m: &Metrics, norm: f64) -> Vec<ParamMsg> {
    let pre = format!("{AVATAR_PREFIX}VRCOSC/Heartrate/");
    let (ones, tens, hundreds) = split_digits(m.hr);
    vec![
        p(&pre, "Enabled", OscValue::Bool(m.connected), Role::Connection),
        p(&pre, "Connected", OscValue::Bool(m.connected), Role::Connection),
        p(&pre, "Value", int(m.hr), Role::Value),
        p(&pre, "Normalised", float(norm), Role::Value),
        p(
            &pre,
            "Average",
            int(m.avg_10s.map_or(m.hr as i64, |a| a.round() as i64)),
            Role::Value,
        ),
        // Upstream legacy digit parameters are floats: digit / 10.
        p(&pre, "Units", float(ones as f64 / 10.0), Role::Value),
        p(&pre, "Tens", float(tens as f64 / 10.0), Role::Value),
        p(&pre, "Hundreds", float(hundreds as f64 / 10.0), Role::Value),
        // Upstream default BeatMode is toggle.
        p(&pre, "Beat", OscValue::Bool(m.beat_toggle), Role::Beat),
    ]
}

/// Raw numeric value of a metric.
pub fn metric_value(metric: Metric, m: &Metrics, norm: f64) -> f64 {
    let (ones, tens, hundreds) = split_digits(m.hr);
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    match metric {
        Metric::Hr => m.hr as f64,
        Metric::HrRaw => m.hr_raw as f64,
        Metric::Normalized => norm,
        Metric::NormalizedSigned => 2.0 * norm - 1.0,
        Metric::Percent => {
            if m.max_hr > 0 {
                (m.hr as f64 / m.max_hr as f64).clamp(0.0, 1.0)
            } else {
                0.0
            }
        }
        Metric::Ones => ones as f64,
        Metric::Tens => tens as f64,
        Metric::Hundreds => hundreds as f64,
        Metric::Connected => b(m.connected),
        Metric::BeatToggle => b(m.beat_toggle),
        Metric::BeatPulse => b(m.beat_pulse),
        Metric::Zone => m.zone as f64,
        Metric::HrvMs => m.hrv_ms.unwrap_or(0.0),
        Metric::HrvNormalized => hrv_normalized(m.hrv_ms),
        Metric::Battery => m.battery.map_or(0.0, |v| v as f64 / 100.0),
        Metric::SessionMin => m.session_min.unwrap_or(0) as f64,
        Metric::SessionAvg => m.session_avg.unwrap_or(0.0),
        Metric::SessionMax => m.session_max.unwrap_or(0) as f64,
        Metric::Alert => m.alert as f64,
    }
}

/// Converts a metric value to the chosen OSC type. Bools are true when the value >= 0.5.
pub fn convert(v: f64, kind: ValueKind) -> OscValue {
    match kind {
        ValueKind::Int => int(v.round() as i64),
        ValueKind::Float => float(v),
        ValueKind::Bool => OscValue::Bool(v >= 0.5),
    }
}

fn metric_role(metric: Metric) -> Role {
    match metric {
        Metric::Connected => Role::Connection,
        Metric::BeatToggle | Metric::BeatPulse => Role::Beat,
        _ => Role::Value,
    }
}

/// User-defined mappings. Invalid addresses are skipped (they are validated on save).
fn custom(maps: &[CustomMapping], m: &Metrics, norm: f64) -> Vec<ParamMsg> {
    maps.iter()
        .filter_map(|c| {
            let address = normalize_address(&c.address).ok()?;
            Some(ParamMsg {
                address,
                value: convert(metric_value(c.metric, m, norm), c.kind),
                role: metric_role(c.metric),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics {
            connected: true,
            hr: 128,
            hr_raw: 130,
            max_hr: 190,
            zone: 3,
            hrv_ms: Some(50.0),
            battery: Some(80),
            beat_toggle: true,
            beat_pulse: false,
            session_min: Some(60),
            session_avg: Some(95.4),
            session_max: Some(150),
            avg_10s: Some(126.6),
            alert: 1,
        }
    }

    fn target(preset: Preset) -> OscTarget {
        OscTarget {
            preset,
            ..Default::default()
        }
    }

    fn get<'a>(v: &'a [ParamMsg], addr: &str) -> &'a ParamMsg {
        v.iter()
            .find(|m| m.address == addr)
            .unwrap_or_else(|| panic!("missing {addr}"))
    }

    #[test]
    fn kittytooth_addresses_and_types() {
        let v = messages(&target(Preset::KittyTooth), &metrics());
        let k = |n: &str| get(&v, &format!("/avatar/parameters/KittyTooth/{n}")).value;
        assert_eq!(k("HR"), OscValue::Int(128));
        assert_eq!(k("Normalized"), OscValue::Float(0.55)); // (128-40)/160
        assert_eq!(k("Percent"), OscValue::Float((128.0f64 / 190.0) as f32));
        assert_eq!(k("Ones"), OscValue::Int(8));
        assert_eq!(k("Tens"), OscValue::Int(2));
        assert_eq!(k("Hundreds"), OscValue::Int(1));
        assert_eq!(k("Connected"), OscValue::Bool(true));
        assert_eq!(k("Beat"), OscValue::Bool(true));
        assert_eq!(k("BeatPulse"), OscValue::Bool(false));
        assert_eq!(k("Zone"), OscValue::Int(3));
        assert_eq!(k("HRV"), OscValue::Float(0.25));
        assert_eq!(k("Battery"), OscValue::Float(0.8));
        assert_eq!(k("Alert"), OscValue::Int(1));
        assert_eq!(v.len(), 13);
    }

    #[test]
    fn kittytooth_omits_unknown_battery() {
        let mut m = metrics();
        m.battery = None;
        let v = messages(&target(Preset::KittyTooth), &m);
        assert!(v.iter().all(|x| !x.address.ends_with("/Battery")));
    }

    #[test]
    fn hrtovrchat_addresses_and_types() {
        let v = messages(&target(Preset::HrToVrchat), &metrics());
        let k = |n: &str| get(&v, &format!("/avatar/parameters/{n}")).value;
        assert_eq!(k("HR"), OscValue::Int(128));
        assert_eq!(k("onesHR"), OscValue::Int(8));
        assert_eq!(k("tensHR"), OscValue::Int(2));
        assert_eq!(k("hundredsHR"), OscValue::Int(1));
        assert_eq!(k("isHRConnected"), OscValue::Bool(true));
        assert_eq!(k("isHRActive"), OscValue::Bool(true));
        assert_eq!(k("isHRBeat"), OscValue::Bool(false));
        assert_eq!(k("HeartBeatToggle"), OscValue::Bool(true));
        let pct = 128.0f64 / 255.0;
        assert_eq!(k("HRPercent"), OscValue::Float(pct as f32));
        assert_eq!(k("FullHRPercent"), OscValue::Float((2.0 * pct - 1.0) as f32));
        assert_eq!(v.len(), 10);
    }

    #[test]
    fn hrtovrchat_clamps_hr_to_255() {
        let mut m = metrics();
        m.hr = 300;
        let v = messages(&target(Preset::HrToVrchat), &m);
        assert_eq!(get(&v, "/avatar/parameters/HR").value, OscValue::Int(255));
    }

    #[test]
    fn vrcosc_addresses_and_types() {
        let v = messages(&target(Preset::Vrcosc), &metrics());
        let k = |n: &str| get(&v, &format!("/avatar/parameters/VRCOSC/Heartrate/{n}")).value;
        assert_eq!(k("Enabled"), OscValue::Bool(true));
        assert_eq!(k("Connected"), OscValue::Bool(true));
        assert_eq!(k("Value"), OscValue::Int(128));
        assert_eq!(k("Normalised"), OscValue::Float((128.0f64 / 240.0) as f32));
        assert_eq!(k("Average"), OscValue::Int(127));
        assert_eq!(k("Units"), OscValue::Float(0.8));
        assert_eq!(k("Tens"), OscValue::Float(0.2));
        assert_eq!(k("Hundreds"), OscValue::Float(0.1));
        assert_eq!(k("Beat"), OscValue::Bool(true));
        assert_eq!(v.len(), 9);
    }

    #[test]
    fn custom_mapping_types() {
        let t = OscTarget {
            preset: Preset::Custom,
            custom: vec![
                CustomMapping {
                    address: "MyHR".into(),
                    metric: Metric::Hr,
                    kind: ValueKind::Int,
                },
                CustomMapping {
                    address: "/foo/norm".into(),
                    metric: Metric::Normalized,
                    kind: ValueKind::Float,
                },
                CustomMapping {
                    address: "Alive".into(),
                    metric: Metric::Connected,
                    kind: ValueKind::Bool,
                },
                CustomMapping {
                    address: "HrvAsInt".into(),
                    metric: Metric::HrvMs,
                    kind: ValueKind::Int,
                },
                CustomMapping {
                    address: "bad address".into(),
                    metric: Metric::Hr,
                    kind: ValueKind::Int,
                },
            ],
            ..Default::default()
        };
        let v = messages(&t, &metrics());
        assert_eq!(v.len(), 4);
        assert_eq!(get(&v, "/avatar/parameters/MyHR").value, OscValue::Int(128));
        assert_eq!(get(&v, "/foo/norm").value, OscValue::Float(0.55));
        assert_eq!(get(&v, "/avatar/parameters/Alive").value, OscValue::Bool(true));
        assert_eq!(get(&v, "/avatar/parameters/Alive").role, Role::Connection);
        assert_eq!(get(&v, "/avatar/parameters/HrvAsInt").value, OscValue::Int(50));
    }

    #[test]
    fn every_metric_maps() {
        let m = metrics();
        let all = [
            (Metric::Hr, 128.0),
            (Metric::HrRaw, 130.0),
            (Metric::Normalized, 0.55),
            (Metric::NormalizedSigned, 0.1),
            (Metric::Ones, 8.0),
            (Metric::Tens, 2.0),
            (Metric::Hundreds, 1.0),
            (Metric::Connected, 1.0),
            (Metric::BeatToggle, 1.0),
            (Metric::BeatPulse, 0.0),
            (Metric::Zone, 3.0),
            (Metric::HrvMs, 50.0),
            (Metric::HrvNormalized, 0.25),
            (Metric::Battery, 0.8),
            (Metric::SessionMin, 60.0),
            (Metric::SessionAvg, 95.4),
            (Metric::SessionMax, 150.0),
            (Metric::Alert, 1.0),
        ];
        for (metric, want) in all {
            let got = metric_value(metric, &m, 0.55);
            assert!((got - want).abs() < 1e-9, "{metric:?}: {got} != {want}");
        }
        assert!((metric_value(Metric::Percent, &m, 0.55) - 128.0 / 190.0).abs() < 1e-9);
    }

    #[test]
    fn conversions() {
        assert_eq!(convert(71.6, ValueKind::Int), OscValue::Int(72));
        assert_eq!(convert(0.25, ValueKind::Float), OscValue::Float(0.25));
        assert_eq!(convert(0.5, ValueKind::Bool), OscValue::Bool(true));
        assert_eq!(convert(0.49, ValueKind::Bool), OscValue::Bool(false));
    }

    #[test]
    fn zeroed_metrics() {
        let z = metrics().zeroed();
        assert!(!z.connected);
        assert_eq!(z.hr, 0);
        assert_eq!(z.max_hr, 190);
        let v = messages(&target(Preset::KittyTooth), &z);
        assert_eq!(
            get(&v, "/avatar/parameters/KittyTooth/HR").value,
            OscValue::Int(0)
        );
        assert_eq!(
            get(&v, "/avatar/parameters/KittyTooth/Normalized").value,
            OscValue::Float(0.0)
        );
    }

    #[test]
    fn hrv_scale() {
        assert_eq!(hrv_normalized(None), 0.0);
        assert_eq!(hrv_normalized(Some(100.0)), 0.5);
        assert_eq!(hrv_normalized(Some(500.0)), 1.0);
    }
}
