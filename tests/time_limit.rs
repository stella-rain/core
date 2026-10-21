//! The `time_limit` mode file (core#212 step a): a run length, boss times, a mid-boss pool, a
//! final boss, an overtime limit, the Star-Hunter's extra time and the map, read from JSON as
//! untrusted bytes (ADR-020). Every time is in ticks (60 a second).

use serde_json::{Value, json};
use stella_rain_core::presets;
use stella_rain_core::registry::{ENTRIES, Kind};
use stella_rain_core::run_replay::content_sha256;
use stella_rain_core::run_setup;
use stella_rain_core::time_limit::{
    MAX_BOSS_TIMES, MAX_ELITE_DEFINITIONS, MAX_ELITE_TIERS, MAX_ELITE_TYPES, MAX_ELITE_WEIGHT,
    MAX_EXTRA_TICKS, MAX_LENGTH_TICKS, MAX_OVERTIME_TICKS, MAX_POOL, MAX_SPAWN_COUNT,
    MAX_TIME_LIMIT_BYTES, MAX_TOTAL_SPAWNS, MAX_WAVES, TimeLimit,
};
use stella_rain_core::validate::{
    Error,
    limits::{MAX_ELITES_ALIVE, MAX_MAP_SIDE},
};

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
    assert_eq!(m.mid_boss_pool().len(), 4);
    assert_eq!(m.final_boss().0, "core_pair");
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
    // One pack, one hash for both modes (ADR-080); a mode that does not exist has none.
    assert_eq!(content_sha256("time_limit"), content_sha256("trial"));
    assert_eq!(content_sha256("giant_sword"), None);
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
fn the_no_call_times_are_unique_and_inside_the_run() {
    assert_eq!(
        paths(&with("no_call_ticks", json!([18000, 18000]))),
        ["$.no_call_ticks[1]"]
    );
    assert!(parse(&with("no_call_ticks", json!([18000, 36000]))).is_ok());
}

#[test]
fn each_boss_definition_is_unique_and_names_a_sprite() {
    let bosses = good()["bosses"].clone();
    let first = bosses[0].clone();
    assert_eq!(
        paths(&with("bosses", json!([first.clone(), first]))),
        ["$.bosses[1].base"]
    );
    let mut unknown = bosses[0].clone();
    unknown["base"] = json!("nope");
    assert_eq!(paths(&with("bosses", json!([unknown]))), ["$.bosses[0].base"]);
}

