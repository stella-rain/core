//! The party cases (ADR-SIM-7): the worst-case tick with three players (`benches/worst_tick_3p.rs`)
//! and a rollback (`benches/rollback.rs`): an `Engine::clone` restored plus 8 ticks, on the heaviest
//! tick of the three-player dense minute. The checks that the first reaches the caps with every
//! slot hitting, and the pins of both, are in `tests/bench_fixture_pins.rs`; the allocator counts
//! in `tests/alloc.rs`.
//!
//! The tick case is `worst_case::warmed_up` (420 ticks) with slots 1 and 2 joined and one step
//! with both pressing Main, so that the measured tick 422 is the first on which they land hits.
//! The rollback case is the bundled `time_limit` mode of `worst_minute_case`, started for three
//! players: slot 0 plays the solo script, slot 1 the same on the other parity, slot 2 as slot 0,
//! and every open offer takes option 0 in all three slots.
//!
//! The struct is named `Case` and the rollback takes the saved `Engine` itself, so that the drop
//! guard (`.github/scripts/bench_drop_guard.py`, which matches `Engine`, `Run` and `Case`) covers
//! both.

use super::worst_case;
use super::worst_minute_case::{self, MINUTE_START, MINUTE_TICKS, input_at};
use crate::engine::{Engine, Load};
use crate::id::PlayerIdx;
use crate::input::Input;
use crate::run_setup::RunSetup;

/// The ticks played before the heaviest tick of the three-player dense minute (`window` finds
/// it; `tests/bench_fixture_pins.rs` pins it).
pub const PEAK_TICK_3P: u32 = 10_441;

/// The ticks of a rollback: the window of ADR-SIM-7's rollback, replayed after the restore.
pub const ROLLBACK_TICKS: u32 = 8;

/// The run's seed: that of the solo dense minute.
const SEED: u64 = 1;

/// A tick ready to measure: a three-player engine one tick before it, and the three inputs.
pub struct Case {
    pub engine: Engine,
    pub inputs: [Input; 3],
}

/// Main held on odd ticks: the input of slots 1 and 2 in the tick case.
fn tap(tick: u32) -> Input {
    Input {
        main: tick % 2 == 1,
        ..Input::default()
    }
}

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot of the party")
}

/// The worst-case tick with three players (`benches/worst_tick_3p.rs`).
pub fn worst_tick_3p() -> Case {
    let mut engine = worst_case::warmed_up();
    engine.join(slot(1)).expect("slot 1 joins");
    engine.join(slot(2)).expect("slot 2 joins");
    let at = worst_case::WARMUP_TICKS + 1;
    // The joiners' first press does nothing on the tick it is made, so it is made one tick early.
    engine.step(&[worst_case::input(at), tap(at), tap(at)]);
    let at = at + 1;
    Case {
        engine,
        inputs: [worst_case::input(at), tap(at), tap(at)],
    }
}

/// The measured body of the tick bench: one `Engine::step` and the engine handed back, so the
/// bench function drops nothing inside the counted region. `tests/alloc.rs` counts this same
/// function.
#[inline]
pub fn tick(case: Case) -> Engine {
    let Case { mut engine, inputs } = case;
    engine.step(&inputs);
    engine
}

/// The three inputs stepped at `tick`: slot 0 plays `input_at`, slot 1 the other parity, slot 2
/// as slot 0.
pub fn minute_inputs(tick: u32) -> [Input; 3] {
    [input_at(tick), input_at(tick + 1), input_at(tick)]
}

/// Answers the open offers (option 0 in every slot), then steps the tick.
fn play_tick(engine: &mut Engine) {
    while engine.offer_open() {
        engine.step(&[Input::picking(worst_minute_case::OFFER_CHOICE); 3]);
    }
    engine.step(&minute_inputs(engine.tick()));
}

/// The three-player dense-minute run, `ticks` ticks in.
fn fast_forwarded(ticks: u32) -> Engine {
    let setup = RunSetup {
        mode: "time_limit".into(),
        seed: SEED,
        ..RunSetup::default()
    };
    let mut engine = Engine::start_party_with(&setup, &worst_minute_case::content(), 3)
        .expect("the three-player run starts");
    for _ in 0..ticks {
        play_tick(&mut engine);
    }
    engine
}

/// The engine the rollback restores: the run `PEAK_TICK_3P` ticks in, with no offer open.
pub fn saved() -> Engine {
    fast_forwarded(PEAK_TICK_3P)
}

/// The measured body of the rollback bench: a clone of `saved`, `ROLLBACK_TICKS` ticks played on
/// it (the first is the peak tick), and both engines handed back so that nothing is dropped
/// inside the counted region.
#[inline]
pub fn rollback(saved: Engine) -> (Engine, Engine) {
    let mut engine = saved.clone();
    for _ in 0..ROLLBACK_TICKS {
        play_tick(&mut engine);
    }
    (engine, saved)
}

fn weight(load: Load) -> u64 {
    [
        load.enemies,
        load.tasks,
        load.zones,
        load.orbs,
        load.emitter_steps,
    ]
    .iter()
    .map(|&n| u64::from(n))
    .sum()
}

/// Scans the dense minute with three players for its heaviest tick, as
/// `worst_minute_case::worst_minute_window` does for one: the minute's start and the number of
/// ticks played before the heaviest tick (the first wins a tie).
pub fn window() -> (u32, u32) {
    let mut engine = fast_forwarded(MINUTE_START);
    let mut heaviest = (0, MINUTE_START);
    for i in 0..MINUTE_TICKS {
        play_tick(&mut engine);
        let w = weight(engine.load());
        if w > heaviest.0 {
            heaviest = (w, MINUTE_START + i);
        }
    }
    (MINUTE_START, heaviest.1)
}
