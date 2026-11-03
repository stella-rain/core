//! Pins what a choice does before it moves into the input (ADR-REPL-16): for corpus run cases
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
            (10391896677877360708, 1, 131),
            (8484830586675025629, 2, 131),
            (2010276723764595405, 1, 131),
            (1780260283155783795, 1, 180),
            (15607545318434772194, 1, 263),
            (18183337339601160506, 1, 267),
        ],
    ),
    (
        "elites",
        [
            (2820564528101712596, 1, 4161),
            (16674744382060622829, 2, 4161),
            (17874868754328318198, 1, 4161),
            (8413317482523437295, 1, 4166),
            (2204263058133942038, 1, 4167),
            (432648924772219799, 1, 4571),
        ],
    ),
    (
        "rainy",
        [
            (1165278218475520790, 1, 297),
            (4686060691238212602, 1, 364),
            (3266926679250451904, 2, 364),
            (3590668328287421709, 1, 364),
            (2440586430219340035, 1, 542),
            (14894828642591010884, 1, 545),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