#[test]
fn every_content_id_must_exist_as_an_asset() {
    assert_eq!(paths(&with("final_boss", json!("nope"))), ["$.final_boss"]);
    assert_eq!(
        paths(&with(
            "mid_boss_pool",
            json!(["golem", "giant_sword", "golem"])
        )),
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
    v["elite_table"] = json!([tier(0, &[("elite_low", 1)])]);
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
fn the_bundled_schedule_is_within_its_caps_and_the_bats_spawn_a_hundred_a_minute() {
    let m = TimeLimit::bundled();
    let total: u32 = m.waves().iter().map(|w| u32::from(w.count)).sum();
    assert!(total <= MAX_TOTAL_SPAWNS && m.waves().len() <= MAX_WAVES);
    // The bats are the schedule's waves without an ID; the six roles have IDs.
    let a_minute: u32 = m
        .waves()
        .iter()
        .filter(|w| w.id.is_none())
        .map(|w| 3600 / w.every_ticks)
        .sum();
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

fn tier(start_tick: u32, weights: &[(&str, u32)]) -> Value {
    let weights: Vec<Value> = weights
        .iter()
        .map(|(elite, weight)| json!({ "elite": elite, "weight": weight }))
        .collect();
    json!({ "start_tick": start_tick, "weights": weights })
}

fn table(tiers: Vec<Value>) -> Value {
    with("elite_table", json!(tiers))
}

#[test]
fn the_bundled_elite_table_has_no_high_grade_before_minute_16() {
    let m = TimeLimit::bundled();
    assert_eq!(m.elite_cap(), 5);
    let tiers = m.elite_table();
    let starts: Vec<u32> = tiers.iter().map(|t| t.start_tick).collect();
    assert_eq!(starts, [0, 6 * MINUTE, 16 * MINUTE]);
    let names = |i: usize| -> Vec<&str> {
        tiers[i]
            .weights
            .iter()
            .map(|w| w.elite.0.as_str())
            .collect()
    };
    assert_eq!(names(0), ["elite_low", "elite_goblin", "elite_shrine"]);
    assert_eq!(
        names(1),
        [
            "elite_low",
            "elite_mid",
            "elite_goblin",
            "elite_shrine",
            "elite_leader",
            "elite_mimic"
        ]
    );
    assert_eq!(
        names(2),
        [
            "elite_low",
            "elite_mid",
            "elite_high",
            "elite_goblin",
            "elite_shrine",
            "elite_leader",
            "elite_mimic"
        ]
    );
}

#[test]
fn the_bundled_file_defines_the_three_grades_by_placeholder_numbers() {
    let all = TimeLimit::bundled().elite_types();
    let ids: Vec<&str> = all.iter().map(|t| t.id.0.as_str()).collect();
    assert_eq!(
        ids,
        [
            "elite_low",
            "elite_mid",
            "elite_high",
            "elite_goblin",
            "elite_shrine",
            "elite_leader",
            "elite_mimic"
        ]
    );
    // The goblin has its own test (`tests/elite_goblin.rs`).
    let types = &all[..3];
    let sprites: Vec<&str> = types.iter().map(|t| t.enemy.base.0.as_str()).collect();
    assert_eq!(sprites, ["bat", "fairy_a", "horns_2"]);
    assert!(types.iter().all(|t| t.enemy.elite.is_some()));
    let hp: Vec<u32> = types.iter().map(|t| t.enemy.hp).collect();
    assert!(
        hp[0] < hp[1] && hp[1] < hp[2],
        "health rises by grade: {hp:?}"
    );
    let damage = |i: usize| -> u32 {
        let v = serde_json::to_value(&types[i].enemy).unwrap();
        v["rules"][0]["do"]["attack"]["inline"][0]["damage"]
            .as_u64()
            .unwrap_or(1) as u32
    };
    // The high elite's attack is its preset (ADR-103): its damage is the slam's, not an inline.
    let slam = presets::definition("elite_slam").expect("the preset");
    let slam_damage = slam.iter().find_map(|phase| phase.damage).unwrap_or(0);
    assert!(damage(0) < damage(1) && damage(1) < slam_damage);
}

#[test]
fn a_valid_elite_table_parses() {
    let v = table(vec![
        tier(0, &[("elite_low", 1)]),
        tier(MINUTE, &[("elite_low", 3), ("elite_mid", MAX_ELITE_WEIGHT)]),
    ]);
    let m = parse(&v).unwrap();
    assert_eq!(m.elite_table().len(), 2);
    assert_eq!(m.elite_table()[1].weights[1].weight, MAX_ELITE_WEIGHT);
}

#[test]
fn elite_tiers_rise_strictly_and_the_first_starts_with_the_run() {
    let weights = [("elite_low", 1)];
    assert_eq!(
        paths(&table(vec![tier(0, &weights), tier(0, &weights)])),
        ["$.elite_table[1].start_tick"]
    );
    assert_eq!(
        paths(&table(vec![tier(MINUTE, &weights), tier(0, &weights)])),
        ["$.elite_table[0].start_tick", "$.elite_table[1].start_tick"]
    );
}

#[test]
fn an_elite_tier_starts_before_the_run_ends() {
    let weights = [("elite_low", 1)];
    let end = 20 * MINUTE;
    assert!(parse(&table(vec![tier(0, &weights), tier(end - 1, &weights)])).is_ok());
    assert_eq!(
        paths(&table(vec![tier(0, &weights), tier(end, &weights)])),
        ["$.elite_table[1].start_tick"]
    );
}

#[test]
fn an_elite_weight_is_above_zero_and_under_its_cap() {
    assert_eq!(
        paths(&table(vec![tier(0, &[("elite_low", 0)])])),
        ["$.elite_table[0].weights[0].weight"]
    );
    assert_eq!(
        paths(&table(vec![tier(
            0,
            &[("elite_low", MAX_ELITE_WEIGHT + 1)]
        )])),
        ["$.elite_table[0].weights[0].weight"]
    );
}

#[test]
fn an_elite_table_names_defined_types_once_per_tier() {
    assert_eq!(
        paths(&table(vec![tier(0, &[("elite_low", 1), ("nope", 1)])])),
        ["$.elite_table[0].weights[1].elite"]
    );
    // A sprite ID is not an elite type.
    assert_eq!(
        paths(&table(vec![tier(0, &[("bat", 1)])])),
        ["$.elite_table[0].weights[0].elite"]
    );
    assert_eq!(
        paths(&table(vec![tier(0, &[("elite_low", 1), ("elite_low", 2)])])),
        ["$.elite_table[0].weights[1].elite"]
    );
}

/// The bundled file with its elite types replaced.
fn types(list: Vec<Value>) -> Value {
    with("elite_types", json!(list))
}

/// The bundled file with its elite types replaced and a table that names only the first.
fn only(list: Vec<Value>) -> Value {
    let first = list.first().map_or("elite_low".to_string(), |t| {
        t["id"].as_str().unwrap_or("x").to_string()
    });
    let mut v = types(list);
    v["elite_table"] = json!([tier(0, &[(first.as_str(), 1)])]);
    v
}

fn low() -> Value {
    good()["elite_types"][0].clone()
}

/// `low` as a type of its own.
fn named(id: &str) -> Value {
    let mut t = low();
    t["id"] = json!(id);
    t
}

#[test]
fn an_elite_type_is_a_well_formed_unique_id_with_a_valid_elite_enemy() {
    let mut mid = good()["elite_types"][1].clone();
    mid["id"] = json!("elite_low");
    let both_low = only(vec![low(), mid]);
    assert_eq!(paths(&both_low), ["$.elite_types[1].id"]);

    let mut t = low();
    t["id"] = json!("Not Valid");
    assert_eq!(paths(&only(vec![t])), ["$.elite_types[0].id"]);

    let mut t = low();
    t["enemy"]["base"] = json!("nope");
    assert_eq!(paths(&only(vec![t])), ["$.elite_types[0].enemy.base"]);

    // A weapon preset is not a sprite.
    let mut t = low();
    t["enemy"]["base"] = json!("giant_sword");
    assert_eq!(paths(&only(vec![t])), ["$.elite_types[0].enemy.base"]);

    let mut t = low();
    t["enemy"]["hp"] = json!(0);
    assert_eq!(paths(&only(vec![t])), ["$.elite_types[0].enemy.hp"]);

    let mut t = low();
    t["enemy"]["rules"][0]["do"]["attack"]["preset"]["id"] = json!("nope");
    assert!(
        paths(&only(vec![t]))
            .iter()
            .all(|p| p.starts_with("$.elite_types[0].enemy.rules[0]"))
    );

    let mut t = low();
    t["xp"] = json!(u32::MAX);
    assert_eq!(paths(&only(vec![t])), ["$.elite_types[0].xp"]);

    // An elite type is an elite.
    let mut t = low();
    t["enemy"].as_object_mut().unwrap().remove("elite");
    assert_eq!(paths(&only(vec![t])), ["$.elite_types[0].enemy.elite"]);
}

#[test]
fn every_type_the_table_names_is_defined_and_every_type_is_named() {
    // The table names `elite_mid`, `elite_high`, `elite_goblin`, `elite_shrine`,
    // `elite_leader` and `elite_mimic`, which are no longer defined.
    assert_eq!(
        paths(&types(vec![low()])),
        [
            "$.elite_table[0].weights[1].elite",
            "$.elite_table[0].weights[2].elite",
            "$.elite_table[1].weights[1].elite",
            "$.elite_table[1].weights[2].elite",
            "$.elite_table[1].weights[3].elite",
            "$.elite_table[1].weights[4].elite",
            "$.elite_table[1].weights[5].elite",
            "$.elite_table[2].weights[1].elite",
            "$.elite_table[2].weights[2].elite",
            "$.elite_table[2].weights[3].elite",
            "$.elite_table[2].weights[4].elite",
            "$.elite_table[2].weights[5].elite",
            "$.elite_table[2].weights[6].elite",
        ]
    );
    // A type no tier names is fine: a later task's table may not reach it yet.
    let mut v = types(vec![low(), named("elite_unused")]);
    v["elite_table"] = json!([tier(0, &[("elite_low", 1)])]);
    assert!(parse(&v).is_ok());
}

#[test]
fn there_are_at_least_one_and_at_most_so_many_elite_types() {
    let mut v = types(vec![]);
    v["elite_table"] = json!([tier(0, &[("elite_low", 1)])]);
    let found = paths(&v);
    assert!(found.contains(&"$.elite_types".to_string()), "{found:?}");

    let ids: Vec<String> = (0..=MAX_ELITE_DEFINITIONS)
        .map(|i| format!("elite_t{i}"))
        .collect();
    let list = |n: usize| -> Vec<Value> { ids[..n].iter().map(|i| named(i)).collect() };
    let mut at_cap = types(list(MAX_ELITE_DEFINITIONS));
    at_cap["elite_table"] = json!([tier(0, &[("elite_t0", 1)])]);
    assert!(parse(&at_cap).is_ok());
    let mut past = types(list(MAX_ELITE_DEFINITIONS + 1));
    past["elite_table"] = json!([tier(0, &[("elite_t0", 1)])]);
    assert_eq!(paths(&past), ["$.elite_types"]);
}

#[test]
fn the_elite_table_has_at_least_one_tier_and_one_type_each_and_at_most_so_many() {
    assert_eq!(paths(&table(vec![])), ["$.elite_table"]);
    assert_eq!(
        paths(&table(vec![tier(0, &[])])),
        ["$.elite_table[0].weights"]
    );
    let at_cap: Vec<Value> = (0..MAX_ELITE_TIERS as u32)
        .map(|i| tier(i * 60, &[("elite_low", 1)]))
        .collect();
    assert!(parse(&table(at_cap)).is_ok());
    let too_many: Vec<Value> = (0..=MAX_ELITE_TIERS as u32)
        .map(|i| tier(i * 60, &[("elite_low", 1)]))
        .collect();
    assert_eq!(paths(&table(too_many)), ["$.elite_table"]);

    let ids: Vec<String> = (0..=MAX_ELITE_TYPES)
        .map(|i| format!("elite_t{i}"))
        .collect();
    let mut v = types(ids.iter().map(|i| named(i)).collect());
    let weights =
        |n: usize| -> Vec<(&str, u32)> { ids[..n].iter().map(|i| (i.as_str(), 1)).collect() };
    v["elite_table"] = json!([tier(0, &weights(MAX_ELITE_TYPES))]);
    assert!(parse(&v).is_ok());
    v["elite_table"] = json!([tier(0, &weights(MAX_ELITE_TYPES + 1))]);
    assert_eq!(paths(&v), ["$.elite_table[0].weights"]);
}

#[test]
fn the_elite_cap_is_from_one_to_the_most_elites_alive() {
    assert_eq!(paths(&with("elite_cap", json!(0))), ["$.elite_cap"]);
    assert_eq!(
        paths(&with("elite_cap", json!(MAX_ELITES_ALIVE + 1))),
        ["$.elite_cap"]
    );
    assert_eq!(
        parse(&with("elite_cap", json!(MAX_ELITES_ALIVE)))
            .unwrap()
            .elite_cap(),
        MAX_ELITES_ALIVE
    );
    assert_eq!(parse(&with("elite_cap", json!(1))).unwrap().elite_cap(), 1);
}

#[test]
fn the_elite_fields_are_strict_and_a_refusal_echoes_no_file_text() {
    let mut v = table(vec![tier(0, &[("elite_low", 1)])]);
    v["elite_table"][0]["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = good();
    v["elite_types"][0]["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = good();
    v.as_object_mut().unwrap().remove("elite_types");
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = good();
    v.as_object_mut().unwrap().remove("elite_cap");
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));

    let secret = "secret_text_from_the_file";
    let mut named_secret = low();
    named_secret["id"] = json!(secret);
    named_secret["enemy"]["base"] = json!(secret);
    for v in [
        table(vec![tier(0, &[(secret, 1)])]),
        types(vec![named_secret]),
    ] {
        let Err(Error::Invalid(issues)) = parse(&v) else {
            panic!("not refused");
        };
        assert!(!issues.is_empty());
        assert!(
            issues
                .iter()
                .all(|i| !i.path.contains(secret) && !i.message.contains(secret))
        );
    }
}
