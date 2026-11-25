//! Versioned JSON configuration stored in the OS app data directory.
//!
//! Every struct uses `#[serde(default)]`, so missing fields (older files, hand edits) fall
//! back to defaults and unknown fields are ignored. Structural changes bump
//! [`CURRENT_SCHEMA_VERSION`] and add a step to [`migrate`].

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{KtError, KtResult};
use crate::processing::{Smoothing, ZoneConfig};
use crate::source::SourceKind;

pub const CURRENT_SCHEMA_VERSION: u32 = 3;
pub const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub device: DeviceConfig,
    pub source: SourceConfig,
    pub processing: ProcessingConfig,
    pub zones: ZoneConfig,
    pub osc: OscConfig,
    pub ui: UiConfig,
    pub overlay: OverlayConfig,
    pub debug: DebugConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            device: DeviceConfig::default(),
            source: SourceConfig::default(),
            processing: ProcessingConfig::default(),
            zones: ZoneConfig::default(),
            osc: OscConfig::default(),
            ui: UiConfig::default(),
            overlay: OverlayConfig::default(),
            debug: DebugConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberedDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub last_device: Option<RememberedDevice>,
    pub auto_connect: bool,
    pub show_all_devices: bool,
    /// Seconds without data before the sensor is treated as disconnected.
    pub stale_timeout_secs: u32,
    /// Show the built-in demo sensor in the device list.
    pub demo_device: bool,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            last_device: None,
            auto_connect: true,
            show_all_devices: false,
            stale_timeout_secs: 5,
            demo_device: true,
        }
    }
}

/// Which heart rate source is active, per-source settings and the optional fallback.
/// Tokens and keys are not stored here; they live in the OS credential store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SourceConfig {
    pub active: SourceKind,
    pub hyperate: HypeRateConfig,
    pub osc_input: OscInputConfig,
    pub fallback: FallbackConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HypeRateConfig {
    /// The ID shown in the HypeRate app.
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OscInputConfig {
    pub port: u16,
    /// OSC address carrying the heart rate (a bare name gets the /avatar/parameters/ prefix).
    pub address: String,
    /// Listen on all interfaces instead of only this PC.
    pub lan: bool,
}

impl Default for OscInputConfig {
    fn default() -> Self {
        Self {
            port: 9010,
            address: "/avatar/parameters/HR".into(),
            lan: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FallbackConfig {
    pub enabled: bool,
    pub backup: SourceKind,
    /// Seconds the main source may be stale before switching to the backup.
    pub after_secs: u32,
}

impl Default for FallbackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backup: SourceKind::Demo,
            after_secs: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProcessingConfig {
    pub smoothing: Smoothing,
}

/// OSC parameter preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    #[default]
    KittyTooth,
    HrToVrchat,
    Vrcosc,
    Custom,
}

impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Preset::KittyTooth => "KittyTooth",
            Preset::HrToVrchat => "HRtoVRChat",
            Preset::Vrcosc => "VRCOSC Heartrate",
            Preset::Custom => "Custom",
        }
    }

    /// Default normalisation range, matching each upstream project's defaults.
    pub fn default_range(self) -> (u16, u16) {
        match self {
            Preset::KittyTooth | Preset::Custom => (40, 200),
            // HRtoVRChat_OSC: MinHR = 0, MaxHR = 255
            Preset::HrToVrchat => (0, 255),
            // VRCOSC: NormalisedLowerbound = 0, NormalisedUpperbound = 240
            Preset::Vrcosc => (0, 240),
        }
    }
}

