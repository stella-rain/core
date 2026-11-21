//! Step groups (ADR-SIM-9 points 3 and 4): the four kinds that name a step (`replace_step`,
//! `repeat`, `proc_factor`, `charge_sooner`) take a single step or a named group (`normals`,
//! `smashes`). A group stands for each member the weapon's base tree has: one grant per step per
//! style for the first, third and fourth kind, `repeat` counted per member.

use serde_json::{Value, json};
use stella_rain_core::expertise::{Expertise, StyleKind};
use stella_rain_core::pack::{Pack, PackError};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

const A: [&str; 1] = ["main"];
const AA: [&str; 2] = ["main", "main"];
const AAAAA: [&str; 5] = ["main"; 5];

fn circle(radius: i32) -> Value {
    json!({ "type": "circle", "offset": { "forward": 0, "right": 0 }, "radius": radius })
}

fn strike(radius: i32) -> Value {
    json!({ "inline": [
        { "kind": "active", "ticks": 3, "type": "strike", "hit_box": circle(radius) },
        { "kind": "recovery", "ticks": 4 }
    ] })
}

fn charge_grant() -> Value {
    json!({ "charge": { "levels": [
        { "at_ticks": 10, "attack": strike(6_000) },
        { "at_ticks": 20, "attack": strike(8_000) }
    ], "max_ticks": 40 } })
}

fn hold_grant() -> Value {
    let active = json!({ "kind": "active", "ticks": 4, "type": "strike", "hit_box": circle(8192) });
    json!({ "hold": { "loop": { "inline": [active.clone()] },
        "release": { "inline": [active] }, "max_ticks": 60 } })
}

fn replace(step: Value, grant: Value) -> Value {
    json!({ "type": "replace_step", "step": step, "grant": grant })
}

fn repeat(step: Value) -> Value {
    json!({ "type": "repeat", "step": step, "after_ticks": 10, "repeat_pct": 50,
        "anchor": "hit_list" })
}

fn factor(step: Value) -> Value {
    json!({ "type": "proc_factor", "step": step, "proc_pct": 40 })
}

fn sooner(step: Value) -> Value {
    json!({ "type": "charge_sooner", "step": step, "pct": 20 })
}

fn parse_at(file: &Value, sim_version: u32) -> Result<Expertise, Error> {
    let versions = Versions {
        sim_version,
        ..Versions::CURRENT
    };
    Expertise::parse(file.to_string().as_bytes(), versions)
}

/// The expertise file with the first weapon's first style naming `own` and its `upgrade_1`,
/// `upgrade_2` and `capstone` levels naming `up1`, `up2` and `cap` (`None`: as the fixture has it).
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

/// The stateless check's issues for a file, `(path, message)`.
fn parse_issues(file: &Value) -> Vec<(String, String)> {
    match parse_at(file, 2) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        Ok(_) => Vec::new(),
        Err(e) => panic!("expected a refusal or a parse, got {e:?}"),
    }
}

fn parses(file: &Value) {
    if let Err(e) = parse_at(file, 2) {
        panic!("expected valid, got {e:?}");
    }
}

fn refused_at(file: &Value, path: &str, word: &str) {
    let issues = parse_issues(file);
    assert!(
        issues.iter().any(|(p, m)| p == path && m.contains(word)),
        "{path} {word}: {issues:?}"
    );
}

const K0: &str = "$.weapons[0].styles[0].kinds";

