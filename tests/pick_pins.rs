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
            (6332248442268475834, 1, 128),
            (16987976956879084517, 2, 128),
            (8348049503842638439, 1, 128),
            (12775434655211000551, 1, 177),
            (10363437424044771377, 2, 263),
            (14812702528104815658, 1, 263),
        ],
    ),
    (
        "elites",
        [
            (5671693779389078952, 1, 4161),
            (10295690195600450867, 2, 4161),
            (13888374517392201854, 1, 4161),
            (18286132053158624661, 1, 4166),
            (4337373875214919040, 1, 4167),
            (18067123891957286051, 1, 4571),
        ],
    ),
    (
        "rainy",
        [
            (10303760656628413762, 1, 177),
            (4220810642159064908, 2, 177),
            (16489442618224753926, 1, 177),
            (15038317366758962713, 1, 248),
            (577722602779091884, 1, 297),
            (1214324560809883422, 1, 488),
        ],
    ),
];

#[test]
fn a_choice_does_what_it_did_before_it_moved_into_the_input() {
    for (name, pins) in PINS {
        assert_eq!(after_commands(name, 6), pins, "run case {name}");
    }
}
