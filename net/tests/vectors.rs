//! The pinned byte vectors of protocol version 1. The literals were worked out by hand from the
//! layout, never produced by running the encoder; a failure here means the code or the layout
//! changed, and a layout change raises `PROTOCOL_VERSION`.

use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::EquipAction;
use stella_rain_net::wire::{
    Ack, Bye, ByeReason, Checkpoint, Hello, InputFrame, Packet, WireInput,
};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn both_ways(packet: Packet, literal: &str) {
    let bytes = hex(literal);
    assert_eq!(
        Packet::decode(&bytes),
        Ok(packet.clone()),
        "decode {literal}"
    );
    assert_eq!(packet.encode(), Ok(bytes), "encode {literal}");
}

#[test]
fn hello() {
    let mut pack_hash = [0u8; 32];
    for (i, b) in pack_hash.iter_mut().enumerate() {
        *b = u8::try_from(i).unwrap();
    }
    both_ways(
        Packet::Hello(Hello {
            protocol: 2,
            sim_version: 2,
            pack_hash,
            delay: 2,
            window: 4,
        }),
        "010100000002000000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
    );
}

#[test]
fn input_one_idle_entry() {
    both_ways(
        Packet::Input(InputFrame {
            slot: slot(0),
            start_tick: 0,
            inputs: vec![WireInput::default()],
        }),
        "02000000000001000000",
    );
}

#[test]
fn input_three_entries() {
    both_ways(
        Packet::Input(InputFrame {
            slot: slot(1),
            start_tick: 258,
            inputs: vec![
                WireInput {
                    evade: true,
                    main: true,
                    move_x: 127,
                    move_y: -127,
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
        }),
        "02010201000003037f81c000000107000000020000008000000501000000",
    );
}

#[test]
fn ack() {
    both_ways(
        Packet::Ack(Ack {
            slot: slot(2),
            next_tick: 259,
        }),
        "030203010000",
    );
}

#[test]
fn checkpoint() {
    both_ways(
        Packet::Checkpoint(Checkpoint {
            slot: slot(0),
            tick: 3600,
            hash: 0x0123_4567_89ab_cdef,
        }),
        "0400100e0000efcdab8967452301",
    );
}

#[test]
fn bye() {
    both_ways(
        Packet::Bye(Bye {
            slot: slot(1),
            tick: 7200,
            reason: ByeReason::Left,
        }),
        "0501201c000001",
    );
}
