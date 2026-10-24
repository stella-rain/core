//! The rune helpers of the worst case (a copy of the subset of `tests/common/on_hit.rs` it uses).

use super::{START_Y, design_run_stats, firing, idle};
use crate::engine::Engine;
use crate::rewards::Rewards;
use crate::run_content::RunContent;
use crate::rune::Rune;
use crate::runes::Runes;
use crate::snapshot::OfferOption;
use crate::stage::Stage;
use crate::stage_skill::StageSkills;
use serde_json::{Value, json};

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once, and inside the step's hit box.
pub const NEAR_Y: i32 = START_Y - 8_192;

pub fn random_rune() -> Value {
    json!({
        "id": "random_rune", "rarity": "common", "effect": "random_rune",
        "title": "Rune", "description": ""
    })
}

/// The run content of `rewards` (read with `skills`, when given) and `runes`.
pub fn content(rewards: Vec<Value>, skills: Option<Value>, runes: Runes) -> RunContent {
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": rewards }).to_string();
    let rewards = match skills {
        Some(skills) => {
            let skills = json!({ "schema_version": 0, "sim_version": 0, "stage_skills": skills });
            let skills = StageSkills::parse(skills.to_string().as_bytes())
                .expect("the case skills validate");
            Rewards::parse_with(file.as_bytes(), &skills)
        }
        None => Rewards::parse(file.as_bytes()),
    }
    .expect("the case rewards validate");
    RunContent::new(design_run_stats(), rewards).with_runes(runes)
}

pub fn with(stage: &Stage, content: &RunContent) -> Engine {
    Engine::new_with(stage, content).expect("the case stage is valid")
}

/// Fires until an offer is open, then takes the candidate that is `reward`.
pub fn take(e: &mut Engine, reward: &str) {
    for _ in 0..600 {
        e.step(firing(idle(), e.tick()));
        if e.offer_open() {
            break;
        }
    }
    let offer = e
        .snapshot()
        .progress
        .expect("a run")
        .offer
        .expect("an open offer");
    let candidate = offer
        .candidates
        .iter()
        .position(|c| c.reward.0 == reward)
        .unwrap_or_else(|| panic!("{reward} is not offered"));
    let option = OfferOption::Take {
        candidate: u8::try_from(candidate).expect("a candidate index"),
    };
    let index = offer
        .options
        .iter()
        .position(|o| *o == option)
        .expect("the take option");
    e.choose(u32::try_from(index).expect("an option index"))
        .expect("the choice");
}

/// Takes the random rune from the next seven offers: the Main holds every rune, in the order
/// drawn.
pub fn take_all(e: &mut Engine) {
    for _ in Rune::ALL {
        take(e, "random_rune");
    }
}

/// A damage-over-time rune's entry in a runes file, of its own type: bleed Slash, poison
/// Dendro, burn Pyro.
pub fn dot(rune: &str, interval: u32, duration: u32, damage: u32, max_stacks: u32) -> Value {
    let ty = match rune {
        "bleed" => "slash",
        "poison" => "dendro",
        "burn" => "pyro",
        other => panic!("{other} is not a damage-over-time rune"),
    };
    json!({ "rune": rune, "interval_ticks": interval, "duration_ticks": duration,
            "damage": damage, "max_stacks": max_stacks, "type": ty })
}
