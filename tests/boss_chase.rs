//! A boss that chases (`Boss.chase`) and a boss that arrives later (`Boss.arrives_at_tick`),
//! both opt-in and absent by default, in the open `sim_version` (ADR-SIM-4, ADR-REPL-3
//! decision 2). The boss stands at (46080, 60000), 87,456 straight above the main character at
//! (46080, 147456), with hit points that outlast a test and a weapon that does nothing.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{Error, Issue, validate};

const BOSS_X: i32 = 46_080;
const BOSS_Y: i32 = 60_000;
/// The default chase: 1 pixel a tick, stopping 96 pixels from its target.
const SPEED_PLACEHOLDER: i32 = 256;
const STOP: i32 = 24_576;

fn boss_json(extra: Value) -> Value {
    let mut b = json!({ "base": "golem", "hp": 1_000_000, "radius": 8192,
                        "spawn": { "x": BOSS_X, "y": BOSS_Y }, "phases": [{}] });
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

fn chase(speed: i32, stop: i32) -> Value {
    json!({ "chase": { "speed": speed, "stop_distance": stop } })
}

/// A stage at the open version with this boss and a quiet weapon.
fn stage_of(boss: Value, length: u32) -> Stage {
    let mut stage = quiet(stage_with(length, Some(boss), vec![]));
    stage.sim_version = stella_rain_core::version::SIM_VERSION;
    stage
}

fn boss_at(e: &Engine) -> Option<(i32, i32)> {
    e.snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Boss)
        .map(|v| (v.at.x.0, v.at.y.0))
}

fn tick(e: &mut Engine) {
    e.step(&[idle(), Default::default(), Default::default()]);
}

/// The boss's place after each of `ticks` ticks.
fn path(stage: &Stage, ticks: u32) -> Vec<(i32, i32)> {
    let mut e = engine(stage);
    (0..ticks)
        .map(|_| {
            tick(&mut e);
            boss_at(&e).expect("the boss is on the field")
        })
        .collect()
}

fn hashes(stage: &Stage, ticks: u32) -> Vec<u64> {
    let mut e = engine(stage);
    (0..ticks)
        .map(|_| {
            tick(&mut e);
            e.state_hash()
        })
        .collect()
}

fn dist((x, y): (i32, i32), (px, py): (i32, i32)) -> i64 {
    let (dx, dy) = (i64::from(x - px), i64::from(y - py));
    (dx * dx + dy * dy).isqrt()
}

fn issues(stage: &Stage) -> Vec<Issue> {
    match validate(stage) {
        Err(Error::Invalid(i)) => i,
        other => panic!("expected issues, got {other:?}"),
    }
}

// --- The chase -------------------------------------------------------------------------------

#[test]
fn the_chase_walks_at_the_set_speed_and_stops() {
    let p = path(&stage_of(boss_json(chase(256, STOP)), 600), 300);
    for k in 1..=245 {
        assert_eq!(p[k - 1], (BOSS_X, BOSS_Y + 256 * k as i32), "tick {k}");
    }
    assert_eq!(p[245], (BOSS_X, 122_880), "the last step is the 160 left");
    assert!(p[245..].iter().all(|&at| at == (BOSS_X, 122_880)));
    assert_eq!(dist(p[299], (START_X, START_Y)), i64::from(STOP));
}

#[test]
fn an_empty_chase_block_is_the_placeholder() {
    let explicit = path(
        &stage_of(boss_json(chase(SPEED_PLACEHOLDER, STOP)), 600),
        300,
    );
    let empty = path(&stage_of(boss_json(json!({ "chase": {} })), 600), 300);
    assert_eq!(empty, explicit);
}

#[test]
fn without_a_chase_the_boss_stays_and_the_file_has_no_chase_key() {
    let stage = stage_of(boss_json(json!({})), 600);
    let p = path(&stage, 300);
    for k in [1, 100, 300] {
        assert_eq!(p[k - 1], (BOSS_X, BOSS_Y));
    }
    assert_eq!(hashes(&stage, 100), hashes(&stage, 100));
    let text = serde_json::to_string(&stage).unwrap();
    assert!(!text.contains("chase") && !text.contains("arrives_at_tick"));
}

