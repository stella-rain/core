//! The boomerang rune (ADR-099 points 2, 3, 7 and 8; ADR-100): an on-hit rune of its own,
//! neutral, on its own cooldown. When a combo step of the Main weapon hits and boomerang's
//! cooldown is 0, it is thrown: its hit box flies from the main character to the point hit over
//! its first active phase, then back toward where the main character is over its second, a
//! straight step each tick that reaches the point and then the main character on each leg's
//! last tick. It hits each enemy once on the way out and once on the way back. No orb: it
//! is a hit box, so the orbs alive from runes stay at 0. Its damage is the main character's
//! without a combo step, through atk-up but no type bonus and no match-up; its hits give no
//! expertise and fire no rune. Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, verify_run_with};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;

use crate::common;

use common::on_hit::*;
use common::run::{TRIAL_LENGTH, run_input, trial_setup};
use common::*;

/// What the test boomerang does: 4 base damage, against the step's 10 and the blast's 3.
const THROW: u32 = 4;
const BLAST: u32 = 3;
/// Each leg's ticks: out over 4, back over 4.
const LEG: u32 = 4;

/// Boomerang's entry: `cooldown`, `damage`, two active legs of `LEG` ticks, a circle of 4
/// pixels, `right` to the right of the path.
fn boomerang(cooldown: u32, damage: u32, right: i32) -> Value {
    let leg = json!({ "kind": "active", "ticks": LEG,
                      "hit_box": { "type": "circle", "offset": { "right": right },
                                   "radius": 1_024 } });
    entry("boomerang", cooldown, damage, json!([leg.clone(), leg]))
}

/// Boomerang of `cooldown`, `damage` and `right`; explosion, thunder and homing missiles never hit.
fn runes(cooldown: u32, damage: u32, right: i32) -> Runes {
    runes_of(vec![
        far("explosion"),
        far("thunder"),
        boomerang(cooldown, damage, right),
        far("homing_missiles"),
    ])
}

/// The stage with seven small enemies and the tough target, every rune taken from its star.
fn holding(runes: Runes, target: Value) -> Engine {
    let mut e = with(
        &stage(7, target),
        &content(vec![random_rune()], None, runes),
    );
    take_all(&mut e);
    e
}

fn cooldown(e: &Engine) -> u32 {
    cooldown_of(e, Rune::Boomerang)
}

/// Where the boomerang's hit box is centred, while it flies.
fn centre(e: &Engine) -> Option<Point> {
    let snapshot = e.snapshot();
    let attack = snapshot.players[0]
        .attacks
        .iter()
        .find(|a| a.slot == BOOMERANG_SLOT)?;
    attack.at
}

fn point(x: i32, y: i32) -> Point {
    Point { x: Fx(x), y: Fx(y) }
}

