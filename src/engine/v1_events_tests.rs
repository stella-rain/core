//! The adapter from the frozen events of `sim_version` 1 to the live `DomainEvent`
//! (ADR-REPL-2 points 1 to 4) is exact: every field is carried, nothing is
//! reordered, and the four player fields are `None`, so a version 1 run's events serialise to the
//! bytes they always did. The scratch the engine hands the frozen rules never allocates.

use super::v1_events::adapt;
use super::*;
use crate::attack::{DamageType, MatchUp};
use crate::bench_fixtures;
use crate::event::{DomainEvent, HitKind, HitSource};
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId, PlayerIdx};
use crate::input::Input;
use crate::kinds::KindIndex;
use crate::rewards::Rarity;
use crate::rune::Rune;
use crate::sim_v1;
use crate::sim_v1::event as v1;
use crate::sim_v1::views::{ExpertiseOfferKind, OfferKind};
use crate::snapshot::{EntityKind, Press};
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
            at: Point {
                x: Fx(10),
                y: Fx(11),
            },
            radius: 11,
            depth: 12,
        },
        E::PlayerHit {
            hp_left: 13,
            source: v1::HitSource {
                kind: v1::HitKind::Zone,
                owner: EntityId(14),
                attack: Some("preset".into()),
            },
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
        E::HoldReleased { ticks: 28 },
        E::HoldBroken,
        E::RuleFired {
            agent: EntityId(29),
            rule: 30,
            target: Some(EntityId(31)),
        },
        E::PhaseChanged { phase: 32 },
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
        E::LeveledUp { level: 40 },
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
        E::ElitesCalled { elites: 49 },
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
            at: Point {
                x: Fx(58),
                y: Fx(59),
            },
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

/// The frozen events with no live counterpart (ADR-SIM-7).
const DROPPED_EVENTS: [&str; 8] = [
    "StatOfferOpened",
    "ExpertiseOfferOpened",
    "ExpertiseOptionTaken",
    "ExpertiseCandidateReplaced",
    "OfferOpened",
    "OfferSkipped",
    "RewardBanished",
    "CandidateReplaced",
];

#[test]
fn the_adapted_events_keep_their_type_names() {
    let events = every_v1_event();
    assert_eq!(events.len(), 43, "one value per variant");
    let mut names = Vec::new();
    for event in events {
        let frozen = frozen_name(&event);
        match adapt(event) {
            Some(live) => {
                let json = serde_json::to_value(&live).unwrap();
                assert_eq!(json["type"], frozen.as_str());
            }
            None => assert!(DROPPED_EVENTS.contains(&frozen.as_str()), "{frozen}"),
        }
        names.push(frozen);
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 43, "the names are distinct");
}

#[test]
fn the_adapter_carries_every_field_in_order() {
    let (mut kept, mut dropped) = (0, 0);
    for event in every_v1_event() {
        let frozen = format!("{event:?}");
        match adapt(event) {
            Some(live) => {
                assert_eq!(without_player_fields(&live), frozen);
                kept += 1;
            }
            None => dropped += 1,
        }
    }
    assert_eq!((kept, dropped), (35, 8));
}

/// The adapter has no live event for the eight offer events, and only for those.
#[test]
fn the_adapter_drops_exactly_the_eight_offer_events() {
    for event in every_v1_event() {
        let name = frozen_name(&event);
        assert_eq!(
            adapt(event).is_none(),
            DROPPED_EVENTS.contains(&name.as_str()),
            "{name}"
        );
    }
}

#[test]
fn the_player_fields_are_none_and_the_bytes_are_those_of_before() {
    // The texts are the live type's serialisation of the same events, taken before the move.
    let literal = |name: &str| match name {
        "Hit" => {
            r#"{"type":"Hit","data":{"source":1,"target":2,"target_kind":3,"damage":4,"damage_type":"pyro","match_up":"strong","rune":"explosion","crit":true,"echo":true}}"#
        }
        "Healed" => r#"{"type":"Healed","data":{"target":15,"amount":16}}"#,
        "StatusApplied" => {
            r#"{"type":"StatusApplied","data":{"target":17,"status":"id18","stacks":19,"duration_ticks":20}}"#
        }
        "Died" => r#"{"type":"Died","data":{"entity":36,"kind":37,"class":"boss","elite":true}}"#,
        other => panic!("no literal for {other}"),
    };
    let mut seen = 0;
    for event in every_v1_event() {
        let name = frozen_name(&event);
        if !["Hit", "Healed", "StatusApplied", "Died"].contains(&name.as_str()) {
            continue;
        }
        let live: DomainEvent = adapt(event).expect("a live event");
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
    e.step(&[input, Default::default(), Default::default()]);
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
    let adapted: Vec<DomainEvent> = frozen.into_iter().filter_map(adapt).collect();
    assert_eq!(e.events(), adapted.as_slice());
    assert!(e.events().iter().all(names_no_player));
    adapted.len()
}

#[test]
fn a_version_1_engine_streams_the_frozen_events_in_order() {
    let bench_fixtures::Case {
        engine: mut e,
        input,
    } = Engine::forced(1, bench_fixtures::offers_cases::worst_tick);
    let mut by_hand = e.clone();
    let mut total = 0;
    for _ in 0..60 {
        total += step_both(&mut e, &mut by_hand, input);
    }
    assert!(total > 0, "the case emits events");
}

#[test]
fn a_version_1_choose_streams_the_frozen_events_in_order() {
    use bench_fixtures::worst_minute_case::{Run as Case, input_at};
    let mut e = Engine::forced(1, || Case::fast_forwarded(0)).engine;
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
            e.v1_pick_step(&[Some(0), None, None]).unwrap();
            let adapted: Vec<DomainEvent> = frozen.into_iter().filter_map(adapt).collect();
            assert_eq!(e.events(), adapted.as_slice(), "pick, tick {tick}");
            total += adapted.len();
            picks += 1;
        }
        let input = input_at(e.tick());
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
    let bench_fixtures::Case { engine, .. } =
        Engine::forced(1, bench_fixtures::offers_cases::worst_tick);
    for e in [&engine, &engine.clone()] {
        let (capacity, room) = v1_room(e);
        assert!(capacity >= room, "{capacity} < {room}");
    }
}

#[test]
fn the_scratch_never_allocates_after_load() {
    let bench_fixtures::Case {
        engine: mut e,
        input,
    } = Engine::forced(1, bench_fixtures::offers_cases::worst_tick);
    let (scratch, _) = v1_room(&e);
    let events = e.events.capacity();
    for tick in 1..=120 {
        e.step(&[input, Default::default(), Default::default()]);
        let Run::V1 { events: left, .. } = &e.run else {
            panic!("version 1");
        };
        assert!(left.is_empty(), "left over after tick {tick}");
        assert_eq!(left.capacity(), scratch, "tick {tick}");
        assert_eq!(e.events.capacity(), events, "tick {tick}");
    }
}

/// What a version 1 run shows of its offers is pinned: every corpus run in `tests/corpus/v1/runs`
/// is replayed with its recorded inputs and picks, and the JSON of the snapshot at each open
/// offer and of the events after each pick is folded into one hash. The offer views the frozen
/// rules build are moved without changing this (ADR-REPL-2 point 6, ADR-SIM-3).
#[test]
fn a_version_1_replay_shows_the_same_offers() {
    let root = super::savepoint_tests::data_root::data_root().join("tests/corpus/v1");
    let mut dirs: Vec<_> = std::fs::read_dir(root.join("runs"))
        .unwrap()
        .map(|d| d.unwrap().path())
        .collect();
    dirs.sort();
    let (mut kept, mut offers) = (String::new(), 0u32);
    for dir in dirs {
        let replay = crate::run_replay::RunReplay::from_bytes(
            &std::fs::read(dir.join("run.replay")).unwrap(),
        )
        .unwrap();
        let pack = std::fs::read_dir(root.join("packs"))
            .unwrap()
            .map(|p| {
                crate::pack::Pack::from_bytes(&std::fs::read(p.unwrap().path()).unwrap()).unwrap()
            })
            .find(|p| crate::content_hash::to_hex(&p.sha256()) == replay.content_sha256)
            .expect("the run's pack is kept");
        let (inputs, commands) = replay
            .step_log(crate::run_replay::MAX_COMMANDS * 100_000)
            .unwrap()
            .to_solo()
            .unwrap();
        let mut e = Engine::start_with(&replay.setup, pack.content()).unwrap();
        let mut next = 0;
        let mut pick_here = |e: &mut Engine, tick: usize, kept: &mut String, offers: &mut u32| {
            while next < commands.len() && commands[next].tick as usize == tick {
                if e.offer_open() {
                    // The same snapshot without `progress.offer`.
                    let mut snap = serde_json::to_value(e.snapshot()).unwrap();
                    if let Some(p) = snap.get_mut("progress").and_then(|p| p.as_object_mut()) {
                        p.remove("offer");
                    }
                    kept.push_str(&snap.to_string());
                    *offers += 1;
                }
                e.v1_pick_step(&[Some(commands[next].index), None, None])
                    .unwrap();
                let events = serde_json::to_value(e.events()).unwrap();
                kept.push_str(&events.to_string());
                next += 1;
            }
        };
        pick_here(&mut e, 0, &mut kept, &mut offers);
        for (tick, input) in inputs.iter().enumerate() {
            e.step(&[*input, Default::default(), Default::default()]);
            pick_here(&mut e, tick + 1, &mut kept, &mut offers);
        }
        assert_eq!(next, commands.len(), "every pick was made");
    }
    assert!(offers > 0, "the corpus opens offers");
    // Seen in the corpus: star offers and constellations; no stat offer and no expertise offer
    // (the hand-built values of `the_offer_views_convert_field_by_field` cover those).
    assert!(kept.contains("\"constellations\":[{"), "a constellation");
    // The fold without `progress.offer` and the four offer events, which leave the live snapshot
    // and events: recorded before they left, a pin in source, not generated. The fold with them
    // (the pin before this one) can no longer be produced.
    assert!(kept.contains("\"RewardTaken\""), "a taken reward");
    assert_eq!(crate::hash::fnv1a64(kept.as_bytes()), 5042547584517530830);
}

#[test]
fn the_offer_views_convert_field_by_field() {
    use crate::sim_v1::views as w;
    use crate::snapshot as live;

    let kinds: [(w::OfferKind, live::OfferKind); 2] = [
        (w::OfferKind::Star, live::OfferKind::Star),
        (w::OfferKind::Boss, live::OfferKind::Boss),
    ];
    for (frozen, expected) in kinds {
        assert_eq!(live::OfferKind::from(frozen), expected);
    }
    let options: [(w::OfferOption, live::OfferOption); 4] = [
        (
            w::OfferOption::Take { candidate: 1 },
            live::OfferOption::Take { candidate: 1 },
        ),
        (
            w::OfferOption::Reroll { candidate: 2 },
            live::OfferOption::Reroll { candidate: 2 },
        ),
        (w::OfferOption::Skip, live::OfferOption::Skip),
        (
            w::OfferOption::Banish { candidate: 0 },
            live::OfferOption::Banish { candidate: 0 },
        ),
    ];
    for (frozen, expected) in options {
        assert_eq!(live::OfferOption::from(frozen), expected);
        assert_eq!(w::OfferOption::from(expected), frozen);
    }

    let offer = w::OfferView {
        kind: w::OfferKind::Boss,
        rarity: Rarity::Epic,
        candidates: vec![
            w::CandidateView {
                reward: ContentId::from("r1"),
                rarity: Rarity::Rare,
                hidden: true,
                named: false,
                star: None,
                constellation: None,
            },
            w::CandidateView {
                reward: ContentId::from("r2"),
                rarity: Rarity::Legendary,
                hidden: false,
                named: true,
                star: Some("s".into()),
                constellation: Some(w::ConstellationProgress {
                    set: ContentId::from("set"),
                    held: 2,
                    needed: 3,
                }),
            },
        ],
        options: vec![
            w::OfferOption::Take { candidate: 0 },
            w::OfferOption::Reroll { candidate: 1 },
            w::OfferOption::Skip,
        ],
        rerolls_left: 4,
        banishes_left: 5,
    };
    let converted = live::OfferView::from(offer);
    assert_eq!(converted.kind, live::OfferKind::Boss);
    assert_eq!(converted.rarity, Rarity::Epic);
    assert_eq!(converted.rerolls_left, 4);
    assert_eq!(converted.banishes_left, 5);
    assert_eq!(
        converted.options,
        [
            live::OfferOption::Take { candidate: 0 },
            live::OfferOption::Reroll { candidate: 1 },
            live::OfferOption::Skip,
        ]
    );
    assert_eq!(
        converted.candidates,
        [
            live::CandidateView {
                reward: ContentId::from("r1"),
                rarity: Rarity::Rare,
                hidden: true,
                named: false,
                star: None,
                constellation: None,
            },
            live::CandidateView {
                reward: ContentId::from("r2"),
                rarity: Rarity::Legendary,
                hidden: false,
                named: true,
                star: Some("s".into()),
                constellation: Some(live::ConstellationProgress {
                    set: ContentId::from("set"),
                    held: 2,
                    needed: 3,
                }),
            },
        ]
    );
    assert_eq!(
        serde_json::to_string(&converted).unwrap(),
        concat!(
            r#"{"kind":"boss","rarity":"epic","candidates":["#,
            r#"{"reward":"r1","rarity":"rare","hidden":true},"#,
            r#"{"reward":"r2","rarity":"legendary","named":true,"star":"s","#,
            r#""constellation":{"set":"set","held":2,"needed":3}}],"#,
            r#""options":[{"type":"take","candidate":0},{"type":"reroll","candidate":1},"#,
            r#"{"type":"skip"}],"rerolls_left":4,"banishes_left":5}"#
        )
    );

    let view = live::ConstellationView::from(w::ConstellationView {
        id: ContentId::from("c"),
        name: "Crown".into(),
        held: 1,
        tier: 2,
        needed: 3,
    });
    assert_eq!(
        view,
        live::ConstellationView {
            id: ContentId::from("c"),
            name: "Crown".into(),
            held: 1,
            tier: 2,
            needed: 3,
        }
    );
}

/// A frozen run still stops on a stat or expertise offer and takes its pick, but its snapshot
/// shows no such offer and its event stream names none of the eight events that left the live
/// rules (ADR-REPL-2 point 6, ADR-SIM-7).
#[test]
fn a_v1_run_stops_on_a_level_offer_without_showing_it() {
    const GONE: [&str; 8] = DROPPED_EVENTS;
    let pack = crate::pack::Pack::from_bytes(include_bytes!(
        "../../tests/corpus/v1/packs/32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5.json"
    ))
    .expect("the version 1 pack");
    let content = pack.content();
    let bytes: &[u8] = include_bytes!("../../tests/corpus/v1/runs/trial/run.replay");
    let recorded = crate::run_replay::RunReplay::from_bytes(bytes).expect("a run replay");
    // The stage comes from the version 1 pack's own mode files, not the bundled ones.
    let stage = run_setup::stage_in(&recorded.setup, content).expect("a stage");
    let inputs = recorded
        .unpack_inputs(run_setup::max_replay_ticks(
            &recorded.setup,
            &stage,
            content.time_limit(),
        ))
        .expect("inputs");
    let commands = recorded.unpack_commands().expect("commands");
    let mut e = Engine::start_with(&recorded.setup, content).expect("the run starts");
    let mut next = commands.iter().peekable();
    let mut level_offers = 0;
    for input in inputs {
        while let Some(c) = next.next_if(|c| c.tick == e.tick()) {
            assert!(e.offer_open(), "a recorded pick answers an open offer");
            let star = !e.v1_offer_candidates().is_empty();
            level_offers += usize::from(!star);
            e.v1_pick_step(&[Some(c.index), None, None])
                .expect("a recorded choice");
        }
        e.step(&[input, Default::default(), Default::default()]);
        let snap = serde_json::to_value(e.snapshot()).unwrap();
        assert!(snap["progress"].get("stat_offer").is_none());
        assert!(
            snap["progress"].get("offer").is_none(),
            "no star offer shown"
        );
        assert!(snap["expertise"].get("offer").is_none());
        for event in e.events() {
            let name = serde_json::to_value(event).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned();
            assert!(!GONE.contains(&name.as_str()), "{name}");
        }
    }
    assert!(level_offers > 0, "the run took a stat or expertise offer");
}
