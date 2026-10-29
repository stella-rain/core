//! The poison and burn runes (ADR-099 points 2, 3, 7 and 8; ADR-100 point 1; ADR-084 point 8):
//! damage-over-time runes as bleed is one, on the same status. A combo step's hit puts the
//! rune's status on the enemy hit, `poison` Dendro or `burn` Pyro, or a stack more of it up to
//! the stack maximum; each kind is a status of its own, with its own stacks. Every interval it
//! deals its damage per stack times its stacks as a hit of its type: the target's match-up
//! and defence at the tick. Its damage gives no expertise and fires no rune. Bleed's own
//! behaviour is `bleed.rs`'s. Every number here is worked out by hand; the on-hit runes are
//! held too, as the random rune draws all seven, but their attacks land far from every target.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, verify_run_with};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::runes::Runes;
use stella_rain_core::snapshot::DamageOverTimeView;

use crate::common;

use common::on_hit::*;
use common::run::{run_input, trial_setup};
use common::*;

/// The two runes, their type, and a type they are not.
const RUNES: [(&str, &str, &str); 2] = [("poison", "dendro", "slash"), ("burn", "pyro", "slash")];

/// A runes file whose dot entries are `dots` (the rest never tick), every on-hit rune far away.
fn runes_of_dots(dots: Vec<Value>) -> Runes {
    let mut all = far_dots();
    for d in dots {
        all = far_dots_but_in(all, d);
    }
    runes_with(
        vec![
            far("explosion"),
            far("thunder"),
            far("boomerang"),
            far("homing_missiles"),
        ],
        all,
    )
}

fn far_dots_but_in(all: Vec<Value>, entry: Value) -> Vec<Value> {
    all.into_iter()
        .map(|d| {
            if d["rune"] == entry["rune"] {
                entry.clone()
            } else {
                d
            }
        })
        .collect()
}

/// The stage with seven small enemies and the tough target, every rune taken from their stars.
fn holding(runes: Runes, target: Value) -> Engine {
    let mut e = with(
        &stage(7, target),
        &content(vec![random_rune()], None, runes),
    );
    take_all(&mut e);
    e
}

/// The statuses named `name` on `id`.
fn statuses(e: &Engine, id: EntityId, name: &str) -> Vec<DamageOverTimeView> {
    e.snapshot()
        .entities
        .into_iter()
        .find(|en| en.id == id)
        .map(|en| en.damage_over_time)
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.status.as_str() == name)
        .collect()
}

fn presses(start: u32, at: &'static [u32]) -> impl Fn(u32) -> Input {
    move |t| Input {
        main: at.iter().any(|d| start + d == t),
        ..idle()
    }
}

#[test]
fn a_combo_step_hit_applies_the_status() {
    for (name, _, _) in RUNES {
        let mut e = holding(
            runes_of_dots(vec![dot(name, 30, 180, 3, 5)]),
            tough(0, None, None),
        );
        let target = wait_for_target(&mut e);
        assert!(statuses(&e, target, name).is_empty());
        let start = e.tick();
        e.step(once(start)(start));
        assert!(
            e.events().iter().any(|ev| matches!(
                ev,
                DomainEvent::StatusApplied { target: t, status, stacks: 1, duration_ticks: 180 }
                    if *t == target && status.as_str() == name
            )),
            "{name}"
        );
        let view = statuses(&e, target, name);
        assert_eq!(view.len(), 1, "{name}");
        assert_eq!(
            (view[0].stacks, view[0].ticks_left, view[0].next_tick_in),
            (1, 180, 30),
            "{name}"
        );
    }
}

#[test]
fn it_ticks_at_its_interval_for_its_duration_in_its_type() {
    // 20 a stack: neutral 20; weak to its type 20 × 1.2 = 24; weak to another type 20.
    for (name, ty, other) in RUNES {
        let run = |profile: Option<Value>| {
            let mut e = holding(
                runes_of_dots(vec![dot(name, 30, 180, 20, 5)]),
                tough(0, profile, None),
            );
            let target = wait_for_target(&mut e);
            let start = e.tick();
            let seen = hits(&mut e, target, 240, once(start));
            let hit = start + 1;
            assert_eq!(of(&seen, 10), [hit], "{name}: the step's hit");
            let ticks: Vec<u32> = seen
                .iter()
                .filter(|(t, _)| *t != hit)
                .map(|(t, _)| *t)
                .collect();
            assert_eq!(
                ticks,
                (1..=6).map(|k| hit + 30 * k).collect::<Vec<_>>(),
                "{name}: 180 / 30 ticks"
            );
            assert!(statuses(&e, target, name).is_empty(), "{name}: run out");
            seen.iter()
                .filter(|(t, _)| *t != hit)
                .map(|(_, d)| *d)
                .collect::<Vec<u32>>()
        };
        assert_eq!(run(None), [20; 6], "{name} neutral");
        assert_eq!(run(Some(json!({ "weak": [ty] }))), [24; 6], "{name} weak");
        assert_eq!(
            run(Some(json!({ "weak": [other] }))),
            [20; 6],
            "{name}: another type's weakness does not raise it"
        );
    }
}