#[test]
fn a_diagonal_chase_is_integer_and_deterministic() {
    let mut b = boss_json(chase(256, STOP));
    b["spawn"] = json!({ "x": BOSS_X - 40_000, "y": BOSS_Y });
    let stage = stage_of(b, 600);
    let p = path(&stage, 400);
    let mut last = (BOSS_X - 40_000, BOSS_Y);
    for &at in &p {
        assert!(dist(at, last) <= 256, "{last:?} -> {at:?}");
        last = at;
    }
    let d = dist(*p.last().unwrap(), (START_X, START_Y));
    assert!(d <= i64::from(STOP) && d > i64::from(STOP) - 256, "{d}");
    assert_eq!(hashes(&stage, 400), hashes(&stage, 400));
}

// --- The arrival -----------------------------------------------------------------------------

#[test]
fn a_boss_that_arrives_is_not_there_before_its_tick_and_the_stage_waits_for_it() {
    let stage = stage_of(boss_json(json!({ "arrives_at_tick": 50 })), 600);
    let mut e = engine(&stage);
    for t in 1..50 {
        tick(&mut e);
        assert_eq!(boss_at(&e), None, "tick {t}");
        assert_eq!(e.outcome(), Outcome::Running, "tick {t}");
    }
    tick(&mut e);
    assert_eq!(boss_at(&e), Some((BOSS_X, BOSS_Y)));
    assert_eq!(e.outcome(), Outcome::Running);
}

#[test]
fn a_boss_that_never_arrives_in_time_fails_the_run_at_its_length() {
    let stage = stage_of(boss_json(json!({ "arrives_at_tick": 590 })), 600);
    let mut e = engine(&stage);
    for _ in 0..600 {
        tick(&mut e);
    }
    assert_eq!(e.outcome(), Outcome::Failed);
}

#[test]
fn an_arrival_at_tick_one_is_the_boss_there_from_the_start() {
    let plain = stage_of(boss_json(json!({})), 600);
    let one = stage_of(boss_json(json!({ "arrives_at_tick": 1 })), 600);
    assert_eq!(hashes(&one, 120), hashes(&plain, 120));
}

#[test]
fn the_chase_starts_on_the_tick_the_boss_arrives() {
    let mut b = boss_json(chase(256, STOP));
    b["arrives_at_tick"] = json!(50);
    let mut e = engine(&stage_of(b, 600));
    for _ in 1..50 {
        tick(&mut e);
        assert_eq!(boss_at(&e), None);
    }
    tick(&mut e);
    // Its first tick is tick 50: it is built, then acts, and steps once.
    assert_eq!(boss_at(&e), Some((BOSS_X, BOSS_Y + 256)));
}

// --- Which mover wins ------------------------------------------------------------------------

fn move_to_phase() -> Value {
    json!({ "timeline": [{ "type": "move_to", "to": { "x": BOSS_X, "y": BOSS_Y - 20_000 },
                           "ticks": 40 }] })
}

#[test]
fn a_move_to_in_progress_wins_and_the_chase_resumes_after_it() {
    let mut with = boss_json(chase(256, STOP));
    with["phases"] = json!([move_to_phase()]);
    let mut without = boss_json(json!({}));
    without["phases"] = json!([move_to_phase()]);
    let a = path(&stage_of(with, 600), 60);
    let b = path(&stage_of(without, 600), 60);
    // Tick 1 is the chase's: the timeline starts the order after the boss has moved, so the
    // order is worked out from a place 256 further on. From tick 2 it alone moves the boss,
    // to the same point, and the chase resumes the tick after it lands.
    let target = (BOSS_X, BOSS_Y - 20_000);
    let landed = b.iter().position(|&at| at == target).unwrap();
    assert!(a[1..landed].iter().all(|&at| at.1 < a[0].1), "{a:?}");
    assert_eq!(a[landed], target);
    assert_eq!(a[landed + 1], (target.0, target.1 + 256));
}

