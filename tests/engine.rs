//! The simulation engine (ADR-004, ADR-015) at version 0. Small stages are built here so that
//! every expected tick can be worked out by hand from the rules in `rules_v0`; the fixture
//! stage pins the hash every platform must reproduce (ADR-019).

use serde_json::{Value, json};
use stella_rain_core::attack::MatchUp;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::rng::SplitMix64;
use stella_rain_core::snapshot::{EntityKind, Snapshot};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::Error;

use crate::common;

use common::*;

const FIXTURE: &str = include_str!("fixtures/stage_v0.json");

// --- Starting a run -------------------------------------------------------------------------

#[test]
fn an_invalid_stage_does_not_start() {
    let mut stage = stage_with(600, Some(far_boss()), vec![]);
    stage.player.hp = 0;
    assert!(matches!(Engine::new(&stage), Err(Error::Invalid(_))));
}

#[test]
fn a_run_starts_at_tick_zero_with_the_character_near_the_bottom() {
    let e = running(vec![]);
    assert_eq!((e.tick(), e.outcome()), (0, Outcome::Running));
    assert_eq!(player_at(&e), (START_X, START_Y));
    let s = e.snapshot();
    assert_eq!(s.player.hp, 3);
    assert_eq!(s.entities.len(), 1, "just the boss");
    assert!(e.events().is_empty());
}

// --- Movement (ADR-050; the speed cap and the facing are in `tests/controls.rs`) -------------

#[test]
fn the_character_stays_inside_the_playfield() {
    let mut e = running(vec![]);
    // A full diagonal covers 541 units an axis per tick: the corner is under 310 ticks away.
    for _ in 0..400 {
        e.step(moving(-127, -127));
    }
    assert_eq!(player_at(&e), (0, 0));
    for _ in 0..400 {
        e.step(moving(127, 127));
    }
    assert_eq!(player_at(&e), (92_160, 163_840));
}

#[test]
fn extreme_input_values_do_not_overflow() {
    let mut e = running(vec![]);
    for (move_x, move_y) in [(i8::MIN, i8::MIN), (i8::MAX, i8::MAX), (i8::MIN, i8::MAX)] {
        e.step(Input {
            move_x,
            move_y,
            evade: true,
            ..Input::default()
        });
    }
    assert_eq!(e.tick(), 3);
}

#[test]
fn the_snapshot_gives_each_circle_the_radius_hits_use() {
    let boss = json!({ "base": "golem", "hp": 1000, "radius": 8192,
                       "spawn": { "x": 46_080, "y": 25_600 },
                       "parts": [{ "id": "horn", "asset": "horns_2", "hp": 10,
                                   "offset": { "x": 4096, "y": 0 }, "radius": 3072 }],
                       "phases": [{}] });
    let bat = enemy(50, hold(20_000, 60_000), json!([]));
    let mut e = engine(&stage_full(
        18_000,
        Some(boss),
        vec![wave(1, 1, 0, bat, 20_000, 60_000)],
        vec![companion(10, hold(30_000, 140_000), json!([]))],
    ));
    e.step(idle());
    let s = e.snapshot();
    // The main character's hurt circle is 2 pixels.
    assert_eq!(s.player.radius.0, 512);
    let radii: Vec<(EntityKind, i32)> = s.entities.iter().map(|v| (v.kind, v.radius.0)).collect();
    assert_eq!(
        radii,
        [
            (EntityKind::Boss, 8192),
            (EntityKind::Companion, 1024),
            (EntityKind::BossPart, 3072),
            (EntityKind::Enemy, 2048),
        ]
    );
}

#[test]
fn the_main_shot_fired_every_six_ticks_leaves_the_field() {
    let mut e = running(vec![]);
    for _ in 0..60 {
        e.step(firing(idle(), e.tick()));
    }
    // Shots on ticks 1, 7, ..., 55; none has left the field yet.
    let s = e.snapshot();
    assert_eq!(s.bullets.len(), 10);
    assert!(s.bullets.iter().all(|b| b.friendly));
    // A shot is gone 80 ticks after it starts, so 14 are ever in the air at once.
    for _ in 0..200 {
        e.step(firing(idle(), e.tick()));
    }
    assert_eq!(e.snapshot().bullets.len(), 14);
}

// --- Collisions, damage and death ------------------------------------------------------------

