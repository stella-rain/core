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
            (2244072567269785274, 1, 128),
            (12899801081880393957, 2, 128),
            (14906568470565095335, 1, 128),
            (10567329490427981671, 1, 177),
            (255059462287243697, 2, 263),
            (12039846753795895402, 1, 263),
        ],
    ),
    (
        "elites",
        [
            (331767112650894632, 1, 4161),
            (11707479695264725171, 2, 4161),
            (8755192715317871870, 1, 4161),
            (7551719094612652693, 1, 4166),
            (8466941941279054272, 1, 4167),
            (7028143905205051555, 1, 4571),
        ],
    ),
    (
        "rainy",
        [
            (6156970784912129602, 1, 177),
            (74020770442780748, 2, 177),
            (8206947157156260422, 1, 177),
            (17850776533629885977, 1, 248),
            (16225182576620097260, 1, 297),
            (12634051775796884254, 1, 488),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
