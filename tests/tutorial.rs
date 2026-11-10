//! The tutorial mode: a three-minute run of gentle bats and one weak golem, with no
//! elites and no mid-boss, that every bundled weapon can be played in, and whose file is in the
//! pack the run's content hash covers.

use crate::common;
use crate::common::run::{run_input_for, trial_setup};
use crate::run_play::play_run_with;
use serde_json::Value;
use sha2::{Digest, Sha256};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::pack::Pack;
use stella_rain_core::registry::{self, Kind};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::validate;

/// Three minutes at 60 ticks a second.
const LENGTH: u32 = 3 * 60 * 60;

fn setup(weapon: &str) -> RunSetup {
    RunSetup {
        mode: "tutorial".into(),
        weapon: weapon.into(),
        ..trial_setup()
    }
}

/// The tick the tutorial golem comes onto the field (the file's `arrives_at_tick`).
const ARRIVAL: u32 = 4_000;

/// The bundled mode file, as JSON.
fn mode_json() -> Value {
    serde_json::from_slice(run_setup::mode_stage("tutorial").unwrap()).unwrap()
}

#[test]
fn the_tutorial_mode_is_registered_and_its_file_validates() {
    assert!(
        registry::ENTRIES
            .iter()
            .any(|e| e.kind == Kind::Mode && e.id == "tutorial"),
        "tutorial is a mode of the registry"
    );
    let stage = validate::parse_and_validate(run_setup::mode_stage("tutorial").unwrap()).unwrap();
    assert_eq!(stage.length_ticks, LENGTH);
    let run = run_setup::stage(&setup("giant_sword")).unwrap();
    assert_eq!(run.length_ticks, LENGTH);
}

#[test]
fn the_tutorial_run_is_three_minutes() {
    assert_eq!(LENGTH, 10_800);
    assert_eq!(mode_json()["length_ticks"], LENGTH);
}

#[test]
fn the_tutorial_has_one_weak_boss_and_no_elites_or_mid_boss() {
    let m = mode_json();
    let waves = m["waves"].as_array().unwrap();
    assert!(!waves.is_empty(), "the tutorial has bats");
    assert!(
        waves.iter().all(|w| w["enemy"]["elite"].is_null()),
        "no elite in any wave"
    );
    assert_eq!(m["boss"]["base"], "golem");
    assert!(
        m["boss"]["call_elites"].is_null(),
        "the boss calls no elites"
    );
    assert!(m["boss"]["parts"].as_array().unwrap().is_empty());
    assert_eq!(m["boss"]["phases"].as_array().unwrap().len(), 1);
    assert!(
        m["boss"]["hp"].as_u64().unwrap() <= 30_000,
        "a weak boss: its hit points stay low"
    );
    assert!(m.get("boss_ticks").is_none() && m.get("mid_boss_pool").is_none());
}

#[test]
fn every_bundled_weapon_runs_the_tutorial_and_its_replay_verifies() {
    let weapons: Vec<&str> = registry::ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Weapon)
        .map(|e| e.id)
        .collect();
    assert!(weapons.len() >= 2, "more than one weapon is bundled");
    for weapon in weapons {
        let setup = setup(weapon);
        let commands = play_run_with(&setup, RunContent::bundled(), LENGTH, |e, t| {
            run_input_for(e, t, &[EntityKind::Enemy, EntityKind::Boss], u32::MAX)
        });
        let recorded =
            run_replay::record_run(&setup, &commands).unwrap_or_else(|e| panic!("{weapon}: {e:?}"));
        let verified = run_replay::verify_run(&recorded.to_bytes())
            .unwrap_or_else(|e| panic!("{weapon}: {e:?}"));
        assert_ne!(verified.outcome, Outcome::Running, "{weapon} ends the run");
        assert_eq!(
            verified.ticks, recorded.ticks,
            "{weapon} replays every tick"
        );
    }
}

