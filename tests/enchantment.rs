//! Enchantments (ADR-070, ADR-078): a reward that changes the Main weapon's combo for the rest of
//! the run. Each is taken once, and once one of a kind is held no reward of that kind is offered
//! again. The first kind, range, widens the hit box of every combo step it starts (A and B steps
//! alike): its radius and offset grow by the reward's percent, its angle stays. Every number here
//! is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::attack::HitBox;
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::{Enchantment, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{AttackView, EnchantmentView, EntityKind, OfferOption};
use stella_rain_core::stage::Stage;

mod common;

use common::*;

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once, and where the first step's hit box is centred.
const NEAR_Y: i32 = START_Y - 8_192;
/// 56 pixels above the main character: out of the first step's reach (its circle ends 40 pixels
/// out and the enemy's starts 48 out), inside it 50 percent wider (60 pixels and 48).
const FAR_Y: i32 = START_Y - 14_336;

/// A Main weapon of two steps: A, a circle of 8 pixels 32 pixels ahead; then B, an arc of 16
/// pixels and 45 degrees each side, 4 pixels to the right. Each is one active tick of 10 damage
/// and 4 ticks of recovery, and the combo keeps its place for 30 ticks after.
fn weapon() -> Value {
    let phases = |hit_box: Value| {
        json!({ "inline": [
            { "kind": "active", "ticks": 1, "damage": 10, "hit_box": hit_box },
            { "kind": "recovery", "ticks": 4 }
        ] })
    };
    json!({ "inline": {
        "combo": { "main": {
            "attack": phases(circle(8_192, 2_048)),
            "next": { "sub": { "attack": phases(arc(1_024, 4_096)) } }
        } },
        "window_ticks": 30,
        "assist_range": 0
    } })
}

fn circle(forward: i32, radius: i32) -> Value {
    json!({ "type": "circle", "offset": { "forward": forward }, "radius": radius })
}

fn arc(right: i32, radius: i32) -> Value {
    json!({ "type": "arc", "offset": { "right": right }, "radius": radius,
            "half_angle": 45 * 256 })
}

fn hit_box(v: Value) -> HitBox {
    serde_json::from_value(v).unwrap()
}

/// A stage with one level and stars, where every kill drops a common star, the weapon above,
/// and `waves`.
fn starry(waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 0, "banishes": 0,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    armed(stage, weapon(), quiet_sub())
}

/// `count` enemies of 10 hp (one step each), one every 6 ticks from tick 1, at `NEAR_Y`.
fn targets(count: u16) -> Value {
    wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )
}

/// An enemy of 50 hp that never moves or attacks, at `FAR_Y`.
fn far() -> Value {
    wave(
        1,
        1,
        0,
        enemy(
            50,
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
            json!([]),
        ),
        START_X,
        FAR_Y,
    )
}

/// A range enchantment `id`, `pct` percent wider, a common reward.
fn range(id: &str, pct: u32) -> Value {
    json!({
        "id": id, "rarity": "common", "effect": "enchantment", "amount": pct, "limit": 1,
        "title": "Reach", "description": "", "enchantment": { "type": "range" }
    })
}

fn might() -> Value {
    json!({
        "id": "might", "rarity": "common", "effect": "attack_pct", "amount": 5, "limit": 5,
        "title": "Might", "description": ""
    })
}

fn with(stage: &Stage, rewards: Vec<Value>) -> Engine {
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": rewards });
    let rewards = Rewards::parse(file.to_string().as_bytes()).expect("the test rewards validate");
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

/// Takes the open offer's candidate that is `reward`.
fn take(e: &mut Engine, reward: &str) {
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

fn offered(e: &Engine) -> Vec<String> {
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    offer
        .candidates
        .iter()
        .map(|c| c.reward.0.clone())
        .collect()
}

/// The combo step in progress (slot 0).
fn step_attack(e: &Engine) -> Option<AttackView> {
    e.snapshot()
        .player
        .attacks
        .into_iter()
        .find(|a| a.slot == 0)
}

/// Waits for the combo step in progress to end, then presses `sub` (B) or `main` (A) for a
/// tick: the hit box of the step that starts.
fn swing(e: &mut Engine, sub: bool) -> HitBox {
    for _ in 0..60 {
        if step_attack(e).is_none() {
            break;
        }
        e.step(idle());
    }
    e.step(Input {
        main: !sub,
        sub,
        ..Input::default()
    });
    let step = step_attack(e).expect("a combo step started");
    step.hit_box.expect("its active phase's hit box")
}

fn far_hp(e: &Engine) -> u32 {
    e.snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Enemy && v.at.y.0 == FAR_Y)
        .map(|v| v.hp)
        .expect("the far enemy")
}

#[test]
fn a_range_enchantment_widens_the_hit_box_of_every_combo_step() {
    let mut e = with(&starry(vec![targets(1), far()]), vec![range("reach", 50)]);
    e.step(idle());
    e.step(idle());
    assert!(e.snapshot().player.enchantments.is_empty());

    // Before it: the first step as written. It kills the near enemy, whose star opens the offer,
    // and misses the far one.
    assert_eq!(swing(&mut e, false), hit_box(circle(8_192, 2_048)));
    assert!(e.offer_open());
    assert_eq!(far_hp(&e), 50);
    take(&mut e, "reach");
    assert_eq!(
        e.snapshot().player.enchantments,
        vec![EnchantmentView {
            reward: "reach".into(),
            enchantment: Enchantment::Range,
            amount: 50,
        }]
    );

    // After it: the B step and then the A step, radius and offset half again, angle as written.
    assert_eq!(swing(&mut e, true), hit_box(arc(1_536, 6_144)));
    assert_eq!(far_hp(&e), 50);
    assert_eq!(swing(&mut e, false), hit_box(circle(12_288, 3_072)));
    assert_eq!(far_hp(&e), 40, "the wider step reaches the far enemy");
}

#[test]
fn an_enchantment_held_is_not_offered_again_nor_another_of_its_kind() {
    let rewards = vec![range("reach", 50), range("reach_more", 20), might()];
    let mut e = with(&starry(vec![targets(2)]), rewards);
    fire_until(&mut e, idle(), 600, Engine::offer_open);
    let mut first = offered(&e);
    first.sort();
    assert_eq!(first, ["might", "reach", "reach_more"]);
    take(&mut e, "reach");

    fire_until(&mut e, idle(), 600, Engine::offer_open);
    assert_eq!(offered(&e), ["might"]);
}