#[test]
fn a_main_shot_hurts_and_then_kills_an_enemy() {
    let stage = stage_with(
        18_000,
        Some(far_boss()),
        vec![wave(
            1,
            1,
            0,
            enemy(20, hold(START_X, 25_600), json!([])),
            START_X,
            25_600,
        )],
    );
    let mut e = engine(&stage);
    let events = fire_until(&mut e, idle(), 200, |e| {
        e.outcome() != Outcome::Running || {
            e.snapshot()
                .entities
                .iter()
                .all(|x| x.kind != EntityKind::Enemy)
                && e.tick() > 1
        }
    });
    // A shot fired on tick 1 has moved 59 times when it touches the enemy: tick 59.
    let hits = ticks_of(&events, |ev| matches!(ev, DomainEvent::Hit { .. }));
    assert_eq!(hits, [59, 65]);
    let died = ticks_of(&events, |ev| matches!(ev, DomainEvent::Died { .. }));
    assert_eq!(died, [65]);
    assert!(matches!(
        events
            .iter()
            .find(|(_, ev)| matches!(ev, DomainEvent::Hit { .. })),
        Some((
            _,
            DomainEvent::Hit {
                source: EntityId(0),
                target: EntityId(2),
                damage: 10,
                match_up: MatchUp::None,
                rune: None,
                crit: false,
                echo: false,
            }
        ))
    ));
    assert_eq!(enemies(&e.snapshot()), 0);
}

#[test]
fn enemy_bullets_hurt_the_character_with_invulnerability_in_between() {
    let mut stage = stage_with(
        18_000,
        Some(far_boss()),
        vec![wave(
            1,
            1,
            0,
            enemy(50, hold(START_X, 25_600), json!([attack_rule(always(), 0)])),
            START_X,
            25_600,
        )],
    );
    // Three hits of the default 100 take the main character's 300 hit points.
    stage.player.hp = 300;
    // The weapons are quiet, so that the enemy lives.
    let mut e = engine(&quiet(stage));
    let events = run_until(&mut e, idle(), 400, |e| e.outcome() != Outcome::Running);
    // The first bullet is fired on tick 1 and has moved 157 times when it touches the
    // character; each hit grants 60 ticks without damage.
    let hits: Vec<_> = events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::PlayerHit { hp_left } => Some((*t, *hp_left)),
            _ => None,
        })
        .collect();
    assert_eq!(hits, [(157, 200), (217, 100), (277, 0)]);
    assert_eq!((e.tick(), e.outcome()), (277, Outcome::Failed));
    assert_eq!(
        events.last().map(|(_, ev)| ev),
        Some(&DomainEvent::StageFailed)
    );
}

#[test]
fn a_finished_run_ignores_further_ticks() {
    let mut e = engine(&stage_with(5, Some(far_boss()), vec![]));
    let events = run_until(&mut e, idle(), 10, |e| e.outcome() != Outcome::Running);
    assert_eq!((e.tick(), e.outcome()), (5, Outcome::Failed));
    assert_eq!(ticks_of(&events, |ev| *ev == DomainEvent::StageFailed), [5]);
    let hash = e.state_hash();
    for _ in 0..50 {
        e.step(idle());
        assert!(e.events().is_empty());
    }
    assert_eq!((e.tick(), e.state_hash()), (5, hash));
}

// --- Clear and fail --------------------------------------------------------------------------

#[test]
fn beating_the_boss_clears_the_stage() {
    let mut e = engine(&stage_with(18_000, Some(boss_at(20, 25_600)), vec![]));
    let events = fire_until(&mut e, idle(), 300, |e| e.outcome() != Outcome::Running);
    // The first shot touches the boss on tick 56, the second (fired on tick 7) on tick 62. The
    // step that fired on tick 61 has run out, and with no window the combo is back at the root.
    assert_eq!((e.tick(), e.outcome()), (62, Outcome::Cleared));
    let last: Vec<_> = events
        .iter()
        .filter(|(t, _)| *t == 62)
        .map(|(_, ev)| ev)
        .collect();
    assert_eq!(
        last,
        [
            &DomainEvent::ComboReset,
            &DomainEvent::Hit {
                source: EntityId(0),
                target: EntityId(1),
                damage: 10,
                match_up: MatchUp::None,
                rune: None,
                crit: false,
                echo: false,
            },
            &DomainEvent::Died {
                entity: EntityId(1)
            },
            &DomainEvent::StageCleared,
        ]
    );
    assert!(e.snapshot().entities.is_empty());
}

