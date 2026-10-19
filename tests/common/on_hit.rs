//! The on-hit rune tests' shared stage: a Main weapon of one step, small enemies whose stars
//! give the runes, and a tough target to hit once they are taken (`explosion.rs`, `thunder.rs`,
//! `boomerang.rs`).

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;
use stella_rain_core::snapshot::OfferOption;
use stella_rain_core::stage::Stage;
use stella_rain_core::stage_skill::StageSkills;

use super::*;

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once, and inside the step's hit box.
pub const NEAR_Y: i32 = START_Y - 8_192;
/// The tick the tough target arrives, once the small enemies' stars have been taken.
pub const TARGET_TICK: u32 = 300;
/// The slot of the first on-hit rune's attack: after the combo (0), the stage skill (1) and
/// the orbits' orbs (2 to 33).
pub const RUNE_SLOT: u8 = 34;
/// The slot of the second, thunder's.
pub const THUNDER_SLOT: u8 = 35;
/// The slot of the third, boomerang's.
pub const BOOMERANG_SLOT: u8 = 36;

/// A Main weapon of one step: a circle of 32 pixels 4 pixels ahead, one active tick of 10
/// damage, neutral, and 4 ticks of recovery.
pub fn weapon() -> Value {
    json!({ "inline": {
        "combo": { "main": { "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } },
            { "kind": "recovery", "ticks": 4 }
        ] } } },
        "window_ticks": 30,
        "assist_range": 0
    } })
}

/// The tough target: an enemy of 100 000 hit points at `(START_X + dx, NEAR_Y)` from
/// `TARGET_TICK`, with `profile` and expertise points `hit` per hit when given.
pub fn tough(dx: i32, profile: Option<Value>, hit: Option<u32>) -> Value {
    let mut e = enemy(100_000, hold(START_X + dx, NEAR_Y), json!([]));
    if let Some(p) = profile {
        e["profile"] = p;
    }
    let mut w = wave(TARGET_TICK, 1, 0, e, START_X + dx, NEAR_Y);
    if let Some(hit) = hit {
        w["expertise"] = json!({ "hit": hit, "kill": 0 });
    }
    w
}

/// `small` enemies of 1 hp at `NEAR_Y`, one every 6 ticks from tick 1, each dropping a common
/// star taken at once, then the `target` wave.
pub fn stage(small: u16, target: Value) -> Stage {
    let waves = vec![
        wave(
            1,
            small,
            6,
            enemy(1, hold(START_X, NEAR_Y), json!([])),
            START_X,
            NEAR_Y,
        ),
        target,
    ];
    let mut stage = armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon(),
        quiet_sub(),
    );
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 0, "banishes": 0,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

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
                .expect("the test skills validate");
            Rewards::parse_with(file.as_bytes(), &skills)
        }
        None => Rewards::parse(file.as_bytes()),
    }
    .expect("the test rewards validate");
    RunContent::new(RunStats::bundled().clone(), rewards).with_runes(runes)
}

pub fn with(stage: &Stage, content: &RunContent) -> Engine {
    Engine::new_with(stage, content).expect("the test stage is valid")
}

/// Fires until an offer is open, then takes the candidate that is `reward`.
pub fn take(e: &mut Engine, reward: &str) {
    fire_until(e, idle(), 600, Engine::offer_open);
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let candidate = offer
        .candidates
        .iter()
        .position(|c| c.reward.0 == reward)
        .unwrap_or_else(|| panic!("{reward} is not offered"));
    let option = OfferOption::Take {
        candidate: u8::try_from(candidate).unwrap(),
    };
    let index = offer.options.iter().position(|o| *o == option).unwrap();
    e.choose(u32::try_from(index).unwrap()).unwrap();
}

/// Takes the random rune from the next three offers: the Main holds every rune, in the order
/// drawn.
pub fn take_all(e: &mut Engine) {
    for _ in Rune::ALL {
        take(e, "random_rune");
    }
    let mut held: Vec<Rune> = e.snapshot().player.runes.iter().map(|r| r.rune).collect();
    held.sort();
    assert_eq!(held, Rune::ALL, "every rune held");
}

/// The cooldown left of `rune`, held.
pub fn cooldown_of(e: &Engine, rune: Rune) -> u32 {
    let runes = e.snapshot().player.runes;
    runes
        .iter()
        .find(|r| r.rune == rune)
        .unwrap_or_else(|| panic!("{rune:?} held"))
        .cooldown_left
}

/// An on-hit rune's entry in a runes file.
pub fn entry(rune: &str, cooldown: u32, damage: u32, attack: Value) -> Value {
    json!({ "rune": rune, "cooldown_ticks": cooldown, "damage": damage, "attack": attack })
}

/// An entry for `rune` whose attack lands 256 pixels ahead of where it is centred, beyond
/// every target of these stages: held, it fires on every hit it may but never hits anything.
/// Two active phases, as a boomerang's out and back.
pub fn far(rune: &str) -> Value {
    let phase = json!({ "kind": "active", "ticks": 2,
                        "hit_box": { "type": "circle", "offset": { "forward": 65_536 },
                                     "radius": 1_024 } });
    entry(rune, 30, 1, json!([phase.clone(), phase]))
}

/// The runes file of `on_hit`'s entries.
pub fn runes_of(on_hit: Vec<Value>) -> Runes {
    let file = json!({ "schema_version": 0, "sim_version": 0, "on_hit": on_hit });
    Runes::parse(file.to_string().as_bytes()).expect("the test runes validate")
}

/// Steps idle until the tough target is on the map, and returns its ID.
pub fn wait_for_target(e: &mut Engine) -> EntityId {
    while e.tick() < TARGET_TICK {
        e.step(idle());
    }
    let s = e.snapshot();
    s.entities
        .iter()
        .find(|en| en.hp > 1 && en.id != EntityId(0) && en.id != EntityId(1))
        .map(|en| en.id)
        .expect("the tough target")
}

/// Steps `n` ticks with `input`, collecting the hits on `target`: `(tick, damage)`.
pub fn hits(
    e: &mut Engine,
    target: EntityId,
    n: u32,
    input: impl Fn(u32) -> Input,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for _ in 0..n {
        e.step(input(e.tick()));
        for ev in e.events() {
            if let DomainEvent::Hit {
                target: t, damage, ..
            } = ev
            {
                if *t == target {
                    out.push((e.tick(), *damage));
                }
            }
        }
    }
    out
}

/// One press of `main` on the first tick, then nothing.
pub fn once(start: u32) -> impl Fn(u32) -> Input {
    move |t| Input {
        main: t == start,
        ..idle()
    }
}

pub fn of(hits: &[(u32, u32)], damage: u32) -> Vec<u32> {
    hits.iter()
        .filter(|(_, d)| *d == damage)
        .map(|(t, _)| *t)
        .collect()
}
