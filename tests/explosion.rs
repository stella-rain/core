//! The explosion rune (ADR-099 points 2, 3, 7 and 8): when a combo step of the Main weapon hits
//! and the main character holds explosion with its cooldown at 0, explosion fires its own
//! attack around the point hit, then waits out its cooldown, which counts down each tick. Its
//! damage is the main character's without a combo step: its base damage through atk-up and the
//! type bonus, fixed when it fires, then the target's match-up and defence. Its hits give no
//! expertise and fire no rune; orbits and the stage skill fire none either. Every number here
//! is worked out by hand. Thunder and boomerang are held too, as the random rune draws all
//! three, but their attacks land far from every target (`far`).

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, verify_run_with};
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;
use stella_rain_core::snapshot::OfferOption;

mod common;

use common::on_hit::*;
use common::run::{run_input, trial_setup};
use common::*;

/// What the test explosion does: 3 base damage, against the step's 10.
const BLAST: u32 = 3;

/// A runes file whose explosion has `cooldown`, `damage` and one active phase of `ticks`, a
/// circle of 16 pixels of type `ty` (or neutral); thunder never hits.
fn runes(cooldown: u32, damage: u32, ticks: u32, ty: Option<&str>) -> Runes {
    let mut phase = json!({ "kind": "active", "ticks": ticks,
                            "hit_box": { "type": "circle", "radius": 4_096 } });
    if let Some(ty) = ty {
        phase["type"] = json!(ty);
    }
    runes_of(vec![
        entry("explosion", cooldown, damage, json!([phase])),
        far("thunder"),
        far("boomerang"),
    ])
}

fn cooldown(e: &Engine) -> u32 {
    cooldown_of(e, Rune::Explosion)
}

/// The stage with three small enemies and the tough target, all three runes taken from their
/// stars. The last small enemy's kill may fire explosion, whose cooldown is over long before
/// the target arrives.
fn holding(runes: Runes, target: Value) -> Engine {
    let mut e = with(
        &stage(3, target),
        &content(vec![random_rune()], None, runes),
    );
    take_all(&mut e);
    e
}

