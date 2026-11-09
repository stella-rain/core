//! Pins what a choice does before it moves into the input (ADR-REPL-5): for corpus run cases
//! that hold commands, the state hash, the number of events and the tick after each of the first
//! commands. The constants were taken once and stayed unchanged through the move of picks into
//! `Input`; the hashes were re-pinned for the per-player hash layout and again for the slot generations and then for the six new rewards that lengthen the hashed lists (ADR-SIM-3), and the
//! event counts and ticks, which say what the choices did, did not move.
use crate::common::run_steps::{self, RunSteps, Step};
use crate::corpus::{case_content, runs_dir};
use stella_rain_core::run_replay::RunReplay;

type Pin = (u64, usize, u32);

/// The (hash, events, tick) after each of the first `take` commands of run case `name`.
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
        if matches!(step, Step::Picks(_)) {
            seen.push((engine.state_hash(), engine.events().len(), engine.tick()));
        }
    }
    seen
}

/// The first six commands of three run cases, which between them take star, level and
/// expertise offers: the state hash, the events and the tick after each, taken on the code
/// where `choose` is its own method.
const PINS: [(&str, [Pin; 6]); 3] = [
    (
        "easy",
        [
            (17117895086852862318, 1, 128),
            (7616054199903736465, 2, 128),
            (6972137321315370495, 1, 128),
            (1836394631138400559, 1, 177),
            (17044189276531530617, 2, 263),
            (1138776114179265986, 1, 263),
        ],
    ),
    (
        "elites",
        [
            (5838056326134554884, 1, 4161),
            (7945712485010896719, 2, 4161),
            (4868091848243456022, 1, 4161),
            (9150931298935964537, 1, 4166),
            (11923648164717174844, 1, 4167),
            (11227868491310892799, 1, 4571),
        ],
    ),
    (
        "rainy",
        [
            (11328778091010703298, 1, 177),
            (14886896750937530136, 2, 177),
            (3417663741640201406, 1, 177),
            (13177262495326042801, 1, 248),
            (7985982310849542212, 1, 297),
            (1163686814899249746, 1, 488),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
