//! The charged Main skill and the Sub skill (ADR-050, core#83), worked out by hand. The main
//! character stands at (46080, 147456), facing up the field; a release attacks along the facing.
//!
//! The test charge: level 1 at 0 ticks, level 2 at 30 (perfect for 8 ticks, +50%), level 3 at
//! 60 (perfect for 8 ticks, +100%, and a different attack), released by itself at 90; a
//! cooldown of 121 ticks, so that half of it rounds up to 61; half speed while charging.

mod common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::ChargeView;
use stella_rain_core::stage::Stage;

const COOLDOWN: u32 = 121;

/// An attack that fires one bullet at `speed` along its aim on the tick it starts.
fn one_bullet(speed: i32) -> Value {
    json!({ "inline": spawned(json!([
        { "type": "fire", "speed": speed, "direction": { "type": "aimed", "offset": 0 } }
    ])) })
}

fn test_charge() -> Value {
    json!({
        "levels": [
            { "at_ticks": 0, "attack": one_bullet(1024) },
            { "at_ticks": 30, "perfect_ticks": 8, "perfect_pct": 50, "attack": one_bullet(2048) },
            { "at_ticks": 60, "perfect_ticks": 8, "perfect_pct": 100, "attack": one_bullet(3072),
              "perfect_attack": one_bullet(4096) }
        ],
        "max_ticks": 90, "cooldown_ticks": COOLDOWN, "move_pct": 50
    })
}

/// A stage whose Main weapon charges with `test_charge`, its automatic attack `main_auto`; the
/// Sub weapon's automatic attack is `sub_auto` and its skill `skill`.
fn charging(stage: Stage, main_auto: Value, sub_auto: Value, skill: Value) -> Stage {
    armed_full(
        stage,
        json!({ "inline": { "auto": main_auto, "charge": test_charge() } }),
        json!({ "inline": { "auto": sub_auto, "skill": skill } }),
    )
}

fn quiet_charging(stage: Stage) -> Stage {
    charging(stage, quiet_weapon(), quiet_weapon(), quiet_weapon())
}

fn far() -> Stage {
    stage_with(18_000, Some(far_boss()), vec![])
}

fn held(charge: bool) -> Input {
    Input {
        main: charge,
        ..idle()
    }
}

/// Steps through `inputs`, returning every event with its tick.
fn play(e: &mut Engine, inputs: impl IntoIterator<Item = Input>) -> Vec<(u32, DomainEvent)> {
    let mut seen = Vec::new();
    for input in inputs {
        e.step(input);
        seen.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
    }
    seen
}

/// Charge held on ticks 1 to `last`, then let go, for `ticks` ticks in all.
fn hold_until(last: u32, ticks: u32) -> Vec<Input> {
    (1..=ticks).map(|t| held(t <= last)).collect()
}

fn releases(events: &[(u32, DomainEvent)]) -> Vec<(u32, u8, bool)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::ChargeReleased { level, perfect } => Some((*t, *level, *perfect)),
            _ => None,
        })
        .collect()
}

fn friendly(e: &Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .bullets
        .iter()
        .filter(|b| b.friendly)
        .map(|b| (b.vx.0, b.vy.0))
        .collect()
}

// --- Charging --------------------------------------------------------------------------------

#[test]
fn holding_raises_the_level_and_the_snapshot_shows_the_perfect_window() {
    let mut e = engine(&quiet_charging(far()));
    let mut seen = Vec::new();
    for _ in 0..40 {
        e.step(held(true));
        seen.push(e.snapshot().player.charge);
    }
    let view = |level, held, perfect| {
        Some(ChargeView {
            level,
            held,
            perfect,
        })
    };
    // Tick t shows t − 1 ticks held: 0 on the press's tick.
    assert_eq!(seen[0], view(1, 0, false));
    assert_eq!(seen[29], view(1, 29, false));
    assert_eq!(seen[30], view(2, 30, true));
    assert_eq!(seen[37], view(2, 37, true));
    assert_eq!(seen[38], view(2, 38, false));
}

#[test]
fn a_release_inside_the_window_is_perfect_and_one_tick_later_is_not() {
    // Held to tick 38, which shows 37 held, the window's last tick: released on tick 39.
    let mut e = engine(&quiet_charging(far()));
    let events = play(&mut e, hold_until(38, 39));
    assert_eq!(releases(&events), [(39, 2, true)]);
    assert_eq!(friendly(&e), [(0, -2048)]);
    assert_eq!(e.snapshot().player.charge, None);

    let mut e = engine(&quiet_charging(far()));
    let events = play(&mut e, hold_until(39, 40));
    assert_eq!(releases(&events), [(40, 2, false)]);
}