#[test]
fn a_combo_step_hit_throws_it_out_to_the_point_hit_and_back() {
    let mut e = holding(runes(60, THROW, 0), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    e.step(once(start)(start));
    let thrown = e.tick();
    assert_eq!(cooldown(&e), 60, "set as it fires");
    // Out from the main character to the target 32 pixels above, 8 pixels a tick, the tick it
    // is thrown counted; then back.
    let mut path = vec![centre(&e)];
    let mut seen = Vec::new();
    for _ in 0..2 * LEG {
        e.step(idle());
        path.push(centre(&e));
        for ev in e.events() {
            if let DomainEvent::Hit {
                target: t, damage, ..
            } = ev
                && *t == target
            {
                seen.push((e.tick(), *damage));
            }
        }
    }
    let y = |d: i32| Some(point(START_X, START_Y - d));
    assert_eq!(
        path,
        [
            y(2_048),
            y(4_096),
            y(6_144),
            y(8_192),
            y(6_144),
            y(4_096),
            y(2_048),
            y(0),
            None
        ]
    );
    // Within 12 pixels of the target (its 8 and the box's 4): once out, once back.
    assert_eq!(of(&seen, THROW), [thrown + 2, thrown + LEG]);
}

#[test]
fn it_hits_an_enemy_on_its_way_once_out_and_once_back() {
    // The throw passes 64 pixels to the right of the step's reach, over an enemy halfway up.
    let right = 16_384;
    let beside = wave(
        TARGET_TICK,
        1,
        0,
        enemy(100_000, hold(START_X + right, START_Y - 4_096), json!([])),
        START_X + right,
        START_Y - 4_096,
    );
    let mut s = stage(7, tough(0, None, None));
    s.waves.push(serde_json::from_value(beside).unwrap());
    let mut e = with(
        &s,
        &content(vec![random_rune()], None, runes(60, THROW, right)),
    );
    take_all(&mut e);
    while e.tick() < TARGET_TICK + 1 {
        e.step(idle());
    }
    let start = e.tick();
    let mut seen: Vec<(u32, EntityId, u32)> = Vec::new();
    for _ in 0..20 {
        e.step(once(start)(e.tick()));
        for ev in e.events() {
            if let DomainEvent::Hit { target, damage, .. } = ev {
                seen.push((e.tick(), *target, *damage));
            }
        }
    }
    let steps: Vec<_> = seen.iter().filter(|h| h.2 == 10).collect();
    assert_eq!(steps.len(), 1, "the step hits the target only: {seen:?}");
    let thrown = steps[0].0;
    let throws: Vec<(u32, EntityId)> = seen
        .iter()
        .filter(|h| h.2 == THROW)
        .map(|h| (h.0, h.1))
        .collect();
    assert_eq!(throws.len(), 2, "{seen:?}");
    assert_ne!(throws[0].1, steps[0].1, "not the target, out of its reach");
    assert_eq!(throws[0].1, throws[1].1, "the same enemy both ways");
    // Out over it on its second tick; back over it on the back leg's first.
    assert_eq!((throws[0].0, throws[1].0), (thrown + 1, thrown + LEG));
}

#[test]
fn it_comes_back_to_where_the_main_character_has_moved() {
    let mut e = holding(runes(60, THROW, 0), tough(0, None, None));
    wait_for_target(&mut e);
    let start = e.tick();
    e.step(once(start)(start));
    for _ in 0..LEG - 1 {
        e.step(idle());
    }
    assert_eq!(centre(&e), Some(point(START_X, START_Y - 8_192)), "turned");
    // Moving right through the whole way back.
    let right = Input {
        move_x: 127,
        ..idle()
    };
    for _ in 0..LEG {
        e.step(right);
    }
    let at = e.snapshot().players[0].at;
    assert!(at.x.0 > START_X, "it moved");
    assert_eq!(centre(&e), Some(at), "back on the main character");
}

#[test]
fn its_cooldown_blocks_a_second_throw_until_it_reaches_0() {
    let mut e = holding(runes(60, THROW, 0), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let seen = hits(&mut e, target, 300, |t| firing(idle(), t));
    let steps = of(&seen, 10);
    let throws = of(&seen, THROW);
    assert!(throws.len() >= 8, "{throws:?}");
    // Two hits a throw, out and back, a throw every 60 ticks at the first step hit after.
    for pair in throws.chunks(2) {
        assert_eq!(pair.len(), 2, "{throws:?}");
        assert_eq!(pair[1] - pair[0], LEG - 2, "{throws:?}");
        assert!(steps.contains(&(pair[0] - 2)), "{pair:?}: {steps:?}");
    }
    for pair in throws.chunks(2).collect::<Vec<_>>().windows(2) {
        assert!((60..66).contains(&(pair[1][0] - pair[0][0])), "{throws:?}");
    }
}

#[test]
fn held_with_explosion_and_thunder_each_fires_on_its_own_cooldown() {
    // Explosion: 3 damage a tick after the hit, every 30 ticks; thunder never hits but cools
    // down every 45; boomerang: 4 damage out and back, every 60.
    let explosion = entry(
        "explosion",
        30,
        BLAST,
        json!([{ "kind": "active", "ticks": 2,
                 "hit_box": { "type": "circle", "radius": 4_096 } }]),
    );
    let mut thunder = far("thunder");
    thunder["cooldown_ticks"] = json!(45);
    let runes = runes_of(vec![
        explosion,
        thunder,
        boomerang(60, THROW, 0),
        far("homing_missiles"),
    ]);
    let mut e = holding(runes, tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    e.step(once(start)(start));
    assert_eq!(
        (
            cooldown_of(&e, Rune::Explosion),
            cooldown_of(&e, Rune::Thunder),
            cooldown(&e)
        ),
        (30, 45, 60),
        "one hit fires all three"
    );
    let seen = hits(&mut e, target, 299, |t| firing(idle(), t));
    let (blasts, throws) = (of(&seen, BLAST), of(&seen, THROW));
    for pair in blasts.windows(2) {
        assert!((30..36).contains(&(pair[1] - pair[0])), "{blasts:?}");
    }
    for pair in throws.chunks(2).collect::<Vec<_>>().windows(2) {
        assert!((60..66).contains(&(pair[1][0] - pair[0][0])), "{throws:?}");
    }
    assert!(blasts.len() >= throws.len() - 1, "{blasts:?} {throws:?}");
}

#[test]
fn its_hits_fire_no_rune() {
    // Explosion, ready again 2 ticks after it fires, would fire on the throw's hits if they
    // fired runes: once out, once back.
    let explosion = entry(
        "explosion",
        2,
        BLAST,
        json!([{ "kind": "active", "ticks": 2,
                 "hit_box": { "type": "circle", "radius": 4_096 } }]),
    );
    let runes = runes_of(vec![
        explosion,
        far("thunder"),
        boomerang(60, THROW, 0),
        far("homing_missiles"),
    ]);
    let mut e = holding(runes, tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 60, once(start));
    assert_eq!(of(&seen, 10).len(), 1);
    assert_eq!(of(&seen, THROW).len(), 2, "out and back");
    assert_eq!(of(&seen, BLAST).len(), 1, "the step's blast only: {seen:?}");
}

#[test]
fn its_hits_give_no_expertise() {
    // The target gives 1 point a hit: the step's, not the throw's two.
    let mut e = holding(runes(60, THROW, 0), tough(0, None, Some(1)));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 20, once(start));
    assert_eq!(of(&seen, THROW).len(), 2, "the throw landed");
    assert_eq!(e.snapshot().expertise.unwrap().points, 1);
}

/// The damage on the tough target of `profile`, holding a bonus of 50 for type `ty` and the
/// war cry's atk-up of 100 used before the step: `(step's hit, the throw's two)`.
fn boosted(ty: &str, profile: Value) -> (u32, Vec<u32>) {
    let war_cry = json!({ "id": "war_cry", "icon": "icon_nova",
        "levels": [{ "cooldown_ticks": 1_000, "effects": [
            { "type": "self_status", "status": "atk_up", "value_pct": 100,
              "duration_ticks": 600 }
        ] }] });
    let rewards = vec![
        random_rune(),
        json!({ "id": "war_cry_r", "rarity": "common", "effect": "stage_skill",
                "skill": "war_cry", "amount": 1, "limit": 2 }),
        json!({ "id": "bonus", "rarity": "common", "effect": "type_bonus_pct",
                "type": ty, "amount": 50, "limit": 2 }),
    ];
    let mut e = with(
        &stage(9, tough(0, Some(profile), None)),
        &content(rewards, Some(json!([war_cry])), runes(60, 10, 0)),
    );
    take_all(&mut e);
    take(&mut e, "war_cry_r");
    take(&mut e, "bonus");
    let target = wait_for_target(&mut e);
    e.step(Input {
        skill: true,
        ..idle()
    });
    let start = e.tick() + 1;
    let seen = hits(&mut e, target, 12, once(start));
    assert_eq!(seen.len(), 3, "{seen:?}");
    (seen[0].1, seen[1..].iter().map(|h| h.1).collect())
}

#[test]
fn a_neutral_throw_gets_atk_up_but_no_type_bonus_or_match_up() {
    // The step, neutral: 10 × 200 / 100 = 20. The throw: 10 × 200 / 100 = 20 each way,
    // whatever bonus is held and whatever the target is weak to or resists.
    for ty in ["slash", "pyro"] {
        let weak = json!({ "weak": [ty] });
        assert_eq!(boosted(ty, weak), (20, vec![20, 20]), "{ty}");
        assert_eq!(boosted(ty, json!({ "strong": [ty] })), (20, vec![20, 20]));
        assert_eq!(
            boosted(ty, json!({ "resist": "physical" })),
            (20, vec![20, 20])
        );
        assert_eq!(
            boosted(ty, json!({ "resist": "elements" })),
            (20, vec![20, 20])
        );
    }
}

#[test]
fn no_orb_comes_from_it() {
    let mut e = holding(runes(60, THROW, 0), tough(0, None, None));
    wait_for_target(&mut e);
    let start = e.tick();
    for _ in 0..2 * LEG + 2 {
        e.step(once(start)(e.tick()));
        assert!(e.snapshot().orbs.is_empty(), "a hit box, never an orb");
    }
}

#[test]
fn the_state_hash_holds_its_flight() {
    // The same cooldown and damage, but legs of 4 and of 8 ticks: the throws differ only in
    // how far they fly on the tick thrown.
    let mut slower = boomerang(60, THROW, 0);
    slower["attack"][0]["ticks"] = json!(8);
    let mut a = holding(runes(60, THROW, 0), tough(0, None, None));
    let mut b = holding(
        runes_of(vec![
            far("explosion"),
            far("thunder"),
            slower,
            far("homing_missiles"),
        ]),
        tough(0, None, None),
    );
    wait_for_target(&mut a);
    wait_for_target(&mut b);
    assert_eq!(a.state_hash(), b.state_hash());
    let start = a.tick();
    a.step(once(start)(start));
    b.step(once(start)(start));
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn a_run_holding_boomerang_records_and_verifies() {
    // The trial with a random rune and one stat reward, the hidden candidate always taken, and
    // the bundled runes: boomerang is drawn last with this seed, so the whole trial is played.
    let content = content(
        vec![
            random_rune(),
            json!({
                "id": "might", "rarity": "common", "effect": "attack_pct", "amount": 5,
                "limit": 20
            }),
        ],
        None,
        Runes::bundled().clone(),
    );
    // A seed whose run draws boomerang among the seven runes: the trial has stars for about four.
    let setup = RunSetup {
        seed: 20_261_012,
        ..trial_setup()
    };
    let mut recorder = RunRecorder::start_with(&setup, &content).unwrap();
    let mut fired = false;
    for t in 0..TRIAL_LENGTH {
        while recorder.engine().offer_open() {
            let offer = recorder.engine().snapshot().progress.and_then(|p| p.offer);
            let rune = offer.and_then(|o| o.candidates.iter().position(|c| c.hidden));
            recorder
                .choose(u32::try_from(rune.unwrap_or(0)).unwrap())
                .unwrap();
        }
        let input: Input = run_input(recorder.engine(), t);
        if recorder.step(input).is_err() {
            break;
        }
        fired |= recorder.engine().snapshot().players[0]
            .runes
            .iter()
            .any(|r| r.rune == Rune::Boomerang && r.cooldown_left > 0);
    }
    assert!(fired, "boomerang fired in the run");
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
}
