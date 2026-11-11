//! The wire format of an online party: five kinds of packet, each with one strict byte encoding
//! (the hello has two layouts, the current one and the retired protocol 1 one).
//!
//! Only inputs and checkpoint hashes cross the network. A message is one datagram or one
//! WebSocket message, with no length prefix and no stream framing. All integers are little
//! endian and all widths are fixed. Bytes from a peer are untrusted (ADR-SVC-1): decoding fails
//! closed on anything but the one canonical encoding, and a value that cannot be decoded cannot
//! be encoded. A change to the layout raises [`PROTOCOL_VERSION`].
//!
//! The first byte of a packet is its tag: `0x06` hello (protocol 2), `0x01` the retired protocol 1
//! hello (never given another meaning), `0x02` input frame, `0x03` ack, `0x04` checkpoint, `0x05`
//! bye. A tag from `0x07` up is refused.

use std::fmt;

use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_replay::CHECKPOINT_TICKS;

/// The wire protocol version, one integer; any change to the wire raises it.
pub const PROTOCOL_VERSION: u32 = 2;
/// The longest packet, in bytes. A longer message is refused on both sides.
pub const MAX_PACKET_LEN: usize = 1200;
/// The most inputs one input frame holds.
pub const MAX_FRAME_INPUTS: usize = 32;

/// The first packet a peer sends: what it runs and how it is set up, for the session to compare
/// with its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    /// The sender's [`PROTOCOL_VERSION`]; not 0.
    pub protocol: u32,
    /// The sender's `sim_version`; not 0.
    pub sim_version: u32,
    /// The SHA-256 of the sender's content pack.
    pub pack_hash: [u8; 32],
    /// The sender's input delay in ticks; any `u32` on the wire, the session compares it.
    pub delay: u32,
    /// The sender's rollback window in ticks; any `u32` on the wire, the session compares it.
    pub window: u32,
}

/// The protocol 1 hello, kept decodable so that a protocol 1 peer is refused, not dropped as
/// garbage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelloV1 {
    /// The sender's protocol version; not 0.
    pub protocol: u32,
    /// The sender's `sim_version`; not 0.
    pub sim_version: u32,
    /// The SHA-256 of the sender's content pack.
    pub pack_hash: [u8; 32],
}

/// One tick of one player's input, as it travels: the stick, the buttons, the switches and the
/// equipment action of core's `Input`. The choice made from an open offer is not on the wire: it
/// is not predicted or exchanged here. A stick axis is −127 to 127.
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
        WireInput {
            move_x: input.move_x.max(-i8::MAX),
            move_y: input.move_y.max(-i8::MAX),
            main: input.main,
            smash: input.smash,
            evade: input.evade,
            skill: input.skill,
            assist: input.assist,
            auto: input.auto,
            interact: input.interact,
            action: input.action,
        }
    }

    /// The core input this wire input stands for.
    pub fn to_input(&self) -> Input {
        Input {
            move_x: self.move_x,
            move_y: self.move_y,
            main: self.main,
            smash: self.smash,
            evade: self.evade,
            skill: self.skill,
            assist: self.assist,
            auto: self.auto,
            interact: self.interact,
            action: self.action,
        }
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
    /// The hello was refused: version, pack or settings mismatch (wire value 3).
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
    /// Tag `0x06`.
    Hello(Hello),
    /// Tag `0x01`: a protocol 1 hello, kept so that a v1 peer is refused, not dropped as garbage.
    HelloV1(HelloV1),
    /// Tag `0x02`.
    Input(InputFrame),
    /// Tag `0x03`.
    Ack(Ack),
    /// Tag `0x04`.
    Checkpoint(Checkpoint),
    /// Tag `0x05`.
    Bye(Bye),
}

const TAG_HELLO_V1: u8 = 0x01;
const TAG_INPUT: u8 = 0x02;
const TAG_ACK: u8 = 0x03;
const TAG_CHECKPOINT: u8 = 0x04;
const TAG_BYE: u8 = 0x05;
const TAG_HELLO: u8 = 0x06;

/// A cursor over untrusted bytes: every read says `Truncated` instead of panicking.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let (head, rest) = self.rest.split_at_checked(n).ok_or(WireError::Truncated)?;
        self.rest = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn finish(self) -> Result<(), WireError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }
}

fn encode_hello(h: &Hello, out: &mut Vec<u8>) -> Result<(), WireError> {
    if h.protocol == 0 || h.sim_version == 0 {
        return Err(WireError::ZeroVersion);
    }
    out.push(TAG_HELLO);
    out.extend_from_slice(&h.protocol.to_le_bytes());
    out.extend_from_slice(&h.sim_version.to_le_bytes());
    out.extend_from_slice(&h.pack_hash);
    out.extend_from_slice(&h.delay.to_le_bytes());
    out.extend_from_slice(&h.window.to_le_bytes());
    Ok(())
}

