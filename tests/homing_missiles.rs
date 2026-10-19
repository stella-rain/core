//! The homing missiles rune (ADR-099 points 2, 3, 7 and 8; ADR-100): an on-hit rune of its own,
//! neutral, on its own cooldown. When a combo step of the Main weapon hits and its cooldown is
//! 0, it launches its missiles from the main character, each a hit box in its own slot that
//! steers each tick toward its target where the target is now, so that it is on it by its
//! phase's last tick. The targets are aim assist's order (ADR-072) within the rune's range of
//! the main character: the boss and its parts first, then elites, then the nearest, then the
//! lower entity ID; missile k takes the k-th, starting over when there are fewer targets than
//! missiles, and with none in range every missile takes the enemy the step hit. Nothing is
//! drawn from the generator. Each missile hits one enemy once, then is gone. No bullet: the
//! bullets alive from runes stay at 0. Its damage is the main character's without a combo
//! step, through atk-up but no type bonus and no match-up; its hits give no expertise and fire
//! no rune. Every number here is worked out by hand; explosion, thunder and boomerang never hit.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, verify_run_with};
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;
use stella_rain_core::stage::Stage;

mod common;

use common::on_hit::*;
use common::run::{TRIAL_LENGTH, run_input, trial_setup};
use common::*;

/// What the test missiles do: 4 base damage, against the step's 10 and the blast's 3.
const MISSILE: u32 = 4;
const BLAST: u32 = 3;
/// Each missile's flight: 8 ticks, the tick it launches counted.
const FLIGHT: u32 = 8;
/// The test range: 160 pixels.
const RANGE: i32 = 40_960;

/// Homing missiles' entry: `cooldown`, `damage`, `missiles` of one active phase of `FLIGHT`
/// ticks with a circle of 4 pixels, `forward` ahead of where each flies, picking within `range`.
fn homing(cooldown: u32, damage: u32, missiles: u32, range: i32, forward: i32) -> Value {
    let mut e = entry(
        "homing_missiles",
        cooldown,
        damage,
        json!([{ "kind": "active", "ticks": FLIGHT,
                 "hit_box": { "type": "circle", "offset": { "forward": forward },
                              "radius": 1_024 } }]),
    );
    e["missiles"] = json!(missiles);
    e["range"] = json!(range);
    e
}

/// Homing missiles of `missiles` within `range`, 60 ticks apart and of `MISSILE` damage.
fn runes(missiles: u32, range: i32) -> Runes {
    with_homing(homing(60, MISSILE, missiles, range, 0))
}

fn with_homing(homing: Value) -> Runes {
    runes_of(vec![
        far("explosion"),
        far("thunder"),
        far("boomerang"),
        homing,
    ])
}

/// An enemy of 100 000 hit points held at `(START_X + dx, START_Y + dy)` from `TARGET_TICK`.
fn other(dx: i32, dy: i32) -> Value {
    wave(
        TARGET_TICK,
        1,
        0,
        enemy(100_000, hold(START_X + dx, START_Y + dy), json!([])),
        START_X + dx,
        START_Y + dy,
    )
}

/// The stage with four small enemies, the tough target 32 pixels above and `others`.
fn arena(others: Vec<Value>) -> Stage {
    let mut s = stage(4, tough(0, None, None));
    for w in others {
        s.waves.push(serde_json::from_value(w).unwrap());
    }
    s
}

/// `stage`, every rune taken from its star, stepped until the targets are on the map.
fn holding(stage: &Stage, runes: Runes) -> Engine {
    let mut e = with(stage, &content(vec![random_rune()], None, runes));
    take_all(&mut e);
    wait_for_target(&mut e);
    e
}

fn cooldown(e: &Engine) -> u32 {
    cooldown_of(e, Rune::HomingMissiles)
}

/// The enemy at `(START_X + dx, START_Y + dy)`.
fn at(e: &Engine, dx: i32, dy: i32) -> EntityId {
    let p = point(START_X + dx, START_Y + dy);
    e.snapshot()
        .entities
        .iter()
        .find(|en| en.at == p)
        .map(|en| en.id)
        .unwrap_or_else(|| panic!("an enemy at {dx}, {dy}"))
}

/// Where each missile flying is centred, by slot.
fn missiles(e: &Engine) -> Vec<(u8, Point)> {
    let snapshot = e.snapshot();
    snapshot
        .player
        .attacks
        .iter()
        .filter(|a| a.slot >= MISSILE_SLOT)
        .map(|a| (a.slot, a.at.expect("a missile is centred where it flies")))
        .collect()
}

