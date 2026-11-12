//! Flinch and poise, control repeats and vulnerable after a stun: the keys, their validators and
//! (below) the rules. Every number is worked out by hand from the placeholders of each test.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::pack::ContentFiles;
use stella_rain_core::rules::Rules;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error};
use stella_rain_core::version::Versions;

use crate::common::on_hit::*;
use crate::common::*;

/// A smash step of one circle, with `phases` as its attack.
fn smashing(phases: Value) -> Value {
    json!({ "inline": {
        "combo": { "smash": { "attack": { "inline": phases } } },
        "window_ticks": 30,
        "assist_range": 0,
    } })
}

/// A hit phase of the main character's: one active tick with a circle in front.
fn hit_phase(extra: Value) -> Value {
    let mut p = json!({ "kind": "active", "ticks": 1, "damage": 10,
        "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

/// A `sim_version` 2 stage with `waves`, a `boss` and the main character's weapon `weapon`.
fn stage_of(waves: Vec<Value>, boss: Value, weapon: Value) -> Stage {
    let mut stage = armed(stage_full(900, Some(boss), waves, vec![]), weapon);
    stage.sim_version = 2;
    stage
}

fn invalid(stage: &Stage) -> Vec<(String, String)> {
    match validate::validate(stage) {
        Err(Error::Invalid(list)) => list.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

fn poised(poise: u32) -> Value {
    let mut e = enemy(1000, hold(START_X, NEAR_Y), json!([]));
    e["poise"] = json!(poise);
    e
}

fn quiet_stage(waves: Vec<Value>, boss: Value) -> Stage {
    stage_of(waves, boss, quiet_weapon())
}

fn one(e: Value) -> Vec<Value> {
    vec![wave(1, 1, 0, e, START_X, NEAR_Y)]
}

#[test]
fn c6_a_poise_is_an_enemys_or_elites_within_its_bound() {
    for ok in [0, 1_000] {
        let stage = quiet_stage(one(poised(ok)), far_boss());
        validate::validate(&stage).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    let (path, _) = invalid(&quiet_stage(one(poised(1_001)), far_boss())).remove(0);
    assert_eq!(path, "$.waves[0].enemy.poise");
    let mut companion = companion(10, hold(START_X, START_Y), json!([]));
    companion["poise"] = json!(5);
    let mut stage = quiet_stage(vec![], far_boss());
    stage.companions = vec![serde_json::from_value(companion).unwrap()];
    assert_eq!(invalid(&stage)[0].0, "$.companions[0].poise");
    // The boss is not an agent: it has no `poise` key at all (a summon of its phases is a
    // regular enemy and may name one).
}

#[test]
fn c7_a_phases_poise_damage_is_a_hit_phase_of_the_main_characters() {
    let ok = smashing(json!([hit_phase(json!({ "poise_damage": 1_000 }))]));
    validate::validate(&stage_of(vec![], far_boss(), ok)).expect("valid");
    let paths = |weapon: Value| {
        invalid(&stage_of(vec![], far_boss(), weapon))
            .into_iter()
            .map(|(p, _)| p)
            .collect::<Vec<_>>()
    };
    let too_big = paths(smashing(json!([hit_phase(
        json!({ "poise_damage": 1_001 })
    )])));
    let wind_up = paths(smashing(json!([
        { "kind": "wind_up", "ticks": 3, "damage": 0, "poise_damage": 1 }
    ])));
    let no_box = paths(smashing(json!([
        { "kind": "active", "ticks": 1, "damage": 0, "poise_damage": 1 }
    ])));
    for found in [too_big, wind_up, no_box] {
        assert!(
            found.iter().any(|p| p.ends_with(".poise_damage")),
            "{found:?}"
        );
    }
    // An enemy's attack is refused.
    let rules = json!([{ "when": always(), "target": "player", "cooldown_ticks": 0,
        "do": { "type": "attack", "attack": { "inline": [
            hit_phase(json!({ "poise_damage": 1 }))] } } }]);
    let e = enemy(1000, hold(START_X, NEAR_Y), rules);
    let found = invalid(&quiet_stage(one(e), far_boss()));
    assert!(
        found.iter().any(|(p, _)| p.ends_with(".poise_damage")),
        "{found:?}"
    );
}

fn content_naming(patch: Value) -> RunContent {
    let mut v: Value = serde_json::from_slice(ContentFiles::bundled_file("rules")).unwrap();
    for (k, x) in patch.as_object().unwrap() {
        v[k] = x.clone();
    }
    let rules = Rules::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();
    design_content().with_rules(rules)
}

#[test]
fn c8_an_enemy_naming_a_poise_needs_rules_naming_a_flinch() {
    let stage = quiet_stage(one(poised(5)), far_boss());
    let refused = Engine::new_with(&stage, &content_naming(json!({})));
    let Err(Error::Invalid(list)) = refused else {
        panic!("the run started without a flinch in its rules");
    };
    assert_eq!(list[0].path, "$.waves[0].enemy.poise");
    Engine::new_with(&stage, &content_naming(json!({ "flinch_ticks": 20 })))
        .expect("loads with a flinch");
    // No poise, no flinch needed.
    let plain = quiet_stage(
        one(enemy(1000, hold(START_X, NEAR_Y), json!([]))),
        far_boss(),
    );
    Engine::new_with(&plain, &content_naming(json!({}))).expect("loads");
}
