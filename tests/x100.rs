//! Default damage and hit points scale by 100 (ADR-103 decision 6): where no percentage
//! applies, a run's outcome does not change. The outcomes below were recorded under the rules
//! before the scale (core v0.0.13): the corpus stages written at design values and scaled with
//! `common::scaled`, and the recorded run cases, must end the same tick, with the same kills
//! and, for a stage, the main character's hit points ×100.

use crate::common::{self, SCALE};
use crate::corpus::{case_content, definitions, runs_dir};
use std::fs;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::run_replay::{CommandKind, RunReplay, verify_run_with};
use stella_rain_core::validate::parse_and_validate;

fn kills(engine: &Engine) -> usize {
    engine
        .events()
        .iter()
        .filter(|e| matches!(e, DomainEvent::Died { .. }))
        .count()
}

/// Outcome, end tick, deaths and the main character's hit points per corpus stage, at the
/// design values.
const STAGES: [(&str, Outcome, u32, usize, u32); 10] = [
    ("agents", Outcome::Running, 80, 1, 99),
    ("assist", Outcome::Running, 300, 0, 99),
    ("boss", Outcome::Cleared, 111, 1, 99),
    ("bullets", Outcome::Running, 300, 0, 99),
    ("clear", Outcome::Cleared, 35, 1, 3),
    ("evade", Outcome::Failed, 300, 0, 97),
    ("fail", Outcome::Failed, 155, 0, 0),
    ("landscape", Outcome::Failed, 300, 1, 98),
    ("melee", Outcome::Running, 300, 3, 97),
    ("timeout", Outcome::Failed, 90, 0, 3),
];

#[test]
fn a_stage_scaled_by_100_ends_as_it_did_at_design_values() {
    let cases = definitions();
    assert_eq!(cases.len(), STAGES.len());
    for (name, bytes, inputs) in cases {
        let scaled = common::scaled_bytes(&bytes, SCALE);
        let stage = parse_and_validate(&scaled).unwrap();
        let mut engine = Engine::new(&stage).unwrap();
        let mut died = 0;
        for input in &inputs {
            engine.step(*input);
            died += kills(&engine);
        }
        let hp = engine.snapshot().player.hp;
        let (_, outcome, ticks, deaths, left) = *STAGES.iter().find(|s| s.0 == name).unwrap();
        assert_eq!(
            (engine.outcome(), engine.tick(), died, hp),
            (outcome, ticks, deaths, left * SCALE as u32),
            "`{name}` scaled by 100"
        );
    }
}

/// Outcome, end tick and deaths of each run case, recorded under the rules before the scale.
const RUNS: [(&str, Outcome, u32, usize); 6] = [
    ("elites", Outcome::Failed, 47_400, 15),
    ("stage_skill", Outcome::Failed, 2_787, 10),
    ("time_limit", Outcome::Failed, 3_398, 31),
    ("time_limit_kill", Outcome::Cleared, 5_569, 2),
    ("time_limit_overtime", Outcome::Failed, 600, 0),
    ("trial", Outcome::Failed, 2_468, 9),
];

#[test]
fn the_run_cases_end_as_they_did_at_design_values() {
    for (name, outcome, ticks, deaths) in RUNS {
        let case = runs_dir().join(name);
        let content = case_content(&case).unwrap();
        let bytes = fs::read(case.join("run.replay")).unwrap();
        let verified = verify_run_with(&bytes, &content).unwrap();
        let replay = RunReplay::from_bytes(&bytes).unwrap();
        let mut engine = Engine::start_with(&replay.setup, &content).unwrap();
        let commands = replay.unpack_commands().unwrap();
        let mut next = commands.iter().peekable();
        let mut died = 0;
        for input in replay.unpack_inputs(u32::MAX).unwrap() {
            while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
                match c.kind {
                    CommandKind::Choose => engine.choose(c.index).unwrap(),
                    CommandKind::Spend => engine.spend(c.index).unwrap(),
                }
            }
            engine.step(input);
            died += kills(&engine);
        }
        assert_eq!(
            (verified.outcome, verified.ticks, died),
            (outcome, ticks, deaths),
            "`{name}`"
        );
    }
}
