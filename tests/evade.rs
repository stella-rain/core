//! The evade's charges (ADR-074): a stage gives up to three, each spent charge recovers on its
//! own timer of the cooldown counted from its press, and a press during a dash with a charge
//! ready starts the next dash at once; a reward adds a ready charge, up to the stage's
//! `max_charges`. Every number here is worked out by hand from the rules' constants and
//! `common::evade()`.

use crate::common;
use stella_rain_core::version::Versions;

use common::*;
use serde_json::json;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{EvadeView, OfferOption};
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

// --- The extra-charge reward (ADR-074 §5) ----------------------------------------------------

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once.
const NEAR_Y: i32 = START_Y - 8_192;
/// A cooldown no test outlasts, so a spent charge is still recovering when a reward comes.
const LONG_COOLDOWN: u32 = 10_000;

/// A stage whose evade has `charges` and, if given, `max_charges`, a long cooldown and a dash
/// of one pixel (so the main character stays by its stars), with one level and `count` enemies
/// of 10 hp straight above it, one every 6 ticks from tick 1: each drops a common star it takes
/// at once.
fn starry(charges: u8, max_charges: Option<u8>, count: u16) -> Stage {
    let target = enemy(10, hold(START_X, NEAR_Y), json!([]));
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, count, 6, target, START_X, NEAR_Y)],
        vec![],
    );
    let mut v = serde_json::to_value(stage).unwrap();
    v["player"]["evade"]["charges"] = json!(charges);
    if let Some(max) = max_charges {
        v["player"]["evade"]["max_charges"] = json!(max);
    }
    v["player"]["evade"]["cooldown_ticks"] = json!(LONG_COOLDOWN);
    v["player"]["evade"]["distance"] = json!(256);
    v["levels"] = json!({ "xp": [1000] });
    v["stars"] = json!({
        "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
        "rerolls": 0, "banishes": 0,
        "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
    });
    serde_json::from_value(v).expect("the test stage parses")
}

/// A run of `stage` whose only reward, `extra`, adds `amount` charges, five times a run.
fn with_extra(stage: &Stage, amount: u32) -> Engine {
    let file = json!({ "rarities": common::rarities(), "rewards": [
        { "id": "extra", "rarity": "common", "effect": "evade_charge", "amount": amount,
          "limit": 5, "title": "Extra charge", "description": "" }
    ] });
    let rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test rewards validate");
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

/// Fires until an offer is open, then takes its first candidate, the only reward there is.
fn take(e: &mut Engine) {
    fire_until(e, idle(), 1200, Engine::offer_open);
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    assert_eq!(offer.candidates[0].reward.0, "extra");
    let option = OfferOption::Take { candidate: 0 };
    let index = offer.options.iter().position(|o| *o == option).unwrap();
    e.choose(u32::try_from(index).unwrap()).unwrap();
}

#[test]
fn the_extra_charge_reward_adds_a_charge_that_is_ready() {
    let mut e = with_extra(&starry(1, Some(3), 1), 1);
    take(&mut e);
    let v = view(&e);
    assert_eq!((v.charges, v.ready, v.recovering.len()), (2, 2, 0));
}

#[test]
fn the_new_charge_is_ready_while_a_spent_one_recovers() {
    let mut e = with_extra(&starry(1, Some(2), 1), 1);
    e.step(evading(0, 0));
    assert_eq!(view(&e).ready, 0);
    take(&mut e);
    let v = view(&e);
    assert_eq!((v.charges, v.ready, v.recovering.len()), (2, 1, 1));
    // The spent charge keeps its own timer, counted from its press on tick 1.
    assert_eq!(v.recovering, [LONG_COOLDOWN + 1 - e.tick()]);
    // And the new charge dashes at once.
    e.step(idle());
    e.step(evading(0, 0));
    assert!(e.events().contains(&DomainEvent::Evaded));
}

#[test]
fn the_extra_charge_reward_stops_at_the_stage_s_max_charges() {
    let mut e = with_extra(&starry(2, Some(3), 2), 2);
    take(&mut e);
    assert_eq!((view(&e).charges, view(&e).ready), (3, 3));
    take(&mut e);
    assert_eq!((view(&e).charges, view(&e).ready), (3, 3));
}

#[test]
fn without_max_charges_the_extra_charge_reward_adds_none() {
    let mut e = with_extra(&starry(2, None, 1), 1);
    take(&mut e);
    assert_eq!((view(&e).charges, view(&e).ready), (2, 2));
}
