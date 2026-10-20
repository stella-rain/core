//! Stun (ADR-085): a smash's stun puts the `stun` status on the enemy it hits, for the smash's
//! ticks, and does what ADR-085 says while it lasts: it cancels the enemy's running attack, pauses
//! its movement and its rules (its cooldowns keep counting), and when it ends the enemy is immune
//! to a new stun for the run's `immune_ticks`. It does not stack: a stun on a stunned enemy keeps
//! the longer remainder. An elite takes its share of the ticks, and a boss takes none. Every
//! number here is worked out by hand from the placeholders: a 30-tick stun, 50 percent for an
//! elite and 60 ticks of immunity.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::Point;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::StatusView;
use stella_rain_core::stage::Stage;

use crate::common;

use common::on_hit::*;
use common::*;

/// A sub step of one circle 1,024 units forward, 10 damage and 4 ticks of recovery, with the
/// stun of `ticks` as its weapon's smash.
fn stunning(ticks: u32) -> Value {
    json!({ "inline": {
        "combo": { "sub": { "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10,
              "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } },
            { "kind": "recovery", "ticks": 4 }
        ] } } },
        "window_ticks": 30,
        "assist_range": 0,
        "smash": { "type": "stun", "ticks": ticks },
    } })
}

/// A stage of `waves` and `boss`, whose main character smashes with a stun of `ticks`. It has
/// 99 hit points, so it outlives the enemies' attacks in a test.
fn stage_with(ticks: u32, boss: Value, waves: Vec<Value>) -> Stage {
    let mut stage = armed(
        stage_full(18_000, Some(boss), waves, vec![]),
        stunning(ticks),
    );
    stage.player.hp = 99;
    stage
}

/// One enemy of 100,000 hit points at the target's place, with `movement` and `rules`.
fn enemy_with(movement: Value, rules: Value) -> Value {
    wave(
        TARGET_TICK,
        1,
        0,
        enemy(100_000, movement, rules),
        START_X,
        NEAR_Y,
    )
}

/// The tough target as an elite: an `elite` with a range of 256, so it takes half a stun.
fn elite_target() -> Value {
    let mut e = enemy(100_000, hold(START_X, NEAR_Y), json!([]));
    e["elite"] = json!({ "range": 256 });
    wave(TARGET_TICK, 1, 0, e, START_X, NEAR_Y)
}

/// What one run of ticks showed of one enemy (`play`).
#[derive(Default)]
struct Played {
    /// Each hit on it, with the `stun` statuses it shows the tick the hit lands.
    hits: Vec<(u32, Vec<StatusView>)>,
    /// The ticks its stun ended on.
    ended: Vec<u32>,
    /// The ticks its rules fired on.
    fired: Vec<u32>,
    /// Its place on each tick.
    at: Vec<(u32, Point)>,
}

/// The entity `id` in the snapshot.
fn entity(e: &Engine, id: EntityId) -> Option<stella_rain_core::snapshot::EntityView> {
    e.snapshot().entities.into_iter().find(|en| en.id == id)
}

/// The `stun` statuses of `id` in the snapshot.
fn stuns(e: &Engine, id: EntityId) -> Vec<StatusView> {
    entity(e, id)
        .map(|en| en.statuses)
        .unwrap_or_default()
        .into_iter()
        .filter(|st| st.id.0 == "stun")
        .collect()
}

/// Runs `ticks` ticks, pressing `sub` on each tick of `presses`, and records what happens to `id`.
fn play(e: &mut Engine, id: EntityId, presses: &[u32], ticks: u32) -> Played {
    let mut p = Played::default();
    for _ in 0..ticks {
        let t = e.tick();
        e.step(Input {
            sub: presses.contains(&t),
            ..idle()
        });
        let now = e.tick();
        let mut hit = false;
        for ev in e.events() {
            match ev {
                DomainEvent::Hit { target, .. } if *target == id => hit = true,
                DomainEvent::StunEnded { target } if *target == id => p.ended.push(now),
                DomainEvent::RuleFired { agent, .. } if *agent == id => p.fired.push(now),
                _ => {}
            }
        }
        if hit {
            p.hits.push((now, stuns(e, id)));
        }
        if let Some(en) = entity(e, id) {
            p.at.push((now, en.at));
        }
    }
    p
}

#[test]
fn a_smash_stuns_the_enemy_it_hits_for_its_ticks() {
    let mut e = engine(&stage_with(30, far_boss(), vec![tough(0, None, None)]));
    let target = wait_for_target(&mut e);
    assert!(stuns(&e, target).is_empty());
    let start = e.tick();
    let p = play(&mut e, target, &[start], 8);
    let (_, first) = p.hits.first().expect("the smash hits the target");
    assert_eq!(first.len(), 1, "one stun status");
    assert_eq!(first[0].id.0, "stun");
    assert_eq!(first[0].total, 30);
    assert_eq!(first[0].remaining, 30);
}

#[test]
fn a_second_stun_does_not_stack_and_keeps_the_longer_remainder() {
    // The second smash lands 10 ticks into the first stun: it refreshes the stun to its own 30
    // ticks (longer than the 20 left), and there is still one stun status.
    let mut e = engine(&stage_with(30, far_boss(), vec![tough(0, None, None)]));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let p = play(&mut e, target, &[start, start + 10], 20);
    assert!(p.hits.len() >= 2, "two smashes hit: {:?}", p.hits.len());
    for (_, stuns) in &p.hits {
        assert_eq!(stuns.len(), 1, "one stun status: {stuns:?}");
    }
    assert_eq!(p.hits[1].1[0].remaining, 30, "the longer stun is kept");
}

