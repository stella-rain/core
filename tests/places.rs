//! `Engine::places`: the light per-tick read of ids, kinds and positions. It must show exactly what
//! `snapshot()` shows for those fields, on every tick, and change nothing.

use crate::common;
use crate::corpus::{apply, case_content, corpus_dir, runs_dir};

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use stella_rain_core::engine::{Engine, Place, Places};
use stella_rain_core::replay::Replay;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::run_setup;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::validate::parse_and_validate;

/// What the corpus walk saw, to show that it reaches what the tests claim to compare.
#[derive(Default)]
struct Seen {
    ticks: u64,
    kinds: BTreeSet<String>,
    ticks_with_stars: u64,
    boss_was_present: bool,
    boss_gone_after_present: bool,
}

/// Fills `places` for `engine` and asserts it equals what the snapshot shows; returns the kinds
/// of the entities, for the callers that count them.
pub(crate) fn assert_places_match(engine: &Engine, places: &mut Places) -> Vec<EntityKind> {
    engine.places(places);
    let snapshot = engine.snapshot();
    let entities: Vec<Place> = snapshot
        .entities
        .iter()
        .map(|e| Place {
            id: e.id,
            kind: e.kind,
            at: e.at,
        })
        .collect();
    assert_eq!(
        places.entities,
        entities,
        "entities at tick {}",
        engine.tick()
    );
    let stars: Vec<_> = snapshot.stars.iter().map(|s| (s.id, s.at)).collect();
    assert_eq!(places.stars, stars, "stars at tick {}", engine.tick());
    let boss = snapshot
        .entities
        .iter()
        .find(|e| e.kind == EntityKind::Boss)
        .map(|e| e.id);
    assert_eq!(places.boss, boss, "boss at tick {}", engine.tick());
    entities.iter().map(|p| p.kind).collect()
}

fn note(seen: &mut Seen, engine: &Engine, places: &mut Places) {
    for kind in assert_places_match(engine, places) {
        seen.kinds.insert(format!("{kind:?}"));
    }
    seen.ticks += 1;
    seen.ticks_with_stars += u64::from(!places.stars.is_empty());
    if places.boss.is_some() {
        seen.boss_was_present = true;
    } else if seen.boss_was_present {
        seen.boss_gone_after_present = true;
    }
}

fn cases(dir: &Path) -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    cases
}

/// Walks every corpus case (stage cases and run cases), comparing after every tick with one
/// reused `Places`.
fn walk_corpus() -> Seen {
    let mut seen = Seen::default();
    let mut places = Places::default();
    for case in cases(&corpus_dir()) {
        let stage = parse_and_validate(&fs::read(case.join("stage.json")).unwrap()).unwrap();
        let replay = Replay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
        let inputs = replay.unpack_inputs(stage.length_ticks).unwrap();
        let mut engine = Engine::new(&stage).unwrap();
        for input in inputs {
            engine.step(input);
            note(&mut seen, &engine, &mut places);
        }
    }
    for case in cases(&runs_dir()) {
        let recorded = RunReplay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
        let content = case_content(&case).unwrap();
        let stage = run_setup::stage_with(&recorded.setup, content.time_limit()).unwrap();
        let inputs = recorded
            .unpack_inputs(run_setup::max_replay_ticks(
                &recorded.setup,
                &stage,
                content.time_limit(),
            ))
            .unwrap();
        let commands = recorded.unpack_commands().unwrap();
        let mut engine = Engine::start_with(&recorded.setup, &content).unwrap();
        let mut next = commands.iter().peekable();
        for input in inputs {
            while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
                apply(&mut engine, c).unwrap();
            }
            engine.step(input);
            note(&mut seen, &engine, &mut places);
        }
    }
    seen
}

#[test]
fn places_agrees_with_the_snapshot_on_every_corpus_tick() {
    let seen = walk_corpus();
    assert!(seen.ticks > 0, "no corpus tick compared");
}

#[test]
fn the_corpus_reaches_every_entity_kind_it_claims() {
    let seen = walk_corpus();
    for kind in ["Enemy", "Boss", "BossPart"] {
        assert!(
            seen.kinds.contains(kind),
            "no {kind} in the corpus: {:?}",
            seen.kinds
        );
    }
    assert!(seen.ticks_with_stars > 0, "no tick with a star on the map");
    assert!(
        seen.boss_gone_after_present,
        "no tick where the boss is gone after being there"
    );
    eprintln!(
        "places: {} ticks compared, kinds {:?}, {} ticks with stars",
        seen.ticks, seen.kinds, seen.ticks_with_stars
    );
}

#[test]
fn a_dirty_buffer_is_cleared() {
    let stage = common::stage_full(18_000, Some(common::far_boss()), vec![], vec![]);
    let mut crowded = common::engine(&stage);
    for _ in 0..30 {
        crowded.step(common::idle());
    }
    let mut places = Places::default();
    crowded.places(&mut places);
    assert!(
        !places.entities.is_empty(),
        "the buffer should hold the boss"
    );

    let fresh = common::engine(&stage);
    fresh.places(&mut places);
    let mut clean = Places::default();
    fresh.places(&mut clean);
    assert_eq!(places, clean);
    assert_places_match(&fresh, &mut places);
}

#[test]
fn places_changes_nothing() {
    let stage = common::stage_full(18_000, Some(common::far_boss()), vec![], vec![]);
    let mut engine = common::engine(&stage);
    for _ in 0..10 {
        engine.step(common::idle());
    }
    let (hash, tick, events) = (engine.state_hash(), engine.tick(), engine.events().to_vec());
    let (mut a, mut b) = (Places::default(), Places::default());
    engine.places(&mut a);
    engine.places(&mut b);
    assert_eq!(a, b);
    assert_eq!(engine.state_hash(), hash);
    assert_eq!(engine.tick(), tick);
    assert_eq!(engine.events(), events.as_slice());
}
