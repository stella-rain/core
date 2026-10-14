//! A run of the trial mode (ADR-051), shared by the run replay, recording and corpus tests.

use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::RunSetup;

/// The trial mode's length.
pub const TRIAL_LENGTH: u32 = 5400;

pub fn trial_setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "trial".into(),
        main: "needle".into(),
        sub: "blaster".into(),
        main_expertise: 3,
        sub_expertise: 0,
        seed: 20_261_010,
    }
}

/// Side to side, evading, charging and pressing the Sub skill: with `trial_setup` the first
/// offer opens near tick 1,000 and the main character falls a few hundred ticks later.
pub fn run_script(tick: u32) -> Input {
    Input {
        move_x: if (tick / 40) % 2 == 0 { 127 } else { -127 },
        move_y: 0,
        evade: tick % 50 == 25,
        charge: (tick / 45) % 3 == 1,
        sub: tick % 9 == 0,
    }
}

/// The commands of a run of `run_script` for up to `ticks` ticks, answering every offer with
/// its option 1 before the next tick (and after the last).
pub fn play_run(ticks: u32) -> Vec<Command> {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    let mut out = Vec::new();
    let answer = |engine: &mut Engine, out: &mut Vec<Command>| {
        while engine.offer().is_some() {
            engine.choose(1).unwrap();
            out.push(Command::Choose(1));
        }
    };
    for t in 0..ticks {
        if engine.outcome() != Outcome::Running {
            break;
        }
        answer(&mut engine, &mut out);
        engine.step(run_script(t));
        out.push(Command::Step(run_script(t)));
    }
    answer(&mut engine, &mut out);
    out
}

/// The tick at whose end the run's first offer opens.
pub fn first_offer_tick() -> u32 {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    for t in 0..TRIAL_LENGTH {
        engine.step(run_script(t));
        if engine.offer().is_some() {
            return engine.tick();
        }
    }
    panic!("the script never opens an offer");
}
