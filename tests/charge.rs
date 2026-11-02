//! Charged combo steps (ADR-070), worked out by hand. The main character stands at (46080,
//! 147456), facing up the field; every attack goes along the facing.
//!
//! The test step charges while its button stays held from its start: level 0 is its own
//! attack (an orb at 1024); level 1 at 30 ticks (perfect for 8 ticks, +50%, an orb at
//! 2048); level 2 at 60 (perfect for 8 ticks, +100%, an orb at 3072, or at 4096 when perfect);
//! released by itself at 90; half speed while charging. No cooldown.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::{DomainEvent, Press};
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::ChargeView;
use stella_rain_core::stage::Stage;

/// An attack that places one zone `ahead` along its aim on the tick it starts.
fn one_orb(ahead: i32) -> Value {
    json!({ "inline": spawned(json!([zone_ahead(ahead)])) })
}

/// A friendly zone `ahead` along the aim, there for 200 ticks.
fn zone_ahead(ahead: i32) -> Value {
    json!({ "type": "zone", "shape": { "type": "circle", "radius": 512 },
            "offset": { "forward": ahead }, "direction": { "type": "aimed", "offset": 0 },
            "wind_up_ticks": 0, "active_ticks": 200 })
}

/// A step whose own attack is an orb at 1024 and which charges as the module says.
fn charged() -> Value {
    json!({
        "attack": one_orb(1024),
        "charge": {
            "levels": [
                { "at_ticks": 30, "perfect_ticks": 8, "perfect_pct": 50,
                  "attack": one_orb(2048) },
                { "at_ticks": 60, "perfect_ticks": 8, "perfect_pct": 100,
                  "attack": one_orb(3072), "perfect_attack": one_orb(4096) }
            ],
            "max_ticks": 90, "move_pct": 50
        }
    })
}

/// `stage` whose Main weapon has the charged step for `main` and for `smash`, both leaves.
fn charging(stage: Stage) -> Stage {
    armed(
        stage,
        weapon_combo(json!({ "main": charged(), "smash": charged() })),
    )
}

fn far() -> Stage {
    charging(stage_with(18_000, Some(far_boss()), vec![]))
}

