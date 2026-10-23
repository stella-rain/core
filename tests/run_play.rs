//! The scripted run of the trial and the other modes as a list of commands: `run_input`'s
//! ticks, every offer answered (`common::run::answer_all`), and every stat point spent on
//! `SPEND_INDEX` as it comes. Beside the common fixtures, not in them, so that the benches that
//! share `tests/common/` do not build it.

use crate::common::run::{STEER_FROM, answer_all, run_input, trial_setup};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::RunSetup;

/// The run stat `play_run` spends every point on.
pub const SPEND_INDEX: u32 = 1;

fn points(engine: &Engine) -> u32 {
    engine.snapshot().progress.map_or(0, |p| p.points)
}

/// The commands of a run of `run_input` for up to `ticks` ticks, spending every point on
/// `SPEND_INDEX` as it comes, between that tick and the next (and after the last), until the
/// stat is at its limit, and answering every offer as `answer_all` does.
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
        while points(engine) > 0 && engine.spend(SPEND_INDEX).is_ok() {
            out.push(Command::Spend(SPEND_INDEX));
        }
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

/// The tick at whose end the run's first point comes: the one `play_run` spends it after.
/// Offers may open before it.
pub fn first_point_tick() -> u32 {
    let commands = play_run(STEER_FROM);
    let at = commands
        .iter()
        .position(|c| matches!(c, Command::Spend(_)))
        .unwrap_or_else(|| panic!("the script gains no point before tick {STEER_FROM}"));
    let steps = commands[..at]
        .iter()
        .filter(|c| matches!(c, Command::Step(_)))
        .count();
    u32::try_from(steps).unwrap()
}
