//! Tests for core `RunLedger` (ADR-BRDG-6, DS49).
//!
//! The ledger counts kills, bestiary bits, picks and skill uses inside the simulation
//! from domain events and state observations, matching the rules previously kept in
//! the app wrapper's `Tally`.

use crate::common;
use std::fs;

use stella_rain_core::attack::{DamageType, MatchUp};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{ContentId, EntityId};
use stella_rain_core::kinds::{KindIndex, KindTable};
use stella_rain_core::ledger::{RunLedger, Seen, StarPick};
use stella_rain_core::rewards::Rarity;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::rune::Rune;
use stella_rain_core::snapshot::EntityKind;

fn kind_table() -> KindTable {
    let ids = ["bat", "chaser", "golem", "knight"].map(ContentId::from);
    KindTable::from_ids(ids.iter(), 16).unwrap()
}

fn seen(id: u32, kind: &str) -> Seen {
    Seen {
        id: EntityId(id),
        kind: ContentId::from(kind),
    }
}

fn hit(target: u32, kind: u16, ty: Option<DamageType>) -> DomainEvent {
    DomainEvent::Hit {
        source: EntityId(0),
        target: EntityId(target),
        target_kind: Some(KindIndex(kind)),
        damage: 10,
        damage_type: ty,
        match_up: MatchUp::None,
        rune: None,
        crit: false,
        echo: false,
    }
}

fn died(entity: u32, kind: u16) -> DomainEvent {
    DomainEvent::Died {
        entity: EntityId(entity),
        kind: Some(KindIndex(kind)),
        class: EntityKind::Enemy,
        elite: false,
    }
}

#[test]
fn two_chasers_killed_with_blade_damage_set_bits_and_count_slain() {
    let mut ledger = RunLedger::default();
    let chasers = [seen(5, "chaser"), seen(6, "chaser")];
    let kinds = kind_table();
    ledger.observe(&kinds, "giant_sword", &chasers, &[]);
    let slash = Some(DamageType::Slash);
    let events = [hit(5, 1, slash), hit(6, 1, slash), died(5, 1), died(6, 1)];
    ledger.observe(&kinds, "giant_sword", &[], &events);

    // Bit 0 encountered and bit 1, Slash, the first type of core's order (0b11 = 3).
    assert_eq!(ledger.bits("chaser"), Some(0b11));
    assert_eq!(ledger.stat("slain.chaser"), 2);
    assert_eq!(ledger.stat("kills.giant_sword.enemy"), 2);
}

#[test]
fn a_rune_hit_sets_its_type_bit() {
    let mut ledger = RunLedger::default();
    let by_rune = DomainEvent::Hit {
        source: EntityId(0),
        target: EntityId(5),
        target_kind: Some(KindIndex(1)),
        damage: 10,
        damage_type: Some(DamageType::Pyro),
        match_up: MatchUp::Weak,
        rune: Some(Rune::Explosion),
        crit: false,
        echo: false,
    };
    ledger.observe(
        &kind_table(),
        "giant_sword",
        &[seen(5, "chaser")],
        &[by_rune],
    );
    let pyro = DamageType::ALL
        .iter()
        .position(|t| *t == DamageType::Pyro)
        .unwrap()
        + 1;
    assert_eq!(ledger.bits("chaser"), Some(1 | (1 << pyro)));
}

#[test]
fn a_kill_counts_by_its_event_kind() {
    let mut ledger = RunLedger::default();
    let died_event = DomainEvent::Died {
        entity: EntityId(7),
        kind: Some(KindIndex(2)),
        class: EntityKind::Enemy,
        elite: false,
    };
    ledger.observe(&kind_table(), "giant_sword", &[], &[died_event]);
    assert_eq!(ledger.stat("slain.golem"), 1);
    assert_eq!(ledger.stat("kills.giant_sword.enemy"), 1);
    // Entity dying is met/seen.
    assert_eq!(ledger.bits("golem"), Some(1));
}