#[test]
fn it_stacks_to_its_maximum_and_each_hit_starts_its_duration_again() {
    // Hits at 0, 10, 20 and 40 (ticks after the first), 3 stacks at most: 1, 2, 3, 3 stacks,
    // each application with its 180 ticks; the timer runs on, so the damage comes at 30, 60, ...
    // of 4 a stack: 12 once three stacks are there (at 30 only 3 are).
    for (name, _, _) in RUNES {
        let mut e = holding(
            runes_of_dots(vec![dot(name, 30, 180, 4, 3)]),
            tough(0, None, None),
        );
        let target = wait_for_target(&mut e);
        let start = e.tick();
        let mut stacks = Vec::new();
        let mut damage = Vec::new();
        for _ in 0..=240 {
            let t = e.tick();
            e.step(presses(start, &[0, 10, 20, 40])(t));
            for ev in e.events() {
                match ev {
                    DomainEvent::StatusApplied {
                        target: id,
                        status,
                        stacks: n,
                        duration_ticks,
                    } if *id == target && status.as_str() == name => {
                        assert_eq!(*duration_ticks, 180);
                        stacks.push(*n);
                    }
                    DomainEvent::Hit {
                        target: id,
                        damage: d,
                        ..
                    } if *id == target && *d != 10 => damage.push((e.tick() - start - 1, *d)),
                    _ => {}
                }
            }
        }
        assert_eq!(stacks, [1, 2, 3, 3], "{name}");
        // The first tick at 30 has the three stacks: 12; and every tick after, until the last
        // application's 180 ticks run out at 220.
        let ticks: Vec<u32> = damage.iter().map(|d| d.0).collect();
        assert_eq!(
            ticks,
            [30, 60, 90, 120, 150, 180, 210],
            "{name}: {damage:?}"
        );
        assert!(damage.iter().all(|d| d.1 == 12), "{name}: {damage:?}");
    }
}

#[test]
fn bleed_poison_and_burn_are_three_statuses_each_with_its_own_stacks() {
    let mut e = holding(
        runes_of_dots(vec![
            dot("bleed", 30, 180, 20, 5),
            dot("poison", 30, 180, 30, 5),
            dot("burn", 30, 180, 40, 5),
        ]),
        tough(0, None, None),
    );
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 100, presses(start, &[0, 10]));
    for (name, per_stack) in [("bleed", 20), ("poison", 30), ("burn", 40)] {
        let v = statuses(&e, target, name);
        assert_eq!(v.len(), 1, "{name}");
        assert_eq!(v[0].stacks, 2, "{name}: its own two stacks");
        assert_eq!(
            of(&seen, per_stack * 2).len(),
            3,
            "{name}: 3 ticks of two stacks by tick 90: {seen:?}"
        );
    }
}

#[test]
fn their_damage_gives_no_expertise_and_fires_no_rune() {
    // The target gives 1 point a hit: the steps', not the ticks'.
    let mut e = holding(
        runes_of_dots(vec![
            dot("poison", 10, 100, 5, 2),
            dot("burn", 10, 100, 5, 2),
        ]),
        tough(0, None, Some(1)),
    );
    let target = wait_for_target(&mut e);
    let before = e.snapshot().expertise.unwrap().points;
    let start = e.tick();
    let seen = hits(&mut e, target, 120, once(start));
    assert!(seen.len() > 10, "{seen:?}");
    assert_eq!(e.snapshot().expertise.unwrap().points, before + 1);
}

#[test]
fn the_state_hash_holds_poison_and_burn() {
    for (name, _, _) in RUNES {
        let mut a = holding(
            runes_of_dots(vec![dot(name, 30, 180, 3, 5)]),
            tough(0, None, None),
        );
        let mut b = holding(
            runes_of_dots(vec![dot(name, 45, 180, 3, 5)]),
            tough(0, None, None),
        );
        wait_for_target(&mut a);
        wait_for_target(&mut b);
        assert_eq!(a.state_hash(), b.state_hash(), "{name}");
        let start = a.tick();
        a.step(once(start)(start));
        b.step(once(start)(start));
        assert_ne!(a.state_hash(), b.state_hash(), "{name}");
    }
}

#[test]
fn a_run_holding_poison_and_burn_records_and_verifies() {
    // The trial with a random rune and one stat reward, the hidden candidate always taken, and
    // the bundled poison and burn.
    let content = content(
        vec![
            random_rune(),
            json!({
                "id": "might", "rarity": "common", "effect": "attack_pct", "amount": 5,
                "limit": 20
            }),
        ],
        None,
        Runes::bundled().clone(),
    );
    // A seed whose run draws poison and burn among the seven runes: the trial has stars for about four.
    let setup = RunSetup {
        seed: 20_261_012,
        ..trial_setup()
    };
    let mut recorder = RunRecorder::start_with(&setup, &content).unwrap();
    let (mut poisoned, mut burnt) = (false, false);
    for t in 0..5_400 {
        while recorder.engine().offer_open() {
            let offer = recorder.engine().snapshot().progress.and_then(|p| p.offer);
            let rune = offer.and_then(|o| o.candidates.iter().position(|c| c.hidden));
            recorder
                .choose(u32::try_from(rune.unwrap_or(0)).unwrap())
                .unwrap();
        }
        let input: Input = run_input(recorder.engine(), t);
        if recorder.step(input).is_err() {
            break;
        }
        for ev in recorder.engine().events() {
            if let DomainEvent::StatusApplied { status, .. } = ev {
                poisoned |= status.as_str() == "poison";
                burnt |= status.as_str() == "burn";
            }
        }
    }
    assert!(poisoned && burnt, "poison and burn were applied in the run");
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
}
