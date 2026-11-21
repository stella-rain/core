//! The Wave 3 content items outside a style that are only typed here (ADR-SIM-9 points 3 and 4,
//! ADR-SIM-5, ADR-SVC-1): the base tree's `size_stacks` and `press_factor`, the attack phase's
//! `linger` node and the rules' `sweep_min_enemies`. Each is bounded by its validator and refused
//! by the pack check as unshipped until its rule lands. Test code only: the numbers are test
//! values, not content.

use serde_json::{Value, json};
use stella_rain_core::attack::AttackPhase;
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::registry;
use stella_rain_core::rules::Rules;
use stella_rain_core::validate::{Error, Issue, validate_attack};
use stella_rain_core::version::Versions;
use stella_rain_core::weapons::Weapons;

const TREE: &str = "$.weapons[0].trees[0].weapon";

fn pack() -> Value {
    serde_json::from_slice(Pack::BUNDLED).unwrap()
}

fn weapons_file() -> Value {
    pack()["weapons"].clone()
}

fn tree(v: &mut Value) -> &mut Value {
    &mut v["weapons"][0]["trees"][0]["weapon"]
}

fn parse_weapons(v: &Value, sim_version: u32) -> Result<Weapons, Error> {
    let versions = Versions {
        sim_version,
        ..Versions::CURRENT
    };
    Weapons::parse(v.to_string().as_bytes(), versions)
}

fn weapon_issues(v: &Value, sim_version: u32) -> Vec<(String, String)> {
    match parse_weapons(v, sim_version) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        Ok(_) => Vec::new(),
        Err(e) => panic!("expected a refusal or a parse, got {e:?}"),
    }
}

fn weapons_with(field: &str, value: Value) -> Value {
    let mut v = weapons_file();
    tree(&mut v)[field] = value;
    v
}

fn stacks(pct: u32, max: u32, ticks: u32) -> Value {
    json!({ "pct_per_stack": pct, "max_stacks": max, "ticks": ticks })
}

fn refused_at(v: &Value, path: &str) {
    let issues = weapon_issues(v, 2);
    assert!(issues.iter().any(|(p, _)| p == path), "{path}: {issues:?}");
}

/// F1: both base fields round-trip; a tree naming neither writes neither key.
#[test]
fn the_base_fields_round_trip() {
    let mut v = weapons_file();
    tree(&mut v)["size_stacks"] = stacks(20, 3, 300);
    tree(&mut v)["press_factor"] = json!({ "pct_per_press": 25 });
    let parsed = parse_weapons(&v, 2).expect("the weapons parse");
    let out: Value = serde_json::to_value(&parsed.list()[0].trees[0].weapon).unwrap();
    assert_eq!(out["size_stacks"], stacks(20, 3, 300));
    assert_eq!(out["press_factor"], json!({ "pct_per_press": 25 }));
    let plain = serde_json::to_value(&Weapons::bundled().list()[0].trees[0].weapon).unwrap();
    assert!(plain.get("size_stacks").is_none() && plain.get("press_factor").is_none());
}

/// F2: each bound of the two fields, one pair per bound; the stacks' product.
#[test]
fn the_base_fields_are_bounded() {
    let at = format!("{TREE}.size_stacks");
    for (key, bad, good) in [
        ("pct_per_stack", [0, 101], 100),
        ("max_stacks", [0, 9], 8),
        ("ticks", [0, 1801], 1800),
    ] {
        for b in bad {
            let mut s = stacks(10, 3, 300);
            s[key] = json!(b);
            refused_at(&weapons_with("size_stacks", s), &format!("{at}.{key}"));
        }
        let mut s = stacks(10, 3, 300);
        s[key] = json!(good);
        // The product of the other two keeps within 100 for the top of `pct_per_stack` only
        // with one stack.
        if key == "pct_per_stack" {
            s["max_stacks"] = json!(1);
        }
        assert!(
            weapon_issues(&weapons_with("size_stacks", s), 2)
                .iter()
                .all(|(p, _)| !p.starts_with(&at)),
            "{key}"
        );
    }
    let over = weapon_issues(&weapons_with("size_stacks", stacks(40, 3, 300)), 2);
    assert!(
        over.iter().any(|(p, m)| p == &format!("{at}.pct_per_stack")
            && m.contains("times max_stacks, at most 100")),
        "{over:?}"
    );
    assert!(
        weapon_issues(&weapons_with("size_stacks", stacks(33, 3, 300)), 2).is_empty(),
        "33 times 3 fits"
    );
    for bad in [0, 1001] {
        refused_at(
            &weapons_with("press_factor", json!({ "pct_per_press": bad })),
            &format!("{TREE}.press_factor.pct_per_press"),
        );
    }
    assert!(
        weapon_issues(
            &weapons_with("press_factor", json!({ "pct_per_press": 1000 })),
            2
        )
        .is_empty()
    );
    for (field, value) in [
        (
            "size_stacks",
            json!({ "pct_per_stack": 20, "max_stacks": 3, "ticks": 300, "x": 1 }),
        ),
        ("press_factor", json!({ "pct_per_press": 25, "x": 1 })),
    ] {
        assert!(
            matches!(
                parse_weapons(&weapons_with(field, value), 2),
                Err(Error::Parse { .. })
            ),
            "{field}"
        );
    }
}

