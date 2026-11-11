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
            (5707232866475366686, 0, 142),
            (753175521192609957, 3, 202),
            (4478498300023649792, 0, 262),
            (16640423573800706393, 0, 292),
            (4447489018207493443, 3, 322),
            (17923012613045348735, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (14696179731370610135, 1, 4162),
            (13515866880830796143, 0, 4192),
            (6881363426089559985, 0, 4222),
            (4265401972693746769, 0, 4582),
            (3652793305981975779, 0, 23182),
            (5176962571063590171, 0, 23212),
        ],
    ),
    (
        "rainy",
        [
            (15859139018590744683, 3, 202),
            (4504892280799388516, 0, 262),
            (11831613322608777805, 0, 352),
            (14147931131702089875, 0, 382),
            (183239364965129780, 0, 412),
            (11746549662101305965, 0, 442),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
