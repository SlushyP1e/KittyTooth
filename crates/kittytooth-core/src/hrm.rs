//! Heart Rate Measurement characteristic (`0x2A37`) parser and encoder.
//!
//! Layout per the Bluetooth SIG Heart Rate Service 1.0 / GATT Specification Supplement:
//!
//! | bit | flag                                               |
//! |-----|----------------------------------------------------|
//! | 0   | Heart Rate Value Format: 0 = UINT8, 1 = UINT16     |
//! | 1-2 | Sensor Contact Status (see [`SensorContact`])      |
//! | 3   | Energy Expended present (UINT16, kilojoules)       |
//! | 4   | RR-Interval present (one or more UINT16, 1/1024 s) |
//! | 5-7 | Reserved for future use (ignored)                  |
//!
//! Multi-byte fields are little endian. RR-intervals fill the rest of the packet.

use serde::{Deserialize, Serialize};

use crate::error::KtError;

pub const FLAG_HR_U16: u8 = 0x01;
pub const FLAG_CONTACT_DETECTED: u8 = 0x02;
pub const FLAG_CONTACT_SUPPORTED: u8 = 0x04;
pub const FLAG_ENERGY: u8 = 0x08;
pub const FLAG_RR: u8 = 0x10;

/// Sensor contact status from flag bits 1-2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SensorContact {
    /// Values 0 and 1: the sensor does not report skin contact.
    #[default]
    NotSupported,
    /// Value 2: contact reporting supported, but no contact detected (strap off / dry).
    NotDetected,
    /// Value 3: contact reporting supported and good contact.
    Detected,
}

impl SensorContact {
    pub fn from_flags(flags: u8) -> Self {
        match (flags >> 1) & 0b11 {
            0b10 => SensorContact::NotDetected,
            0b11 => SensorContact::Detected,
            _ => SensorContact::NotSupported,
        }
    }

    pub fn to_flags(self) -> u8 {
        match self {
            SensorContact::NotSupported => 0,
            SensorContact::NotDetected => FLAG_CONTACT_SUPPORTED,
            SensorContact::Detected => FLAG_CONTACT_SUPPORTED | FLAG_CONTACT_DETECTED,
        }
    }

    /// `false` only when the sensor positively reports lost contact.
    pub fn has_contact(self) -> bool {
        !matches!(self, SensorContact::NotDetected)
    }
}

/// A decoded Heart Rate Measurement notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartRateMeasurement {
    /// Heart rate in beats per minute.
    pub bpm: u16,
    /// `true` if the value was encoded as UINT16.
    pub u16_format: bool,
    pub contact: SensorContact,
    /// Cumulative energy expended in kilojoules, if present.
    pub energy_expended_kj: Option<u16>,
    /// Raw RR-intervals in units of 1/1024 second, oldest first.
    pub rr_intervals: Vec<u16>,
}

impl HeartRateMeasurement {
    /// A plain 8-bit measurement with no optional fields.
    pub fn simple(bpm: u8) -> Self {
        Self {
            bpm: bpm as u16,
            u16_format: false,
            contact: SensorContact::NotSupported,
            energy_expended_kj: None,
            rr_intervals: Vec::new(),
        }
    }

    /// RR-intervals converted to milliseconds.
    pub fn rr_ms(&self) -> impl Iterator<Item = f64> + '_ {
        self.rr_intervals.iter().map(|&r| rr_to_ms(r))
    }
}

/// Converts a raw RR-interval (1/1024 s) to milliseconds.
pub fn rr_to_ms(raw: u16) -> f64 {
    raw as f64 * 1000.0 / 1024.0
}

/// Converts milliseconds to a raw RR-interval (1/1024 s), rounded and clamped.
pub fn ms_to_rr(ms: f64) -> u16 {
    (ms * 1024.0 / 1000.0).round().clamp(0.0, u16::MAX as f64) as u16
}

/// Parses a `0x2A37` notification payload.
///
/// Errors only on truncated mandatory/optional fields. A dangling odd byte at the end of the
/// RR-interval area is ignored (seen on some cheap straps).
pub fn parse(data: &[u8]) -> Result<HeartRateMeasurement, KtError> {
    let (&flags, rest) = data
        .split_first()
        .ok_or_else(|| KtError::Parse("empty packet".into()))?;
    let u16_format = flags & FLAG_HR_U16 != 0;
    let mut idx = 0usize;
    let take_u16 = |idx: &mut usize, what: &str| -> Result<u16, KtError> {
        let b = rest
            .get(*idx..*idx + 2)
            .ok_or_else(|| KtError::Parse(format!("truncated {what}")))?;
        *idx += 2;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    };
    let bpm = if u16_format {
        take_u16(&mut idx, "heart rate")?
    } else {
        let b = *rest
            .first()
            .ok_or_else(|| KtError::Parse("truncated heart rate".into()))?;
        idx += 1;
        b as u16
    };
    let energy_expended_kj = if flags & FLAG_ENERGY != 0 {
        Some(take_u16(&mut idx, "energy expended")?)
    } else {
        None
    };
    let mut rr_intervals = Vec::new();
    if flags & FLAG_RR != 0 {
        while idx + 2 <= rest.len() {
            rr_intervals.push(take_u16(&mut idx, "rr interval")?);
        }
    }
    Ok(HeartRateMeasurement {
        bpm,
        u16_format,
        contact: SensorContact::from_flags(flags),
        energy_expended_kj,
        rr_intervals,
    })
}