fn circle(px: i32) -> Value {
    json!({ "type": "circle", "radius": px * 256 })
}

fn node() -> Value {
    json!({ "anchor": "place", "ticks": 180, "rate_ticks": 15, "shape": circle(80),
        "damage_pct": 30, "pull": 8 * 256, "cuts_orbs": true, "proc_pct": 30,
        "opens_with_runes": true })
}

fn node_with(key: &str, value: Value) -> Value {
    let mut n = node();
    n[key] = value;
    n
}

/// The weapons file with `linger` on the first tree's AA step's active phase (inline).
fn weapons_with_node(node: Value) -> Value {
    let mut v = weapons_file();
    tree(&mut v)["combo"]["main"]["attack"]["inline"][1]["linger"] = node;
    v
}

const NODE_AT: &str = "combo.main.attack.inline[1].linger";

fn node_issues(n: Value) -> Vec<(String, String)> {
    weapon_issues(&weapons_with_node(n), 2)
}

fn node_refused(n: Value, key: &str) {
    let issues = node_issues(n);
    let want = format!("{TREE}.{NODE_AT}{key}");
    assert!(issues.iter().any(|(p, _)| p == &want), "{want}: {issues:?}");
}

/// L1: the node round-trips; `anchor` `player` parses; a phase without it writes no key.
#[test]
fn the_node_round_trips() {
    for anchor in ["place", "player"] {
        let n = node_with("anchor", json!(anchor));
        let parsed = parse_weapons(&weapons_with_node(n.clone()), 2).expect("the node parses");
        let out = serde_json::to_value(&parsed.list()[0].trees[0].weapon).unwrap();
        assert_eq!(out["combo"]["main"]["attack"]["inline"][1]["linger"], n);
    }
    let plain = serde_json::to_value(&Weapons::bundled().list()[0].trees[0].weapon).unwrap();
    assert!(
        plain["combo"]["main"]["attack"]["inline"][1]
            .get("linger")
            .is_none()
    );
}

/// L2: each bound of the node, one pair per bound.
#[test]
fn the_node_is_bounded() {
    let max_pull = 420 * 256;
    for (key, bad, good) in [
        ("ticks", vec![0, 1801], 1800),
        ("rate_ticks", vec![0, 181], 180),
        ("damage_pct", vec![0, 1001], 1000),
        ("pull", vec![0, 255, max_pull + 1], max_pull),
        ("proc_pct", vec![0, 101], 100),
    ] {
        for b in bad {
            node_refused(node_with(key, json!(b)), &format!(".{key}"));
        }
        assert!(
            node_issues(node_with(key, json!(good))).is_empty(),
            "{key} at its top"
        );
    }
    node_refused(node_with("shape", circle(0)), ".shape.radius");
    for (key, value) in [("anchor", json!("sky")), ("extra", json!(1))] {
        assert!(
            matches!(
                parse_weapons(&weapons_with_node(node_with(key, value)), 2),
                Err(Error::Parse { .. })
            ),
            "{key}"
        );
    }
}

