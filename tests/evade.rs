//! The evade's charges (ADR-074): a stage gives up to three, each spent charge recovers on its
//! own timer of the cooldown counted from its press, and a press during a dash with a charge
//! ready starts the next dash at once. Every number here is worked out by hand from the rules'
//! constants and `common::evade()`.

mod common;

use common::*;
use serde_json::json;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EvadeView;
use stella_rain_core::stage::Stage;

fn evading(move_x: i8, move_y: i8) -> Input {
    Input {
        move_x,
        move_y,
        evade: true,
        ..Input::default()
    }
}

/// The test stage with `charges` and, if given, `max_charges` on its evade.
fn charged(charges: u8, max_charges: Option<u8>) -> Stage {
    let mut v = serde_json::to_value(stage_with(18_000, Some(far_boss()), vec![])).unwrap();
    v["player"]["evade"]["charges"] = json!(charges);
    if let Some(max) = max_charges {
        v["player"]["evade"]["max_charges"] = json!(max);
    }
    serde_json::from_value(v).expect("the test stage parses")
}

fn view(e: &Engine) -> EvadeView {
    e.snapshot().player.evade.clone()
}

/// Presses the evade on every odd tick from 1 to `ticks` (let go in between, so each is a
/// press), and returns the ticks on which it evaded.
fn press_every_other_tick(e: &mut Engine, ticks: u32) -> Vec<u32> {
    let mut evaded = Vec::new();
    for tick in 1..=ticks {
        e.step(if tick % 2 == 1 {
            evading(127, 0)
        } else {
            idle()
        });
        if e.events().contains(&DomainEvent::Evaded) {
            evaded.push(tick);
        }
    }
    evaded
}

#[test]
fn a_stage_without_charges_holds_one() {
    let e = running(vec![]);
    let v = view(&e);
    assert_eq!((v.charges, v.ready, v.recovering.len()), (1, 1, 0));
}

#[test]
fn a_run_starts_with_its_charges_not_its_most() {
    let e = engine(&charged(2, Some(3)));
    let v = view(&e);
    assert_eq!((v.charges, v.ready), (2, 2));
}

#[test]
fn three_charges_give_three_evades_in_a_row_then_none() {
    let mut e = engine(&charged(3, None));
    // Ticks 1, 3 and 5 spend the three charges, each during the dash before it; the press on
    // tick 7 finds none ready.
    assert_eq!(press_every_other_tick(&mut e, 8), [1, 3, 5]);
    let v = view(&e);
    assert_eq!((v.charges, v.ready), (3, 0));
}

#[test]
fn one_charge_ignores_a_press_during_its_dash() {
    let mut e = running(vec![]);
    assert_eq!(press_every_other_tick(&mut e, EVADE_TICKS), [1]);
}

#[test]
fn each_spent_charge_recovers_on_its_own_timer_from_its_press() {
    let mut e = engine(&charged(3, None));
    press_every_other_tick(&mut e, 5);
    // Spent on ticks 1, 3 and 5: after tick 5 they have 36, 38 and 40 ticks left, in the order
    // they were spent.
    assert_eq!(
        view(&e).recovering,
        [EVADE_COOLDOWN - 4, EVADE_COOLDOWN - 2, EVADE_COOLDOWN]
    );
    let mut ready = Vec::new();
    let mut last = 0;
    for tick in 6..=EVADE_COOLDOWN + 6 {
        e.step(idle());
        let now = view(&e).ready;
        if now != last {
            ready.push((tick, now));
            last = now;
        }
    }
    // Back one by one, each a cooldown after its own press: ticks 41, 43 and 45.
    assert_eq!(
        ready,
        [
            (EVADE_COOLDOWN + 1, 1),
            (EVADE_COOLDOWN + 3, 2),
            (EVADE_COOLDOWN + 5, 3)
        ]
    );
    assert!(view(&e).recovering.is_empty());
}

#[test]
fn a_press_during_a_dash_starts_the_next_dash_from_where_it_is() {
    let mut e = engine(&charged(2, None));
    let step = EVADE_DISTANCE / EVADE_TICKS as i32;
    e.step(evading(127, 0));
    e.step(moving(-127, 0));
    assert_eq!(player_at(&e), (START_X + 2 * step, START_Y));
    // The second dash goes up the stick's new direction, from two ticks into the first.
    e.step(evading(0, -127));
    assert_eq!(player_at(&e), (START_X + 2 * step, START_Y - step));
    assert_eq!(view(&e).dash_left, EVADE_TICKS - 1);
    for _ in 1..EVADE_TICKS {
        e.step(idle());
    }
    assert_eq!(
        player_at(&e),
        (START_X + 2 * step, START_Y - EVADE_DISTANCE),
        "the whole second dash, and nothing more of the first"
    );
    assert_eq!(view(&e).dash_left, 0);
}

#[test]
fn a_chained_dash_is_invulnerable_from_its_press() {
    let mut e = engine(&charged(2, None));
    e.step(evading(127, 0));
    for _ in 0..5 {
        e.step(moving(127, 0));
    }
    e.step(evading(127, 0));
    assert_eq!(e.snapshot().player.invulnerable, EVADE_INVULNERABLE);
}

#[test]
fn the_snapshot_counts_the_charges_and_the_dash() {
    let mut e = engine(&charged(2, None));
    let v = view(&e);
    assert_eq!(
        (v.charges, v.ready, v.recovering.clone(), v.dash_left),
        (2, 2, vec![], 0)
    );
    e.step(evading(127, 0));
    let v = view(&e);
    assert_eq!(
        (v.charges, v.ready, v.recovering.clone(), v.dash_left),
        (2, 1, vec![EVADE_COOLDOWN], EVADE_TICKS - 1)
    );
}

#[test]
fn a_charge_with_no_cooldown_is_ready_again_at_once() {
    let mut v = serde_json::to_value(charged(1, None)).unwrap();
    v["player"]["evade"]["cooldown_ticks"] = json!(0);
    let mut e = engine(&serde_json::from_value(v).unwrap());
    // Every press evades, each during the dash before it.
    assert_eq!(press_every_other_tick(&mut e, 5), [1, 3, 5]);
    assert_eq!(view(&e).ready, 1);
}

#[test]
fn charges_absent_from_a_stage_write_nothing_back() {
    // A stage that never named charges keeps its JSON, and so its parsed-stage hash (ADR-046).
    let v = serde_json::to_value(stage_with(18_000, Some(far_boss()), vec![])).unwrap();
    let evade = v["player"]["evade"].as_object().unwrap();
    assert!(!evade.contains_key("charges") && !evade.contains_key("max_charges"));
    let v = serde_json::to_value(charged(2, Some(3))).unwrap();
    assert_eq!(v["player"]["evade"]["charges"], json!(2));
    assert_eq!(v["player"]["evade"]["max_charges"], json!(3));
}
