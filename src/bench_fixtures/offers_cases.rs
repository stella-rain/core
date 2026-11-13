//! The bench cases as they were before they moved to equipment mode: the engines of the tests that
//! need an offers-mode run or the frozen rules of `sim_version` 1 on the same stages (ADR-SIM-5,
//! ADR-SIM-7, ADR-REPL-2). `Engine::new_equipped` builds the live rules only, so a frozen-v1
//! engine and an engine that opens star offers cannot come from the equipped cases.
//!
//! Test-only: the benches never call it. Each builder is the code of the fixture it replaces, so
//! the numbers of the tests that read it stay as they were (`tests::*_keep_their_state_hash`).

use super::common::{self, on_hit};
use super::{Case, hit_case, time_limit_case, worst_case};
use crate::engine::{Engine, Outcome};
use crate::stage::Stage;
use serde_json::json;

/// The worst case's stage as it was: the equipment stage with, first in its list, a wave of
/// `PICKS` one-hit-point enemies whose stars give the runes (its `levels` and `stars` are the
/// equipment stage's).
pub(crate) fn worst_stage() -> Stage {
    const PICKS: u16 = 8;
    let mut stage = worst_case::stage();
    stage.waves.insert(
        0,
        serde_json::from_value(common::wave(
            1,
            PICKS,
            6,
            common::enemy(1, common::hold(common::START_X, on_hit::NEAR_Y), json!([])),
            common::START_X,
            on_hit::NEAR_Y,
        ))
        .expect("the wave parses"),
    );
    stage
}

/// The worst case's content as it was: the random rune and Rune Haste rewards to pick from, no
/// `equipment` object.
pub(crate) fn worst_content() -> crate::run_content::RunContent {
    on_hit::content(
        vec![on_hit::random_rune(), worst_case::haste()],
        None,
        worst_case::runes(),
    )
}

/// The engine of the worst case at the start of the scripted run: every rune and Rune Haste taken
/// from the offers of the first eight enemies' stars.
fn worst_runes_taken() -> Engine {
    let mut engine = on_hit::with(&worst_stage(), &worst_content());
    on_hit::take_all(&mut engine);
    on_hit::take(&mut engine, worst_case::HASTE);
    assert!(
        engine.tick() < worst_case::WALK_FROM,
        "the runes were taken by tick {}",
        engine.tick()
    );
    engine
}

/// The offers-mode worst case, `WARMUP_TICKS` ticks in.
pub(crate) fn worst_warmed_up() -> Engine {
    let mut engine = worst_runes_taken();
    for tick in engine.tick() + 1..=worst_case::WARMUP_TICKS {
        engine.step(&[
            worst_case::input(tick),
            Default::default(),
            Default::default(),
        ]);
        assert_eq!(engine.outcome(), Outcome::Running);
    }
    engine
}

/// The offers-mode worst tick.
pub(crate) fn worst_tick() -> Case {
    Case {
        engine: worst_warmed_up(),
        input: worst_case::input(worst_case::WARMUP_TICKS + 1),
    }
}

fn hit_warmed(stage: &Stage) -> Engine {
    let mut engine = Engine::new_with(stage, &common::design_content()).expect("the case stage");
    for tick in 1..=hit_case::WARMUP_TICKS {
        engine.step(&[
            hit_case::input(tick),
            Default::default(),
            Default::default(),
        ]);
    }
    engine
}

/// The offers-mode hit tick, the crowd in a clump.
pub(crate) fn hit_tick() -> Case {
    Case {
        engine: hit_warmed(&hit_case::stage()),
        input: hit_case::input(hit_case::WARMUP_TICKS + 1),
    }
}

/// The offers-mode time-limit tick at the window the case had in offers mode (tick 1,027).
pub(crate) fn time_limit_tick() -> Case {
    const WARMUP: u32 = 1_026;
    let mut engine = Engine::new_with(&time_limit_case::stage(), &time_limit_case::content())
        .expect("the run starts");
    for tick in 1..=WARMUP {
        engine.step(&[
            time_limit_case::input(tick),
            Default::default(),
            Default::default(),
        ]);
    }
    Case {
        engine,
        input: time_limit_case::input(WARMUP + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// The first 16 hex of the SHA-256 over the old worst stage's JSON.
    const WORST_STAGE_DIGEST: &str = "fe001eada0c1a953";

    #[test]
    fn the_old_worst_stage_keeps_its_digest() {
        let json = serde_json::to_vec(&worst_stage()).expect("the stage serialises");
        let hex: String = Sha256::digest(&json)
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, WORST_STAGE_DIGEST);
    }
}
