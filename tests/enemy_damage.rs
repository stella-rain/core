//! An enemy's bullet does 100 shown hit points (10,000 stored) from the run stats file, bounded
//! by a code ceiling, and the expertise hit point amounts are on the same scale (DS6 items 4
//! and 5).

use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::expertise::Expertise;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::run_stats::{DEFAULT_ENEMY_ATTACK_DAMAGE, RunStats};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::limits::MAX_PLAYER_HP;

use crate::common::run::trial_setup;

fn stats_with(
    edit: impl FnOnce(&mut Value),
) -> Result<RunStats, stella_rain_core::validate::Error> {
    let mut v: Value = serde_json::from_slice(RunStats::BUNDLED).unwrap();
    edit(&mut v);
    RunStats::parse(v.to_string().as_bytes())
}

#[test]
fn the_bundled_file_names_100_shown() {
    assert_eq!(RunStats::bundled().enemy_attack_damage(), 10_000);
    assert_eq!(DEFAULT_ENEMY_ATTACK_DAMAGE, 10_000);
}

#[test]
fn an_absent_field_reads_as_the_default_and_the_bounds_hold() {
    let absent = stats_with(|v| {
        v.as_object_mut().unwrap().remove("enemy_attack_damage");
    })
    .unwrap();
    assert_eq!(absent.enemy_attack_damage(), 10_000);
    let with = |n: u32| stats_with(|v| v["enemy_attack_damage"] = json!(n));
    assert_eq!(with(1).unwrap().enemy_attack_damage(), 1);
    assert_eq!(
        with(MAX_PLAYER_HP).unwrap().enemy_attack_damage(),
        MAX_PLAYER_HP
    );
    assert!(with(0).is_err());
    assert!(with(MAX_PLAYER_HP + 1).is_err());
    assert_eq!(MAX_PLAYER_HP, 99_900);
}

fn shooter_mode() -> TimeLimit {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    v["waves"] = json!([{
        "at_tick": 1,
        "enemy": {
            "base": "bat", "hp": 9_000_000, "radius": 2048,
            "movement": { "type": "straight", "vx": 0, "vy": 0 },
            "rules": [{ "when": { "type": "always" }, "target": "nearest_ally",
                "do": { "type": "attack", "attack": { "preset": {
                    "id": "aimed_single", "args": [15] } } } }]
        },
        "spawn": { "x": 0, "y": 0 }, "count": 1, "every_ticks": 0, "around": 30_000
    }]);
    TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

fn setup() -> RunSetup {
    RunSetup {
        mode: "time_limit".into(),
        ..trial_setup()
    }
}

#[test]
fn an_enemy_bullet_takes_10_000_and_five_hits_end_a_trial_run_from_50_000() {
    let mode = shooter_mode();
    let mut e = Engine::start_time_limit(&setup(), RunContent::bundled(), &mode).unwrap();
    let start = e.snapshot().player.hp;
    assert_eq!(start, 50_000, "the trial's main character");
    let (mut last, mut hits) = (start, 0);
    for _ in 0..20_000 {
        e.step(Input::default());
        let hp = e.snapshot().player.hp;
        if hp < last {
            assert_eq!(last - hp, if hp == 0 { last } else { 10_000 }, "hit {hits}");
            hits += 1;
        }
        last = hp;
        if e.outcome() != Outcome::Running {
            break;
        }
    }
    assert_eq!(hits, 5);
    assert_eq!(e.outcome(), Outcome::Failed);
}

#[test]
fn every_expertise_hit_point_amount_is_on_the_scaled_scale() {
    let json: Value = serde_json::from_slice(Expertise::BUNDLED).unwrap();
    let mut found = 0;
    fn walk(v: &Value, found: &mut u32) {
        match v {
            Value::Object(m) => {
                if m.get("effect") == Some(&json!("max_hp")) {
                    assert!(m["amount"].as_u64().unwrap() >= 10_000, "{m:?}");
                    *found += 1;
                }
                m.values().for_each(|c| walk(c, found));
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, found)),
            _ => {}
        }
    }
    walk(&json, &mut found);
    assert_eq!(found, 5);
}
