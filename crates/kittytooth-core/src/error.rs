//! Typed errors. Every error can be turned into a [`UserError`] with a plain-language
//! message and a concrete fix suggestion; the UI never shows raw error text or stack traces.

use serde::{Deserialize, Serialize};

/// All fallible operations in KittyTooth return this error type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KtError {
    #[error("no Bluetooth adapter found")]
    AdapterMissing,
    #[error("Bluetooth radio is off")]
    RadioOff,
    #[error("Bluetooth permission denied")]
    PermissionDenied,
    #[error("device {0} was not found")]
    DeviceNotFound(String),
    #[error("timed out connecting to {0}")]
    ConnectTimeout(String),
    #[error("device {0} is busy or connected to another app")]
    DeviceBusy(String),
    #[error("device {0} has no heart rate service")]
    NoHeartRateService(String),
    #[error("Bluetooth error: {0}")]
    Ble(String),
    #[error("invalid heart rate packet: {0}")]
    Parse(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("could not resolve OSC target {0}")]
    InvalidOscTarget(String),
    #[error("invalid OSC address {0}")]
    InvalidOscAddress(String),
    #[error("OSC socket error: {0}")]
    OscSocket(String),
    #[error("port {0} is already in use")]
    PortInUse(u16),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("engine is not running")]
    EngineStopped,
    #[error("no Bluetooth sensor chosen")]
    NoSensorChosen,
    #[error("missing credential {0}")]
    MissingCredential(&'static str),
    #[error("invalid device ID {0}")]
    InvalidDeviceId(String),
    #[error("token rejected: {0}")]
    BadToken(String),
    #[error("HypeRate API key rejected")]
    BadApiKey,
    #[error("Pulsoid plan required")]
    PremiumRequired,
    #[error("{0} unreachable")]
    Offline(&'static str),
    #[error("{0} rate limited")]
    RateLimited(&'static str),
    #[error("{0} error: {1}")]
    Service(&'static str, String),
}

/// What the UI shows for an error: never a stack trace, always a suggested fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserError {
    /// Stable machine-readable code (`adapter_missing`, `radio_off`, ...).
    pub code: String,
    /// Short headline, e.g. "Bluetooth is turned off".
    pub title: String,
    /// One-sentence explanation in plain language.
    pub message: String,
    /// What the user can do about it.
    pub fix: String,
}

impl KtError {
    pub fn code(&self) -> &'static str {
        match self {
            KtError::AdapterMissing => "adapter_missing",
            KtError::RadioOff => "radio_off",
            KtError::PermissionDenied => "permission_denied",
            KtError::DeviceNotFound(_) => "device_not_found",
            KtError::ConnectTimeout(_) => "connect_timeout",
            KtError::DeviceBusy(_) => "device_busy",
            KtError::NoHeartRateService(_) => "no_hr_service",
            KtError::Ble(_) => "ble_error",
            KtError::Parse(_) => "parse_error",
            KtError::Config(_) => "config_error",
            KtError::InvalidOscTarget(_) => "invalid_osc_target",
            KtError::InvalidOscAddress(_) => "invalid_osc_address",
            KtError::OscSocket(_) => "osc_socket",
            KtError::PortInUse(_) => "port_in_use",
            KtError::Io(_) => "io_error",
            KtError::InvalidInput(_) => "invalid_input",
            KtError::EngineStopped => "engine_stopped",
            KtError::NoSensorChosen => "no_sensor_chosen",
            KtError::MissingCredential(_) => "missing_credential",
            KtError::InvalidDeviceId(_) => "invalid_device_id",
            KtError::BadToken(_) => "bad_token",
            KtError::BadApiKey => "bad_api_key",
            KtError::PremiumRequired => "premium_required",
            KtError::Offline(_) => "offline",
            KtError::RateLimited(_) => "rate_limited",
            KtError::Service(..) => "service_error",
        }
    }

    /// Whether retrying with the same settings can succeed. Rejected credentials and IDs stop
    /// the reconnect loop until the user changes something.
    pub fn retriable(&self) -> bool {
        !matches!(
            self,
            KtError::NoSensorChosen
                | KtError::MissingCredential(_)
                | KtError::InvalidDeviceId(_)
                | KtError::BadToken(_)
                | KtError::BadApiKey
                | KtError::PremiumRequired
                | KtError::InvalidOscAddress(_)
        )
    }

    /// Plain-language description with a fix suggestion.
    pub fn user(&self) -> UserError {
        let (title, message, fix): (&str, String, String) = match self {
            KtError::AdapterMissing => (
                "No Bluetooth adapter",
                "KittyTooth could not find a Bluetooth adapter on this PC.".into(),
                "Plug in a Bluetooth 4.0+ (LE) dongle, or enable Bluetooth in Device Manager / BIOS, then KittyTooth will pick it up automatically.".into(),
            ),
            KtError::RadioOff => (
                "Bluetooth is turned off",
                "The Bluetooth radio is switched off, so sensors can't be reached.".into(),
                "Turn Bluetooth on in Windows Settings > Bluetooth & devices (or the Action Center). KittyTooth will reconnect on its own.".into(),
            ),
            KtError::PermissionDenied => (
                "Bluetooth access denied",
                "The operating system refused KittyTooth access to Bluetooth.".into(),
                "Open Windows Settings > Privacy & security > Radios and allow apps to control radios, then try again.".into(),
            ),
            KtError::DeviceNotFound(name) => (
                "Sensor not found",
                format!("{name} isn't advertising right now."),
                "Wake the sensor (wet the strap electrodes and put it on, or enable HR broadcast on your watch) and keep it within a few metres.".into(),
            ),
            KtError::ConnectTimeout(name) => (
                "Sensor not responding",
                format!("{name} didn't answer the connection request in time."),
                "Most straps allow only one connection. Close other apps using it (Zwift, Polar Beat, phone apps) or turn your phone's Bluetooth off. KittyTooth keeps retrying.".into(),
            ),
            KtError::DeviceBusy(name) => (
                "Sensor is busy",
                format!("{name} appears to be connected to another app or device."),
                "Disconnect it from other apps/phones. If it is paired in Windows to another app, remove it in Settings > Bluetooth & devices and let KittyTooth connect directly.".into(),
            ),
            KtError::NoHeartRateService(name) => (
                "Not a heart rate sensor",
                format!("{name} doesn't offer the standard Bluetooth heart rate service."),
                "Pick a device marked with a heart in the Devices list. For watches, enable \"HR broadcast\" / \"Broadcast heart rate\" mode first.".into(),
            ),
            KtError::Ble(detail) => (
                "Bluetooth problem",
                format!("Bluetooth reported a problem ({}).", short(detail)),
                "Toggle Bluetooth off and on. If it keeps happening, update your Bluetooth driver or try a different USB port for your dongle.".into(),
            ),
            KtError::Parse(_) => (
                "Unreadable sensor data",
                "The sensor sent data that doesn't follow the heart rate standard.".into(),
                "Update the sensor firmware. If this persists, use Copy diagnostics in Settings and report it.".into(),
            ),
            KtError::Config(detail) => (
                "Settings problem",
                format!("Your settings couldn't be loaded or saved ({}).", short(detail)),
                "KittyTooth keeps a backup and falls back to defaults. Check that your user folder isn't read-only or full.".into(),
            ),
            KtError::InvalidOscTarget(target) => (
                "OSC target not reachable",
                format!("The OSC target \"{target}\" isn't a valid host:port."),
                "Use an IP address or hostname, e.g. 127.0.0.1 and port 9000 for VRChat on the same PC.".into(),
            ),
            KtError::InvalidOscAddress(addr) => (
                "Invalid OSC address",
                format!("\"{addr}\" isn't a valid OSC address."),
                "Addresses start with / and may not contain spaces or # * , ? [ ] { }. A bare name like MyHR becomes /avatar/parameters/MyHR.".into(),
            ),
            KtError::OscSocket(detail) => (
                "Couldn't send OSC",
                format!("Sending OSC data failed ({}).", short(detail)),
                "Check the target host and port. A firewall or VPN may block local UDP; allow KittyTooth in Windows Defender Firewall.".into(),
            ),
            KtError::PortInUse(port) => (
                "Port already in use",
                format!("Port {port} is used by another program."),
                "Pick a different port in Settings, or close the other program.".into(),
            ),
            KtError::Io(detail) => (
                "File problem",
                format!("A file operation failed ({}).", short(detail)),
                "Make sure the file isn't open in another program and that the folder is writable.".into(),
            ),
            KtError::InvalidInput(detail) => (
                "Invalid value",
                format!("{}.", capitalize(detail)),
                "Correct the highlighted value and try again.".into(),
            ),
            KtError::EngineStopped => (
                "KittyTooth stopped",
                "The background engine isn't running.".into(),
                "Restart KittyTooth. If this happens again, use Copy diagnostics in Settings and report it.".into(),
            ),
            KtError::NoSensorChosen => (
                "No sensor chosen",
                "Bluetooth is the active source, but no sensor has been picked yet.".into(),
                "Open Devices, scan, and connect your strap or watch. KittyTooth remembers it.".into(),
            ),
            KtError::MissingCredential(which) => match *which {
                "pulsoid_token" => (
                    "Pulsoid token needed",
                    "KittyTooth needs your Pulsoid access token to read your heart rate.".into(),
                    "Press Get token on the Source page, create a token on Pulsoid, and paste it into the token field.".into(),
                ),
                _ => (
                    "HypeRate API key needed",
                    "This build has no HypeRate API key, and none has been entered.".into(),
                    "Paste a HypeRate developer API key on the Source page (request one at hyperate.io/api).".into(),
                ),
            },
            KtError::InvalidDeviceId(id) => (
                "HypeRate device ID not accepted",
                if id.is_empty() {
                    "No HypeRate device ID has been entered.".into()
                } else {
                    format!("HypeRate didn't accept the device ID \"{id}\".")
                },
                "Copy the ID exactly as the HypeRate app shows it (letters and digits only) into the Device ID field.".into(),
            ),
            KtError::BadToken(_) => (
                "Pulsoid token rejected",
                "Pulsoid says the access token is invalid, revoked or expired.".into(),
                "Create a new token with Get token on the Source page and paste it in. KittyTooth stops retrying until you do.".into(),
            ),
            KtError::BadApiKey => (
                "HypeRate API key rejected",
                "HypeRate refused the developer API key.".into(),
                "Check the API key override on the Source page, or clear it to use the built-in key.".into(),
            ),
            KtError::PremiumRequired => (
                "Pulsoid plan needed",
                "Pulsoid only allows API tokens on its BRO plan or trial.".into(),
                "Start the Pulsoid BRO plan or trial, or switch to Bluetooth or another source.".into(),
            ),
            KtError::Offline(service) => (
                "Can't reach the service",
                format!("{service} can't be reached from this PC right now."),
                "Check your internet connection, VPN and firewall. KittyTooth keeps retrying.".into(),
            ),
            KtError::RateLimited(service) => (
                "Too many connections",
                format!("{service} is rate limiting connections from this PC."),
                "Close other apps or browser tabs reading the same heart rate. KittyTooth waits and retries.".into(),
            ),
            KtError::Service(service, detail) => (
                "Service problem",
                format!("{service} reported a problem ({}).", short(detail)),
                "This is usually temporary. KittyTooth keeps retrying; check the service's status page if it lasts.".into(),
            ),
        };
        UserError {
            code: self.code().into(),
            title: title.into(),
            message,
            fix,
        }
    }
}

fn short(detail: &str) -> String {
    let d = detail.lines().next().unwrap_or("").trim();
    if d.chars().count() > 80 {
        let mut s: String = d.chars().take(77).collect();
        s.push_str("...");
        s
    } else {
        d.to_string()
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

impl From<std::io::Error> for KtError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::PermissionDenied => KtError::Io(format!("permission denied: {e}")),
            _ => KtError::Io(e.to_string()),
        }
    }
}

