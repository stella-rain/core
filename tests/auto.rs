//! Auto mode (ADR-073): while `auto` is on and there is a target, the simulation presses for
//! the player through the auto combo, step by step, and starts it again from the root once its
//! last step has run out. It never holds, evades or takes the dash branch. The auto combo starts
//! as the Main weapon's A chain; a combo the player joins by hand replaces it when it resets,
//! unless it began in the dash branch.
//!
//! The test weapon is `tests/weapons.rs`'s: every step winds up for 2 ticks, is active for 2 and
//! recovers for 6, so a step taken on tick t can be followed from t + 4 and ends after t + 9.
//! The main character stands at (46080, 147456), facing up the field.

mod common;

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
        "combo": { "main": first, "sub": step(swing()) },
        "dash": { "main": step(swing()) },
        "window_ticks": WINDOW, "dash_strike_ticks": 5, "assist_range": RANGE
    } })
}

fn a_step() -> Value {
    json!({ "attack": swing(), "next": {
        "main": { "attack": swing(), "next": { "main": step(swing()) } },
        "sub": step(swing()) } })
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
        quiet_sub(),
    )
}

/// A bat in range straight up the field, so a step faces where it would anyway.
fn target_above() -> Stage {
    stage_with_bat(a_step(), 0, -RANGE / 2)
}

/// The inputs of ticks 1 to `ticks` with `auto` on: on each tick of `presses` the buttons its
/// letters name are held, for that tick only: `a` (`main`), `b` (`sub`), `x` (`evade`).
fn auto(presses: &[(u32, &str)], ticks: u32) -> Vec<Input> {
    (1..=ticks)
        .map(|t| {
            let held = presses
                .iter()
                .find(|(at, _)| *at == t)
                .map_or("", |(_, keys)| *keys);
            Input {
                main: held.contains('a'),
                sub: held.contains('b'),
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
        e.step(*input);
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
    e.snapshot().player.combo.auto
}

const M: Press = Press::Main;
const S: Press = Press::Sub;

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
    assert_eq!(e.snapshot().player.charge, None);
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
    assert!(!e.snapshot().player.combo.dash);
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
    let f = e.snapshot().player.facing;
    assert_eq!((f.x, f.y), (SCALE, 0));

    let mut e = engine(&to_the_right);
    play(&mut e, &auto(&[(1, "b")], 1));
    let f = e.snapshot().player.facing;
    assert_eq!((f.x, f.y), (0, -SCALE));
}
