//! The ranged tokens (RA1, DS82, DS83): the 4 tokens of `MAX_RANGED_ATTACKERS` are 3 zone tokens
//! and 1 line token, a ranged attack taking the line one when any of its phases lays a line
//! zone. At the start of the act step each pool grants its free tokens to the nearest ready
//! regular enemies in the 840 by 360 px view (edges included), ties by entity ID; an enemy out of
//! view starts no ranged attack. A token a run stops holding (recovery, or its owner gone) cools
//! for 30 ticks before the pool can grant it again. A stunned enemy, or one whose ranged rule's
//! condition or selector fails at that moment, is no candidate: it could not use the grant, and
//! would take it from a shooter that can (RA-STUN).

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::PhaseKind;
use stella_rain_core::engine::Engine;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::stage::{Map, Stage};

const PX: i32 = 256;
/// The main character stands at (2,000, 2,000) px of a 4,000 px square map.
const CENTRE: i32 = 2000;
/// The boss is entity 1; the shooters come after it, in wave order.
const FIRST_SHOOTER: u32 = 2;
/// Ticks a freed token cools.
const COOLDOWN: u32 = 30;
/// Three ticks of wind-up, three of active, then recovery: a token is held for six ticks.
const HELD_TICKS: u32 = 6;

/// A shooter at (`dx`, `dy`) px from the main character; `line` makes its attack lay a line zone.
#[derive(Clone, Copy)]
struct Shooter {
    dx: i32,
    dy: i32,
    line: bool,
    hp: u32,
    gate: Gate,
}

/// What the shooter's ranged rule asks before it fires.
#[derive(Clone, Copy)]
enum Gate {
    /// Nothing: `always`.
    Open,
    /// The shooter's age reaches this many ticks: room to stun it first.
    After(u32),
    /// A condition that is false: the main character is below 0 percent of its hit points.
    ConditionFails,
}

fn zone_shooter(dx: i32, dy: i32) -> Shooter {
    Shooter {
        dx,
        dy,
        line: false,
        hp: 50,
        gate: Gate::Open,
    }
}

fn line_shooter(dx: i32, dy: i32) -> Shooter {
    Shooter {
        dx,
        dy,
        line: true,
        hp: 50,
        gate: Gate::Open,
    }
}

/// An attack held for `HELD_TICKS`; a line one lays a line zone as it winds up.
fn phases(line: bool) -> Value {
    let mut wind_up = json!({ "kind": "wind_up", "ticks": 3 });
    if line {
        wind_up["spawn"] = json!([{ "type": "zone",
            "shape": { "type": "line", "length": 100 * PX, "width": 8 * PX },
            "wind_up_ticks": 3, "active_ticks": 3 }]);
    }
    json!([wind_up, { "kind": "active", "ticks": 3 }, { "kind": "recovery", "ticks": 400 }])
}

/// A ranged rule that attacks the main character whenever it can; the long cooldown makes each
/// shooter's first attack its only one, and a refused start keeps it ready.
fn shooting(line: bool, gate: Gate) -> Value {
    let (when, target) = match gate {
        Gate::Open => (always(), json!("player")),
        Gate::After(ticks) => (
            json!({ "type": "time_above", "ticks": ticks }),
            json!("player"),
        ),
        Gate::ConditionFails => (
            json!({ "type": "hp_below", "who": "player", "pct": 0 }),
            json!("player"),
        ),
    };
    json!([{ "when": when, "target": target, "cooldown_ticks": 1000,
             "do": { "type": "attack", "ranged": true, "attack": { "inline": phases(line) } } }])
}

fn stage(shooters: &[Shooter]) -> Stage {
    stage_from((CENTRE, CENTRE), shooters)
}

