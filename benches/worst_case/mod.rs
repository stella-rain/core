//! The worst-case tick (ADR-020, ADR-021). Shared by `benches/worst_tick.rs`, which counts its
//! instructions, and `tests/worst_case.rs`, which checks that it really reaches every dynamic
//! cap: a benchmark of a tick that does not would pass without measuring the worst case.
#![allow(dead_code)]

#[path = "../../tests/common/mod.rs"]
mod common;

use common::START_X;
use common::on_hit::{self, NEAR_Y};
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::runes::{MAX_DOT_STACKS, Runes};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::limits::MAX_ASSIST_RANGE;

/// Ticks to run before the tick that is measured, so that the caps are reached and held, the
/// statuses are at their stack cap, and the main character is charged.
pub const WARMUP_TICKS: u32 = 420;

/// The tick the 500 enemies start to arrive: after the main character has taken every rune and
/// Rune Haste from the first eight enemies' stars (`take_runes`).
pub const WAVES_AT: u32 = 150;

/// The tick the main character sets off for the enemies, and the ticks it walks left and up
/// before it stands about 36,000 units from them with every enemy inside its step.
const WALK_FROM: u32 = 100;
const WALK_LEFT_TICKS: u32 = 40;
const WALK_UP_TICKS: u32 = 140;

/// From this tick the main character taps its Main weapon every other tick: its step lands on
/// every enemy, and each hit puts a stack of each status on it.
const TAP_FROM: u32 = WALK_FROM + WALK_LEFT_TICKS + WALK_UP_TICKS + 4;

/// Ticks the main character charges before it lets go on the measured tick.
const CHARGE_TICKS: u32 = 24;

/// The Rune Haste reward: the most one gives (`MAX_RUNE_HASTE_PCT`), so that the picks reach
/// the cap.
const HASTE: &str = "haste";

/// Where the enemies stand.
pub const ENEMIES_AT: (i32, i32) = (60_000, 20_480);