/// L3: where the node may stand.
#[test]
fn the_node_stands_on_a_players_active_phase_at_version_2() {
    // A wind-up phase.
    let mut v = weapons_file();
    tree(&mut v)["combo"]["main"]["attack"]["inline"][0]["linger"] = node();
    let issues = weapon_issues(&v, 2);
    assert!(
        issues
            .iter()
            .any(|(_, m)| m.contains("only an active phase opens a lingering area")),
        "{issues:?}"
    );
    // A recovery phase.
    let mut v = weapons_file();
    tree(&mut v)["combo"]["main"]["attack"]["inline"][2]["linger"] = node();
    assert!(
        weapon_issues(&v, 2)
            .iter()
            .any(|(_, m)| m.contains("only an active phase opens a lingering area"))
    );
    // An enemy's or the boss's attack (the validator reads it as a foe's).
    let phases: Vec<AttackPhase> = serde_json::from_value(json!([
        { "kind": "active", "ticks": 3, "type": "strike", "hit_box":
            { "type": "circle", "offset": { "forward": 0, "right": 0 }, "radius": 8192 },
          "linger": node() }
    ]))
    .expect("the phase parses");
    let found: Vec<Issue> = validate_attack(&phases, 0, registry::ENTRIES).unwrap_err();
    assert!(
        found.iter().any(|i| i
            .message
            .contains("a main character's or an ally's attack only")),
        "{found:?}"
    );
    // sim_version 1.
    assert!(
        weapon_issues(&weapons_with_node(node()), 1)
            .iter()
            .any(|(p, m)| p.ends_with("linger") && m.contains("needs sim_version 2")),
    );
}

/// A pack's refusals of the weapons file (`file: "weapons"`) or the rules file, as
/// `(path, message)`.
fn pack_issues(v: &Value, file: &str) -> Vec<(String, String)> {
    match Pack::from_bytes(&serde_json::to_vec(v).unwrap()) {
        Ok(_) => Vec::new(),
        Err(PackError::File {
            file: f,
            error: Error::Invalid(issues),
        }) if f == file => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        Err(e) => panic!("expected a {file} refusal or a load, got {e:?}"),
    }
}

fn gated(v: &Value, file: &str, path: &str, field: &str) {
    let issues = pack_issues(v, file);
    assert!(
        issues
            .iter()
            .any(|(p, m)| p == path && m.contains(&format!("the field {field} is not shipped yet"))),
        "{path}: {issues:?}"
    );
}

/// L4: the pack check refuses `linger` wherever an attack of a base tree stands: a step's, a
/// charge level's, a hold's, a style's full-gauge attack.
#[test]
fn the_pack_refuses_a_node_on_any_attack() {
    let phase = |i: usize| format!("attack.inline[{i}].linger");
    // A step's attack.
    let mut v = pack();
    v["weapons"]["weapons"][0]["trees"][0]["weapon"]["combo"]["main"]["attack"]["inline"][1]["linger"] =
        node();
    gated(
        &v,
        "weapons",
        &format!("{TREE}.combo.main.{}", phase(1)),
        "linger",
    );
    // A charge level's attack.
    let active = |linger: bool| {
        let mut p = json!({ "kind": "active", "ticks": 3, "type": "strike",
            "hit_box": { "type": "circle", "offset": { "forward": 0, "right": 0 },
                "radius": 8192 } });
        if linger {
            p["linger"] = node();
        }
        p
    };
    let mut v = pack();
    v["weapons"]["weapons"][0]["trees"][0]["weapon"]["combo"]["main"]["charge"] = json!({
        "levels": [{ "at_ticks": 10, "attack": { "inline": [active(true)] } }],
        "max_ticks": 40 });
    assert!(
        pack_issues(&v, "weapons")
            .iter()
            .any(|(_, m)| m.contains("the field linger is not shipped yet")),
        "a charge level's attack"
    );
    // A hold's loop attack.
    let mut v = pack();
    v["weapons"]["weapons"][0]["trees"][0]["weapon"]["combo"]["smash"]["hold"] = json!({
        "loop": { "inline": [active(true)] },
        "release": { "inline": [active(false)] }, "max_ticks": 60 });
    assert!(
        pack_issues(&v, "weapons")
            .iter()
            .any(|(_, m)| m.contains("the field linger is not shipped yet")),
        "a hold's loop"
    );
    // A style's full-gauge attack.
    let mut v = pack();
    let hammer = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .unwrap();
    let kinds = hammer["styles"][0]["kinds"].as_array_mut().unwrap();
    let gauge = kinds.iter_mut().find(|k| k["type"] == "gauge").unwrap();
    gauge["spend"]["attack"]["inline"][1]["linger"] = node();
    assert!(
        pack_issues(&v, "weapons")
            .iter()
            .any(|(_, m)| m.contains("the field linger is not shipped yet")),
        "a style's gauge attack"
    );
    assert!(Pack::from_bytes(Pack::BUNDLED).is_ok());
}

