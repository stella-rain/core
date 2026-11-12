//! Style grants (ADR-SIM-9 points 2 to 5): a style level's `replace_step` kind gives a step of
//! the base tree a charge or a hold once the style, or the upgrade, is taken. The simulation
//! reads it from the places already recorded, so nothing is added to the state. The runs are
//! equipment runs on the Hammer preset with test expertise; picks are `TreePick`s.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::version::Versions;

use crate::common;
use crate::common::equip::{Parts, equipped_in};

use common::*;

fn circle(radius: i32) -> Value {
    json!({ "type": "circle", "offset": { "forward": 0, "right": 0 }, "radius": radius })
}

/// An attack of one active phase whose hit box is a circle of `radius`: told apart by it.
fn strike(radius: i32) -> Value {
    json!({ "inline": [
        { "kind": "active", "ticks": 3, "type": "strike", "hit_box": circle(radius) },
        { "kind": "recovery", "ticks": 4 }
    ] })
}

fn charge(radii: [i32; 2]) -> Value {
    json!({ "charge": {
        "levels": [
            { "at_ticks": 10, "attack": strike(radii[0]) },
            { "at_ticks": 20, "attack": strike(radii[1]) }
        ],
        "max_ticks": 40
    } })
}

fn hold_grant() -> Value {
    let active =
        |r: i32| json!({ "kind": "active", "ticks": 4, "type": "strike", "hit_box": circle(r) });
    json!({ "hold": { "loop": { "inline": [active(7_000)] },
        "release": { "inline": [active(9_000)] }, "max_ticks": 30 } })
}

fn kind(step: &[&str], grant: Value) -> Value {
    json!({ "type": "replace_step", "step": step, "grant": grant })
}

fn level(id: &str, kinds: Vec<Value>) -> Value {
    json!({ "id": id, "effect": "none", "kinds": kinds })
}

/// Style 0 grants a two-level charge on A; style 1 names no kind; style 2's first upgrade grants
/// a hold on AA and its capstone a charge on B.
fn test_expertise(kinds: bool) -> Expertise {
    expertise_for("giant_hammer", kinds)
}

fn expertise_for(weapon: &str, kinds: bool) -> Expertise {
    let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    for w in file["weapons"].as_array_mut().unwrap() {
        if w["weapon"] != weapon {
            continue;
        }
        w["base"] = json!([]);
        for s in w["styles"].as_array_mut().unwrap() {
            s["stats"] = json!([]);
        }
        if weapon == "giant_sword" {
            // The Sword's base AAA has a charge: a hold replaces it.
            if kinds {
                w["styles"][0]["kinds"] = json!([kind(&["main", "main", "main"], hold_grant())]);
            }
        } else if kinds {
            w["styles"][0]["kinds"] = json!([kind(&["main"], charge([6_000, 8_000]))]);
            w["styles"][2]["upgrade_1"] = json!([level(
                "hold_aa",
                vec![kind(&["main", "main"], hold_grant())]
            )]);
            w["styles"][2]["capstone"] = json!([level(
                "charge_b",
                vec![kind(&["smash"], charge([5_000, 5_500]))]
            )]);
        }
    }
    Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the test expertise")
}

pub(crate) fn build(expertise: Expertise) -> Engine {
    build_with("giant_hammer", expertise)
}

fn build_with(weapon: &str, expertise: Expertise) -> Engine {
    let far = START_Y - 90_000;
    let mut w = wave(
        1,
        1,
        0,
        enemy(1_000_000, hold(START_X, far), json!([])),
        START_X,
        far,
    );
    w["expertise"] = json!({ "hit": 0, "kill": 10 });
    let mut stage = armed(
        stage_full(18_000, Some(far_boss()), vec![w], vec![]),
        json!({ "preset": { "id": weapon } }),
    );
    stage.sim_version = 2;
    let parts = Parts {
        expertise: Some(expertise),
        expertise_points: 550,
        ..Parts::default()
    };
    let file = json!({ "rarities": common::rarities(), "rewards": [
        { "id": "reach", "rarity": "common", "effect": "range_pct", "amount": 30, "limit": 2 }
    ] });
    let mut e = equipped_in(&stage, file, parts, &[], 1);
    e.step(&[idle(), Default::default(), Default::default()]);
    e
}