fn held(main: bool) -> Input {
    Input { main, ..idle() }
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

/// `main` held on ticks 1 to `last`, then let go, for `ticks` ticks in all.
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

/// Where the friendly zones are, from where the main character stands.
fn friendly(e: &Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .zones
        .iter()
        .filter(|z| z.friendly)
        .map(|z| (z.at.x.0 - START_X, z.at.y.0 - START_Y))
        .collect()
}

// --- Charging --------------------------------------------------------------------------------

#[test]
fn holding_raises_the_level_and_the_snapshot_shows_the_perfect_window() {
    let mut e = engine(&far());
    let mut seen = Vec::new();
    for _ in 0..40 {
        e.step(held(true));
        seen.push(e.snapshot().players[0].charge);
    }
    let view = |level, held, perfect| {
        Some(ChargeView {
            level,
            held,
            perfect,
        })
    };
    // Tick t shows t − 1 ticks held: 0 on the press's tick, at level 0.
    assert_eq!(seen[0], view(0, 0, false));
    assert_eq!(seen[29], view(0, 29, false));
    assert_eq!(seen[30], view(1, 30, true));
    assert_eq!(seen[37], view(1, 37, true));
    assert_eq!(seen[38], view(1, 38, false));
}

#[test]
fn a_release_inside_the_window_is_perfect_and_one_tick_later_is_not() {
    // Held to tick 38, which shows 37 held, the window's last tick: released on tick 39.
    let mut e = engine(&far());
    let events = play(&mut e, hold_until(38, 39));
    assert_eq!(releases(&events), [(39, 1, true)]);
    assert_eq!(friendly(&e), [(0, -2048)]);
    assert_eq!(e.snapshot().players[0].charge, None);

    let mut e = engine(&far());
    let events = play(&mut e, hold_until(39, 40));
    assert_eq!(releases(&events), [(40, 1, false)]);
}

#[test]
fn a_release_before_level_1_is_the_step_s_own_attack() {
    let mut e = engine(&far());
    let events = play(&mut e, hold_until(1, 2));
    assert!(releases(&events).is_empty(), "a plain step says nothing");
    assert_eq!(friendly(&e), [(0, -1024)]);
}

#[test]
fn a_perfect_release_makes_the_levels_replacement_attack() {
    let mut e = engine(&far());
    let events = play(&mut e, hold_until(61, 62));
    assert_eq!(releases(&events), [(62, 2, true)]);
    assert_eq!(friendly(&e), [(0, -4096)]);
}

#[test]
fn at_max_ticks_it_releases_by_itself_without_the_bonus() {
    // Held throughout: tick 91 would show 90 held, the maximum.
    let mut e = engine(&far());
    let events = play(&mut e, hold_until(200, 100));
    assert_eq!(releases(&events), [(91, 2, false)]);
    assert_eq!(
        friendly(&e).len(),
        1,
        "the button still held starts nothing"
    );
}

#[test]
fn the_perfect_bonus_raises_every_hit_of_the_release() {
    // A sturdy enemy 12 pixels above, where the release at level 1 places its zone.
    let target = enemy(1000, hold(START_X, START_Y - 3_072), json!([]));
    let stage = || {
        charging(stage_with(
            18_000,
            Some(far_boss()),
            vec![wave(1, 1, 0, target.clone(), START_X, START_Y - 3_072)],
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
fn a_charge_has_no_cooldown() {
    // A tap releases on tick 2; a press on tick 4 charges again at once.
    let mut e = engine(&far());
    play(&mut e, (1..=3).map(|t| held(t == 1)));
    e.step(held(true));
    assert_eq!(
        e.snapshot().players[0].charge.map(|c| (c.level, c.held)),
        Some((0, 0))
    );
}

#[test]
fn presses_while_charging_are_not_buffered() {
    // `main` held from tick 1 to 20, `smash` pressed on tick 10: the release on tick 21 is the
    // only step's attack, and the combo is not buffered for `smash`.
    let mut e = engine(&far());
    let inputs = (1..=30).map(|t| Input {
        smash: t == 10,
        ..held(t <= 20)
    });
    let events = play(&mut e, inputs);
    let steps: Vec<_> = events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::ComboStep { press, depth } => Some((*t, *press, *depth)),
            _ => None,
        })
        .collect();
    assert_eq!(steps, [(1, Press::Main, 1)]);
    assert_eq!(friendly(&e), [(0, -1024)]);
    assert_eq!(e.snapshot().players[0].combo.buffered, None);
}

#[test]
fn a_step_reached_by_sub_charges_while_sub_is_held() {
    // `smash` held from tick 1 to 38 and `main` tapped on tick 5: still charging, then perfect.
    let mut e = engine(&far());
    let inputs = (1..=39).map(|t| Input {
        main: t == 5,
        smash: t <= 38,
        ..idle()
    });
    let events = play(&mut e, inputs);
    assert_eq!(releases(&events), [(39, 1, true)]);
    assert_eq!(e.snapshot().players[0].combo.presses, [Press::Smash]);
}

#[test]
fn a_buffered_step_charges_only_if_its_button_is_held_as_it_starts() {
    // The first step winds up for 5 ticks, hits on tick 6 and recovers from tick 7; the charged
    // step follows on `main`. A press on tick 3 is buffered and starts the next step on tick 7,
    // cancelling the recovery.
    let first = json!({ "inline": [
        { "kind": "wind_up", "ticks": 5 },
        { "kind": "active", "ticks": 1,
          "spawn": [zone_ahead(512)] },
        { "kind": "recovery", "ticks": 10 }
    ] });
    let stage = armed(
        stage_with(18_000, Some(far_boss()), vec![]),
        weapon_combo(json!({ "main": { "attack": first, "next": { "main": charged() } } })),
    );
    // Let go before tick 7: a plain step, its orb on tick 7.
    let mut e = engine(&stage);
    play(&mut e, (1..=7).map(|t| held(t == 1 || t == 3)));
    assert_eq!(
        e.snapshot().players[0].combo.presses,
        [Press::Main, Press::Main]
    );
    assert_eq!(e.snapshot().players[0].charge, None);
    assert_eq!(friendly(&e), [(0, -512), (0, -1024)]);
    // Held through tick 7: it charges from there.
    let mut e = engine(&stage);
    play(&mut e, (1..=7).map(|t| held(t == 1 || t >= 3)));
    assert_eq!(
        e.snapshot().players[0].charge.map(|c| (c.level, c.held)),
        Some((0, 0))
    );
}

#[test]
fn charging_slows_the_main_character() {
    let mut e = engine(&far());
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
fn an_evade_breaks_the_charge_with_no_attack_and_the_combo_starts_again() {
    let mut e = engine(&far());
    let inputs = (1..=10).map(|t| Input {
        evade: t == 5,
        ..held(true)
    });
    let events = play(&mut e, inputs);
    assert_eq!(
        ticks_of(&events, |ev| *ev == DomainEvent::ChargeBroken),
        [5]
    );
    assert_eq!(ticks_of(&events, |ev| *ev == DomainEvent::ComboReset), [5]);
    assert!(releases(&events).is_empty());
    assert!(friendly(&e).is_empty(), "no attack");
    // The button still held does not press again.
    let p = e.snapshot().players[0].clone();
    assert_eq!((p.charge, p.combo.presses.len()), (None, 0));
}

/// A stage where an enemy 40 pixels above places a zone on the main character on tick 1,
/// which hurts it on tick 10; `breaks` marks the attack as one that breaks charges.
fn shot_at(breaks: bool) -> Stage {
    let attack = json!({ "inline": [{ "kind": "active", "ticks": 1, "breaks_charge": breaks,
        "spawn": [{ "type": "zone", "shape": { "type": "circle", "radius": 3_072 },
                    "anchor": "target", "wind_up_ticks": 9, "active_ticks": 2 }] }] });
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
    charging(stage_with(
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
            assert_eq!(e.snapshot().players[0].charge, None);
        } else {
            assert!(broken.is_empty());
            assert!(e.snapshot().players[0].charge.is_some());
        }
    }
}