#[test]
fn an_elite_takes_half_the_ticks_of_a_stun() {
    let mut e = engine(&stage_with(30, far_boss(), vec![elite_target()]));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let p = play(&mut e, target, &[start], 8);
    let (_, first) = p.hits.first().expect("the smash hits the elite");
    assert_eq!(first[0].total, 15, "an elite takes 50 percent of 30 ticks");
}

#[test]
fn a_boss_takes_no_stun_from_a_smash() {
    let mut e = engine(&stage_with(30, boss_at(100_000, NEAR_Y), vec![]));
    let mut hit = None;
    for _ in 0..10 {
        let t = e.tick();
        e.step(Input {
            sub: t == 0,
            ..idle()
        });
        for ev in e.events() {
            if let DomainEvent::Hit { target, damage, .. } = ev {
                assert!(*damage > 0, "the smash hits the boss");
                hit = Some(*target);
            }
        }
    }
    let boss = hit.expect("the smash hits the boss");
    assert!(stuns(&e, boss).is_empty(), "a boss resists a stun");
}

#[test]
fn a_stun_cancels_the_enemy_s_running_attack() {
    // The enemy starts its attack the tick it appears; the smash lands the tick after, and the
    // stun takes the attack away with it.
    let rules = json!([attack_rule(always(), 1000)]);
    let mut e = engine(&stage_with(
        30,
        far_boss(),
        vec![enemy_with(hold(START_X, NEAR_Y), rules)],
    ));
    let target = wait_for_target(&mut e);
    assert!(entity(&e, target).unwrap().attack.is_some(), "it attacks");
    let start = e.tick();
    play(&mut e, target, &[start], 3);
    assert!(
        entity(&e, target).unwrap().attack.is_none(),
        "the attack is cancelled"
    );
}

#[test]
fn a_stunned_enemy_fires_no_rule_until_the_stun_ends() {
    // A rule that is ready from 10 ticks after the enemy appears fires on the tick it is ready
    // without a stun. Here the smash lands 1 tick after the enemy appears and stuns it for 30
    // ticks, so the rule holds its fire until the stun runs out, and then fires at once.
    let rules = json!([attack_rule(
        json!({ "type": "time_above", "ticks": 10 }),
        1000
    )]);
    let mut e = engine(&stage_with(
        30,
        far_boss(),
        vec![enemy_with(hold(START_X, NEAR_Y), rules.clone())],
    ));
    let target = wait_for_target(&mut e);
    let base = play(&mut e, target, &[], 60);
    assert_eq!(
        base.fired.first(),
        Some(&(TARGET_TICK + 10)),
        "unstunned, it fires when ready"
    );
    let mut e = engine(&stage_with(
        30,
        far_boss(),
        vec![enemy_with(hold(START_X, NEAR_Y), rules)],
    ));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let p = play(&mut e, target, &[start], 60);
    let landed = p.hits[0].0;
    assert_eq!(
        p.fired.first(),
        Some(&(landed + 30)),
        "it holds its fire through the stun: {:?}",
        p.fired
    );
}

#[test]
fn a_stunned_enemy_does_not_move_until_the_stun_ends() {
    // An enemy that follows the main character moves every tick, except while stunned.
    let follow = json!({ "type": "follow", "distance": 0 });
    let mut e = engine(&stage_with(
        30,
        far_boss(),
        vec![enemy_with(follow, json!([]))],
    ));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let p = play(&mut e, target, &[start], 60);
    let landed = p.hits[0].0;
    // The stun runs out at `landed + 30`, and the enemy moves on that tick, so the held ticks are
    // the 30 before it.
    let held: Vec<Point> =
        p.at.iter()
            .filter(|(t, _)| *t >= landed && *t < landed + 30)
            .map(|(_, at)| *at)
            .collect();
    assert_eq!(held.len(), 30, "the window is measured");
    assert!(held.iter().all(|at| *at == held[0]), "it does not move");
    let after = p.at.iter().find(|(t, _)| *t >= landed + 30).unwrap().1;
    assert_ne!(after, held[0], "it moves again once the stun ends");
}

#[test]
fn a_stun_ends_and_the_enemy_is_immune_for_the_immunity_ticks() {
    // The smashes land 1 tick after each press: at 1, 41 and 101 ticks into the run. The stun
    // lands on the first and ends 30 ticks on; the immunity of 60 ticks then holds the second
    // smash (41) off, and the third (101) stuns again.
    let mut e = engine(&stage_with(30, far_boss(), vec![tough(0, None, None)]));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let p = play(&mut e, target, &[start, start + 40, start + 100], 110);
    assert_eq!(p.hits.len(), 3, "three smashes hit: {:?}", p.hits.len());
    assert_eq!(p.hits[0].1.len(), 1, "the first stuns");
    assert!(p.hits[1].1.is_empty(), "the second is in the immunity");
    assert_eq!(p.hits[2].1.len(), 1, "the third stuns again");
    assert_eq!(p.ended, vec![p.hits[0].0 + 30], "the stun ends 30 ticks on");
}
