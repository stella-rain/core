//! The Wave 3 style kinds that are only typed here (ADR-SIM-9 points 3 and 4, ADR-SIM-5,
//! ADR-SVC-1): `encore`, `knockdown_steps`, `down_bonus_pct` and `area_bonus` are bounded,
//! validated against the base tree, viewed, and refused by the pack check as unshipped until their
//! rules land. Test code only: the numbers are test values, not content.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

use crate::style_grants::build;

const K0: &str = "$.weapons[0].styles[0].kinds";
const MAIN5: [&str; 5] = ["main"; 5];

fn encore() -> Value {
    json!({ "type": "encore", "step": ["main", "main", "main", "main", "main", "smash"],
        "skip_presses": 2, "ticks": 180, "speed_pct": 25, "wave_presses": 1 })
}

fn encore_with(key: &str, value: Value) -> Value {
    let mut e = encore();
    e[key] = value;
    e
}

fn knockdown(steps: Value) -> Value {
    json!({ "type": "knockdown_steps", "steps": steps, "vulnerable_after": 60,
        "second_hit_pct": 50 })
}

fn down_bonus(pct: u32) -> Value {
    json!({ "type": "down_bonus_pct", "pct": pct })
}

fn area() -> Value {
    json!({ "type": "area_bonus", "ticks_pct": 33, "range_pct": 20, "proc_pct": 100 })
}

fn parse_at(v: &Value, sim_version: u32) -> Result<Expertise, Error> {
    let versions = Versions {
        sim_version,
        ..Versions::CURRENT
    };
    Expertise::parse(v.to_string().as_bytes(), versions)
}

/// The expertise file with the first style naming `own`, and its upgrade_1, upgrade_2 and
/// capstone levels naming `levels` (`None`: as the fixture has it).
fn file_with(own: Vec<Value>, levels: [Option<Vec<Value>>; 3]) -> Value {
    let mut file: Value = serde_json::from_slice(crate::before::EXPERTISE).unwrap();
    file["weapons"][0]["styles"][0]["kinds"] = json!(own);
    for (field, kinds) in ["upgrade_1", "upgrade_2", "capstone"].iter().zip(levels) {
        if let Some(kinds) = kinds {
            let level = &mut file["weapons"][0]["styles"][0][field][0];
            level["effect"] = json!("none");
            level.as_object_mut().unwrap().remove("amount");
            level["kinds"] = json!(kinds);
        }
    }
    file
}

fn own(kinds: Vec<Value>) -> Value {
    file_with(kinds, [None, None, None])
}

fn valid(file: &Value) {
    if let Err(e) = parse_at(file, 2) {
        panic!("expected valid, got {e:?}");
    }
}

fn issues_of(file: &Value) -> Vec<(String, String)> {
    match parse_at(file, 2) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        Ok(_) => Vec::new(),
        Err(e) => panic!("expected a refusal or a parse, got {e:?}"),
    }
}

fn refused_at(file: &Value, path: &str, word: &str) {
    let issues = issues_of(file);
    assert!(
        issues.iter().any(|(p, m)| p == path && m.contains(word)),
        "{path} {word}: {issues:?}"
    );
}

fn parse_error(file: &Value) {
    assert!(
        matches!(parse_at(file, 2), Err(Error::Parse { .. })),
        "expected a parse error"
    );
}

fn field(kind: Value, key: &str, value: Value, name: &str) -> (Value, String) {
    let mut kind = kind;
    kind[key] = value;
    (kind, format!("{K0}[0].{name}"))
}

/// K1: the four kinds across the levels of one style parse and serialize back to the same JSON.
#[test]
fn the_four_kinds_round_trip() {
    for kind in [
        encore(),
        knockdown(json!(["smashes"])),
        down_bonus(50),
        area(),
    ] {
        let parsed: StyleKind = serde_json::from_value(kind.clone()).expect("the kind parses");
        assert_eq!(serde_json::to_value(&parsed).unwrap(), kind);
    }
    valid(&file_with(
        vec![encore(), knockdown(json!(["smashes"]))],
        [Some(vec![down_bonus(50)]), Some(vec![area()]), None],
    ));
    // An absent optional is absent in the output.
    let bare = json!({ "type": "encore", "step": ["main"], "skip_presses": 1, "ticks": 60,
        "speed_pct": 10 });
    let parsed: StyleKind = serde_json::from_value(bare.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), bare);
}

