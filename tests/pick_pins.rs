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
            (17757522315702623266, 0, 142),
            (11518053193506889483, 3, 202),
            (9133819761888567334, 0, 262),
            (15042083373528594425, 0, 292),
            (11233703107952276717, 3, 322),
            (8474466895905531543, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (7591291788340952035, 1, 4162),
            (4018522901715134073, 0, 4192),
            (8652470227687283647, 0, 4222),
            (10576837100731263713, 0, 4582),
            (12290828385073133345, 0, 23182),
            (11851884623270776683, 0, 23212),
        ],
    ),
    (
        "rainy",
        [
            (13703659459205289955, 3, 202),
            (9701204297926600776, 0, 262),
            (14811620643704616861, 0, 352),
            (4011162363195410853, 0, 382),
            (5344521305735242672, 0, 412),
            (5053450836959386330, 0, 442),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