/// The versions and pack hash both hello layouts start with.
fn decode_hello_head(r: &mut Reader<'_>) -> Result<(u32, u32, [u8; 32]), WireError> {
    let protocol = r.u32()?;
    if protocol == 0 {
        return Err(WireError::ZeroVersion);
    }
    let sim_version = r.u32()?;
    if sim_version == 0 {
        return Err(WireError::ZeroVersion);
    }
    let mut pack_hash = [0u8; 32];
    pack_hash.copy_from_slice(r.take(32)?);
    Ok((protocol, sim_version, pack_hash))
}

fn decode_hello(r: &mut Reader<'_>) -> Result<Hello, WireError> {
    let (protocol, sim_version, pack_hash) = decode_hello_head(r)?;
    let delay = r.u32()?;
    let window = r.u32()?;
    Ok(Hello {
        protocol,
        sim_version,
        pack_hash,
        delay,
        window,
    })
}

fn encode_hello_v1(h: &HelloV1, out: &mut Vec<u8>) -> Result<(), WireError> {
    if h.protocol == 0 || h.sim_version == 0 {
        return Err(WireError::ZeroVersion);
    }
    out.push(TAG_HELLO_V1);
    out.extend_from_slice(&h.protocol.to_le_bytes());
    out.extend_from_slice(&h.sim_version.to_le_bytes());
    out.extend_from_slice(&h.pack_hash);
    Ok(())
}

fn decode_hello_v1(r: &mut Reader<'_>) -> Result<HelloV1, WireError> {
    let (protocol, sim_version, pack_hash) = decode_hello_head(r)?;
    Ok(HelloV1 {
        protocol,
        sim_version,
        pack_hash,
    })
}

fn slot_byte(slot: PlayerIdx) -> u8 {
    match slot {
        PlayerIdx::First => 0,
        PlayerIdx::Second => 1,
        PlayerIdx::Third => 2,
    }
}

fn read_slot(r: &mut Reader<'_>) -> Result<PlayerIdx, WireError> {
    PlayerIdx::new(r.u8()?).ok_or(WireError::SlotOutOfRange)
}

const FLAG_ACTION: u8 = 0x80;
const STICK_REFUSED: i8 = i8::MIN;