/// `stage` with the main character starting at `from` px; the shooters stand `dx`, `dy` px from it.
fn stage_from(from: (i32, i32), shooters: &[Shooter]) -> Stage {
    let waves = shooters
        .iter()
        .map(|s| {
            let (x, y) = ((from.0 + s.dx) * PX, (from.1 + s.dy) * PX);
            wave(
                0,
                1,
                1,
                enemy(s.hp, hold(x, y), shooting(s.line, s.gate)),
                x,
                y,
            )
        })
        .collect();
    let mut stage = quiet(stage_with(18_000, Some(far_boss()), waves));
    stage.map = Some(Map {
        width: Fx::px(4000),
        height: Fx::px(4000),
        start: Some(Point {
            x: Fx::px(from.0),
            y: Fx::px(from.1),
        }),
        facing: None,
    });
    stage
}

fn kind_of(e: &Engine, id: u32) -> Option<PhaseKind> {
    e.snapshot()
        .entities
        .iter()
        .find(|v| v.id == EntityId(id))
        .and_then(|v| v.attack.as_ref().map(|a| a.kind))
}

/// The first tick each of `count` shooters is seen making an attack, over `ticks` ticks.
fn first_attacks(stage: &Stage, count: usize, ticks: u32) -> Vec<Option<u32>> {
    first_attacks_with(stage, count, ticks, |_| idle())
}

/// `first_attacks` with `input` pressed on each tick (the engine's tick before the step).
fn first_attacks_with(
    stage: &Stage,
    count: usize,
    ticks: u32,
    input: impl Fn(u32) -> Input,
) -> Vec<Option<u32>> {
    let mut e = engine(stage);
    let mut first = vec![None; count];
    for _ in 0..ticks {
        e.step(&[input(e.tick())]);
        for (i, f) in first.iter_mut().enumerate() {
            if f.is_none() && kind_of(&e, FIRST_SHOOTER + u32::try_from(i).unwrap()).is_some() {
                *f = Some(e.tick());
            }
        }
    }
    first
}

fn started(first: &[Option<u32>]) -> Vec<usize> {
    (0..first.len()).filter(|i| first[*i].is_some()).collect()
}

#[test]
fn the_three_nearest_of_five_zone_shooters_start() {
    // Listed farthest first, so the entity-ID order is not the distance order. The view is
    // 180 px high, so the shooters stand to the side.
    let shooters = [300, 250, 200, 150, 100].map(|d| zone_shooter(d, 0));
    let first = first_attacks(&stage(&shooters), 5, 20);
    assert_eq!(started(&first), [2, 3, 4]);
}

#[test]
fn a_tie_goes_to_the_lower_entity_id() {
    let shooters = [
        zone_shooter(-100, 0),
        zone_shooter(100, 0),
        zone_shooter(0, -100),
        zone_shooter(0, 100),
    ];
    let first = first_attacks(&stage(&shooters), 4, 20);
    assert_eq!(started(&first), [0, 1, 2]);
}

#[test]
fn an_enemy_outside_the_view_starts_nothing() {
    for (dx, dy) in [(430, 0), (-430, 0), (0, 190), (0, -190), (430, 190)] {
        let first = first_attacks(&stage(&[zone_shooter(dx, dy)]), 1, 20);
        assert_eq!(first, [None], "at ({dx}, {dy})");
    }
}

#[test]
fn an_enemy_on_the_edge_of_the_view_may_start() {
    for (dx, dy) in [(420, 0), (-420, 0), (0, 180), (0, -180), (420, -180)] {
        let first = first_attacks(&stage(&[zone_shooter(dx, dy)]), 1, 20);
        assert!(first[0].is_some(), "at ({dx}, {dy})");
    }
}

#[test]
fn near_a_map_edge_the_view_is_the_cameras_rectangle_not_one_centred_on_the_player() {
    // The main character at (100, 2000): the camera stops at the map's left edge and shows x
    // from 0 to 840, so 700 px to the right is in view and 850 px is not.
    let first = first_attacks(&stage_from((100, CENTRE), &[zone_shooter(700, 0)]), 1, 20);
    assert!(first[0].is_some());
    let first = first_attacks(&stage_from((100, CENTRE), &[zone_shooter(850, 0)]), 1, 20);
    assert_eq!(first, [None]);
}