fn rules_file() -> Value {
    crate::common::dormant_rules_value()
}

fn parse_rules(v: &Value, sim_version: u32) -> Result<Rules, Error> {
    let versions = Versions {
        sim_version,
        ..Versions::CURRENT
    };
    Rules::parse(v.to_string().as_bytes(), versions)
}

fn rule_paths(r: Result<Rules, Error>) -> Vec<(String, String)> {
    match r {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        Ok(_) => Vec::new(),
        Err(e) => panic!("{e:?}"),
    }
}

/// R1: the key's bounds, its version and its accessor.
#[test]
fn the_sweep_key_is_bounded() {
    let with = |n: u64| {
        let mut v = rules_file();
        v["sweep_min_enemies"] = json!(n);
        v
    };
    parse_rules(&with(5), 2).expect("5 loads");
    for bad in [0, 501] {
        let issues = rule_paths(parse_rules(&with(bad), 2));
        assert!(
            issues.iter().any(|(p, _)| p == "$.sweep_min_enemies"),
            "{bad}: {issues:?}"
        );
    }
    assert!(parse_rules(&with(500), 2).is_ok());
    assert!(parse_rules(&with(1), 2).is_ok());
    let old = rule_paths(parse_rules(&with(5), 1));
    assert!(
        old.iter()
            .any(|(p, m)| p == "$.sweep_min_enemies" && m.contains("needs sim_version 2")),
        "{old:?}"
    );
}

/// E2: the pack check refuses the base fields and the key as unshipped.
#[test]
fn the_pack_refuses_the_fields_and_the_key() {
    let mut v = pack();
    tree(&mut v["weapons"])["size_stacks"] = stacks(20, 3, 300);
    gated(&v, "weapons", &format!("{TREE}.size_stacks"), "size_stacks");
    let mut v = pack();
    tree(&mut v["weapons"])["press_factor"] = json!({ "pct_per_press": 25 });
    gated(
        &v,
        "weapons",
        &format!("{TREE}.press_factor"),
        "press_factor",
    );
    let mut v = pack();
    v["rules"]["sweep_min_enemies"] = json!(5);
    gated(&v, "rules", "$.sweep_min_enemies", "sweep_min_enemies");
    assert!(Pack::from_bytes(Pack::BUNDLED).is_ok());
}

/// F3: `repeat_size_stack` finds the field on the base tree: a pack whose tree names it has no
/// "no size-stack field" issue, the same style on a tree without it has.
#[test]
fn repeat_size_stack_finds_the_base_field() {
    let word = "the weapon's base tree has no size-stack field";
    let style = |v: &mut Value| {
        let hammer = v["expertise"]["weapons"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|w| w["weapon"] == "giant_hammer")
            .unwrap();
        hammer["styles"][0]["kinds"] = json!([
            { "type": "repeat", "step": ["main"], "after_ticks": 10, "repeat_pct": 50,
              "anchor": "hit_list" },
            { "type": "repeat_size_stack" }
        ]);
        for field in ["upgrade_1", "upgrade_2", "capstone"] {
            let level = &mut hammer["styles"][0][field][0];
            level["effect"] = json!("attack_pct");
            level["amount"] = json!(10);
            level["kinds"] = json!([]);
        }
    };
    let issues = |v: &Value| match Pack::from_bytes(&serde_json::to_vec(v).unwrap()) {
        Err(PackError::File {
            file: "expertise",
            error: Error::Invalid(issues),
        }) => issues
            .into_iter()
            .map(|i| i.message)
            .collect::<Vec<String>>(),
        other => panic!("expected an expertise refusal, got {other:?}"),
    };
    let mut without = pack();
    style(&mut without);
    assert!(issues(&without).iter().any(|m| m.contains(word)));
    let mut with = pack();
    style(&mut with);
    tree(&mut with["weapons"])["size_stacks"] = stacks(20, 3, 300);
    let found = issues(&with);
    assert!(!found.iter().any(|m| m.contains(word)), "{found:?}");
    assert!(
        found.iter().all(|m| m.contains("is not shipped yet")),
        "{found:?}"
    );
}