#[test]
fn running_out_of_ticks_fails_the_stage() {
    let mut e = engine(&stage_with(100, Some(far_boss()), vec![]));
    run_until(&mut e, idle(), 500, |e| e.outcome() != Outcome::Running);
    assert_eq!((e.tick(), e.outcome()), (100, Outcome::Failed));
}

#[test]
fn without_a_boss_the_stage_clears_when_every_enemy_is_gone() {
    // Three enemies run straight down and leave the field; the last spawns on tick 21 and is
    // out of the field after 89 moves, on tick 109.
    let runner = enemy(
        5,
        json!({ "type": "straight", "vx": 0, "vy": 2048 }),
        json!([]),
    );
    // The weapons are quiet, so that the enemies leave rather than die.
    let mut e = engine(&quiet(stage_with(
        18_000,
        None,
        vec![wave(1, 3, 10, runner, START_X, 0)],
    )));
    run_until(&mut e, idle(), 500, |e| e.outcome() != Outcome::Running);
    assert_eq!((e.tick(), e.outcome()), (109, Outcome::Cleared));
}

#[test]
fn a_stage_with_nothing_to_beat_clears_at_once() {
    let mut e = engine(&stage_with(600, None, vec![]));
    e.step(idle());
    assert_eq!((e.tick(), e.outcome()), (1, Outcome::Cleared));
}

// --- Waves and the runtime caps (ADR-020) ----------------------------------------------------

#[test]
fn waves_spawn_on_schedule_in_entity_id_order() {
    let still = || enemy(5, hold(START_X, 25_600), json!([]));
    let mut e = running(vec![
        wave(3, 2, 4, still(), START_X, 1000),
        wave(5, 1, 0, still(), START_X, 2000),
    ]);
    let mut counts = Vec::new();
    for _ in 0..8 {
        e.step(idle());
        counts.push(enemies(&e.snapshot()));
    }
    // Wave 0 spawns on ticks 3 and 7, wave 1 on tick 5.
    assert_eq!(counts, [0, 0, 1, 1, 2, 2, 3, 3]);
    let ids: Vec<u32> = e.snapshot().entities.iter().map(|x| x.id.0).collect();
    assert_eq!(ids, [1, 2, 3, 4], "boss first, then enemies in spawn order");
}

#[test]
fn at_most_five_hundred_enemies_are_alive_and_dropped_spawns_still_count() {
    let waves = (0..200)
        .map(|_| {
            wave(
                1,
                100,
                1,
                enemy(5, hold(START_X, 25_600), json!([])),
                START_X,
                0,
            )
        })
        .collect();
    let mut e = running(waves);
    let mut alive = Vec::new();
    for _ in 0..5 {
        e.step(idle());
        alive.push(enemies(&e.snapshot()));
    }
    assert_eq!(alive, [200, 400, 500, 500, 500]);
    // Dropped spawns take no entity ID: the boss is 1 and the enemies are 2 to 501.
    let ids: Vec<u32> = e.snapshot().entities.iter().map(|x| x.id.0).collect();
    assert_eq!(ids, (1..=501).collect::<Vec<u32>>());
}

#[test]
fn bullet_caps_are_two_hundred_new_and_four_thousand_alive() {
    let shooter = || enemy(5, hold(START_X, 0), json!([attack_rule(always(), 0)]));
    let waves = (0..200)
        .map(|_| wave(1, 100, 1, shooter(), START_X, 0))
        .collect();
    // Quiet weapons: the enemies spread from their point as they push apart, into the path of
    // the main character's own bullets, which would count here and be used up there.
    let mut e = engine(&quiet(stage_with(18_000, Some(far_boss()), waves)));
    let mut before = 0;
    for tick in 1..=60 {
        e.step(idle());
        let bullets = e.snapshot().bullets.len();
        assert!(
            bullets - before <= 200,
            "tick {tick}: {} new bullets",
            bullets - before
        );
        assert_eq!(bullets, (200 * tick).min(4000), "tick {tick}");
        before = bullets;
    }
}

// --- Rules (ADR-036) -------------------------------------------------------------------------

/// One enemy standing far from the character, with `rules`; every `RuleFired` as (tick, rule).
/// A running stage whose weapons are quiet, so that the enemies the tests set up live.
fn quiet_running(waves: Vec<Value>) -> Engine {
    engine(&quiet(stage_with(18_000, Some(far_boss()), waves)))
}