fn hammer_pack(own: Vec<Value>, levels: [Option<Vec<Value>>; 3]) -> Value {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = json!(own);
    // A level left out is emptied: the fixture's own (the Titan's hold on AAA) would join the path.
    for (field, kinds) in ["upgrade_1", "upgrade_2", "capstone"].iter().zip(levels) {
        let level = &mut w["styles"][0][field][0];
        if let Some(kinds) = kinds {
            level["effect"] = json!("none");
            level.as_object_mut().unwrap().remove("amount");
            level["kinds"] = json!(kinds);
        } else {
            // A level with no kind needs a stat effect instead.
            level["effect"] = json!("attack_pct");
            level["amount"] = json!(10);
            level["kinds"] = json!([]);
        }
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

fn loads(v: &Value) {
    let issues = pack_issues(v);
    assert!(issues.is_empty(), "{issues:?}");
}

fn pack_refused_at(v: &Value, path: &str, word: &str) {
    let issues = pack_issues(v);
    assert!(
        issues.iter().any(|(p, m)| p == path && m.contains(word)),
        "{path} {word}: {issues:?}"
    );
}

/// G1: a group is a `step`, it serializes back, and a path is as before.
#[test]
fn a_group_parses_and_round_trips_for_each_of_the_four_kinds() {
    for group in ["normals", "smashes"] {
        for kind in [
            replace(json!(group), charge_grant()),
            repeat(json!(group)),
            factor(json!(group)),
            sooner(json!(group)),
        ] {
            let parsed: StyleKind = serde_json::from_value(kind.clone())
                .unwrap_or_else(|e| panic!("{kind}: a group parses: {e}"));
            assert_eq!(serde_json::to_value(&parsed).unwrap(), kind);
        }
    }
    // Neither a made-up group nor a press word the tree has no name for.
    for step in [json!("dash"), json!(["main", "main", "bogus"])] {
        assert!(serde_json::from_value::<StyleKind>(factor(step)).is_err());
    }
    // A path of seven presses is still refused at its key.
    refused_at(
        &own(vec![factor(json!(vec!["main"; 7]))]),
        &format!("{K0}[0].step"),
        "1 to 6",
    );
    // A path is as before.
    parses(&own(vec![factor(json!(["main", "main"]))]));
}

/// G3: one grant per step per style, decided from the closed member lists.
#[test]
fn a_step_takes_one_grant_per_style_whatever_names_it() {
    let again = [
        (
            replace(json!("normals"), charge_grant()),
            replace(json!(AA), hold_grant()),
            "already granted",
        ),
        (factor(json!("normals")), factor(json!(AA)), "proc factor"),
        (sooner(json!("normals")), sooner(json!(AA)), "made sooner"),
        (
            replace(json!("normals"), charge_grant()),
            replace(json!("normals"), hold_grant()),
            "already granted",
        ),
    ];
    for (first, second, word) in again {
        let file = file_with(
            vec![first.clone()],
            [Some(vec![second.clone()]), None, None],
        );
        refused_at(
            &file,
            "$.weapons[0].styles[0].upgrade_1[0].kinds[0].step",
            word,
        );
    }
    // The two groups share no step.
    parses(&file_with(
        vec![factor(json!("normals"))],
        [Some(vec![factor(json!("smashes"))]), None, None],
    ));
    // A repeat on a group and a repeat on a path stand together.
    parses(&own(vec![repeat(json!("normals")), repeat(json!(A))]));
}

/// G4: the pack accepts a group where the tree has members, and refuses one it has none of.
#[test]
fn the_pack_takes_a_group_for_the_members_the_tree_has() {
    loads(&hammer_pack(
        vec![replace(json!("smashes"), charge_grant())],
        [None, None, None],
    ));
    loads(&hammer_pack(
        vec![replace(json!("normals"), hold_grant())],
        [None, None, None],
    ));
}

/// G5: a `charge_sooner` on a group needs a charge on every member it reaches.
#[test]
fn charge_sooner_on_a_group_needs_a_charge_on_each_member() {
    // After the grant on the same group: accepted.
    loads(&hammer_pack(
        vec![replace(json!("smashes"), charge_grant())],
        [Some(vec![sooner(json!("smashes"))]), None, None],
    ));
    // No charge on the Hammer's AAA: refused at its step.
    pack_refused_at(
        &hammer_pack(vec![sooner(json!("normals"))], [None, None, None]),
        &format!("{K0}[0].step"),
        "no charge at this level of the path",
    );
}

/// G6: `repeat` is counted per member over the style's path.
#[test]
fn repeat_is_counted_per_member() {
    let two_normals = vec![repeat(json!("normals")), repeat(json!("normals"))];
    // Two group repeats and two on AAAAA: four on AAAAA.
    let mut four = two_normals.clone();
    four.extend([repeat(json!(AAAAA)), repeat(json!(AAAAA))]);
    // The style's own level holds at most four kinds, so a fifth comes from an upgrade.
    loads(&hammer_pack(four.clone(), [None, None, None]));
    let fifth = hammer_pack(four.clone(), [Some(vec![repeat(json!(AAAAA))]), None, None]);
    pack_refused_at(
        &fifth,
        "$.weapons[0].styles[0].upgrade_1[0].kinds[0]",
        "at most 4 repeats on one step",
    );
    // A group over a step already at four is refused and counts none of its members: a later
    // repeat on A is accepted.
    let aa4 = vec![
        repeat(json!(AA)),
        repeat(json!(AA)),
        repeat(json!(AA)),
        repeat(json!(AA)),
    ];
    let group_over = hammer_pack(
        aa4.clone(),
        [Some(vec![repeat(json!("normals"))]), None, None],
    );
    pack_refused_at(
        &group_over,
        "$.weapons[0].styles[0].upgrade_1[0].kinds[0]",
        "at most 4 repeats on one step",
    );
    let issues = pack_issues(&group_over);
    assert!(issues.iter().any(|(_, m)| m.contains("AA")), "{issues:?}");
    loads(&hammer_pack(
        aa4,
        [Some(vec![repeat(json!(A))]), None, None],
    ));
    // A group repeat on a held member.
    pack_refused_at(
        &hammer_pack(
            vec![replace(json!(A), hold_grant()), repeat(json!("normals"))],
            [None, None, None],
        ),
        &format!("{K0}[1]"),
        "a held step does not repeat",
    );
    // One group repeat is one kind against the level's four.
    let five = file_with(
        vec![
            repeat(json!(A)),
            repeat(json!(AA)),
            repeat(json!("smashes")),
            factor(json!(A)),
            repeat(json!("normals")),
        ],
        [None, None, None],
    );
    refused_at(&five, K0, "");
    parses(&own(vec![
        repeat(json!(A)),
        repeat(json!(AA)),
        factor(json!(A)),
        repeat(json!("normals")),
    ]));
}

fn range_pct(pct: u32) -> Value {
    json!({ "type": "repeat_range_pct", "pct": pct })
}

fn proc_pct(pct: u32) -> Value {
    json!({ "type": "repeat_proc_pct", "proc_pct": pct })
}

fn size_stack() -> Value {
    json!({ "type": "repeat_size_stack" })
}

fn place_repeat() -> Value {
    json!({ "type": "repeat", "step": A, "after_ticks": 10, "repeat_pct": 50, "anchor": "place" })
}

const UP1: &str = "$.weapons[0].styles[0].upgrade_1[0].kinds[0]";

/// R1: each rider round trips; its number is bounded at its key; it needs `sim_version` 2 and is
/// named once per style path.
#[test]
fn a_rider_round_trips_and_is_bounded_at_its_key() {
    for rider in [range_pct(20), proc_pct(50), size_stack()] {
        let parsed: StyleKind = serde_json::from_value(rider.clone())
            .unwrap_or_else(|e| panic!("{rider}: a rider parses: {e}"));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), rider);
        parses(&own(vec![rider]));
    }
    for (rider, key) in [
        (range_pct(0), "pct"),
        (range_pct(101), "pct"),
        (proc_pct(101), "proc_pct"),
    ] {
        refused_at(&own(vec![rider]), &format!("{K0}[0].{key}"), "");
    }
    for rider in [range_pct(1), range_pct(100), proc_pct(0), proc_pct(100)] {
        parses(&own(vec![rider]));
    }
    // No key but its own.
    assert!(
        serde_json::from_value::<StyleKind>(json!({ "type": "repeat_size_stack", "pct": 1 }))
            .is_err()
    );
    for rider in [range_pct(20), proc_pct(50), size_stack()] {
        match parse_at(&own(vec![rider.clone()]), 1) {
            Err(Error::Invalid(issues)) => assert!(
                issues
                    .iter()
                    .any(|i| i.path == K0 && i.message.contains("sim_version 2")),
                "{rider}: {issues:?}"
            ),
            other => panic!("{rider}: expected invalid, got {:?}", other.map(|_| ())),
        }
        // Once per level and once per style path.
        refused_at(
            &own(vec![rider.clone(), rider.clone()]),
            &format!("{K0}[1]"),
            "already",
        );
        refused_at(
            &file_with(vec![rider.clone()], [Some(vec![rider.clone()]), None, None]),
            UP1,
            "already",
        );
    }
}

