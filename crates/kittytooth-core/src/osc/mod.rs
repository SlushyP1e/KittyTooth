//! OSC output: presets, encoding, rate-limited per-target senders, chatbox and OSCQuery.

pub mod chatbox;
pub mod oscquery;
pub mod presets;
pub mod sender;

use serde::Serialize;

use crate::error::{KtError, KtResult};

/// Prefix VRChat uses for avatar parameters.
pub const AVATAR_PREFIX: &str = "/avatar/parameters/";

/// A typed OSC argument.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum OscValue {
    Int(i32),
    Float(f32),
    Bool(bool),
}

impl OscValue {
    pub fn to_rosc(self) -> rosc::OscType {
        match self {
            OscValue::Int(i) => rosc::OscType::Int(i),
            OscValue::Float(f) => rosc::OscType::Float(f),
            OscValue::Bool(b) => rosc::OscType::Bool(b),
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            OscValue::Int(_) => "int",
            OscValue::Float(_) => "float",
            OscValue::Bool(_) => "bool",
        }
    }

    pub fn display(self) -> String {
        match self {
            OscValue::Int(i) => i.to_string(),
            OscValue::Float(f) => format!("{f:.3}"),
            OscValue::Bool(b) => b.to_string(),
        }
    }
}

/// Encodes a single OSC message.
pub fn encode_message(address: &str, args: Vec<rosc::OscType>) -> KtResult<Vec<u8>> {
    rosc::encoder::encode(&rosc::OscPacket::Message(rosc::OscMessage {
        addr: address.to_string(),
        args,
    }))
    .map_err(|e| KtError::InvalidOscAddress(format!("{address}: {e}")))
}

/// Normalises a user-entered address: bare names get the avatar parameter prefix.
pub fn normalize_address(input: &str) -> KtResult<String> {
    let s = input.trim();
    if s.is_empty() {
        return Err(KtError::InvalidOscAddress(input.to_string()));
    }
    let full = if s.starts_with('/') {
        s.to_string()
    } else {
        format!("{AVATAR_PREFIX}{s}")
    };
    let bad = |c: char| c.is_whitespace() || c.is_control() || "#*,?[]{}".contains(c);
    if full.len() > 256 || full.ends_with('/') || full.contains("//") || full.chars().any(bad) {
        return Err(KtError::InvalidOscAddress(input.to_string()));
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_normalization() {
        assert_eq!(normalize_address("HR").unwrap(), "/avatar/parameters/HR");
        assert_eq!(
            normalize_address(" Foo/Bar ").unwrap(),
            "/avatar/parameters/Foo/Bar"
        );
        assert_eq!(normalize_address("/custom/addr").unwrap(), "/custom/addr");
        for bad in [
            "",
            "  ",
            "has space",
            "/a//b",
            "/a/",
            "a#b",
            "x*",
            "a,b",
            "q?",
            "[x]",
            "{y}",
        ] {
            assert!(normalize_address(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn encodes_messages() {
        let b = encode_message("/avatar/parameters/HR", vec![OscValue::Int(72).to_rosc()]).unwrap();
        let (_, pkt) = rosc::decoder::decode_udp(&b).unwrap();
        match pkt {
            rosc::OscPacket::Message(m) => {
                assert_eq!(m.addr, "/avatar/parameters/HR");
                assert_eq!(m.args, vec![rosc::OscType::Int(72)]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn value_display() {
        assert_eq!(OscValue::Float(0.5).display(), "0.500");
        assert_eq!(OscValue::Int(3).kind(), "int");
        assert_eq!(OscValue::Bool(true).display(), "true");
    }
}