fn point(x: i32, y: i32) -> Point {
    Point { x: Fx(x), y: Fx(y) }
}

/// Steps `n` ticks with `input`, collecting every hit: `(tick, target, damage)`.
fn every_hit(e: &mut Engine, n: u32, input: impl Fn(u32) -> Input) -> Vec<(u32, EntityId, u32)> {
    let mut out = Vec::new();
    for _ in 0..n {
        e.step(input(e.tick()));
        for ev in e.events() {
            if let DomainEvent::Hit { target, damage, .. } = ev {
                out.push((e.tick(), *target, *damage));
            }
        }
    }
    out
}

/// The targets of the missiles' hits, in order: their entity IDs, sorted within a tick.
fn missile_targets(seen: &[(u32, EntityId, u32)]) -> Vec<EntityId> {
    let mut hits: Vec<(u32, EntityId)> = seen
        .iter()
        .filter(|h| h.2 == MISSILE)
        .map(|h| (h.0, h.1))
        .collect();
    hits.sort();
    hits.into_iter().map(|h| h.1).collect()
}

#[test]
fn a_combo_step_hit_launches_its_missiles_from_the_main_character_and_each_hits_once() {
    let mut e = holding(&arena(vec![]), runes(3, RANGE));
    let target = at(&e, 0, NEAR_Y - START_Y);
    let start = e.tick();
    e.step(once(start)(start));
    let fired = e.tick();
    assert_eq!(cooldown(&e), 60, "set as it fires");
    // Three missiles, in their own slots, each a step of 32 / 8 = 4 pixels up on the tick they
    // launch.
    let first = point(START_X, START_Y - 1_024);
    assert_eq!(
        missiles(&e),
        [
            (MISSILE_SLOT, first),
            (MISSILE_SLOT + 1, first),
            (MISSILE_SLOT + 2, first)
        ]
    );
    let seen = every_hit(&mut e, FLIGHT, |_| idle());
    // Within 12 pixels of the target (its 8 and the missile's 4) after 5 steps of 4 pixels:
    // the three hit it together, once each, and are gone.
    let hits: Vec<_> = seen.iter().filter(|h| h.2 == MISSILE).collect();
    assert_eq!(hits.len(), 3, "{seen:?}");
    for h in &hits {
        assert_eq!((h.0, h.1), (fired + 4, target), "{seen:?}");
    }
    assert!(missiles(&e).is_empty(), "gone once they hit");
}

#[test]
fn the_missiles_fly_toward_where_the_target_is_now() {
    // The target walks right a pixel a tick; one missile, landing 256 pixels ahead of where it
    // flies, never hits and is on the target on its last tick.
    let walker = enemy(
        100_000,
        json!({ "type": "straight", "vx": 256, "vy": 0 }),
        json!([]),
    );
    let s = stage(4, wave(TARGET_TICK, 1, 0, walker, START_X, NEAR_Y));
    let mut e = with(
        &s,
        &content(
            vec![random_rune()],
            None,
            with_homing(homing(60, MISSILE, 1, RANGE, 65_536)),
        ),
    );
    take_all(&mut e);
    let target = wait_for_target(&mut e);
    let start = e.tick();
    e.step(once(start)(start));
    assert_eq!(missiles(&e).len(), 1, "launched");
    for _ in 0..FLIGHT - 1 {
        e.step(idle());
    }
    let snapshot = e.snapshot();
    let now = snapshot.entities.iter().find(|en| en.id == target).unwrap();
    assert!(now.at.x.0 > START_X + 1_024, "it walked: {:?}", now.at);
    assert_eq!(missiles(&e), [(MISSILE_SLOT, now.at)], "on the target");
    e.step(idle());
    assert!(missiles(&e).is_empty(), "its flight is over");
}

#[test]
fn each_missile_takes_the_next_target_in_aim_assist_order() {
    // The tough target 32 pixels above; one 48 pixels right and 16 up; one 96 pixels left and
    // 32 up. Each missile's path passes 30 pixels from the others, beyond its reach.
    let others = || vec![other(12_288, -4_096), other(-24_576, -8_192)];
    let order = |n: u32| {
        let mut e = holding(&arena(others()), runes(n, RANGE));
        let ids = [
            at(&e, 0, NEAR_Y - START_Y),
            at(&e, 12_288, -4_096),
            at(&e, -24_576, -8_192),
        ];
        let start = e.tick();
        let seen = every_hit(&mut e, 20, once(start));
        assert_eq!(seen.iter().filter(|h| h.2 == 10).count(), 1, "{seen:?}");
        let mut hit = missile_targets(&seen);
        hit.sort();
        let mut want: Vec<EntityId> = match n {
            1 => vec![ids[0]],
            2 => vec![ids[0], ids[1]],
            3 => ids.to_vec(),
            // Fewer targets than missiles: the fourth starts over at the nearest.
            _ => vec![ids[0], ids[0], ids[1], ids[2]],
        };
        want.sort();
        assert_eq!(hit, want, "{n} missiles: {seen:?}");
    };
    for n in 1..=4 {
        order(n);
    }
}

