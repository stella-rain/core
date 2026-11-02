//! The adapter from the frozen events of `sim_version` 1 to the live `DomainEvent`
//! (ADR-REPL-9 point 1, ADR-REPL-7 points 3 and 4) is exact: every field is carried, nothing is
//! reordered, and the four player fields are `None`, so a version 1 run's events serialise to the
//! bytes they always did. The scratch the engine hands the frozen rules never allocates.

use super::*;
use crate::attack::{DamageType, MatchUp};
use crate::bench_fixtures;
use crate::input::Input;
use crate::event::{DomainEvent, HitKind, HitSource};
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId, PlayerIdx};
use crate::kinds::KindIndex;
use crate::rewards::Rarity;
use crate::rune::Rune;
use crate::run_setup::RunSetup;
use crate::sim_v1::event as v1;
use crate::snapshot::{EntityKind, ExpertiseOfferKind, OfferKind, Press};
use std::sync::Arc;

/// One frozen value of each of the 43 variants, with distinct, non-default field values.
fn every_v1_event() -> Vec<v1::DomainEvent> {
    use v1::DomainEvent as E;
    vec![
        E::Hit {
            source: EntityId(1),
            target: EntityId(2),
            target_kind: Some(KindIndex(3)),
            damage: 4,
            damage_type: Some(DamageType::Pyro),
            match_up: MatchUp::Strong,
            rune: Some(Rune::Explosion),
            crit: true,
            echo: true,
        },
        E::KillBurst {
            at: Point { x: Fx(10), y: Fx(11) },
            radius: 11,
            depth: 12,
        },
        E::PlayerHit {
            hp_left: 13,
            source: HitSource { kind: HitKind::Zone, owner: EntityId(14), attack: Some("preset".into()) },
        },
        E::Healed {
            target: EntityId(15),
            amount: 16,
        },
        E::StatusApplied {
            target: EntityId(17),
            status: ContentId::from("id18"),
            stacks: 19,
            duration_ticks: 20,
        },
        E::StunEnded {
            target: EntityId(21),
        },
        E::Evaded,
        E::StageSkillUsed {
            skill: ContentId::from("id22"),
            level: 23,
        },
        E::ComboStep {
            press: Press::Smash,
            depth: 25,
        },
        E::ComboReset,
        E::ChargeReleased {
            level: 26,
            perfect: true,
        },
        E::ChargeBroken,
        E::HoldReleased {
            ticks: 28,
        },
        E::HoldBroken,
        E::RuleFired {
            agent: EntityId(29),
            rule: 30,
            target: Some(EntityId(31)),
        },
        E::PhaseChanged {
            phase: 32,
        },
        E::Telegraph {
            source: EntityId(33),
            ticks: 34,
        },
        E::PartBroken {
            part: ContentId::from("id35"),
        },
        E::Died {
            entity: EntityId(36),
            kind: Some(KindIndex(37)),
            class: EntityKind::Boss,
            elite: true,
        },
        E::StageCleared,
        E::StageFailed,
        E::LeveledUp {
            level: 40,
        },
        E::StatRaised {
            stat: ContentId::from("id41"),
            spent: 42,
        },
        E::StatOfferOpened {
            options: vec![ContentId::from("a43"), ContentId::from("b43")],
        },
        E::StarDropped {
            star: EntityId(44),
            rarity: Rarity::Epic,
        },
        E::StarTaken {
            star: EntityId(46),
            rarity: Rarity::Epic,
        },
        E::HealDropped {
            pickup: EntityId(48),
        },
        E::ElitesCalled {
            elites: 49,
        },
        E::EliteSpawned {
            elite: EntityId(50),
            asset: ContentId::from("id51"),
            kind: Some(KindIndex(52)),
        },
        E::EnemyRelocated {
            entity: EntityId(53),
        },
        E::EliteLeft {
            elite: EntityId(54),
        },
        E::ShrineActivated {
            elite: EntityId(55),
        },
        E::MimicRevealed {
            elite: EntityId(56),
        },
        E::OrbBroken {
            owner: EntityId(57),
            at: Point { x: Fx(58), y: Fx(59) },
        },
        E::OfferOpened {
            kind: OfferKind::Boss,
            rarity: Rarity::Epic,
            candidates: vec![ContentId::from("a61"), ContentId::from("b61")],
        },
        E::RewardTaken {
            reward: ContentId::from("id62"),
            taken: 63,
            rune: Some(Rune::Explosion),
            star: Some("star65".into()),
        },
        E::Constellation {
            set: ContentId::from("id66"),
            tier: 67,
        },
        E::OfferSkipped,
        E::RewardBanished {
            reward: ContentId::from("id68"),
        },
        E::CandidateReplaced {
            slot: 69,
            reward: ContentId::from("id70"),
        },
        E::ExpertiseOfferOpened {
            level: 71,
            kind: ExpertiseOfferKind::Style,
            candidates: vec![ContentId::from("a73"), ContentId::from("b73")],
        },
        E::ExpertiseOptionTaken {
            option: ContentId::from("id74"),
            taken: 75,
        },
        E::ExpertiseCandidateReplaced {
            slot: 76,
            option: ContentId::from("id77"),
        },
    ]
}

