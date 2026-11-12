//! The threat colour table (ADR-CONT-1 point 1, ADR-BRDG-4 point 9): an optional member of the
//! pack, an enemy field naming one entry, and an index on the snapshot's enemy attack and zone
//! views. The colour decides nothing in the simulation. The fixtures here are this file's own
//! numbers and colour names, never the pack's.

use crate::common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::pack::{ContentFiles, Pack, PackError};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::stage::Stage;
use stella_rain_core::threat_colours::{
    MAX_THREAT_COLOURS, MAX_THREAT_COLOURS_BYTES, ThreatColourIndex, ThreatColours,
};
use stella_rain_core::validate::{self, Error};
use stella_rain_core::version::Versions;

const PX: i32 = 256;

fn table_json() -> Value {
    json!({ "colours": [
        { "id": "boss_lane", "r": 200, "g": 40, "b": 40 },
        { "id": "elite_ring", "r": 255, "g": 160, "b": 0 },
    ] })
}

fn parse(v: &Value) -> Result<ThreatColours, Error> {
    ThreatColours::parse(v.to_string().as_bytes(), Versions::CURRENT)
}

fn issues(e: Error) -> Vec<(String, String)> {
    match e {
        Error::Invalid(list) => list.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not an Invalid: {other:?}"),
    }
}

// --- T1: the file -------------------------------------------------------------------------

#[test]
fn a_table_parses_and_names_its_entries_by_position() {
    let table = parse(&table_json()).unwrap();
    let ids: Vec<&str> = table.colours().iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["boss_lane", "elite_ring"]);
    assert_eq!(table.colours()[1].g, 160);
    assert_eq!(table.index_of("elite_ring"), Some(ThreatColourIndex(1)));
    assert_eq!(table.index_of("missing"), None);
    assert!(parse(&json!({ "colours": [] })).unwrap().is_empty());
}

#[test]
fn a_bad_table_is_refused_with_paths_and_no_text_from_the_file() {
    let dup = json!({ "colours": [
        { "id": "a", "r": 0, "g": 0, "b": 0 }, { "id": "a", "r": 1, "g": 1, "b": 1 } ] });
    assert_eq!(issues(parse(&dup).unwrap_err())[0].0, "$.colours[1].id");
    let upper = json!({ "colours": [{ "id": "Boss", "r": 0, "g": 0, "b": 0 }] });
    let found = issues(parse(&upper).unwrap_err());
    assert_eq!(found[0].0, "$.colours[0].id");
    assert!(!found[0].1.contains("Boss"));
    let many: Vec<Value> = (0..=MAX_THREAT_COLOURS)
        .map(|i| json!({ "id": format!("c{i}"), "r": 0, "g": 0, "b": 0 }))
        .collect();
    let found = issues(parse(&json!({ "colours": many })).unwrap_err());
    assert_eq!(found[0].0, "$.colours");
    assert!(found[0].1.contains("at most 16"), "{}", found[0].1);
    for bad in [
        json!({ "colours": [{ "id": "a", "r": 256, "g": 0, "b": 0 }] }),
        json!({ "colours": [], "extra": 1 }),
        json!({ "colours": [{ "id": "a", "r": 0, "g": 0, "b": 0, "alpha": 1 }] }),
    ] {
        assert!(matches!(parse(&bad), Err(Error::Parse { .. })), "{bad}");
    }
    let big = vec![b' '; MAX_THREAT_COLOURS_BYTES + 1];
    assert!(matches!(
        ThreatColours::parse(&big, Versions::CURRENT),
        Err(Error::TooLarge { .. })
    ));
}

// --- T2: the pack member ------------------------------------------------------------------

fn pack_value() -> Value {
    serde_json::from_slice(Pack::BUNDLED).unwrap()
}

fn load(v: &Value) -> Result<Pack, PackError> {
    Pack::from_bytes(&serde_json::to_vec(v).unwrap())
}

fn content_with(table: Value) -> RunContent {
    let mut v = pack_value();
    v["threat_colours"] = table;
    load(&v).expect("the pack loads").content().clone()
}

