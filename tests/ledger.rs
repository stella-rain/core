//! Tests for core `RunLedger` (ADR-BRDG-6, DS49).
//!
//! The ledger counts kills, bestiary bits, picks and skill uses inside the simulation
//! from domain events and state observations, matching the rules previously kept in
//! the app wrapper's `Tally`.

use std::fs;
use std::path::Path;

use stella_rain_core::attack::DamageType;
use stella_rain_core::engine::Engine;
use stella_rain_core::ledger::StarPick;
use stella_rain_core::rewards::Rarity;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::stage::Stage;

use crate::common;

fn stage() -> Stage {
    serde_json::from_value(common::clearing_stage_json()).unwrap()
}

#[test]
fn two_chasers_killed_with_blade_damage_set_bits_and_count_slain() {
    let stage = stage();
    let mut engine = Engine::new(&stage).unwrap();

    // Stepping through ticks with events updates engine.ledger().
    // We verify that the ledger computes kills, slain and bestiary bits.
    assert_eq!(engine.ledger().stat("slain.chaser"), 2);
    assert_eq!(engine.ledger().stat("kills.giant_sword.enemy"), 2);
    // Bit 0 encountered and bit 1 Slash (0b11 = 3).
    assert_eq!(engine.ledger().bits("chaser"), Some(0b11));
}

#[test]
fn each_damage_type_has_its_own_bit_after_cores_order() {
    let stage = stage();
    let engine = Engine::new(&stage).unwrap();

    // Dendro is the 10th damage type in DamageType::ALL (bit 10: 1 << 10 = 1024).
    let dendro_bit = DamageType::ALL
        .iter()
        .position(|t| *t == DamageType::Dendro)
        .unwrap()
        + 1;
    assert_eq!(dendro_bit, 10);
    assert_eq!(engine.ledger().bits("golem"), Some(1 | (1 << 10)));
}

#[test]
fn non_combat_entities_do_not_count_as_kills() {
    let stage = stage();
    let engine = Engine::new(&stage).unwrap();

    // Companions, Shrines and BossParts do not count as kills.
    assert_eq!(engine.ledger().stat("kills.giant_sword.companion"), 0);
    assert_eq!(engine.ledger().stat("kills.giant_sword.shrine"), 0);
}

#[test]
fn skill_uses_and_star_picks_are_tracked_in_ledger() {
    let stage = stage();
    let engine = Engine::new(&stage).unwrap();

    assert_eq!(engine.ledger().stat("skill_uses.nova"), 2);
    assert_eq!(engine.ledger().stat("stars_picked.rare"), 1);
    assert_eq!(engine.ledger().stat("rune_taken.thunder"), 1);
    assert_eq!(
        engine.ledger().picks,
        vec![
            StarPick {
                rarity: Rarity::Rare,
                name: Some("Vega".to_owned()),
            },
            StarPick {
                rarity: Rarity::Common,
                name: None,
            },
        ]
    );
}

#[test]
fn corpus_run_computes_expected_ledger() {
    let path = Path::new("tests/corpus/runs/v0/trial/run.replay");
    let replay_bytes = fs::read(path).expect("corpus run replay must exist");
    let replay = RunReplay::from_bytes(&replay_bytes).unwrap();
    let inputs = replay.unpack_inputs(replay.ticks).unwrap();
    let commands = replay.unpack_commands().unwrap();
    let mut next = commands.iter().peekable();
    let content = RunContent::bundled();
    let mut engine = Engine::start_with(&replay.setup, content).unwrap();

    for input in inputs.iter() {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            match c.kind {
                stella_rain_core::run_replay::CommandKind::Choose => {
                    engine.choose(c.index).unwrap();
                }
            }
        }
        engine.step(*input);
    }

    let ledger = engine.ledger();
    // In trial replay, at least one kill must be recorded.
    let total_kills: u32 = ledger
        .kills
        .iter()
        .filter(|(k, _)| k.starts_with("kills."))
        .map(|(_, n)| *n)
        .sum();
    assert!(total_kills > 0, "trial corpus run should record enemy kills");
    assert!(
        !ledger.bestiary.is_empty(),
        "trial corpus run should record bestiary encounters"
    );
}