#[test]
fn a_combo_step_hit_fires_an_explosion_around_the_point_hit() {
    let mut e = holding(runes(30, BLAST, 2, None), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    // The step's hit on the tick the press starts it.
    e.step(once(start)(start));
    let step_hit = e.tick();
    assert!(e.events().iter().any(|ev| matches!(
        ev,
        DomainEvent::Hit { target: t, damage: 10, .. } if *t == target
    )));
    assert_eq!(cooldown(&e), 30, "set as it fires");
    let snapshot = e.snapshot();
    let blast = snapshot
        .player
        .attacks
        .iter()
        .find(|a| a.slot == RUNE_SLOT)
        .expect("the explosion's attack");
    assert_eq!(
        blast.at,
        Some(Point {
            x: Fx(START_X),
            y: Fx(NEAR_Y)
        }),
        "around the target hit"
    );
    // It lands on the next tick, once.
    let later = hits(&mut e, target, 20, once(start));
    assert_eq!(of(&later, BLAST), [step_hit + 1]);
    assert_eq!(cooldown(&e), 10, "30 less 20 ticks");
}

#[test]
fn the_explosion_hits_everything_around_the_point() {
    // A second target 20 pixels above the first: beyond the step's reach (32 pixels round a
    // point 4 pixels ahead, plus its 8), within the explosion's (16 round the first, plus 8).
    let behind = wave(
        TARGET_TICK,
        1,
        0,
        enemy(100_000, hold(START_X, NEAR_Y - 5_120), json!([])),
        START_X,
        NEAR_Y - 5_120,
    );
    let mut s = stage(3, tough(0, None, None));
    s.waves.push(serde_json::from_value(behind).unwrap());
    let mut e = with(
        &s,
        &content(vec![random_rune()], None, runes(30, BLAST, 2, None)),
    );
    take_all(&mut e);
    while e.tick() < TARGET_TICK {
        e.step(idle());
    }
    let start = e.tick();
    let (mut steps, mut blasts) = (0, 0);
    for _ in 0..10 {
        e.step(once(start)(e.tick()));
        for ev in e.events() {
            match ev {
                DomainEvent::Hit { damage: 10, .. } => steps += 1,
                DomainEvent::Hit { damage: BLAST, .. } => blasts += 1,
                _ => {}
            }
        }
    }
    assert_eq!((steps, blasts), (1, 2), "the step hits one, the blast both");
}

#[test]
fn its_cooldown_blocks_a_second_fire_until_it_reaches_0() {
    let mut e = holding(runes(30, BLAST, 2, None), tough(0, None, None));
    let target = wait_for_target(&mut e);
    // A step every 6 ticks, each hitting.
    let seen = hits(&mut e, target, 200, |t| firing(idle(), t));
    let steps = of(&seen, 10);
    let blasts = of(&seen, BLAST);
    assert!(steps.len() > 30, "{steps:?}");
    assert!(blasts.len() >= 5, "{blasts:?}");
    for pair in blasts.windows(2) {
        // At the first step hit once 30 ticks have gone, landing a tick later.
        assert!((30..36).contains(&(pair[1] - pair[0])), "{blasts:?}");
    }
    // Each blast lands a tick after a step's hit.
    for b in &blasts {
        assert!(steps.contains(&(b - 1)), "{b}: {steps:?}");
    }
}

#[test]
fn the_cooldown_counts_down_each_tick() {
    let mut e = holding(runes(30, BLAST, 2, None), tough(0, None, None));
    wait_for_target(&mut e);
    let start = e.tick();
    let mut left = Vec::new();
    for _ in 0..33 {
        e.step(once(start)(e.tick()));
        left.push(cooldown(&e));
    }
    let expected: Vec<u32> = (0..=30).rev().chain([0, 0]).collect();
    assert_eq!(left, expected);
}

#[test]
fn without_the_rune_a_step_fires_nothing() {
    let mut e = with(
        &stage(2, tough(0, None, None)),
        &content(vec![random_rune()], None, runes(30, BLAST, 2, None)),
    );
    // The star's offer is skipped.
    fire_until(&mut e, idle(), 600, Engine::offer_open);
    let skip = e
        .snapshot()
        .progress
        .unwrap()
        .offer
        .unwrap()
        .options
        .iter()
        .position(|o| *o == OfferOption::Skip)
        .unwrap();
    e.choose(u32::try_from(skip).unwrap()).unwrap();
    let target = wait_for_target(&mut e);
    let seen = hits(&mut e, target, 100, |t| firing(idle(), t));
    assert!(of(&seen, BLAST).is_empty());
    assert!(
        e.snapshot()
            .player
            .attacks
            .iter()
            .all(|a| a.slot != RUNE_SLOT)
    );
}

#[test]
fn its_hits_fire_no_rune_one_blast_for_one_step() {
    // A cooldown of 2: had the blast's own hit fired explosion again, a second blast would land.
    let mut e = holding(runes(2, BLAST, 2, None), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 60, once(start));
    assert_eq!(of(&seen, 10).len(), 1);
    assert_eq!(of(&seen, BLAST).len(), 1);
}

#[test]
fn its_hits_give_no_expertise() {
    // The target gives 1 point a hit: one step's hit, not the blast's.
    let mut e = holding(runes(30, BLAST, 2, None), tough(0, None, Some(1)));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 20, once(start));
    assert_eq!(of(&seen, BLAST).len(), 1, "the blast landed");
    assert_eq!(e.snapshot().expertise.unwrap().points, 1);
}

#[test]
fn an_orbit_s_hits_fire_no_explosion() {
    // An orb's circle 40 pixels out, turning a quarter turn in 6 ticks, 7 damage; the target 40
    // pixels to the right of the main character, out of the step's reach, which is never pressed.
    let orbit = json!({
        "id": "orbit", "rarity": "common", "effect": "orbit", "amount": 1, "limit": 1,
        "title": "Orbit", "description": "",
        "orbit": {
            "attack": { "inline": [
                { "kind": "active", "ticks": 60, "damage": 7, "rehit_ticks": 10,
                  "hit_box": { "type": "circle", "offset": { "forward": 10_240 },
                               "radius": 1_024 } }
            ] },
            "turn_per_tick": 15 * 256,
            "cooldown_ticks": 60
        }
    });
    let target = json!(wave(
        TARGET_TICK,
        1,
        0,
        enemy(
            100_000,
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
            json!([])
        ),
        START_X + 10_240,
        START_Y
    ));
    let mut e = with(
        &stage(4, target),
        &content(vec![random_rune(), orbit], None, runes(30, BLAST, 2, None)),
    );
    take_all(&mut e);
    take(&mut e, "orbit");
    let target = wait_for_target(&mut e);
    let seen = hits(&mut e, target, 240, |_| idle());
    assert!(!of(&seen, 7).is_empty(), "the orb hits: {seen:?}");
    assert!(of(&seen, BLAST).is_empty(), "{seen:?}");
    assert_eq!(cooldown(&e), 0);
}

