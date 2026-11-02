//! Auto mode (ADR-073): while `auto` is on and there is a target, the simulation presses for
//! the player through the auto combo, step by step, and starts it again from the root once its
//! last step has run out. It never holds, evades or takes the dash branch. The auto combo starts
//! as the Main weapon's A chain; a combo the player joins by hand replaces it when it resets,
//! unless it began in the dash branch.
//!
//! The test weapon is `tests/weapons.rs`'s: every step winds up for 2 ticks, is active for 2 and
//! recovers for 6, so a step taken on tick t can be followed from t + 4 and ends after t + 9.
//! The main character stands at (46080, 147456), facing up the field.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::{DomainEvent, Press};
use stella_rain_core::input::Input;
use stella_rain_core::stage::Stage;

/// 100 pixels.
const RANGE: i32 = 25_600;
const WINDOW: u32 = 10;
/// `trig::SCALE`: a facing's length.
const SCALE: i32 = 65_536;

fn swing() -> Value {
    json!({ "inline": [
        { "kind": "wind_up", "ticks": 2 },
        { "kind": "active", "ticks": 2 },
        { "kind": "recovery", "ticks": 6 }
    ] })
}

/// A, then AA, then AAA (a leaf); AB after A (a leaf); B from the root (a leaf); a dash-strike
/// for A. `first` is the step A, so a test can make it charge.
fn weapon(first: Value) -> Value {
    json!({ "inline": {
        "combo": { "main": first, "smash": step(swing()) },
        "dash": { "main": step(swing()) },
        "window_ticks": WINDOW, "dash_strike_ticks": 5, "assist_range": RANGE
    } })
}

fn a_step() -> Value {
    json!({ "attack": swing(), "next": {
        "main": { "attack": swing(), "next": { "main": step(swing()) } },
        "smash": step(swing()) } })
}

/// A step A that charges while held, with the same next steps.
fn charging_a() -> Value {
    let mut a = a_step();
    a["charge"] = json!({ "max_ticks": 60, "levels": [{ "at_ticks": 5, "attack": swing() }] });
    a
}

/// A bat that holds `dx`, `dy` from the main character's start from tick 1.
fn bat_at(dx: i32, dy: i32) -> Value {
    let (x, y) = (START_X + dx, START_Y + dy);
    wave(1, 1, 0, enemy(100_000, hold(x, y), json!([])), x, y)
}

fn stage_with_bat(first: Value, dx: i32, dy: i32) -> Stage {
    armed(
        stage_with(600, Some(far_boss()), vec![bat_at(dx, dy)]),
        weapon(first),
    )
}

/// A bat in range straight up the field, so a step faces where it would anyway.
fn target_above() -> Stage {
    stage_with_bat(a_step(), 0, -RANGE / 2)
}

/// The inputs of ticks 1 to `ticks` with `auto` on: on each tick of `presses` the buttons its
/// letters name are held, for that tick only: `a` (`main`), `b` (`smash`), `x` (`evade`).
fn auto(presses: &[(u32, &str)], ticks: u32) -> Vec<Input> {
    (1..=ticks)
        .map(|t| {
            let held = presses
                .iter()
                .find(|(at, _)| *at == t)
                .map_or("", |(_, keys)| *keys);
            Input {
                main: held.contains('a'),
                smash: held.contains('b'),
                evade: held.contains('x'),
                auto: true,
                ..idle()
            }
        })
        .collect()
}

/// Every event of a run of `inputs`, with its tick.
fn play(e: &mut Engine, inputs: &[Input]) -> Vec<(u32, DomainEvent)> {
    let mut events = Vec::new();
    for input in inputs {
        e.step(&[*input]);
        events.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
    }
    events
}

/// The steps taken: the tick, the press and the depth reached.
fn steps(events: &[(u32, DomainEvent)]) -> Vec<(u32, Press, u8)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::ComboStep { press, depth } => Some((*t, *press, *depth)),
            _ => None,
        })
        .collect()
}