fn rule_firings(rules: Value, input: Input, ticks: u32) -> Vec<(u32, u8)> {
    let mut e = quiet_running(vec![wave(
        1,
        1,
        0,
        enemy(5, hold(START_X, 25_600), rules),
        START_X,
        25_600,
    )]);
    let events = run_until(&mut e, input, ticks, |_| false);
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::RuleFired { rule, .. } => Some((*t, *rule)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_cooldown_spaces_firings_out() {
    let fired = rule_firings(json!([attack_rule(always(), 30)]), idle(), 100);
    assert_eq!(fired, [(1, 0), (31, 0), (61, 0), (91, 0)]);
}

#[test]
fn once_and_charges_limit_firings() {
    let mut once = attack_rule(always(), 0);
    once["once"] = json!(true);
    assert_eq!(rule_firings(json!([once]), idle(), 50), [(1, 0)]);
    let mut two = attack_rule(always(), 0);
    two["charges"] = json!(2);
    assert_eq!(rule_firings(json!([two]), idle(), 50), [(1, 0), (2, 0)]);
}

#[test]
fn time_above_waits_for_the_agent_to_be_older() {
    let fired = rule_firings(
        json!([attack_rule(
            json!({ "type": "time_above", "ticks": 50 }),
            1000
        )]),
        idle(),
        100,
    );
    assert_eq!(fired, [(51, 0)]);
}

#[test]
fn only_the_first_ready_rule_fires_and_the_others_wait() {
    let telegraph = json!({ "when": always(), "target": "player",
                            "do": { "type": "telegraph", "ticks": 10 } });
    let fired = rule_firings(json!([attack_rule(always(), 50), telegraph]), idle(), 51);
    let mut expected = vec![(1, 0)];
    expected.extend((2..=50).map(|t| (t, 1)));
    expected.push((51, 0));
    assert_eq!(fired, expected);
}

#[test]
fn an_attack_fires_one_bullet_aimed_at_the_character() {
    let mut e = quiet_running(vec![wave(
        1,
        1,
        0,
        enemy(
            5,
            hold(START_X, 25_600),
            json!([attack_rule(always(), 1000)]),
        ),
        START_X,
        25_600,
    )]);
    e.step(idle());
    let s = e.snapshot();
    assert_eq!(s.bullets.len(), 1);
    assert!(!s.bullets[0].friendly);
    // Straight down at 3 pixels a tick, after one move.
    assert_eq!(
        (s.bullets[0].at.x.0, s.bullets[0].at.y.0),
        (START_X, 25_600 + 768)
    );
}

// --- Events and the snapshot -----------------------------------------------------------------

#[test]
fn events_are_those_of_the_last_tick_only() {
    let mut e = running(vec![wave(
        1,
        1,
        0,
        enemy(
            5,
            hold(START_X, 25_600),
            json!([attack_rule(always(), 1000)]),
        ),
        START_X,
        25_600,
    )]);
    e.step(idle());
    assert_eq!(e.events().len(), 1);
    e.step(idle());
    assert!(e.events().is_empty());
}

#[test]
fn the_snapshot_survives_json() {
    let mut e = running(vec![]);
    e.step(Input {
        move_x: 5,
        move_y: -5,
        evade: true,
        ..Input::default()
    });
    let s = e.snapshot();
    assert_eq!(s.tick, 1);
    assert_eq!(s.player.evade.dash_left, EVADE_TICKS - 1);
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), s);
}

// --- Determinism (ADR-004, ADR-019) ----------------------------------------------------------

fn fixture() -> Stage {
    // The fixture is written at design values and scaled like the corpus stages (ADR-103).
    let value = serde_json::from_str(FIXTURE).expect("the fixture is JSON");
    serde_json::from_value(common::scaled(value, common::SCALE)).expect("the fixture parses")
}

/// A fixed, varied script: drift about at different strengths, evade now and then, and play
/// the fixture's `giant_sword` every 200 ticks: slash, slash and the thrust held for 56 ticks (a
/// charge to level 2), then B and B again from the root (two slashes, each a smash that pulls).
fn script(tick: u32) -> Input {
    let (move_x, move_y) = match (tick / 45) % 4 {
        0 => (64, 0),
        1 => (-48, -32),
        2 => (0, 80),
        _ => (40, 40),
    };
    Input {
        move_x,
        move_y,
        evade: tick % 120 == 60,
        main: matches!(tick % 200, 0 | 8) || (16..72).contains(&(tick % 200)),
        smash: matches!(tick % 200, 100 | 108),
        ..Input::default()
    }
}

