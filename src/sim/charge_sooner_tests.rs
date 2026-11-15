//! The `charge_sooner` kind of a style level (ADR-SIM-9 points 2 to 7, ADR-SIM-5): the weapon
//! built for a path that has taken the kind reaches each level of the step's charge sooner and
//! changes nothing else. Every number is a test value or the bundled Sword's own.

use crate::expertise::{Expertise, sooner_at, weapon_with_style};
use crate::pack::Pack;
use crate::version::Versions;
use crate::weapon::Weapon;
use serde_json::{Value, json};

const AAA: [&str; 3] = ["main", "main", "main"];

/// The bundled Sword's expertise with `upgrade_2` naming the kind on `step` at `pct`.
fn sword_with(step: &[&str], pct: u32) -> (Weapon, Expertise) {
    let pack = Pack::from_bytes(Pack::BUNDLED).expect("the bundled pack");
    let base = pack
        .content()
        .weapons()
        .tree("giant_sword", None)
        .expect("the Sword's base tree")
        .clone();
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_sword")
        .unwrap();
    let up = &mut w["styles"][0]["upgrade_2"][0];
    up["effect"] = json!("none");
    up.as_object_mut().unwrap().remove("amount");
    up["kinds"] = json!([{ "type": "charge_sooner", "step": step, "pct": pct }]);
    let file = v["expertise"].to_string();
    let expertise = Expertise::parse(file.as_bytes(), Versions::CURRENT).expect("the expertise");
    (base, expertise)
}

fn built(base: &Weapon, expertise: &Expertise, taken: usize) -> Weapon {
    let style = &expertise
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_sword")
        .unwrap()
        .styles[0];
    weapon_with_style(base, style, taken)
}

fn charge_of(weapon: &Weapon) -> &crate::weapon::Charge {
    weapon
        .combo
        .main
        .as_deref()
        .and_then(|a| a.next.main.as_deref())
        .and_then(|a| a.next.main.as_deref())
        .and_then(|s| s.charge.as_ref())
        .expect("AAA's charge")
}

/// W1: places 0 and 1 keep the charge's ticks, places 2 and 3 take 25% off each level; the rest
/// of the charge equals the place-0 weapon's.
#[test]
fn the_levels_come_sooner_from_the_second_place_and_nothing_else_moves() {
    let (base, expertise) = sword_with(&AAA, 25);
    let first = built(&base, &expertise, 0);
    for (taken, at) in [(0, [20, 50]), (1, [20, 50]), (2, [15, 37]), (3, [15, 37])] {
        let weapon = built(&base, &expertise, taken);
        let charge = charge_of(&weapon);
        let ticks: Vec<u32> = charge.levels.iter().map(|l| l.at_ticks).collect();
        assert_eq!(ticks, at, "taken {taken}");
        let mut expected = charge_of(&first).clone();
        for (level, now) in expected.levels.iter_mut().zip(at) {
            level.at_ticks = now;
        }
        assert_eq!(charge, &expected, "taken {taken}: only at_ticks differ");
        assert_eq!(charge.max_ticks, 90);
    }
}

/// W2: the one formula, floored, at least 1, with no overflow.
#[test]
fn the_formula_floors_and_never_overflows() {
    assert_eq!(sooner_at(20, 25), 15);
    assert_eq!(sooner_at(50, 25), 37);
    assert_eq!(sooner_at(1, 90), 1);
    assert_eq!(sooner_at(10, 25), 7);
    assert_eq!(sooner_at(u32::MAX, 90), 429_496_729);
}

/// W3: a step with no charge in the base (AAAAA) is left alone by the build.
#[test]
fn a_step_with_no_charge_leaves_the_weapon_unchanged() {
    let (base, expertise) = sword_with(&["main"; 5], 25);
    assert_eq!(built(&base, &expertise, 2), built(&base, &expertise, 0));
}
