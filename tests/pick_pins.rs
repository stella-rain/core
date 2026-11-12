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
            (159378162650484382, 0, 142),
            (535463991438187429, 3, 202),
            (5942041092405481536, 0, 262),
            (13906879585267846361, 0, 292),
            (2162095112574469827, 3, 322),
            (13568093524901936063, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (457543918659784215, 1, 4162),
            (6067677992085217919, 0, 4192),
            (9416079403283275249, 0, 4222),
            (1244780224434870289, 0, 4582),
            (2272425063744061175, 0, 23182),
            (14539785374842775637, 0, 23212),
        ],
    ),
    (
        "rainy",
        [
            (2526984361438670395, 3, 202),
            (6888824899024201956, 0, 262),
            (3528164648320767181, 0, 352),
            (130583323548571155, 0, 382),
            (1750805359773736244, 0, 412),
            (12456169915312031352, 0, 442),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
