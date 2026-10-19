//! The `time_limit` mode file (core#212 step a): a run length, boss times, a mid-boss pool, a
//! final boss, an overtime limit, the Star-Hunter's extra time and the map, read from JSON as
//! untrusted bytes (ADR-020). Every time is in ticks (60 a second).

use serde_json::{Value, json};
use stella_rain_core::registry::{ENTRIES, Kind};
use stella_rain_core::run_replay::content_sha256;
use stella_rain_core::run_setup;
use stella_rain_core::time_limit::{
    MAX_BOSS_TIMES, MAX_EXTRA_TICKS, MAX_LENGTH_TICKS, MAX_OVERTIME_TICKS, MAX_POOL,
    MAX_TIME_LIMIT_BYTES, TimeLimit,
};
use stella_rain_core::validate::{Error, limits::MAX_MAP_SIDE};

const MINUTE: u32 = 3600;

fn good() -> Value {
    serde_json::from_slice(TimeLimit::BUNDLED).unwrap()
}

fn parse(v: &Value) -> Result<TimeLimit, Error> {
    TimeLimit::parse(v.to_string().as_bytes())
}

fn paths(v: &Value) -> Vec<String> {
    match parse(v) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

fn with(key: &str, value: Value) -> Value {
    let mut v = good();
    v[key] = value;
    v
}

#[test]
fn the_bundled_file_loads_with_the_defaults() {
    let m = TimeLimit::bundled();
    assert_eq!(m.length_ticks(), 20 * MINUTE);
    assert_eq!(
        m.boss_ticks(),
        [5 * MINUTE, 10 * MINUTE, 15 * MINUTE, 20 * MINUTE]
    );
    assert_eq!(m.mid_boss_pool().len(), 3);
    assert_eq!(m.final_boss().0, "golem");
    assert_eq!(m.overtime_ticks(), 5 * MINUTE);
    assert_eq!(m.star_hunter_extra_ticks(), MINUTE);
    assert_eq!((m.map().width.0, m.map().height.0), (1_048_576, 1_048_576));
    assert_eq!((m.map().start.x.0, m.map().start.y.0), (524_288, 524_288));
}

#[test]
fn the_mode_is_registered_and_its_file_is_part_of_the_content_hash() {
    assert!(
        ENTRIES
            .iter()
            .any(|e| e.kind == Kind::Mode && e.id == "time_limit")
    );
    assert_eq!(run_setup::mode_file("time_limit"), Some(TimeLimit::BUNDLED));
    assert_ne!(content_sha256("time_limit"), None);
    assert_ne!(content_sha256("time_limit"), content_sha256("trial"));
    // Not a stage: the run itself is a later step.
    assert_eq!(run_setup::mode_stage("time_limit"), None);
}

#[test]
fn a_file_over_the_size_limit_is_refused_before_parsing() {
    let big = vec![b' '; MAX_TIME_LIMIT_BYTES + 1];
    assert!(matches!(
        TimeLimit::parse(&big),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn unknown_fields_and_other_versions_are_refused() {
    let mut v = good();
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    assert!(matches!(
        parse(&with("sim_version", json!(1))),
        Err(Error::UnsupportedVersion { .. })
    ));
}

#[test]
fn boss_times_must_rise_strictly() {
    let up = |t: Vec<u32>| with("boss_ticks", json!(t));
    assert_eq!(paths(&up(vec![600, 600, 900, 72000])), ["$.boss_ticks[1]"]);
    assert_eq!(paths(&up(vec![900, 600, 700, 72000])), ["$.boss_ticks[1]"]);
}

#[test]
fn boss_times_fall_inside_the_length_and_the_last_is_the_length() {
    let t = |t: Vec<u32>| with("boss_ticks", json!(t));
    assert_eq!(paths(&t(vec![600, 900, 1200, 71999])), ["$.boss_ticks[3]"]);
    assert_eq!(
        paths(&t(vec![600, 900, 72001, 72000])),
        ["$.boss_ticks[2]", "$.boss_ticks[3]"]
    );
    assert_eq!(paths(&t(vec![0, 900, 1200, 72000])), ["$.boss_ticks[0]"]);
    assert_eq!(paths(&t(vec![])), ["$.boss_ticks"]);
}

#[test]
fn the_pool_holds_a_boss_for_each_mid_boss_time() {
    assert_eq!(
        paths(&with("mid_boss_pool", json!(["golem", "golem"]))),
        ["$.mid_boss_pool"]
    );
    assert!(
        parse(&with(
            "mid_boss_pool",
            json!(["golem", "golem", "golem", "golem"])
        ))
        .is_ok()
    );
}

#[test]
fn every_content_id_must_exist_as_an_asset() {
    assert_eq!(paths(&with("final_boss", json!("nope"))), ["$.final_boss"]);
    assert_eq!(
        paths(&with("mid_boss_pool", json!(["golem", "blade", "golem"]))),
        ["$.mid_boss_pool[1]"]
    );
    assert_eq!(
        paths(&with("final_boss", json!("Not Valid"))),
        ["$.final_boss"]
    );
}

#[test]
fn each_number_is_under_its_hard_cap() {
    assert_eq!(
        paths(&with("length_ticks", json!(MAX_LENGTH_TICKS + 1))),
        ["$.length_ticks", "$.boss_ticks[3]"]
    );
    assert_eq!(
        paths(&with("overtime_ticks", json!(MAX_OVERTIME_TICKS + 1))),
        ["$.overtime_ticks"]
    );
    assert_eq!(
        paths(&with("star_hunter_extra_ticks", json!(MAX_EXTRA_TICKS + 1))),
        ["$.star_hunter_extra_ticks"]
    );
    assert!(parse(&with("overtime_ticks", json!(MAX_OVERTIME_TICKS))).is_ok());
    assert!(parse(&with("star_hunter_extra_ticks", json!(MAX_EXTRA_TICKS))).is_ok());
    let times: Vec<u32> = (1..=MAX_BOSS_TIMES as u32 + 1).collect();
    let mut v = with("boss_ticks", json!(times));
    v["length_ticks"] = json!(MAX_BOSS_TIMES as u32 + 1);
    v["mid_boss_pool"] = json!(vec!["golem"; MAX_BOSS_TIMES]);
    assert_eq!(paths(&v), ["$.boss_ticks"]);
    let pool = vec!["golem"; MAX_POOL + 1];
    assert_eq!(
        paths(&with("mid_boss_pool", json!(pool))),
        ["$.mid_boss_pool"]
    );
}

#[test]
fn the_map_is_within_the_map_bounds_and_the_player_starts_on_it() {
    let map = |w: i32, h: i32, x: i32, y: i32| {
        with(
            "map",
            json!({ "width": w, "height": h, "start": { "x": x, "y": y } }),
        )
    };
    assert!(parse(&map(256, 256, 0, 256)).is_ok());
    assert_eq!(paths(&map(255, 1000, 0, 0)), ["$.map.width"]);
    assert_eq!(paths(&map(1000, MAX_MAP_SIDE + 1, 0, 0)), ["$.map.height"]);
    assert_eq!(paths(&map(1000, 1000, 1001, 0)), ["$.map.start.x"]);
    assert_eq!(paths(&map(1000, 1000, 0, -1)), ["$.map.start.y"]);
}