#[test]
fn a_tap_releases_level_one_without_a_bonus() {
    let mut e = engine(&quiet_charging(far()));
    let events = play(&mut e, hold_until(1, 2));
    assert_eq!(releases(&events), [(2, 1, false)]);
    assert_eq!(friendly(&e), [(0, -1024)]);
}

#[test]
fn a_perfect_release_makes_the_levels_replacement_attack() {
    let mut e = engine(&quiet_charging(far()));
    let events = play(&mut e, hold_until(61, 62));
    assert_eq!(releases(&events), [(62, 3, true)]);
    assert_eq!(friendly(&e), [(0, -4096)]);
}

#[test]
fn at_max_ticks_it_releases_by_itself_without_the_bonus() {
    // Held throughout: tick 91 would show 90 held, the maximum.
    let mut e = engine(&quiet_charging(far()));
    let events = play(&mut e, hold_until(200, 100));
    assert_eq!(releases(&events), [(91, 3, false)]);
}

#[test]
fn the_perfect_bonus_raises_every_hit_of_the_release() {
    // A sturdy enemy 80 pixels above; the release at level 2 reaches it in 10 ticks.
    let target = enemy(1000, hold(START_X, START_Y - 20_480), json!([]));
    let stage = || {
        quiet_charging(stage_with(
            18_000,
            Some(far_boss()),
            vec![wave(1, 1, 0, target.clone(), START_X, START_Y - 20_480)],
        ))
    };
    let hp_after = |last_held: u32| {
        let mut e = engine(&stage());
        play(&mut e, hold_until(last_held, last_held + 20));
        e.snapshot().entities[1].hp
    };
    // 10 a hit, or 15 with the +50% of a perfect release.
    assert_eq!(hp_after(38), 985);
    assert_eq!(hp_after(39), 990);
}

#[test]
fn only_a_press_while_ready_starts_a_charge() {
    // A tap releases on tick 2; the cooldown of 121 ticks runs out on tick 123. The key, pressed
    // again on tick 3 and held through tick 130, starts nothing; a new press does.
    let mut e = engine(&quiet_charging(far()));
    let inputs: Vec<Input> = (1..=140)
        .map(|t| held(t == 1 || (3..=130).contains(&t) || t >= 132))
        .collect();
    let mut charging_on = Vec::new();
    for (i, input) in inputs.into_iter().enumerate() {
        e.step(input);
        if e.snapshot().player.charge.is_some() {
            charging_on.push(i as u32 + 1);
        }
        if i + 1 == 122 {
            assert_eq!(e.snapshot().player.main_skill.cooldown_left, 1);
            assert!(!e.snapshot().player.main_skill.ready);
        }
        if i + 1 == 123 {
            assert!(e.snapshot().player.main_skill.ready);
        }
    }
    assert_eq!(charging_on.first(), Some(&1));
    assert_eq!(charging_on.get(1), Some(&132));
}

#[test]
fn the_main_weapons_automatic_attack_pauses_while_charging_and_the_subs_goes_on() {
    let stage = charging(
        far(),
        weapon(one_bullet(512), json!("facing"), 6),
        weapon(one_bullet(256), json!("facing"), 6),
        quiet_weapon(),
    );
    let mut e = engine(&stage);
    let (mut main_ticks, mut sub_ticks) = (Vec::new(), Vec::new());
    let (mut mains, mut subs) = (0, 0);
    for input in hold_until(20, 30) {
        e.step(input);
        let shots = friendly(&e);
        let count = |vy| shots.iter().filter(|b| b.1 == vy).count();
        if count(-512) > mains {
            main_ticks.push(e.tick());
        }
        if count(-256) > subs {
            sub_ticks.push(e.tick());
        }
        (mains, subs) = (count(-512), count(-256));
    }
    // Charging from tick 1 to 20 and released on tick 21.
    assert_eq!(main_ticks, [21, 27]);
    assert_eq!(sub_ticks, [1, 7, 13, 19, 25]);
}

#[test]
fn charging_slows_the_main_character() {
    let mut e = engine(&quiet_charging(far()));
    e.step(Input {
        main: true,
        ..moving(127, 0)
    });
    assert_eq!(player_at(&e), (START_X + SPEED / 2, START_Y));
    // Let go: the release, then full speed.
    e.step(moving(127, 0));
    assert_eq!(player_at(&e), (START_X + SPEED / 2 + SPEED, START_Y));
}

