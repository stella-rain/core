//! The stage field `try_styles` (ADR-SIM-9 points 3, 5, 8 and 9, ADR-REPL-5 point 8): under it
//! every tree point of the level table is a style pick again, recorded as a `TreePick`. The field
//! is for the tutorial mode alone; the stage parser, the version gate and the run builder are
//! tested here.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::pack::Pack;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::stage::{LevelTable, Stage};
use stella_rain_core::validate::{self, Error};
use stella_rain_core::version::Versions;

use crate::common;
use crate::common::equip::{Parts, equipped_in};

use common::*;

const V1_CLEAR: &[u8] = include_bytes!("corpus/v1/stages/clear/stage.json");

fn tutorial_file() -> Value {
    serde_json::from_slice(run_setup::mode_file("tutorial").unwrap()).unwrap()
}

fn parse(v: &Value) -> Result<Stage, Error> {
    validate::parse_and_validate(v.to_string().as_bytes())
}

fn written(stage: &Stage) -> Value {
    serde_json::to_value(stage).unwrap()
}

fn pack_with(mode: &str) -> Pack {
    let mut v: Value = serde_json::from_slice(crate::before::pack_bytes()).unwrap();
    v["modes"][mode]["try_styles"] = true.into();
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the pack loads")
}

fn stage_of(pack: &Pack, mode: &str) -> Value {
    let setup = RunSetup {
        mode: mode.into(),
        ..RunSetup::default()
    };
    written(&run_setup::stage_in(&setup, pack.content()).expect("a stage"))
}

// T1: the field, its version gate and its parse.

#[test]
fn the_field_parses_and_is_written_back() {
    let mut v = tutorial_file();
    v["try_styles"] = true.into();
    let stage = parse(&v).expect("valid at sim_version 2");
    assert_eq!(written(&stage)["try_styles"], true);
}

#[test]
fn a_false_or_absent_field_writes_no_key() {
    // The bundled tutorial names the field, so the claim is made on a copy without it.
    for mode in ["trial", "tutorial"] {
        let mut v: Value = serde_json::from_slice(run_setup::mode_file(mode).unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("try_styles");
        assert!(written(&parse(&v).unwrap()).get("try_styles").is_none());
    }
    let mut v = tutorial_file();
    v["try_styles"] = false.into();
    assert!(written(&parse(&v).unwrap()).get("try_styles").is_none());
}

#[test]
fn only_the_bundled_tutorial_names_the_field() {
    for mode in ["tutorial", "trial"] {
        let v: Value = serde_json::from_slice(run_setup::mode_file(mode).unwrap()).unwrap();
        let named = written(&parse(&v).unwrap()).get("try_styles") == Some(&Value::Bool(true));
        assert_eq!(named, mode == "tutorial", "{mode}");
    }
}

#[test]
fn version_1_refuses_the_field_and_accepts_false() {
    let mut v: Value = serde_json::from_slice(V1_CLEAR).unwrap();
    v["try_styles"] = true.into();
    match parse(&v) {
        Err(Error::Invalid(issues)) => {
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].path, "$.try_styles");
            assert_eq!(issues[0].message, "needs sim_version 2");
        }
        other => panic!("not refused as invalid: {other:?}"),
    }
    v["try_styles"] = false.into();
    parse(&v).expect("false is accepted at any version");
}

#[test]
fn a_non_bool_is_refused_by_the_parse() {
    for bad in [Value::from("yes"), Value::from(1)] {
        let mut v = tutorial_file();
        v["try_styles"] = bad;
        assert!(parse(&v).is_err());
    }
}

// T2: the field is inert outside the tutorial.

#[test]
fn a_trial_file_that_names_the_field_changes_no_run() {
    let pack = pack_with("trial");
    assert!(stage_of(&pack, "trial").get("try_styles").is_none());
    assert!(stage_of(&pack, "time_limit").get("try_styles").is_none());
}

#[test]
fn the_tutorial_file_that_names_the_field_gives_the_tutorial_run_alone() {
    let pack = pack_with("tutorial");
    assert_eq!(stage_of(&pack, "tutorial")["try_styles"], true);
    assert!(stage_of(&pack, "trial").get("try_styles").is_none());
}

#[test]
fn the_bundled_pack_names_the_field_nowhere_outside_the_tutorial() {
    let pack = Pack::bundled();
    assert!(stage_of(pack, "trial").get("try_styles").is_none());
    assert!(stage_of(pack, "time_limit").get("try_styles").is_none());
}

