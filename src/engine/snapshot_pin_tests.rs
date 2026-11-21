//! The snapshot and places bytes, pinned over three worst-case walks, so that moving the view
//! (ADR-REPL-2 point 4) cannot change a byte of either.
//!
//! The pins of the walks on the newest rules were set again when the entity numbers 0 to 2 were
//! reserved for the player slots (ADR-SIM-7): every other entity's number in the bytes is 2
//! higher at one player. `tests/gameplay_observables.rs` pins the same views with those numbers
//! mapped back, and did not move. The walks on version 1 engines are as they were.
//!
//! The pins moved once when the player view gained `max_hp`: the only new bytes are that key.

use crate::bench_fixtures::{hit_case, time_limit_case, worst_case, worst_minute_case};
use crate::fixed::Point;
use crate::hash::StateHasher;
use crate::snapshot::{EntityKind, PlayerView, Snapshot};

use super::*;

fn orb_cage() -> crate::stage::Stage {
    let bytes = include_bytes!("../../tests/corpus/v1/stages/orb_cage/stage.json");
    crate::validate::parse_and_validate(bytes).expect("a valid stage")
}

/// What a walk hashes after each tick.
#[derive(Clone, Copy)]
enum Hashed {
    /// The whole snapshot's JSON (as a `Value`) and the places' debug text.
    Whole,
    /// Only the first player's view and position.
    FirstPlayer,
}

fn first_player(snap: &Snapshot) -> &PlayerView {
    &snap.players[0]
}

fn first_place(places: &Places) -> Point {
    places.players[0]
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
}

const TICKS: u32 = 600;

/// Hashes the snapshot's JSON and the places' debug text after a tick, and notes what it showed.
fn observe(e: &Engine, places: &mut Places, h: &mut StateHasher, seen: &mut Seen, hashed: Hashed) {
    let snap = e.snapshot();
    e.places(places);
    match hashed {
        Hashed::Whole => {
            // Through a `Value`, whose keys sort: the pins were recorded over the snapshot as a
            // `Value`, once `progress.offer` had left the text (the offer left the snapshot).
            let value = serde_json::to_value(&snap).expect("a snapshot serializes");
            h.write_bytes(value.to_string().as_bytes());
            h.write_bytes(format!("{places:?}").as_bytes());
        }
        Hashed::FirstPlayer => {
            let view = first_player(&snap);
            h.write_bytes(&serde_json::to_vec(view).expect("a view serializes"));
            h.write_bytes(format!("{:?}", first_place(places)).as_bytes());
        }
    }
    seen.attack |= snap.entities.iter().any(|v| v.attack.is_some());
    seen.boss |= snap.entities.iter().any(|v| v.kind == EntityKind::Boss);
    seen.link |= snap.orbs.iter().any(|o| o.link_to.is_some());
    seen.zone |= !snap.zones.is_empty();
    seen.star |= !snap.stars.is_empty();
    seen.offer |= e.offer_open();
}

/// Plays `TICKS` ticks taking option 0 at every offer, observing each.
fn walk(mut e: Engine, input: fn(u32) -> Input, seen: &mut Seen, hashed: Hashed) -> u64 {
    let mut h = StateHasher::new();
    let mut places = Places::default();
    for _ in 0..TICKS {
        if e.offer_open() {
            e.v1_pick_step(&[Some(0), None, None])
                .expect("option 0 exists");
        }
        let next = e.tick() + 1;
        e.step(&[input(next), Default::default(), Default::default()]);
        observe(&e, &mut places, &mut h, seen, hashed);
    }
    h.finish()
}