#[test]
fn a_line_caster_fires_on_its_own_token_beside_three_zone_shooters() {
    let shooters = [
        zone_shooter(100, 0),
        zone_shooter(150, 0),
        zone_shooter(200, 0),
        line_shooter(250, 0),
    ];
    let first = first_attacks(&stage(&shooters), 4, 20);
    assert_eq!(started(&first), [0, 1, 2, 3]);
    assert_eq!(first[3], first[0]);
}

#[test]
fn only_the_nearer_of_two_line_casters_starts() {
    let shooters = [line_shooter(250, 0), line_shooter(100, 0)];
    let first = first_attacks(&stage(&shooters), 2, 20);
    assert_eq!(started(&first), [1]);
}

#[test]
fn a_freed_token_cools_for_30_ticks() {
    // Four zone shooters: the fourth waits for a token, which frees six ticks after the start
    // and then cools.
    let shooters = [100, 150, 200, 250].map(|d| zone_shooter(d, 0));
    let first = first_attacks(&stage(&shooters), 4, 80);
    let start = first[0].expect("the nearest starts");
    assert_eq!(first[..3], [Some(start); 3]);
    assert_eq!(first[3], Some(start + HELD_TICKS + COOLDOWN));
}

#[test]
fn a_run_killed_in_its_wind_up_cools_its_token_too() {
    // The nearest shooter dies of the main character's ray in its wind-up; the others fill the
    // pool, and the fourth shooter may not take the dead one's token before it has cooled.
    let victim = Shooter {
        hp: 1,
        ..zone_shooter(0, -100)
    };
    let shooters = [
        victim,
        zone_shooter(-120, -60),
        zone_shooter(-150, -80),
        zone_shooter(-200, -90),
    ];
    let mut e = engine(&armed(stage(&shooters), weapon_ref(ray())));
    let mut died = None;
    let mut fourth = None;
    for _ in 0..80 {
        let input = if died.is_none() && kind_of(&e, FIRST_SHOOTER) == Some(PhaseKind::WindUp) {
            pressing_main()
        } else {
            idle()
        };
        e.step(&[input]);
        if died.is_none() && input.main {
            died = Some(e.tick());
        }
        if fourth.is_none() && kind_of(&e, FIRST_SHOOTER + 3).is_some() {
            fourth = Some(e.tick());
        }
    }
    let died = died.expect("the nearest shooter is hit in its wind-up");
    let fourth = fourth.expect("the fourth shooter starts once a token cooled");
    assert!(
        fourth >= died + COOLDOWN,
        "the fourth started on {fourth}, the dead one's run ended by {died}"
    );
}

#[test]
fn a_shooter_still_in_its_attack_does_not_hold_a_grant_it_cannot_use() {
    // The nearest shooter's rule has no cooldown and its attack runs for 600 ticks: its rule is
    // ready but the shooter is not. The two that come five ticks later still get the two free
    // tokens.
    let long = |dx: i32, at_tick: u32| {
        let (x, y) = ((CENTRE + dx) * PX, CENTRE * PX);
        let rules = json!([{ "when": always(), "target": "player", "cooldown_ticks": 0,
            "do": { "type": "attack", "ranged": true, "attack": { "inline": [
                { "kind": "wind_up", "ticks": 3 }, { "kind": "active", "ticks": 600 }] } } }]);
        wave(at_tick, 1, 1, enemy(50, hold(x, y), rules), x, y)
    };
    let mut st = stage(&[]);
    st.waves = serde_json::from_value(json!([long(100, 0), long(150, 5), long(200, 5)])).unwrap();
    let mut e = engine(&st);
    for _ in 0..20 {
        e.step(&[idle()]);
    }
    let holding = (FIRST_SHOOTER..FIRST_SHOOTER + 3)
        .filter(|id| kind_of(&e, *id).is_some())
        .count();
    assert_eq!(holding, 3);
}

/// A shooter 32 px above the main character, whose rule waits `GATE` ticks: the smash below hits
/// it and nothing else.
const GATE: u32 = 20;