#[test]
fn the_bundled_pack_has_no_table_and_exports_its_own_bytes() {
    let pack = Pack::bundled();
    assert!(pack.threat_colours().is_empty());
    assert!(RunContent::bundled().threat_colours().is_empty());
    let files = ContentFiles::bundled();
    assert!(files.get("threat_colours").is_none());
    assert_eq!(files.export().unwrap(), Pack::BUNDLED);
}

#[test]
fn a_pack_with_a_table_loads_and_round_trips() {
    let mut files = ContentFiles::bundled().clone();
    let mut member = serde_json::to_vec_pretty(&table_json()).unwrap();
    member.push(b'\n');
    assert!(files.insert("threat_colours", member));
    let bytes = files.export().expect("the files export as a pack");
    let pack = Pack::from_bytes(&bytes).unwrap();
    assert_eq!(pack.content().threat_colours().len(), 2);
    assert_ne!(pack.sha256(), Pack::bundled().sha256());
    // Unpacked and exported again, the pack gives its own bytes back.
    let unpacked = ContentFiles::from_pack(&bytes).expect("export form");
    assert_eq!(
        unpacked.get("threat_colours"),
        files.get("threat_colours"),
        "the member comes out as it went in"
    );
    assert_eq!(unpacked.export().unwrap(), bytes);
}

#[test]
fn a_pack_table_over_its_budget_or_with_a_bad_entry_is_refused() {
    let mut v = pack_value();
    let many: Vec<Value> = (0..=MAX_THREAT_COLOURS)
        .map(|i| json!({ "id": format!("c{i}"), "r": 0, "g": 0, "b": 0 }))
        .collect();
    v["threat_colours"] = json!({ "colours": many });
    match load(&v) {
        Err(PackError::Budget { path, .. }) => assert_eq!(path, "$.threat_colours.colours"),
        other => panic!("{other:?}"),
    }
    v["threat_colours"] = json!({ "colours": [{ "id": "Boss", "r": 0, "g": 0, "b": 0 }] });
    match load(&v) {
        Err(PackError::File { file, .. }) => assert_eq!(file, "threat_colours"),
        other => panic!("{other:?}"),
    }
}

// --- the stage ----------------------------------------------------------------------------

fn circle(radius: i32) -> Value {
    json!({ "type": "circle", "radius": radius })
}

fn line() -> Value {
    json!({ "type": "line", "length": 60 * PX, "width": 8 * PX })
}

fn zone(shape: Value, warn_only: bool) -> Value {
    let mut z = json!({ "type": "zone", "shape": shape, "anchor": "target",
        "wind_up_ticks": 30, "active_ticks": 30 });
    if warn_only {
        z["warn_only"] = json!(true);
        z.as_object_mut().unwrap().remove("active_ticks");
    }
    z
}

/// An enemy that sends one attack of `zones` at the main character once, and names `colour`.
fn zoner(colour: Option<&str>, zones: Vec<Value>, self_destruct: bool) -> Value {
    let rules = json!([{ "when": always(), "target": "player", "once": true, "cooldown_ticks": 0,
        "self_destruct": self_destruct,
        "do": { "type": "attack", "attack": { "inline": [
            { "kind": "wind_up", "ticks": 30, "damage": 1, "spawn": zones }] } } }]);
    let mut e = enemy(1000, hold(START_X, START_Y - 100 * PX), rules);
    if let Some(name) = colour {
        e["threat_colour"] = json!(name);
    }
    e
}

fn stage_of(waves: Vec<Value>, boss: Option<Value>, sim_version: u32) -> Stage {
    let mut stage = quiet(stage_with(900, Some(boss.unwrap_or_else(far_boss)), waves));
    stage.sim_version = sim_version;
    stage.player.hp = 99;
    stage
}

fn waves_of(enemies: Vec<Value>) -> Vec<Value> {
    enemies
        .into_iter()
        .enumerate()
        .map(|(i, e)| {
            wave(
                1,
                1,
                0,
                e,
                START_X + 10 * PX * i32::try_from(i).unwrap(),
                START_Y - 100 * PX,
            )
        })
        .collect()
}