fn pick(e: &mut Engine, option: u32) {
    e.act(PlayerIdx::SOLO, EquipAction::TreePick { option })
        .expect("a pick");
}

fn step(e: &mut Engine, input: Input) {
    e.step(&[input, Default::default(), Default::default()]);
}

fn main_held() -> Input {
    Input {
        main: true,
        ..Input::default()
    }
}

fn smash_held() -> Input {
    Input {
        smash: true,
        ..Input::default()
    }
}

/// Lets the combo run out and the character idle.
fn settle(e: &mut Engine) {
    for _ in 0..80 {
        step(e, idle());
    }
}

fn charge_of(e: &Engine) -> Option<u8> {
    e.snapshot().players[0].charge.map(|c| c.level)
}

fn hold_of(e: &Engine) -> bool {
    e.snapshot().players[0].hold.is_some()
}

/// The hit box radius of the attack the combo plays on this tick, if any.
fn radius_of(e: &Engine) -> Option<i64> {
    let s = e.snapshot();
    s.players[0]
        .attacks
        .iter()
        .find_map(|a| serde_json::to_value(a.hit_box.as_ref()?).ok())
        .and_then(|b| b["radius"].as_i64())
}

/// Holds `input` for `ticks`, then lets go; returns the radii seen in the 12 ticks after.
fn hold_then_release(e: &mut Engine, input: Input, ticks: u32) -> Vec<i64> {
    for _ in 0..ticks {
        step(e, input);
    }
    let mut seen = Vec::new();
    for _ in 0..12 {
        step(e, idle());
        seen.extend(radius_of(e));
    }
    seen
}

#[test]
fn a_style_grant_gives_its_step_a_charge_once_the_style_is_taken() {
    let mut e = build(test_expertise(true));
    // (1) No style yet: a held A does not charge.
    for _ in 0..12 {
        step(&mut e, main_held());
        assert_eq!(charge_of(&e), None);
    }
    settle(&mut e);
    // (2) Style 0 is taken: the same held A charges, levels at 10 and 20, and the release
    // plays that level's attack.
    pick(&mut e, 0);
    step(&mut e, main_held());
    assert_eq!(charge_of(&e), Some(0));
    for _ in 0..10 {
        step(&mut e, main_held());
    }
    assert_eq!(charge_of(&e), Some(1));
    for _ in 0..10 {
        step(&mut e, main_held());
    }
    assert_eq!(charge_of(&e), Some(2));
    let seen = hold_then_release(&mut e, idle(), 0);
    assert!(seen.contains(&8_000), "{seen:?}");
    settle(&mut e);
    let seen = hold_then_release(&mut e, main_held(), 12);
    assert!(seen.contains(&6_000), "{seen:?}");
}

#[test]
fn another_style_does_not_charge() {
    let mut e = build(test_expertise(true));
    pick(&mut e, 1);
    for _ in 0..12 {
        step(&mut e, main_held());
        assert_eq!(charge_of(&e), None);
    }
}

#[test]
fn an_upgrade_s_grant_applies_once_it_is_taken() {
    let mut e = build(test_expertise(true));
    pick(&mut e, 2);
    let aa = |e: &mut Engine| {
        // A, then A again held: the second step is the granted one.
        step(e, main_held());
        for _ in 0..30 {
            step(e, idle());
        }
        let mut held = false;
        for _ in 0..12 {
            step(e, main_held());
            held |= hold_of(e);
        }
        held
    };
    assert!(!aa(&mut e), "no hold before the upgrade");
    settle(&mut e);
    pick(&mut e, 0);
    assert!(aa(&mut e), "a hold after it");
    settle(&mut e);
    // B from neutral charges only after the capstone.
    for _ in 0..12 {
        step(&mut e, smash_held());
        assert_eq!(charge_of(&e), None);
    }
    settle(&mut e);
    // The second upgrade is a stat; the capstone is the grant.
    pick(&mut e, 0);
    for _ in 0..12 {
        step(&mut e, smash_held());
        assert_eq!(charge_of(&e), None);
    }
    settle(&mut e);
    pick(&mut e, 0);
    for _ in 0..12 {
        step(&mut e, smash_held());
    }
    assert!(charge_of(&e).is_some());
}

