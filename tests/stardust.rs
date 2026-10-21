//! Stardust (ADR-109): every kill by the player earns the enemy's base times the time factor at
//! the kill tick, counted in the run state (so in the state hash), in the snapshot and in the
//! verified run, saturating at a cap. Bases are held times 10 (ADR-103 style), the factor is a
//! percent that grows linearly over the run's length, integer math truncated toward zero.
//! A stage without a `stardust` factor counts nothing and hashes as it did.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
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

fn with_factor(mut stage: Stage, start: u32, end: u32) -> Stage {
    stage.stardust = Some(
        serde_json::from_value(json!({ "start_pct": start, "end_pct": end }))
            .expect("a factor parses"),
    );
    stage
}

fn stage(start: u32, end: u32, waves: Vec<Value>) -> Stage {
    with_factor(
        stage_full(LENGTH, Some(far_boss()), waves, vec![]),
        start,
        end,
    )
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

fn factor_pct(start: u32, end: u32, tick: u32) -> u64 {
    u64::from(start) + u64::from(end - start) * u64::from(tick.min(LENGTH)) / u64::from(LENGTH)
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
    let mut e = engine(&stage(100, 100, vec![targets(5, Some(10))]));
    assert_eq!(stardust(&e), 0);
    let got = kills(&mut e, 300);
    assert_eq!(got.len(), 5, "{got:?}");
    assert!(got.iter().all(|&(_, g)| g == 10), "{got:?}");
    assert_eq!(stardust(&e), 50);
}

#[test]
fn a_wave_that_names_no_base_earns_nothing() {
    let mut e = engine(&stage(100, 100, vec![targets(3, None)]));
    assert!(kills(&mut e, 300).is_empty());
    assert_eq!(stardust(&e), 0);
}

#[test]
fn the_factor_grows_linearly_over_the_run_by_the_kill_tick_truncated() {
    // 200% to 400% over 18,000 ticks, an odd base so that truncation shows.
    let mut e = engine(&stage(200, 400, vec![targets(40, Some(7))]));
    let got = kills(&mut e, 600);
    assert!(got.len() >= 30, "{got:?}");
    for (tick, gained) in &got {
        let expected = 7 * factor_pct(200, 400, *tick) / 100;
        assert_eq!(u64::from(*gained), expected, "tick {tick}");
    }
    // The factor moved, so the later kills earn more than the first.
    assert!(got.last().unwrap().1 > got.first().unwrap().1, "{got:?}");
}

#[test]
fn a_boss_kill_earns_the_bosss_base() {
    let mut boss = boss_at(10, 60_000);
    boss["stardust"] = json!(2_000);
    let s = with_factor(stage_full(LENGTH, Some(boss), vec![], vec![]), 100, 100);
    let mut e = engine(&s);
    let got = kills(&mut e, 600);
    assert_eq!(got.iter().map(|&(_, g)| g).sum::<u32>(), 2_000, "{got:?}");
}

#[test]
fn the_run_total_saturates_at_the_cap() {
    // Every kill is worth the largest base at the largest percent: 1,000,000.
    let mut e = engine(&stage(
        1000,
        1000,
        vec![targets(30, Some(limits::MAX_STARDUST_BASE))],
    ));
    kills(&mut e, 400);
    assert_eq!(stardust(&e), limits::MAX_RUN_STARDUST);
}

#[test]
fn the_counter_is_in_the_state_hash() {
    let run = |base| {
        let mut e = engine(&stage(100, 100, vec![targets(2, Some(base))]));
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
fn bases_percents_and_the_factor_are_bounded() {
    let at_limit = stage(
        1,
        limits::MAX_STARDUST_PCT,
        vec![targets(1, Some(limits::MAX_STARDUST_BASE))],
    );
    assert_eq!(validate::validate(&at_limit), Ok(()));

    let past = stage(1, 1, vec![targets(1, Some(limits::MAX_STARDUST_BASE + 1))]);
    assert_eq!(problems(&past), ["$.waves[0].stardust"]);

    let past_pct = stage(1, limits::MAX_STARDUST_PCT + 1, vec![targets(1, None)]);
    assert_eq!(problems(&past_pct), ["$.stardust.end_pct"]);

    let zero = stage(0, 100, vec![targets(1, None)]);
    assert_eq!(problems(&zero), ["$.stardust.start_pct"]);

    let falling = stage(200, 199, vec![targets(1, None)]);
    assert_eq!(problems(&falling), ["$.stardust.end_pct"]);

    let mut boss = boss_at(10, 60_000);
    boss["stardust"] = json!(limits::MAX_STARDUST_BASE + 1);
    let boss_past = with_factor(stage_full(LENGTH, Some(boss), vec![], vec![]), 100, 100);
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

use common::run::{play_run_from, time_limit_setup};

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
            Command::Choose(i) => e.choose(*i).unwrap(),
            Command::Spend(i) => e.spend(*i).unwrap(),
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
    // A regular enemy's base is 10: the first kills, in the first minutes, earn the start factor
    // of their difficulty (100% on normal, 200% on rainy) plus a hair.
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
            Command::Choose(i) => e.choose(*i).unwrap(),
            Command::Spend(i) => e.spend(*i).unwrap(),
        }
    }
    assert!(e.stardust() > 0);
    let replay = record_run(&setup, &commands).unwrap();
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.stardust, e.stardust());
}
