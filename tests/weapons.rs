//! The Main and Sub weapons (ADR-050, core#83): each attacks on its own, on its own cooldown,
//! along the facing or at the enemy its targeting picks. Small stages whose every tick can be
//! worked out by hand: the main character stands at (46080, 147456), facing up the field.

mod common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::presets;
use stella_rain_core::registry::{self, Kind};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::validate_weapon_preset;

// --- Builders --------------------------------------------------------------------------------

fn fire(speed: i32, direction: Value) -> Value {
    json!({ "type": "fire", "speed": speed, "direction": direction })
}

fn aimed() -> Value {
    json!({ "type": "aimed", "offset": 0 })
}

/// An attack that fires one bullet at `speed` on the tick it starts.
fn one_bullet(speed: i32) -> Value {
    json!({ "inline": spawned(json!([fire(speed, aimed())])) })
}

/// An enemy that stands at `(x, y)` from the main character and does nothing.
fn target_at(hp: u32, dx: i32, dy: i32) -> Value {
    wave(
        1,
        1,
        0,
        enemy(hp, hold(START_X + dx, START_Y + dy), json!([])),
        START_X + dx,
        START_Y + dy,
    )
}

fn friendly(e: &Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .bullets
        .iter()
        .filter(|b| b.friendly)
        .map(|b| (b.vx.0, b.vy.0))
        .collect()
}

/// The velocities of the friendly bullets made on the first tick.
fn first_tick(stage: &Stage) -> Vec<(i32, i32)> {
    let mut e = engine(stage);
    e.step(idle());
    friendly(&e)
}

/// The ticks, of the first `ticks`, on which friendly bullets with velocity `v` were made.
fn ticks_firing(stage: &Stage, ticks: u32, v: (i32, i32)) -> Vec<u32> {
    let mut e = engine(stage);
    let mut before = 0;
    let mut fired = Vec::new();
    for _ in 0..ticks {
        e.step(idle());
        let now = friendly(&e).iter().filter(|b| **b == v).count();
        if now > before {
            fired.push(e.tick());
        }
        before = now;
    }
    fired
}

fn far(waves: Vec<Value>) -> Stage {
    stage_with(600, Some(far_boss()), waves)
}

// --- Two weapons, each on its own ----------------------------------------------------------

#[test]
fn main_and_sub_attack_side_by_side_each_on_its_own_cooldown() {
    let stage = armed(
        far(vec![]),
        weapon(one_bullet(2048), json!("facing"), 6),
        weapon(one_bullet(1024), json!("facing"), 10),
    );
    assert_eq!(first_tick(&stage), [(0, -2048), (0, -1024)]);
    assert_eq!(ticks_firing(&stage, 30, (0, -2048)), [1, 7, 13, 19, 25]);
    assert_eq!(ticks_firing(&stage, 30, (0, -1024)), [1, 11, 21]);
}

#[test]
fn the_cooldown_counts_from_the_attacks_start_and_a_longer_attack_waits() {
    // One tick that fires, then 10 of recovery: the attack lasts 11 ticks.
    let long = json!({ "inline": [
        { "kind": "active", "ticks": 1, "spawn": [fire(1024, aimed())] },
        { "kind": "recovery", "ticks": 10 }
    ] });
    let with = |cooldown| {
        armed(
            far(vec![]),
            quiet_weapon(),
            weapon(long.clone(), json!("facing"), cooldown),
        )
    };
    // A cooldown of 6 is over on tick 7, but the attack runs until tick 11.
    assert_eq!(ticks_firing(&with(6), 40, (0, -1024)), [1, 12, 23, 34]);
    // A cooldown of 20 outlasts the attack.
    assert_eq!(ticks_firing(&with(20), 60, (0, -1024)), [1, 21, 41]);
}

// --- Targeting -------------------------------------------------------------------------------

#[test]
fn the_nearest_enemy_is_the_default_target() {
    // 160 pixels to the right and 168 to the left; the boss is far up and to the left.
    let waves = vec![target_at(1000, 40_960, 0), target_at(1000, -43_000, 0)];
    let sub = json!({ "attack": one_bullet(1024), "cooldown_ticks": 600 });
    let stage = armed(far(waves), quiet_weapon(), sub);
    assert_eq!(first_tick(&stage), [(1024, 0)]);
}

#[test]
fn with_no_target_the_weapon_waits_without_spending_its_cooldown() {
    // No boss, and the only enemy arrives on tick 5.
    let arrives = wave(
        5,
        1,
        0,
        enemy(1000, hold(START_X + 40_960, START_Y), json!([])),
        START_X + 40_960,
        START_Y,
    );
    let stage = armed(
        stage_with(600, None, vec![arrives]),
        quiet_weapon(),
        weapon(one_bullet(1024), json!("nearest_enemy"), 30),
    );
    assert_eq!(ticks_firing(&stage, 40, (1024, 0)), [5, 35]);
}

#[test]
fn boss_first_and_part_aim_where_they_say() {
    // The boss stands 80 pixels to the left, its part 80 pixels to the right; an enemy is
    // nearer, 40 pixels above.
    let boss = json!({ "base": "golem", "hp": 1_000_000, "radius": 8192,
                       "spawn": { "x": START_X - 20_480, "y": START_Y },
                       "parts": [{ "id": "horn", "asset": "horns_2", "hp": 30,
                                   "offset": { "x": 40_960, "y": 0 }, "radius": 2048 }],
                       "phases": [{}] });
    let near = target_at(1000, 0, -10_240);
    let with = |targeting: Value| {
        armed(
            stage_with(600, Some(boss.clone()), vec![near.clone()]),
            quiet_weapon(),
            weapon(one_bullet(1024), targeting, 600),
        )
    };
    assert_eq!(first_tick(&with(json!("nearest_enemy"))), [(0, -1024)]);
    assert_eq!(first_tick(&with(json!("boss_first"))), [(-1024, 0)]);
    assert_eq!(first_tick(&with(json!({ "part": "horn" }))), [(1024, 0)]);
    assert_eq!(first_tick(&with(json!("facing"))), [(0, -1024)]);
}

