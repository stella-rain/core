//! Pins what a choice does when it enters the step log (ADR-REPL-5, ADR-SIM-7): for corpus run
//! cases, the state hash, the number of events and the tick after each of the first equipment
//! actions (the run cases are equipment runs: no offer opens, so no PICKS step; each of the first
//! six ACTIONS steps takes the place of the first six picks the offers-era pins held). The
//! constants are what the blessed files compute (ADR-SIM-3).
use crate::common::run_steps::{self, RunSteps, Step};
use crate::corpus::{case_content, runs_dir};
use stella_rain_core::run_replay::RunReplay;

type Pin = (u64, usize, u32);

/// The (hash, events, tick) after each of the first `take` ACTIONS steps of run case `name`.
fn after_commands(name: &str, take: usize) -> Vec<Pin> {
    let case = runs_dir().join(name);
    let content = case_content(&case).unwrap();
    let replay = RunReplay::from_bytes(&std::fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = run_steps::start(&replay, &content);
    let mut seen = Vec::new();
    for step in RunSteps::of(&replay).steps() {
        if matches!(step, Step::Tick(_)) && seen.len() >= take {
            break;
        }
        run_steps::apply(&mut engine, step);
        if matches!(step, Step::Actions(_)) {
            seen.push((engine.state_hash(), engine.events().len(), engine.tick()));
        }
    }
    seen
}

/// The first six equipment actions of three run cases: the state hash, the events and the tick
/// after each.
const PINS: [(&str, [Pin; 6]); 3] = [
    (
        "easy",
        [
            (5258690193052181752, 0, 142),
            (3871479659213889245, 3, 202),
            (4645394526417405774, 0, 262),
            (15040772900438336773, 0, 292),
            (14830364557391481975, 3, 322),
            (9041462158439515615, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (7999433345995392171, 1, 4162),
            (2525281370900161599, 0, 4192),
            (14515387583999708967, 0, 4222),
            (15681815419118518817, 0, 4582),
            (4909152243033836359, 0, 23182),
            (1042512839229261167, 0, 23212),
        ],
    ),
    (
        "rainy",
        [
            (423296454310853225, 3, 202),
            (10126380438934930330, 0, 262),
            (4392355865511509627, 0, 352),
            (10433521993171615077, 0, 382),
            (10257509353831631292, 0, 412),
            (14526833087431535088, 0, 442),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