/// K2: each bound of the encore, one pair per bound; the naming rules.
#[test]
fn the_encore_is_bounded() {
    let cases: [(&str, u32, u32, u32, u32); 4] = [
        ("skip_presses", 0, 5, 1, 4),
        ("ticks", 0, 1801, 1, 1800),
        ("speed_pct", 0, 801, 1, 800),
        ("wave_presses", 0, 17, 1, 16),
    ];
    for (key, lo, hi, ok_lo, ok_hi) in cases {
        for bad in [lo, hi] {
            let (kind, at) = field(encore(), key, json!(bad), key);
            refused_at(&own(vec![kind]), &at, "");
        }
        for good in [ok_lo, ok_hi] {
            valid(&own(vec![encore_with(key, json!(good))]));
        }
    }
    let seven = json!(["main", "main", "main", "main", "main", "main", "smash"]);
    refused_at(
        &own(vec![encore_with("step", seven)]),
        &format!("{K0}[0].step"),
        "1 to 6 presses",
    );
    let mut missing = encore();
    missing.as_object_mut().unwrap().remove("ticks");
    parse_error(&own(vec![missing]));
    parse_error(&own(vec![encore_with("extra", json!(1))]));
    refused_at(
        &own(vec![encore(), encore()]),
        &format!("{K0}[1]"),
        "already names an encore",
    );
    valid(&file_with(
        vec![encore()],
        [Some(vec![encore()]), None, None],
    ));
}

/// K3: the knockdown steps' bounds and the once-per-step rules.
#[test]
fn the_knockdown_is_bounded() {
    let at = format!("{K0}[0]");
    refused_at(&own(vec![knockdown(json!([]))]), &format!("{at}.steps"), "");
    let seven = json!(vec!["smashes"; 7]);
    refused_at(&own(vec![knockdown(seven)]), &format!("{at}.steps"), "");
    for (key, bad) in [
        ("vulnerable_after", 0),
        ("vulnerable_after", 301),
        ("second_hit_pct", 0),
        ("second_hit_pct", 1001),
    ] {
        let mut k = knockdown(json!(["smashes"]));
        k[key] = json!(bad);
        refused_at(&own(vec![k]), &format!("{at}.{key}"), "");
    }
    for (key, good) in [
        ("vulnerable_after", 300),
        ("second_hit_pct", 1000),
        ("second_hit_pct", 1),
    ] {
        let mut k = knockdown(json!(["smashes"]));
        k[key] = json!(good);
        valid(&own(vec![k]));
    }
    // A later level may list no step when it names an effect, and none when it names nothing.
    let later = |k: Value| {
        file_with(
            vec![knockdown(json!(["smashes"]))],
            [Some(vec![k]), None, None],
        )
    };
    valid(&later(
        json!({ "type": "knockdown_steps", "steps": [], "vulnerable_after": 30 }),
    ));
    refused_at(
        &later(json!({ "type": "knockdown_steps", "steps": [] })),
        "$.weapons[0].styles[0].upgrade_1[0].kinds[0]",
        "names nothing",
    );
    // A group and a member of it in one level share a member.
    refused_at(
        &own(vec![knockdown(json!([
            "smashes",
            ["main", "main", "smash"]
        ]))]),
        &format!("{at}.steps[1]"),
        "already listed",
    );
    // An earlier level's member listed again at a later level.
    refused_at(
        &later(knockdown(json!(["smashes"]))),
        "$.weapons[0].styles[0].upgrade_1[0].kinds[0].steps[0]",
        "already listed",
    );
    valid(&own(vec![knockdown(json!(["normals", "smashes"]))]));
}

/// K4: the down bonus and the area bonus.
#[test]
fn the_down_bonus_and_the_area_bonus_are_bounded() {
    for bad in [0, 1001] {
        refused_at(&own(vec![down_bonus(bad)]), &format!("{K0}[0].pct"), "");
    }
    valid(&own(vec![down_bonus(1)]));
    valid(&own(vec![down_bonus(1000)]));
    refused_at(
        &own(vec![down_bonus(50), down_bonus(60)]),
        &format!("{K0}[1]"),
        "already sets the down bonus",
    );
    refused_at(
        &file_with(
            vec![down_bonus(50)],
            [Some(vec![down_bonus(60)]), None, None],
        ),
        "$.weapons[0].styles[0].upgrade_1[0].kinds[0]",
        "already sets the down bonus",
    );
    refused_at(
        &own(vec![json!({ "type": "area_bonus" })]),
        &format!("{K0}[0]"),
        "names no effect",
    );
    for (key, bad, good) in [
        ("ticks_pct", [0, 1001], 1000),
        ("range_pct", [0, 101], 100),
        ("proc_pct", [0, 101], 100),
    ] {
        for b in bad {
            refused_at(
                &own(vec![json!({ "type": "area_bonus", key: b })]),
                &format!("{K0}[0].{key}"),
                "",
            );
        }
        valid(&own(vec![json!({ "type": "area_bonus", key: good })]));
    }
    valid(&file_with(vec![area()], [Some(vec![area()]), None, None]));
}

