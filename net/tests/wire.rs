//! The wire codec through the public API only.

use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_net::wire::{
    Ack, Bye, ByeReason, Checkpoint, Hello, HelloV1, InputFrame, MAX_FRAME_INPUTS, MAX_PACKET_LEN,
    PROTOCOL_VERSION, Packet, WireError, WireInput,
};

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

const HELLO: &str = "06020000000200000000010203040506070809\
                     0a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\
                     0200000004000000";
const HELLO_16_0: &str = concat!(
    "060200000003000000",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "1000000000000000"
);
const HELLO_V1: &str = "010100000002000000000102030405060708090a0b0c0d0e0f\
                        101112131415161718191a1b1c1d1e1f";

fn hello_v1_packet() -> Packet {
    let mut pack_hash = [0u8; 32];
    for (i, b) in pack_hash.iter_mut().enumerate() {
        *b = u8::try_from(i).unwrap();
    }
    Packet::HelloV1(HelloV1 {
        protocol: 1,
        sim_version: 2,
        pack_hash,
    })
}

fn hello_packet() -> Packet {
    let mut pack_hash = [0u8; 32];
    for (i, b) in pack_hash.iter_mut().enumerate() {
        *b = u8::try_from(i).unwrap();
    }
    Packet::Hello(Hello {
        protocol: 2,
        sim_version: 2,
        pack_hash,
        delay: 2,
        window: 4,
    })
}

const INPUT_IDLE: &str = "02000000000001000000";
const INPUT_THREE: &str = "02010201000003037f81c000000107000000020000008000000501000000";

const ACK: &str = "030203010000";
const CHECKPOINT: &str = "0400100e0000efcdab8967452301";
const BYE: &str = "0501201c000001";

fn ack_packet() -> Packet {
    Packet::Ack(Ack {
        slot: slot(2),
        next_tick: 259,
    })
}

fn checkpoint_packet() -> Packet {
    Packet::Checkpoint(Checkpoint {
        slot: slot(0),
        tick: 3600,
        hash: 0x0123_4567_89ab_cdef,
    })
}

fn bye_packet() -> Packet {
    Packet::Bye(Bye {
        slot: slot(1),
        tick: 7200,
        reason: ByeReason::Left,
    })
}

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn frame(slot_: u8, start_tick: u32, inputs: Vec<WireInput>) -> Packet {
    Packet::Input(InputFrame {
        slot: slot(slot_),
        start_tick,
        inputs,
    })
}

fn three_entries() -> Packet {
    frame(
        1,
        258,
        vec![
            WireInput {
                move_x: 127,
                move_y: -127,
                evade: true,
                main: true,
                ..WireInput::default()
            },
            WireInput {
                interact: true,
                action: Some(EquipAction::Equip { item: 7, slot: 2 }),
                ..WireInput::default()
            },
            WireInput {
                action: Some(EquipAction::TreePick { option: 1 }),
                ..WireInput::default()
            },
        ],
    )
}

/// Every vector the file knows; later steps add theirs.
fn vectors() -> Vec<Vec<u8>> {
    vec![
        hex(HELLO),
        hex(HELLO_16_0),
        hex(HELLO_V1),
        hex(INPUT_IDLE),
        hex(INPUT_THREE),
        hex(ACK),
        hex(CHECKPOINT),
        hex(BYE),
    ]
}

#[test]
fn the_constants_are_what_the_protocol_says() {
    assert_eq!(PROTOCOL_VERSION, 2);
    assert_eq!(MAX_PACKET_LEN, 1200);
    assert_eq!(MAX_FRAME_INPUTS, 32);
}

#[test]
fn framing_refuses_the_empty_the_unknown_and_the_too_long() {
    assert_eq!(Packet::decode(&[]), Err(WireError::Empty));
    for tag in [0x00u8, 0x07, 0xff] {
        assert_eq!(Packet::decode(&[tag]), Err(WireError::UnknownTag(tag)));
    }
    let mut long = vec![0u8; MAX_PACKET_LEN + 1];
    long[0] = 0x03;
    assert_eq!(Packet::decode(&long), Err(WireError::TooLong));
}

#[test]
fn trailing_and_missing_bytes_are_refused() {
    let mut more = hex(HELLO);
    more.push(0);
    assert_eq!(Packet::decode(&more), Err(WireError::TrailingBytes));
    let mut less = hex(HELLO);
    less.pop();
    assert_eq!(Packet::decode(&less), Err(WireError::Truncated));
}

