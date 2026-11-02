//! The expertise gauge (ADR-078 §2, §3): one gauge for both weapons. A hit and a kill by an A
//! or a B step of the combo add the enemy's points to it; other damage gives none. Content
//! thresholds take it through levels 1 to 10, then each level needs the one before's points
//! times the growth factor, saturating at the content maximum. The weapons here are inline,
//! with no expertise content, so a level opens no offer (`expertise_offers.rs` tests those).
//! Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{content_sha256, content_sha256_with};
use stella_rain_core::snapshot::ExpertiseView;
use stella_rain_core::stage::Stage;
use stella_rain_core::version::Versions;

use crate::common;

use common::*;

/// 32 pixels above the main character, where both steps below land.
const NEAR_Y: i32 = START_Y - 8_192;

/// A Main weapon whose A step (`main`) is a line 64 pixels long straight ahead, and whose B step
/// (`smash`) is a hit box of 8 pixels 32 pixels ahead: 10 damage each, both from the root, so
/// each press starts its own step.
fn weapon() -> Value {
    let swing = json!({ "inline": [
        { "kind": "active", "ticks": 1, "damage": 10,
          "hit_box": { "type": "circle", "offset": { "forward": 8_192 }, "radius": 2_048 } },
        { "kind": "recovery", "ticks": 4 }
    ] });
    let line = json!({ "inline": [
        { "kind": "active", "ticks": 1, "damage": 10,
          "hit_box": { "type": "line", "length": 16_384, "width": 2_048 } },
        { "kind": "recovery", "ticks": 4 }
    ] });
    weapon_combo(json!({
        "main": step(line),
        "smash": step(swing),
    }))
}

/// One still enemy of `hp` at `NEAR_Y` that never attacks, giving `hit` and `kill` points.
fn target(hp: u32, hit: u32, kill: u32) -> Value {
    let mut w = wave(
        1,
        1,
        0,
        enemy(hp, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    );
    w["expertise"] = json!({ "hit": hit, "kill": kill });
    w
}

fn stage(waves: Vec<Value>, companions: Vec<Value>) -> Stage {
    let stage = stage_full(18_000, Some(far_boss()), waves, companions);
    armed(stage, weapon())
}

/// The bundled expertise content with `thresholds`, `growth_pct` and `max_points`.
fn content(thresholds: [u32; 10], growth_pct: u16, max_points: u32) -> RunContent {
    let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
    file["thresholds"] = json!(thresholds);
    file["growth_pct"] = json!(growth_pct);
    file["max_points"] = json!(max_points);
    let expertise = Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test expertise validates");
    RunContent::bundled().clone().with_expertise(expertise)
}

fn with(stage: &Stage, content: &RunContent) -> Engine {
    Engine::new_with(stage, content).expect("the test stage is valid")
}

fn gauge(e: &Engine) -> ExpertiseView {
    e.snapshot()
        .expertise
        .expect("a stage that gives points has the gauge")
}

fn view(level: u32, points: u32, next: u32) -> ExpertiseView {
    ExpertiseView {
        level,
        points,
        next,
        ..ExpertiseView::default()
    }
}

/// Waits for the combo step in progress to end, presses `smash` (B) or `main` (A) for a tick,
/// then waits 20 ticks for its hit to land.
fn press(e: &mut Engine, smash: bool) {
    for _ in 0..60 {
        if e.snapshot().players[0].attacks.iter().all(|a| a.slot != 0) {
            break;
        }
        e.step(idle());
    }
    e.step(Input {
        main: !smash,
        smash,
        ..Input::default()
    });
    for _ in 0..20 {
        e.step(idle());
    }
}

/// Level 1 needs 10 points; levels 2 to 10 more.
const TABLE: [u32; 10] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];

#[test]
fn hits_by_a_and_b_steps_fill_the_one_gauge_to_a_level() {
    let mut e = with(
        &stage(vec![target(1_000, 4, 50)], vec![]),
        &content(TABLE, 150, 1_000),
    );
    e.step(idle());
    assert_eq!(gauge(&e), view(0, 0, 10), "every run starts at level 0");

    press(&mut e, false);
    assert_eq!(gauge(&e), view(0, 4, 10), "the A step's line hit");
    press(&mut e, true);
    assert_eq!(
        gauge(&e),
        view(0, 8, 10),
        "the B step's hit box, into the same gauge"
    );
    press(&mut e, false);
    assert_eq!(
        gauge(&e),
        view(1, 2, 20),
        "12 points: level 1 and 2 toward level 2"
    );
    press(&mut e, true);
    assert_eq!(gauge(&e), view(1, 6, 20));
}

