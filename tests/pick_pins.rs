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
/// after each. The hash column was re-pinned when the swing's hit record replaced `sweep_count`;
/// the events and ticks are as they were.
const PINS: [(&str, [Pin; 6]); 3] = [
    (
        "easy",
        [
            (18408800813197971776, 0, 142),
            (2329696341256996970, 3, 202),
            (13627199773482140598, 0, 262),
            (1901205862033837204, 0, 292),
            (15147723869945546369, 3, 322),
            (14980755596556695581, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (15464828693569185994, 0, 4042),
            (729879092101194436, 0, 4072),
            (9089936161114583301, 0, 4102),
            (14638214403801726399, 0, 4132),
            (7923818824100026697, 0, 23122),
            (14567784700431274210, 0, 23152),
        ],
    ),
    (
        "rainy",
        [
            (15999059601219761785, 3, 202),
            (13627313401903612250, 0, 262),
            (14675180750847740423, 3, 322),
            (10324764677369216235, 0, 352),
            (2854938998873554181, 0, 382),
            (3160909482539460344, 0, 412),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