#[test]
fn a_dash_moves_the_boss_by_the_dash_alone() {
    let attack = json!([{ "kind": "wind_up", "ticks": 10, "move_pct": 0 },
        { "kind": "active", "ticks": 20, "damage": 5, "move_pct": 0,
          "dash": { "distance": 200 * 256 } }]);
    let phase = json!({ "rules": [{ "when": always(), "target": "player", "once": true,
        "cooldown_ticks": 0, "do": { "type": "attack", "attack": { "inline": attack } } }] });
    let mut with = boss_json(chase(256, STOP));
    with["phases"] = json!([phase.clone()]);
    let mut without = boss_json(json!({}));
    without["phases"] = json!([phase]);
    let a = path(&stage_of(with, 600), 30);
    let b = path(&stage_of(without, 600), 30);
    // Tick 1 is the chase's, before the attack starts; the wind-up and the dash then move the
    // boss by the dash alone.
    for k in 0..30 {
        assert_eq!(a[k], (b[k].0, b[k].1 + 256), "tick {}", k + 1);
    }
}

#[test]
fn an_attack_phase_with_half_move_pct_halves_the_chase_step() {
    let attack = json!([{ "kind": "wind_up", "ticks": 30, "move_pct": 50 }]);
    let phase = json!({ "rules": [{ "when": always(), "target": "player", "once": true,
        "cooldown_ticks": 0, "do": { "type": "attack", "attack": { "inline": attack } } }] });
    let mut b = boss_json(chase(256, STOP));
    b["phases"] = json!([phase]);
    let p = path(&stage_of(b, 600), 20);
    // From tick 2 the wind-up holds `move_pct` 50.
    let steps: Vec<i32> = p.windows(2).map(|w| w[1].1 - w[0].1).collect();
    assert!(steps.iter().all(|&d| d == 128), "{steps:?}");
    assert_eq!(p[0].1 - BOSS_Y, 256);
}

// --- A party ---------------------------------------------------------------------------------

#[test]
fn a_party_boss_walks_toward_the_first_player_when_the_second_is_down_or_gone() {
    let mut e = engine(&stage_of(boss_json(chase(256, STOP)), 600));
    e.join(PlayerIdx::new(1).unwrap()).unwrap();
    e.leave(PlayerIdx::new(1).unwrap()).unwrap();
    for _ in 0..30 {
        tick(&mut e);
    }
    assert_eq!(boss_at(&e), Some((BOSS_X, BOSS_Y + 256 * 30)));
}

#[test]
fn with_two_players_up_each_step_points_at_one_of_them() {
    let mut b = boss_json(chase(256, STOP));
    b["spawn"] = json!({ "x": BOSS_X - 40_000, "y": BOSS_Y });
    let mut e = engine(&stage_of(b, 600));
    e.join(PlayerIdx::new(1).unwrap()).unwrap();
    let mut last = boss_at(&e).unwrap_or((BOSS_X - 40_000, BOSS_Y));
    for _ in 0..60 {
        let before: Vec<(i32, i32)> = e
            .snapshot()
            .players
            .iter()
            .map(|p| (p.at.x.0, p.at.y.0))
            .collect();
        tick(&mut e);
        let at = boss_at(&e).unwrap();
        let step = (i64::from(at.0 - last.0), i64::from(at.1 - last.1));
        assert!(step.0 != 0 || step.1 != 0);
        let toward = |p: (i32, i32)| {
            let to = (i64::from(p.0 - last.0), i64::from(p.1 - last.1));
            (step.0 * to.1 - step.1 * to.0).abs() <= to.0.abs().max(to.1.abs()) + 256
        };
        assert!(before.iter().any(|&p| toward(p)), "{step:?} {before:?}");
        last = at;
    }
}

// --- A copy of the run -----------------------------------------------------------------------

