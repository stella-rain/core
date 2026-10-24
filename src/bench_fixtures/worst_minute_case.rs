//! The densest real minute of the `time_limit` corpus run (BR1e, core#317, D-BUDGET A in DS49).
//! A one-tick bench misses wave spawns, elite minutes, the boss's arrival and death bursts, so
//! `benches/worst_minute.rs` counts the 3,600 ticks of the run's heaviest minute and
//! `benches/worst_minute_peak.rs` its heaviest tick. The run is the corpus's own,
//! `tests/corpus/runs/v0/time_limit/run.replay`, played from its start with its commands.
//!
//! "Heaviest" is the sum of the five fields of `Load` (enemies, attack tasks, zones, orbs, emitter
//! steps), unweighted. `tests/worst_minute.rs` finds both places from the run and asserts the
//! constants below equal them.
//!
//! Every later change of the run's outcome (a regenerated corpus) can move them: the test then
//! fails and names the new values; set them here and the state hashes in
//! `tests/bench_fixture_pins.rs` (from the failing pin's message).

use crate::engine::Engine;
use crate::input::Input;
use crate::run_content::RunContent;
use crate::run_replay::{CommandKind, RunReplay, TickCommand};

/// A minute: 60 s at 60 ticks a second.
pub const MINUTE_TICKS: u32 = 3_600;

/// The number of ticks played before the densest minute: the minute is the ticks
/// `WORST_MINUTE_START..WORST_MINUTE_START + MINUTE_TICKS`, the window of the largest summed load.
pub const WORST_MINUTE_START: u32 = 1_684;

/// The number of ticks played before the heaviest tick of that minute (by the same load): the
/// next `step` is that tick.
pub const WORST_TICK_IN_MINUTE: u32 = 2_699;

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

/// The run at the start of its densest minute (`benches/worst_minute.rs` plays `MINUTE_TICKS`).
pub fn worst_minute() -> Run {
    Run::fast_forwarded(WORST_MINUTE_START)
}

/// The run one tick before the heaviest tick of that minute (`benches/worst_minute_peak.rs`).
pub fn worst_minute_peak() -> Run {
    Run::fast_forwarded(WORST_TICK_IN_MINUTE)
}
