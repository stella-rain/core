//! The unlocked styles of the run weapon (ADR-SIM-9 point 8, ADR-SAVE-7 point 10, ADR-REPL-5
//! point 1): the setup lists the styles the player unlocked, the style offer lists only those, and
//! the setup, which the replay stores, is the only place the list lives. Equipment runs on the
//! Hammer preset, with the bundled styles at small thresholds; picks are `TreePick`s.

use serde_json::{Value, json};
use stella_rain_core::engine::{ActionError, Engine};
use stella_rain_core::expertise::Expertise;
use stella_rain_core::id::{ContentId, PlayerIdx};
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_replay::{Command, record_run_with_pack, verify_run_with_pack};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::version::Versions;

use crate::common;
use crate::common::equip::{Parts, equipped_in};

use common::*;

const HAMMER: [&str; 3] = ["crush", "quake", "bulwark"];

/// The bundled expertise with thresholds small enough that the seeded points hold every level.
fn expertise() -> Expertise {
    let mut file: Value = serde_json::from_slice(crate::before::EXPERTISE).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the test expertise")
}

fn ids(list: &[&str]) -> Option<Vec<ContentId>> {
    Some(list.iter().map(|s| ContentId::from(*s)).collect())
}

fn build(unlocked: Option<Vec<ContentId>>) -> Engine {
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
        json!({ "preset": { "id": "giant_hammer" } }),
    );
    stage.sim_version = 2;
    let parts = Parts {
        expertise: Some(expertise()),
        expertise_points: 550,
        unlocked_styles: unlocked,
        ..Parts::default()
    };
    let file = json!({ "rarities": common::rarities(), "rewards": [
        { "id": "reach", "rarity": "common", "effect": "range_pct", "amount": 30, "limit": 2 }
    ] });
    let mut e = equipped_in(&stage, file, parts, &[], 1);
    e.step(&[idle(), Default::default(), Default::default()]);
    e
}

fn pick(e: &mut Engine, option: u32) -> Result<(), ActionError> {
    e.act(PlayerIdx::SOLO, EquipAction::TreePick { option })
}

fn options(e: &Engine) -> Option<Vec<String>> {
    e.snapshot().players[0]
        .tree_ready
        .as_ref()
        .map(|r| r.options.iter().map(|o| o.as_str().to_owned()).collect())
}

fn active(e: &Engine) -> Option<String> {
    let snap = e.snapshot();
    let stats = snap.players[0].weapon_stats.as_ref().unwrap();
    stats.active.as_ref().map(|a| a.as_str().to_owned())
}

fn strs(list: &[&str]) -> Option<Vec<String>> {
    Some(list.iter().map(|s| (*s).to_owned()).collect())
}

fn hashes(e: &mut Engine, ticks: u32) -> Vec<u64> {
    (0..ticks)
        .map(|_| {
            e.step(&[idle(), Default::default(), Default::default()]);
            e.state_hash()
        })
        .collect()
}

/// (1) No field: every style, in the file's order, and the pick is its place.
#[test]
fn an_absent_field_offers_every_style() {
    let mut e = build(None);
    assert_eq!(options(&e), strs(&HAMMER));
    pick(&mut e, 2).unwrap();
    assert_eq!(active(&e).as_deref(), Some("bulwark"));
}

/// (2) A listed subset is offered in the file's order, and the pick stores the style's place in
/// the weapon's styles, not the offered index.
#[test]
fn a_subset_is_offered_in_file_order_and_the_pick_stores_the_place() {
    let mut e = build(ids(&["bulwark", "crush"]));
    assert_eq!(options(&e), strs(&["crush", "bulwark"]));
    assert_eq!(
        pick(&mut e, 2),
        Err(ActionError::NoTreeOption {
            option: 2,
            options: 2
        })
    );
    pick(&mut e, 1).unwrap();
    assert_eq!(active(&e).as_deref(), Some("bulwark"));
    let snap = e.snapshot();
    let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
    assert_eq!(stats["trees"][2]["style"], "bulwark");
}

/// (3) One style: its path is offered from that style's own lists, and every pick takes.
#[test]
fn one_style_leaves_its_path_to_take() {
    let mut e = build(ids(&["quake"]));
    assert_eq!(options(&e), strs(&["quake"]));
    pick(&mut e, 0).unwrap();
    assert_eq!(active(&e).as_deref(), Some("quake"));
    let file = expertise();
    let quake = &file.weapon(&"giant_hammer".into()).unwrap().styles[1];
    let first: Vec<String> = quake
        .upgrade_1
        .iter()
        .map(|o| o.id.as_str().to_owned())
        .collect();
    assert_eq!(options(&e), Some(first));
    for _ in 0..3 {
        pick(&mut e, 0).unwrap();
    }
    assert_eq!(options(&e), None);
}

/// (4) An empty list: no style pick is offered and the run plays the plain base, hash for hash
/// like a run that never picks.
#[test]
fn an_empty_list_offers_no_pick_and_plays_the_plain_base() {
    let mut none = build(ids(&[]));
    assert_eq!(options(&none), None);
    assert_eq!(pick(&mut none, 0), Err(ActionError::NoTreePoint));
    assert_eq!(active(&none), None);
    let mut absent = build(None);
    assert_eq!(hashes(&mut none, 120), hashes(&mut absent, 120));
    assert_eq!(options(&none), None);
    assert_eq!(pick(&mut none, 0), Err(ActionError::NoTreePoint));
    assert_eq!(active(&none), None);
}

/// (5) Every style listed equals the field absent.
#[test]
fn every_style_listed_equals_absent() {
    let mut all = build(ids(&["bulwark", "crush", "quake"]));
    let mut absent = build(None);
    assert_eq!(options(&all), options(&absent));
    assert_eq!(hashes(&mut all, 60), hashes(&mut absent, 60));
    pick(&mut all, 1).unwrap();
    pick(&mut absent, 1).unwrap();
    assert_eq!(hashes(&mut all, 60), hashes(&mut absent, 60));
}

/// (6) A recorded run with a list replays: the list lives in the setup the replay stores.
#[test]
fn a_run_with_a_list_records_and_verifies() {
    let setup = RunSetup {
        weapon: "giant_hammer".into(),
        seed: 11,
        ..RunSetup::default()
    };
    let mut v = serde_json::to_value(&setup).unwrap();
    v["unlocked_styles"] = json!(["bulwark", "crush"]);
    let setup: RunSetup = serde_json::from_value(v).unwrap();
    let steps: Vec<Command> = (0..600)
        .map(|t| {
            Command::Step(Input {
                move_x: if (t / 40) % 2 == 0 { 127 } else { -127 },
                main: (t / 45) % 3 == 1,
                ..Input::default()
            })
        })
        .collect();
    let replay = record_run_with_pack(&setup, crate::before::pack(), &steps).unwrap();
    let stored = serde_json::to_value(&replay.setup).unwrap();
    assert_eq!(stored["unlocked_styles"], json!(["bulwark", "crush"]));
    let verified = verify_run_with_pack(&replay.to_bytes(), crate::before::pack()).unwrap();
    let again = verify_run_with_pack(
        &record_run_with_pack(&setup, crate::before::pack(), &steps)
            .unwrap()
            .to_bytes(),
        crate::before::pack(),
    )
    .unwrap();
    assert_eq!(verified.final_hash, again.final_hash);
}