fn hashes(stage: &Stage, ticks: u32) -> Vec<u64> {
    let mut e = engine(stage);
    (0..ticks)
        .map(|t| {
            e.step(script(t));
            e.state_hash()
        })
        .collect()
}

/// The hash after `GOLDEN_TICKS` ticks of `script` on the fixture stage. Every platform must
/// produce exactly this (CI runs it on x86_64 and arm64); a change to it is a change to the
/// rules of version 0, so a PR that changes it regenerates this value and says why
/// (`Corpus regenerated: <why>`, ADR-035).
const GOLDEN_TICKS: u32 = 900;
const GOLDEN_HASH: u64 = 0xd4de_ae5e_c45d_e899;

#[test]
fn the_same_stage_and_inputs_give_the_same_hash_every_tick() {
    let stage = fixture();
    assert_eq!(hashes(&stage, 300), hashes(&stage, 300));
}

#[test]
fn the_seed_and_the_input_change_the_hash() {
    let stage = fixture();
    let mut other = stage.clone();
    other.seed += 1;
    assert_ne!(hashes(&stage, 30), hashes(&other, 30));
    let mut e = engine(&stage);
    let mut f = engine(&stage);
    e.step(idle());
    f.step(moving(1, 0));
    assert_ne!(e.state_hash(), f.state_hash());
}

#[test]
fn a_cloned_run_continues_identically() {
    let stage = fixture();
    let mut a = engine(&stage);
    for t in 0..200 {
        a.step(script(t));
    }
    let mut b = a.clone();
    for t in 200..400 {
        a.step(script(t));
        b.step(script(t));
        assert_eq!(a.state_hash(), b.state_hash(), "tick {t}");
    }
}

#[test]
fn the_golden_hash_is_the_same_on_every_platform() {
    let hash = *hashes(&fixture(), GOLDEN_TICKS).last().unwrap();
    assert_eq!(
        hash,
        GOLDEN_HASH,
        "golden hash differs on {}: now {hash:#018x}",
        std::env::consts::ARCH
    );
}

// --- No input breaks it ----------------------------------------------------------------------

#[test]
fn random_inputs_keep_every_invariant() {
    // Heavy fixtures: the fixture stage, and 500 shooters against a 99-hp character.
    let shooters = (0..200)
        .map(|_| {
            wave(
                1,
                100,
                1,
                enemy(
                    30,
                    json!({ "type": "straight", "vx": 0, "vy": 512 }),
                    json!([attack_rule(always(), 3)]),
                ),
                START_X,
                0,
            )
        })
        .collect();
    let mut heavy = stage_with(18_000, Some(far_boss()), shooters);
    heavy.player.hp = 99;
    for stage in [fixture(), heavy] {
        let mut e = engine(&stage);
        let mut rng = SplitMix64::new(0xE16E);
        let (mut last_hp, mut last_tick, mut finished_at) = (u32::MAX, 0, None);
        for _ in 0..2500 {
            let any = |rng: &mut SplitMix64| (rng.next_u64() & 0xff) as u8 as i8;
            let input = Input {
                move_x: any(&mut rng),
                move_y: any(&mut rng),
                evade: rng.below(8) == 0,
                main: rng.below(2) == 0,
                smash: rng.below(16) == 0,
                ..Input::default()
            };
            e.step(input);
            let s = e.snapshot();
            assert!((0..=92_160).contains(&s.player.at.x.0));
            assert!((0..=163_840).contains(&s.player.at.y.0));
            assert!(s.bullets.len() <= 4000);
            assert!(enemies(&s) <= 500);
            assert!(s.entities.windows(2).all(|w| w[0].id < w[1].id));
            // HP rises only by a heal (the fixture's companion heals the main character below
            // half its HP), as `tests/properties.rs` checks for every entity.
            let healed = e
                .events()
                .iter()
                .any(|ev| matches!(ev, DomainEvent::Healed { target, .. } if target.0 == 0));
            assert!(s.player.hp <= last_hp || healed, "hp rises only by a heal");
            last_hp = s.player.hp;
            assert!(e.tick() >= last_tick);
            last_tick = e.tick();
            if e.outcome() != Outcome::Running {
                finished_at.get_or_insert(e.tick());
            }
            if let Some(t) = finished_at {
                assert_eq!(e.tick(), t, "a finished run stops");
            }
        }
    }
}
