//! The step groups of a style kind (ADR-SIM-9 points 3 and 4): a group stands for each member the
//! weapon's base tree has, and the weapon built for a path has the kind on every one. The base is
//! the bundled Hammer's tree.

use crate::event::Press;
use crate::event::Press::{Main, Smash};
use crate::expertise::{Expertise, StepGroup, StepRef, step_mut, step_ref, weapon_with_style};
use crate::pack::Pack;
use crate::version::Versions;
use crate::weapon::Weapon;
use serde_json::{Value, json};

fn base() -> Weapon {
    Pack::from_bytes(Pack::BUNDLED)
        .expect("the bundled pack")
        .content()
        .weapons()
        .tree("giant_hammer", None)
        .expect("the Hammer's base tree")
        .clone()
}

fn circle(radius: i32) -> Value {
    json!({ "type": "circle", "offset": { "forward": 0, "right": 0 }, "radius": radius })
}

fn charge_grant() -> Value {
    let strike = |r: i32| {
        json!({ "inline": [
            { "kind": "active", "ticks": 3, "type": "strike", "hit_box": circle(r) },
            { "kind": "recovery", "ticks": 4 }
        ] })
    };
    json!({ "charge": { "levels": [
        { "at_ticks": 10, "attack": strike(6_000) },
        { "at_ticks": 20, "attack": strike(8_000) }
    ], "max_ticks": 40 } })
}

/// The Hammer's first style with `kinds` on its own level, built with no place taken.
fn built(base: &Weapon, kinds: Value) -> Weapon {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .unwrap();
    w["styles"][0]["kinds"] = kinds;
    // The path's other levels name nothing, so the bundled kinds' ties to each other stay out.
    for field in ["upgrade_1", "upgrade_2", "capstone"] {
        let level = &mut w["styles"][0][field][0];
        level["effect"] = json!("attack_pct");
        level["amount"] = json!(10);
        level["kinds"] = json!([]);
    }
    let expertise =
        Expertise::parse(v["expertise"].to_string().as_bytes(), Versions::CURRENT).unwrap();
    let style = &expertise
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_hammer")
        .unwrap()
        .styles[0];
    weapon_with_style(base, style, 0)
}

fn path(presses: &str) -> Vec<Press> {
    presses
        .chars()
        .map(|c| if c == 'A' { Main } else { Smash })
        .collect()
}

fn group(name: &str) -> Value {
    json!(name)
}

/// G2: the closed member lists, and `members_in` keeps what a tree has.
#[test]
fn a_group_is_its_closed_list_of_members() {
    let normals: Vec<Vec<Press>> = ["A", "AA", "AAA", "AAAA", "AAAAA"].map(path).to_vec();
    assert_eq!(StepGroup::Normals.members(), normals);
    let smashes: Vec<Vec<Press>> = ["B", "AB", "AAB", "AAAB", "AAAAB", "AAAAAB"]
        .map(path)
        .to_vec();
    assert_eq!(StepGroup::Smashes.members(), smashes);
    assert_eq!(StepRef::Group(StepGroup::Normals).members(), normals);
    assert_eq!(StepRef::Path(path("AB")).members(), vec![path("AB")]);

    // A tree without AAAAA and AAAAAB drops those two members.
    let mut tree = base();
    let aaaa = step_mut(&mut tree.combo, &path("AAAA")).unwrap();
    aaaa.next.main = None;
    aaaa.next.smash = None;
    let kept = StepRef::Group(StepGroup::Normals).members_in(&tree.combo);
    assert_eq!(kept, normals[..4].to_vec());
    let kept = StepRef::Group(StepGroup::Smashes).members_in(&tree.combo);
    assert_eq!(kept, smashes[..4].to_vec());
}

/// G4: a `replace_step` on a group gives each member the tree has the grant.
#[test]
fn a_grant_on_a_group_reaches_every_member_the_tree_has() {
    let tree = base();
    let kind = json!({ "type": "replace_step", "step": group("smashes"), "grant": charge_grant() });
    let weapon = built(&tree, json!([kind.clone()]));
    for m in StepGroup::Smashes.members() {
        let step = step_ref(&weapon.combo, &m).expect("a smash");
        assert!(step.charge.is_some(), "{m:?}");
    }
    // Without AAAAAB: five members charge, the rest of the tree is as it was.
    let mut thin = base();
    step_mut(&mut thin.combo, &path("AAAAA"))
        .unwrap()
        .next
        .smash = None;
    let weapon = built(&thin, json!([kind]));
    let charged = StepGroup::Smashes
        .members()
        .iter()
        .filter(|m| step_ref(&weapon.combo, m).is_some_and(|s| s.charge.is_some()))
        .count();
    assert_eq!(charged, 5);
}

/// G5: a factor and a shortened charge reach every member.
#[test]
fn a_factor_and_a_sooner_charge_reach_every_member() {
    let tree = base();
    let factor = json!({ "type": "proc_factor", "step": group("normals"), "proc_pct": 40 });
    let weapon = built(&tree, json!([factor]));
    for m in StepGroup::Normals.members() {
        assert_eq!(
            step_ref(&weapon.combo, &m).unwrap().proc_pct,
            Some(40),
            "{m:?}"
        );
    }
    let grant =
        json!({ "type": "replace_step", "step": group("smashes"), "grant": charge_grant() });
    let sooner = json!({ "type": "charge_sooner", "step": group("smashes"), "pct": 50 });
    let weapon = built(&tree, json!([grant, sooner]));
    for m in StepGroup::Smashes.members() {
        let ticks: Vec<u32> = step_ref(&weapon.combo, &m)
            .and_then(|s| s.charge.as_ref())
            .expect("a charge")
            .levels
            .iter()
            .map(|l| l.at_ticks)
            .collect();
        assert_eq!(ticks, [5, 10], "{m:?}");
    }
}

/// G7: a group `repeat` leaves one entry on each of the five A steps, after the single-step ones.
#[test]
fn a_group_repeat_is_queued_on_each_member_in_path_order() {
    let repeat = |step: Value, after: u32| {
        json!({ "type": "repeat", "step": step, "after_ticks": after, "repeat_pct": 50,
            "anchor": "hit_list" })
    };
    let weapon = built(
        &base(),
        json!([
            repeat(json!(["main", "main"]), 7),
            repeat(group("normals"), 9)
        ]),
    );
    for m in StepGroup::Normals.members() {
        let after: Vec<u32> = step_ref(&weapon.combo, &m)
            .unwrap()
            .repeats
            .iter()
            .map(|r| r.after_ticks)
            .collect();
        let expected: &[u32] = if m == path("AA") { &[7, 9] } else { &[9] };
        assert_eq!(after, expected, "{m:?}");
    }
    for m in StepGroup::Smashes.members() {
        assert!(
            step_ref(&weapon.combo, &m).unwrap().repeats.is_empty(),
            "{m:?}"
        );
    }
}