/// 500 enemies (`MAX_ENEMIES_ALIVE`) stand in one place, five waves of 100 that spawn one enemy
/// a tick each.
///
/// The push-apart (ADR-058) parts a fresh stack on every tick: on each tick it does not start
/// its attack, every enemy orders itself back to the point in one tick (`move_to`, which a
/// crowded enemy still carries out). So when the pass runs, all stand on the point but the 15
/// or 20 that started their attack the tick before, which are within 8 pixels of it. Every
/// enemy reads 32 others from the grid and counts 6. It also means every enemy fires a rule on
/// every tick.
///
/// Every enemy starts the same attack every 30 ticks: 100 rounds of 100 rounds of
/// one bullet. Each start is refused once 256 attacks are going (`MAX_ATTACK_TASKS`), and the
/// oldest attacks use up the 4,096 node steps of the tick (`MAX_EMITTER_STEPS_PER_TICK`) and
/// ask for far more than 200 bullets (`MAX_BULLET_SPAWNS_PER_TICK`).
///
/// The bullets fly straight down at 8,192 (32 pixels a tick) from where their enemy stands:
/// y = 20,480, or up to 12 pixels (3,072) from it once the enemy has been pushed. The field is
/// 163,840 high and its margin 16,384, so from 17,408 to 23,552 a bullet is gone on its 20th
/// move, wherever in that range it started: 19 batches of 200
/// are in the air when a tick starts, the tick makes the 20th, and 4,000 are alive
/// (`MAX_BULLETS_ALIVE`) until the 200 that are about to leave go.
///
/// The main character is a run that holds every rune (ADR-099): before the enemies arrive it
/// kills eight small ones in front of it and takes the seven runes and Rune Haste of 300 percent
/// (`MAX_RUNE_HASTE_PCT`) from their stars, as a run does, through `Engine::new_with`. It then
/// walks to the left of the enemies, about 36,000 units from them (out of the 128-pixel arcs of their
/// attacks, and of their bullets' column), where its Main weapon's step, a circle of 256
/// pixels, lands on all 500 of them every tick: each hit puts a stack of bleed, poison and burn
/// on the enemy, to the stack cap (`MAX_DOT_STACKS`), and fires every on-hit rune that is ready.
/// The statuses last 600 ticks and tick every 4, which full Rune Haste makes every tick; so on
/// every tick 1,500 statuses deal damage, and each on-hit rune, whose cooldown the haste cuts to
/// a quarter, fires again as soon as its attack is over. The step charges for the 24 ticks
/// before the measured tick and lets go on it, and the release's bullet finds 256 attacks going
/// and spawns nothing (ADR-070). From the measured tick aim assist and auto mode are on, with
/// the longest assist range, so every tick looks through all 500 enemies for a target
/// (ADR-072, ADR-073).
///
/// Hit boxes (ADR-055): the enemies' attack is one active phase of 30 ticks, so every enemy is
/// always in it, with a 128-pixel arc facing the main character, out of its reach. The four
/// companions (`MAX_COMPANIONS`) stand 78 pixels to the right of the enemies, behind every
/// arc, so each arc tests each of them against its angle and both edges and misses. They cleave
/// all 500 enemies every tick with an arc of their own (`rehit_ticks` 1, 1 damage a hit).
pub fn stage() -> Stage {
    let burst = json!({
        "type": "repeat", "count": 100, "interval_ticks": 0, "body": [{
            "type": "repeat", "count": 100, "interval_ticks": 0, "body": [{
                "type": "fire", "speed": 8192,
                "direction": { "type": "absolute", "angle": 0 }
            }]
        }]
    });
    let swing = json!([{ "kind": "active", "ticks": 30, "spawn": [burst],
                         "hit_box": { "type": "arc", "radius": 32_768,
                                      "half_angle": 60 * 256 } }]);
    let attack = common::rule(
        common::always(),
        json!("player"),
        json!({ "type": "attack", "attack": { "inline": swing } }),
        30,
    );
    let (x, y) = ENEMIES_AT;
    let back = common::rule(
        common::always(),
        json!("self"),
        json!({ "type": "move_to", "to": { "x": x, "y": y }, "ticks": 1 }),
        0,
    );
    let enemy = common::enemy(100_000, common::hold(x, y), json!([attack, back]));
    let mut waves = vec![common::wave(
        1,
        PICKS,
        6,
        common::enemy(1, common::hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    waves.extend((0..5).map(|_| common::wave(WAVES_AT, 100, 1, enemy.clone(), x, y)));
    let cleave = json!([{ "kind": "active", "ticks": 30, "damage": 1, "rehit_ticks": 1,
                          "hit_box": { "type": "arc", "offset": { "forward": 10_000 },
                                       "radius": 65_536, "half_angle": 90 * 256 } }]);
    let guard = common::companion(
        50,
        common::hold(x + 20_000, y),
        json!([common::rule(
            common::always(),
            json!("nearest_enemy"),
            json!({ "type": "attack", "attack": { "inline": cleave } }),
            0
        )]),
    );
    // The Main weapon's step: a circle of 256 pixels round the main character, active for two
    // ticks and hitting every tick, so that a tap every other tick lands on every enemy in it,
    // every tick. Held, the charge lets go with the same step and a bullet, which finds 256
    // attacks going and is refused.
    let step = json!([{ "kind": "active", "ticks": 2, "damage": 1, "rehit_ticks": 1,
                        "hit_box": { "type": "circle", "radius": 65_536 } }]);
    let released = json!({ "inline": [{ "kind": "active", "ticks": 2, "damage": 1,
        "rehit_ticks": 1, "hit_box": { "type": "circle", "radius": 65_536 },
        "spawn": [{ "type": "fire", "speed": 2048,
                    "direction": { "type": "aimed", "offset": 0 } }] }] });
    let charged = json!({ "attack": { "inline": step }, "charge": {
        "levels": [{ "at_ticks": CHARGE_TICKS, "attack": released }], "max_ticks": 600 } });
    let mut main = common::weapon_combo(json!({ "main": charged }));
    main["inline"]["assist_range"] = json!(MAX_ASSIST_RANGE);
    let mut stage = common::armed(
        common::stage_full(18_000, Some(common::far_boss()), waves, vec![guard; 4]),
        main,
    );
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 0, "banishes": 0,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

/// The first enemies the main character kills, for the stars that give it every rune and Rune
/// Haste: seven runes, then the Rune Haste reward.
const PICKS: u16 = 8;

/// The run's content: the random rune and Rune Haste of 300 percent (`MAX_RUNE_HASTE_PCT`) to
/// pick from, the bundled on-hit runes, and damage-over-time runes that stack to the cap
/// (`MAX_DOT_STACKS`), last for 600 ticks and tick every 4, which full Rune Haste makes every
/// tick.
pub fn content() -> RunContent {
    let haste = json!({ "id": HASTE, "rarity": "common", "effect": "rune_haste",
                        "amount": 300, "limit": 2, "title": "Haste", "description": "" });
    let mut file: Value = serde_json::from_slice(Runes::BUNDLED).expect("the bundled runes");
    let dots: Vec<Value> = ["bleed", "poison", "burn"]
        .into_iter()
        .map(|r| on_hit::dot(r, 4, 600, 1, MAX_DOT_STACKS))
        .collect();
    file["damage_over_time"] = json!(dots);
    let runes = Runes::parse(file.to_string().as_bytes()).expect("the worst-case runes");
    on_hit::content(vec![on_hit::random_rune(), haste], None, runes)
}

/// The input of tick `tick` (from 1) once the runes are taken: the main character walks left
/// and up to stand about 36,000 units from the enemies, taps its Main weapon every other tick, and
/// from 24 ticks before the measured tick holds it, charging, to let go on the measured tick;
/// from then on it taps again, with aim assist and auto mode on.
pub fn input(tick: u32) -> Input {
    let after_warmup = tick > WARMUP_TICKS;
    let walked = tick - WALK_FROM.min(tick);
    let charging = tick > WARMUP_TICKS - CHARGE_TICKS && !after_warmup;
    let tapping = tick >= TAP_FROM && !charging && tick % 2 == 0;
    Input {
        main: charging || tapping,
        assist: after_warmup,
        auto: after_warmup,
        ..if walked == 0 {
            common::idle()
        } else if walked <= WALK_LEFT_TICKS {
            common::moving(-127, -127)
        } else if walked <= WALK_LEFT_TICKS + WALK_UP_TICKS {
            common::moving(0, -127)
        } else {
            common::idle()
        }
    }
}

/// An engine `WARMUP_TICKS` ticks into the run: the next `step` is the worst-case tick. The
/// main character takes every rune and Rune Haste first, from the stars of the eight enemies
/// that stand in front of it, as a run does (`on_hit::take_all`).
pub fn warmed_up() -> Engine {
    let mut engine = on_hit::with(&stage(), &content());
    on_hit::take_all(&mut engine);
    on_hit::take(&mut engine, HASTE);
    assert!(
        engine.tick() < WALK_FROM,
        "the runes were taken by tick {}",
        engine.tick()
    );
    for tick in engine.tick() + 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}
