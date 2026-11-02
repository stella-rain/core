//! Stardust (ADR-109): every kill by the player earns the enemy's base times the time factor at
//! the kill tick, counted in the run state (so in the state hash), in the snapshot and in the
//! verified run, saturating at a cap. Bases are held times 10 (ADR-103 style), the factor is the
//! percent of a time tier (`stardust_tiers.rs`), integer math truncated toward zero.
//! A stage without a `stardust` factor counts nothing and hashes as it did.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error, limits};

use crate::common;

use common::*;

const ENEMY_Y: i32 = 100_000;
const LENGTH: u32 = 1_200;

/// `count` one-shot enemies waiting above the main character, one every 6 ticks from tick 1,
/// each with `base` Stardust when given.
fn targets(count: u16, base: Option<u32>) -> Value {
    let mut w = wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, ENEMY_Y), json!([])),
        START_X,
        ENEMY_Y,
    );
    if let Some(base) = base {
        w["stardust"] = json!(base);
    }
    w
}

/// A one-tier table: `pct` percent the whole run.
fn with_factor(mut stage: Stage, pct: u32) -> Stage {
    stage.stardust = Some(
        serde_json::from_value(json!({ "tiers": [{ "start_tick": 0, "pct": pct }] }))
            .expect("a factor parses"),
    );
    stage
}

fn stage(pct: u32, waves: Vec<Value>) -> Stage {
    with_factor(stage_full(LENGTH, Some(far_boss()), waves, vec![]), pct)
}

fn stardust(e: &Engine) -> u32 {
    e.snapshot().stardust.expect("a stage with a factor counts")
}

/// Fires for `ticks` ticks, returning each kill as (tick, Stardust added that tick).
fn kills(e: &mut Engine, ticks: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        let before = stardust(e);
        e.step(firing(idle(), e.tick()));
        let gained = stardust(e) - before;
        if gained > 0 {
            out.push((e.tick(), gained));
        }
    }
    out
}

#[test]
fn a_stage_without_a_factor_counts_nothing_and_its_snapshot_has_no_stardust() {
    let mut e = engine(&stage_full(
        LENGTH,
        Some(far_boss()),
        vec![targets(3, None)],
        vec![],
    ));
    for _ in 0..300 {
        e.step(firing(idle(), e.tick()));
    }
    assert_eq!(e.snapshot().stardust, None);
}

#[test]
fn a_kill_earns_its_base_at_a_flat_factor() {
    let mut e = engine(&stage(100, vec![targets(5, Some(10))]));
    assert_eq!(stardust(&e), 0);
    let got = kills(&mut e, 300);
    assert_eq!(got.len(), 5, "{got:?}");
    assert!(got.iter().all(|&(_, g)| g == 10), "{got:?}");
    assert_eq!(stardust(&e), 50);
}

#[test]
fn a_wave_that_names_no_base_earns_nothing() {
    let mut e = engine(&stage(100, vec![targets(3, None)]));
    assert!(kills(&mut e, 300).is_empty());
    assert_eq!(stardust(&e), 0);
}

#[test]
fn a_boss_kill_earns_the_bosss_base() {
    let mut boss = boss_at(10, 60_000);
    boss["stardust"] = json!(2_000);
    let s = with_factor(stage_full(LENGTH, Some(boss), vec![], vec![]), 100);
    let mut e = engine(&s);
    let got = kills(&mut e, 600);
    assert_eq!(got.iter().map(|&(_, g)| g).sum::<u32>(), 2_000, "{got:?}");
}

#[test]
fn the_run_total_saturates_at_the_cap() {
    // Every kill is worth the largest base at the largest percent: 1,000,000.
    let mut e = engine(&stage(
        1000,
        vec![targets(30, Some(limits::MAX_STARDUST_BASE))],
    ));
    kills(&mut e, 400);
    assert_eq!(stardust(&e), limits::MAX_RUN_STARDUST);
}