fn run_with(stage: &Stage, table: Value) -> Engine {
    Engine::new_with(stage, &content_with(table)).expect("the run starts")
}

fn zones(e: &Engine) -> Vec<Value> {
    serde_json::to_value(e.snapshot()).unwrap()["zones"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn step(e: &mut Engine, ticks: u32) {
    for _ in 0..ticks {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
}

fn parses(v: &Value) -> Stage {
    serde_json::from_value(v.clone()).expect("parses")
}

// --- T3: the enemy field and its stage rules ----------------------------------------------

#[test]
fn an_enemy_naming_a_colour_parses_validates_and_round_trips() {
    let stage = stage_of(
        waves_of(vec![zoner(Some("boss_lane"), vec![], false)]),
        None,
        2,
    );
    validate::validate(&stage).expect("valid at sim_version 2");
    let v = serde_json::to_value(&stage).unwrap();
    assert_eq!(v["waves"][0]["enemy"]["threat_colour"], json!("boss_lane"));
    assert_eq!(parses(&v), stage);
}

#[test]
fn a_boss_an_elite_and_a_summoned_agent_may_name_a_colour() {
    let mut boss = far_boss();
    boss["threat_colour"] = json!("boss_lane");
    let mut summoned = enemy(10, hold(START_X, START_Y - 50 * PX), json!([]));
    summoned["threat_colour"] = json!("elite_ring");
    let summoner = enemy(
        10,
        hold(START_X, START_Y - 80 * PX),
        json!([{ "when": always(), "target": "player", "once": true,
                 "do": { "type": "summon", "agent": summoned, "count": 1 } }]),
    );
    let stage = stage_of(waves_of(vec![summoner]), Some(boss), 2);
    validate::validate(&stage).expect("valid");
    let v = serde_json::to_value(&stage).unwrap();
    assert_eq!(v["boss"]["threat_colour"], json!("boss_lane"));
    assert_eq!(parses(&v), stage);
}

#[test]
fn a_colour_is_refused_below_sim_version_2_when_malformed_and_on_a_companion() {
    let named = |name: &str| waves_of(vec![zoner(Some(name), vec![], false)]);
    let old = stage_of(named("boss_lane"), None, 1);
    let found = match validate::validate(&old) {
        Err(Error::Invalid(list)) => list,
        other => panic!("{other:?}"),
    };
    assert_eq!(found[0].path, "$.waves[0].enemy.threat_colour");
    assert!(found[0].message.contains("needs sim_version 2"));
    let bad = stage_of(named("Boss"), None, 2);
    let found = match validate::validate(&bad) {
        Err(Error::Invalid(list)) => list,
        other => panic!("{other:?}"),
    };
    assert_eq!(found[0].path, "$.waves[0].enemy.threat_colour");
    assert!(!found[0].message.contains("Boss"));
    let mut companion = enemy(10, hold(START_X, START_Y), json!([]));
    companion["threat_colour"] = json!("boss_lane");
    let mut stage = stage_of(vec![], None, 2);
    stage.companions = vec![serde_json::from_value(companion).unwrap()];
    let found = match validate::validate(&stage) {
        Err(Error::Invalid(list)) => list,
        other => panic!("{other:?}"),
    };
    assert_eq!(found[0].path, "$.companions[0].threat_colour");
}

#[test]
fn a_stage_that_names_none_has_no_key() {
    let stage = stage_of(waves_of(vec![zoner(None, vec![], false)]), None, 2);
    let text = serde_json::to_string(&stage).unwrap();
    assert!(!text.contains("threat_colour"));
}

// --- T4: an unknown entry ----------------------------------------------------------------

fn refused(result: Result<Engine, Error>) -> (String, String) {
    match result {
        Err(Error::Invalid(mut list)) => {
            assert_eq!(list.len(), 1, "{list:?}");
            let i = list.remove(0);
            (i.path, i.message)
        }
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("the run started"),
    }
}

#[test]
fn an_enemy_naming_an_entry_the_table_lacks_is_refused_at_run_build() {
    let content = content_with(json!({ "colours": [
        { "id": "boss_lane", "r": 200, "g": 40, "b": 40 }] }));
    let stage = stage_of(
        waves_of(vec![zoner(Some("missing"), vec![], false)]),
        None,
        2,
    );
    let (path, message) = refused(Engine::new_with(&stage, &content));
    assert_eq!(path, "$.waves[0].enemy.threat_colour");
    assert!(!message.contains("missing"), "{message}");
    let mut boss = far_boss();
    boss["threat_colour"] = json!("missing");
    let stage = stage_of(vec![], Some(boss), 2);
    let (path, message) = refused(Engine::new_with(&stage, &content));
    assert_eq!(path, "$.boss.threat_colour");
    assert!(!message.contains("missing"));
    // The bundled content has no table: a standalone stage naming a colour needs a pack.
    let stage = stage_of(
        waves_of(vec![zoner(Some("boss_lane"), vec![], false)]),
        None,
        2,
    );
    let (path, _) = refused(Engine::new(&stage));
    assert_eq!(path, "$.waves[0].enemy.threat_colour");
}

#[test]
fn a_pack_whose_stage_or_time_limit_names_a_missing_entry_is_refused_at_load() {
    let named = |mut v: Value, at: &[&str]| {
        let mut cur = &mut v;
        for key in at {
            cur = if let Ok(i) = key.parse::<usize>() {
                &mut cur[i]
            } else {
                &mut cur[*key]
            };
        }
        cur["threat_colour"] = json!("boss_lane");
        v
    };
    let table = json!({ "colours": [{ "id": "boss_lane", "r": 1, "g": 2, "b": 3 }] });
    // The trial stage's first enemy.
    let mut v = pack_value();
    v["modes"]["trial"] = named(v["modes"]["trial"].clone(), &["waves", "0", "enemy"]);
    match load(&v) {
        Err(PackError::File { file, error }) => {
            assert_eq!(file, "trial");
            assert_eq!(issues(error)[0].0, "$.waves[0].enemy.threat_colour");
        }
        other => panic!("{other:?}"),
    }
    v["threat_colours"] = table.clone();
    assert!(load(&v).is_ok(), "with the entry the pack loads");
    // The time_limit file's wave enemy, elite type enemy and boss.
    for (at, path) in [
        (
            vec!["waves", "0", "enemy"],
            "$.waves[0].enemy.threat_colour",
        ),
        (
            vec!["elite_types", "0", "enemy"],
            "$.elite_types[0].enemy.threat_colour",
        ),
        (vec!["bosses", "0"], "$.bosses[0].threat_colour"),
    ] {
        let mut v = pack_value();
        v["modes"]["time_limit"] = named(v["modes"]["time_limit"].clone(), &at);
        match load(&v) {
            Err(PackError::File { file, error }) => {
                assert_eq!(file, "time_limit");
                assert_eq!(issues(error)[0].0, path);
            }
            other => panic!("{other:?}"),
        }
        v["threat_colours"] = table.clone();
        assert!(load(&v).is_ok(), "{path}: with the entry the pack loads");
    }
}

// --- T5: the snapshot --------------------------------------------------------------------

fn colour_of(v: &Value) -> Option<u64> {
    v.get("threat_colour")
        .map(|c| c.as_u64().expect("a number"))
}

fn attack_colours(e: &Engine) -> Vec<Option<u64>> {
    let snap = serde_json::to_value(e.snapshot()).unwrap();
    snap["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.get("attack"))
        .map(colour_of)
        .collect()
}

#[test]
fn an_enemy_s_areas_carry_its_colour_in_wind_up_and_active_and_every_shape() {
    let areas = vec![
        zone(circle(8 * PX), false),
        zone(line(), false),
        zone(line(), true),
    ];
    let table = json!({ "colours": [
        { "id": "a", "r": 1, "g": 1, "b": 1 }, { "id": "b", "r": 2, "g": 2, "b": 2 }] });
    let stage = stage_of(waves_of(vec![zoner(Some("b"), areas, false)]), None, 2);
    let mut e = run_with(&stage, table);
    let mut seen = [false; 2];
    let mut attack_seen = false;
    for _ in 0..80 {
        step(&mut e, 1);
        for z in zones(&e) {
            assert_eq!(colour_of(&z), Some(1), "{z}");
            seen[usize::from(z["phase"] == json!("active"))] = true;
        }
        attack_seen |= attack_colours(&e).contains(&Some(1));
    }
    assert_eq!(seen, [true, true], "wind-up and active were both seen");
    assert!(attack_seen, "the enemy's AttackView carries the index");
}

#[test]
fn an_enemy_that_names_none_has_no_key_and_neither_has_its_attack() {
    let table = json!({ "colours": [{ "id": "a", "r": 1, "g": 1, "b": 1 }] });
    let stage = stage_of(
        waves_of(vec![zoner(None, vec![zone(circle(8 * PX), false)], false)]),
        None,
        2,
    );
    let mut e = run_with(&stage, table);
    let mut any = false;
    for _ in 0..40 {
        step(&mut e, 1);
        for z in zones(&e) {
            any = true;
            assert!(z.get("threat_colour").is_none(), "{z}");
        }
        assert!(attack_colours(&e).iter().all(Option::is_none));
    }
    assert!(any);
}

#[test]
fn two_enemies_on_one_attack_keep_their_own_colour() {
    let table = json!({ "colours": [
        { "id": "a", "r": 1, "g": 1, "b": 1 }, { "id": "b", "r": 2, "g": 2, "b": 2 }] });
    let one = vec![zone(circle(8 * PX), false)];
    let stage = stage_of(
        waves_of(vec![
            zoner(Some("a"), one.clone(), false),
            zoner(Some("b"), one.clone(), false),
            zoner(None, one, false),
        ]),
        None,
        2,
    );
    let mut e = run_with(&stage, table);
    let mut colours = std::collections::BTreeSet::new();
    for _ in 0..40 {
        step(&mut e, 1);
        colours.extend(zones(&e).iter().map(colour_of));
    }
    assert_eq!(colours, [None, Some(0), Some(1)].into_iter().collect());
}

#[test]
fn the_boss_and_a_summoned_agent_carry_their_own_colours() {
    let table = json!({ "colours": [
        { "id": "a", "r": 1, "g": 1, "b": 1 }, { "id": "b", "r": 2, "g": 2, "b": 2 }] });
    let phase = json!([{ "kind": "wind_up", "ticks": 30, "damage": 1,
        "spawn": [zone(circle(8 * PX), false)] }]);
    let mut summoned = zoner(Some("b"), vec![zone(circle(9 * PX), false)], false);
    summoned["movement"] = hold(START_X, START_Y - 60 * PX);
    let summoner = enemy(
        1000,
        hold(START_X, START_Y - 120 * PX),
        json!([{ "when": always(), "target": "player", "once": true,
                 "do": { "type": "summon", "agent": summoned, "count": 1 } }]),
    );
    let mut boss = far_boss();
    boss["threat_colour"] = json!("a");
    boss["phases"] = json!([{ "rules": [{ "when": always(), "target": "player",
        "once": true, "do": { "type": "attack", "attack": { "inline": phase } } }] }]);
    let stage = stage_of(waves_of(vec![summoner]), Some(boss), 2);
    let mut e = run_with(&stage, table);
    let mut colours = std::collections::BTreeSet::new();
    for _ in 0..60 {
        step(&mut e, 1);
        colours.extend(zones(&e).iter().map(colour_of));
    }
    assert!(colours.contains(&Some(0)), "the boss's area: {colours:?}");
    assert!(
        colours.contains(&Some(1)),
        "the summoned agent's: {colours:?}"
    );
}

#[test]
fn a_zone_keeps_its_colour_after_its_placer_is_gone() {
    let table = json!({ "colours": [{ "id": "a", "r": 1, "g": 1, "b": 1 }] });
    let stage = stage_of(
        waves_of(vec![zoner(
            Some("a"),
            vec![zone(circle(8 * PX), false)],
            true,
        )]),
        None,
        2,
    );
    let mut e = run_with(&stage, table);
    let mut alone = 0;
    for _ in 0..60 {
        step(&mut e, 1);
        let snap = serde_json::to_value(e.snapshot()).unwrap();
        let enemies = snap["entities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["kind"] == json!("enemy"))
            .count();
        for z in zones(&e) {
            assert_eq!(colour_of(&z), Some(0), "{z}");
            if enemies == 0 {
                alone += 1;
            }
        }
    }
    assert!(alone > 0, "a zone outlived its placer");
}

#[test]
fn the_player_s_attacks_and_friendly_zones_carry_none() {
    let table = json!({ "colours": [{ "id": "a", "r": 1, "g": 1, "b": 1 }] });
    // The player's own weapon is a plain one: its attack views never carry a colour.
    let stage = stage_with(300, Some(far_boss()), vec![]);
    let mut stage = stage;
    stage.sim_version = 2;
    let mut e = run_with(&stage, table);
    let press = stella_rain_core::input::Input {
        main: true,
        ..Default::default()
    };
    for _ in 0..30 {
        e.step(&[press, Default::default(), Default::default()]);
        assert!(attack_colours(&e).iter().all(Option::is_none));
        assert!(zones(&e).iter().all(|z| colour_of(z).is_none()));
    }
}

#[test]
fn only_a_coloured_area_has_the_key_and_it_is_a_number() {
    let table = json!({ "colours": [
        { "id": "a", "r": 1, "g": 1, "b": 1 }, { "id": "b", "r": 2, "g": 2, "b": 2 }] });
    let stage = stage_of(
        waves_of(vec![zoner(
            Some("b"),
            vec![zone(circle(8 * PX), false)],
            false,
        )]),
        None,
        2,
    );
    let mut e = run_with(&stage, table);
    step(&mut e, 5);
    let z = zones(&e);
    assert_eq!(z[0]["threat_colour"], json!(1));
    assert!(!serde_json::to_string(&z[0]).unwrap().contains("null"));
}

// --- T6: dormant, and the colour decides nothing ------------------------------------------

#[test]
fn a_run_of_the_bundled_pack_carries_no_colour() {
    let mut e = Engine::start(&common_trial_setup()).expect("starts");
    assert!(e.threat_colours().is_empty());
    for _ in 0..600 {
        e.step(&[idle(), Default::default(), Default::default()]);
        let text = serde_json::to_string(&e.snapshot()).unwrap();
        assert!(!text.contains("threat_colour"));
    }
}

fn common_trial_setup() -> stella_rain_core::run_setup::RunSetup {
    crate::common::run::trial_setup()
}

#[test]
fn the_colour_never_changes_a_state_hash() {
    let area = vec![zone(circle(8 * PX), false), zone(line(), false)];
    let table = json!({ "colours": [
        { "id": "a", "r": 1, "g": 1, "b": 1 }, { "id": "b", "r": 2, "g": 2, "b": 2 }] });
    let plain = stage_of(waves_of(vec![zoner(None, area.clone(), false)]), None, 2);
    let named = stage_of(waves_of(vec![zoner(Some("b"), area, false)]), None, 2);
    let hashes = |stage: &Stage, table: Option<&Value>| {
        let mut e = match table {
            Some(t) => run_with(stage, t.clone()),
            None => Engine::new(stage).unwrap(),
        };
        (0..200)
            .map(|_| {
                step(&mut e, 1);
                e.state_hash()
            })
            .collect::<Vec<u64>>()
    };
    let none = hashes(&plain, None);
    let unnamed = hashes(&plain, Some(&table));
    let coloured = hashes(&named, Some(&table));
    assert_eq!(none, unnamed);
    assert_eq!(none, coloured);
}
