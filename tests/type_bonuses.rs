//! Type bonuses (ADR-100 points 4, 5 and 7): falling-star reward effects that raise the damage
//! of one damage type (`type_bonus_pct`, with its `type`), of every attack type
//! (`attack_type_bonus_pct`) or of every element (`element_bonus_pct`). The bonuses that apply
//! to an attack's type add up into one of the attacker's multipliers, fixed when the attack
//! starts: `damage × (100 + bonus) / 100`, the sum held to `MAX_TYPE_BONUS_PCT`. The match-up
//! stays a target's multiplier at the hit, so the two multiply. A neutral attack gets no bonus.
//! Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::attack::DamageType;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::match_ups::MatchUps;
use stella_rain_core::rewards::{MAX_TYPE_BONUS_PCT, RewardEffect, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{RunRecorder, RunReplayError, verify_run, verify_run_with};
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{EntityKind, OfferOption, TypeBonusView};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::limits::MAX_DAMAGE;

use crate::common;

use common::run::{run_input, trial_setup};
use common::*;

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once, and inside the step's hit box.
const NEAR_Y: i32 = START_Y - 8_192;
/// 160 pixels above the main character: beyond the step's hit box, within a bullet's reach.
const FAR_Y: i32 = START_Y - 40_960;
/// The tick the target arrives, once every star of the small enemies has been taken.
const TARGET_TICK: u32 = 300;

/// A Main weapon of one step: a circle of 32 pixels 4 pixels ahead, one active tick of
/// `damage`, typed `ty` (or neutral), firing a bullet ahead when `shoots`, then 4 ticks of
/// recovery.
fn weapon(damage: u32, ty: Option<&str>, shoots: bool) -> Value {
    let mut active = json!({ "kind": "active", "ticks": 1, "damage": damage,
                             "hit_box": { "type": "circle", "offset": { "forward": 1_024 },
                                          "radius": 8_192 } });
    if let Some(ty) = ty {
        active["type"] = json!(ty);
    }
    if shoots {
        active["spawn"] = json!([
            { "type": "fire", "speed": 512, "direction": { "type": "aimed", "offset": 0 } }
        ]);
    }
    json!({ "inline": {
        "combo": { "main": { "attack": { "inline": [active, { "kind": "recovery", "ticks": 4 }] } } },
        "window_ticks": 30,
        "assist_range": 0
    } })
}

/// Levels of 1000 experience and stars that every kill drops as common, taken at once.
fn starry(stage: Stage) -> Stage {
    let mut stage = stage;
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

/// The tough target: an enemy of 100 000 hit points with `profile`, holding still at `y`.
fn tough(at_tick: u32, y: i32, profile: Option<Value>) -> Value {
    let mut e = enemy(100_000, hold(START_X, y), json!([]));
    if let Some(p) = profile {
        e["profile"] = p;
    }
    wave(at_tick, 1, 0, e, START_X, y)
}

/// `weapon(damage, ty, false)`, `small` enemies of 1 hp (one step each) at `NEAR_Y`, one every
/// 6 ticks from tick 1, and the tough target with `profile` at `NEAR_Y` from `TARGET_TICK`.
fn targets(damage: u32, ty: Option<&str>, small: u16, profile: Option<Value>) -> Stage {
    let mut waves = vec![tough(TARGET_TICK, NEAR_Y, profile)];
    if small > 0 {
        waves.insert(
            0,
            wave(
                1,
                small,
                6,
                enemy(1, hold(START_X, NEAR_Y), json!([])),
                START_X,
                NEAR_Y,
            ),
        );
    }
    starry(armed(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        weapon(damage, ty, false),
    ))
}

/// A common reward `id` of `effect` (with `"type": ty` when given), `amount` percent, taken at
/// most `limit` times.
fn bonus(id: &str, effect: &str, ty: Option<&str>, amount: u32, limit: u8) -> Value {
    let mut r = json!({
        "id": id, "rarity": "common", "effect": effect, "amount": amount, "limit": limit,
        "title": "Bonus", "description": ""
    });
    if let Some(ty) = ty {
        r["type"] = json!(ty);
    }
    r
}

fn pyro(amount: u32) -> Value {
    bonus("pyro", "type_bonus_pct", Some("pyro"), amount, 20)
}

fn file(rewards: Vec<Value>) -> String {
    json!({ "schema_version": 0, "sim_version": 0, "rewards": rewards }).to_string()
}

fn rewards(list: Vec<Value>) -> Rewards {
    Rewards::parse(file(list).as_bytes()).expect("the test rewards validate")
}

fn content(list: Vec<Value>) -> RunContent {
    RunContent::new(RunStats::bundled().clone(), rewards(list))
}

fn with(stage: &Stage, list: Vec<Value>) -> Engine {
    Engine::new_with(stage, &content(list)).expect("the test stage is valid")
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

/// Fires until the next offer opens, and takes `reward` from it.
fn take_next(e: &mut Engine, reward: &str) {
    fire_until(e, idle(), 200, Engine::offer_open);
    assert!(e.offer_open(), "no offer within 200 ticks");
    take(e, reward);
}

/// The tough target's entity ID, once it is there.
fn tough_id(e: &Engine) -> EntityId {
    e.snapshot()
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Enemy && v.hp > 1_000)
        .map(|v| v.id)
        .expect("the tough target")
}

/// The damage of every hit on `target` in `events`.
fn damage_on(events: &[(u32, DomainEvent)], target: EntityId) -> Vec<u32> {
    events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Hit {
                target: t, damage, ..
            } if *t == target => Some(*damage),
            _ => None,
        })
        .collect()
}