#[test]
fn a_kill_adds_its_points_to_the_hit_s() {
    let mut e = with(
        &stage(vec![target(10, 4, 50)], vec![]),
        &content(TABLE, 150, 1_000),
    );
    press(&mut e, true);
    // 54 points: level 1 (10), level 2 (20), and 24 toward level 3's 30.
    assert_eq!(gauge(&e), view(2, 24, 30));
}

#[test]
fn a_companion_s_hits_give_no_points() {
    let strikes = json!([{ "when": always(), "target": "nearest_enemy", "cooldown_ticks": 0,
        "do": { "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 1,
              "hit_box": { "type": "circle", "radius": 16_384 } },
            { "kind": "recovery", "ticks": 4 }
        ] } } }]);
    let fighter = companion(50, hold(START_X + 4_096, START_Y), strikes);
    let mut e = with(
        &stage(vec![target(1_000, 4, 50)], vec![fighter]),
        &content(TABLE, 150, 1_000),
    );
    for _ in 0..30 {
        e.step(idle());
    }
    let hp = e.snapshot().entities.iter().map(|v| v.hp).min().unwrap();
    assert!(hp < 1_000, "the companion hit the enemy");
    assert_eq!(gauge(&e), view(0, 0, 10));
}

#[test]
fn levels_go_past_10_by_the_growth_factor_and_saturate_at_the_maximum() {
    // Levels 1 to 10 need 55 points in all. Then 150 percent of the level before, rounded
    // down: 15 for level 11, 22 for 12, then 33 held to the maximum of 30 for 13 and every
    // level after. One kill of 159 points: 55 + 15 + 22 + 30 + 30 = 152 reach level 14, and 7
    // are left toward level 15's 30.
    let table = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let mut e = with(
        &stage(vec![target(10, 0, 159)], vec![]),
        &content(table, 150, 30),
    );
    press(&mut e, true);
    assert_eq!(gauge(&e), view(14, 7, 30));
}

#[test]
fn the_growth_from_level_10_is_its_own_threshold_times_the_factor() {
    // 55 + 15 = 70 points reach level 11 exactly; level 12 needs 22.
    let table = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let mut e = with(
        &stage(vec![target(10, 0, 70)], vec![]),
        &content(table, 150, 30),
    );
    press(&mut e, true);
    assert_eq!(gauge(&e), view(11, 0, 22));
}

#[test]
fn a_saturated_gauge_takes_many_levels_in_one_tick() {
    // The table's 55 points, then 400 percent of the level before held to 100 points a level.
    let table = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let mut e = with(
        &stage(vec![target(10, 0, 100_000)], vec![]),
        &content(table, 400, 100),
    );
    press(&mut e, true);
    // Level 11 needs 40, 12 needs 100 (160 held to 100): 55 + 40 = 95, then 99_905 = 999 x 100
    // + 5.
    assert_eq!(gauge(&e), view(1_010, 5, 100));
}

#[test]
fn the_gauge_is_in_the_state_hash() {
    let content = content(TABLE, 150, 1_000);
    let mut one = with(&stage(vec![target(1_000, 1, 0)], vec![]), &content);
    let mut two = with(&stage(vec![target(1_000, 2, 0)], vec![]), &content);
    one.step(idle());
    two.step(idle());
    assert_eq!(one.state_hash(), two.state_hash(), "nothing hit yet");
    press(&mut one, true);
    press(&mut two, true);
    assert_ne!(gauge(&one), gauge(&two));
    assert_ne!(one.state_hash(), two.state_hash());
}

#[test]
fn a_stage_that_gives_no_points_has_no_gauge() {
    let mut plain = target(1_000, 0, 0);
    plain.as_object_mut().unwrap().remove("expertise");
    let mut e = with(&stage(vec![plain], vec![]), RunContent::bundled());
    press(&mut e, true);
    assert_eq!(e.snapshot().expertise, None);
}

#[test]
fn a_run_s_content_hash_covers_the_expertise() {
    assert_eq!(
        content_sha256_with("trial", &content(TABLE, 115, 50_000)),
        content_sha256_with("trial", &content(TABLE, 115, 50_000)),
    );
    assert_ne!(
        content_sha256_with("trial", &content(TABLE, 115, 50_000)),
        content_sha256("trial")
    );
}