/// Encodes a measurement into a `0x2A37` payload (used by the mock sensor and tests).
pub fn encode(m: &HeartRateMeasurement) -> Vec<u8> {
    let mut flags = m.contact.to_flags();
    let u16_format = m.u16_format || m.bpm > u8::MAX as u16;
    if u16_format {
        flags |= FLAG_HR_U16;
    }
    if m.energy_expended_kj.is_some() {
        flags |= FLAG_ENERGY;
    }
    if !m.rr_intervals.is_empty() {
        flags |= FLAG_RR;
    }
    let mut out = Vec::with_capacity(1 + 2 + 2 + m.rr_intervals.len() * 2);
    out.push(flags);
    if u16_format {
        out.extend_from_slice(&m.bpm.to_le_bytes());
    } else {
        out.push(m.bpm as u8);
    }
    if let Some(e) = m.energy_expended_kj {
        out.extend_from_slice(&e.to_le_bytes());
    }
    for rr in &m.rr_intervals {
        out.extend_from_slice(&rr.to_le_bytes());
    }
    out
}

/// Body Sensor Location (`0x2A38`) values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodySensorLocation {
    Other,
    Chest,
    Wrist,
    Finger,
    Hand,
    EarLobe,
    Foot,
    Unknown,
}

impl BodySensorLocation {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::Other,
            1 => Self::Chest,
            2 => Self::Wrist,
            3 => Self::Finger,
            4 => Self::Hand,
            5 => Self::EarLobe,
            6 => Self::Foot,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Other => "Other",
            Self::Chest => "Chest",
            Self::Wrist => "Wrist",
            Self::Finger => "Finger",
            Self::Hand => "Hand",
            Self::EarLobe => "Ear lobe",
            Self::Foot => "Foot",
            Self::Unknown => "Unknown",
        }
    }
}

