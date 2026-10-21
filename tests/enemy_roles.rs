//! Regular enemy roles (R1): a tank's slow `follow` (its `speed`), an exploder's `near_target`
//! burst that can `self_destruct`, and a splitter's half-HP split (`hp_below` and `summon`, under
//! the cap of ADR-020). Small stages whose numbers are worked out by hand, as `tests/agents.rs`.

use crate::common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::Fx;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;

/// The main character's start (`tests/agents.rs`).
const PLAYER_X: i32 = 46_080;
const PLAYER_Y: i32 = 147_456;
/// The largest length a field may hold (`MAX_LEN`, ADR-020).
const MAX_LEN: i32 = 262_144;

// --- Builders --------------------------------------------------------------------------------

/// `(id, x, y, hp)` of every enemy alive.
fn enemies_of(e: &Engine) -> Vec<(u32, i32, i32, u32)> {
    e.snapshot()
        .entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .map(|v| (v.id.0, v.at.x.0, v.at.y.0, v.hp))
        .collect()
}

/// The only enemy alive: its `(x, y, hp)`.
fn only_enemy(e: &Engine) -> (i32, i32, u32) {
    let all = enemies_of(e);
    assert_eq!(all.len(), 1, "one enemy alive: {all:?}");
    (all[0].1, all[0].2, all[0].3)
}

fn follow(speed: Option<i32>) -> Value {
    let mut movement = json!({ "type": "follow", "distance": 0 });
    if let Some(speed) = speed {
        movement["speed"] = json!(speed);
    }
    movement
}

/// A stage whose one follower spawns 20,000 units below the main character, on its line.
fn chase(speed: Option<i32>) -> Stage {
    quiet(stage_with(
        18_000,
        Some(far_boss()),
        vec![wave(
            0,
            1,
            1,
            enemy(50, follow(speed), json!([])),
            PLAYER_X,
            PLAYER_Y - 20_000,
        )],
    ))
}

/// The follower's `y` on each of `ticks` + 1 idle ticks, from the first one it is alive.
fn follower_ys(speed: Option<i32>, ticks: u32) -> Vec<i32> {
    let mut e = engine(&chase(speed));
    (0..=ticks)
        .map(|_| {
            e.step(idle());
            only_enemy(&e).1
        })
        .collect()
}

/// An exploder 5,000 units above the main character on a hold, with a burst on `near_target`
/// `distance` (its centre to the main character's), which `self_destruct`s when asked.
fn exploder(distance: i32, self_destruct: bool) -> Stage {
    let mut burst = rule(
        json!({ "type": "near_target", "distance": distance }),
        json!("player"),
        telegraph(),
        0,
    );
    if self_destruct {
        burst["self_destruct"] = json!(true);
    }
    let at = PLAYER_Y + 5_000;
    quiet(stage_with(
        18_000,
        Some(far_boss()),
        vec![wave(
            0,
            1,
            1,
            enemy(50, hold(PLAYER_X, at), json!([burst])),
            PLAYER_X,
            at,
        )],
    ))
}

/// A child of a split: a small enemy that chases the main character.
fn child() -> Value {
    enemy(5, follow(None), json!([]))
}

/// A splitter 3,000 units above the main character on a hold, with `hp` hit points. Its one
/// split (`once`) summons `children` small enemies where it stands, when `when` holds.
fn splitter_with(hp: u32, when: Value, children: u8, extra_waves: Vec<Value>) -> Stage {
    let split = json!({
        "when": when,
        "target": "self",
        "do": { "type": "summon", "count": children, "agent": child() },
        "once": true,
    });
    let at = PLAYER_Y - 3_000;
    let mut waves = vec![wave(
        0,
        1,
        1,
        enemy(hp, hold(PLAYER_X, at), json!([split])),
        PLAYER_X,
        at,
    )];
    waves.extend(extra_waves);
    stage_with(18_000, Some(far_boss()), waves)
}

/// The number of times a rule fired in `ticks` idle ticks.
fn fired(stage: &Stage, ticks: u32) -> usize {
    let mut e = engine(stage);
    let mut count = 0;
    for _ in 0..ticks {
        e.step(idle());
        count += e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::RuleFired { .. }))
            .count();
    }
    count
}

// --- Tank: a follow at its own speed (ADR-036) -----------------------------------------------

#[test]
fn a_tank_follows_at_its_own_speed() {
    let slow = 256; // one pixel a tick
    let ys = follower_ys(Some(slow), 10);
    for pair in ys.windows(2) {
        assert_eq!(pair[1] - pair[0], slow, "{ys:?}");
    }
}

#[test]
fn a_follow_without_a_speed_moves_four_pixels_a_tick_as_it_always_did() {
    let four = Fx::px(4).0;
    let ys = follower_ys(None, 10);
    for pair in ys.windows(2) {
        assert_eq!(pair[1] - pair[0], four, "{ys:?}");
    }
    assert_eq!(ys, follower_ys(Some(four), 10));
}