#[test]
fn the_counter_is_in_the_state_hash() {
    let run = |base| {
        let mut e = engine(&stage(100, vec![targets(2, Some(base))]));
        for _ in 0..120 {
            e.step(firing(idle(), e.tick()));
        }
        e.state_hash()
    };
    assert_eq!(run(10), run(10));
    assert_ne!(run(10), run(11));
}

fn problems(s: &Stage) -> Vec<String> {
    match validate::validate(s) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("expected issues, got {other:?}"),
    }
}

#[test]
fn bases_are_bounded() {
    let at_limit = stage(
        limits::MAX_STARDUST_PCT,
        vec![targets(1, Some(limits::MAX_STARDUST_BASE))],
    );
    assert_eq!(validate::validate(&at_limit), Ok(()));

    let past = stage(1, vec![targets(1, Some(limits::MAX_STARDUST_BASE + 1))]);
    assert_eq!(problems(&past), ["$.waves[0].stardust"]);

    let mut boss = boss_at(10, 60_000);
    boss["stardust"] = json!(limits::MAX_STARDUST_BASE + 1);
    let boss_past = with_factor(stage_full(LENGTH, Some(boss), vec![], vec![]), 100);
    assert_eq!(problems(&boss_past), ["$.boss.stardust"]);
}

#[test]
fn a_base_without_a_factor_counts_nothing() {
    let mut e = engine(&stage_full(
        LENGTH,
        Some(far_boss()),
        vec![targets(3, Some(10))],
        vec![],
    ));
    for _ in 0..300 {
        e.step(firing(idle(), e.tick()));
    }
    assert_eq!(e.snapshot().stardust, None);
}

// --- A mode run -------------------------------------------------------------------------------

use stella_rain_core::engine::Outcome;
use stella_rain_core::run_replay::{Command, record_run, verify_run};
use stella_rain_core::run_setup::RunSetup;

use crate::run_play::play_run_from;
use common::run::time_limit_setup;

fn setup(difficulty: &str) -> RunSetup {
    RunSetup {
        difficulty: difficulty.into(),
        ..time_limit_setup()
    }
}

/// Plays `commands` on a fresh run of `setup`, returning each kill's tick and its Stardust.
fn mode_kills(setup: &RunSetup, commands: &[Command]) -> Vec<(u32, u32)> {
    let mut e = Engine::start(setup).unwrap();
    let mut out = Vec::new();
    for c in commands {
        let before = e.stardust();
        match c {
            Command::Step(input) => e.step(*input),
            Command::Choose(i) => e.choose(PlayerIdx::SOLO, *i).unwrap(),
        }
        if e.stardust() > before {
            out.push((e.tick(), e.stardust() - before));
        }
    }
    assert_eq!(e.outcome(), Outcome::Running);
    out
}

#[test]
fn a_mode_run_earns_by_its_difficultys_factor_from_the_bundled_bases() {
    // A regular enemy's base is 10: the first kills, in the first minutes, earn the first tier
    // of their difficulty's table (100% on normal, 200% on rainy).
    for (id, first) in [("normal", 10), ("rainy", 20)] {
        let setup = setup(id);
        let commands = play_run_from(&setup, 1_500);
        let got = mode_kills(&setup, &commands);
        assert!(!got.is_empty(), "{id}: no kill in 1,500 ticks");
        assert_eq!(got[0].1, first, "{id}: {got:?}");
    }
}

#[test]
fn a_verified_run_reports_the_stardust_it_earned() {
    let setup = setup("normal");
    let commands = play_run_from(&setup, 1_500);
    let mut e = Engine::start(&setup).unwrap();
    for c in &commands {
        match c {
            Command::Step(input) => e.step(*input),
            Command::Choose(i) => e.choose(PlayerIdx::SOLO, *i).unwrap(),
        }
    }
    assert!(e.stardust() > 0);
    let replay = record_run(&setup, &commands).unwrap();
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.stardust, e.stardust());
}
