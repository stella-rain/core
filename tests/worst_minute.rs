//! The densest real minute of the `time_limit` corpus run (BR1e, core#317, D-BUDGET A in DS49):
//! `bench_fixtures::worst_minute_case` names where the benches start counting, and this finds the
//! same place from the run itself, so a corpus that moves the minute fails here until the two
//! constants are re-pinned. The same minute's peak heap stays at or below 16 MiB.
//!
//! "Heaviest" is the sum of the five fields of `Load` (enemies, attack tasks, zones, orbs and
//! emitter steps), unweighted; the first tick or window wins a tie.

use std::fs;
use std::path::Path;

use stella_rain_core::bench_fixtures::worst_minute_case::{
    MINUTE_TICKS, WORST_MINUTE_START, WORST_TICK_IN_MINUTE,
};
use stella_rain_core::engine::{Engine, Load};
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{CommandKind, RunReplay, TickCommand};

use crate::alloc::{live_bytes, peak_bytes, reset_peak};

/// The most heap the engine may hold at once in the worst minute (16 MiB).
const PEAK_HEAP_LIMIT: i64 = 16 * 1024 * 1024;

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

struct Run {
    replay: RunReplay,
    inputs: Vec<Input>,
    commands: Vec<TickCommand>,
}

fn run() -> Run {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/runs/v0/time_limit");
    let replay = RunReplay::from_bytes(&fs::read(path.join("run.replay")).unwrap()).unwrap();
    let inputs = replay.unpack_inputs(u32::MAX).unwrap();
    let commands = replay.unpack_commands().unwrap();
    Run {
        replay,
        inputs,
        commands,
    }
}

/// Plays ticks `from..to` of the run on `engine`, calling `after` with the index of each tick
/// and the engine after it.
fn play(
    run: &Run,
    engine: &mut Engine,
    from: usize,
    to: usize,
    mut after: impl FnMut(usize, &Engine),
) {
    let mut next = run
        .commands
        .iter()
        .skip_while(|c| c.tick < from as u32)
        .peekable();
    for (i, input) in run.inputs.iter().enumerate().take(to).skip(from) {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            match c.kind {
                CommandKind::Choose => engine.choose(c.index).unwrap(),
            }
        }
        engine.step(*input);
        after(i, engine);
    }
}

#[test]
fn the_constants_name_the_densest_minute_of_the_run() {
    let run = run();
    let mut engine = Engine::start_with(&run.replay.setup, RunContent::bundled()).unwrap();
    let mut loads = Vec::with_capacity(run.inputs.len());
    play(&run, &mut engine, 0, run.inputs.len(), |_, e| {
        loads.push(weight(e.load()))
    });
    let minute = MINUTE_TICKS as usize;
    assert!(loads.len() > minute, "the run is longer than a minute");

    let mut sum: u64 = loads[..minute].iter().sum();
    let (mut best, mut start) = (sum, 0);
    for from in 1..=loads.len() - minute {
        sum = sum - loads[from - 1] + loads[from + minute - 1];
        if sum > best {
            (best, start) = (sum, from);
        }
    }
    let window = &loads[start..start + minute];
    let heaviest = window.iter().max().unwrap();
    let tick = start + window.iter().position(|l| l == heaviest).unwrap();

    assert_eq!(
        (WORST_MINUTE_START as usize, WORST_TICK_IN_MINUTE as usize),
        (start, tick),
        "the densest minute (summed load {best}) starts at tick {start} and its heaviest tick \
         (load {heaviest}) is tick {tick}: re-pin the constants in \
         src/bench_fixtures/worst_minute_case.rs and the hashes in tests/bench_fixture_pins.rs"
    );
}

#[test]
fn the_densest_minute_holds_at_most_16_mib() {
    let run = run();
    let held = live_bytes();
    let mut engine = Engine::start_with(&run.replay.setup, RunContent::bundled()).unwrap();
    let (start, end) = (
        WORST_MINUTE_START as usize,
        (WORST_MINUTE_START + MINUTE_TICKS) as usize,
    );
    play(&run, &mut engine, 0, start, |_, _| {});
    reset_peak();
    play(&run, &mut engine, start, end, |_, _| {});
    let peak = peak_bytes() - held;
    eprintln!("peak heap of the densest minute: {peak} bytes");
    assert!(
        peak <= PEAK_HEAP_LIMIT,
        "peak heap {peak} bytes, limit {PEAK_HEAP_LIMIT}"
    );
}