/// K5: the new kinds count against `MAX_LEVEL_KINDS`; they need sim_version 2.
#[test]
fn the_kinds_count_and_need_version_2() {
    let five = vec![
        encore(),
        knockdown(json!(["smashes"])),
        down_bonus(50),
        area(),
        json!({ "type": "combo_tempo_pct", "max_stacks": 2, "effects": [] }),
    ];
    refused_at(&own(five), &format!("{K0}"), "");
    for kind in [
        encore(),
        knockdown(json!(["smashes"])),
        down_bonus(5),
        area(),
    ] {
        match parse_at(&own(vec![kind]), 1) {
            Err(Error::Invalid(issues)) => {
                assert!(issues.iter().any(|i| i.path == K0), "{issues:?}")
            }
            other => panic!("{other:?}"),
        }
    }
}

fn hammer_pack(kinds: Vec<Value>) -> Value {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = json!(kinds);
    for field in ["upgrade_1", "upgrade_2", "capstone"] {
        let level = &mut w["styles"][0][field][0];
        level["effect"] = json!("attack_pct");
        level["amount"] = json!(10);
        level["kinds"] = json!([]);
    }
    v
}

fn pack_issues(v: &Value) -> Vec<(String, String)> {
    match Pack::from_bytes(&serde_json::to_vec(v).unwrap()) {
        Ok(_) => Vec::new(),
        Err(PackError::File {
            file: "expertise",
            error: Error::Invalid(issues),
        }) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        Err(e) => panic!("expected an expertise refusal or a load, got {e:?}"),
    }
}

fn pack_has(v: &Value, path: &str, word: &str) -> bool {
    pack_issues(v)
        .iter()
        .any(|(p, m)| p == path && m.contains(word))
}

fn only_the_unshipped(v: &Value) {
    let issues = pack_issues(v);
    assert!(
        issues.iter().all(|(_, m)| m.contains("is not shipped yet")),
        "{issues:?}"
    );
}

/// K6: the pack check reads each step against the weapon's base tree.
#[test]
fn the_pack_reads_the_steps_against_the_tree() {
    let at = format!("{K0}[0]");
    let lacks = "no such step in the weapon's base tree";
    let missing = encore_with("step", json!(["smash", "main"]));
    assert!(pack_has(
        &hammer_pack(vec![missing]),
        &format!("{at}.step"),
        lacks
    ));
    let ok = encore_with("step", json!(MAIN5));
    let ok = encore_with_skip(ok, 4);
    only_the_unshipped(&hammer_pack(vec![ok]));
    // A chain that is three deep: skip_presses 3 would need a step four deep.
    let mut shallow = hammer_pack(vec![encore_with_skip(
        encore_with("step", json!(["main"])),
        3,
    )]);
    for w in shallow["weapons"].as_array_mut().unwrap() {
        if w["id"] == "giant_hammer" {
            let main =
                &mut w["trees"][0]["weapon"]["combo"]["main"]["next"]["main"]["next"]["main"];
            main.as_object_mut().unwrap().remove("next");
        }
    }
    assert!(pack_has(
        &shallow,
        &format!("{at}.skip_presses"),
        "no step 4 presses deep"
    ));
    // A knockdown entry naming a missing step, and a group with a member the tree has.
    let k = knockdown(json!([["smash", "main"]]));
    assert!(pack_has(
        &hammer_pack(vec![k]),
        &format!("{at}.steps[0]"),
        lacks
    ));
    only_the_unshipped(&hammer_pack(vec![knockdown(json!(["normals"]))]));
}

fn encore_with_skip(kind: Value, skip: u32) -> Value {
    let mut kind = kind;
    kind["skip_presses"] = json!(skip);
    kind
}