fn resets(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| *ev == DomainEvent::ComboReset)
}

fn auto_combo(e: &Engine) -> Vec<Press> {
    e.snapshot().players[0].combo.auto.clone()
}

const M: Press = Press::Main;
const S: Press = Press::Smash;

// --- Playing the auto combo --------------------------------------------------------------------

#[test]
fn auto_mode_plays_the_a_chain_and_starts_it_again_after_its_leaf() {
    let mut e = engine(&target_above());
    assert_eq!(auto_combo(&e), [M, M, M]);
    let events = play(&mut e, &auto(&[], 30));
    // AAA, a leaf, starts on tick 9 and ends after tick 18.
    assert_eq!(
        steps(&events),
        [
            (1, M, 1),
            (5, M, 2),
            (9, M, 3),
            (19, M, 1),
            (23, M, 2),
            (27, M, 3)
        ]
    );
    assert_eq!(resets(&events), [19]);
}

#[test]
fn without_a_target_in_range_auto_mode_presses_nothing() {
    let mut e = engine(&stage_with_bat(a_step(), 0, -RANGE * 2));
    assert!(steps(&play(&mut e, &auto(&[], 30))).is_empty());
}

#[test]
fn with_auto_off_nothing_is_pressed() {
    let mut e = engine(&target_above());
    assert!(steps(&play(&mut e, &vec![idle(); 30])).is_empty());
}

#[test]
fn auto_mode_plays_a_charged_step_as_its_plain_step() {
    let mut e = engine(&stage_with_bat(charging_a(), 0, -RANGE / 2));
    let events = play(&mut e, &auto(&[], 1));
    assert_eq!(e.snapshot().players[0].charge, None);
    let events = [events, play(&mut e, &auto(&[], 9))].concat();
    assert_eq!(steps(&events), [(1, M, 1), (5, M, 2), (9, M, 3)]);
}

// --- The player's hand -------------------------------------------------------------------------

#[test]
fn a_combo_played_by_hand_becomes_the_auto_combo_when_its_window_ends() {
    // B by hand on tick 1 ends after tick 10; auto mode waits through the window to tick 20.
    let mut e = engine(&target_above());
    let events = play(&mut e, &auto(&[(1, "b")], 35));
    assert_eq!(steps(&events), [(1, S, 1), (21, S, 1), (31, S, 1)]);
    assert_eq!(resets(&events), [21, 31]);
    assert_eq!(auto_combo(&e), [S]);
}

#[test]
fn a_press_by_hand_joins_the_auto_combo_and_the_whole_combo_replaces_it() {
    // Auto mode plays A on tick 1; B pressed by hand on tick 3 takes AB on tick 5, a leaf that
    // ends after tick 14 and whose window ends after tick 24.
    let mut e = engine(&target_above());
    let events = play(&mut e, &auto(&[(3, "b")], 40));
    assert_eq!(
        steps(&events),
        [(1, M, 1), (5, S, 2), (25, M, 1), (29, S, 2), (39, M, 1)]
    );
    assert_eq!(auto_combo(&e), [M, S]);
}

#[test]
fn an_evade_ends_a_hand_combo_into_the_auto_combo_and_auto_mode_never_dash_strikes() {
    // A by hand on tick 1; the evade on tick 3 resets the combo, which becomes the auto combo,
    // and auto mode presses A at once, from the root and not the dash branch.
    let mut e = engine(&target_above());
    let events = play(&mut e, &auto(&[(1, "a"), (3, "x")], 13));
    assert_eq!(steps(&events), [(1, M, 1), (3, M, 1), (13, M, 1)]);
    assert_eq!(resets(&events), [3, 13]);
    assert!(!e.snapshot().players[0].combo.dash);
    assert_eq!(auto_combo(&e), [M]);
}