// T4: what the weapon plays follows the style at once, and a re-picked style starts its path over.

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
    let active =
        |r: i32| json!({ "kind": "active", "ticks": 4, "type": "strike", "hit_box": circle(r) });
    json!({ "hold": { "loop": { "inline": [active(7_000)] },
        "release": { "inline": [active(9_000)] }, "max_ticks": 30 } })
}

fn kind(step: &[&str], grant: Value) -> Value {
    json!({ "type": "replace_step", "step": step, "grant": grant })
}

/// Style 0 grants a charge on A, style 1 a hold on A, style 2's first upgrade a hold on AA.
fn test_expertise() -> Expertise {
    let mut file: Value = serde_json::from_slice(crate::before::EXPERTISE).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    for w in file["weapons"].as_array_mut().unwrap() {
        if w["weapon"] != "giant_hammer" {
            continue;
        }
        w["base"] = json!([]);
        for st in w["styles"].as_array_mut().unwrap() {
            st["stats"] = json!([]);
        }
        w["styles"][0]["kinds"] = json!([kind(&["main"], charge_grant())]);
        w["styles"][1]["kinds"] = json!([kind(&["main"], hold_grant())]);
        w["styles"][2]["upgrade_1"] = json!([{ "id": "hold_aa", "effect": "none",
            "kinds": [kind(&["main", "main"], hold_grant())] }]);
    }
    Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the test expertise")
}

