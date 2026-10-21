//! The `time_limit` run (core#212 step b): a setup naming the mode starts on the open field,
//! enemies spawn from the mode file's schedule under the cap on enemies alive, a mid-boss comes
//! at each boss time but the last, drawn from the pool without repeating an entry, and the final
//! boss comes at the last.

use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::{EntityKind, Snapshot};
use stella_rain_core::time_limit::TimeLimit;

fn setup(seed: u64) -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        seed,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        auto_combo: false,
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        difficulty: "normal".into(),
    }
}

/// An enemy that stands where it spawns and does nothing, so a run with it is only counted.
fn dummy() -> Value {
    json!({
        "base": "bat", "hp": 30, "radius": 2048,
        "movement": { "type": "straight", "vx": 0, "vy": 0 }
    })
}

/// The bundled file with `edit` applied.
fn mode(edit: impl FnOnce(&mut Value)) -> TimeLimit {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    edit(&mut v);
    TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

/// A short run with bosses at `ticks`, no enemies, and `pool` for its mid-bosses.
fn bosses_at(ticks: &[u32], pool: &[&str]) -> TimeLimit {
    mode(|v| {
        v["length_ticks"] = json!(ticks.last().unwrap());
        v["boss_ticks"] = json!(ticks);
        v["mid_boss_pool"] = json!(pool);
        v["waves"] = json!([]);
        v["elite_table"] =
            json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    })
}

fn start(mode: &TimeLimit, seed: u64) -> Engine {
    Engine::start_time_limit(&setup(seed), RunContent::bundled(), mode).unwrap()
}

fn step_to(e: &mut Engine, tick: u32) -> Snapshot {
    while e.tick() < tick {
        e.step(Input::default());
    }
    e.snapshot()
}

fn of_kind(s: &Snapshot, kind: EntityKind) -> Vec<(u32, String)> {
    s.entities
        .iter()
        .filter(|v| v.kind == kind)
        .map(|v| (v.id.0, v.asset.0.clone()))
        .collect()
}

#[test]
fn a_time_limit_setup_starts_on_the_open_field() {
    let mut e = Engine::start(&setup(1)).unwrap();
    let s = e.snapshot();
    assert_eq!((s.map.width.0, s.map.height.0), (1_048_576, 1_048_576));
    assert_eq!((s.player.at.x.0, s.player.at.y.0), (524_288, 524_288));
    for _ in 0..120 {
        e.step(Input::default());
    }
    assert_eq!(e.tick(), 120);
    assert_eq!(e.outcome(), Outcome::Running);
}

#[test]
fn a_run_of_the_bundled_mode_is_the_same_every_time() {
    let run = || {
        let mut e = Engine::start(&setup(7)).unwrap();
        (0..300)
            .map(|_| {
                e.step(Input::default());
                e.state_hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}

#[test]
fn enemies_spawn_on_the_schedule_around_the_main_character() {
    let ring = 102_400;
    let m = mode(|v| {
        v["waves"] = json!([{
            "at_tick": 30, "enemy": dummy(), "spawn": { "x": 0, "y": 0 },
            "count": 5, "every_ticks": 20, "around": ring
        }]);
    });
    let mut e = start(&m, 3);
    let counts: Vec<(u32, usize)> = [29, 30, 49, 50, 110, 300]
        .iter()
        .map(|&t| (t, of_kind(&step_to(&mut e, t), EntityKind::Enemy).len()))
        .collect();
    assert_eq!(
        counts,
        [(29, 0), (30, 1), (49, 1), (50, 2), (110, 5), (300, 5)]
    );
    let s = e.snapshot();
    let me = s.player.at;
    for v in s.entities.iter().filter(|v| v.kind == EntityKind::Enemy) {
        let (dx, dy) = (i64::from(v.at.x.0 - me.x.0), i64::from(v.at.y.0 - me.y.0));
        let d = (dx * dx + dy * dy).isqrt();
        let ring = i64::from(ring);
        assert!(
            (d - ring).abs() < ring / 20,
            "an enemy {d} away, not about {ring}"
        );
    }
}

#[test]
fn the_enemies_alive_never_pass_the_cap() {
    let m = mode(|v| {
        v["waves"] = json!([
            { "at_tick": 1, "enemy": dummy(), "spawn": { "x": 0, "y": 0 },
              "count": 400, "every_ticks": 1, "around": 102_400 },
            { "at_tick": 1, "enemy": dummy(), "spawn": { "x": 0, "y": 0 },
              "count": 400, "every_ticks": 1, "around": 51_200 }
        ]);
    });
    let mut e = start(&m, 5);
    let mut most = 0;
    for _ in 0..600 {
        e.step(Input::default());
        most = most.max(e.load().enemies);
        assert!(e.load().enemies <= 500);
    }
    assert_eq!(most, 500, "the cap is reached");
    assert_eq!(of_kind(&e.snapshot(), EntityKind::Enemy).len(), 500);
}

#[test]
fn a_mid_boss_comes_at_each_boss_time_but_the_last_and_the_final_boss_at_the_last() {
    let m = bosses_at(&[100, 200, 300, 400], &["bat", "fairy_a", "pilot_a"]);
    let mut e = start(&m, 11);
    assert!(of_kind(&step_to(&mut e, 99), EntityKind::Boss).is_empty());
    let mut seen = Vec::new();
    for t in [100, 150, 199, 200, 299, 300] {
        let b = of_kind(&step_to(&mut e, t), EntityKind::Boss);
        assert_eq!(b.len(), 1, "one boss at tick {t}");
        seen.push((t, b[0].clone()));
    }
    // Each time brings a new boss: the entity changes at 100, 200 and 300 only.
    assert_eq!(seen[0].1, seen[1].1);
    assert_eq!(seen[1].1, seen[2].1);
    assert_ne!(seen[2].1.0, seen[3].1.0);
    assert_eq!(seen[3].1, seen[4].1);
    assert_ne!(seen[4].1.0, seen[5].1.0);
    let last = of_kind(&step_to(&mut e, 400), EntityKind::Boss);
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].1, "golem", "the final boss is the file's");
    assert_eq!(e.outcome(), Outcome::Running);
}

/// The assets of the three mid-bosses of a run with `pool`.
fn drawn(pool: &[&str], seed: u64) -> Vec<String> {
    let mut e = start(&bosses_at(&[100, 200, 300, 400], pool), seed);
    [100, 200, 300]
        .iter()
        .map(|&t| of_kind(&step_to(&mut e, t), EntityKind::Boss)[0].1.clone())
        .collect()
}

#[test]
fn the_mid_bosses_are_drawn_without_repeating_a_pool_entry() {
    let mut orders = std::collections::BTreeSet::new();
    for seed in 0..24 {
        let mut d = drawn(&["bat", "fairy_a", "pilot_a"], seed);
        orders.insert(d.clone());
        d.sort();
        assert_eq!(d, ["bat", "fairy_a", "pilot_a"], "seed {seed}");
        let from_more = drawn(&["bat", "fairy_a", "pilot_a", "golem", "horns_2"], seed);
        let mut unique = from_more.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 3, "seed {seed}: {from_more:?}");
    }
    assert!(orders.len() > 1, "the draw depends on the seed: {orders:?}");
}

#[test]
fn equal_entries_are_different_entries() {
    assert_eq!(drawn(&["golem", "golem", "golem"], 9), ["golem"; 3]);
}

#[test]
fn the_draws_are_recorded_in_the_run_s_hash() {
    let hashes = |seed| {
        let mut e = start(
            &bosses_at(&[100, 200, 300, 400], &["bat", "fairy_a", "pilot_a"]),
            seed,
        );
        (0..400)
            .map(|_| {
                e.step(Input::default());
                e.state_hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(hashes(4), hashes(4));
    assert_ne!(hashes(4), hashes(5));
}
