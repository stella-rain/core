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
            (16811710654302327794, 0, 142),
            (1552703580904643322, 3, 202),
            (8381413811734010446, 0, 262),
            (1529310704208537432, 0, 292),
            (11213807097149616017, 3, 322),
            (423721061660404043, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (1757141395110042556, 0, 4042),
            (17360325493195266576, 0, 4072),
            (6077877984530701745, 0, 4102),
            (1908624769013052747, 0, 4132),
            (9423992246262225647, 0, 23122),
            (16552727611213353976, 0, 23152),
        ],
    ),
    (
        "rainy",
        [
            (929087376886359563, 3, 202),
            (16829154511114997408, 0, 262),
            (13227382940023554077, 3, 322),
            (6604838010090844061, 0, 352),
            (4529956484968707933, 0, 382),
            (4626798967975636996, 0, 412),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