/// The name of an event as a recording writes it: the `Debug` text up to the first space.
fn frozen_name(event: &v1::DomainEvent) -> String {
    let text = format!("{event:?}");
    text.split([' ', '(']).next().unwrap().to_string()
}

/// The `Debug` text of a live event without the player fields version 1 has no name for.
fn without_player_fields(event: &DomainEvent) -> String {
    let mut text = format!("{event:?}");
    for field in [
        ", player: None, buffed_by: None",
        ", source: None",
        ", killer: None",
    ] {
        text = text.replace(field, "");
    }
    text
}

#[test]
fn the_adapted_events_keep_their_type_names() {
    let events = every_v1_event();
    assert_eq!(events.len(), 43, "one value per variant");
    let mut names = Vec::new();
    for event in events {
        let frozen = frozen_name(&event);
        let live: DomainEvent = event.into();
        let json = serde_json::to_value(&live).unwrap();
        assert_eq!(json["type"], frozen.as_str());
        names.push(frozen);
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 43, "the names are distinct");
}

#[test]
fn the_adapter_carries_every_field_in_order() {
    for event in every_v1_event() {
        let frozen = format!("{event:?}");
        let live: DomainEvent = event.into();
        assert_eq!(without_player_fields(&live), frozen);
    }
}

#[test]
fn the_player_fields_are_none_and_the_bytes_are_those_of_before() {
    // The texts are the live type's serialisation of the same events, taken before the move.
    let literal = |name: &str| match name {
        "Hit" => r#"{"type":"Hit","data":{"source":1,"target":2,"target_kind":3,"damage":4,"damage_type":"pyro","match_up":"strong","rune":"explosion","crit":true,"echo":true}}"#,
        "Healed" => r#"{"type":"Healed","data":{"target":15,"amount":16}}"#,
        "StatusApplied" => r#"{"type":"StatusApplied","data":{"target":17,"status":"id18","stacks":19,"duration_ticks":20}}"#,
        "Died" => r#"{"type":"Died","data":{"entity":36,"kind":37,"class":"boss","elite":true}}"#,
        other => panic!("no literal for {other}"),
    };
    let mut seen = 0;
    for event in every_v1_event() {
        let name = frozen_name(&event);
        if !["Hit", "Healed", "StatusApplied", "Died"].contains(&name.as_str()) {
            continue;
        }
        let live: DomainEvent = event.into();
        match &live {
            DomainEvent::Hit {
                player, buffed_by, ..
            } => assert_eq!((*player, *buffed_by), (None, None)),
            DomainEvent::Healed { source, .. } | DomainEvent::StatusApplied { source, .. } => {
                assert_eq!(*source, None);
            }
            DomainEvent::Died { killer, .. } => assert_eq!(*killer, None),
            other => panic!("not a player-field event: {other:?}"),
        }
        assert_eq!(serde_json::to_string(&live).unwrap(), literal(&name));
        seen += 1;
    }
    assert_eq!(seen, 4);
}

#[test]
fn hit_kinds_and_sources_round_trip() {
    for (frozen, live) in [
        (v1::HitKind::Attack, HitKind::Attack),
        (v1::HitKind::Zone, HitKind::Zone),
        (v1::HitKind::Link, HitKind::Link),
    ] {
        assert_eq!(HitKind::from(frozen), live);
        for attack in [None, Some(Arc::<str>::from("preset"))] {
            let source = v1::HitSource {
                kind: frozen,
                owner: EntityId(9),
                attack: attack.clone(),
            };
            assert_eq!(
                HitSource::from(source),
                HitSource {
                    kind: live,
                    owner: EntityId(9),
                    attack,
                }
            );
        }
    }
}

