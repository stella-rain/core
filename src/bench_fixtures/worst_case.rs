//! The worst-case tick (ADR-020, ADR-021). Shared by `benches/worst_tick.rs`, which counts its
//! instructions, and `tests/worst_case.rs`, which checks that it really reaches every dynamic
//! cap: a benchmark of a tick that does not would pass without measuring the worst case.

use super::common::on_hit::{self, NEAR_Y};
use super::common::{self, START_X};
use crate::engine::Engine;
use crate::input::Input;
use crate::run_content::RunContent;
use crate::runes::{MAX_DOT_STACKS, Runes};
use crate::stage::Stage;
use crate::validate::limits::{MAX_ASSIST_RANGE, MAX_ELITES_ALIVE};
use serde_json::{Value, json};

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

/// The Rune Haste reward: the most one gives (`DEFAULT_MAX_RUNE_HASTE_PCT`), so that the picks reach
/// the cap.
const HASTE: &str = "haste";

/// Where the enemies stand.
pub const ENEMIES_AT: (i32, i32) = (60_000, 20_480);

/// 500 enemies (`MAX_ENEMIES_ALIVE`) stand in one place: four shooters, then 496 more in five
/// streams of waves that spawn one enemy a tick each, 32 of them elites (`MAX_ELITES_ALIVE`)
/// (core BR1b: nothing here fires a projectile).
///
/// The push-apart (ADR-058) parts a fresh stack on every tick: on each tick it does not start
/// its attack, every enemy orders itself back to the point in one tick (`move_to`, which a
/// crowded enemy still carries out). So when the pass runs, all stand on the point but the 15
/// or 20 that started their attack the tick before, which are within 8 pixels of it. Every
/// enemy reads 32 others from the grid and counts 6. It also means every enemy fires a rule on
/// every tick.
///
/// Every crowd enemy starts the same attack every 30 ticks: an active phase of 30 ticks with a
/// hit box that spawns a wait of 29 ticks, which holds an attack task. Each start is refused
/// once 256 attacks are going (`MAX_ATTACK_TASKS`).
///
/// The four shooters (`MAX_RANGED_ATTACKERS`) start first, with a `ranged` run each: one
/// wind-up tick that sends a ring of eight orbs (`MAX_ORBS_PER_OWNER`), then an active phase
/// longer than the case, so each holds its token. The 32 orbs (`MAX_ORBS_ALIVE`) fly slowly
/// for 600 ticks and burst into nothing. No other projectile is alive or made.
///
/// The main character is a run that holds every rune (ADR-099): before the enemies arrive it
/// kills eight small ones in front of it and takes the seven runes and Rune Haste of 300 percent
/// (`DEFAULT_MAX_RUNE_HASTE_PCT`) from their stars, as a run does, through `Engine::new_with`. It then
/// walks to the left of the enemies, about 36,000 units from them (out of the 128-pixel arcs of their
/// attacks), where its Main weapon's step, a circle of 256
/// pixels, lands on all 500 of them every tick: each hit puts a stack of bleed, poison and burn
/// on the enemy, to the stack cap (`MAX_DOT_STACKS`), and fires every on-hit rune that is ready.
/// The statuses last 600 ticks and tick every 4, which full Rune Haste makes every tick; so on
/// every tick 1,500 statuses deal damage, and each on-hit rune, whose cooldown the haste cuts to
/// a quarter, fires again as soon as its attack is over. The step charges for the 24 ticks
/// before the measured tick and lets go on it. From the measured tick aim assist and auto mode are on, with
/// the longest assist range, so every tick looks through all 500 enemies for a target
/// (ADR-072, ADR-073).
///
/// Hit boxes (ADR-055): the enemies' attack is one active phase of 30 ticks, so every enemy is
/// always in it, with a 128-pixel arc facing the main character, out of its reach. The four
/// companions (`MAX_COMPANIONS`) stand 78 pixels to the right of the enemies, behind every
/// arc, so each arc tests each of them against its angle and both edges and misses. They cleave
/// all 500 enemies every tick with an arc of their own (`rehit_ticks` 1, 1 damage a hit).
///
/// Zones (core Z1, `MAX_ZONES_ALIVE`): on the first tick the boss places 112 and each companion 4
/// (16 friendly), all active for 600 ticks, so 128 are alive and active on every measured tick.
/// The boss's are enemy zones at the map's corner, out of reach of everyone: each is tested
/// against the main character and the four companions and misses. The companions' are circles of
/// 100 pixels round the companion, which cover all 500 enemies: each of the 16 is looked up
/// through the enemy grid and tests, on every tick, the 500 enemies it has hit already (each
/// once, long before the measured tick). Later attempts to place more are dropped at the cap.
pub fn stage() -> Stage {
    let swing = json!([{ "kind": "active", "ticks": 30,
                         "spawn": [{ "type": "wait", "ticks": 29 }],
                         "hit_box": { "type": "arc", "radius": 32_768,
                                      "half_angle": 60 * 256 } }]);
    let attack = common::rule(
        common::always(),
        json!("player"),
        json!({ "type": "attack", "attack": { "inline": swing } }),
        30,
    );
    // A shooter's run: one wind-up tick that sends a ring of eight orbs, then an active phase
    // that outlasts the measured ticks, so it holds a ranged token (`MAX_RANGED_ATTACKERS`) and
    // an attack task for all of them. The orbs fly 32 units a tick, aimed at the main character
    // 36,000 units away, and last 600 ticks: they stay alive, short of it.
    let orb = json!({ "type": "orb", "speed": 32, "ticks": 600, "breakable": false,
                      "direction": { "type": "aimed", "offset": 0 }, "burst": [] });
    let shot = json!([
        { "kind": "wind_up", "ticks": 1,
          "spawn": [{ "type": "ring", "count": 8, "body": [orb] }] },
        { "kind": "active", "ticks": 600, "hit_box": { "type": "arc", "radius": 32_768,
                                                        "half_angle": 60 * 256 } },
    ]);
    let mut volley = common::rule(
        common::always(),
        json!("player"),
        json!({ "type": "attack", "attack": { "inline": shot } }),
        0,
    );
    volley["do"]["ranged"] = json!(true);
    let (x, y) = ENEMIES_AT;
    let back = common::rule(
        common::always(),
        json!("self"),
        json!({ "type": "move_to", "to": { "x": x, "y": y }, "ticks": 1 }),
        0,
    );
    let enemy = common::enemy(100_000, common::hold(x, y), json!([attack, back.clone()]));
    let shooter = common::enemy(100_000, common::hold(x, y), json!([volley, back]));
    let mut waves = vec![common::wave(
        1,
        PICKS,
        6,
        common::enemy(1, common::hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    // The shooters arrive first, so that their runs take their tokens and attack tasks before
    // the crowd's swings fill the 256; then 496 enemies in five streams of waves.
    waves.push(common::wave(WAVES_AT, SHOOTERS, 1, shooter, x, y));
    // 32 of the crowd are elites (`MAX_ELITES_ALIVE`): the crowd's own enemy with an elite's
    // mark, in three kinds, so that their rules and every other cap are as they were. Each of
    // the first three streams of 100 starts with its share of elites and goes on with the
    // crowd, so five enemies still arrive a tick and the stack is parted as before.
    for (i, base) in ELITE_BASES.iter().enumerate() {
        let mut elite = enemy.clone();
        elite["base"] = json!(base);
        elite["elite"] = json!({ "range": ELITE_RANGE });
        let share = MAX_ELITES_ALIVE / 3 + u16::from(i < usize::from(MAX_ELITES_ALIVE % 3));
        waves.push(common::wave(WAVES_AT + 4, share, 1, elite, x, y));
        let after = WAVES_AT + 4 + u32::from(share);
        waves.push(common::wave(after, 100 - share, 1, enemy.clone(), x, y));
    }
    waves.push(common::wave(WAVES_AT + 4, 100, 1, enemy.clone(), x, y));
    waves.push(common::wave(WAVES_AT + 4, 100 - SHOOTERS, 1, enemy, x, y));
    let zone = |radius: i32| {
        json!({ "type": "zone", "shape": { "type": "circle", "radius": radius },
                "wind_up_ticks": 0, "active_ticks": 600 })
    };
    let cleave = json!([{ "kind": "active", "ticks": 30, "damage": 1, "rehit_ticks": 1,
                          "hit_box": { "type": "arc", "offset": { "forward": 10_000 },
                                       "radius": 65_536, "half_angle": 90 * 256 },
                          "spawn": [{ "type": "ring", "count": 4, "body": [zone(25_600)] }] }]);
    let corner = json!([{ "kind": "active", "ticks": 1, "spawn": [{
        "type": "repeat", "count": 2, "interval_ticks": 0,
        "body": [{ "type": "ring", "count": 56, "body": [zone(2_048)] }]
    }] }]);
    let mut boss = common::far_boss();
    boss["phases"] = json!([{ "timeline": [
        { "type": "attack", "attack": { "inline": corner } },
    ] }]);
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
    // every tick. Held, the charge lets go with the same step.
    let step = json!([{ "kind": "active", "ticks": 2, "damage": 1, "rehit_ticks": 1,
                        "hit_box": { "type": "circle", "radius": 65_536 } }]);
    let released = json!({ "inline": [{ "kind": "active", "ticks": 2, "damage": 1,
        "rehit_ticks": 1, "hit_box": { "type": "circle", "radius": 65_536 } }] });
    let charged = json!({ "attack": { "inline": step }, "charge": {
        "levels": [{ "at_ticks": CHARGE_TICKS, "attack": released }], "max_ticks": 600 } });
    let mut main = common::weapon_combo(json!({ "main": charged }));
    main["inline"]["assist_range"] = json!(MAX_ASSIST_RANGE);
    let mut stage = common::armed(
        common::stage_full(18_000, Some(boss), waves, vec![guard; 4]),
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

/// The enemies that send orbs: `MAX_RANGED_ATTACKERS`, each with its eight (`MAX_ORBS_PER_OWNER`),
/// 32 in all (`MAX_ORBS_ALIVE`).
const SHOOTERS: u16 = 4;

/// The sprites the 32 elites are drawn as, a share each.
const ELITE_BASES: [&str; 3] = ["bat", "fairy_a", "horns_2"];

/// How near the main character must come for an elite's rules to fire: the bundled elites'
/// (a placeholder; the main character stands 36,000 units away, so they fire on every tick).
const ELITE_RANGE: i32 = 51_200;

/// The first enemies the main character kills, for the stars that give it every rune and Rune
/// Haste: seven runes, then the Rune Haste reward.
const PICKS: u16 = 8;

/// The run's content: the random rune and Rune Haste of 300 percent (`DEFAULT_MAX_RUNE_HASTE_PCT`) to
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
