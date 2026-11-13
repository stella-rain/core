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
            (10537527601524700336, 0, 142),
            (12300025580768239594, 3, 202),
            (509721543310426406, 0, 262),
            (6593122667086964724, 0, 292),
            (11956915695598156225, 3, 322),
            (8702587218063131853, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (10880626895656568210, 0, 4042),
            (14428958475530381436, 0, 4072),
            (1347679065542602541, 0, 4102),
            (9810955208042804223, 0, 4132),
            (2773283731664317657, 0, 23122),
            (11705866944559691610, 0, 23152),
        ],
    ),
    (
        "rainy",
        [
            (14218426133166762425, 3, 202),
            (17296185783635673866, 0, 262),
            (6903004053078823079, 3, 322),
            (13975187932286374107, 0, 352),
            (135756079057435381, 0, 382),
            (6693677864763884776, 0, 412),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