fn build(try_styles: bool) -> Engine {
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
    stage.try_styles = try_styles;
    stage.level_table = Some(LevelTable {
        style: 1,
        upgrades: [2, 3],
        capstone: 4,
    });
    let parts = Parts {
        expertise: Some(test_expertise()),
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

fn settle(e: &mut Engine) {
    for _ in 0..80 {
        step(e, idle());
    }
}

fn charging(e: &Engine) -> bool {
    e.snapshot().players[0].charge.is_some()
}

fn holding(e: &Engine) -> bool {
    e.snapshot().players[0].hold.is_some()
}

/// A, then A again held: the second step is the granted one.
fn holds_on_aa(e: &mut Engine) -> bool {
    step(e, main_held());
    for _ in 0..30 {
        step(e, idle());
    }
    let mut held = false;
    for _ in 0..12 {
        step(e, main_held());
        held |= holding(e);
    }
    held
}

#[test]
fn the_grant_follows_the_re_picked_style_at_once() {
    let mut e = build(true);
    pick(&mut e, 0);
    step(&mut e, main_held());
    assert!(charging(&e) && !holding(&e));
    settle(&mut e);
    pick(&mut e, 1);
    let (mut held, mut charged) = (false, false);
    for _ in 0..40 {
        step(&mut e, main_held());
        held |= holding(&e);
        charged |= charging(&e);
    }
    assert!(held && !charged, "the new style holds instead of charging");
}

#[test]
fn a_re_picked_style_starts_its_upgrade_path_over() {
    let mut e = build(true);
    pick(&mut e, 2);
    pick(&mut e, 2);
    assert!(!holds_on_aa(&mut e), "the log is not an upgrade taken");
    // The control: without the field the second pick is the upgrade and the hold is granted.
    let mut e = build(false);
    pick(&mut e, 2);
    pick(&mut e, 0);
    assert!(holds_on_aa(&mut e));
}

#[test]
fn a_switch_while_charging_or_holding_clears_the_state_and_stays_deterministic() {
    let run = |from: u32, to: u32| {
        let mut e = build(true);
        pick(&mut e, from);
        let mut ticks = 0;
        while !(charging(&e) || holding(&e)) {
            step(&mut e, main_held());
            ticks += 1;
            assert!(ticks < 60, "a charge or hold starts");
        }
        for _ in 0..4 {
            step(&mut e, main_held());
        }
        pick(&mut e, to);
        step(&mut e, main_held());
        let cleared = if from == 0 {
            !charging(&e)
        } else {
            !holding(&e)
        };
        assert!(
            cleared,
            "the old charge or hold ends on the tick after the switch"
        );
        (0..120)
            .map(|_| {
                step(&mut e, main_held());
                e.state_hash()
            })
            .collect::<Vec<_>>()
    };
    for (from, to) in [(0, 1), (1, 0)] {
        assert_eq!(run(from, to), run(from, to));
    }
}

// T5: a tutorial run under the field replays, and its savepoint loads.

use crate::run_play::scripted_input;
use stella_rain_core::run_replay::{self, RunRecorder, RunReplay, RunReplayError};

fn tutorial_pack(try_styles: bool) -> Pack {
    let mut v: Value = serde_json::from_slice(crate::before::pack_bytes()).unwrap();
    if try_styles {
        v["modes"]["tutorial"]["try_styles"] = true.into();
    }
    v["modes"]["tutorial"]["level_table"] =
        json!({ "style": 1, "upgrades": [2, 3], "capstone": 4 });
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack")
}

fn tutorial_setup() -> RunSetup {
    RunSetup {
        mode: "tutorial".into(),
        weapon: "giant_hammer".into(),
        seed: 20_261_010,
        ..RunSetup::default()
    }
}

/// Plays `ticks` ticks, taking `TreePick { option: held % 3 }` whenever the engine accepts it;
/// returns how many picks were recorded.
fn play(rec: &mut RunRecorder, from: u32, to: u32) -> u32 {
    let mut picks = 0;
    for t in from..to {
        let held = rec.engine().snapshot().players[0]
            .tree_ready
            .as_ref()
            .map_or(0, |r| u32::from(r.held));
        let pick = EquipAction::TreePick { option: held % 3 };
        if rec.engine().check_action(PlayerIdx::SOLO, pick).is_ok() {
            rec.actions(&[Some(pick), None, None]).expect("recorded");
            picks += 1;
        }
        let input = scripted_input(rec.engine(), 0, t);
        rec.step(&[input, Default::default(), Default::default()])
            .expect("a step");
    }
    picks
}

#[test]
fn a_tutorial_run_with_re_picks_replays_on_its_pack_alone() {
    let pack = tutorial_pack(true);
    let mut rec = RunRecorder::start_with_pack(&tutorial_setup(), &pack).unwrap();
    let picks = play(&mut rec, 0, 6_000);
    assert!(picks >= 4, "four points are taken: {picks}");
    let replay = rec.replay();
    let bytes = replay.to_bytes();
    let verified = run_replay::verify_run_with_pack(&bytes, &pack).expect("verifies");
    assert_eq!(verified.final_hash, rec.engine().state_hash());
    assert!(matches!(
        run_replay::verify_run(&bytes),
        Err(RunReplayError::ContentChanged)
    ));
}

#[test]
fn the_same_driver_on_the_pack_without_the_field_offers_upgrades_and_verifies() {
    let pack = tutorial_pack(false);
    let mut rec = RunRecorder::start_with_pack(&tutorial_setup(), &pack).unwrap();
    let mut second = None;
    for t in 0..6_000 {
        let r = rec.engine().snapshot().players[0].tree_ready.clone();
        if let Some(r) = &r
            && r.level == 2
        {
            second = Some(format!("{:?}", r.kind));
        }
        let held = r.as_ref().map_or(0, |r| u32::from(r.held));
        let pick = EquipAction::TreePick { option: held % 3 };
        if rec.engine().check_action(PlayerIdx::SOLO, pick).is_ok() {
            rec.actions(&[Some(pick), None, None]).unwrap();
        }
        let input = scripted_input(rec.engine(), 0, t);
        rec.step(&[input, Default::default(), Default::default()])
            .unwrap();
    }
    assert_eq!(second.as_deref(), Some("Upgrade"));
    let bytes = rec.replay().to_bytes();
    let verified = run_replay::verify_run_with_pack(&bytes, &pack).unwrap();
    assert_eq!(verified.final_hash, rec.engine().state_hash());
}

#[test]
fn a_savepoint_after_a_re_pick_loads() {
    let pack = tutorial_pack(true);
    let mut rec = RunRecorder::start_with_pack(&tutorial_setup(), &pack).unwrap();
    let mut picks = 0;
    let mut t = 0;
    while picks < 2 {
        picks += play(&mut rec, t, t + 1);
        t += 1;
        assert!(t < 6_000, "two picks are taken");
    }
    let save = rec.save();
    let prefix = RunReplay::from_bytes(&rec.replay().to_bytes()).expect("the prefix");
    let mut resumed = RunRecorder::resume_from_savepoint(&save, &prefix, &pack).expect("loads");
    for _ in 0..600 {
        let input = scripted_input(rec.engine(), 0, t);
        rec.step(&[input, Default::default(), Default::default()])
            .unwrap();
        let input = scripted_input(resumed.engine(), 0, t);
        resumed
            .step(&[input, Default::default(), Default::default()])
            .unwrap();
        t += 1;
    }
    assert_eq!(resumed.engine().state_hash(), rec.engine().state_hash());
}
