//! The `time_limit` mode file (core#212 step a): a run length, boss times, a mid-boss pool, a
//! final boss, an overtime limit, the Star-Hunter's extra time and the map, read from JSON as
//! untrusted bytes (ADR-020). Every time is in ticks (60 a second).

use serde_json::{Value, json};
use stella_rain_core::registry::{ENTRIES, Kind};
use stella_rain_core::run_replay::content_sha256;
use stella_rain_core::run_setup;
use stella_rain_core::time_limit::{
    MAX_BOSS_TIMES, MAX_EXTRA_TICKS, MAX_LENGTH_TICKS, MAX_OVERTIME_TICKS, MAX_POOL,
    MAX_SPAWN_COUNT, MAX_TIME_LIMIT_BYTES, MAX_TOTAL_SPAWNS, MAX_WAVES, TimeLimit,
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
    // The mode file, then the trial stage whose placeholder content the run takes.
    let file = run_setup::mode_file("time_limit").unwrap();
    assert!(file.starts_with(TimeLimit::BUNDLED));
    assert!(file.ends_with(run_setup::mode_stage("trial").unwrap()));
    assert_ne!(content_sha256("time_limit"), None);
    assert_ne!(content_sha256("time_limit"), content_sha256("trial"));
    // Not a stage file: its run is built from the mode file.
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
    v["waves"] = json!([]);
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
        let mut v = with(
            "map",
            json!({ "width": w, "height": h, "start": { "x": x, "y": y } }),
        );
        v["waves"] = json!([]);
        v
    };
    assert!(parse(&map(256, 256, 0, 256)).is_ok());
    assert_eq!(paths(&map(255, 1000, 0, 0)), ["$.map.width"]);
    assert_eq!(paths(&map(1000, MAX_MAP_SIDE + 1, 0, 0)), ["$.map.height"]);
    assert_eq!(paths(&map(1000, 1000, 1001, 0)), ["$.map.start.x"]);
    assert_eq!(paths(&map(1000, 1000, 0, -1)), ["$.map.start.y"]);
}

// --- The schedule (core#212 step b) ------------------------------------------------------------

fn wave() -> Value {
    good()["waves"][0].clone()
}

fn waves(list: Vec<Value>) -> Value {
    with("waves", json!(list))
}

#[test]
fn the_bundled_schedule_spawns_about_a_hundred_a_minute_and_is_within_its_caps() {
    let m = TimeLimit::bundled();
    let total: u32 = m.waves().iter().map(|w| u32::from(w.count)).sum();
    assert!(total <= MAX_TOTAL_SPAWNS && m.waves().len() <= MAX_WAVES);
    let a_minute: u32 = m.waves().iter().map(|w| 3600 / w.every_ticks).sum();
    assert_eq!(a_minute, 100);
    assert!(m.waves().iter().all(|w| w.enemy.elite.is_none()));
    // The last spawn comes before the run ends.
    for w in m.waves() {
        let last = w.at_tick + u32::from(w.count - 1) * w.every_ticks;
        assert!(last < m.length_ticks(), "{last}");
    }
}

#[test]
fn a_schedule_may_be_empty_but_is_part_of_the_file() {
    assert!(parse(&waves(vec![])).is_ok());
    let mut v = good();
    v.as_object_mut().unwrap().remove("waves");
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
}

#[test]
fn a_wave_is_checked_as_a_stage_s_is_with_the_mode_s_caps() {
    let mut w = wave();
    w["count"] = json!(MAX_SPAWN_COUNT);
    assert!(parse(&waves(vec![w.clone()])).is_ok());
    w["count"] = json!(MAX_SPAWN_COUNT + 1);
    assert_eq!(paths(&waves(vec![w])), ["$.waves[0].count"]);
    let mut w = wave();
    w["at_tick"] = json!(20 * MINUTE);
    assert_eq!(paths(&waves(vec![w])), ["$.waves[0].at_tick"]);
    let mut w = wave();
    w["enemy"]["hp"] = json!(0);
    assert_eq!(paths(&waves(vec![w])), ["$.waves[0].enemy.hp"]);
    let mut w = wave();
    w["enemy"]["base"] = json!("no_such_sprite");
    assert_eq!(paths(&waves(vec![w])), ["$.waves[0].enemy.base"]);
    let mut w = wave();
    w["around"] = json!(0);
    assert_eq!(paths(&waves(vec![w])), ["$.waves[0].around"]);
    let mut w = wave();
    w["spawn"] = json!({ "x": -1_000_000, "y": 0 });
    assert_eq!(paths(&waves(vec![w])), ["$.waves[0].spawn.x"]);
    let mut w = wave();
    w["surprise"] = json!(1);
    assert!(matches!(parse(&waves(vec![w])), Err(Error::Parse { .. })));
}

#[test]
fn the_schedule_has_at_most_so_many_waves_and_so_many_enemies_in_all() {
    let mut w = wave();
    w["count"] = json!(1);
    w["every_ticks"] = json!(0);
    assert!(parse(&waves(vec![w.clone(); MAX_WAVES])).is_ok());
    assert_eq!(
        paths(&waves(vec![w; MAX_WAVES + 1])),
        ["$.waves"],
        "one wave too many"
    );
    let mut w = wave();
    w["count"] = json!(MAX_SPAWN_COUNT);
    let fits = (MAX_TOTAL_SPAWNS / u32::from(MAX_SPAWN_COUNT)) as usize;
    assert!(parse(&waves(vec![w.clone(); fits])).is_ok());
    assert_eq!(paths(&waves(vec![w; fits + 1])), ["$.waves"]);
}

#[test]
fn the_schedule_has_no_elites_yet() {
    let mut w = wave();
    w["enemy"]["elite"] = json!({ "range": 40960 });
    assert_eq!(paths(&waves(vec![w])), ["$.waves[0].enemy.elite"]);
}