#[test]
fn a_follow_speed_is_bounded_to_one_to_the_map_length() {
    assert!(Engine::new(&chase(Some(0))).is_err());
    assert!(Engine::new(&chase(Some(-1))).is_err());
    assert!(Engine::new(&chase(Some(MAX_LEN + 1))).is_err());
    assert!(Engine::new(&chase(Some(1))).is_ok());
    assert!(Engine::new(&chase(Some(MAX_LEN))).is_ok());
}

// --- Exploder: a burst near its target (ADR-036) ---------------------------------------------

#[test]
fn an_exploder_fires_within_its_distance_and_not_beyond_it() {
    // The exploder is 5,000 from the main character: the edge counts, so 5,000 fires and 4,999
    // does not.
    assert!(fired(&exploder(5_000, false), 20) > 0);
    assert_eq!(fired(&exploder(4_999, false), 20), 0);
}

#[test]
fn an_exploder_that_self_destructs_fires_once_and_is_gone() {
    let mut e = engine(&exploder(5_000, true));
    let mut firings = 0;
    for _ in 0..5 {
        e.step(idle());
        firings += e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::RuleFired { .. }))
            .count();
    }
    // Without `self_destruct` the burst would fire every tick (no cooldown): one firing and an
    // empty map show that it removed itself.
    assert_eq!(firings, 1);
    assert!(enemies_of(&e).is_empty(), "{:?}", enemies_of(&e));
}

#[test]
fn a_near_target_distance_is_bounded_to_one_to_the_map_length() {
    assert!(Engine::new(&exploder(0, false)).is_err());
    assert!(Engine::new(&exploder(MAX_LEN + 1, false)).is_err());
    assert!(Engine::new(&exploder(1, false)).is_ok());
    assert!(Engine::new(&exploder(MAX_LEN, false)).is_ok());
}

// --- Splitter: a split at half HP, under the cap (ADR-020, ADR-036) --------------------------

#[test]
fn a_splitter_splits_once_on_the_tick_after_it_drops_below_half() {
    // 100 hit points; a hit is a bullet of the main character's weapon, fired up the field.
    let mut e = engine(&splitter_with(100, hp_below_half(), 2, vec![]));
    e.step(idle());
    let parent = enemies_of(&e)[0].0;
    let mut crossed = None;
    let mut split = None;
    for _ in 0..600 {
        e.step(firing(idle(), e.tick()));
        let all = enemies_of(&e);
        let hp = all.iter().find(|v| v.0 == parent).map_or(0, |v| v.3);
        if crossed.is_none() && hp * 2 < 100 {
            crossed = Some(e.tick());
        }
        let children = all.iter().filter(|v| v.0 > parent).count();
        if children > 0 {
            split = Some((e.tick(), children));
            break;
        }
        if hp == 0 {
            break;
        }
    }
    // The condition is read in the next tick's act, so the children come one tick after the hit
    // that took it below half, and they are two.
    let crossed = crossed.expect("a hit took the splitter below half");
    assert_eq!(split, Some((crossed + 1, 2)));
    // Once only: the split never comes again, even while the splitter stays below half.
    for _ in 0..30 {
        e.step(idle());
        let children = enemies_of(&e).iter().filter(|v| v.0 > parent).count();
        assert!(children <= 2, "{children} children");
    }
}

#[test]
fn a_split_spawns_only_what_the_enemy_cap_has_room_for() {
    // 498 bats (7 hit points, away from the splitter) and the splitter make 499 alive; the cap of
    // 500 has room for one child, which has 5 hit points.
    // A wave holds at most 100, so the bats come in five waves.
    let at = PLAYER_Y + 8_000;
    let bats: Vec<Value> = [100, 100, 100, 100, 98]
        .into_iter()
        .map(|count| {
            wave(
                0,
                count,
                1,
                enemy(7, hold(PLAYER_X + 8_000, at), json!([])),
                PLAYER_X + 8_000,
                at,
            )
        })
        .collect();
    // The waves spawn one copy a tick, so the splitter waits until they are all alive.
    let when = json!({ "type": "time_above", "ticks": 150 });
    let mut e = engine(&splitter_with(100, when, 2, bats));
    for _ in 0..160 {
        e.step(idle());
    }
    let all = enemies_of(&e);
    assert_eq!(all.len(), 500, "the cap holds");
    assert_eq!(all.iter().filter(|v| v.3 == 5).count(), 1, "one child fits");
}

#[test]
fn a_split_with_room_spawns_all_its_children() {
    let mut e = engine(&splitter_with(100, always(), 2, vec![]));
    for _ in 0..5 {
        e.step(idle());
    }
    assert_eq!(enemies_of(&e).len(), 3, "the splitter and its two children");
}

/// The splitter's rule: when it is below half its hit points (strictly, `hp_below`).
fn hp_below_half() -> Value {
    json!({ "type": "hp_below", "who": "self", "pct": 50 })
}