#[test]
fn the_bot_kills_the_golem_in_the_tutorial() {
    let setup = setup("giant_sword");
    let commands = play_run_with(&setup, RunContent::bundled(), LENGTH, |e, t| {
        run_input_for(e, t, &[EntityKind::Enemy, EntityKind::Boss], u32::MAX)
    });
    let recorded = run_replay::record_run(&setup, &commands).unwrap();
    let verified = run_replay::verify_run(&recorded.to_bytes()).unwrap();
    assert_eq!(verified.outcome, Outcome::Cleared);
    assert!(
        verified.ticks < LENGTH,
        "the golem dies before the time is up"
    );
    assert!(verified.ticks > ARRIVAL, "no clear before the golem exists");
}

#[test]
fn the_bundled_pack_holds_the_tutorial_file_and_the_run_hash_covers_it() {
    let pack: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let file: Value = serde_json::from_slice(common::bundled_file("tutorial")).unwrap();
    assert_eq!(pack["modes"]["tutorial"], file);
    let sha: [u8; 32] = Sha256::digest(Pack::BUNDLED).into();
    assert_eq!(run_replay::content_sha256("tutorial"), Some(sha));
    let mut edited = pack.clone();
    edited["modes"]["tutorial"]["length_ticks"] = 0.into();
    let edited = serde_json::to_vec(&edited).unwrap();
    assert!(
        Pack::from_bytes(&edited).is_err(),
        "a pack whose tutorial file is changed is refused"
    );
}

/// The tutorial earns no Stardust: a stage with no Stardust factor counts
/// nothing, and the tutorial's stage has none, so the bot's run, which kills what it reaches,
/// leaves the counter at zero.
#[test]
fn the_tutorial_earns_no_stardust() {
    let setup = setup("giant_sword");
    let bundled = RunContent::bundled();
    let commands = play_run_with(&setup, bundled, LENGTH, |e, t| {
        run_input_for(e, t, &[EntityKind::Enemy, EntityKind::Boss], u32::MAX)
    });
    let mut engine = Engine::start_with(&setup, bundled).unwrap();
    let mut kills = 0;
    for command in &commands {
        match *command {
            Command::Step(input) => {
                engine.step(&[input, Default::default(), Default::default()]);
                kills += engine
                    .events()
                    .iter()
                    .filter(|e| matches!(e, DomainEvent::Died { .. }))
                    .count();
            }
            #[allow(unreachable_patterns)]
            _ => unreachable!("a run at the live rules holds no choice"),
        }
    }
    assert!(
        kills > 0,
        "the bot kills something, so the counter is tested"
    );
    assert_eq!(engine.stardust(), 0);
}

#[test]
fn the_file_opts_the_golem_into_the_chase_and_a_late_arrival() {
    let m = mode_json();
    assert_eq!(m["sim_version"], 2);
    let boss = &m["boss"];
    assert_eq!(boss["chase"]["speed"], 256, "1 px a tick");
    assert_eq!(boss["chase"]["stop_distance"], 24_576, "96 px");
    assert_eq!(boss["arrives_at_tick"], ARRIVAL);
    validate::parse_and_validate(run_setup::mode_stage("tutorial").unwrap()).unwrap();
}

/// What the golem and the player do, tick by tick: the bot fights the bats until `ARRIVAL`, then
/// the player stands still. Each row is the engine tick after a step, the boss's position (if any)
/// and the player's.
type Row = (u32, Option<(i32, i32)>, (i32, i32), Outcome);

fn trace(until: u32) -> Vec<Row> {
    let setup = setup("giant_sword");
    let bundled = RunContent::bundled();
    let commands = play_run_with(&setup, bundled, until, |e, t| {
        if t >= ARRIVAL {
            return Default::default();
        }
        run_input_for(e, t, &[EntityKind::Enemy], u32::MAX)
    });
    let mut engine = Engine::start_with(&setup, bundled).unwrap();
    let mut rows = Vec::new();
    for command in &commands {
        match *command {
            Command::Step(input) => {
                engine.step(&[input, Default::default(), Default::default()]);
                let s = engine.snapshot();
                let boss: Vec<_> = s
                    .entities
                    .iter()
                    .filter(|v| v.kind == EntityKind::Boss)
                    .collect();
                assert!(boss.len() <= 1, "one golem at most");
                let me = s.players[0].at;
                rows.push((
                    engine.tick(),
                    boss.first().map(|b| (b.at.x.0, b.at.y.0)),
                    (me.x.0, me.y.0),
                    engine.outcome(),
                ));
            }
            #[allow(unreachable_patterns)]
            _ => unreachable!("a run at the live rules holds no choice"),
        }
    }
    rows
}

