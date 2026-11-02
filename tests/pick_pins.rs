//! Pins what a choice does before it moves into the input (ADR-REPL-3): for corpus run cases
//! that hold commands, the state hash, the number of events and the tick after each of the first
//! commands. The constants were taken once and stay unchanged through the move of picks into
//! `Input`.
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
        engine.step(&[input]);
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
            (1933978024201087910, 1, 131),
            (1487868923222751991, 2, 131),
            (10705536155938927401, 1, 131),
            (17684500445707244715, 1, 180),
            (8909626012898947170, 1, 263),
            (9388823688583554274, 1, 267),
        ],
    ),
    (
        "elites",
        [
            (8969827991600578126, 1, 4161),
            (12812949893835142647, 2, 4161),
            (4592872326677725886, 1, 4161),
            (10262401378563450959, 1, 4166),
            (398116249341126454, 1, 4167),
            (17297741677679038849, 1, 4571),
        ],
    ),
    (
        "rainy",
        [
            (897167017184085048, 1, 297),
            (11904550819520454996, 1, 364),
            (9181289099137083804, 2, 364),
            (3920342818534441417, 1, 364),
            (14397456347246979235, 1, 542),
            (7892114522913036254, 1, 545),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
