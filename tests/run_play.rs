//! The scripted run of the trial and the other modes as a list of commands: `run_input`'s
//! ticks and every offer answered (`common::run::answer_all`); the setup's stat priority spends the
//! level-ups. Beside the common fixtures, not in them, so that the benches that
//! share `tests/common/` do not build it.

use crate::common::run::{STEER_FROM, answer_all, trial_setup};
use crate::zone_bot::run_input;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::RunSetup;

/// The commands of a run of `run_input` for up to `ticks` ticks, answering every offer as
/// `answer_all` does, between a tick and the next (and after the last).
pub fn play_run(ticks: u32) -> Vec<Command> {
    play_run_from(&trial_setup(), ticks)
}

/// `play_run` from `setup` instead of the trial's.
pub fn play_run_from(setup: &RunSetup, ticks: u32) -> Vec<Command> {
    play_run_in(setup, RunContent::bundled(), ticks)
}

/// `play_run_from` with `content` in place of the bundled files.
pub fn play_run_in(setup: &RunSetup, content: &RunContent, ticks: u32) -> Vec<Command> {
    play_run_with(setup, content, ticks, run_input)
}

/// `play_run_in` with `input` choosing each tick's buttons.
pub fn play_run_with(
    setup: &RunSetup,
    content: &RunContent,
    ticks: u32,
    input: impl Fn(&Engine, u32) -> Input,
) -> Vec<Command> {
    let mut engine = Engine::start_with(setup, content).unwrap();
    let mut out = Vec::new();
    let mut offers = 0;
    let between = |engine: &mut Engine, out: &mut Vec<Command>, offers: &mut usize| {
        answer_all(engine, offers, out);
    };
    for t in 0..ticks {
        if engine.outcome() != Outcome::Running {
            break;
        }
        between(&mut engine, &mut out, &mut offers);
        let input = input(&engine, t);
        engine.step(input);
        out.push(Command::Step(input));
    }
    between(&mut engine, &mut out, &mut offers);
    out
}

/// The tick at whose end the run's first level-up comes: the one that raises the first stat.
/// Offers may open before it.
pub fn first_point_tick() -> u32 {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    for command in play_run(STEER_FROM) {
        match command {
            Command::Step(input) => engine.step(input),
            Command::Choose(index) => engine.choose(index).unwrap(),
        }
        if engine.snapshot().progress.is_some_and(|p| p.level > 1) {
            return engine.tick();
        }
    }
    panic!("the script gains no level before tick {STEER_FROM}");
}