#[test]
fn a_stage_skill_s_hits_fire_no_explosion() {
    // An attack 100 pixels ahead, on the tough target there, out of the step's reach.
    let ahead = 25_600;
    let nova = json!({ "id": "nova", "icon": "icon_nova", "title": "Nova", "description": "",
        "ahead": ahead,
        "levels": [{ "cooldown_ticks": 120, "effects": [{ "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 20, "damage": 7,
              "hit_box": { "type": "circle", "offset": { "forward": ahead }, "radius": 8_192 } }
        ] } }] }] });
    let reward = json!({ "id": "nova_r", "rarity": "common", "effect": "stage_skill",
                         "skill": "nova", "amount": 1, "limit": 1, "title": "Skill",
                         "description": "" });
    let target = json!(wave(
        TARGET_TICK,
        1,
        0,
        enemy(
            100_000,
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
            json!([])
        ),
        START_X,
        START_Y - ahead
    ));
    let mut e = with(
        &stage(4, target),
        &content(
            vec![random_rune(), reward],
            Some(json!([nova])),
            runes(30, BLAST, 2, None),
        ),
    );
    take_all(&mut e);
    take(&mut e, "nova_r");
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 40, |t| Input {
        skill: t == start,
        ..idle()
    });
    assert_eq!(of(&seen, 7).len(), 1, "the skill's hit: {seen:?}");
    assert!(of(&seen, BLAST).is_empty(), "{seen:?}");
    assert_eq!(cooldown(&e), 0);
}

/// Explosion's damage against the tough target weak to Pyro, holding a Pyro bonus of 50 and
/// the war cry's atk-up of 100 used before the step: `(base, step's hit, blast's hit)`.
fn boosted(ty: Option<&str>) -> (u32, u32) {
    let war_cry = json!({ "id": "war_cry", "icon": "icon_nova", "title": "War Cry",
        "description": "",
        "levels": [{ "cooldown_ticks": 1_000, "effects": [
            { "type": "self_status", "status": "atk_up", "value_pct": 100,
              "duration_ticks": 600 }
        ] }] });
    let rewards = vec![
        random_rune(),
        json!({ "id": "war_cry_r", "rarity": "common", "effect": "stage_skill",
                "skill": "war_cry", "amount": 1, "limit": 1, "title": "Skill",
                "description": "" }),
        json!({ "id": "pyro", "rarity": "common", "effect": "type_bonus_pct", "type": "pyro",
                "amount": 50, "limit": 1, "title": "Pyro", "description": "" }),
    ];
    let weak = json!({ "weak": ["pyro"] });
    let mut e = with(
        &stage(5, tough(0, Some(weak), None)),
        &content(rewards, Some(json!([war_cry])), runes(30, 10, 2, ty)),
    );
    take_all(&mut e);
    take(&mut e, "war_cry_r");
    take(&mut e, "pyro");
    let target = wait_for_target(&mut e);
    e.step(Input {
        skill: true,
        ..idle()
    });
    let start = e.tick() + 1;
    let seen = hits(&mut e, target, 10, once(start));
    assert_eq!(seen.len(), 2, "{seen:?}");
    (seen[0].1, seen[1].1)
}

#[test]
fn its_damage_goes_through_atk_up_the_type_bonus_and_the_match_up() {
    // The step, neutral: 10 × 200 / 100 = 20. The blast, Pyro: 10 × 200 / 100 = 20,
    // × 150 / 100 = 30, × 120 / 100 = 36.
    assert_eq!(boosted(Some("pyro")), (20, 36));
}

#[test]
fn a_neutral_explosion_gets_atk_up_but_no_type_bonus_or_match_up() {
    assert_eq!(boosted(None), (20, 20));
}

#[test]
fn the_state_hash_holds_the_cooldown() {
    // The same run with explosion's cooldown 30 or 40: the same until the first blast.
    let mut a = holding(runes(30, BLAST, 2, None), tough(0, None, None));
    let mut b = holding(runes(40, BLAST, 2, None), tough(0, None, None));
    wait_for_target(&mut a);
    wait_for_target(&mut b);
    assert_eq!(a.state_hash(), b.state_hash());
    let start = a.tick();
    a.step(once(start)(start));
    b.step(once(start)(start));
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn a_run_holding_explosion_records_and_verifies() {
    // The trial with a random rune and one stat reward, the hidden candidate always taken, and
    // the bundled explosion.
    let content = content(
        vec![
            random_rune(),
            json!({
                "id": "might", "rarity": "common", "effect": "attack_pct", "amount": 5,
                "limit": 20, "title": "Might", "description": ""
            }),
        ],
        None,
        Runes::bundled().clone(),
    );
    let mut recorder = RunRecorder::start_with(&trial_setup(), &content).unwrap();
    let mut fired = false;
    for t in 0..1_800 {
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
        fired |= recorder
            .engine()
            .snapshot()
            .player
            .runes
            .iter()
            .any(|r| r.cooldown_left > 0);
    }
    assert!(fired, "explosion fired in the run");
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
}