#[test]
fn every_strict_prefix_of_every_vector_is_an_error() {
    for v in vectors() {
        for n in 0..v.len() {
            assert!(Packet::decode(&v[..n]).is_err(), "prefix {n} of {v:02x?}");
        }
    }
}

#[test]
fn a_hello_is_49_bytes_and_round_trips() {
    let p = hello_packet();
    let bytes = p.encode().unwrap();
    assert_eq!(bytes.len(), 49);
    assert_eq!(bytes, hex(HELLO));
    assert_eq!(Packet::decode(&bytes), Ok(p));
}

#[test]
fn the_second_hello_vector_round_trips() {
    let p = Packet::Hello(Hello {
        protocol: 2,
        sim_version: 3,
        pack_hash: [0xaa; 32],
        delay: 16,
        window: 0,
    });
    let bytes = p.encode().unwrap();
    assert_eq!(bytes, hex(HELLO_16_0));
    assert_eq!(Packet::decode(&bytes), Ok(p));
}

#[test]
fn delay_and_window_are_not_range_checked_on_the_wire() {
    for (delay, window) in [(0, 0), (u32::MAX, u32::MAX), (0, u32::MAX), (u32::MAX, 0)] {
        let p = Packet::Hello(Hello {
            protocol: 2,
            sim_version: 2,
            pack_hash: [1; 32],
            delay,
            window,
        });
        assert_eq!(Packet::decode(&p.encode().unwrap()), Ok(p));
    }
}

#[test]
fn delay_and_window_are_in_that_order() {
    let mut bytes = hex(HELLO);
    let n = bytes.len();
    let (a, b) = (bytes[n - 8..n - 4].to_vec(), bytes[n - 4..].to_vec());
    bytes[n - 8..n - 4].copy_from_slice(&b);
    bytes[n - 4..].copy_from_slice(&a);
    let Ok(Packet::Hello(h)) = Packet::decode(&bytes) else {
        panic!("a hello")
    };
    assert_eq!((h.delay, h.window), (4, 2));
    assert_ne!(Packet::Hello(h), hello_packet());
}

#[test]
fn a_protocol_1_hello_is_41_bytes_and_decodes_as_hello_v1() {
    let p = hello_v1_packet();
    let bytes = p.encode().unwrap();
    assert_eq!(bytes.len(), 41);
    assert_eq!(bytes, hex(HELLO_V1));
    assert_eq!(Packet::decode(&bytes), Ok(p));
    let mut more = bytes.clone();
    more.push(0);
    assert_eq!(Packet::decode(&more), Err(WireError::TrailingBytes));
    assert_eq!(Packet::decode(&bytes[..40]), Err(WireError::Truncated));
    for (protocol, sim_version) in [(0, 2), (1, 0)] {
        let p = Packet::HelloV1(HelloV1 {
            protocol,
            sim_version,
            pack_hash: [0; 32],
        });
        assert_eq!(p.encode(), Err(WireError::ZeroVersion));
        let mut bytes = hex(HELLO_V1);
        bytes[1..5].copy_from_slice(&protocol.to_le_bytes());
        bytes[5..9].copy_from_slice(&sim_version.to_le_bytes());
        assert_eq!(Packet::decode(&bytes), Err(WireError::ZeroVersion));
    }
}

#[test]
fn the_two_hello_layouts_are_never_read_as_each_other() {
    let mut short_new = hex(HELLO_V1);
    short_new[0] = 0x06;
    assert_eq!(Packet::decode(&short_new), Err(WireError::Truncated));
    let mut long_old = hex(HELLO);
    long_old[0] = 0x01;
    assert_eq!(Packet::decode(&long_old), Err(WireError::TrailingBytes));
}

#[test]
fn a_zero_version_is_refused_both_ways() {
    for (protocol, sim_version) in [(0, 2), (1, 0)] {
        let p = Packet::Hello(Hello {
            protocol,
            sim_version,
            pack_hash: [0; 32],
            delay: 2,
            window: 4,
        });
        assert_eq!(p.encode(), Err(WireError::ZeroVersion));
        let mut bytes = hex(HELLO);
        bytes[1..5].copy_from_slice(&protocol.to_le_bytes());
        bytes[5..9].copy_from_slice(&sim_version.to_le_bytes());
        assert_eq!(Packet::decode(&bytes), Err(WireError::ZeroVersion));
    }
}

// ---- the input frame ----

fn entry_len(w: WireInput) -> usize {
    Packet::decode(&frame(0, 0, vec![w]).encode().unwrap()).unwrap();
    frame(0, 0, vec![w]).encode().unwrap().len() - 7
}

