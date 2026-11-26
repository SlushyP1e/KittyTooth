//! The small subset of the ANT serial message protocol KittyTooth needs to receive an ANT+
//! heart rate monitor: reset, network key, one wildcard slave channel and broadcast data.
//!
//! Every message is `SYNC (0xA4) | LEN | ID | DATA[LEN] | CHECKSUM`, where the checksum is the
//! XOR of all preceding bytes. Messages from the stick can arrive split across or packed into
//! USB transfers, so [`Deframer`] reassembles a byte stream and resynchronises on garbage.

/// Start of every ANT message.
pub const SYNC: u8 = 0xA4;

/// The ANT+ managed network key. It is public (published with the ANT+ device profiles and
/// shipped by every open-source ANT+ project); devices only talk to apps that set it.
pub const ANT_PLUS_NETWORK_KEY: [u8; 8] = [0xB9, 0xA5, 0x21, 0xFB, 0xBD, 0x72, 0xC3, 0x45];

/// ANT+ Heart Rate Monitor device profile.
pub const HRM_DEVICE_TYPE: u8 = 120;
/// ANT+ RF channel 57 (2457 MHz).
pub const ANT_PLUS_RF: u8 = 57;
/// HRM message period in 1/32768 s units (8070 = ~4.06 Hz).
pub const HRM_PERIOD: u16 = 8070;

// Message IDs.
pub const MSG_CHANNEL_EVENT: u8 = 0x40;
pub const MSG_ASSIGN_CHANNEL: u8 = 0x42;
pub const MSG_CHANNEL_PERIOD: u8 = 0x43;
pub const MSG_SEARCH_TIMEOUT: u8 = 0x44;
pub const MSG_RF_FREQUENCY: u8 = 0x45;
pub const MSG_NETWORK_KEY: u8 = 0x46;
pub const MSG_RESET: u8 = 0x4A;
pub const MSG_OPEN_CHANNEL: u8 = 0x4B;
pub const MSG_CLOSE_CHANNEL: u8 = 0x4C;
pub const MSG_REQUEST: u8 = 0x4D;
pub const MSG_BROADCAST_DATA: u8 = 0x4E;
pub const MSG_ACK_DATA: u8 = 0x4F;
pub const MSG_CHANNEL_ID: u8 = 0x51;
pub const MSG_STARTUP: u8 = 0x6F;

// Channel event codes (the third byte of a 0x40 message whose second byte is 0x01).
pub const EVENT_RX_SEARCH_TIMEOUT: u8 = 0x01;
pub const EVENT_RX_FAIL: u8 = 0x02;
pub const EVENT_CHANNEL_CLOSED: u8 = 0x07;
pub const EVENT_RX_FAIL_GO_TO_SEARCH: u8 = 0x08;
pub const RESPONSE_NO_ERROR: u8 = 0x00;

/// Channel type: bidirectional slave (receive).
pub const CHANNEL_TYPE_SLAVE: u8 = 0x00;
/// Search timeout value that means "search forever".
pub const SEARCH_FOREVER: u8 = 0xFF;

/// Builds one framed message.
pub fn encode(id: u8, data: &[u8]) -> Vec<u8> {
    debug_assert!(data.len() <= u8::MAX as usize);
    let mut out = Vec::with_capacity(data.len() + 4);
    out.push(SYNC);
    out.push(data.len() as u8);
    out.push(id);
    out.extend_from_slice(data);
    out.push(out.iter().fold(0, |a, b| a ^ b));
    out
}

pub fn reset() -> Vec<u8> {
    encode(MSG_RESET, &[0])
}

pub fn set_network_key(network: u8, key: &[u8; 8]) -> Vec<u8> {
    let mut d = vec![network];
    d.extend_from_slice(key);
    encode(MSG_NETWORK_KEY, &d)
}

pub fn assign_channel(channel: u8, channel_type: u8, network: u8) -> Vec<u8> {
    encode(MSG_ASSIGN_CHANNEL, &[channel, channel_type, network])
}

/// Device number 0 and transmission type 0 are wildcards: pair with any matching device.
pub fn set_channel_id(channel: u8, device_number: u16, device_type: u8, transmission_type: u8) -> Vec<u8> {
    let [lo, hi] = device_number.to_le_bytes();
    encode(MSG_CHANNEL_ID, &[channel, lo, hi, device_type, transmission_type])
}

