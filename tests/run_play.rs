//! The scripted run of the trial and the other modes as a list of commands: `run_input`'s
//! ticks and every offer answered (`common::run::answer_all`); the setup's stat priority spends the
//! level-ups. Beside the common fixtures, not in them, so that the benches that
//! share `tests/common/` do not build it.

use crate::common;
use crate::common::run::{STEER_FROM, answer_all, trial_setup};
use crate::zone_bot::run_input;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{Command, RunRecorder};
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
        engine.step(&[input]);
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
            Command::Step(input) => engine.step(&[input]),
            Command::Choose(index) => common::pick(&mut engine, index).unwrap(),
        }
        if engine.snapshot().progress.is_some_and(|p| p.level > 1) {
            return engine.tick();
        }
    }
    panic!("the script gains no level before tick {STEER_FROM}");
}

/// The scripted input of slot `slot` (1 or 2) of a party at `tick`: distinct patterns, so a
/// log carries each slot. Slots 1 and 2 are passive in the rules for now.
pub fn scripted_input(slot: u8, tick: u32) -> Input {
    match slot {
        1 => Input {
            move_y: 50,
            skill: tick % 90 < 30,
            ..Input::default()
        },
        _ => Input {
            move_x: -30,
            evade: tick % 120 < 10,
            ..Input::default()
        },
    }
}

/// The inputs of a party of `n` at `tick`: slot 0 played by the zone bot, the others scripted.
pub fn party_inputs(rec: &RunRecorder, n: u8, tick: u32) -> Vec<Input> {
    (0..n)
        .map(|s| match s {
            0 => run_input(rec.engine(), tick),
            _ => scripted_input(s, tick),
        })
        .collect()
}

/// Answers every open offer, each by the slot that holds it (option 0), one pick at a time.
pub fn answer_party_offers(rec: &mut RunRecorder, n: u8) {
    while rec.engine().offer_open() {
        let holder = (0..n)
            .find(|i| {
                rec.engine()
                    .check_pick(PlayerIdx::new(*i).unwrap(), 0)
                    .is_ok()
            })
            .expect("an open offer belongs to a slot");
        rec.choose(PlayerIdx::new(holder).unwrap(), 0).unwrap();
    }
}

/// A party of `n` played from `setup` for up to `ticks` ticks, every offer answered by its slot.
pub fn play_party(setup: &RunSetup, n: u8, ticks: u32) -> RunRecorder {
    let mut rec = RunRecorder::start_party(setup, n).unwrap();
    for t in 0..ticks {
        answer_party_offers(&mut rec, n);
        if rec.engine().outcome() != Outcome::Running {
            break;
        }
        let inputs = party_inputs(&rec, n, t);
        rec.step(&inputs).unwrap();
    }
    answer_party_offers(&mut rec, n);
    rec
}

/// A party case of the corpus (`tests/corpus/current/parties/<name>/`): `party` players on
/// `setup`, played for up to `ticks` ticks by `play_party`.
pub struct PartyCase {
    pub name: &'static str,
    pub setup: RunSetup,
    pub party: u8,
    pub ticks: u32,
}

/// The party cases. Slots 1 and 2 get an offer of their own only in some modes and seeds (the
/// seeds are found where, in the middle of the run, each answers an offer of its own): the
/// time_limit mode for two and for three. In the tutorial both slots answer, but only as the run
/// is cleared; in time_limit slot 1 answers in the middle of the run and slot 2 is not offered one
/// (no seed of 1 to 300 gives both).
pub fn party_cases() -> Vec<PartyCase> {
    let at = |mode: &str, seed: u64| RunSetup {
        mode: mode.into(),
        seed,
        ..trial_setup()
    };
    vec![
        PartyCase {
            name: "two_time_limit",
            setup: at("time_limit", 40),
            party: 2,
            ticks: 3000,
        },
        PartyCase {
            name: "three_tutorial",
            setup: at("tutorial", 67),
            party: 3,
            ticks: 3000,
        },
        PartyCase {
            name: "three_time_limit",
            setup: at("time_limit", 22),
            party: 3,
            ticks: 3000,
        },
    ]
}
