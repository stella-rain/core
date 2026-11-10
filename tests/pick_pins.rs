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
            (12051350532347612903, 0, 142),
            (191896089292007458, 3, 202),
            (8373393733246669573, 0, 262),
            (17966372704470878492, 0, 292),
            (6234177215932111734, 3, 322),
            (9801478092340061368, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (10973417413542430197, 0, 4039),
            (13951890148286192289, 0, 23119),
            (2422805346188747137, 0, 23149),
            (12435025799983263692, 0, 23179),
            (14574148183119732085, 0, 24319),
            (8972230896671449973, 0, 46819),
        ],
    ),
    (
        "rainy",
        [
            (7989541165453812690, 3, 202),
            (11627535342718594731, 0, 262),
            (6725244551131898732, 0, 352),
            (9211514614624169266, 0, 382),
            (7987233272884585419, 0, 412),
            (5938051679395157358, 0, 442),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
