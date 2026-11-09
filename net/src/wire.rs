//! The wire format of an online party: five packets, each with one strict byte encoding.
//!
//! Only inputs and checkpoint hashes cross the network. A message is one datagram or one
//! WebSocket message, with no length prefix and no stream framing. All integers are little
//! endian and all widths are fixed. Bytes from a peer are untrusted (ADR-SVC-1): decoding fails
//! closed on anything but the one canonical encoding, and a value that cannot be decoded cannot
//! be encoded. A change to the layout raises [`PROTOCOL_VERSION`].
//!
//! The first byte of a packet is its tag: `0x01` hello, `0x02` input frame, `0x03` ack, `0x04`
//! checkpoint, `0x05` bye.

use std::fmt;

use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};

/// The wire protocol version, one integer; any change to the wire raises it.
pub const PROTOCOL_VERSION: u32 = 1;
/// The longest packet, in bytes. A longer message is refused on both sides.
pub const MAX_PACKET_LEN: usize = 1200;
/// The most inputs one input frame holds.
pub const MAX_FRAME_INPUTS: usize = 32;

/// The first packet a peer sends: what it runs, for the session to compare with its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    /// The sender's [`PROTOCOL_VERSION`]; not 0.
    pub protocol: u32,
    /// The sender's `sim_version`; not 0.
    pub sim_version: u32,
    /// The SHA-256 of the sender's content pack.
    pub pack_hash: [u8; 32],
}

/// One tick of one player's input, as it travels: the named fields of core's `Input` and
/// nothing else. A stick axis is −127 to 127.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WireInput {
    /// Stick x, −127 to 127.
    pub move_x: i8,
    /// Stick y, −127 to 127.
    pub move_y: i8,
    /// The Main button is held.
    pub main: bool,
    /// The smash button is held.
    pub smash: bool,
    /// The evade button is held.
    pub evade: bool,
    /// The skill button is held.
    pub skill: bool,
    /// Aim assist is on.
    pub assist: bool,
    /// Auto mode is on.
    pub auto: bool,
    /// The interact button is held.
    pub interact: bool,
    /// An equipment action made on this tick.
    pub action: Option<EquipAction>,
}

impl WireInput {
    /// The wire form of `input`; a stick axis of −128 becomes −127, as a replay block does.
    pub fn from_input(input: &Input) -> WireInput {
        let _ = input;
        todo!("from_input")
    }

    /// The core input this wire input stands for.
    pub fn to_input(&self) -> Input {
        todo!("to_input")
    }
}

/// One slot's inputs for `inputs.len()` consecutive ticks from `start_tick`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputFrame {
    /// The sender's own slot.
    pub slot: PlayerIdx,
    /// The tick of the first input.
    pub start_tick: u32,
    /// 1 to [`MAX_FRAME_INPUTS`] inputs, for ticks `start_tick`, `start_tick + 1`, ...
    pub inputs: Vec<WireInput>,
}

/// The receiver has every tick of `slot`'s inputs below `next_tick`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ack {
    /// Whose inputs are acknowledged.
    pub slot: PlayerIdx,
    /// Every tick below this was received; 0 means none.
    pub next_tick: u32,
}

/// The sender's state hash after `tick` ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// The sender.
    pub slot: PlayerIdx,
    /// A multiple of 60 (`CHECKPOINT_TICKS`), not 0.
    pub tick: u32,
    /// The state hash after that many ticks.
    pub hash: u64,
}

/// Why a peer leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByeReason {
    /// The player left (wire value 1).
    Left,
    /// The state hashes differ (wire value 2).
    Desync,
    /// The hello was refused: version or pack mismatch (wire value 3).
    Refused,
}

/// A peer is leaving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bye {
    /// The sender.
    pub slot: PlayerIdx,
    /// The tick the leave takes effect.
    pub tick: u32,
    /// Why.
    pub reason: ByeReason,
}

/// One message of the protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    /// Tag `0x01`.
    Hello(Hello),
    /// Tag `0x02`.
    Input(InputFrame),
    /// Tag `0x03`.
    Ack(Ack),
    /// Tag `0x04`.
    Checkpoint(Checkpoint),
    /// Tag `0x05`.
    Bye(Bye),
}

impl Packet {
    /// The one byte string of this packet, or the rule it breaks.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        todo!("encode")
    }

    /// The packet `bytes` hold, or the first rule they break.
    pub fn decode(bytes: &[u8]) -> Result<Packet, WireError> {
        let _ = bytes;
        todo!("decode")
    }
}

/// Why bytes are not a packet, or a packet cannot be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// Over [`MAX_PACKET_LEN`] bytes.
    TooLong,
    /// No bytes.
    Empty,
    /// The first byte is no tag.
    UnknownTag(u8),
    /// The bytes end before the packet does.
    Truncated,
    /// Bytes remain after the packet.
    TrailingBytes,
    /// A slot byte is over 2.
    SlotOutOfRange,
    /// A hello version is 0.
    ZeroVersion,
    /// An input frame count is 0 or over [`MAX_FRAME_INPUTS`].
    CountOutOfRange,
    /// The last tick of an input frame does not fit in `u32`.
    TickOverflow,
    /// A stick axis is −128.
    StickOutOfRange,
    /// An action kind is not 1 to 5.
    UnknownActionKind(u8),
    /// A checkpoint tick is 0 or not a multiple of 60.
    CheckpointTick,
    /// A bye reason is not 1 to 3.
    UnknownReason(u8),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::TooLong => write!(f, "packet over {MAX_PACKET_LEN} bytes"),
            WireError::Empty => f.write_str("empty packet"),
            WireError::UnknownTag(t) => write!(f, "unknown packet tag {t:#04x}"),
            WireError::Truncated => f.write_str("packet ends early"),
            WireError::TrailingBytes => f.write_str("bytes after the end of the packet"),
            WireError::SlotOutOfRange => f.write_str("slot out of range"),
            WireError::ZeroVersion => f.write_str("a version is 0"),
            WireError::CountOutOfRange => f.write_str("input count out of range"),
            WireError::TickOverflow => f.write_str("input ticks overflow"),
            WireError::StickOutOfRange => f.write_str("stick axis out of range"),
            WireError::UnknownActionKind(k) => write!(f, "unknown action kind {k}"),
            WireError::CheckpointTick => f.write_str("checkpoint tick is not a multiple of 60"),
            WireError::UnknownReason(r) => write!(f, "unknown bye reason {r}"),
        }
    }
}

impl std::error::Error for WireError {}
