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
            (3977576155220313138, 0, 292),
            (5958927013327159102, 3, 322),
            (5476887877322884974, 0, 352),
        ],
    ),
    (
        "elites",
        [
            (18300483907585456609, 6, 4162),
            (9185801575139604725, 0, 4192),
            (10754993341782320650, 0, 23152),
            (15106414334273199637, 3, 23182),
            (3663937651090799813, 0, 23812),
            (10716807829692287883, 0, 24082),
        ],
    ),
    (
        "rainy",
        [
            (14218426133166762425, 3, 202),
            (17296185783635673866, 0, 262),
            (16787208932069996529, 3, 322),
            (17085655311915307622, 0, 352),
            (10090305861374806735, 0, 382),
            (15467140404081545676, 0, 412),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
