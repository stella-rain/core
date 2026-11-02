//! The Stardust time factor as a table of tiers (ADR-109 decision 2): each tier the tick it
//! starts at and a percent, the first at tick 0, the ticks rising strictly. A kill earns its base
//! times the percent of the last tier that started at or before the kill tick, truncated toward
//! zero; past the last tier's start (overtime included) the last tier holds. The `time_limit`
//! mode file gives the default table and a difficulty may give its own (ADR-110 decision 2).

use serde_json::{Value, json};
use stella_rain_core::difficulties::Difficulties;
use stella_rain_core::engine::Engine;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{Command, record_run, verify_run};
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::stage::{Stage, StardustTier};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{self, Error, limits};
use stella_rain_core::version::Versions;

use crate::common;

use crate::run_play::play_run_from;
use common::run::time_limit_setup;
use common::*;

const ENEMY_Y: i32 = 100_000;
const LENGTH: u32 = 1_200;
const MINUTE: u32 = 3_600;

fn tiers(list: &[(u32, u32)]) -> Value {
    let tiers: Vec<Value> = list
        .iter()
        .map(|&(start_tick, pct)| json!({ "start_tick": start_tick, "pct": pct }))
        .collect();
    json!({ "tiers": tiers })
}

fn stage(table: &[(u32, u32)], count: u16, base: u32) -> Stage {
    let mut w = wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, ENEMY_Y), json!([])),
        START_X,
        ENEMY_Y,
    );
    w["stardust"] = json!(base);
    let mut stage = stage_full(LENGTH, Some(far_boss()), vec![w], vec![]);
    stage.stardust = Some(serde_json::from_value(tiers(table)).expect("a table parses"));
    stage
}

/// The percent of the last tier that started at or before `tick`.
fn pct_at(table: &[(u32, u32)], tick: u32) -> u32 {
    table
        .iter()
        .rev()
        .find(|&&(start, _)| start <= tick)
        .map(|&(_, pct)| pct)
        .unwrap()
}

/// Fires for `ticks` ticks, returning each kill as (tick, Stardust added that tick).
fn kills(e: &mut Engine, ticks: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        let before = e.stardust();
        e.step(&[firing(idle(), e.tick())]);
        let gained = e.stardust() - before;
        if gained > 0 {
            out.push((e.tick(), gained));
        }
    }
    out
}

#[test]
fn a_kill_earns_the_percent_of_its_tier_truncated() {
    // Tiers close together so that kills land just before and at each start; an odd base so
    // that truncation shows (7 × 115% = 8.05 → 8).
    let table = [(0, 100), (40, 115), (80, 150), (120, 333)];
    let mut e = engine(&stage(&table, 40, 7));
    let got = kills(&mut e, 400);
    assert!(got.len() >= 30, "{got:?}");
    for &(tick, gained) in &got {
        assert_eq!(gained, 7 * pct_at(&table, tick) / 100, "tick {tick}");
    }
    let percents: Vec<u32> = got.iter().map(|&(t, _)| pct_at(&table, t)).collect();
    for (_, pct) in table {
        assert!(
            percents.contains(&pct),
            "no kill in the {pct}% tier: {got:?}"
        );
    }
}

#[test]
fn past_the_last_tier_its_percent_holds() {
    let table = [(0, 100), (10, 200)];
    let mut e = engine(&stage(&table, 40, 10));
    let got = kills(&mut e, 400);
    assert!(got.iter().filter(|&&(t, _)| t > 100).count() > 5, "{got:?}");
    assert!(
        got.iter().filter(|&&(t, _)| t >= 10).all(|&(_, g)| g == 20),
        "{got:?}"
    );
}

// --- The bundled tables -----------------------------------------------------------------------

fn run_tiers(difficulty: &str) -> Vec<(u32, u32)> {
    let setup = RunSetup {
        difficulty: difficulty.into(),
        ..time_limit_setup()
    };
    let stage = run_setup::stage(&setup).unwrap();
    stage
        .stardust
        .expect("a time_limit run counts Stardust")
        .tiers
        .iter()
        .map(|t| (t.start_tick, t.pct))
        .collect()
}

const MINUTES: [u32; 5] = [0, 5 * MINUTE, 10 * MINUTE, 15 * MINUTE, 20 * MINUTE];

#[test]
fn easy_normal_and_hard_take_the_modes_table() {
    let expected: Vec<(u32, u32)> = MINUTES.into_iter().zip([100, 110, 120, 135, 150]).collect();
    let mode: Vec<(u32, u32)> = TimeLimit::bundled()
        .stardust_tiers()
        .iter()
        .map(|t| (t.start_tick, t.pct))
        .collect();
    assert_eq!(mode, expected);
    for id in ["easy", "normal", "hard"] {
        assert_eq!(run_tiers(id), expected, "{id}");
        assert!(
            Difficulties::bundled()
                .get(&id.into())
                .unwrap()
                .stardust_tiers
                .is_none(),
            "{id}"
        );
    }
}

