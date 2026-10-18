//! The thunder rune (ADR-099 points 2, 3, 7 and 8; ADR-100): an on-hit rune of its own, Electro,
//! on its own cooldown. When a combo step of the Main weapon hits and thunder's cooldown is 0,
//! it calls its strike down on the point hit: a wind-up that warns, then the hit. Its damage is
//! the main character's without a combo step, through atk-up, the Electro bonus and the
//! target's match-up; its hits give no expertise and fire no rune. Held with explosion, each
//! fires and cools down on its own. Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::Engine;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, verify_run_with};
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;

mod common;

use common::on_hit::*;
use common::run::{run_input, trial_setup};
use common::*;

/// What the test thunder does: 4 base damage, against the step's 10 and the blast's 3.
const STRIKE: u32 = 4;
const BLAST: u32 = 3;
/// The test thunder's wind-up: 4 ticks, counting the tick it is called.
const WIND_UP: u32 = 4;

/// Thunder's entry: `cooldown`, `damage`, a wind-up of `WIND_UP` ticks then 2 active ticks, a
/// circle of 16 pixels round the point hit, the strike of type `ty` (or neutral).
fn thunder(cooldown: u32, damage: u32, ty: Option<&str>) -> Value {
    let mut strike = json!({ "kind": "active", "ticks": 2,
                             "hit_box": { "type": "circle", "radius": 4_096 } });
    if let Some(ty) = ty {
        strike["type"] = json!(ty);
    }
    entry(
        "thunder",
        cooldown,
        damage,
        json!([
            { "kind": "wind_up", "ticks": WIND_UP,
              "hit_box": { "type": "circle", "radius": 4_096 } },
            strike
        ]),
    )
}

/// Thunder of `cooldown`, `damage` and type `ty`; explosion never hits.
fn runes(cooldown: u32, damage: u32, ty: Option<&str>) -> Runes {
    runes_of(vec![far("explosion"), thunder(cooldown, damage, ty)])
}

/// The stage with two small enemies and the tough target, both runes taken from their stars.
fn holding(runes: Runes, target: Value) -> Engine {
    let mut e = with(
        &stage(2, target),
        &content(vec![random_rune()], None, runes),
    );
    take_both(&mut e);
    e
}

fn cooldown(e: &Engine) -> u32 {
    cooldown_of(e, Rune::Thunder)
}