#[test]
fn a_dash_strike_by_hand_does_not_become_the_auto_combo() {
    // The dash-strike on tick 1 ends after tick 10 and its window after tick 20.
    let mut e = engine(&target_above());
    let events = play(&mut e, &auto(&[(1, "xa")], 30));
    assert_eq!(
        steps(&events),
        [(1, M, 1), (21, M, 1), (25, M, 2), (29, M, 3)]
    );
    assert_eq!(auto_combo(&e), [M, M, M]);
}

// --- Aim ---------------------------------------------------------------------------------------

#[test]
fn auto_mode_faces_the_target_with_assist_off_and_the_hand_does_not() {
    let to_the_right = stage_with_bat(a_step(), RANGE / 2, 0);
    let mut e = engine(&to_the_right);
    play(&mut e, &auto(&[], 1));
    let f = e.snapshot().players[0].facing;
    assert_eq!((f.x, f.y), (SCALE, 0));

    let mut e = engine(&to_the_right);
    play(&mut e, &auto(&[(1, "b")], 1));
    let f = e.snapshot().players[0].facing;
    assert_eq!((f.x, f.y), (0, -SCALE));
}

// --- Charged steps (ADR-114) -------------------------------------------------------------------

/// A, then AA that charges (level 1 at 5 ticks held, level 2 at 10, both with a perfect window
/// that would add damage), then AAA, a leaf.
fn charged_chain() -> Value {
    let level = |at: u32| json!({ "at_ticks": at, "attack": swing(), "perfect_ticks": 3, "perfect_pct": 50 });
    json!({ "inline": {
        "combo": { "main": { "attack": swing(), "next": { "main": {
            "attack": swing(),
            "charge": { "max_ticks": 60, "levels": [level(5), level(10)] },
            "next": { "main": step(swing()) }
        } } }, "smash": step(swing()) },
        "dash": { "main": step(swing()) },
        "window_ticks": WINDOW, "dash_strike_ticks": 5, "assist_range": RANGE
    } })
}

fn charged_stage() -> Stage {
    armed(
        stage_with(600, Some(far_boss()), vec![bat_at(0, -RANGE / 2)]),
        charged_chain(),
    )
}

/// The auto field of the snapshot, read from its JSON so that these tests state the contract
/// (absent when auto mode is not charging) and not the Rust type.
fn auto_charge(e: &Engine) -> Option<u64> {
    let player = serde_json::to_value(e.snapshot().players[0].clone()).unwrap();
    player.get("auto_charge").and_then(Value::as_u64)
}

/// Ticks 1 to 40 by hand with `auto` off: A, then AA with `main` held through tick 15 (level 2
/// at its release, let go on tick 16), then AAA on tick 20; the window ends after tick 39.
fn hand_released_at_level_2() -> Vec<Input> {
    (1..=40)
        .map(|t| Input {
            main: t == 1 || (5..=15).contains(&t) || t == 20,
            ..idle()
        })
        .collect()
}

/// An engine whose hand combo has been played and has reset, so that it is the auto combo.
fn after_the_hand_combo(hand: &[Input]) -> (Engine, Vec<(u32, DomainEvent)>) {
    let mut e = engine(&charged_stage());
    let events = play(&mut e, hand);
    (e, events)
}

/// Auto mode for `ticks` ticks, one at a time: the events with their tick, and per tick the
/// hand charge's level and the auto field.
#[allow(clippy::type_complexity)]
fn auto_run(
    e: &mut Engine,
    inputs: &[Input],
) -> (Vec<(u32, DomainEvent)>, Vec<(Option<u8>, Option<u64>)>) {
    let (mut events, mut views) = (Vec::new(), Vec::new());
    for input in inputs {
        e.step(&[*input]);
        events.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        views.push((
            e.snapshot().players[0].charge.map(|c| c.level),
            auto_charge(e),
        ));
    }
    (events, views)
}

