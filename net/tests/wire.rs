//! The wire codec through the public API only.

use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_net::wire::{
    Hello, InputFrame, MAX_FRAME_INPUTS, MAX_PACKET_LEN, PROTOCOL_VERSION, Packet, WireError,
    WireInput,
};

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

const HELLO: &str = "01010000000200000000010203040506070809\
                     0a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn hello_packet() -> Packet {
    let mut pack_hash = [0u8; 32];
    for (i, b) in pack_hash.iter_mut().enumerate() {
        *b = u8::try_from(i).unwrap();
    }
    Packet::Hello(Hello {
        protocol: 1,
        sim_version: 2,
        pack_hash,
    })
}

const INPUT_IDLE: &str = "02000000000001000000";
const INPUT_THREE: &str = "02010201000003037f81c000000107000000020000008000000501000000";

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
    vec![hex(HELLO), hex(INPUT_IDLE), hex(INPUT_THREE)]
}

#[test]
fn the_constants_are_what_the_protocol_says() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(MAX_PACKET_LEN, 1200);
    assert_eq!(MAX_FRAME_INPUTS, 32);
}

#[test]
fn framing_refuses_the_empty_the_unknown_and_the_too_long() {
    assert_eq!(Packet::decode(&[]), Err(WireError::Empty));
    for tag in [0x00u8, 0x06, 0xff] {
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
fn a_hello_is_41_bytes_and_round_trips() {
    let p = hello_packet();
    let bytes = p.encode().unwrap();
    assert_eq!(bytes.len(), 41);
    assert_eq!(bytes, hex(HELLO));
    assert_eq!(Packet::decode(&bytes), Ok(p));
}

#[test]
fn a_zero_version_is_refused_both_ways() {
    for (protocol, sim_version) in [(0, 2), (1, 0)] {
        let p = Packet::Hello(Hello {
            protocol,
            sim_version,
            pack_hash: [0; 32],
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