/// (7) Dormant: with the grants named but never taken, every tick's hash is the one of the same
/// expertise with no kind at all: no state, no hash change.
#[test]
fn an_untaken_grant_changes_no_hash() {
    let run = |e: &mut Engine| {
        let mut hashes = Vec::new();
        pick(e, 1);
        for t in 0..300u32 {
            step(
                e,
                Input {
                    main: t % 7 < 3,
                    ..Input::default()
                },
            );
            hashes.push(e.state_hash());
        }
        hashes
    };
    let with_kinds = run(&mut build(test_expertise(true)));
    let without = run(&mut build(test_expertise(false)));
    assert_eq!(with_kinds, without);
}

/// (6) A style picked in the middle of a combo keeps the combo's place.
#[test]
fn a_style_picked_mid_combo_keeps_the_combo_s_place() {
    let mut e = build(test_expertise(true));
    step(&mut e, main_held());
    for _ in 0..4 {
        step(&mut e, idle());
    }
    let before = e.snapshot().players[0].combo.clone();
    pick(&mut e, 0);
    assert_eq!(e.snapshot().players[0].combo, before);
}

/// T5: the views carry the kinds compactly: no attack inside, and nothing when none is named.
#[test]
fn the_views_carry_the_kinds() {
    let e = build(test_expertise(true));
    let snap = e.snapshot();
    let ready =
        serde_json::to_value(snap.players[0].tree_ready.as_ref().expect("a point")).unwrap();
    let charge = json!({ "type": "replace_step", "step": ["main"], "grant": "charge" });
    assert_eq!(ready["kinds"], json!([[charge], [], []]));
    let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
    let grants = &stats["trees"][0]["grants"];
    assert_eq!(grants[0]["offer"], "style");
    assert_eq!(grants[0]["kinds"], json!([charge]));
    assert_eq!(
        grants.as_array().unwrap().len(),
        4,
        "the style's level, then its path"
    );
    assert!(stats["trees"][1].get("grants").is_none());
    let hold = &stats["trees"][2]["grants"][1];
    assert_eq!(hold["offer"], "upgrade");
    assert_eq!(hold["option"], "hold_aa");
    assert_eq!(
        hold["kinds"],
        json!([{ "type": "replace_step", "step": ["main", "main"], "grant": "hold" }])
    );
    // The bundled expertise names none: no key in either view.
    let e = build(Expertise::bundled().clone());
    let snap = e.snapshot();
    let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
    assert!(ready.get("kinds").is_none());
    let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
    for t in stats["trees"].as_array().unwrap() {
        assert!(t.get("grants").is_none());
    }
}

/// (5) A grant replaces: a hold granted on the step where the base has a charge leaves a hold
/// and no charge there.
#[test]
fn a_grant_replaces_the_step_s_charge_with_a_hold() {
    let aaa = |e: &mut Engine| {
        let (mut charged, mut held) = (false, false);
        for _ in 0..2 {
            step(e, main_held());
            for _ in 0..12 {
                step(e, idle());
            }
        }
        for _ in 0..40 {
            step(e, main_held());
            charged |= charge_of(e).is_some();
            held |= hold_of(e);
        }
        (charged, held)
    };
    let mut e = build_with("giant_sword", expertise_for("giant_sword", true));
    assert_eq!(aaa(&mut e), (true, false), "the base AAA charges");
    settle(&mut e);
    pick(&mut e, 0);
    assert_eq!(
        aaa(&mut e),
        (false, true),
        "the grant leaves a hold and no charge"
    );
}