fn released(events: &[(u32, DomainEvent)]) -> Vec<(u32, u8, bool)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::ChargeReleased { level, perfect } => Some((*t, *level, *perfect)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_step_the_hand_released_at_level_2_is_charged_to_level_2_by_auto_mode() {
    let (mut e, hand) = after_the_hand_combo(&hand_released_at_level_2());
    // The hand let go inside level 2's perfect window; auto mode must not get that bonus.
    assert_eq!(released(&hand), [(16, 2, true)]);
    let (events, _) = auto_run(&mut e, &auto(&[], 40));
    // Auto mode takes A on tick 41, AA on 45 and charges it: 10 ticks to level 2, released on
    // the tick that reaches it, then AAA four ticks into the release and A again after its leaf.
    assert_eq!(released(&events).len(), 1, "{events:?}");
    let (t, level, perfect) = released(&events)[0];
    assert_eq!((t, level, perfect), (55, 2, false));
    let steps = steps(&events);
    assert_eq!(steps[0].2, 1);
    assert_eq!(steps[1].2, 2);
    assert_eq!(steps[2].2, 3);
}

#[test]
fn during_the_auto_charge_the_hand_charge_is_none_and_the_auto_field_shows_the_level() {
    let (mut e, _) = after_the_hand_combo(&hand_released_at_level_2());
    let (events, views) = auto_run(&mut e, &auto(&[], 30));
    let charging: Vec<usize> = (0..views.len()).filter(|&i| views[i].1.is_some()).collect();
    assert!(!charging.is_empty(), "auto mode never charged: {events:?}");
    assert!(charging.iter().all(|&i| views[i] == (None, Some(2))));
    // The charge shows from the tick AA starts to the tick before its release.
    assert_eq!(charging.len(), 10);
    assert_eq!(auto_charge(&e), None);
}

#[test]
fn an_auto_release_is_never_perfect() {
    let (mut e, _) = after_the_hand_combo(&hand_released_at_level_2());
    let (events, _) = auto_run(&mut e, &auto(&[], 100));
    let all = released(&events);
    assert!(all.len() >= 2, "{all:?}");
    assert!(
        all.iter()
            .all(|&(_, level, perfect)| level == 2 && !perfect)
    );
}

#[test]
fn the_initial_auto_combo_plays_no_charge() {
    let mut e = engine(&charged_stage());
    let (events, views) = auto_run(&mut e, &auto(&[], 40));
    assert!(released(&events).is_empty());
    assert!(views.iter().all(|v| *v == (None, None)));
}

#[test]
fn a_hand_tap_of_the_charged_step_leaves_the_auto_combo_without_a_charge() {
    // AA tapped by hand is level 0: the auto combo plays it plain.
    let hand: Vec<Input> = (1..=40)
        .map(|t| Input {
            main: matches!(t, 1 | 5 | 9),
            ..idle()
        })
        .collect();
    let (mut e, _) = after_the_hand_combo(&hand);
    let (events, views) = auto_run(&mut e, &auto(&[], 30));
    assert!(released(&events).is_empty());
    assert!(views.iter().all(|v| v.1.is_none()));
}

#[test]
fn a_press_by_hand_during_the_auto_charge_takes_over() {
    let (mut e, _) = after_the_hand_combo(&hand_released_at_level_2());
    // Auto mode charges AA from tick 45 (5 of the run); the hand presses A on tick 8 of the run.
    let inputs = auto(&[(8, "a")], 30);
    let (events, _) = auto_run(&mut e, &inputs);
    // The charge runs on to its level; the press is kept, takes AAA once the step can be
    // followed, and the combo is then the hand's: auto mode waits.
    assert_eq!(released(&events).len(), 1, "{events:?}");
    let by_hand = steps(&events);
    assert_eq!(
        by_hand.iter().filter(|s| s.2 == 3).count(),
        1,
        "{by_hand:?}"
    );
    assert_eq!(resets(&events).len(), 0, "{events:?}");
}