// --- Breaking a charge -----------------------------------------------------------------------

#[test]
fn an_evade_breaks_the_charge_and_spends_half_the_cooldown_rounded_up() {
    let mut e = engine(&quiet_charging(far()));
    let inputs = (1..=10).map(|t| Input {
        evade: t == 5,
        ..held(true)
    });
    let events = play(&mut e, inputs);
    let broken: Vec<u32> = ticks_of(&events, |ev| *ev == DomainEvent::ChargeBroken);
    assert_eq!(broken, [5]);
    assert!(releases(&events).is_empty());
    assert!(friendly(&e).is_empty(), "no attack");
    // Set to 61 on tick 5, down by 5 since.
    let p = e.snapshot().player;
    assert_eq!((p.charge, p.main_skill.cooldown_left), (None, 56));
}

/// A stage where an enemy 40 pixels above fires one bullet at the main character on tick 1,
/// which reaches it on tick 10; `breaks` marks the attack as one that breaks charges.
fn shot_at(breaks: bool) -> Stage {
    let attack = json!({ "inline": [{ "kind": "active", "ticks": 1, "breaks_charge": breaks,
        "spawn": [{ "type": "fire", "speed": 1024,
                    "direction": { "type": "aimed", "offset": 0 } }] }] });
    let shooter = enemy(
        1000,
        hold(START_X, START_Y - 10_240),
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack", "attack": attack }),
            600
        )]),
    );
    quiet_charging(stage_with(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, START_X, START_Y - 10_240)],
    ))
}

#[test]
fn a_hit_from_a_powerful_attack_breaks_the_charge_and_others_do_not() {
    for breaks in [true, false] {
        let mut e = engine(&shot_at(breaks));
        let events = play(&mut e, (1..=15).map(|_| held(true)));
        let hit = ticks_of(&events, |ev| matches!(ev, DomainEvent::PlayerHit { .. }));
        let broken = ticks_of(&events, |ev| *ev == DomainEvent::ChargeBroken);
        assert_eq!(hit.len(), 1, "{breaks}");
        if breaks {
            assert_eq!(broken, hit);
            assert_eq!(e.snapshot().player.charge, None);
        } else {
            assert!(broken.is_empty());
            assert!(e.snapshot().player.charge.is_some());
        }
    }
}

// --- The Sub skill ---------------------------------------------------------------------------

#[test]
fn the_sub_skill_starts_on_a_ready_press_at_its_target() {
    // An enemy 160 pixels to the right; the skill fires at it, then waits 60 ticks.
    let target = enemy(1000, hold(START_X + 40_960, START_Y), json!([]));
    let stage = charging(
        stage_with(
            18_000,
            Some(far_boss()),
            vec![wave(1, 1, 0, target, START_X + 40_960, START_Y)],
        ),
        quiet_weapon(),
        quiet_weapon(),
        weapon(one_bullet(1024), json!("nearest_enemy"), 60),
    );
    let mut e = engine(&stage);
    let mut fired = Vec::new();
    let mut before = 0;
    for t in 1..=70 {
        e.step(Input {
            sub: [3, 10, 62, 63].contains(&t),
            ..idle()
        });
        let now = friendly(&e).iter().filter(|b| **b == (1024, 0)).count();
        if now > before {
            fired.push(t);
        }
        before = now;
        if t == 10 {
            assert_eq!(e.snapshot().player.sub_skill.cooldown_left, 53);
        }
    }
    // Pressed on 3, then on 10 and 62 while it waits (set to 60 on tick 3, ready on 63).
    assert_eq!(fired, [3, 63]);
}

#[test]
fn the_sub_skill_with_nothing_to_aim_at_does_nothing_and_stays_ready() {
    let stage = charging(
        stage_with(
            18_000,
            None,
            vec![wave(50, 1, 0, enemy(1000, hold(0, 0), json!([])), 0, 0)],
        ),
        quiet_weapon(),
        quiet_weapon(),
        weapon(one_bullet(1024), json!("nearest_enemy"), 60),
    );
    let mut e = engine(&stage);
    e.step(Input {
        sub: true,
        ..idle()
    });
    assert!(friendly(&e).is_empty());
    let view = e.snapshot().player.sub_skill;
    assert!(view.ready);
    assert_eq!(view.cooldown_left, 0);
}