/// The damage of the first step on the tough target, after taking `taken` in order from a run
/// with the rewards `list`, a step of `damage` typed `ty`, and the target's `profile`.
fn first_hit(
    damage: u32,
    ty: Option<&str>,
    profile: Option<Value>,
    list: Vec<Value>,
    taken: &[&str],
) -> u32 {
    let stage = targets(damage, ty, u16::try_from(taken.len()).unwrap(), profile);
    let mut e = with(&stage, list);
    for id in taken {
        take_next(&mut e, id);
    }
    assert!(e.tick() < TARGET_TICK, "every star taken before the target");
    let wait = TARGET_TICK - e.tick();
    run_until(&mut e, idle(), wait, |_| false);
    let target = tough_id(&e);
    let events = fire_until(&mut e, idle(), 12, |_| false);
    *damage_on(&events, target)
        .first()
        .expect("a hit on the target")
}

#[test]
fn the_three_effects_parse_and_a_one_type_bonus_names_its_type() {
    let parsed = rewards(vec![
        pyro(50),
        bonus("physical", "attack_type_bonus_pct", None, 10, 5),
        bonus("elemental", "element_bonus_pct", None, 10, 5),
    ]);
    let r = parsed.rewards();
    assert_eq!(r[0].effect(), RewardEffect::TypeBonusPct);
    assert_eq!(r[0].effects[0].damage_type, Some(DamageType::Pyro));
    assert_eq!(r[1].effect(), RewardEffect::AttackTypeBonusPct);
    assert_eq!(r[2].effect(), RewardEffect::ElementBonusPct);
}

#[test]
fn a_type_bonus_reward_is_bounded_like_the_others() {
    let bad = |r: Value| Rewards::parse(file(vec![r]).as_bytes()).is_err();
    assert!(
        !bad(pyro(200)),
        "the most a reward adds (MAX_TYPE_BONUS_AMOUNT)"
    );
    assert!(bad(pyro(0)), "an amount of 0");
    assert!(bad(pyro(201)), "an amount past MAX_TYPE_BONUS_AMOUNT");
    assert!(
        bad(bonus("pyro", "type_bonus_pct", Some("pyro"), 10, 0)),
        "a limit of 0"
    );
    assert!(
        bad(bonus("pyro", "type_bonus_pct", Some("pyro"), 10, 21)),
        "a limit past MAX_STACK"
    );
    assert!(
        bad(bonus("pyro", "type_bonus_pct", None, 10, 1)),
        "a one-type bonus without a type"
    );
    assert!(
        bad(bonus("pyro", "type_bonus_pct", Some("plasma"), 10, 1)),
        "an unknown type"
    );
    assert!(
        bad(bonus("all", "attack_type_bonus_pct", Some("slash"), 10, 1)),
        "a group bonus with a type"
    );
    assert!(
        bad(bonus("all", "element_bonus_pct", Some("pyro"), 10, 1)),
        "a group bonus with a type"
    );
    assert!(
        bad(bonus("might", "attack_pct", Some("pyro"), 10, 1)),
        "another effect with a type"
    );
}

#[test]
fn a_type_bonus_names_its_path_when_refused() {
    let Err(stella_rain_core::validate::Error::Invalid(issues)) =
        Rewards::parse(file(vec![bonus("pyro", "type_bonus_pct", None, 10, 1)]).as_bytes())
    else {
        panic!("refused as invalid");
    };
    assert_eq!(issues[0].path, "$.rewards[0].effects[0].type");
}

#[test]
fn a_one_type_bonus_raises_its_type_only() {
    let list = || vec![pyro(50), bonus("filler", "max_hp", None, 1, 20)];
    assert_eq!(
        first_hit(100, Some("pyro"), None, list(), &[]),
        100,
        "none taken"
    );
    assert_eq!(first_hit(100, Some("pyro"), None, list(), &["pyro"]), 150);
    assert_eq!(
        first_hit(100, Some("hydro"), None, list(), &["pyro"]),
        100,
        "another element"
    );
    assert_eq!(
        first_hit(100, Some("slash"), None, list(), &["pyro"]),
        100,
        "an attack type"
    );
}

