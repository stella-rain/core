//! Tests for the equipment slots (ADR-SIM-3 point 2, ADR-SIM-7): the layout at tick 0, the
//! build from the equipped set, the grants and the equip and replace rules.

use super::equipment::{Held, SlotKind};
use super::state::{PlayerIdx, State};
use crate::engine::{Engine, Run};
use crate::run_setup::{BuildMode, RunSetup};
use crate::run_stats::StatEffect;
use crate::rune::Rune;

fn setup(rune: Option<&str>, lumine: Option<&str>) -> RunSetup {
    RunSetup {
        build_mode: BuildMode::Equipment,
        starting_rune: rune.map(|r| r.into()),
        lumine: u8::from(lumine.is_some()),
        lumine_star: lumine.map(|l| l.into()),
        ..RunSetup::default()
    }
}

fn state_of(e: &Engine) -> &State {
    match e.run() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// The layout the slots take until the pack carries it: Attack 5, Defence 2, Mobility 2, Rune 3.
fn layout() -> Vec<SlotKind> {
    let mut v = Vec::new();
    for (kind, n) in [
        (SlotKind::Category(0), 5),
        (SlotKind::Category(1), 2),
        (SlotKind::Category(2), 2),
        (SlotKind::Rune, 3),
    ] {
        v.extend(std::iter::repeat_n(kind, n));
    }
    v
}

/// T1: an equipment run starts with the layout, all empty; an offers run has none.
#[test]
fn an_equipment_run_starts_with_the_layout() {
    let e = Engine::start(&setup(None, None)).expect("starts");
    let eq = &state_of(&e).players.career(PlayerIdx::SOLO).equipment;
    let kinds: Vec<SlotKind> = eq.slots.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, layout());
    assert!(
        eq.slots
            .iter()
            .all(|s| s.held.is_none() && s.level == 0 && s.expertise == 0)
    );
    assert!(eq.lumine.is_none());

    let e = Engine::start(&setup(None, Some("max_hp_common"))).expect("starts");
    let eq = &state_of(&e).players.career(PlayerIdx::SOLO).equipment;
    assert_eq!(eq.slots.len(), 12);
    assert!(eq.lumine.is_some());

    let e = Engine::start(&RunSetup::default()).expect("starts");
    let eq = &state_of(&e).players.career(PlayerIdx::SOLO).equipment;
    assert!(eq.slots.is_empty() && eq.lumine.is_none());
}

/// T2: the starting rune fills rune slot 0 and the runes the Main fires, not the inventory.
#[test]
fn the_starting_rune_fills_rune_slot_0() {
    let plain = Engine::start(&setup(None, None)).expect("starts");
    let e = Engine::start(&setup(Some("thunder"), None)).expect("starts");
    let (s, p) = (state_of(&e), state_of(&plain));
    let c = s.players.career(PlayerIdx::SOLO);
    assert_eq!(c.equipment.slots[9].held, Some(Held::Rune(Rune::Thunder)));
    assert!(c.equipment.slots[10].held.is_none() && c.equipment.slots[11].held.is_none());
    assert!(c.equipment.inventory.is_empty());
    let runes: Vec<Rune> = s.players[PlayerIdx::SOLO]
        .runes
        .iter()
        .map(|h| h.rune)
        .collect();
    assert_eq!(runes, [Rune::Thunder]);
    let q = p.players.career(PlayerIdx::SOLO);
    assert!(q.equipment.slots.iter().all(|s| s.held.is_none()));
    assert!(p.players[PlayerIdx::SOLO].runes.is_empty());
    assert_eq!(c.offer.state(), q.offer.state(), "no draw moved");
}

/// T3: the Lumine star is in the build at tick 0 (its +20,000 max hp, hp full).
#[test]
fn the_lumine_star_is_in_the_build_at_tick_0() {
    let plain = Engine::start(&setup(None, None)).expect("starts");
    let e = Engine::start(&setup(None, Some("max_hp_common"))).expect("starts");
    let (s, p) = (state_of(&e), state_of(&plain));
    let (me, was) = (&s.players[PlayerIdx::SOLO], &p.players[PlayerIdx::SOLO]);
    assert_eq!(me.max_hp, was.max_hp + 20_000);
    assert_eq!(me.hp, me.max_hp);
    let total = |s: &State| {
        s.players
            .career(PlayerIdx::SOLO)
            .progress
            .as_ref()
            .expect("levels")
            .totals[StatEffect::MaxHp as usize]
    };
    assert_eq!(total(s), total(p) + 20_000);
}