/// R2: the views carry the riders (read from an `Expertise` that parse accepted).
#[test]
fn the_views_carry_the_riders() {
    let riders = vec![range_pct(20), proc_pct(50), size_stack()];
    let mut file: Value = serde_json::from_slice(crate::before::EXPERTISE).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    for w in file["weapons"].as_array_mut().unwrap() {
        if w["weapon"] != "giant_hammer" {
            continue;
        }
        w["base"] = json!([]);
        for style in w["styles"].as_array_mut().unwrap() {
            style["stats"] = json!([]);
        }
        w["styles"][0]["kinds"] = json!(riders);
    }
    let expertise = parse_at(&file, 2).expect("the riders parse");
    let e = crate::style_grants::build(expertise);
    let snap = e.snapshot();
    let ready =
        serde_json::to_value(snap.players[0].tree_ready.as_ref().expect("a point")).unwrap();
    assert_eq!(ready["kinds"][0], json!(riders));
}

fn is_refused(v: &Value, message: &str) -> bool {
    pack_issues(v).iter().any(|(_, m)| m.contains(message))
}

/// M1: parse accepts an unshipped kind; the pack check refuses it at the kind, whatever the tree.
#[test]
fn the_pack_refuses_an_unshipped_kind() {
    for (rider, name) in [
        (range_pct(20), "repeat_range_pct"),
        (proc_pct(50), "repeat_proc_pct"),
        (size_stack(), "repeat_size_stack"),
    ] {
        parses(&own(vec![repeat(json!(A)), rider.clone()]));
        pack_refused_at(
            &hammer_pack(vec![repeat(json!(A)), rider], [None, None, None]),
            &format!("{K0}[1]"),
            &format!("the kind {name} is not shipped yet"),
        );
    }
    // A shipped kind is not refused for it.
    loads(&hammer_pack(
        vec![repeat(json!(A)), factor(json!(AA))],
        [None, None, None],
    ));
    // Every weapon of the expertise file is gated, not only the Hammer. (A weapon with no tree is
    // unreachable here: the weapons file's own checks refuse it first, so the gate that runs
    // before the tree lookup is code, not a case.)
    let mut v = hammer_pack(vec![repeat(json!(A))], [None, None, None]);
    let sword = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_sword")
        .expect("the Sword's expertise");
    sword["styles"][0]["kinds"] = json!([range_pct(20)]);
    assert!(
        is_refused(&v, "is not shipped yet"),
        "{:?}",
        pack_issues(&v)
    );
}