/// The frozen version 1 run cases that play on its pack, so the walk does not move with the
/// bundled content.
fn corpus_runs(seen: &mut Seen, hashed: Hashed) -> u64 {
    let pack = crate::pack::Pack::from_bytes(include_bytes!(
        "../../tests/corpus/v1/packs/32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5.json"
    ))
    .expect("the version 1 pack");
    let content = pack.content();
    let replays: [&[u8]; 5] = [
        include_bytes!("../../tests/corpus/v1/runs/easy/run.replay"),
        include_bytes!("../../tests/corpus/v1/runs/rainy/run.replay"),
        include_bytes!("../../tests/corpus/v1/runs/time_limit/run.replay"),
        include_bytes!("../../tests/corpus/v1/runs/trial/run.replay"),
        include_bytes!("../../tests/corpus/v1/runs/tutorial/run.replay"),
    ];
    let mut h = StateHasher::new();
    for bytes in replays {
        let recorded = crate::run_replay::RunReplay::from_bytes(bytes).expect("a run replay");
        // The stage comes from the version 1 pack's own mode files, not the bundled ones: the
        // bundled tutorial file is a version 2 stage.
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
        let mut places = Places::default();
        for input in inputs {
            while let Some(c) = next.next_if(|c| c.tick == e.tick()) {
                e.v1_pick_step(&[Some(c.index), None, None])
                    .expect("a recorded choice");
            }
            e.step(&[input, Default::default(), Default::default()]);
            observe(&e, &mut places, &mut h, seen, hashed);
        }
    }
    h.finish()
}

/// The six walks, hashing `hashed`.
fn six_walks(seen: &mut Seen, hashed: Hashed) -> [u64; 6] {
    [
        walk(worst_case::warmed_up(), worst_case::input, seen, hashed),
        walk(hit_case::warmed_up(), hit_case::input, seen, hashed),
        walk(
            time_limit_case::warmed_up(),
            time_limit_case::input,
            seen,
            hashed,
        ),
        walk(
            worst_minute_case::worst_minute().engine,
            worst_minute_case::input_at,
            seen,
            hashed,
        ),
        walk(
            Engine::new(&orb_cage()).expect("the stage runs"),
            worst_minute_case::input_at,
            seen,
            hashed,
        ),
        corpus_runs(seen, hashed),
    ]
}

/// The snapshot and places bytes, which are those of before the offer left the snapshot with
/// `progress.offer` taken out: the offer view's removal changed no other byte.
#[test]
fn the_snapshot_and_places_bytes_are_pinned() {
    let mut seen = Seen::default();
    let got = six_walks(&mut seen, Hashed::Whole);
    assert_eq!(
        got,
        [
            17_491_086_472_955_404_066,
            4_128_990_721_140_487_317,
            2_497_766_836_364_182_085,
            14_113_708_835_994_285_066,
            4_085_162_667_984_718_047,
            4_415_562_993_771_971_609
        ],
        "worst_case, hit_case, time_limit_case, worst_minute_case, orb_cage, corpus runs"
    );
    let missing: Vec<&str> = [
        (seen.attack, "an entity with an attack"),
        (seen.boss, "a boss entity"),
        (seen.link, "an orb with a link"),
        (seen.zone, "a zone"),
        (seen.star, "a star"),
        (seen.offer, "an offer open (version 1)"),
    ]
    .iter()
    .filter(|(ok, _)| !ok)
    .map(|(_, what)| *what)
    .collect();
    assert!(missing.is_empty(), "the walks never saw: {missing:?}");
}

/// Pins the first player's view and position bytes before the view becomes a list.
#[test]
fn the_player_view_bytes_are_pinned() {
    let got = six_walks(&mut Seen::default(), Hashed::FirstPlayer);
    assert_eq!(
        got,
        [
            15_509_909_753_405_260_730,
            17_672_839_608_273_985_701,
            4_689_982_256_015_557_451,
            9_970_105_601_580_836_545,
            5_976_785_851_359_207_326,
            2_149_628_225_251_215_676
        ],
        "worst_case, hit_case, time_limit_case, worst_minute_case, orb_cage, corpus runs (time_limit_case moved when the mode bosses began to chase)"
    );
}

