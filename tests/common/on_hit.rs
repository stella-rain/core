//! The rune tests' shared stage: a Main weapon of one step and a tough target to hit, on an
//! equipment-mode run whose runes come from the inventory (`Parts.equip_runes`), with no small
//! enemies and no stars (`thunder.rs`, `boomerang.rs`, `homing_missiles.rs`, `bleed.rs`,
//! `poison_burn.rs`, `hit_marks.rs`, `rune_haste.rs`).

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;
use stella_rain_core::stage::Stage;
use stella_rain_core::version::Versions;

use super::equip::{Parts, equipped_in, rune_slot};
use super::*;

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once, and inside the step's hit box.
pub const NEAR_Y: i32 = START_Y - 8_192;
/// The tick the tough target arrives.
pub const TARGET_TICK: u32 = 300;
/// The slot of the first on-hit rune's attack: after the combo (0), the stage skill (1) and
/// the orbits' orbs (2 to 33).
pub const RUNE_SLOT: u8 = 34;
/// The slot of the second, thunder's.
pub const THUNDER_SLOT: u8 = 35;
/// The slot of the third, boomerang's.
pub const BOOMERANG_SLOT: u8 = 36;
/// The slot of homing missiles' first missile; the others follow it.
pub const MISSILE_SLOT: u8 = 64;

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

/// The stage of the equipped runs: `waves` alone, no small enemies and no stars, at the live
/// `sim_version`.
pub fn target_stage(waves: Vec<Value>) -> Stage {
    let mut stage = armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon(),
    );
    stage.sim_version = 2;
    stage
}

/// A rewards file of `rewards` (and a `filler`, as a file holds at least one reward and this one
/// is never equipped) with the equipment rules and one rune slot per rune.
pub fn rewards_file(rewards: Vec<Value>) -> Value {
    let mut rewards = rewards;
    rewards.push(
        json!({ "id": "filler", "rarity": "common", "effect": "max_hp", "amount": 1,
                "limit": 2 }),
    );
    let mut file = json!({ "rarities": super::rarities(), "rewards": rewards });
    let mut rules = serde_json::to_value(Rewards::bundled().equipment().expect("rules")).unwrap();
    rules["rune_slots"] = json!(Rune::ALL.len());
    file["equipment"] = rules;
    file
}

/// An equipment run of `stage` over the rewards `file` (read with `skills`), with `stars`
/// (reward, slot) equipped at `level` and, when `rune` is true, the seven runes in seven rune
/// slots. Every rune is ready; a stage skill star arrives cooling, so a test that presses one
/// sets its cooldown to 1 and `wait_ready`s.
fn equipped_run(
    stage: &Stage,
    file: Value,
    runes: Runes,
    skills: Option<Value>,
    stars: &[(&str, usize)],
    rune: bool,
    level: u8,
) -> Engine {
    let equip_runes = match rune {
        true => Rune::ALL
            .into_iter()
            .enumerate()
            .map(|(n, r)| (r, rune_slot(&file, n)))
            .collect(),
        false => vec![],
    };
    let parts = Parts {
        skills,
        runes: Some(runes),
        equip_runes,
        ..Parts::default()
    };
    equipped_in(stage, file, parts, stars, level)
}

/// `stage` played with the seven runes equipped, `stars` equipped, over `rewards`.
pub fn every_rune_held(
    stage: &Stage,
    runes: Runes,
    rewards: Vec<Value>,
    skills: Option<Value>,
    stars: &[(&str, usize)],
) -> Engine {
    every_rune_held_in(stage, rewards_file(rewards), runes, skills, stars, 1)
}

/// `every_rune_held` over a rewards `file` (from `rewards_file`, which a test may change) with
/// the stars equipped at `level`.
pub fn every_rune_held_in(
    stage: &Stage,
    file: Value,
    runes: Runes,
    skills: Option<Value>,
    stars: &[(&str, usize)],
    level: u8,
) -> Engine {
    equipped_run(stage, file, runes, skills, stars, true, level)
}

/// As `every_rune_held`, with no rune equipped (the runes file is read all the same).
pub fn no_rune_held(
    stage: &Stage,
    runes: Runes,
    rewards: Vec<Value>,
    skills: Option<Value>,
    stars: &[(&str, usize)],
) -> Engine {
    equipped_run(stage, rewards_file(rewards), runes, skills, stars, false, 1)
}

/// Steps idle until the stage skill is ready.
pub fn wait_ready(e: &mut Engine) {
    for _ in 0..30 {
        if e.snapshot().players[0]
            .skill
            .as_ref()
            .is_some_and(|s| s.ready)
        {
            return;
        }
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    panic!("the stage skill never became ready");
}

/// The cooldown left of `rune`, held.
pub fn cooldown_of(e: &Engine, rune: Rune) -> u32 {
    let runes = e.snapshot().players[0].runes.clone();
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
/// Two active phases, as a boomerang's out and back; homing missiles' one missile has one, and
/// lands as far ahead of where it flies.
pub fn far(rune: &str) -> Value {
    let phase = json!({ "kind": "active", "ticks": 2,
                        "hit_box": { "type": "circle", "offset": { "forward": 65_536 },
                                     "radius": 1_024 } });
    if rune == "homing_missiles" {
        let mut e = entry(rune, 30, 1, json!([phase]));
        e["missiles"] = json!(1);
        e["range"] = json!(40_960);
        return e;
    }
    entry(rune, 30, 1, json!([phase.clone(), phase]))
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

/// Every damage-over-time rune with a first tick 10 seconds after it is first applied, beyond
/// every on-hit rune test: held, each is applied on every step's hit but never deals damage
/// there.
pub fn far_dots() -> Vec<Value> {
    ["bleed", "poison", "burn"]
        .into_iter()
        .map(|r| dot(r, 600, 600, 1, 1))
        .collect()
}

/// `far_dots` with `entry`'s rune set to `entry`.
pub fn far_dots_but(entry: Value) -> Vec<Value> {
    far_dots()
        .into_iter()
        .map(|d| {
            if d["rune"] == entry["rune"] {
                entry.clone()
            } else {
                d
            }
        })
        .collect()
}

/// The runes file of `on_hit`'s entries and damage-over-time runes that never tick
/// (`far_dots`).
pub fn runes_of(on_hit: Vec<Value>) -> Runes {
    runes_with(on_hit, far_dots())
}

/// The runes file of `on_hit`'s and `damage_over_time`'s entries.
pub fn runes_with(on_hit: Vec<Value>, damage_over_time: Vec<Value>) -> Runes {
    let file = json!({ "on_hit": on_hit,
                       "damage_over_time": damage_over_time });
    Runes::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the test runes validate")
}

/// Steps idle until the tough target is on the map, and returns its ID.
pub fn wait_for_target(e: &mut Engine) -> EntityId {
    while e.tick() < TARGET_TICK {
        e.step(&[idle(), Default::default(), Default::default()]);
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
        e.step(&[input(e.tick()), Default::default(), Default::default()]);
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