/// Whether `event` has none of the four player fields.
fn names_no_player(event: &DomainEvent) -> bool {
    match event {
        DomainEvent::Hit {
            player, buffed_by, ..
        } => player.is_none() && buffed_by.is_none(),
        DomainEvent::Healed { source, .. } | DomainEvent::StatusApplied { source, .. } => {
            source.is_none()
        }
        DomainEvent::Died { killer, .. } => killer.is_none(),
        _ => true,
    }
}

/// Steps `e` and, by hand, a clone of it through the frozen rules into a frozen `Vec` that
/// `Into` adapts: the engine's events must be those, in that order.
fn step_both(e: &mut Engine, by_hand: &mut Engine, input: Input) -> usize {
    e.step(&[input]);
    let mut frozen = Vec::new();
    let mut load = Load::default();
    let Run::V1 { world, state, .. } = &mut by_hand.run else {
        panic!("version 1");
    };
    sim_v1::step(
        1,
        world,
        by_hand.content.rewards(),
        by_hand.content.expertise(),
        state,
        input,
        &mut frozen,
        &mut load,
    );
    let adapted: Vec<DomainEvent> = frozen.into_iter().map(Into::into).collect();
    assert_eq!(e.events(), adapted.as_slice());
    assert!(e.events().iter().all(names_no_player));
    adapted.len()
}

#[test]
fn a_version_1_engine_streams_the_frozen_events_in_order() {
    let bench_fixtures::Case { engine: mut e, input } =
        Engine::forced(1, bench_fixtures::worst_tick);
    let mut by_hand = e.clone();
    let mut total = 0;
    for _ in 0..60 {
        total += step_both(&mut e, &mut by_hand, input);
    }
    assert!(total > 0, "the case emits events");
}

#[test]
fn a_version_1_choose_streams_the_frozen_events_in_order() {
    let setup = RunSetup {
        mode: "time_limit".into(),
        seed: 7,
        ..RunSetup::default()
    };
    let mut e = Engine::forced(1, || Engine::start(&setup).unwrap());
    let mut by_hand = e.clone();
    let (mut picks, mut total) = (0, 0);
    for tick in 1..=600u32 {
        while e.offer_open() {
            let options = by_hand.run.offer_options(
                by_hand.content.rewards(),
                by_hand.content.expertise(),
                PlayerIdx::SOLO,
            );
            let option = options[0];
            let mut frozen = Vec::new();
            let Run::V1 { world, state, .. } = &mut by_hand.run else {
                panic!("version 1");
            };
            sim_v1::choose(
                1,
                world,
                by_hand.content.rewards(),
                by_hand.content.expertise(),
                state,
                option,
                &mut frozen,
            );
            crate::engine::pick(&mut e, PlayerIdx::SOLO, 0).unwrap();
            let adapted: Vec<DomainEvent> = frozen.into_iter().map(Into::into).collect();
            assert_eq!(e.events(), adapted.as_slice(), "pick, tick {tick}");
            total += adapted.len();
            picks += 1;
        }
        let input = Input {
            main: true,
            move_x: if tick % 90 < 45 { 100 } else { -100 },
            ..Input::default()
        };
        total += step_both(&mut e, &mut by_hand, input);
    }
    assert!(picks > 0, "an offer opened and was chosen");
    assert!(total > 0);
}

/// The frozen scratch and the engine's events of version 1 run `e`.
fn v1_room(e: &Engine) -> (usize, usize) {
    let Run::V1 { events, .. } = &e.run else {
        panic!("version 1");
    };
    (events.capacity(), e.run.events_room())
}

#[test]
fn the_scratch_is_reserved_at_load_and_after_a_clone() {
    let bench_fixtures::Case { engine, .. } = Engine::forced(1, bench_fixtures::worst_tick);
    for e in [&engine, &engine.clone()] {
        let (capacity, room) = v1_room(e);
        assert!(capacity >= room, "{capacity} < {room}");
    }
}

#[test]
fn the_scratch_never_allocates_after_load() {
    let bench_fixtures::Case { engine: mut e, input } =
        Engine::forced(1, bench_fixtures::worst_tick);
    let (scratch, _) = v1_room(&e);
    let events = e.events.capacity();
    for tick in 1..=120 {
        e.step(&[input]);
        let Run::V1 { events: left, .. } = &e.run else {
            panic!("version 1");
        };
        assert!(left.is_empty(), "left over after tick {tick}");
        assert_eq!(left.capacity(), scratch, "tick {tick}");
        assert_eq!(e.events.capacity(), events, "tick {tick}");
    }
}