#[test]
fn the_boss_comes_before_a_nearer_enemy() {
    // A boss 112 pixels right and 32 up, its circle 32: within range, beyond the step's reach,
    // and its missile's path 30 pixels from the tough target.
    let mut s = arena(vec![]);
    let mut boss = boss_at(100_000, START_Y - 8_192);
    boss["spawn"]["x"] = json!(START_X + 28_672);
    s.boss = Some(serde_json::from_value(boss).unwrap());
    let mut e = holding(&s, runes(2, RANGE));
    let tough = at(&e, 0, NEAR_Y - START_Y);
    let boss = at(&e, 28_672, -8_192);
    let start = e.tick();
    let seen = every_hit(&mut e, 30, once(start));
    let mut hit = missile_targets(&seen);
    hit.sort();
    let mut want = vec![boss, tough];
    want.sort();
    assert_eq!(hit, want, "{seen:?}");
}

#[test]
fn an_enemy_out_of_range_is_no_target_and_without_one_they_take_the_enemy_hit() {
    // A range of 16 pixels: neither the tough target nor the enemy 48 pixels right is in it, so
    // both missiles take the target the step hit.
    let mut e = holding(&arena(vec![other(12_288, -4_096)]), runes(2, 4_096));
    let tough = at(&e, 0, NEAR_Y - START_Y);
    let start = e.tick();
    let seen = every_hit(&mut e, 20, once(start));
    assert_eq!(missile_targets(&seen), [tough, tough], "{seen:?}");
}

#[test]
fn the_pick_draws_nothing_from_the_generator() {
    // The same stage under other seeds: the runes are drawn in another order, the missiles hit
    // the same targets on the same ticks.
    let run = |seed: u32| {
        let mut s = arena(vec![other(12_288, -4_096), other(-24_576, -8_192)]);
        s.seed = seed;
        let mut e = holding(&s, runes(3, RANGE));
        let seen = every_hit(&mut e, 200, |t| firing(idle(), t));
        let missiles: Vec<_> = seen.into_iter().filter(|h| h.2 == MISSILE).collect();
        assert!(missiles.len() >= 6, "{missiles:?}");
        missiles
    };
    let first = run(0);
    for seed in 1..6 {
        assert_eq!(run(seed), first, "seed {seed}");
    }
}

#[test]
fn its_cooldown_blocks_a_second_launch_until_it_reaches_0() {
    let mut e = holding(&arena(vec![]), runes(1, RANGE));
    let seen = every_hit(&mut e, 300, |t| firing(idle(), t));
    let steps: Vec<u32> = seen.iter().filter(|h| h.2 == 10).map(|h| h.0).collect();
    let missiles: Vec<u32> = seen
        .iter()
        .filter(|h| h.2 == MISSILE)
        .map(|h| h.0)
        .collect();
    assert!(missiles.len() >= 4, "{missiles:?}");
    for m in &missiles {
        assert!(
            steps.contains(&(m - 4)),
            "{m} after a step's hit: {steps:?}"
        );
    }
    for pair in missiles.windows(2) {
        assert!((60..66).contains(&(pair[1] - pair[0])), "{missiles:?}");
    }
}

#[test]
fn held_with_explosion_each_fires_on_its_own_cooldown() {
    let explosion = entry(
        "explosion",
        30,
        BLAST,
        json!([{ "kind": "active", "ticks": 2,
                 "hit_box": { "type": "circle", "radius": 4_096 } }]),
    );
    let runes = runes_of(vec![
        explosion,
        far("thunder"),
        far("boomerang"),
        homing(60, MISSILE, 1, RANGE, 0),
    ]);
    let mut e = holding(&arena(vec![]), runes);
    let start = e.tick();
    e.step(once(start)(start));
    assert_eq!(
        (cooldown_of(&e, Rune::Explosion), cooldown(&e)),
        (30, 60),
        "one hit fires both"
    );
    let seen = every_hit(&mut e, 299, |t| firing(idle(), t));
    let of = |d: u32| -> Vec<u32> { seen.iter().filter(|h| h.2 == d).map(|h| h.0).collect() };
    let (blasts, missiles) = (of(BLAST), of(MISSILE));
    for pair in blasts.windows(2) {
        assert!((30..36).contains(&(pair[1] - pair[0])), "{blasts:?}");
    }
    for pair in missiles.windows(2) {
        assert!((60..66).contains(&(pair[1] - pair[0])), "{missiles:?}");
    }
    assert!(
        blasts.len() >= 2 * missiles.len() - 1,
        "{blasts:?} {missiles:?}"
    );
}