/// Values that can be mapped in the custom preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Current (smoothed if enabled) BPM.
    Hr,
    /// Raw, unsmoothed BPM.
    HrRaw,
    /// 0..1 over the target's min/max range.
    Normalized,
    /// -1..1 over the target's min/max range.
    NormalizedSigned,
    /// 0..1 fraction of max heart rate.
    Percent,
    Ones,
    Tens,
    Hundreds,
    Connected,
    BeatToggle,
    BeatPulse,
    Zone,
    /// RMSSD in milliseconds.
    HrvMs,
    /// RMSSD mapped 0..200 ms -> 0..1.
    HrvNormalized,
    /// 0..1.
    Battery,
    SessionMin,
    SessionAvg,
    SessionMax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Int,
    Float,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomMapping {
    /// Full OSC address or a bare parameter name (prefixed with `/avatar/parameters/`).
    pub address: String,
    pub metric: Metric,
    pub kind: ValueKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OscTarget {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub enabled: bool,
    pub preset: Preset,
    pub custom: Vec<CustomMapping>,
    /// Use the VRChat OSC port discovered through OSCQuery when available.
    pub use_oscquery: bool,
    /// Send chatbox messages to this target (when chatbox output is on).
    pub chatbox: bool,
    /// Normalisation range override; `None` uses the preset default.
    pub range: Option<(u16, u16)>,
}

impl Default for OscTarget {
    fn default() -> Self {
        Self {
            id: "vrchat".into(),
            name: "VRChat".into(),
            host: "127.0.0.1".into(),
            port: 9000,
            enabled: true,
            preset: Preset::KittyTooth,
            custom: Vec::new(),
            use_oscquery: true,
            chatbox: true,
            range: None,
        }
    }
}

impl OscTarget {
    pub fn range(&self) -> (u16, u16) {
        self.range.unwrap_or_else(|| self.preset.default_range())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisconnectBehavior {
    /// Send Connected=false and leave the other parameters at their last value.
    #[default]
    HoldLast,
    /// Send Connected=false and zero all values.
    SendZero,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatboxConfig {
    pub enabled: bool,
    /// Tokens: {hr} {heart} {min} {avg} {max} {hrv} {zone} {battery}
    pub template: String,
    /// Seconds between sends; never less than 2.
    pub interval_secs: f64,
    /// Send without the notification sound.
    pub silent: bool,
}

impl Default for ChatboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            template: "{hr} bpm {heart}".into(),
            interval_secs: 2.0,
            silent: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OscConfig {
    pub targets: Vec<OscTarget>,
    /// Keepalive resend interval in ms (0 disables keepalive).
    pub keepalive_ms: u32,
    /// Max messages per second per target (1..=20).
    pub max_rate: u32,
    pub on_disconnect: DisconnectBehavior,
    /// Advertise KittyTooth via OSCQuery/mDNS and discover VRChat's OSC port.
    pub oscquery: bool,
    pub chatbox: ChatboxConfig,
}

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            targets: vec![OscTarget::default()],
            keepalive_ms: 1000,
            max_rate: 20,
            on_disconnect: DisconnectBehavior::HoldLast,
            oscquery: true,
            chatbox: ChatboxConfig::default(),
        }
    }
}

pub const MAX_RATE_CAP: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: Theme,
    /// A preset name ("pink", "peach", "lilac", "sky") or a custom "#rrggbb".
    pub accent: String,
    pub start_minimized: bool,
    /// Closing the main window hides it to the tray instead of quitting.
    pub close_to_tray: bool,
    pub autostart: bool,
    pub mini_always_on_top: bool,
    /// Graph range: "60s", "5m" or "session".
    pub graph_range: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            accent: "pink".into(),
            start_minimized: false,
            close_to_tray: true,
            autostart: false,
            mini_always_on_top: true,
            graph_range: "60s".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 9123,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DebugConfig {
    /// Log BLE packet -> OSC send latency for every packet.
    pub log_latency: bool,
}

impl Config {
    /// Clamps every value into its valid range. Returns `self` for chaining.
    pub fn sanitized(mut self) -> Self {
        self.schema_version = CURRENT_SCHEMA_VERSION;
        self.device.stale_timeout_secs = self.device.stale_timeout_secs.clamp(2, 60);
        self.zones.sanitize();
        self.osc.max_rate = self.osc.max_rate.clamp(1, MAX_RATE_CAP);
        if self.osc.keepalive_ms != 0 {
            self.osc.keepalive_ms = self.osc.keepalive_ms.clamp(200, 10_000);
        }
        if !self.osc.chatbox.interval_secs.is_finite() || self.osc.chatbox.interval_secs < 2.0 {
            self.osc.chatbox.interval_secs = 2.0;
        }
        self.osc.chatbox.interval_secs = self.osc.chatbox.interval_secs.min(60.0);
        if self.osc.chatbox.template.chars().count() > 144 {
            self.osc.chatbox.template = self.osc.chatbox.template.chars().take(144).collect();
        }
        let mut seen = std::collections::HashSet::new();
        for (i, t) in self.osc.targets.iter_mut().enumerate() {
            if t.id.trim().is_empty() || !seen.insert(t.id.clone()) {
                t.id = format!("target-{}", i + 1);
                seen.insert(t.id.clone());
            }
            if t.port == 0 {
                t.port = 9000;
            }
            t.host = t.host.trim().to_string();
            if t.host.is_empty() {
                t.host = "127.0.0.1".into();
            }
            if let Some((lo, hi)) = t.range {
                if lo >= hi {
                    t.range = None;
                }
            }
        }
        if !is_hex_color(&self.ui.accent) && !ACCENT_PRESETS.contains(&self.ui.accent.as_str()) {
            self.ui.accent = UiConfig::default().accent;
        }
        if !matches!(self.ui.graph_range.as_str(), "60s" | "5m" | "session") {
            self.ui.graph_range = "60s".into();
        }
        self.source.fallback.after_secs = self.source.fallback.after_secs.clamp(2, 120);
        if self.source.osc_input.port < 1024 {
            self.source.osc_input.port = OscInputConfig::default().port;
        }
        self.source.osc_input.address = self.source.osc_input.address.trim().to_string();
        if self.source.osc_input.address.is_empty() {
            self.source.osc_input.address = OscInputConfig::default().address;
        }
        self.source.hyperate.device_id = self.source.hyperate.device_id.trim().chars().take(32).collect();
        if self.overlay.port < 1024 {
            self.overlay.port = OverlayConfig::default().port;
        }
        self
    }
}

/// Named accent presets (colors defined in the UI's tokens.css).
pub const ACCENT_PRESETS: [&str; 4] = ["pink", "peach", "lilac", "sky"];

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Migrates a raw JSON config of any known version to the current schema.
/// Returns the migrated value and the version it started from.
pub fn migrate(mut v: Value) -> KtResult<(Value, u32)> {
    if !v.is_object() {
        return Err(KtError::Config("config root is not an object".into()));
    }
    let from = v
        .get("schema_version")
        .and_then(Value::as_u64)
        .map(|n| n as u32)
        .unwrap_or(1);
    let mut ver = from;
    while ver < CURRENT_SCHEMA_VERSION {
        v = match ver {
            1 => migrate_v1_to_v2(v),
            2 => migrate_v2_to_v3(v),
            _ => return Err(KtError::Config(format!("unknown schema version {ver}"))),
        };
        ver += 1;
    }
    // Newer than us (downgrade): keep known fields, defaults fill the rest.
    if let Some(o) = v.as_object_mut() {
        o.insert("schema_version".into(), json!(CURRENT_SCHEMA_VERSION));
    }
    Ok((v, from))
}

/// v1 (0.x prototype) was a flat object with a single OSC target.
fn migrate_v1_to_v2(v: Value) -> Value {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let n = |k: &str| v.get(k).and_then(Value::as_u64);
    let b = |k: &str| v.get(k).and_then(Value::as_bool);
    let preset = match s("preset").as_deref() {
        Some("hrtovrchat") | Some("hr_to_vrchat") => "hr_to_vrchat",
        Some("vrcosc") => "vrcosc",
        _ => "kitty_tooth",
    };
    let last_device = s("last_device").map(
        |id| json!({ "id": id, "name": s("last_device_name").unwrap_or_else(|| "Heart rate sensor".into()) }),
    );
    json!({
        "schema_version": 2,
        "device": {
            "last_device": last_device,
            "auto_connect": b("auto_connect").unwrap_or(true),
            "stale_timeout_secs": n("stale_timeout").unwrap_or(5),
        },
        "processing": { "smoothing": s("smoothing").unwrap_or_else(|| "off".into()) },
        "osc": {
            "targets": [{
                "id": "vrchat",
                "name": "VRChat",
                "host": s("osc_host").unwrap_or_else(|| "127.0.0.1".into()),
                "port": n("osc_port").unwrap_or(9000),
                "enabled": true,
                "preset": preset,
            }]
        },
        "ui": { "theme": s("theme").unwrap_or_else(|| "dark".into()) }
    })
}

/// v3 adds `source`. A remembered demo device means the demo source was in use.
fn migrate_v2_to_v3(mut v: Value) -> Value {
    let demo = v
        .pointer("/device/last_device/id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.starts_with(crate::sensor::DEMO_PREFIX));
    if let Some(o) = v.as_object_mut() {
        o.entry("source")
            .or_insert_with(|| json!({ "active": if demo { "demo" } else { "ble" } }));
        o.insert("schema_version".into(), json!(3));
    }
    v
}

/// Outcome of loading the config file.
#[derive(Debug, Clone)]
pub struct LoadReport {
    pub config: Config,
    /// Version the file was migrated from (if a migration happened).
    pub migrated_from: Option<u32>,
    /// Set when the file was unreadable and defaults were used (a backup is kept).
    pub recovered: Option<KtError>,
}

/// Loads and saves the config file atomically.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    dir: PathBuf,
}

impl ConfigStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(CONFIG_FILE)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Parses a config from JSON text, migrating as needed.
    pub fn parse(text: &str) -> KtResult<(Config, u32)> {
        let raw: Value = serde_json::from_str(text)?;
        let (migrated, from) = migrate(raw)?;
        let cfg: Config = serde_json::from_value(migrated)?;
        Ok((cfg.sanitized(), from))
    }

    /// Loads the config. Never fails: a missing file yields defaults, a corrupt file is
    /// backed up and replaced by defaults (reported in [`LoadReport::recovered`]).
    pub fn load(&self) -> LoadReport {
        let path = self.path();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return LoadReport {
                    config: Config::default(),
                    migrated_from: None,
                    recovered: None,
                }
            }
            Err(e) => {
                return LoadReport {
                    config: Config::default(),
                    migrated_from: None,
                    recovered: Some(e.into()),
                }
            }
        };
        match Self::parse(&text) {
            Ok((config, from)) => {
                let migrated_from = (from != CURRENT_SCHEMA_VERSION).then_some(from);
                if let Some(v) = migrated_from {
                    let _ = fs::write(self.dir.join(format!("config.v{v}.bak.json")), &text);
                    let _ = self.save(&config);
                }
                LoadReport {
                    config,
                    migrated_from,
                    recovered: None,
                }
            }
            Err(e) => {
                let backup = self.dir.join(format!(
                    "config.corrupt-{}.json",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                ));
                let _ = fs::write(backup, &text);
                LoadReport {
                    config: Config::default(),
                    migrated_from: None,
                    recovered: Some(e),
                }
            }
        }
    }

    /// Atomically writes the config (temp file + rename).
    pub fn save(&self, cfg: &Config) -> KtResult<()> {
        fs::create_dir_all(&self.dir)?;
        let text = serde_json::to_string_pretty(cfg)?;
        let tmp = self.dir.join(format!("{CONFIG_FILE}.tmp"));
        fs::write(&tmp, text)?;
        fs::rename(&tmp, self.path())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "kittytooth-test-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn default_roundtrip() {
        let c = Config::default();
        let s = serde_json::to_string(&c).unwrap();
        let (back, from) = ConfigStore::parse(&s).unwrap();
        assert_eq!(from, CURRENT_SCHEMA_VERSION);
        assert_eq!(back, c);
    }

    #[test]
    fn defaults_are_vrchat() {
        let c = Config::default();
        assert_eq!(c.osc.targets.len(), 1);
        assert_eq!(c.osc.targets[0].host, "127.0.0.1");
        assert_eq!(c.osc.targets[0].port, 9000);
        assert_eq!(c.osc.keepalive_ms, 1000);
        assert_eq!(c.osc.max_rate, 20);
        assert_eq!(c.device.stale_timeout_secs, 5);
        assert_eq!(c.ui.theme, Theme::Dark);
    }

    #[test]
    fn missing_fields_get_defaults() {
        let (c, _) = ConfigStore::parse(r#"{"schema_version":2,"device":{"auto_connect":false}}"#).unwrap();
        assert!(!c.device.auto_connect);
        assert_eq!(c.device.stale_timeout_secs, 5);
        assert_eq!(c.osc.targets[0].port, 9000);
    }

    #[test]
    fn unknown_fields_ignored() {
        let (c, _) = ConfigStore::parse(r#"{"schema_version":2,"future_thing":{"x":1}}"#).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn migrates_v1() {
        let v1 = r#"{
            "osc_host": "192.168.1.20", "osc_port": 9001, "preset": "hrtovrchat",
            "smoothing": "light", "last_device": "AA:BB:CC:DD:EE:FF",
            "last_device_name": "Polar H10 1234", "stale_timeout": 8, "theme": "light"
        }"#;
        let (c, from) = ConfigStore::parse(v1).unwrap();
        assert_eq!(from, 1);
        assert_eq!(c.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(c.osc.targets.len(), 1);
        assert_eq!(c.osc.targets[0].host, "192.168.1.20");
        assert_eq!(c.osc.targets[0].port, 9001);
        assert_eq!(c.osc.targets[0].preset, Preset::HrToVrchat);
        assert_eq!(c.processing.smoothing, Smoothing::Light);
        assert_eq!(c.device.stale_timeout_secs, 8);
        assert_eq!(c.ui.theme, Theme::Light);
        assert_eq!(
            c.device.last_device,
            Some(RememberedDevice {
                id: "AA:BB:CC:DD:EE:FF".into(),
                name: "Polar H10 1234".into()
            })
        );
    }

    #[test]
    fn newer_version_loads_known_fields() {
        let (c, from) =
            ConfigStore::parse(r#"{"schema_version":99,"device":{"stale_timeout_secs":9}}"#).unwrap();
        assert_eq!(from, 99);
        assert_eq!(c.device.stale_timeout_secs, 9);
        assert_eq!(c.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn sanitize_clamps() {
        let mut c = Config::default();
        c.device.stale_timeout_secs = 0;
        c.osc.max_rate = 500;
        c.osc.keepalive_ms = 5;
        c.osc.chatbox.interval_secs = 0.5;
        c.ui.accent = "pink".into();
        c.osc.targets.push(OscTarget {
            port: 0,
            host: "  ".into(),
            range: Some((200, 100)),
            ..OscTarget::default()
        });
        let c = c.sanitized();
        assert_eq!(c.device.stale_timeout_secs, 2);
        assert_eq!(c.osc.max_rate, 20);
        assert_eq!(c.osc.keepalive_ms, 200);
        assert_eq!(c.osc.chatbox.interval_secs, 2.0);
        assert_eq!(c.ui.accent, "pink");
        assert_eq!(c.osc.targets[1].port, 9000);
        assert_eq!(c.osc.targets[1].host, "127.0.0.1");
        assert_eq!(c.osc.targets[1].range, None);
        assert_ne!(c.osc.targets[0].id, c.osc.targets[1].id, "duplicate ids fixed");
    }

    #[test]
    fn keepalive_zero_means_disabled() {
        let mut c = Config::default();
        c.osc.keepalive_ms = 0;
        assert_eq!(c.sanitized().osc.keepalive_ms, 0);
    }

    #[test]
    fn store_save_load_and_missing_file() {
        let d = tmpdir("store");
        let store = ConfigStore::new(&d);
        let r = store.load();
        assert!(r.recovered.is_none());
        assert_eq!(r.config, Config::default());
        let mut c = Config::default();
        c.zones.age = Some(30);
        store.save(&c).unwrap();
        assert_eq!(store.load().config, c);
        fs::remove_dir_all(d).ok();
    }

    #[test]
    fn store_recovers_from_corrupt_file() {
        let d = tmpdir("corrupt");
        let store = ConfigStore::new(&d);
        fs::write(store.path(), "{ not json").unwrap();
        let r = store.load();
        assert!(r.recovered.is_some());
        assert_eq!(r.config, Config::default());
        let backups = fs::read_dir(&d)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("config.corrupt-")
            })
            .count();
        assert_eq!(backups, 1);
        fs::remove_dir_all(d).ok();
    }

    #[test]
    fn store_migrates_on_disk_with_backup() {
        let d = tmpdir("migrate");
        let store = ConfigStore::new(&d);
        fs::write(store.path(), r#"{"osc_port": 9002}"#).unwrap();
        let r = store.load();
        assert_eq!(r.migrated_from, Some(1));
        assert_eq!(r.config.osc.targets[0].port, 9002);
        assert!(d.join("config.v1.bak.json").exists());
        let on_disk: Value = serde_json::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap();
        assert_eq!(on_disk["schema_version"], CURRENT_SCHEMA_VERSION);
        assert_eq!(on_disk["source"]["active"], "ble");
        fs::remove_dir_all(d).ok();
    }

    #[test]
    fn migrates_v2_source() {
        let (c, from) = ConfigStore::parse(
            r#"{"schema_version":2,"device":{"last_device":{"id":"demo:strap","name":"Demo"}}}"#,
        )
        .unwrap();
        assert_eq!(from, 2);
        assert_eq!(c.source.active, SourceKind::Demo);
        let (c, _) = ConfigStore::parse(
            r#"{"schema_version":2,"device":{"last_device":{"id":"AA:BB","name":"H10"}}}"#,
        )
        .unwrap();
        assert_eq!(c.source.active, SourceKind::Ble);
        assert_eq!(c.source.osc_input.port, 9010);
        assert!(!c.source.fallback.enabled);
    }

    #[test]
    fn source_sanitize() {
        let mut c = Config::default();
        c.source.fallback.after_secs = 0;
        c.source.osc_input.port = 80;
        c.source.osc_input.address = "  ".into();
        c.source.hyperate.device_id = "  abc  ".into();
        let c = c.sanitized();
        assert_eq!(c.source.fallback.after_secs, 2);
        assert_eq!(c.source.osc_input.port, 9010);
        assert_eq!(c.source.osc_input.address, "/avatar/parameters/HR");
        assert_eq!(c.source.hyperate.device_id, "abc");
    }

    #[test]
    fn non_object_rejected() {
        assert!(ConfigStore::parse("[1,2]").is_err());
    }

    #[test]
    fn preset_ranges() {
        assert_eq!(Preset::HrToVrchat.default_range(), (0, 255));
        assert_eq!(Preset::Vrcosc.default_range(), (0, 240));
        let t = OscTarget {
            range: Some((50, 150)),
            ..Default::default()
        };
        assert_eq!(t.range(), (50, 150));
    }
}
