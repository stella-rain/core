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
            (2846634048323356856, 1, 131),
            (12481127673786745105, 2, 131),
            (8573777594413945567, 1, 131),
            (14861386334520140195, 1, 180),
            (4322937710756932454, 1, 263),
            (17923153949614276416, 1, 267),
        ],
    ),
    (
        "elites",
        [
            (2682779738299509816, 1, 4161),
            (3524173969912177857, 2, 4161),
            (2162724210928052618, 1, 4161),
            (4082802254498425047, 1, 4166),
            (1470372520015105856, 1, 4167),
            (11640293671944319249, 1, 4571),
        ],
    ),
    (
        "rainy",
        [
            (15198664357618475680, 1, 297),
            (8337391547053372314, 1, 364),
            (9014651797825256736, 2, 364),
            (820082800218627757, 1, 364),
            (9146536945027813247, 1, 542),
            (9401833734255915892, 1, 545),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