#[test]
fn the_golem_is_not_on_the_field_before_the_arrival() {
    let rows = trace(ARRIVAL + 600);
    let setup = setup("giant_sword");
    let mut engine = Engine::start_with(&setup, RunContent::bundled()).unwrap();
    let commands = play_run_with(&setup, RunContent::bundled(), ARRIVAL, |e, t| {
        run_input_for(e, t, &[EntityKind::Enemy], u32::MAX)
    });
    let mut first_empty = None;
    let mut seen_bat = false;
    for command in &commands {
        match *command {
            Command::Step(input) => {
                engine.step(&[input, Default::default(), Default::default()]);
                let s = engine.snapshot();
                if engine.tick() < ARRIVAL {
                    assert!(s.entities.iter().all(|v| v.kind != EntityKind::Boss));
                    assert_eq!(engine.outcome(), Outcome::Running);
                    let bats = s.entities.iter().any(|v| v.kind == EntityKind::Enemy);
                    seen_bat |= bats;
                    if seen_bat && first_empty.is_none() && !bats && engine.tick() > 1_260 {
                        first_empty = Some(engine.tick());
                    }
                }
            }
            #[allow(unreachable_patterns)]
            _ => unreachable!("a run at the live rules holds no choice"),
        }
    }
    let empty = first_empty.expect("the bats run out before the golem comes");
    assert!(empty > 1_260 && empty < ARRIVAL, "empty field at {empty}");
    for (tick, boss, _, _) in &rows {
        assert_eq!(boss.is_some(), *tick >= ARRIVAL, "tick {tick}");
    }
    let (_, boss, _, _) = rows.iter().find(|r| r.0 == ARRIVAL).unwrap();
    let (x, y) = boss.unwrap();
    let (dx, dy) = (i64::from(x - 163_840), i64::from(y - 35_840));
    assert!(
        dx * dx + dy * dy <= 256 * 256,
        "its first step from the spawn"
    );
}

#[test]
fn the_golem_walks_toward_a_standing_player_and_stops_96_px_away() {
    let rows = trace(ARRIVAL + 700);
    let again = trace(ARRIVAL + 700);
    assert_eq!(rows, again, "the same positions every run");
    let dist2 = |b: (i32, i32), p: (i32, i32)| {
        let (dx, dy) = (i64::from(b.0 - p.0), i64::from(b.1 - p.1));
        dx * dx + dy * dy
    };
    // The golem stops within one step of 96 px (24,576 units): never nearer than a step inside it.
    let (stop, step) = (24_576_i64, 256_i64);
    let mut prev: Option<(i32, i32)> = None;
    let mut reached = false;
    for (tick, boss, me, outcome) in rows.iter().filter(|r| r.0 >= ARRIVAL) {
        assert_eq!(*outcome, Outcome::Running, "tick {tick}");
        let b = boss.expect("the golem is there");
        if let Some(p) = prev {
            let moved = dist2(b, p);
            assert!(moved <= step * step, "tick {tick}: moved {moved}");
            assert!(dist2(b, *me) <= dist2(p, *me), "tick {tick}: moved away");
        }
        let d = dist2(b, *me);
        assert!(d >= (stop - step) * (stop - step), "tick {tick}: too near");
        if d <= (stop + step) * (stop + step) {
            reached = true;
        }
        prev = Some(b);
    }
    // It has stopped for good: the last hundred ticks it does not move.
    let last = rows.last().unwrap();
    let before = rows.iter().find(|r| r.0 == last.0 - 100).unwrap();
    assert_eq!(last.1, before.1, "the golem stands at its stop distance");
    assert!(reached, "the golem reaches 96 px within 700 ticks");
}
