//! The stagger gauge of elites and the boss: the keys, their validators and (below) the rules.
//! Every number is worked out by hand from the placeholders of each test.

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

/// A hit phase of the main character's: one active tick with a circle in front.
fn hit_phase(extra: Value) -> Value {
    let mut p = json!({ "kind": "active", "ticks": 1, "damage": 10,
        "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

/// A smash step of one circle, with `phases` as its attack.
fn smashing(phases: Value) -> Value {
    json!({ "inline": {
        "combo": { "smash": { "attack": { "inline": phases } } },
        "window_ticks": 30,
        "assist_range": 0,
    } })
}

/// A stage at `sim_version` 2 with `waves`, a `boss` and the main character's weapon `weapon`.
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

fn paths(stage: &Stage) -> Vec<String> {
    invalid(stage).into_iter().map(|(p, _)| p).collect()
}

/// An elite of 1,000 hit points with `extra` keys.
fn elite(extra: Value) -> Value {
    let mut e = enemy(1000, hold(START_X, NEAR_Y), json!([]));
    e["elite"] = json!({ "range": 256 });
    for (k, v) in extra.as_object().unwrap() {
        e[k] = v.clone();
    }
    e
}

fn one(e: Value) -> Vec<Value> {
    vec![wave(1, 1, 0, e, START_X, NEAR_Y)]
}

fn quiet_stage(waves: Vec<Value>, boss: Value) -> Stage {
    stage_of(waves, boss, quiet_weapon())
}

/// The boss of the far-boss fixture with `extra` keys.
fn boss_with(extra: Value) -> Value {
    let mut b = far_boss();
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

#[test]
fn v1_a_stagger_max_is_an_elites_within_its_bound() {
    for ok in [1, 10_000] {
        let stage = quiet_stage(one(elite(json!({ "stagger_max": ok }))), far_boss());
        validate::validate(&stage).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    for bad in [0, 10_001] {
        let stage = quiet_stage(one(elite(json!({ "stagger_max": bad }))), far_boss());
        assert_eq!(paths(&stage), ["$.waves[0].enemy.stagger_max"], "{bad}");
    }
    // A regular enemy (no `elite`) and a companion name none.
    let mut plain = enemy(1000, hold(START_X, NEAR_Y), json!([]));
    plain["stagger_max"] = json!(100);
    let stage = quiet_stage(one(plain), far_boss());
    assert_eq!(paths(&stage), ["$.waves[0].enemy.stagger_max"]);
    let mut companion = companion(10, hold(START_X, START_Y), json!([]));
    companion["stagger_max"] = json!(100);
    let mut stage = quiet_stage(vec![], far_boss());
    stage.companions = vec![serde_json::from_value(companion).unwrap()];
    assert_eq!(paths(&stage), ["$.companions[0].stagger_max"]);
    // At `sim_version` 1.
    let mut stage = quiet_stage(one(elite(json!({ "stagger_max": 100 }))), far_boss());
    stage.sim_version = 1;
    let found = invalid(&stage);
    assert_eq!(found[0].0, "$.waves[0].enemy.stagger_max");
    assert!(found[0].1.contains("needs sim_version 2"), "{found:?}");
}

#[test]
fn v2_an_agents_staggered_ticks_needs_a_stagger_max_and_its_bound() {
    for ok in [1, 600] {
        let e = elite(json!({ "stagger_max": 100, "staggered_ticks": ok }));
        validate::validate(&quiet_stage(one(e), far_boss()))
            .unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    for bad in [0, 601] {
        let e = elite(json!({ "stagger_max": 100, "staggered_ticks": bad }));
        assert_eq!(
            paths(&quiet_stage(one(e), far_boss())),
            ["$.waves[0].enemy.staggered_ticks"],
            "{bad}"
        );
    }
    let e = elite(json!({ "staggered_ticks": 20 }));
    assert_eq!(
        paths(&quiet_stage(one(e), far_boss())),
        ["$.waves[0].enemy.staggered_ticks"]
    );
}

#[test]
fn v3_the_boss_names_its_stagger_max_and_staggered_ticks_the_same_way() {
    for ok in [1, 10_000] {
        let boss = boss_with(json!({ "stagger_max": ok }));
        validate::validate(&quiet_stage(vec![], boss)).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    for bad in [0, 10_001] {
        let boss = boss_with(json!({ "stagger_max": bad }));
        assert_eq!(
            paths(&quiet_stage(vec![], boss)),
            ["$.boss.stagger_max"],
            "{bad}"
        );
    }
    for bad in [0, 601] {
        let boss = boss_with(json!({ "stagger_max": 100, "staggered_ticks": bad }));
        assert_eq!(
            paths(&quiet_stage(vec![], boss)),
            ["$.boss.staggered_ticks"],
            "{bad}"
        );
    }
    let boss = boss_with(json!({ "staggered_ticks": 20 }));
    assert_eq!(
        paths(&quiet_stage(vec![], boss)),
        ["$.boss.staggered_ticks"]
    );
    let mut stage = quiet_stage(vec![], boss_with(json!({ "stagger_max": 100 })));
    stage.sim_version = 1;
    let found = invalid(&stage);
    assert_eq!(found[0].0, "$.boss.stagger_max");
    assert!(found[0].1.contains("needs sim_version 2"), "{found:?}");
}

#[test]
fn v4_a_phases_stagger_is_a_hit_phase_of_the_main_characters() {
    let ok = smashing(json!([hit_phase(json!({ "stagger": 10_000 }))]));
    validate::validate(&stage_of(vec![], far_boss(), ok)).expect("valid");
    let found = |weapon: Value| paths(&stage_of(vec![], far_boss(), weapon));
    let too_big = found(smashing(json!([hit_phase(json!({ "stagger": 10_001 }))])));
    let wind_up = found(smashing(json!([
        { "kind": "wind_up", "ticks": 3, "damage": 0, "stagger": 1 }
    ])));
    let no_box = found(smashing(json!([
        { "kind": "active", "ticks": 1, "damage": 0, "stagger": 1 }
    ])));
    for list in [too_big, wind_up, no_box] {
        assert!(list.iter().any(|p| p.ends_with(".stagger")), "{list:?}");
    }
    // An enemy's attack is refused.
    let rules = json!([{ "when": always(), "target": "player", "cooldown_ticks": 0,
        "do": { "type": "attack", "attack": { "inline": [
            hit_phase(json!({ "stagger": 1 }))] } } }]);
    let e = enemy(1000, hold(START_X, NEAR_Y), rules);
    let list = paths(&quiet_stage(one(e), far_boss()));
    assert!(list.iter().any(|p| p.ends_with(".stagger")), "{list:?}");
    // At `sim_version` 1.
    let mut stage = stage_of(
        vec![],
        far_boss(),
        smashing(json!([hit_phase(json!({ "stagger": 5 }))])),
    );
    stage.sim_version = 1;
    assert!(paths(&stage).iter().any(|p| p.ends_with(".stagger")));
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
fn v5_a_gauge_needs_staggered_ticks_from_the_rules_or_the_agent() {
    let refused = |stage: &Stage, patch: Value, path: &str| {
        let Err(Error::Invalid(list)) = Engine::new_with(stage, &content_naming(patch)) else {
            panic!("the run started without staggered ticks");
        };
        assert_eq!(list[0].path, path);
    };
    let elite_stage = quiet_stage(one(elite(json!({ "stagger_max": 100 }))), far_boss());
    refused(&elite_stage, json!({}), "$.waves[0].enemy.stagger_max");
    Engine::new_with(
        &elite_stage,
        &content_naming(json!({ "staggered_ticks": 20 })),
    )
    .expect("the rules' ticks");
    let own = quiet_stage(
        one(elite(json!({ "stagger_max": 100, "staggered_ticks": 20 }))),
        far_boss(),
    );
    Engine::new_with(&own, &content_naming(json!({}))).expect("the agent's ticks");

    let boss_stage = quiet_stage(vec![], boss_with(json!({ "stagger_max": 100 })));
    refused(&boss_stage, json!({}), "$.boss.stagger_max");
    Engine::new_with(
        &boss_stage,
        &content_naming(json!({ "staggered_ticks": 20 })),
    )
    .expect("the rules' ticks");
    let own = quiet_stage(
        vec![],
        boss_with(json!({ "stagger_max": 100, "staggered_ticks": 20 })),
    );
    Engine::new_with(&own, &content_naming(json!({}))).expect("the boss's ticks");
}