/// K7: the knockdown kinds need the rules' `down_ticks`.
#[test]
fn the_down_kinds_need_the_rules_down_ticks() {
    let kinds = vec![knockdown(json!(["smashes"])), down_bonus(50)];
    let word = "needs the rules' down_ticks";
    let mut without = hammer_pack(kinds.clone());
    without["rules"]
        .as_object_mut()
        .unwrap()
        .remove("down_ticks");
    assert!(pack_has(&without, &format!("{K0}[0]"), word));
    assert!(pack_has(&without, &format!("{K0}[1]"), word));
    let with = hammer_pack(kinds);
    assert!(
        !pack_issues(&with).iter().any(|(_, m)| m.contains(word)),
        "{:?}",
        pack_issues(&with)
    );
    let mut neither = hammer_pack(vec![encore(), area()]);
    neither["rules"]
        .as_object_mut()
        .unwrap()
        .remove("down_ticks");
    assert!(!pack_issues(&neither).iter().any(|(_, m)| m.contains(word)));
}

/// K8: the views carry each kind compactly; an optional not named is an absent key.
#[test]
fn the_views_carry_the_kinds() {
    let view = |kinds: Vec<Value>| {
        let mut f: Value = serde_json::from_slice(crate::before::EXPERTISE).unwrap();
        f["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
        let w = f["weapons"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|w| w["weapon"] == "giant_hammer")
            .unwrap();
        w["base"] = json!([]);
        for st in w["styles"].as_array_mut().unwrap() {
            st["stats"] = json!([]);
        }
        w["styles"][0]["kinds"] = json!(kinds);
        let expertise = parse_at(&f, 2).expect("the expertise");
        let snap = build(expertise).snapshot();
        let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
        ready["kinds"][0].clone()
    };
    assert_eq!(
        view(vec![
            encore(),
            knockdown(json!(["smashes"])),
            down_bonus(50),
            area()
        ]),
        json!([
            { "type": "encore",
              "step": ["main", "main", "main", "main", "main", "smash"],
              "skip_presses": 2, "ticks": 180, "speed_pct": 25, "wave_presses": 1 },
            { "type": "knockdown_steps", "steps": ["smashes"], "vulnerable_after": 60,
              "second_hit_pct": 50 },
            { "type": "down_bonus_pct", "pct": 50 },
            { "type": "area_bonus", "ticks_pct": 33, "range_pct": 20, "proc_pct": 100 }
        ])
    );
    let bare = view(vec![
        json!({ "type": "encore", "step": ["main"], "skip_presses": 1, "ticks": 60,
            "speed_pct": 10 }),
        json!({ "type": "knockdown_steps", "steps": ["smashes"] }),
        json!({ "type": "area_bonus", "range_pct": 5 }),
    ]);
    assert_eq!(
        bare,
        json!([
            { "type": "encore", "step": ["main"], "skip_presses": 1, "ticks": 60,
              "speed_pct": 10 },
            { "type": "knockdown_steps", "steps": ["smashes"] },
            { "type": "area_bonus", "range_pct": 5 }
        ])
    );
}

/// E1: the pack check refuses each kind, and each named optional, as unshipped.
#[test]
fn the_pack_refuses_each_unshipped_kind_and_effect() {
    for (i, (kind, tag)) in [
        (encore(), "encore"),
        (knockdown(json!(["smashes"])), "knockdown_steps"),
        (down_bonus(50), "down_bonus_pct"),
        (area(), "area_bonus"),
    ]
    .into_iter()
    .enumerate()
    {
        let v = hammer_pack(vec![kind]);
        assert!(
            pack_has(
                &v,
                &format!("{K0}[0]"),
                &format!("the kind {tag} is not shipped yet")
            ),
            "{i} {tag}: {:?}",
            pack_issues(&v)
        );
    }
    for (kind, key) in [
        (encore(), "wave_presses"),
        (knockdown(json!(["smashes"])), "vulnerable_after"),
        (knockdown(json!(["smashes"])), "second_hit_pct"),
        (area(), "ticks_pct"),
        (area(), "range_pct"),
        (area(), "proc_pct"),
    ] {
        let v = hammer_pack(vec![kind]);
        assert!(
            pack_has(
                &v,
                &format!("{K0}[0].{key}"),
                &format!("the effect {key} is not shipped yet")
            ),
            "{key}: {:?}",
            pack_issues(&v)
        );
    }
    assert!(Pack::from_bytes(Pack::BUNDLED).is_ok());
}