fn flags_input(bits: u8) -> WireInput {
    WireInput {
        evade: bits & 1 != 0,
        main: bits & 2 != 0,
        smash: bits & 4 != 0,
        skill: bits & 8 != 0,
        assist: bits & 16 != 0,
        auto: bits & 32 != 0,
        interact: bits & 64 != 0,
        ..WireInput::default()
    }
}

#[test]
fn an_idle_entry_is_ten_bytes_in_all() {
    let p = frame(0, 0, vec![WireInput::default()]);
    assert_eq!(p.encode().unwrap(), hex(INPUT_IDLE));
    assert_eq!(Packet::decode(&hex(INPUT_IDLE)), Ok(p));
}

#[test]
fn an_entry_without_an_action_is_three_bytes() {
    for bits in 0..128u8 {
        for axis in [0i8, 1, -1, 127, -127] {
            let w = WireInput {
                move_x: axis,
                move_y: -axis,
                ..flags_input(bits)
            };
            assert_eq!(entry_len(w), 3, "bits {bits} axis {axis}");
        }
    }
}

#[test]
fn an_entry_with_an_action_is_eight_or_twelve_bytes() {
    let with = |action| WireInput {
        action: Some(action),
        ..WireInput::default()
    };
    assert_eq!(entry_len(with(EquipAction::Extract { item: 1 })), 8);
    assert_eq!(entry_len(with(EquipAction::Craft { star: 1 })), 8);
    assert_eq!(entry_len(with(EquipAction::TreePick { option: 1 })), 8);
    assert_eq!(entry_len(with(EquipAction::Equip { item: 1, slot: 2 })), 12);
    assert_eq!(
        entry_len(with(EquipAction::Replace { item: 1, slot: 2 })),
        12
    );
}

#[test]
fn every_action_round_trips_with_any_operand() {
    for action in [
        EquipAction::Equip {
            item: u32::MAX,
            slot: 0,
        },
        EquipAction::Replace {
            item: 0,
            slot: u32::MAX,
        },
        EquipAction::Extract { item: u32::MAX },
        EquipAction::Craft { star: 9 },
        EquipAction::TreePick { option: u32::MAX },
    ] {
        let p = frame(
            2,
            5,
            vec![WireInput {
                action: Some(action),
                ..WireInput::default()
            }],
        );
        assert_eq!(Packet::decode(&p.encode().unwrap()), Ok(p));
    }
}

#[test]
fn the_three_entry_vector_round_trips() {
    let bytes = hex(INPUT_THREE);
    assert_eq!(bytes.len(), 30);
    assert_eq!(three_entries().encode().unwrap(), bytes);
    assert_eq!(Packet::decode(&bytes), Ok(three_entries()));
}

#[test]
fn the_count_is_one_to_thirty_two() {
    for n in [0usize, MAX_FRAME_INPUTS + 1] {
        let p = frame(0, 0, vec![WireInput::default(); n]);
        assert_eq!(p.encode(), Err(WireError::CountOutOfRange), "encode {n}");
        let mut bytes = hex(INPUT_IDLE);
        bytes[6] = u8::try_from(n).unwrap();
        bytes.resize(7 + 3 * n, 0);
        assert_eq!(
            Packet::decode(&bytes),
            Err(WireError::CountOutOfRange),
            "decode {n}"
        );
    }
}

#[test]
fn a_slot_byte_of_three_is_refused() {
    let mut bytes = hex(INPUT_IDLE);
    bytes[1] = 3;
    assert_eq!(Packet::decode(&bytes), Err(WireError::SlotOutOfRange));
}

#[test]
fn the_ticks_must_fit_in_u32() {
    let two = frame(0, u32::MAX, vec![WireInput::default(); 2]);
    assert_eq!(two.encode(), Err(WireError::TickOverflow));
    let mut bytes = hex(INPUT_IDLE);
    bytes[2..6].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[6] = 2;
    bytes.extend_from_slice(&[0, 0, 0]);
    assert_eq!(Packet::decode(&bytes), Err(WireError::TickOverflow));
    let one = frame(0, u32::MAX, vec![WireInput::default()]);
    assert_eq!(Packet::decode(&one.encode().unwrap()), Ok(one));
}

#[test]
fn a_stick_axis_of_minus_128_is_refused() {
    for (x, y) in [(i8::MIN, 0), (0, i8::MIN)] {
        let p = frame(
            0,
            0,
            vec![WireInput {
                move_x: x,
                move_y: y,
                ..WireInput::default()
            }],
        );
        assert_eq!(p.encode(), Err(WireError::StickOutOfRange));
        let mut bytes = hex(INPUT_IDLE);
        bytes[8] = x.to_le_bytes()[0];
        bytes[9] = y.to_le_bytes()[0];
        assert_eq!(Packet::decode(&bytes), Err(WireError::StickOutOfRange));
    }
}