#[test]
fn the_attack_type_bonus_raises_every_attack_type_and_no_element() {
    let list = || vec![bonus("physical", "attack_type_bonus_pct", None, 30, 5)];
    for ty in ["slash", "pierce", "strike"] {
        assert_eq!(
            first_hit(100, Some(ty), None, list(), &["physical"]),
            130,
            "{ty}"
        );
    }
    assert_eq!(
        first_hit(100, Some("pyro"), None, list(), &["physical"]),
        100
    );
}

#[test]
fn the_element_bonus_raises_every_element_and_no_attack_type() {
    let list = || vec![bonus("elemental", "element_bonus_pct", None, 30, 5)];
    for ty in ["pyro", "hydro", "electro", "cryo", "anemo", "geo", "dendro"] {
        assert_eq!(
            first_hit(100, Some(ty), None, list(), &["elemental"]),
            130,
            "{ty}"
        );
    }
    assert_eq!(
        first_hit(100, Some("slash"), None, list(), &["elemental"]),
        100
    );
}

#[test]
fn the_bonuses_that_apply_add_up_into_one_kind() {
    let list = || {
        vec![
            pyro(30),
            bonus("elemental", "element_bonus_pct", None, 20, 5),
            bonus("physical", "attack_type_bonus_pct", None, 40, 5),
        ]
    };
    // 30 + 30 + 20 percent, added: 1.8 times, not 1.3 × 1.3 × 1.2.
    let taken = ["pyro", "pyro", "elemental", "physical"];
    assert_eq!(first_hit(100, Some("pyro"), None, list(), &taken), 180);
    // Only the attack-type bonus applies to a slash.
    assert_eq!(first_hit(100, Some("slash"), None, list(), &taken), 140);
}

#[test]
fn the_bonus_multiplies_with_each_match_up_and_never_adds() {
    let list = || vec![pyro(50)];
    let hit = |profile: Value| first_hit(100, Some("pyro"), Some(profile), list(), &["pyro"]);
    // ADR-100's example: 1.5 × 0.8 = 1.2, never 1 + 0.5 − 0.2.
    assert_eq!(hit(json!({ "strong": ["pyro"] })), 120, "strong");
    assert_eq!(hit(json!({ "resist": "elements" })), 120, "resisted");
    assert_eq!(hit(json!({ "weak": ["pyro"] })), 180, "weak: 1.5 × 1.2");
    assert_eq!(hit(json!({ "weak": ["hydro"] })), 150, "no match-up");
}

#[test]
fn a_neutral_attack_gets_no_bonus() {
    let list = || {
        vec![
            pyro(50),
            bonus("elemental", "element_bonus_pct", None, 50, 5),
            bonus("physical", "attack_type_bonus_pct", None, 50, 5),
        ]
    };
    let taken = ["pyro", "elemental", "physical"];
    assert_eq!(first_hit(100, None, None, list(), &taken), 100);
}

#[test]
fn the_summed_bonus_saturates_at_the_cap_and_damage_stays_bounded() {
    assert_eq!(MAX_TYPE_BONUS_PCT, 1000);
    // Eleven takes of 100: 1100 percent held, 1000 applied, so a step of the largest damage
    // deals 11 times it.
    let taken = ["pyro"; 11];
    let dealt = first_hit(MAX_DAMAGE, Some("pyro"), None, vec![pyro(100)], &taken);
    assert_eq!(dealt, MAX_DAMAGE * 11);
    // With a weakness on top, still bounded: 11 × 1.2.
    let weak = Some(json!({ "weak": ["pyro"] }));
    let dealt = first_hit(MAX_DAMAGE, Some("pyro"), weak, vec![pyro(100)], &taken);
    assert_eq!(dealt, MAX_DAMAGE * 11 * 6 / 5);
}

#[test]
fn the_summed_bonus_saturates_at_the_content_value_under_the_cap() {
    // The match-up file's `max_type_bonus_pct` of 250: three takes of 100 apply 250 percent.
    let mut table: Value = serde_json::from_slice(MatchUps::BUNDLED).unwrap();
    table["max_type_bonus_pct"] = json!(250);
    let table = MatchUps::parse(table.to_string().as_bytes()).expect("the test table");
    let stage = targets(100, Some("pyro"), 3, None);
    let mut e = Engine::new_with(&stage, &content(vec![pyro(100)]).with_match_ups(table))
        .expect("the test stage is valid");
    for _ in 0..3 {
        take_next(&mut e, "pyro");
    }
    assert_eq!(
        e.snapshot().player.type_bonuses,
        [TypeBonusView {
            damage_type: DamageType::Pyro,
            pct: 300
        }],
        "the sum as taken"
    );
    let wait = TARGET_TICK - e.tick();
    run_until(&mut e, idle(), wait, |_| false);
    let target = tough_id(&e);
    let events = fire_until(&mut e, idle(), 12, |_| false);
    assert_eq!(damage_on(&events, target).first(), Some(&350));
}