pub fn set_channel_period(channel: u8, period: u16) -> Vec<u8> {
    let [lo, hi] = period.to_le_bytes();
    encode(MSG_CHANNEL_PERIOD, &[channel, lo, hi])
}

pub fn set_rf_frequency(channel: u8, rf: u8) -> Vec<u8> {
    encode(MSG_RF_FREQUENCY, &[channel, rf])
}

/// Search timeout in 2.5 s units; [`SEARCH_FOREVER`] never times out.
pub fn set_search_timeout(channel: u8, timeout: u8) -> Vec<u8> {
    encode(MSG_SEARCH_TIMEOUT, &[channel, timeout])
}

pub fn open_channel(channel: u8) -> Vec<u8> {
    encode(MSG_OPEN_CHANNEL, &[channel])
}

pub fn close_channel(channel: u8) -> Vec<u8> {
    encode(MSG_CLOSE_CHANNEL, &[channel])
}

/// Asks the stick which device the channel paired with (answered with a 0x51 message).
pub fn request_channel_id(channel: u8) -> Vec<u8> {
    encode(MSG_REQUEST, &[channel, MSG_CHANNEL_ID])
}

/// A decoded message from the stick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The stick finished a reset.
    Startup(u8),
    /// Reply to a configuration command: `code` 0 means success.
    Response { channel: u8, msg_id: u8, code: u8 },
    /// An RF event on a channel (search timeout, dropped packet, channel closed, ...).
    Event { channel: u8, code: u8 },
    /// An 8-byte data page (broadcast or acknowledged). Extended data, if any, is dropped.
    Data { channel: u8, page: [u8; 8] },
    /// The paired device's identity.
    ChannelId {
        channel: u8,
        device_number: u16,
        device_type: u8,
        transmission_type: u8,
    },
    /// Anything else (capabilities, serial numbers, ...).
    Other { id: u8, data: Vec<u8> },
}

impl Message {
    fn decode(id: u8, data: &[u8]) -> Message {
        match (id, data) {
            (MSG_STARTUP, [reason, ..]) => Message::Startup(*reason),
            (MSG_CHANNEL_EVENT, [channel, 0x01, code, ..]) => Message::Event {
                channel: *channel,
                code: *code,
            },
            (MSG_CHANNEL_EVENT, [channel, msg_id, code, ..]) => Message::Response {
                channel: *channel,
                msg_id: *msg_id,
                code: *code,
            },
            (MSG_BROADCAST_DATA | MSG_ACK_DATA, [channel, rest @ ..]) if rest.len() >= 8 => {
                let mut page = [0u8; 8];
                page.copy_from_slice(&rest[..8]);
                Message::Data {
                    channel: *channel,
                    page,
                }
            }
            (MSG_CHANNEL_ID, [channel, lo, hi, dt, tt, ..]) => Message::ChannelId {
                channel: *channel,
                device_number: u16::from_le_bytes([*lo, *hi]),
                device_type: *dt,
                transmission_type: *tt,
            },
            _ => Message::Other {
                id,
                data: data.to_vec(),
            },
        }
    }
}

/// Reassembles ANT messages from a byte stream.
#[derive(Debug, Default)]
pub struct Deframer {
    buf: Vec<u8>,
    /// Bytes thrown away while resynchronising (bad checksum, missing sync).
    pub dropped: usize,
}