#[test]
fn a_copy_mid_chase_and_before_an_arrival_finishes_with_the_same_hashes() {
    for (boss, cut) in [
        (chase(256, STOP), 100),
        (json!({ "arrives_at_tick": 50 }), 20),
    ] {
        let stage = stage_of(boss_json(boss), 600);
        let whole = hashes(&stage, 160);
        let mut e = engine(&stage);
        for _ in 0..cut {
            tick(&mut e);
        }
        let mut copy = e.clone();
        let rest: Vec<u64> = (cut..160)
            .map(|_| {
                tick(&mut copy);
                copy.state_hash()
            })
            .collect();
        assert_eq!(rest, whole[cut as usize..]);
    }
}

// --- The validator ---------------------------------------------------------------------------

fn refused(boss: Value, version: u32, path: &str) {
    let mut stage = stage_of(boss_json(boss), 600);
    stage.sim_version = version;
    let found = issues(&stage);
    assert!(found.iter().any(|i| i.path == path), "{path}: {found:?}");
}

fn accepted(boss: Value) {
    validate(&stage_of(boss_json(boss), 600)).unwrap_or_else(|e| panic!("{e:?}"));
}

const V: u32 = stella_rain_core::version::SIM_VERSION;

#[test]
fn the_chase_and_the_arrival_are_checked() {
    refused(chase(0, STOP), V, "$.boss.chase.speed");
    refused(chase(4_097, STOP), V, "$.boss.chase.speed");
    refused(chase(256, 0), V, "$.boss.chase.stop_distance");
    refused(chase(256, 10_000_000), V, "$.boss.chase.stop_distance");
    refused(json!({ "arrives_at_tick": 0 }), V, "$.boss.arrives_at_tick");
    refused(
        json!({ "arrives_at_tick": 600 }),
        V,
        "$.boss.arrives_at_tick",
    );
    accepted(json!({ "arrives_at_tick": 1 }));
    accepted(json!({ "arrives_at_tick": 599 }));
    accepted(chase(4_096, 1));
}

#[test]
fn version_one_refuses_both_fields() {
    refused(chase(256, STOP), 1, "$.boss.chase");
    refused(
        json!({ "arrives_at_tick": 50 }),
        1,
        "$.boss.arrives_at_tick",
    );
}

#[test]
fn a_boss_that_outruns_the_player_is_refused_at_the_start() {
    let player_speed = SPEED;
    assert!(
        Engine::new_with(
            &stage_of(boss_json(chase(player_speed - 1, STOP)), 600),
            &design_content()
        )
        .is_ok()
    );
    assert!(
        Engine::new_with(
            &stage_of(boss_json(chase(player_speed, STOP)), 600),
            &design_content()
        )
        .is_err()
    );
}

#[test]
fn a_time_limit_boss_takes_neither_field() {
    use stella_rain_core::run_content::RunContent;
    use stella_rain_core::run_setup::RunSetup;
    use stella_rain_core::time_limit::TimeLimit;
    use stella_rain_core::version::Versions;
    for (field, value) in [("arrives_at_tick", json!(50)), ("chase", json!({}))] {
        let mut v: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
        v["bosses"][0][field] = value;
        let mode = TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();
        let setup = RunSetup {
            mode: "time_limit".into(),
            weapon: "giant_sword".into(),
            ..RunSetup::default()
        };
        let Err(Error::Invalid(found)) =
            Engine::start_time_limit(&setup, RunContent::bundled(), &mode)
        else {
            panic!("{field}: the run started");
        };
        let path = format!("$.bosses[0].{field}");
        assert!(found.iter().any(|i| i.path == path), "{found:?}");
    }
}

#[test]
fn killing_the_boss_that_arrived_clears_the_stage() {
    let mut b = boss_json(json!({ "arrives_at_tick": 50 }));
    b["hp"] = json!(1);
    b["spawn"] = json!({ "x": BOSS_X, "y": START_Y - 20_000 });
    let mut stage = stage_with(600, Some(b), vec![]);
    stage.sim_version = stella_rain_core::version::SIM_VERSION;
    let mut e = engine(&stage);
    fire_until(&mut e, idle(), 300, |e| e.outcome() != Outcome::Running);
    assert_eq!(e.outcome(), Outcome::Cleared);
    assert!(e.tick() > 50, "{}", e.tick());
}