#[test]
fn a_combo_step_hit_calls_thunder_down_on_the_point_hit_after_its_wind_up() {
    let mut e = holding(runes(60, STRIKE, Some("electro")), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    e.step(once(start)(start));
    let step_hit = e.tick();
    assert_eq!(cooldown(&e), 60, "set as it fires");
    let snapshot = e.snapshot();
    let strike = snapshot
        .player
        .attacks
        .iter()
        .find(|a| a.slot == THUNDER_SLOT)
        .expect("thunder's attack");
    assert_eq!(strike.kind, PhaseKind::WindUp, "it warns first");
    assert_eq!(
        strike.at,
        Some(Point {
            x: Fx(START_X),
            y: Fx(NEAR_Y)
        }),
        "on the target hit"
    );
    // The wind-up counts the tick it is called and 3 more; the strike lands on the next, once.
    let later = hits(&mut e, target, 20, once(start));
    assert_eq!(of(&later, STRIKE), [step_hit + WIND_UP]);
    assert_eq!(cooldown(&e), 40, "60 less 20 ticks");
}

#[test]
fn its_cooldown_blocks_a_second_strike_until_it_reaches_0() {
    let mut e = holding(runes(60, STRIKE, None), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let seen = hits(&mut e, target, 300, |t| firing(idle(), t));
    let steps = of(&seen, 10);
    let strikes = of(&seen, STRIKE);
    assert!(strikes.len() >= 4, "{strikes:?}");
    for pair in strikes.windows(2) {
        // At the first step hit once 60 ticks have gone.
        assert!((60..66).contains(&(pair[1] - pair[0])), "{strikes:?}");
    }
    for s in &strikes {
        assert!(steps.contains(&(s - WIND_UP)), "{s}: {steps:?}");
    }
}

#[test]
fn held_with_explosion_each_fires_on_its_own_cooldown() {
    // Explosion: 3 damage a tick after the hit, every 30 ticks; thunder: 4 damage after its
    // wind-up, every 60. The same step hits fire both.
    let explosion = entry(
        "explosion",
        30,
        BLAST,
        json!([{ "kind": "active", "ticks": 2,
                 "hit_box": { "type": "circle", "radius": 4_096 } }]),
    );
    let runes = runes_of(vec![explosion, thunder(60, STRIKE, None)]);
    let mut e = holding(runes, tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    e.step(once(start)(start));
    let step_hit = e.tick();
    assert_eq!(
        (cooldown_of(&e, Rune::Explosion), cooldown(&e)),
        (30, 60),
        "one hit fires both"
    );
    let seen = hits(&mut e, target, 299, |t| firing(idle(), t));
    let (blasts, strikes) = (of(&seen, BLAST), of(&seen, STRIKE));
    assert_eq!(blasts[0], step_hit + 1);
    assert_eq!(strikes[0], step_hit + WIND_UP);
    for pair in blasts.windows(2) {
        assert!((30..36).contains(&(pair[1] - pair[0])), "{blasts:?}");
    }
    for pair in strikes.windows(2) {
        assert!((60..66).contains(&(pair[1] - pair[0])), "{strikes:?}");
    }
    assert!(
        blasts.len() >= 2 * strikes.len() - 1,
        "{blasts:?} {strikes:?}"
    );
}

#[test]
fn its_hits_fire_no_rune_one_strike_for_one_step() {
    let mut e = holding(runes(6, STRIKE, None), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 60, once(start));
    assert_eq!(of(&seen, 10).len(), 1);
    assert_eq!(of(&seen, STRIKE).len(), 1);
}

#[test]
fn its_hits_give_no_expertise() {
    // The target gives 1 point a hit: the step's, not the strike's.
    let mut e = holding(runes(60, STRIKE, None), tough(0, None, Some(1)));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 20, once(start));
    assert_eq!(of(&seen, STRIKE).len(), 1, "the strike landed");
    assert_eq!(e.snapshot().expertise.unwrap().points, 1);
}

/// Thunder's damage against the tough target of `profile`, holding an Electro bonus of 50
/// and the war cry's atk-up of 100 used before the step: `(step's hit, strike's hit)`.
fn boosted(ty: Option<&str>, profile: Value) -> (u32, u32) {
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
        json!({ "id": "electro", "rarity": "common", "effect": "type_bonus_pct",
                "type": "electro", "amount": 50, "limit": 1, "title": "Electro",
                "description": "" }),
    ];
    let mut e = with(
        &stage(4, tough(0, Some(profile), None)),
        &content(rewards, Some(json!([war_cry])), runes(60, 10, ty)),
    );
    take_both(&mut e);
    take(&mut e, "war_cry_r");
    take(&mut e, "electro");
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
fn an_electro_strike_goes_through_atk_up_the_electro_bonus_and_the_match_up() {
    // The step, neutral: 10 × 200 / 100 = 20. The strike: 10 × 200 / 100 = 20,
    // × 150 / 100 = 30, then × 120 / 100 = 36 weak, × 80 / 100 = 24 strong or resisted.
    let electro = Some("electro");
    assert_eq!(boosted(electro, json!({ "weak": ["electro"] })), (20, 36));
    assert_eq!(boosted(electro, json!({ "strong": ["electro"] })), (20, 24));
    assert_eq!(boosted(electro, json!({ "resist": "elements" })), (20, 24));
    // A Pyro weakness is no Electro match-up.
    assert_eq!(boosted(electro, json!({ "weak": ["pyro"] })), (20, 30));
}

#[test]
fn a_neutral_strike_gets_atk_up_but_no_electro_bonus_or_match_up() {
    assert_eq!(boosted(None, json!({ "weak": ["electro"] })), (20, 20));
}

#[test]
fn the_state_hash_holds_its_cooldown() {
    let mut a = holding(runes(60, STRIKE, None), tough(0, None, None));
    let mut b = holding(runes(70, STRIKE, None), tough(0, None, None));
    wait_for_target(&mut a);
    wait_for_target(&mut b);
    assert_eq!(a.state_hash(), b.state_hash());
    let start = a.tick();
    a.step(once(start)(start));
    b.step(once(start)(start));
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn a_run_holding_thunder_records_and_verifies() {
    // The trial with a random rune and one stat reward, the hidden candidate always taken, and
    // the bundled runes.
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
            .any(|r| r.rune == Rune::Thunder && r.cooldown_left > 0);
    }
    assert!(fired, "thunder fired in the run");
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
}
