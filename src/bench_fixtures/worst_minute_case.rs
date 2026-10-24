//! The densest real minute of the `time_limit` corpus run (BR1e, core#317, D-BUDGET A in DS49).
//! A one-tick bench misses wave spawns, elite minutes, the boss's arrival and death bursts, so
//! `benches/worst_minute.rs` counts the 3,600 ticks of the run's heaviest minute and
//! `benches/worst_minute_peak.rs` its heaviest tick. The run is the corpus's own,
//! `tests/corpus/runs/v0/time_limit/run.replay`, played from its start with its commands.
//!
//! "Heaviest" is the sum of the five fields of `Load` (enemies, attack tasks, zones, orbs, emitter
//! steps), unweighted; the first window or tick wins a tie. `worst_minute_window` finds both
//! places by replaying the run, so the benches (which call it in their uncounted setup) need no
//! constants: a change of the run's outcome moves the minute without touching bench input. Where
//! the minute is is pinned in `tests/worst_minute.rs`, and the state hashes there in
//! `tests/bench_fixture_pins.rs`.

use crate::engine::{Engine, Load};
use crate::input::Input;
use crate::run_content::RunContent;
use crate::run_replay::{CommandKind, RunReplay, TickCommand};

/// A minute: 60 s at 60 ticks a second.
pub const MINUTE_TICKS: u32 = 3_600;

const RUN_REPLAY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/corpus/runs/v0/time_limit/run.replay"
));

/// The corpus run, part way through: `play` takes the next ticks of its inputs and commands.
pub struct Run {
    pub engine: Engine,
    inputs: Vec<Input>,
    commands: Vec<TickCommand>,
    next_command: usize,
}

impl Run {
    /// The run, `ticks` ticks in.
    pub fn fast_forwarded(ticks: u32) -> Run {
        let replay = RunReplay::from_bytes(RUN_REPLAY).expect("the corpus run replay");
        let mut run = Run {
            engine: Engine::start_with(&replay.setup, RunContent::bundled())
                .expect("the run starts"),
            inputs: replay.unpack_inputs(u32::MAX).expect("the run's inputs"),
            commands: replay.unpack_commands().expect("the run's commands"),
            next_command: 0,
        };
        run.play(ticks);
        run
    }

    /// Plays the next `ticks` ticks, taking the commands between them as the corpus does.
    pub fn play(&mut self, ticks: u32) {
        for _ in 0..ticks {
            while let Some(c) = self.commands.get(self.next_command) {
                if c.tick != self.engine.tick() {
                    break;
                }
                match c.kind {
                    CommandKind::Choose => self.engine.choose(c.index).expect("the offer is open"),
                }
                self.next_command += 1;
            }
            let input = self.inputs[self.engine.tick() as usize];
            self.engine.step(input);
        }
    }
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

/// The densest minute of the run and its heaviest tick, as numbers of ticks played before them:
/// the minute is the ticks `start..start + MINUTE_TICKS`, the window of the largest summed load,
/// and the next `step` of a run `peak_tick` ticks in is the heaviest tick of that window.
pub fn worst_minute_window() -> (u32, u32) {
    let mut run = Run::fast_forwarded(0);
    let ticks = run.inputs.len();
    let mut loads = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        run.play(1);
        loads.push(weight(run.engine.load()));
    }
    let minute = MINUTE_TICKS as usize;
    let mut sum: u64 = loads[..minute].iter().sum();
    let (mut best, mut start) = (sum, 0);
    for from in 1..=ticks - minute {
        sum = sum - loads[from - 1] + loads[from + minute - 1];
        if sum > best {
            (best, start) = (sum, from);
        }
    }
    let window = &loads[start..start + minute];
    let heaviest = window.iter().max().expect("a minute has ticks");
    let peak = start
        + window
            .iter()
            .position(|l| l == heaviest)
            .expect("it is in it");
    (
        u32::try_from(start).expect("a tick count"),
        u32::try_from(peak).expect("a tick count"),
    )
}

/// The run at the start of its densest minute (`benches/worst_minute.rs` plays `MINUTE_TICKS`).
pub fn worst_minute() -> Run {
    Run::fast_forwarded(worst_minute_window().0)
}

/// The run one tick before the heaviest tick of that minute (`benches/worst_minute_peak.rs`).
pub fn worst_minute_peak() -> Run {
    Run::fast_forwarded(worst_minute_window().1)
}