#[test]
fn an_action_kind_of_zero_or_six_is_refused() {
    for kind in [0u8, 6] {
        let mut bytes = hex(INPUT_IDLE);
        bytes[7] |= 0x80;
        bytes.extend_from_slice(&[kind, 0, 0, 0, 0]);
        assert_eq!(
            Packet::decode(&bytes),
            Err(WireError::UnknownActionKind(kind))
        );
    }
}

#[test]
fn the_largest_frame_is_391_bytes() {
    let w = WireInput {
        action: Some(EquipAction::Equip { item: 1, slot: 1 }),
        ..WireInput::default()
    };
    let p = frame(0, 10, vec![w; MAX_FRAME_INPUTS]);
    let bytes = p.encode().unwrap();
    assert_eq!(bytes.len(), 391);
    assert_eq!(Packet::decode(&bytes), Ok(p));
}

#[test]
fn a_byte_after_the_last_entry_is_refused() {
    let mut bytes = hex(INPUT_THREE);
    bytes.push(0);
    assert_eq!(Packet::decode(&bytes), Err(WireError::TrailingBytes));
}

#[test]
fn from_input_folds_minus_128_and_to_input_gives_it_back() {
    let w = WireInput::from_input(&Input {
        move_x: i8::MIN,
        move_y: i8::MIN,
        ..Input::default()
    });
    assert_eq!((w.move_x, w.move_y), (-127, -127));
    let i = Input {
        move_x: 100,
        move_y: -90,
        main: true,
        smash: true,
        evade: true,
        skill: true,
        assist: true,
        auto: true,
        interact: true,
        action: Some(EquipAction::Craft { star: 3 }),
        ..Input::default()
    };
    assert_eq!(WireInput::from_input(&i).to_input(), i);
    assert_eq!(WireInput::default().to_input(), Input::default());
}

// ---- no offer choice ever travels ----

#[test]
fn a_wire_input_has_exactly_the_named_fields() {
    let w = WireInput::default();
    // A new field, an offer choice included, fails to compile here.
    let WireInput {
        move_x: _,
        move_y: _,
        main: _,
        smash: _,
        evade: _,
        skill: _,
        assist: _,
        auto: _,
        interact: _,
        action: _,
    } = w;
}

#[test]
fn the_wire_source_never_names_the_offer_choice() {
    let src = include_str!("../src/wire.rs");
    for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
        for token in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            assert!(token != "pick" && token != "picking", "{line}");
        }
    }
}

#[test]
fn every_flag_set_is_still_three_bytes() {
    let w = WireInput {
        main: true,
        smash: true,
        evade: true,
        skill: true,
        assist: true,
        auto: true,
        interact: true,
        ..WireInput::default()
    };
    assert_eq!(entry_len(w), 3);
}

// ---- ack, checkpoint, bye ----

#[test]
fn ack_checkpoint_and_bye_round_trip_their_vectors() {
    for (p, v) in [
        (ack_packet(), ACK),
        (checkpoint_packet(), CHECKPOINT),
        (bye_packet(), BYE),
    ] {
        assert_eq!(p.encode().unwrap(), hex(v));
        assert_eq!(Packet::decode(&hex(v)), Ok(p));
    }
}

#[test]
fn a_checkpoint_tick_is_a_non_zero_multiple_of_60() {
    for (tick, ok) in [
        (0, false),
        (59, false),
        (61, false),
        (60, true),
        (3600, true),
    ] {
        let p = Packet::Checkpoint(Checkpoint {
            slot: slot(0),
            tick,
            hash: 1,
        });
        let mut bytes = hex(CHECKPOINT);
        bytes[2..6].copy_from_slice(&tick.to_le_bytes());
        if ok {
            assert_eq!(Packet::decode(&p.encode().unwrap()), Ok(p));
            assert!(Packet::decode(&bytes).is_ok());
        } else {
            assert_eq!(p.encode(), Err(WireError::CheckpointTick), "{tick}");
            assert_eq!(
                Packet::decode(&bytes),
                Err(WireError::CheckpointTick),
                "{tick}"
            );
        }
    }
}