#[test]
fn run_snapshot_is_the_engines() {
    let mut e = worst_case::warmed_up();
    let (mut p, mut q) = (Places::default(), Places::default());
    for tick in 0..=100 {
        if tick == 0 || tick == 1 || tick == 100 {
            assert_eq!(e.run.snapshot(e.content()), e.snapshot(), "tick {tick}");
            e.run.places(&mut p);
            e.places(&mut q);
            assert_eq!(p, q, "tick {tick}");
        }
        e.step(&[
            worst_case::input(e.tick() + 1),
            Default::default(),
            Default::default(),
        ]);
    }
}

/// The Wave 3 views (derived each frame, never hashed): with every field zero the snapshot holds
/// no `size`, `encore` or `lingers` key; each is built from its state while that is non-zero.
/// `sweep_hits`, `rune_fire_owed` and `stack_given` have no view and change no snapshot byte.
#[test]
fn the_wave_three_views_are_absent_at_zero_and_built_from_the_state() {
    use crate::id::PlayerIdx;
    use crate::sim::state::{Linger, LingerAnchor};
    let mut e = super::live_support::live_bundled_run("normal", 1);
    e.join(PlayerIdx::new(1).unwrap()).unwrap();
    let zero = serde_json::to_value(e.snapshot()).unwrap();
    assert!(zero.get("lingers").is_none());
    for p in zero["players"].as_array().unwrap() {
        assert!(p.get("size").is_none() && p.get("encore").is_none());
    }

    // The fields without a view change no byte.
    {
        let s = e.run.live_mut();
        s.players[PlayerIdx::SOLO].sweep_hits = vec![crate::id::EntityId(5)];
        s.players[PlayerIdx::SOLO].combat.rune_fire_owed = 1;
    }
    assert_eq!(serde_json::to_value(e.snapshot()).unwrap(), zero);

    {
        let s = e.run.live_mut();
        let c = &mut s.players[PlayerIdx::SOLO].combat;
        c.size_stacks = 2;
        c.size_ticks = 90;
        c.encore_ticks = 50;
        c.encore_ready = 1;
    }
    let v = serde_json::to_value(e.snapshot()).unwrap();
    let p0 = &v["players"][0];
    assert_eq!(
        p0["size"],
        serde_json::json!({"stacks": 2, "ticks_left": 90})
    );
    assert_eq!(
        p0["encore"],
        serde_json::json!({"ticks_left": 50, "ready": true})
    );
    assert!(v["players"][1].get("size").is_none());
    assert!(v["players"][1].get("encore").is_none());
    e.run.live_mut().players[PlayerIdx::SOLO]
        .combat
        .encore_ready = 0;
    let v = serde_json::to_value(e.snapshot()).unwrap();
    assert_eq!(
        v["players"][0]["encore"],
        serde_json::json!({"ticks_left": 50})
    );

    let at = Point::default();
    let owner_at = e.run.live().players[PlayerIdx::new(1).unwrap()].at;
    let linger = |owner: u8, anchor: LingerAnchor| Linger {
        owner,
        anchor,
        ticks_left: 40,
        ticks: 100,
        next_pulse: 5,
        rate_ticks: 10,
        damage: 7,
        damage_pct: 50,
        shape: crate::attack::ZoneShape::Circle {
            radius: crate::fixed::Fx(256),
        },
        pull: crate::fixed::Fx(0),
        cuts_orbs: false,
        proc_pct: None,
        damage_type: None,
    };
    e.run.live_mut().lingers = vec![
        linger(
            0,
            LingerAnchor::Place {
                at,
                facing: crate::sim::state::Dir::UP,
            },
        ),
        linger(1, LingerAnchor::Player),
    ];
    let v = serde_json::to_value(e.snapshot()).unwrap();
    let l = v["lingers"].as_array().unwrap();
    assert_eq!(l.len(), 2);
    assert_eq!(l[0]["owner"], "first");
    assert_eq!(l[0]["follows_owner"], false);
    assert_eq!(
        (l[0]["ticks_left"].as_u64(), l[0]["ticks"].as_u64()),
        (Some(40), Some(100))
    );
    assert_eq!(l[1]["owner"], "second");
    assert_eq!(l[1]["follows_owner"], true);
    assert_eq!(l[1]["at"], serde_json::to_value(owner_at).unwrap());
}
