//! The rune helpers of the worst case (a copy of the subset of `tests/common/on_hit.rs` it uses).

#[cfg(test)]
use super::START_Y;
use super::design_run_stats;
#[cfg(test)]
use super::{firing, idle};
#[cfg(test)]
use crate::engine::Engine;
use crate::rewards::Rewards;
use crate::run_content::RunContent;
#[cfg(test)]
use crate::rune::Rune;
use crate::runes::Runes;
#[cfg(test)]
#[cfg(test)]
use crate::stage::Stage;
use crate::stage_skill::StageSkills;
use crate::version::Versions;
use serde_json::{Value, json};

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once, and inside the step's hit box.
#[cfg(test)]
pub const NEAR_Y: i32 = START_Y - 8_192;

pub fn random_rune() -> Value {
    json!({
        "id": "random_rune", "rarity": "common", "effect": "random_rune"
    })
}

/// The `rarities` list of a rewards file: what each rarity costs in Lumine, as the bundled file
/// has it.
pub fn rarities() -> Value {
    json!([
        { "rarity": "common", "lumine_cost": 1 },
        { "rarity": "uncommon", "lumine_cost": 2 },
        { "rarity": "rare", "lumine_cost": 3 },
        { "rarity": "epic", "lumine_cost": 4 },
        { "rarity": "legendary", "lumine_cost": 6 },
        { "rarity": "supernova", "lumine_cost": 10 },
    ])
}

/// The run content of `rewards` (read with `skills`, when given) and `runes`.
#[cfg(test)]
pub fn content(rewards: Vec<Value>, skills: Option<Value>, runes: Runes) -> RunContent {
    content_of(
        json!({ "rarities": rarities(), "rewards": rewards }),
        skills,
        runes,
    )
}

/// The run content of the rewards file `file` (read with `skills`, when given) and `runes`: a file
/// that carries an `equipment` object runs in equipment mode.
pub fn content_of(file: Value, skills: Option<Value>, runes: Runes) -> RunContent {
    let file = file.to_string();
    let rewards = match skills {
        Some(skills) => {
            let skills = json!({ "stage_skills": skills });
            let skills = StageSkills::parse(skills.to_string().as_bytes(), Versions::CURRENT)
                .expect("the case skills validate");
            Rewards::parse_with(file.as_bytes(), Versions::CURRENT, &skills)
        }
        None => Rewards::parse(file.as_bytes(), Versions::CURRENT),
    }
    .expect("the case rewards validate");
    RunContent::new(design_run_stats(), rewards).with_runes(runes)
}

#[cfg(test)]
pub fn with(stage: &Stage, content: &RunContent) -> Engine {
    Engine::new_with(stage, content).expect("the case stage is valid")
}

/// Fires until an offer is open, then takes the candidate that is `reward`.
#[cfg(test)]
pub fn take(e: &mut Engine, reward: &str) {
    for _ in 0..600 {
        e.step(&[
            firing(idle(), e.tick()),
            Default::default(),
            Default::default(),
        ]);
        if e.offer_open() {
            break;
        }
    }
    // The options of an offer start with a take of each candidate, in order: the index of the
    // candidate is the index of its option.
    let index = e
        .v1_offer_candidates()
        .iter()
        .position(|c| c.as_str() == reward)
        .unwrap_or_else(|| panic!("{reward} is not offered"));
    crate::bench_fixtures::take_option(e, u32::try_from(index).expect("an option index"));
}

/// Takes the random rune from the next seven offers: the Main holds every rune, in the order
/// drawn.
#[cfg(test)]
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