fn encode_entry(w: &WireInput, out: &mut Vec<u8>) -> Result<(), WireError> {
    if w.move_x == STICK_REFUSED || w.move_y == STICK_REFUSED {
        return Err(WireError::StickOutOfRange);
    }
    let flags = u8::from(w.evade)
        | u8::from(w.main) << 1
        | u8::from(w.smash) << 2
        | u8::from(w.skill) << 3
        | u8::from(w.assist) << 4
        | u8::from(w.auto) << 5
        | u8::from(w.interact) << 6
        | if w.action.is_some() { FLAG_ACTION } else { 0 };
    out.push(flags);
    out.extend_from_slice(&w.move_x.to_le_bytes());
    out.extend_from_slice(&w.move_y.to_le_bytes());
    if let Some(action) = w.action {
        let (kind, operands): (u8, &[u32]) = match &action {
            EquipAction::Equip { item, slot } => (1, &[*item, *slot]),
            EquipAction::Replace { item, slot } => (2, &[*item, *slot]),
            EquipAction::Extract { item } => (3, &[*item]),
            EquipAction::Craft { star } => (4, &[*star]),
            EquipAction::TreePick { option } => (5, &[*option]),
        };
        out.push(kind);
        for n in operands {
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
    Ok(())
}

fn decode_entry(r: &mut Reader<'_>) -> Result<WireInput, WireError> {
    let flags = r.u8()?;
    let move_x = i8::from_le_bytes([r.u8()?]);
    let move_y = i8::from_le_bytes([r.u8()?]);
    if move_x == STICK_REFUSED || move_y == STICK_REFUSED {
        return Err(WireError::StickOutOfRange);
    }
    let action = if flags & FLAG_ACTION != 0 {
        let kind = r.u8()?;
        Some(match kind {
            1 => EquipAction::Equip {
                item: r.u32()?,
                slot: r.u32()?,
            },
            2 => EquipAction::Replace {
                item: r.u32()?,
                slot: r.u32()?,
            },
            3 => EquipAction::Extract { item: r.u32()? },
            4 => EquipAction::Craft { star: r.u32()? },
            5 => EquipAction::TreePick { option: r.u32()? },
            other => return Err(WireError::UnknownActionKind(other)),
        })
    } else {
        None
    };
    Ok(WireInput {
        move_x,
        move_y,
        evade: flags & 1 != 0,
        main: flags & 2 != 0,
        smash: flags & 4 != 0,
        skill: flags & 8 != 0,
        assist: flags & 16 != 0,
        auto: flags & 32 != 0,
        interact: flags & 64 != 0,
        action,
    })
}

fn encode_input(f: &InputFrame, out: &mut Vec<u8>) -> Result<(), WireError> {
    let count = u8::try_from(f.inputs.len())
        .ok()
        .filter(|n| (1..=MAX_FRAME_INPUTS).contains(&usize::from(*n)))
        .ok_or(WireError::CountOutOfRange)?;
    f.start_tick
        .checked_add(u32::from(count) - 1)
        .ok_or(WireError::TickOverflow)?;
    out.push(TAG_INPUT);
    out.push(slot_byte(f.slot));
    out.extend_from_slice(&f.start_tick.to_le_bytes());
    out.push(count);
    for w in &f.inputs {
        encode_entry(w, out)?;
    }
    Ok(())
}

fn decode_input(r: &mut Reader<'_>) -> Result<InputFrame, WireError> {
    let slot = read_slot(r)?;
    let start_tick = r.u32()?;
    let count = r.u8()?;
    if !(1..=MAX_FRAME_INPUTS).contains(&usize::from(count)) {
        return Err(WireError::CountOutOfRange);
    }
    start_tick
        .checked_add(u32::from(count) - 1)
        .ok_or(WireError::TickOverflow)?;
    let mut inputs = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        inputs.push(decode_entry(r)?);
    }
    Ok(InputFrame {
        slot,
        start_tick,
        inputs,
    })
}

fn encode_ack(a: &Ack, out: &mut Vec<u8>) {
    out.push(TAG_ACK);
    out.push(slot_byte(a.slot));
    out.extend_from_slice(&a.next_tick.to_le_bytes());
}

fn checkpoint_tick_ok(tick: u32) -> bool {
    tick != 0 && tick % CHECKPOINT_TICKS == 0
}

fn encode_checkpoint(c: &Checkpoint, out: &mut Vec<u8>) -> Result<(), WireError> {
    if !checkpoint_tick_ok(c.tick) {
        return Err(WireError::CheckpointTick);
    }
    out.push(TAG_CHECKPOINT);
    out.push(slot_byte(c.slot));
    out.extend_from_slice(&c.tick.to_le_bytes());
    out.extend_from_slice(&c.hash.to_le_bytes());
    Ok(())
}

fn decode_checkpoint(r: &mut Reader<'_>) -> Result<Checkpoint, WireError> {
    let slot = read_slot(r)?;
    let tick = r.u32()?;
    if !checkpoint_tick_ok(tick) {
        return Err(WireError::CheckpointTick);
    }
    let b = r.take(8)?;
    let hash = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
    Ok(Checkpoint { slot, tick, hash })
}

fn encode_bye(b: &Bye, out: &mut Vec<u8>) {
    out.push(TAG_BYE);
    out.push(slot_byte(b.slot));
    out.extend_from_slice(&b.tick.to_le_bytes());
    out.push(match b.reason {
        ByeReason::Left => 1,
        ByeReason::Desync => 2,
        ByeReason::Refused => 3,
    });
}

fn decode_bye(r: &mut Reader<'_>) -> Result<Bye, WireError> {
    let slot = read_slot(r)?;
    let tick = r.u32()?;
    let reason = match r.u8()? {
        1 => ByeReason::Left,
        2 => ByeReason::Desync,
        3 => ByeReason::Refused,
        other => return Err(WireError::UnknownReason(other)),
    };
    Ok(Bye { slot, tick, reason })
}

impl Packet {
    /// The one byte string of this packet, or the rule it breaks.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut out = Vec::new();
        match self {
            Packet::Hello(h) => encode_hello(h, &mut out)?,
            Packet::HelloV1(h) => encode_hello_v1(h, &mut out)?,
            Packet::Input(f) => encode_input(f, &mut out)?,
            Packet::Ack(a) => encode_ack(a, &mut out),
            Packet::Checkpoint(c) => encode_checkpoint(c, &mut out)?,
            Packet::Bye(b) => encode_bye(b, &mut out),
        }
        if out.len() > MAX_PACKET_LEN {
            return Err(WireError::TooLong);
        }
        Ok(out)
    }

    /// The packet `bytes` hold, or the first rule they break.
    pub fn decode(bytes: &[u8]) -> Result<Packet, WireError> {
        if bytes.len() > MAX_PACKET_LEN {
            return Err(WireError::TooLong);
        }
        let (&tag, body) = bytes.split_first().ok_or(WireError::Empty)?;
        let mut r = Reader { rest: body };
        let packet = match tag {
            TAG_HELLO => Packet::Hello(decode_hello(&mut r)?),
            TAG_HELLO_V1 => Packet::HelloV1(decode_hello_v1(&mut r)?),
            TAG_INPUT => Packet::Input(decode_input(&mut r)?),
            TAG_ACK => Packet::Ack(Ack {
                slot: read_slot(&mut r)?,
                next_tick: r.u32()?,
            }),
            TAG_CHECKPOINT => Packet::Checkpoint(decode_checkpoint(&mut r)?),
            TAG_BYE => Packet::Bye(decode_bye(&mut r)?),
            other => return Err(WireError::UnknownTag(other)),
        };
        r.finish()?;
        Ok(packet)
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