fn gated(dx: i32, dy: i32, gate: Gate) -> Shooter {
    Shooter {
        gate,
        ..zone_shooter(dx, dy)
    }
}

/// `stage` with a main character that outlives the test and whose smash, pressed on tick 2,
/// stuns for `ticks` what stands 32 px above it (a circle of 32 px, 4 px ahead).
fn stunning(mut stage: Stage, ticks: u32) -> Stage {
    stage.player.hp = 99;
    armed(
        stage,
        json!({ "inline": {
            "combo": { "smash": { "attack": { "inline": [
                { "kind": "active", "ticks": 1, "damage": 1,
                  "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } },
                { "kind": "recovery", "ticks": 4 }
            ] } } },
            "window_ticks": 30,
            "assist_range": 0,
            "smash": { "type": "stun", "ticks": ticks },
        } }),
    )
}

fn smash_on_tick_2(tick: u32) -> Input {
    Input {
        smash: tick == 2,
        ..idle()
    }
}

/// Four zone shooters that wait `GATE` ticks, the nearest at 32 px above the main character.
fn waiting_four() -> [Shooter; 4] {
    [
        gated(0, -32, Gate::After(GATE)),
        gated(150, 0, Gate::After(GATE)),
        gated(200, 0, Gate::After(GATE)),
        gated(250, 0, Gate::After(GATE)),
    ]
}

#[test]
fn a_stunned_shooter_takes_no_grant_from_the_shooters_that_can_fire() {
    let st = stunning(stage(&waiting_four()), 100);
    let first = first_attacks_with(&st, 4, 45, smash_on_tick_2);
    assert_eq!(first[0], None, "the stunned shooter never attacks");
    assert_eq!(started(&first), [1, 2, 3]);
}

#[test]
fn a_stunned_line_caster_does_not_hold_the_line_token() {
    let caster = |dx, dy| Shooter {
        gate: Gate::After(GATE),
        ..line_shooter(dx, dy)
    };
    let st = stunning(stage(&[caster(0, -32), caster(250, 0)]), 100);
    let first = first_attacks_with(&st, 2, 45, smash_on_tick_2);
    assert_eq!(started(&first), [1]);
}

#[test]
fn a_shooter_whose_stun_ran_out_takes_its_grant_again() {
    let st = stunning(stage(&waiting_four()), 10);
    let first = first_attacks_with(&st, 4, 40, smash_on_tick_2);
    assert_eq!(started(&first), [0, 1, 2]);
}

#[test]
fn a_shooter_whose_condition_fails_takes_no_grant() {
    let shooters = [
        gated(100, 0, Gate::ConditionFails),
        gated(150, 0, Gate::ConditionFails),
        zone_shooter(200, 0),
        zone_shooter(250, 0),
        zone_shooter(300, 0),
    ];
    let first = first_attacks(&stage(&shooters), 5, 20);
    assert_eq!(started(&first), [2, 3, 4]);
}

#[test]
fn the_boss_is_not_limited_by_the_tokens() {
    let mut boss = far_boss();
    boss["phases"] = json!([{ "rules": [{ "when": always(), "target": "player",
        "cooldown_ticks": 1000,
        "do": { "type": "attack", "attack": { "inline": phases(false) } } }] }]);
    let shooters = [100, 150, 200, 250].map(|d| zone_shooter(d, 0));
    let mut st = stage(&shooters);
    st.boss = Some(serde_json::from_value(boss).expect("the boss parses"));
    let mut e = engine(&st);
    e.step(&[idle()]);
    e.step(&[idle()]);
    assert!(kind_of(&e, 1).is_some(), "the boss attacks");
}

#[test]
fn the_same_seed_and_inputs_give_the_same_hash_every_tick() {
    let shooters = [100, 150, 200, 250, 300].map(|d| zone_shooter(d, 0));
    let st = stage(&shooters);
    let run = || {
        let mut e = engine(&st);
        (0..120)
            .map(|_| {
                e.step(&[idle()]);
                e.state_hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