#[test]
fn a_bye_reason_is_one_to_three() {
    for reason in 0..=4u8 {
        let mut bytes = hex(BYE);
        bytes[6] = reason;
        match reason {
            1..=3 => assert!(Packet::decode(&bytes).is_ok()),
            _ => assert_eq!(
                Packet::decode(&bytes),
                Err(WireError::UnknownReason(reason))
            ),
        }
    }
    for (reason, byte) in [
        (ByeReason::Left, 1u8),
        (ByeReason::Desync, 2),
        (ByeReason::Refused, 3),
    ] {
        let p = Packet::Bye(Bye {
            slot: slot(0),
            tick: 0,
            reason,
        });
        assert_eq!(p.encode().unwrap()[6], byte);
    }
}

#[test]
fn a_slot_byte_of_three_is_refused_in_every_slot_packet() {
    for v in [ACK, CHECKPOINT, BYE, INPUT_IDLE] {
        let mut bytes = hex(v);
        bytes[1] = 3;
        assert_eq!(
            Packet::decode(&bytes),
            Err(WireError::SlotOutOfRange),
            "{v}"
        );
    }
}

// ---- sweeps ----

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn byte(&mut self) -> u8 {
        u8::try_from(self.below(256)).unwrap()
    }

    fn word(&mut self) -> u32 {
        u32::try_from(self.next() >> 32).unwrap()
    }
}

fn random_action(rng: &mut Xorshift) -> Option<EquipAction> {
    let (a, b) = (rng.word(), rng.word());
    match rng.below(6) {
        0 => Some(EquipAction::Equip { item: a, slot: b }),
        1 => Some(EquipAction::Replace { item: a, slot: b }),
        2 => Some(EquipAction::Extract { item: a }),
        3 => Some(EquipAction::Craft { star: a }),
        4 => Some(EquipAction::TreePick { option: a }),
        _ => None,
    }
}

fn random_input(rng: &mut Xorshift) -> WireInput {
    let axis =
        |rng: &mut Xorshift| i8::try_from(i64::try_from(rng.below(255)).unwrap() - 127).unwrap();
    let bits = rng.byte();
    WireInput {
        move_x: axis(rng),
        move_y: axis(rng),
        action: random_action(rng),
        ..flags_input(bits & 0x7f)
    }
}

fn random_packet(rng: &mut Xorshift) -> Packet {
    let s = slot(u8::try_from(rng.below(3)).unwrap());
    match rng.below(6) {
        5 => {
            let mut pack_hash = [0u8; 32];
            pack_hash.iter_mut().for_each(|b| *b = rng.byte());
            Packet::HelloV1(HelloV1 {
                protocol: rng.word().max(1),
                sim_version: rng.word().max(1),
                pack_hash,
            })
        }
        0 => {
            let mut pack_hash = [0u8; 32];
            pack_hash.iter_mut().for_each(|b| *b = rng.byte());
            Packet::Hello(Hello {
                protocol: rng.word().max(1),
                sim_version: rng.word().max(1),
                pack_hash,
                delay: rng.word(),
                window: rng.word(),
            })
        }
        1 => {
            let count = usize::try_from(rng.below(32) + 1).unwrap();
            let start_tick = rng.word() / 2;
            let inputs = (0..count).map(|_| random_input(rng)).collect();
            Packet::Input(InputFrame {
                slot: s,
                start_tick,
                inputs,
            })
        }
        2 => Packet::Ack(Ack {
            slot: s,
            next_tick: rng.word(),
        }),
        3 => Packet::Checkpoint(Checkpoint {
            slot: s,
            tick: (rng.word() / 60 % 1_000_000 + 1) * 60,
            hash: rng.next(),
        }),
        _ => Packet::Bye(Bye {
            slot: s,
            tick: rng.word(),
            reason: [ByeReason::Left, ByeReason::Desync, ByeReason::Refused]
                [usize::try_from(rng.below(3)).unwrap()],
        }),
    }
}

#[test]
fn two_thousand_random_packets_round_trip() {
    let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15);
    for _ in 0..2000 {
        let p = random_packet(&mut rng);
        let bytes = p.encode().unwrap();
        assert!(bytes.len() <= MAX_PACKET_LEN);
        assert_eq!(Packet::decode(&bytes), Ok(p));
    }
}

#[test]
fn every_single_byte_change_is_refused_or_re_encodes_to_itself() {
    for v in vectors() {
        for i in 0..v.len() {
            for b in 0..=255u8 {
                if b == v[i] {
                    continue;
                }
                let mut bytes = v.clone();
                bytes[i] = b;
                if let Ok(p) = Packet::decode(&bytes) {
                    assert_eq!(
                        p.encode().unwrap(),
                        bytes,
                        "byte {i} = {b:#04x} of {v:02x?}"
                    );
                }
            }
        }
    }
}
