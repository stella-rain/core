//! Pins what a choice does before it moves into the input (ADR-REPL-3): for corpus run cases
//! that hold commands, the state hash, the number of events and the tick after each of the first
//! commands. The constants were taken once and stayed unchanged through the move of picks into
//! `Input`; the hashes were re-pinned once, for the per-player hash layout (ADR-SIM-3), and the
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
            (15088056578059988746, 1, 131),
            (2565152698418978923, 2, 131),
            (5884671321364980147, 1, 131),
            (18191247334575400005, 1, 180),
            (11458105588549784710, 1, 263),
            (13134248338712789878, 1, 267),
        ],
    ),
    (
        "elites",
        [
            (15878028384449595788, 1, 4161),
            (8824851563351547973, 2, 4161),
            (11123160038185959134, 1, 4161),
            (13175782734731607799, 1, 4166),
            (3692340680101300552, 1, 4167),
            (1427729788914567051, 1, 4571),
        ],
    ),
    (
        "rainy",
        [
            (664616534288299688, 1, 297),
            (7151412866391759502, 1, 364),
            (15711213776568434300, 2, 364),
            (4027098685349898825, 1, 364),
            (7589974340249661069, 1, 542),
            (7970061417358312750, 1, 545),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