#[test]
fn its_hits_fire_no_rune() {
    // Explosion, ready again 2 ticks after it fires, would fire on the missiles' hit.
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
        far("boomerang"),
        homing(60, MISSILE, 2, RANGE, 0),
    ]);
    let mut e = holding(&arena(vec![]), runes);
    let start = e.tick();
    let seen = every_hit(&mut e, 60, once(start));
    let count = |d: u32| seen.iter().filter(|h| h.2 == d).count();
    assert_eq!(count(10), 1);
    assert_eq!(count(MISSILE), 2, "both missiles");
    assert_eq!(count(BLAST), 1, "the step's blast only: {seen:?}");
}

#[test]
fn its_hits_give_no_expertise() {
    // The target gives 1 point a hit: the step's, not the missiles' two.
    let mut e = with(
        &stage(4, tough(0, None, Some(1))),
        &content(vec![random_rune()], None, runes(2, RANGE)),
    );
    take_all(&mut e);
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 20, once(start));
    assert_eq!(of(&seen, MISSILE).len(), 2, "the missiles landed");
    assert_eq!(e.snapshot().expertise.unwrap().points, 1);
}

/// The damage on the tough target of `profile`, holding a bonus of 50 for type `ty` and the
/// war cry's atk-up of 100 used before the step: `(step's hit, the missile's)`.
fn boosted(ty: &str, profile: Value) -> (u32, Vec<u32>) {
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
        json!({ "id": "bonus", "rarity": "common", "effect": "type_bonus_pct",
                "type": ty, "amount": 50, "limit": 1, "title": "Type",
                "description": "" }),
    ];
    let runes = with_homing(homing(60, 10, 1, RANGE, 0));
    let mut e = with(
        &stage(6, tough(0, Some(profile), None)),
        &content(rewards, Some(json!([war_cry])), runes),
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
    assert_eq!(seen.len(), 2, "{seen:?}");
    (seen[0].1, seen[1..].iter().map(|h| h.1).collect())
}

#[test]
fn a_neutral_missile_gets_atk_up_but_no_type_bonus_or_match_up() {
    // The step, neutral: 10 × 200 / 100 = 20. The missile: 10 × 200 / 100 = 20, whatever bonus
    // is held and whatever the target is weak to or resists.
    for ty in ["pierce", "electro"] {
        for profile in [
            json!({ "weak": [ty] }),
            json!({ "strong": [ty] }),
            json!({ "resist": "physical" }),
            json!({ "resist": "elements" }),
        ] {
            assert_eq!(boosted(ty, profile.clone()), (20, vec![20]), "{profile}");
        }
    }
}

#[test]
fn no_bullet_comes_from_it() {
    let mut e = holding(&arena(vec![]), runes(4, RANGE));
    let start = e.tick();
    for _ in 0..FLIGHT + 2 {
        e.step(once(start)(e.tick()));
        assert!(e.snapshot().bullets.is_empty(), "hit boxes, never bullets");
    }
}

#[test]
fn the_state_hash_holds_its_flight() {
    // The same cooldown and damage, but flights of 8 and of 16 ticks: the missiles differ only
    // in how far they fly on the tick they launch.
    let mut slower = homing(60, MISSILE, 2, RANGE, 0);
    slower["attack"][0]["ticks"] = json!(16);
    let mut a = holding(&arena(vec![]), runes(2, RANGE));
    let mut b = holding(&arena(vec![]), with_homing(slower));
    assert_eq!(a.state_hash(), b.state_hash());
    let start = a.tick();
    a.step(once(start)(start));
    b.step(once(start)(start));
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn a_run_holding_homing_missiles_records_and_verifies() {
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
        fired |= recorder
            .engine()
            .snapshot()
            .player
            .runes
            .iter()
            .any(|r| r.rune == Rune::HomingMissiles && r.cooldown_left > 0);
    }
    assert!(fired, "homing missiles fired in the run");
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
}
