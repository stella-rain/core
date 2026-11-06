//! Pins what a choice does before it moves into the input (ADR-REPL-5): for corpus run cases
//! that hold commands, the state hash, the number of events and the tick after each of the first
//! commands. The constants were taken once and stayed unchanged through the move of picks into
//! `Input`; the hashes were re-pinned for the per-player hash layout and again for the slot generations (ADR-SIM-3), and the
//! event counts and ticks, which say what the choices did, did not move.
use crate::corpus::{apply, case_content, runs_dir};
use stella_rain_core::engine::Engine;
use stella_rain_core::run_replay::RunReplay;

type Pin = (u64, usize, u32);

/// The (hash, events, tick) after each of the first `take` commands of run case `name`.
fn after_commands(name: &str, take: usize) -> Vec<Pin> {
    let case = runs_dir().join(name);
    let content = case_content(&case).unwrap();
    let replay = RunReplay::from_bytes(&std::fs::read(case.join("run.replay")).unwrap()).unwrap();
    let mut engine = Engine::start_with(&replay.setup, &content).unwrap();
    let commands = replay.unpack_commands().unwrap();
    let mut next = commands.iter().peekable();
    let mut seen = Vec::new();
    for input in replay.unpack_inputs(u32::MAX).unwrap() {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            apply(&mut engine, c).unwrap();
            seen.push((engine.state_hash(), engine.events().len(), engine.tick()));
        }
        if seen.len() >= take {
            break;
        }
        engine.step(&[input, Default::default(), Default::default()]);
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
            (5847746189528665978, 1, 128),
            (2873325775938988837, 2, 128),
            (6320844031187375735, 1, 128),
            (7272889099614623879, 1, 177),
            (6566831765752091153, 2, 263),
            (14223262443237869370, 1, 263),
        ],
    ),
    (
        "elites",
        [
            (11434688083884504264, 1, 4161),
            (4886338611328659475, 2, 4161),
            (3610460253043329502, 1, 4161),
            (15641447959890206421, 1, 4166),
            (7644249347745915792, 1, 4167),
            (17336095129078235555, 1, 4571),
        ],
    ),
    (
        "rainy",
        [
            (10519008264217851778, 1, 177),
            (8028390631829768716, 2, 177),
            (15777912680962629750, 1, 177),
            (12650726019290829849, 1, 248),
            (14179217665697212732, 1, 297),
            (7758371562864208926, 1, 488),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
