//! The snapshot and places bytes, pinned over three worst-case walks, so that moving the view
//! (ADR-REPL-7 point 4) cannot change a byte of either.

use crate::bench_fixtures::{hit_case, time_limit_case, worst_case, worst_minute_case};
use crate::hash::StateHasher;
use crate::snapshot::EntityKind;

use super::*;

fn orb_cage() -> crate::stage::Stage {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/current/stages/orb_cage/stage.json");
    crate::validate::parse_and_validate(&std::fs::read(path).expect("orb_cage")).expect("valid")
}

/// What a walk saw of the views that are rare; the walks together must reach each.
#[derive(Default)]
struct Seen {
    attack: bool,
    boss: bool,
    link: bool,
    zone: bool,
    star: bool,
    offer: bool,
    expertise_offer: bool,
}

const TICKS: u32 = 600;

/// Hashes the snapshot's JSON and the places' debug text after a tick, and notes what it showed.
fn observe(e: &Engine, places: &mut Places, h: &mut StateHasher, seen: &mut Seen) {
    let snap = e.snapshot();
    h.write_bytes(&serde_json::to_vec(&snap).expect("a snapshot serializes"));
    e.places(places);
    h.write_bytes(format!("{places:?}").as_bytes());
    seen.attack |= snap.entities.iter().any(|v| v.attack.is_some());
    seen.boss |= snap.entities.iter().any(|v| v.kind == EntityKind::Boss);
    seen.link |= snap.orbs.iter().any(|o| o.link_to.is_some());
    seen.zone |= !snap.zones.is_empty();
    seen.star |= !snap.stars.is_empty();
    seen.offer |= snap.progress.as_ref().is_some_and(|p| p.offer.is_some());
    seen.expertise_offer |= snap.expertise.as_ref().is_some_and(|x| x.offer.is_some());
}

/// Plays `TICKS` ticks taking option 0 at every offer, observing each.
fn walk(mut e: Engine, input: fn(u32) -> Input, seen: &mut Seen) -> u64 {
    let mut h = StateHasher::new();
    let mut places = Places::default();
    for _ in 0..TICKS {
        if e.offer_open() {
            e.choose(0).expect("option 0 exists");
        }
        let next = e.tick() + 1;
        e.step(input(next));
        observe(&e, &mut places, &mut h, seen);
    }
    h.finish()
}

/// The corpus run cases that play on the bundled pack, in name order.
fn corpus_runs(seen: &mut Seen) -> u64 {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/current/runs");
    let mut names: Vec<_> = std::fs::read_dir(&root)
        .expect("the corpus runs")
        .map(|d| d.expect("an entry").path())
        .collect();
    names.sort();
    let bundled = crate::pack::Pack::bundled().sha256();
    let mut h = StateHasher::new();
    for case in names {
        let pinned = std::fs::read_to_string(case.join("case.json")).expect("case.json");
        let hex: String = bundled.iter().map(|b| format!("{b:02x}")).collect();
        if !pinned.contains(&hex) {
            continue;
        }
        let bytes = std::fs::read(case.join("run.replay")).expect("run.replay");
        let recorded = crate::run_replay::RunReplay::from_bytes(&bytes).expect("a run replay");
        let content = RunContent::bundled();
        let stage = run_setup::stage_with(&recorded.setup, content.time_limit()).expect("a stage");
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
        let mut places = Places::default();
        for input in inputs {
            while let Some(c) = next.next_if(|c| c.tick == e.tick()) {
                e.choose(c.index).expect("a recorded choice");
            }
            e.step(input);
            observe(&e, &mut places, &mut h, seen);
        }
    }
    h.finish()
}

#[test]
fn the_snapshot_and_places_bytes_are_pinned() {
    let mut seen = Seen::default();
    let got = [
        walk(worst_case::warmed_up(), worst_case::input, &mut seen),
        walk(hit_case::warmed_up(), hit_case::input, &mut seen),
        walk(
            time_limit_case::warmed_up(),
            time_limit_case::input,
            &mut seen,
        ),
        walk(
            worst_minute_case::worst_minute().engine,
            worst_minute_case::input_at,
            &mut seen,
        ),
        walk(
            Engine::new(&orb_cage()).expect("the stage runs"),
            worst_minute_case::input_at,
            &mut seen,
        ),
        corpus_runs(&mut seen),
    ];
    assert_eq!(
        got,
        [
            6_007_333_234_870_677_627,
            1_083_962_551_720_018_677,
            18_257_309_650_369_995_727,
            13_557_465_715_812_010_972,
            3_394_240_720_001_409_529,
            12_260_812_894_625_264_736
        ],
        "worst_case, hit_case, time_limit_case, worst_minute_case, orb_cage, corpus runs"
    );
    let missing: Vec<&str> = [
        (seen.attack, "an entity with an attack"),
        (seen.boss, "a boss entity"),
        (seen.link, "an orb with a link"),
        (seen.zone, "a zone"),
        (seen.star, "a star"),
        (seen.offer, "progress.offer"),
        (seen.expertise_offer, "expertise.offer"),
    ]
    .iter()
    .filter(|(ok, _)| !ok)
    .map(|(_, what)| *what)
    .collect();
    assert!(missing.is_empty(), "the walks never saw: {missing:?}");
}