impl From<serde_json::Error> for KtError {
    fn from(e: serde_json::Error) -> Self {
        KtError::Config(e.to_string())
    }
}

pub type KtResult<T> = Result<T, KtError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_message_and_fix() {
        let all = [
            KtError::AdapterMissing,
            KtError::RadioOff,
            KtError::PermissionDenied,
            KtError::DeviceNotFound("H10".into()),
            KtError::ConnectTimeout("H10".into()),
            KtError::DeviceBusy("H10".into()),
            KtError::NoHeartRateService("Mouse".into()),
            KtError::Ble("x".into()),
            KtError::Parse("x".into()),
            KtError::Config("x".into()),
            KtError::InvalidOscTarget("x".into()),
            KtError::InvalidOscAddress("x".into()),
            KtError::OscSocket("x".into()),
            KtError::PortInUse(9000),
            KtError::Io("x".into()),
            KtError::InvalidInput("port must be 1-65535".into()),
            KtError::EngineStopped,
            KtError::NoSensorChosen,
            KtError::MissingCredential("pulsoid_token"),
            KtError::MissingCredential("hyperate_key"),
            KtError::InvalidDeviceId("x".into()),
            KtError::InvalidDeviceId(String::new()),
            KtError::BadToken("token_expired".into()),
            KtError::BadApiKey,
            KtError::PremiumRequired,
            KtError::Offline("Pulsoid"),
            KtError::RateLimited("HypeRate"),
            KtError::Service("Pulsoid", "HTTP 503".into()),
        ];
        for e in all {
            let u = e.user();
            assert!(!u.title.is_empty() && !u.message.is_empty() && !u.fix.is_empty());
            assert_eq!(u.code, e.code());
        }
    }

    #[test]
    fn long_details_are_shortened() {
        let e = KtError::Ble("a".repeat(500) + "\nstack frame 1\nstack frame 2");
        let u = e.user();
        assert!(u.message.len() < 140);
        assert!(!u.message.contains("stack frame"));
    }
}