#[test]
fn rainy_gives_its_own_table() {
    let expected: Vec<(u32, u32)> = MINUTES.into_iter().zip([200, 240, 280, 340, 400]).collect();
    assert_eq!(run_tiers("rainy"), expected);
    for (tick, normal, rainy) in [(0, 100, 200), (5 * MINUTE, 110, 240), (u32::MAX, 150, 400)] {
        assert_eq!(pct_at(&run_tiers("normal"), tick), normal);
        assert_eq!(pct_at(&run_tiers("rainy"), tick), rainy);
    }
}

#[test]
fn easys_ten_minute_run_reaches_the_first_three_tiers() {
    let easy = Difficulties::bundled().get(&"easy".into()).unwrap();
    let length = easy.length_ticks.unwrap();
    assert_eq!(length, 10 * MINUTE);
    let table = run_tiers("easy");
    let reached: Vec<u32> = table
        .iter()
        .filter(|&&(start, _)| start <= length)
        .map(|&(_, pct)| pct)
        .collect();
    assert_eq!(reached, [100, 110, 120]);
}

#[test]
fn a_mode_run_earns_by_its_tier_and_its_replay_verifies() {
    // A regular enemy's base is 10: the first kills earn the first tier.
    for (id, first) in [("normal", 10), ("rainy", 20)] {
        let setup = RunSetup {
            difficulty: id.into(),
            ..time_limit_setup()
        };
        let commands = play_run_from(&setup, 1_500);
        let mut e = Engine::start(&setup).unwrap();
        let mut gains = Vec::new();
        for c in &commands {
            let before = e.stardust();
            match c {
                Command::Step(input) => e.step(&[*input]),
                Command::Choose(i) => common::pick(&mut e, *i).unwrap(),
            }
            if e.stardust() > before {
                gains.push(e.stardust() - before);
            }
        }
        assert_eq!(gains.first(), Some(&first), "{id}: {gains:?}");
        let replay = record_run(&setup, &commands).unwrap();
        let verified = verify_run(&replay.to_bytes()).unwrap();
        assert_eq!(verified.stardust, e.stardust(), "{id}");
    }
}

// --- Invalid tables are refused ---------------------------------------------------------------

fn bad_tables() -> Vec<(&'static str, Vec<(u32, u32)>)> {
    let too_many: Vec<(u32, u32)> = (0..=u32::try_from(limits::MAX_STARDUST_TIERS).unwrap())
        .map(|i| (i * 10, 100))
        .collect();
    vec![
        ("empty", vec![]),
        ("not at 0", vec![(1, 100)]),
        ("not rising", vec![(0, 100), (50, 110), (50, 120)]),
        ("falling", vec![(0, 100), (50, 110), (40, 120)]),
        ("pct 0", vec![(0, 100), (50, 0)]),
        ("pct past max", vec![(0, limits::MAX_STARDUST_PCT + 1)]),
        ("too many", too_many),
    ]
}

fn at_most() -> Vec<(u32, u32)> {
    (0..u32::try_from(limits::MAX_STARDUST_TIERS).unwrap())
        .map(|i| (i * 10, if i == 0 { 1 } else { limits::MAX_STARDUST_PCT }))
        .collect()
}

fn paths(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

#[test]
fn a_stage_refuses_a_bad_table() {
    assert_eq!(validate::validate(&stage(&at_most(), 1, 10)), Ok(()));
    for (name, table) in bad_tables() {
        let got = paths(validate::validate(&stage(&table, 1, 10)).unwrap_err());
        assert_eq!(got, ["$.stardust.tiers"], "{name}");
    }
}

#[test]
fn the_mode_file_refuses_a_bad_table() {
    let mode = |table: &[(u32, u32)]| {
        let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
        v["stardust_tiers"] = tiers(table)["tiers"].clone();
        TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT)
    };
    assert!(mode(&at_most()).is_ok());
    for (name, table) in bad_tables() {
        assert_eq!(
            paths(mode(&table).unwrap_err()),
            ["$.stardust_tiers"],
            "{name}"
        );
    }
}

#[test]
fn a_difficulty_refuses_a_bad_table() {
    let file = |table: &[(u32, u32)]| {
        let mut v: Value = serde_json::from_slice(common::bundled_file("difficulties")).unwrap();
        v["difficulties"][1]["stardust_tiers"] = tiers(table)["tiers"].clone();
        Difficulties::parse(v.to_string().as_bytes(), Versions::CURRENT)
    };
    let ok = file(&at_most()).unwrap();
    let content = RunContent::bundled().clone().with_difficulties(ok);
    let setup = time_limit_setup();
    let stage = run_setup::stage_in(&setup, &content).unwrap();
    let expected: Vec<StardustTier> = at_most()
        .into_iter()
        .map(|(start_tick, pct)| StardustTier { start_tick, pct })
        .collect();
    assert_eq!(stage.stardust.unwrap().tiers, expected);
    for (name, table) in bad_tables() {
        assert_eq!(
            paths(file(&table).unwrap_err()),
            ["$.difficulties[1].stardust_tiers"],
            "{name}"
        );
    }
}
