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
            (15373035537714819308, 0, 142),
            (1188971751525923911, 3, 202),
            (16556548393670711982, 0, 262),
            (9082092075595902726, 0, 292),
            (11860052105243039688, 3, 322),
            (17877435619128160330, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (14470112924262184256, 0, 4039),
            (6196528020749832898, 0, 23119),
            (8270932808395225994, 0, 23149),
            (13750384682124744663, 0, 23179),
            (2266286503337886961, 0, 24319),
            (15377724959504386313, 0, 46819),
        ],
    ),
    (
        "rainy",
        [
            (15054778341186555137, 3, 202),
            (4512563832467928062, 0, 262),
            (2526813088110314490, 0, 352),
            (13423378008874116400, 0, 382),
            (18361484826215154697, 0, 412),
            (5763681066869071904, 0, 442),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