#[test]
fn an_attack_at_a_target_starts_its_relative_directions_toward_it() {
    let relative = json!({ "inline": spawned(json!([
        fire(1024, json!({ "type": "relative", "angle": 0 }))
    ])) });
    let stage = armed(
        far(vec![target_at(1000, 40_960, 0)]),
        quiet_weapon(),
        weapon(relative, json!("nearest_enemy"), 600),
    );
    assert_eq!(first_tick(&stage), [(1024, 0)]);
}

#[test]
fn a_later_phase_still_heads_where_the_target_was_when_the_attack_started() {
    // The enemy starts 160 pixels to the right and drifts down 1 pixel a tick; the bullet comes
    // from the second phase, 10 ticks into the attack, and still goes straight right.
    let drifting = wave(
        1,
        1,
        0,
        enemy(
            1000,
            json!({ "type": "straight", "vx": 0, "vy": 256 }),
            json!([]),
        ),
        START_X + 40_960,
        START_Y,
    );
    let later = json!({ "inline": [
        { "kind": "recovery", "ticks": 10 },
        { "kind": "active", "ticks": 1,
          "spawn": [fire(1024, json!({ "type": "relative", "angle": 0 }))] }
    ] });
    let stage = armed(
        far(vec![drifting]),
        quiet_weapon(),
        weapon(later, json!("nearest_enemy"), 600),
    );
    let mut e = engine(&stage);
    for _ in 0..11 {
        e.step(idle());
    }
    assert_eq!(friendly(&e), [(1024, 0)]);
}

#[test]
fn aimed_bullets_keep_the_last_direction_to_a_target_that_is_gone() {
    // Two bullets, 20 ticks apart, at an enemy of 10 hp 80 pixels to the right. The first, at 16
    // pixels a tick, reaches it on tick 5 and kills it; the second still goes right.
    let twice = json!({ "inline": spawned(json!([
        { "type": "repeat", "count": 2, "interval_ticks": 20, "body": [fire(4096, aimed())] }
    ])) });
    let stage = armed(
        far(vec![target_at(10, 20_480, 0)]),
        quiet_weapon(),
        weapon(twice, json!("nearest_enemy"), 600),
    );
    let mut e = engine(&stage);
    for _ in 0..21 {
        e.step(idle());
    }
    assert_eq!(enemies(&e.snapshot()), 0, "the first bullet killed it");
    assert_eq!(friendly(&e), [(4096, 0)]);
}

// --- Presets ---------------------------------------------------------------------------------

#[test]
fn a_preset_runs_its_main_role_as_main_and_its_sub_role_as_sub() {
    let preset = |id: &str| json!({ "preset": { "id": id } });
    let waves = || vec![target_at(1000, 40_960, 0)];
    // blaster: Main is the twin shot along the facing, Sub one bullet at the nearest enemy.
    let blaster = first_tick(&armed(far(waves()), preset("blaster"), preset("blaster")));
    assert_eq!(blaster.len(), 3, "{blaster:?}");
    assert!(
        blaster[..2].iter().all(|(_, vy)| *vy < -2000),
        "{blaster:?}"
    );
    assert_eq!(blaster[2], (2048, 0));
    // needle: Main is one fast bullet at the nearest enemy, Sub five along the facing.
    let needle = first_tick(&armed(far(waves()), preset("needle"), preset("needle")));
    assert_eq!(needle.len(), 6, "{needle:?}");
    assert_eq!(needle[0], (3072, 0));
}

#[test]
fn a_preset_and_its_inline_copy_run_the_same() {
    let preset = |id: &str| json!({ "preset": { "id": id } });
    let preset_of = |id: &str| presets::weapon_preset(id).expect("a weapon preset");
    let copy = |w: Value| json!({ "inline": w });
    let waves = || {
        vec![
            target_at(1000, 40_960, -20_000),
            target_at(30, -30_000, -40_000),
        ]
    };
    let by_id = armed(far(waves()), preset("needle"), preset("blaster"));
    let inline = armed_full(
        far(waves()),
        copy(serde_json::to_value(&preset_of("needle").main).unwrap()),
        copy(serde_json::to_value(&preset_of("blaster").sub).unwrap()),
    );
    let (mut a, mut b) = (engine(&by_id), engine(&inline));
    // Charging for 45 ticks (a perfect release at level 2 of needle's charge), and the Sub
    // skill pressed on tick 50.
    for t in 0..120 {
        let input = Input {
            main: (5..50).contains(&t),
            sub: t == 50,
            ..if t < 60 { idle() } else { moving(40, -20) }
        };
        a.step(input);
        b.step(input);
        assert_eq!(a.state_hash(), b.state_hash(), "tick {}", a.tick());
    }
}

#[test]
fn every_weapon_preset_is_registered_and_both_roles_are_valid() {
    let registered: Vec<_> = registry::ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Weapon)
        .map(|e| e.id)
        .collect();
    let defined: Vec<_> = presets::weapon_ids().collect();
    assert_eq!(registered, defined);
    for id in defined {
        let w = presets::weapon_preset(id).expect("a definition");
        assert_eq!(validate_weapon_preset(w, registry::ENTRIES), Ok(()), "{id}");
    }
}