#[test]
fn the_snapshot_holds_the_bonuses_taken_before_the_cap() {
    let list = vec![
        pyro(100),
        bonus("elemental", "element_bonus_pct", None, 10, 5),
        bonus("physical", "attack_type_bonus_pct", None, 20, 5),
    ];
    let mut e = with(&targets(10, None, 13, None), list);
    assert!(
        e.snapshot().player.type_bonuses.is_empty(),
        "none at the start"
    );
    for id in ["pyro"; 11] {
        take_next(&mut e, id);
    }
    take_next(&mut e, "elemental");
    take_next(&mut e, "physical");
    let view = |ty: DamageType, pct: u32| TypeBonusView {
        damage_type: ty,
        pct,
    };
    let mut expected = vec![
        view(DamageType::Slash, 20),
        view(DamageType::Pierce, 20),
        view(DamageType::Strike, 20),
        view(DamageType::Pyro, 1110),
    ];
    for ty in [
        DamageType::Hydro,
        DamageType::Electro,
        DamageType::Cryo,
        DamageType::Anemo,
        DamageType::Geo,
        DamageType::Dendro,
    ] {
        expected.push(view(ty, 10));
    }
    assert_eq!(e.snapshot().player.type_bonuses, expected);
}

#[test]
fn the_bonus_is_fixed_when_the_attack_starts() {
    // The step kills a small enemy with its hit box and fires a bullet at the target far
    // ahead; the kill's star opens an offer while the bullet flies, and the Pyro bonus taken
    // then leaves that bullet as it was. The next step's bullet has it.
    let small = wave(
        1,
        1,
        0,
        enemy(1, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    );
    let stage = starry(armed(
        stage_full(
            18_000,
            Some(far_boss()),
            vec![small, tough(1, FAR_Y, None)],
            vec![],
        ),
        weapon(100, Some("pyro"), true),
    ));
    let mut e = with(&stage, vec![pyro(50)]);
    e.step(idle());
    e.step(idle());
    let target = tough_id(&e);
    let mut events = run_until(&mut e, pressing_main(), 1, |_| false);
    events.extend(run_until(&mut e, idle(), 30, Engine::offer_open));
    assert!(e.offer_open(), "the small enemy's star");
    assert!(
        !e.snapshot().bullets.is_empty(),
        "the bullet is in flight while the offer is open"
    );
    take(&mut e, "pyro");
    events.extend(run_until(&mut e, idle(), 200, |_| false));
    assert_eq!(damage_on(&events, target), [100], "fired before the bonus");
    let mut events = run_until(&mut e, pressing_main(), 1, |_| false);
    events.extend(run_until(&mut e, idle(), 200, |_| false));
    assert_eq!(damage_on(&events, target), [150], "fired after it");
}

#[test]
fn the_state_hash_holds_the_bonuses() {
    let list = || {
        vec![
            pyro(10),
            bonus("hydro", "type_bonus_pct", Some("hydro"), 10, 5),
        ]
    };
    let stage = targets(10, None, 1, None);
    let mut a = with(&stage, list());
    let mut b = with(&stage, list());
    take_next(&mut a, "pyro");
    take_next(&mut b, "hydro");
    assert_eq!(a.tick(), b.tick());
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn a_run_with_a_type_bonus_taken_records_and_verifies() {
    // The trial with a rewards file of type bonuses only: every offer holds one.
    let content = content(vec![
        bonus("fire", "type_bonus_pct", Some("pyro"), 10, 20),
        bonus("cut", "type_bonus_pct", Some("slash"), 10, 20),
        bonus("physical", "attack_type_bonus_pct", None, 10, 20),
        bonus("elemental", "element_bonus_pct", None, 10, 20),
    ]);
    let mut recorder = RunRecorder::start_with(&trial_setup(), &content).unwrap();
    for t in 0..1_200 {
        while recorder.engine().offer_open() {
            recorder.choose(0).unwrap();
        }
        let input: Input = run_input(recorder.engine(), t);
        if recorder.step(input).is_err() {
            break;
        }
    }
    assert!(
        !recorder.engine().snapshot().player.type_bonuses.is_empty(),
        "a type bonus was taken"
    );
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
    assert_eq!(verify_run(&bytes), Err(RunReplayError::ContentChanged));
}
