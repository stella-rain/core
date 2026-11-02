//! Big boss patterns (ADR-103): a boss's attack step marked `big` is announced in the snapshot
//! while the timed steps before it run, and the boss's call of its elites is a big pattern
//! counted from the boss's arrival, not from the run's first tick.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::{EntityKind, PatternView};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error};

fn aimed(big: Option<bool>) -> Value {
    let mut step = json!({ "type": "attack",
        "attack": { "preset": { "id": "aimed_single", "args": [15] } } });
    if let Some(big) = big {
        step["big"] = json!(big);
    }
    step
}

/// A boss that waits `wait` ticks, makes the attack and waits again, forever.
fn timed(wait: u32, big: Option<bool>) -> Stage {
    let mut boss = far_boss();
    boss["phases"] = json!([{ "timeline": [
        { "type": "wait", "ticks": wait }, aimed(big),
        { "type": "wait", "ticks": 600 }, { "type": "loop" }
    ] }]);
    stage_full(18_000, Some(boss), vec![], vec![])
}

fn patterns(e: &Engine) -> Vec<PatternView> {
    e.snapshot().patterns
}

fn view(id: &str, total: u32, remaining: u32) -> PatternView {
    PatternView {
        id: id.into(),
        total,
        remaining,
    }
}

fn none() -> Vec<PatternView> {
    Vec::new()
}

#[test]
fn a_big_pattern_is_announced_with_its_ticks_counting_down_and_dropped_when_it_starts() {
    let mut e = Engine::new(&timed(30, Some(true))).expect("valid");
    for tick in 1..=30 {
        e.step(&[Input::default()]);
        assert_eq!(
            patterns(&e),
            [view("aimed_single", 30, 31 - tick)],
            "tick {tick}"
        );
    }
    e.step(&[Input::default()]);
    assert_eq!(patterns(&e), none(), "it starts on tick 31");
    let boss = e
        .snapshot()
        .entities
        .into_iter()
        .find(|v| v.kind == EntityKind::Boss);
    assert!(boss.is_some_and(|b| b.attack.is_some()), "the boss attacks");
}

#[test]
fn a_pattern_that_is_not_big_is_never_announced() {
    for big in [None, Some(false)] {
        let mut e = Engine::new(&timed(30, big)).expect("valid");
        for _ in 0..40 {
            e.step(&[Input::default()]);
            assert_eq!(patterns(&e), none(), "{big:?}");
        }
    }
}

#[test]
fn a_big_pattern_counts_the_timed_steps_before_it() {
    let mut boss = far_boss();
    boss["phases"] = json!([{ "timeline": [
        { "type": "wait", "ticks": 20 }, { "type": "telegraph", "ticks": 10 }, aimed(Some(true))
    ] }]);
    let mut e = Engine::new(&stage_full(18_000, Some(boss), vec![], vec![])).expect("valid");
    e.step(&[Input::default()]);
    assert_eq!(patterns(&e), [view("aimed_single", 30, 30)]);
    for _ in 0..24 {
        e.step(&[Input::default()]);
    }
    assert_eq!(patterns(&e), [view("aimed_single", 30, 6)]);
}

#[test]
fn only_a_preset_can_be_big() {
    let mut boss = far_boss();
    boss["phases"] = json!([{ "timeline": [{ "type": "attack", "big": true, "attack": { "inline": [
        { "kind": "wind_up", "ticks": 10 }
    ] } }] }]);
    let stage = stage_full(600, Some(boss), vec![], vec![]);
    let issues: Vec<String> = match validate::validate(&stage) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("{e:?}"),
    };
    assert_eq!(issues, ["$.boss.phases[0].timeline[0].big"]);
}

// --- The call of the elites ---------------------------------------------------------------

fn calling(hp: u32, call: Value) -> Stage {
    let mut boss = boss_at(hp, 100_000);
    boss["call_elites"] = call;
    stage_full(18_000, Some(boss), vec![], vec![])
}

fn calls(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
}

#[test]
fn the_call_is_a_big_pattern_counting_down_from_the_boss_s_arrival() {
    let mut e = Engine::new(&calling(1_000_000, json!({ "after_ticks": 60 }))).expect("valid");
    let mut seen = Vec::new();
    for tick in 1..=61 {
        e.step(&[Input::default()]);
        seen.extend(e.events().iter().map(|ev| (tick, ev.clone())));
        let expected = if tick <= 60 {
            vec![view("call_elites", 60, 61 - tick)]
        } else {
            none()
        };
        assert_eq!(patterns(&e), expected, "tick {tick}");
    }
    assert_eq!(calls(&seen), [61]);
}

#[test]
fn a_call_by_the_share_of_hit_points_is_unchanged_and_drops_the_announcement() {
    // 100 hp and 10 a shot: below 70% after the 4th hit, called on the tick after.
    let call = json!({ "after_ticks": 6000, "below_pct": 70 });
    let mut e = Engine::new(&calling(100, call)).expect("valid");
    let events = fire_until(&mut e, idle(), 600, |e| {
        e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
    });
    let hits: Vec<u32> = ticks_of(&events, |ev| {
        matches!(ev, DomainEvent::Hit { damage: 10, .. })
    });
    assert_eq!(calls(&events), [hits[3] + 1]);
    assert_eq!(patterns(&e), none());
}

#[test]
fn a_call_at_once_is_never_announced() {
    let mut e = Engine::new(&calling(1_000_000, json!({ "after_ticks": 0 }))).expect("valid");
    e.step(&[Input::default()]);
    assert_eq!(patterns(&e), none());
}

// --- The golem's slam -------------------------------------------------------------------

/// The trial's stage with the golem's phase 1 alone, as the stage file has it: a copy of the
/// timeline would not show what the bundled file says.
fn golem_phase_one() -> Stage {
    let mut stage: Value = serde_json::from_slice(common::bundled_file("trial")).unwrap();
    let phases = stage["boss"]["phases"].as_array_mut().unwrap();
    let phase = phases.remove(1);
    assert!(
        phase.to_string().contains("\"elite_slam\""),
        "phase 1 holds the slam"
    );
    *phases = vec![phase];
    validate::parse_and_validate(&serde_json::to_vec(&stage).unwrap()).expect("valid")
}

#[test]
fn the_golem_s_slam_is_announced_for_its_telegraph_before_it_starts() {
    let mut e = Engine::new(&golem_phase_one()).expect("valid");
    let slams = |e: &Engine| -> Vec<PatternView> {
        e.snapshot()
            .patterns
            .into_iter()
            .filter(|p| p.id == "elite_slam".into())
            .collect()
    };
    for tick in 1..=60 {
        e.step(&[Input::default()]);
        assert_eq!(
            slams(&e),
            [view("elite_slam", 60, 61 - tick)],
            "tick {tick}"
        );
    }
    e.step(&[Input::default()]);
    assert_eq!(slams(&e), none(), "it starts on tick 61");
}