#[test]
fn each_damage_type_has_its_own_bit_after_cores_order() {
    let mut ledger = RunLedger::default();
    ledger.observe(
        &kind_table(),
        "giant_sword",
        &[seen(5, "golem")],
        &[hit(5, 2, Some(DamageType::Dendro))],
    );
    let bits = ledger.bits("golem").unwrap();
    // Dendro is 10th in DamageType::ALL.
    assert_eq!(bits, 1 | (1 << 10));
}

#[test]
fn a_hit_with_no_type_sets_no_type_bit() {
    let mut ledger = RunLedger::default();
    ledger.observe(
        &kind_table(),
        "giant_sword",
        &[seen(5, "bat")],
        &[hit(5, 0, None)],
    );
    assert_eq!(ledger.bits("bat"), Some(1));
}

#[test]
fn non_combat_entities_do_not_count_as_kills() {
    let mut ledger = RunLedger::default();
    let companion = DomainEvent::Died {
        entity: EntityId(10),
        kind: Some(KindIndex(0)),
        class: EntityKind::Companion,
        elite: false,
    };
    let shrine = DomainEvent::Died {
        entity: EntityId(11),
        kind: Some(KindIndex(1)),
        class: EntityKind::Shrine,
        elite: true,
    };
    let part = DomainEvent::Died {
        entity: EntityId(12),
        kind: Some(KindIndex(2)),
        class: EntityKind::BossPart,
        elite: false,
    };
    ledger.observe(
        &kind_table(),
        "giant_sword",
        &[],
        &[companion, shrine, part],
    );
    assert_eq!(ledger.stat("kills.giant_sword.companion"), 0);
    assert_eq!(ledger.stat("kills.giant_sword.shrine"), 0);
    assert_eq!(ledger.stat("slain.bat"), 0);
    assert_eq!(ledger.stat("slain.chaser"), 0);
}

#[test]
fn skills_stars_and_runes_are_counted() {
    let mut ledger = RunLedger::default();
    let events = [
        DomainEvent::StageSkillUsed {
            skill: "nova".into(),
            level: 1,
        },
        DomainEvent::StageSkillUsed {
            skill: "nova".into(),
            level: 1,
        },
        DomainEvent::StarTaken {
            star: EntityId(1),
            rarity: Rarity::Rare,
        },
        DomainEvent::RewardTaken {
            reward: "star_vega".into(),
            taken: 1,
            rune: None,
            star: Some("Vega".into()),
        },
        DomainEvent::StarTaken {
            star: EntityId(2),
            rarity: Rarity::Common,
        },
        DomainEvent::RewardTaken {
            reward: "random_rune".into(),
            taken: 1,
            rune: Some(Rune::Thunder),
            star: None,
        },
    ];
    ledger.observe(&kind_table(), "giant_sword", &[], &events);
    assert_eq!(ledger.stat("skill_uses.nova"), 2);
    assert_eq!(ledger.stat("stars_picked.rare"), 1);
    assert_eq!(ledger.stat("stars_picked.common"), 1);
    assert_eq!(ledger.stat("rune_taken.thunder"), 1);
    assert_eq!(
        ledger.picks,
        vec![
            StarPick {
                rarity: Rarity::Rare,
                name: Some("Vega".into()),
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
    let path = crate::corpus::runs_dir().join("trial/run.replay");
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
                    common::pick(&mut engine, c.index).unwrap();
                }
            }
        }
        engine.step(&[*input]);
    }

    let ledger = engine.ledger();
    let total_kills: u32 = ledger
        .kills
        .iter()
        .filter(|(k, _)| k.starts_with("kills."))
        .map(|(_, n)| *n)
        .sum();
    assert!(
        total_kills > 0,
        "trial corpus run should record enemy kills (got {total_kills})"
    );
    assert!(
        !ledger.bestiary.is_empty(),
        "trial corpus run should record bestiary encounters"
    );
}