impl Deframer {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Returns the next complete, checksum-valid message, or `None` if more bytes are needed.
    pub fn next_message(&mut self) -> Option<Message> {
        loop {
            match self.buf.iter().position(|&b| b == SYNC) {
                Some(0) => {}
                Some(n) => {
                    self.dropped += n;
                    self.buf.drain(..n);
                }
                None => {
                    self.dropped += self.buf.len();
                    self.buf.clear();
                    return None;
                }
            }
            if self.buf.len() < 2 {
                return None;
            }
            let total = self.buf[1] as usize + 4;
            if self.buf.len() < total {
                return None;
            }
            let checksum = self.buf[..total - 1].iter().fold(0, |a, b| a ^ b);
            if checksum != self.buf[total - 1] {
                // Not a real message start: skip this sync byte and look for the next one.
                self.dropped += 1;
                self.buf.drain(..1);
                continue;
            }
            let msg = Message::decode(self.buf[2], &self.buf[3..total - 1]);
            self.buf.drain(..total);
            return Some(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_known_frames() {
        // Reference frames from the ANT Message Protocol and Usage document.
        assert_eq!(reset(), vec![0xA4, 0x01, 0x4A, 0x00, 0xEF]);
        assert_eq!(open_channel(0), vec![0xA4, 0x01, 0x4B, 0x00, 0xEE]);
        assert_eq!(
            set_channel_id(0, 0, HRM_DEVICE_TYPE, 0),
            vec![0xA4, 0x05, 0x51, 0x00, 0x00, 0x00, 0x78, 0x00, 0x88]
        );
        assert_eq!(
            set_channel_period(0, HRM_PERIOD),
            vec![0xA4, 0x03, 0x43, 0x00, 0x86, 0x1F, 0x7D]
        );
        assert_eq!(
            set_rf_frequency(0, ANT_PLUS_RF),
            vec![0xA4, 0x02, 0x45, 0x00, 0x39, 0xDA]
        );
        let key = set_network_key(0, &ANT_PLUS_NETWORK_KEY);
        assert_eq!(key.len(), 13);
        assert_eq!(&key[..4], &[0xA4, 0x09, 0x46, 0x00]);
        assert_eq!(*key.last().unwrap(), key[..12].iter().fold(0, |a, b| a ^ b));
    }

    #[test]
    fn deframes_split_packed_and_noisy_streams() {
        // Fixture: a broadcast page followed by a channel-ID reply, delivered
        // in three USB transfers with line noise in front.
        let page = [
            0xA4, 0x09, 0x4E, 0x00, 0x84, 0x00, 0x5A, 0xB0, 0x6E, 0x47, 0x1A, 0x48, 0xF6,
        ];
        let chid = [0xA4, 0x05, 0x51, 0x00, 0x39, 0x30, 0x78, 0x01, 0x80];
        let mut stream = vec![0x00, 0xFF, 0xA4, 0x13];
        stream.extend_from_slice(&page);
        stream.extend_from_slice(&chid);
        let mut d = Deframer::default();
        d.push(&stream[..7]);
        assert_eq!(d.next_message(), None);
        d.push(&stream[7..15]);
        d.push(&stream[15..]);
        assert_eq!(
            d.next_message(),
            Some(Message::Data {
                channel: 0,
                page: [0x84, 0x00, 0x5A, 0xB0, 0x6E, 0x47, 0x1A, 0x48],
            })
        );
        assert_eq!(
            d.next_message(),
            Some(Message::ChannelId {
                channel: 0,
                device_number: 12345,
                device_type: HRM_DEVICE_TYPE,
                transmission_type: 1,
            })
        );
        assert_eq!(d.next_message(), None);
        assert_eq!(d.dropped, 4);
    }

    #[test]
    fn decodes_events_and_responses() {
        let mut d = Deframer::default();
        d.push(&encode(
            MSG_CHANNEL_EVENT,
            &[0, MSG_OPEN_CHANNEL, RESPONSE_NO_ERROR],
        ));
        d.push(&encode(MSG_CHANNEL_EVENT, &[0, 0x01, EVENT_RX_FAIL_GO_TO_SEARCH]));
        d.push(&encode(MSG_STARTUP, &[0x20]));
        d.push(&encode(0x54, &[8, 3, 0]));
        assert_eq!(
            d.next_message(),
            Some(Message::Response {
                channel: 0,
                msg_id: MSG_OPEN_CHANNEL,
                code: 0
            })
        );
        assert_eq!(
            d.next_message(),
            Some(Message::Event {
                channel: 0,
                code: EVENT_RX_FAIL_GO_TO_SEARCH
            })
        );
        assert_eq!(d.next_message(), Some(Message::Startup(0x20)));
        assert!(matches!(d.next_message(), Some(Message::Other { id: 0x54, .. })));
    }

    #[test]
    fn extended_broadcast_keeps_the_page() {
        // Broadcast with the extended-data flag (0x80) and channel ID appended.
        let mut data = vec![
            0, 0x04, 0x22, 0x10, 0x05, 0x6E, 0x09, 0x20, 0x3C, 0x80, 0x39, 0x30, 0x78, 0x01,
        ];
        let frame = encode(MSG_BROADCAST_DATA, &data);
        let mut d = Deframer::default();
        d.push(&frame);
        data.truncate(9);
        assert_eq!(
            d.next_message(),
            Some(Message::Data {
                channel: 0,
                page: data[1..9].try_into().unwrap(),
            })
        );
    }
}