/// M3: the riders need a repeat to close; `repeat_range_pct` a place repeat; `repeat_size_stack`
/// the base tree's size-stack field, which no tree has yet.
#[test]
fn a_rider_with_nothing_to_close_is_refused() {
    let nothing = "nothing closes";
    let no_place = "no place repeat";
    let no_field = "no size-stack field";
    let run = |own: Vec<Value>, up1: Option<Vec<Value>>, cap: Option<Vec<Value>>, word: &str| {
        is_refused(&hammer_pack(own, [up1, None, cap]), word)
    };
    // `repeat_range_pct`: none, a hit-list repeat only, a place repeat before, one only after.
    assert!(run(vec![range_pct(20)], None, None, nothing));
    assert!(run(
        vec![repeat(json!(A)), range_pct(20)],
        None,
        None,
        no_place
    ));
    assert!(!run(
        vec![place_repeat(), range_pct(20)],
        None,
        None,
        no_place
    ));
    assert!(!run(
        vec![place_repeat(), range_pct(20)],
        None,
        None,
        nothing
    ));
    assert!(run(
        vec![range_pct(20)],
        None,
        Some(vec![place_repeat()]),
        no_place
    ));
    // `repeat_proc_pct`: none, and a repeat at or before its level.
    assert!(run(vec![proc_pct(50)], None, None, nothing));
    assert!(!run(
        vec![repeat(json!(A)), proc_pct(50)],
        None,
        None,
        nothing
    ));
    assert!(!run(
        vec![repeat(json!(A))],
        Some(vec![proc_pct(50)]),
        None,
        nothing
    ));
    assert!(run(
        vec![proc_pct(50)],
        None,
        Some(vec![repeat(json!(A))]),
        nothing
    ));
    // `repeat_size_stack`: a repeat does not make up for the missing field.
    assert!(run(
        vec![repeat(json!(A)), size_stack()],
        None,
        None,
        no_field
    ));
    assert!(!run(
        vec![repeat(json!(A)), size_stack()],
        None,
        None,
        nothing
    ));
    assert!(run(vec![size_stack()], None, None, nothing));
    assert!(run(vec![size_stack()], None, None, no_field));
}
