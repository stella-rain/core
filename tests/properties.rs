//! Property tests (ADR-019, ADR-021): invariants that must hold for any input sequence, run on
//! the stages of the replay corpus. `proptest` shrinks a failure to the shortest inputs that
//! still fail and saves the seed in `proptest-regressions/`; commit that file with the fix.
//!
//! Each property runs 64 cases; `PROPTEST_CASES=2000 cargo test --test properties` runs more.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use common::*;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use serde_json::json;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::Snapshot;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::limits::{FIELD_H, FIELD_MARGIN, FIELD_W};
use stella_rain_core::validate::parse_and_validate;

fn config() -> ProptestConfig {
    let mut config = ProptestConfig {
        // `tests/properties.proptest-regressions`, next to this file.
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..ProptestConfig::default()
    };
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 64;
    }
    config
}

// --- Generators ------------------------------------------------------------------------------

/// The stages of `tests/corpus/v0/`: agents, statuses, healing, bullets, a boss with a part,
/// a run that fails and one that times out.
fn corpus_stages() -> &'static [Stage] {
    static STAGES: OnceLock<Vec<Stage>> = OnceLock::new();
    STAGES.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/v0");
        let mut names: Vec<_> = fs::read_dir(&dir)
            .expect("the corpus directory")
            .map(|entry| entry.expect("a corpus entry").file_name())
            .collect();
        names.sort();
        assert!(!names.is_empty(), "the corpus has no cases");
        names
            .iter()
            .map(|name| {
                let bytes = fs::read(dir.join(name).join("stage.json")).expect("stage.json");
                parse_and_validate(&bytes).expect("a corpus stage is valid")
            })
            .collect()
    })
}

/// Mostly speeds a player can have, sometimes the whole `i16` range; `held` is mostly a real
/// skill slot, sometimes any byte, because the record is untrusted until it is replayed.
fn input() -> impl Strategy<Value = Input> {
    (
        prop_oneof![3 => -800i16..=800, 1 => any::<i16>()],
        prop_oneof![3 => -800i16..=800, 1 => any::<i16>()],
        any::<bool>(),
        prop::bool::weighted(0.75),
        prop_oneof![3 => 0u8..=3, 1 => any::<u8>()],
    )
        .prop_map(|(dx, dy, focus, fire, held)| Input {
            dx,
            dy,
            focus,
            fire,
            held,
        })
}

/// Runs of one input held for a while, up to 300 ticks in all: crossing the field at the main
/// character's speed takes over a hundred ticks, so single random inputs would never reach a
/// wall.
fn inputs() -> impl Strategy<Value = Vec<Input>> {
    prop::collection::vec((input(), 1usize..=60), 1..=12).prop_map(|runs| {
        runs.into_iter()
            .flat_map(|(input, ticks)| std::iter::repeat_n(input, ticks))
            .take(300)
            .collect()
    })
}

// --- Determinism (ADR-004, ADR-019) ----------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    #[test]
    fn two_runs_of_the_same_inputs_agree_on_every_tick(
        stage in 0..corpus_stages().len(),
        inputs in inputs(),
    ) {
        let stage = &corpus_stages()[stage];
        let (mut a, mut b) = (engine(stage), engine(stage));
        prop_assert_eq!(a.state_hash(), b.state_hash());
        for input in &inputs {
            a.step(*input);
            b.step(*input);
            prop_assert_eq!(a.state_hash(), b.state_hash(), "after tick {}", a.tick());
            prop_assert_eq!(a.outcome(), b.outcome(), "after tick {}", a.tick());
            prop_assert_eq!(a.events(), b.events(), "after tick {}", a.tick());
            prop_assert_eq!(a.snapshot(), b.snapshot(), "after tick {}", a.tick());
        }
    }

    #[test]
    fn a_clone_continues_like_the_original(
        stage in 0..corpus_stages().len(),
        inputs in inputs(),
        cut in any::<prop::sample::Index>(),
    ) {
        let mut original = engine(&corpus_stages()[stage]);
        let (before, after) = inputs.split_at(cut.index(inputs.len()));
        for input in before {
            original.step(*input);
        }
        let mut clone = original.clone();
        for input in after {
            original.step(*input);
            clone.step(*input);
            prop_assert_eq!(original.state_hash(), clone.state_hash(), "after tick {}", original.tick());
        }
    }
}

// --- Positions stay in bounds (ADR-020) ------------------------------------------------------

fn within(x: i32, y: i32, margin: i32) -> bool {
    (-margin..=FIELD_W + margin).contains(&x) && (-margin..=FIELD_H + margin).contains(&y)
}

