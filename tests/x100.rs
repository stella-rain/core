//! Default damage and hit points scale by 100 (ADR-103 decision 6): where no percentage
//! applies, a run's outcome does not change. The outcomes below were recorded under the rules
//! before the scale (core v0.0.13): the corpus stages, written at design values and scaled with
//! `common::scaled`, must end the same tick, with the same kills and the main character's hit
//! points x100.

use crate::common::{self, SCALE};
use crate::corpus::definitions;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::validate::parse_and_validate;

fn kills(engine: &Engine) -> usize {
    engine
        .events()
        .iter()
        .filter(|e| matches!(e, DomainEvent::Died { .. }))
        .count()
}

/// Outcome, end tick, deaths and the main character's hit points per corpus stage, at the
/// design values. `zones`, `orbs` and `ranged_cap` came after the scale: their rows are what they ended as when they were added.
const STAGES: [(&str, Outcome, u32, usize, u32); 13] = [
    ("agents", Outcome::Running, 80, 1, 99),
    ("assist", Outcome::Running, 360, 0, 99),
    ("boss", Outcome::Cleared, 111, 1, 99),
    ("bullets", Outcome::Running, 300, 0, 99),
    ("clear", Outcome::Cleared, 35, 1, 3),
    ("evade", Outcome::Failed, 300, 0, 97),
    ("fail", Outcome::Failed, 155, 0, 0),
    ("landscape", Outcome::Failed, 300, 1, 98),
    ("melee", Outcome::Running, 300, 3, 97),
    ("orbs", Outcome::Running, 300, 0, 95),
    ("ranged_cap", Outcome::Running, 300, 0, 95),
    ("timeout", Outcome::Failed, 90, 0, 3),
    ("zones", Outcome::Running, 300, 0, 93),
];

#[test]
fn a_stage_scaled_by_100_ends_as_it_did_at_design_values() {
    let cases = definitions();
    assert_eq!(cases.len(), STAGES.len());
    for (name, bytes, inputs) in cases {
        // The corpus's evade stage holds the most hit points a stage may, for the bundled hit of
        // 10,000; at design values it holds the 99 the table above was recorded with.
        let bytes = if name == "evade" {
            let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            v["player"]["hp"] = serde_json::json!(99);
            v.to_string().into_bytes()
        } else {
            bytes
        };
        let scaled = common::scaled_bytes(&bytes, SCALE);
        let stage = parse_and_validate(&scaled).unwrap();
        let mut engine = common::engine(&stage);
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
