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
            (8498388809958196872, 0, 142),
            (7451455277596582058, 3, 202),
            (10565860738930075798, 0, 262),
            (8478750016291817220, 0, 292),
            (1584894101898918081, 3, 322),
            (13003820139921940885, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (18245234859001499602, 0, 4042),
            (15622711954490719732, 0, 4072),
            (13687932374821210357, 0, 4102),
            (4922718564109094575, 0, 4132),
            (16048157604580873857, 0, 23122),
            (16392756511680373354, 0, 23152),
        ],
    ),
    (
        "rainy",
        [
            (11064105086253684753, 3, 202),
            (15502216532347747426, 0, 262),
            (9050724357170725007, 3, 322),
            (2862552681677373683, 0, 352),
            (17823625670889817509, 0, 382),
            (3476843821336823880, 0, 412),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