proptest! {
    #![proptest_config(config())]

    /// The main character is clamped to the field. Everything else may spawn, and bullets may
    /// fly, up to the margin around it, and is gone beyond that.
    #[test]
    fn positions_stay_in_bounds(stage in 0..corpus_stages().len(), inputs in inputs()) {
        let mut e = engine(&corpus_stages()[stage]);
        for input in &inputs {
            e.step(*input);
            let s = e.snapshot();
            let player = s.player.at;
            prop_assert!(within(player.x.0, player.y.0, 0), "player {:?} on tick {}", player, s.tick);
            for entity in &s.entities {
                let at = entity.at;
                prop_assert!(within(at.x.0, at.y.0, FIELD_MARGIN), "entity {:?} on tick {}", entity, s.tick);
            }
            for bullet in &s.bullets {
                let at = bullet.at;
                prop_assert!(within(at.x.0, at.y.0, FIELD_MARGIN), "bullet {:?} on tick {}", bullet, s.tick);
            }
        }
    }
}

// --- Hit points (ADR-036) --------------------------------------------------------------------

/// `hp` is a `u32`, so it cannot go below zero; what can go wrong is going up. The main
/// character has entity ID 0.
fn hp_by_id(s: &Snapshot) -> BTreeMap<u32, u32> {
    let mut hp = BTreeMap::from([(0, s.player.hp)]);
    hp.extend(s.entities.iter().map(|e| (e.id.0, e.hp)));
    hp
}

proptest! {
    #![proptest_config(config())]

    /// An entity's hit points rise only on a tick with a `Healed` event for it, and never above
    /// what it started with.
    #[test]
    fn hp_rises_only_by_healing_and_never_above_its_start(
        stage in 0..corpus_stages().len(),
        inputs in inputs(),
    ) {
        let mut e = engine(&corpus_stages()[stage]);
        let mut before = hp_by_id(&e.snapshot());
        let mut start = before.clone();
        for input in &inputs {
            e.step(*input);
            let healed: Vec<u32> = e
                .events()
                .iter()
                .filter_map(|ev| match ev {
                    DomainEvent::Healed { target, .. } => Some(target.0),
                    _ => None,
                })
                .collect();
            let now = hp_by_id(&e.snapshot());
            for (id, hp) in &now {
                let cap = *start.entry(*id).or_insert(*hp);
                prop_assert!(*hp <= cap, "entity {} has {} hp, it started with {}, tick {}", id, hp, cap, e.tick());
                if let Some(was) = before.get(id) {
                    prop_assert!(
                        hp <= was || healed.contains(id),
                        "entity {} went from {} to {} hp with no heal, tick {}", id, was, hp, e.tick()
                    );
                }
            }
            before = now;
        }
    }
}

// --- Statuses expire (ADR-036) ---------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// One companion applies `atk_up` to the main character over and over, and a second fires a
    /// rule whenever the main character has it (only one rule fires per agent and tick, so the
    /// probe has an agent of its own). Agents act in entity-ID order and the probe comes after
    /// the caster, so it sees an application on the tick it happens: the status is seen from
    /// the tick of an application until `duration` ticks later, and on no other tick. Stacks go
    /// up to five, never beyond.
    #[test]
    fn a_status_is_seen_exactly_while_an_application_holds_it(
        duration in 1u32..=40,
        cooldown in 0u32..=30,
    ) {
        const TICKS: u32 = 150;
        let (player, caster, probe) = (EntityId(0), EntityId(2), EntityId(3)); // The boss is 1.
        let apply = json!([rule(always(), json!("player"), atk_up(10, duration), cooldown)]);
        let watch = json!([rule(
            json!({ "type": "has_status", "who": "player", "status": "atk_up" }),
            json!("self"),
            telegraph(),
            0
        )]);
        let stage = stage_full(
            18_000,
            Some(far_boss()),
            vec![],
            vec![
                companion(50, hold(46_080, 90_000), apply),
                companion(50, hold(46_080, 90_000), watch),
            ],
            vec![],
        );
        let mut e = engine(&stage);
        let events = run_until(&mut e, idle(), TICKS, |_| false);

        let applied: Vec<(u32, u16)> = events
            .iter()
            .filter_map(|(tick, ev)| match ev {
                DomainEvent::StatusApplied { target, stacks, .. } if *target == player => {
                    Some((*tick, *stacks))
                }
                _ => None,
            })
            .collect();
        prop_assert!(!applied.is_empty(), "the status was never applied by {:?}", caster);
        for (tick, stacks) in &applied {
            prop_assert!((1..=5).contains(stacks), "{} stacks on tick {}", stacks, tick);
        }

        let seen: Vec<u32> = events
            .iter()
            .filter_map(|(tick, ev)| match ev {
                DomainEvent::RuleFired { agent, .. } if *agent == probe => Some(*tick),
                _ => None,
            })
            .collect();
        let expected: Vec<u32> = (1..=TICKS)
            .filter(|tick| applied.iter().any(|(a, _)| *a <= *tick && *tick < a + duration))
            .collect();
        prop_assert_eq!(seen, expected, "applied on ticks {:?}", applied);
    }
}