/// Battery Level (`0x2A19`): a single UINT8 percentage 0-100.
pub fn parse_battery(data: &[u8]) -> Option<u8> {
    data.first().map(|&b| b.min(100))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_example_uint8_minimal() {
        // flags 0x00: UINT8, contact not supported, nothing else.
        let m = parse(&[0x00, 72]).unwrap();
        assert_eq!(m.bpm, 72);
        assert!(!m.u16_format);
        assert_eq!(m.contact, SensorContact::NotSupported);
        assert_eq!(m.energy_expended_kj, None);
        assert!(m.rr_intervals.is_empty());
    }

    #[test]
    fn spec_example_uint16() {
        let m = parse(&[0x01, 0x2C, 0x01]).unwrap(); // 300 bpm (out of human range, still decoded)
        assert_eq!(m.bpm, 300);
        assert!(m.u16_format);
    }

    #[test]
    fn polar_h10_style_packet() {
        // Real-world Polar H10: flags 0x16 = contact supported+detected, RR present.
        // HR 62, RR 0x03D5 (981/1024 s = 958 ms).
        let m = parse(&[0x16, 0x3E, 0xD5, 0x03]).unwrap();
        assert_eq!(m.bpm, 62);
        assert_eq!(m.contact, SensorContact::Detected);
        assert_eq!(m.rr_intervals, vec![0x03D5]);
        let ms: Vec<f64> = m.rr_ms().collect();
        assert!((ms[0] - 958.0).abs() < 0.1);
    }

    #[test]
    fn energy_and_multiple_rr() {
        // flags: u16 HR, contact detected, energy, rr
        let bytes = [0x1F, 0x8C, 0x00, 0x10, 0x27, 0x00, 0x04, 0x10, 0x04, 0xF0, 0x03];
        let m = parse(&bytes).unwrap();
        assert_eq!(m.bpm, 140);
        assert_eq!(m.energy_expended_kj, Some(10_000));
        assert_eq!(m.rr_intervals, vec![1024, 1040, 1008]);
        assert_eq!(rr_to_ms(1024), 1000.0);
    }

    #[test]
    fn contact_status_values() {
        assert_eq!(SensorContact::from_flags(0b000), SensorContact::NotSupported);
        assert_eq!(SensorContact::from_flags(0b010), SensorContact::NotSupported); // value 1
        assert_eq!(SensorContact::from_flags(0b100), SensorContact::NotDetected); // value 2
        assert_eq!(SensorContact::from_flags(0b110), SensorContact::Detected); // value 3
        assert!(SensorContact::NotSupported.has_contact());
        assert!(!SensorContact::NotDetected.has_contact());
    }

    #[test]
    fn reserved_bits_are_ignored() {
        let m = parse(&[0xE0, 80]).unwrap();
        assert_eq!(m.bpm, 80);
        assert_eq!(m.contact, SensorContact::NotSupported);
    }

    #[test]
    fn rr_flag_without_values_is_empty() {
        let m = parse(&[FLAG_RR, 70]).unwrap();
        assert!(m.rr_intervals.is_empty());
    }

    #[test]
    fn dangling_rr_byte_is_ignored() {
        let m = parse(&[FLAG_RR, 70, 0x00, 0x04, 0x99]).unwrap();
        assert_eq!(m.rr_intervals, vec![1024]);
    }

    #[test]
    fn truncated_packets_error() {
        assert!(parse(&[]).is_err());
        assert!(parse(&[0x00]).is_err());
        assert!(parse(&[0x01, 0x50]).is_err());
        assert!(parse(&[FLAG_ENERGY, 70, 0x01]).is_err());
        assert!(parse(&[FLAG_ENERGY | FLAG_HR_U16, 70, 0]).is_err());
    }

    /// Exhaustively covers every combination of the five defined flag bits (32 combos),
    /// each with zero, one and three RR intervals, round-tripping through encode/parse
    /// and checking the exact byte layout.
    #[test]
    fn every_flag_combination_roundtrips() {
        for flags in 0u8..32 {
            for rr_count in [0usize, 1, 3] {
                let u16_format = flags & FLAG_HR_U16 != 0;
                let has_energy = flags & FLAG_ENERGY != 0;
                let has_rr = flags & FLAG_RR != 0;
                let bpm: u16 = if u16_format { 0x0123 } else { 0x55 };
                let mut bytes = vec![flags];
                if u16_format {
                    bytes.extend_from_slice(&bpm.to_le_bytes());
                } else {
                    bytes.push(bpm as u8);
                }
                if has_energy {
                    bytes.extend_from_slice(&0xBEEFu16.to_le_bytes());
                }
                let rr: Vec<u16> = (0..rr_count).map(|i| 800 + i as u16 * 17).collect();
                if has_rr {
                    for r in &rr {
                        bytes.extend_from_slice(&r.to_le_bytes());
                    }
                }
                let m = parse(&bytes).unwrap_or_else(|e| panic!("flags {flags:#04x}: {e}"));
                assert_eq!(m.bpm, bpm, "flags {flags:#04x}");
                assert_eq!(m.u16_format, u16_format);
                assert_eq!(m.contact, SensorContact::from_flags(flags));
                assert_eq!(m.energy_expended_kj, has_energy.then_some(0xBEEF));
                if has_rr {
                    assert_eq!(m.rr_intervals, rr);
                } else {
                    assert!(m.rr_intervals.is_empty());
                }
                // Re-encode: identical bytes unless RR flag was set with zero intervals
                // (the encoder omits an empty RR flag) or contact value 1 (normalised to 0).
                let re = encode(&m);
                let mut expected = bytes.clone();
                if has_rr && rr_count == 0 {
                    expected[0] &= !FLAG_RR;
                }
                if (flags >> 1) & 0b11 == 0b01 {
                    expected[0] &= !FLAG_CONTACT_DETECTED;
                }
                assert_eq!(re, expected, "flags {flags:#04x} rr {rr_count}");
                assert_eq!(parse(&re).unwrap().bpm, bpm);
            }
        }
    }

    #[test]
    fn encoder_promotes_large_bpm_to_u16() {
        let m = HeartRateMeasurement {
            bpm: 300,
            ..HeartRateMeasurement::simple(0)
        };
        let b = encode(&m);
        assert_eq!(b[0] & FLAG_HR_U16, FLAG_HR_U16);
        assert_eq!(parse(&b).unwrap().bpm, 300);
    }

    #[test]
    fn rr_conversion_roundtrip() {
        for ms in [300.0, 857.4, 1000.0, 1999.0] {
            let raw = ms_to_rr(ms);
            assert!((rr_to_ms(raw) - ms).abs() < 1.0);
        }
    }

    #[test]
    fn battery_and_location() {
        assert_eq!(parse_battery(&[87]), Some(87));
        assert_eq!(parse_battery(&[250]), Some(100));
        assert_eq!(parse_battery(&[]), None);
        assert_eq!(BodySensorLocation::from_byte(1), BodySensorLocation::Chest);
        assert_eq!(BodySensorLocation::from_byte(2), BodySensorLocation::Wrist);
        assert_eq!(BodySensorLocation::from_byte(9), BodySensorLocation::Unknown);
        assert_eq!(BodySensorLocation::EarLobe.label(), "Ear lobe");
    }
}
