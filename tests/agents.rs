//! Agents (ADR-036 tier 1): rule lists, selectors, conditions, statuses, aggro, movement,
//! summons and the skills they cast. Small stages whose every tick can be
//! worked out by hand; the expected numbers come from the constants in `rules_v0`.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EntityKind;

// --- Builders --------------------------------------------------------------------------------

fn id(n: u32) -> EntityId {
    EntityId(n)
}

/// Runs `ticks` ticks with `input(tick)`, collecting `(tick, event)`.
fn play(e: &mut Engine, ticks: u32, input: impl Fn(u32) -> Input) -> Vec<(u32, DomainEvent)> {
    let mut seen = Vec::new();
    for _ in 0..ticks {
        e.step(input(e.tick() + 1));
        seen.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
    }
    seen
}

fn rule_firings_of(events: &[(u32, DomainEvent)], agent: u32) -> Vec<(u32, u8, Option<EntityId>)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::RuleFired {
                agent: a,
                rule,
                target,
            } if *a == id(agent) => Some((*t, *rule, *target)),
            _ => None,
        })
        .collect()
}

/// The entity the first firing of `agent` targeted.
fn first_target(
    stage: &stella_rain_core::stage::Stage,
    agent: u32,
    ticks: u32,
) -> Option<EntityId> {
    let mut e = engine(stage);
    let events = play(&mut e, ticks, |_| idle());
    rule_firings_of(&events, agent).first().and_then(|f| f.2)
}

/// A boss that cannot be reached, then `companions`: boss is entity 1, companions 2, 3, ...
fn with_companions(companions: Vec<Value>, waves: Vec<Value>) -> stella_rain_core::stage::Stage {
    stage_full(18_000, Some(far_boss()), waves, companions)
}

fn probe(selector: Value) -> Value {
    rule(always(), selector, telegraph(), 0)
}

// --- Selectors (ADR-036) ---------------------------------------------------------------------

/// Companions A (2), B (3), C (4) and the probe P (5) stand at fixed points; the player is at
/// (46080, 147456). From P at (46080, 120000): A is 20000 away, the player 27456, B about
/// 27734, C about 32867.
fn crowd(probe_target: Value, hps: [u32; 3]) -> stella_rain_core::stage::Stage {
    with_companions(
        vec![
            companion(hps[0], hold(46_080, 140_000), json!([])),
            companion(hps[1], hold(50_000, 147_456), json!([])),
            companion(hps[2], hold(20_000, 100_000), json!([])),
            companion(50, hold(46_080, 120_000), json!([probe(probe_target)])),
        ],
        vec![],
    )
}

#[test]
fn self_and_player_select_themselves_and_the_main_character() {
    assert_eq!(
        first_target(&crowd(json!("self"), [50; 3]), 5, 3),
        Some(id(5))
    );
    assert_eq!(
        first_target(&crowd(json!("player"), [50; 3]), 5, 3),
        Some(id(0))
    );
}

#[test]
fn nearest_ally_is_never_the_agent_itself() {
    // A is 20000 away and the closest of the others; P's own distance is 0.
    assert_eq!(
        first_target(&crowd(json!("nearest_ally"), [50; 3]), 5, 3),
        Some(id(2))
    );
}

#[test]
fn lowest_hp_ally_counts_the_agent_and_the_main_character() {
    // Hit points: main character 99, A, B, C, P 50.
    assert_eq!(
        first_target(&crowd(json!("lowest_hp_ally"), [30, 7, 9]), 5, 3),
        Some(id(3))
    );
    // P itself is the lowest.
    assert_eq!(
        first_target(&crowd(json!("lowest_hp_ally"), [60, 70, 80]), 5, 3),
        Some(id(5))
    );
}

#[test]
fn equal_hit_points_break_ties_by_distance_then_entity_id() {
    // A, B and C all have 7 hp, the lowest: A is nearest to P.
    assert_eq!(
        first_target(&crowd(json!("lowest_hp_ally"), [7, 7, 7]), 5, 3),
        Some(id(2))
    );
    // Two allies at the same distance: the lower ID wins.
    let stage = with_companions(
        vec![
            companion(7, hold(40_000, 120_000), json!([])),
            companion(7, hold(52_000, 120_000), json!([])),
            companion(
                50,
                hold(46_000, 120_000),
                json!([probe(json!("lowest_hp_ally"))]),
            ),
        ],
        vec![],
    );
    assert_eq!(first_target(&stage, 4, 3), Some(id(2)));
}

#[test]
fn a_tie_goes_to_the_nearer_ally_even_when_it_has_the_higher_id() {
    // X (2) is far, Y (3) is near, both at 7 hp: the probe Z (4) picks Y, not the lower ID.
    let stage = with_companions(
        vec![
            companion(7, hold(20_000, 100_000), json!([])),
            companion(7, hold(46_000, 119_000), json!([])),
            companion(
                50,
                hold(46_080, 120_000),
                json!([probe(json!("lowest_hp_ally"))]),
            ),
        ],
        vec![],
    );
    assert_eq!(first_target(&stage, 4, 3), Some(id(3)));
    // The same for aggro: nobody has any, so the nearer one leads.
    let stage = with_companions(
        vec![
            companion(7, hold(20_000, 100_000), json!([])),
            companion(7, hold(46_000, 119_000), json!([])),
            companion(
                50,
                hold(46_080, 125_000),
                json!([probe(json!("highest_aggro_ally"))]),
            ),
        ],
        vec![],
    );
    // Z itself is nearest (distance 0), then Y, then X.
    assert_eq!(first_target(&stage, 4, 3), Some(id(4)));
}

#[test]
fn highest_aggro_ally_falls_back_to_the_nearest_when_nobody_has_any() {
    // Nobody has aggro yet: the nearest ally is P itself.
    assert_eq!(
        first_target(&crowd(json!("highest_aggro_ally"), [50; 3]), 5, 1),
        Some(id(5))
    );
}

#[test]
fn highest_aggro_ally_follows_the_aggro() {
    // C buffs itself on tick 1: +40 aggro, fading by 1 a tick, so it leads from then on.
    let mut companions = vec![
        companion(50, hold(46_080, 140_000), json!([])),
        companion(50, hold(50_000, 147_456), json!([])),
        companion(
            50,
            hold(20_000, 100_000),
            json!([rule(always(), json!("self"), atk_up(10, 50), 1000)]),
        ),
        companion(
            50,
            hold(46_080, 120_000),
            json!([probe(json!("highest_aggro_ally"))]),
        ),
    ];
    // P acts after C within a tick, so it sees C's aggro on tick 1 already.
    let stage = with_companions(std::mem::take(&mut companions), vec![]);
    assert_eq!(first_target(&stage, 5, 1), Some(id(4)));
}

#[test]
fn nearest_enemy_picks_the_closest_and_breaks_ties_by_entity_id() {
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![
            wave(
                1,
                1,
                0,
                enemy(50, hold(30_000, 100_000), json!([])),
                30_000,
                100_000,
            ),
            wave(
                1,
                1,
                0,
                enemy(50, hold(60_000, 100_000), json!([])),
                60_000,
                100_000,
            ),
            wave(
                1,
                1,
                0,
                enemy(50, hold(46_080, 100_000), json!([])),
                46_080,
                100_000,
            ),
        ],
        vec![companion(
            50,
            hold(46_080, 90_000),
            json!([probe(json!("nearest_enemy"))]),
        )],
    );
    // Spawned on tick 1, so enemies 3, 4, 5. From (46080, 90000) the third is nearest.
    assert_eq!(first_target(&stage, 2, 3), Some(id(5)));
    // Two at the same distance: the one spawned first (lower ID).
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![
            wave(
                1,
                1,
                0,
                enemy(50, hold(40_000, 100_000), json!([])),
                40_000,
                100_000,
            ),
            wave(
                1,
                1,
                0,
                enemy(50, hold(52_160, 100_000), json!([])),
                52_160,
                100_000,
            ),
        ],
        vec![companion(
            50,
            hold(46_080, 90_000),
            json!([probe(json!("nearest_enemy"))]),
        )],
    );
    assert_eq!(first_target(&stage, 2, 3), Some(id(3)));
}

#[test]
fn boss_first_prefers_the_boss_and_otherwise_the_nearest_enemy() {
    let probe_rule = || json!([probe(json!("boss_first"))]);
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(
            1,
            1,
            0,
            enemy(50, hold(46_080, 100_000), json!([])),
            46_080,
            100_000,
        )],
        vec![companion(50, hold(46_080, 90_000), probe_rule())],
    );
    assert_eq!(
        first_target(&stage, 2, 3),
        Some(id(1)),
        "the boss, though an enemy is closer"
    );
    let stage = stage_full(
        18_000,
        None,
        vec![wave(
            1,
            1,
            0,
            enemy(50, hold(46_080, 100_000), json!([])),
            46_080,
            100_000,
        )],
        vec![companion(50, hold(46_080, 90_000), probe_rule())],
    );
    // No boss: the enemy, spawned on tick 1 after the companion (entity 1).
    assert_eq!(first_target(&stage, 1, 3), Some(id(2)));
}

#[test]
fn a_selector_that_finds_nobody_does_not_fire() {
    // No enemies at all.
    let stage = stage_full(
        18_000,
        None,
        vec![wave(50, 1, 0, enemy(50, hold(0, 0), json!([])), 0, 0)],
        vec![companion(
            50,
            hold(46_080, 90_000),
            json!([probe(json!("nearest_enemy"))]),
        )],
    );
    let mut e = engine(&stage);
    let events = play(&mut e, 40, |_| idle());
    assert_eq!(rule_firings_of(&events, 1), []);
}

#[test]
fn rules_run_companions_first_then_enemies_in_entity_id_order() {
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![
            wave(
                1,
                1,
                0,
                enemy(50, hold(0, 0), json!([attack_rule(always(), 1000)])),
                0,
                0,
            ),
            wave(
                1,
                1,
                0,
                enemy(50, hold(0, 0), json!([attack_rule(always(), 1000)])),
                0,
                0,
            ),
        ],
        vec![
            companion(50, hold(46_080, 90_000), json!([probe(json!("player"))])),
            companion(50, hold(46_080, 91_000), json!([probe(json!("player"))])),
        ],
    );
    let mut e = engine(&stage);
    e.step(idle());
    let order: Vec<EntityId> = e
        .events()
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::RuleFired { agent, .. } => Some(*agent),
            _ => None,
        })
        .collect();
    assert_eq!(order, [id(2), id(3), id(4), id(5)]);
}

// --- Conditions ------------------------------------------------------------------------------

#[test]
fn hp_below_is_strictly_below_a_percent_of_the_maximum() {
    // An enemy shoots the companion K (1,000 hp) once per tick for the default 100; K reacts when
    // hit points are below 100% (the first hit) and below 50% (after the sixth hit, at 400 hp).
    let shooter = enemy(
        50,
        hold(46_080, 100_000),
        json!([rule(
            always(),
            json!("nearest_ally"),
            json!({
        "type": "attack", "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
            0
        )]),
    );
    let k_rules = json!([
        rule(
            json!({ "type": "hp_below", "who": "self", "pct": 100 }),
            json!("self"),
            telegraph(),
            1000
        ),
        rule(
            json!({ "type": "hp_below", "who": "self", "pct": 50 }),
            json!("self"),
            telegraph(),
            1000
        ),
    ]);
    // Far from the main character, so K is the shooter's nearest ally.
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![companion(1000, hold(46_080, 130_000), k_rules)],
    );
    let mut e = engine(&stage);
    let events = play(&mut e, 120, |_| idle());
    let hits: Vec<u32> = events
        .iter()
        .filter_map(|(t, ev)| {
            matches!(ev, DomainEvent::Hit { target, .. } if *target == id(2)).then_some(*t)
        })
        .collect();
    assert!(hits.len() >= 6, "{hits:?}");
    let fired = rule_firings_of(&events, 2);
    // Rule 0 fires the tick after the first hit (900 of 1,000 hp), rule 1 the tick after the
    // sixth (400 of 1,000 hp; 500 is not below 50%).
    assert_eq!(
        fired.iter().map(|f| (f.0, f.1)).collect::<Vec<_>>(),
        [(hits[0] + 1, 0), (hits[5] + 1, 1)]
    );
}

#[test]
fn has_status_sees_a_status_until_it_runs_out() {
    let rules = json!([
        {
            "when": always(), "target": "self", "do": atk_up(10, 5), "once": true
        },
        rule(json!({ "type": "has_status", "who": "self", "status": "atk_up" }), json!("self"), telegraph(), 0),
    ]);
    let stage = with_companions(vec![companion(50, hold(46_080, 90_000), rules)], vec![]);
    let mut e = engine(&stage);
    let events = play(&mut e, 12, |_| idle());
    // Applied on tick 1 with 5 ticks left; it ticks down at the start of each tick and is gone
    // at the start of tick 6, so the second rule sees it on ticks 2 to 5.
    let ticks: Vec<u32> = rule_firings_of(&events, 2)
        .iter()
        .filter(|f| f.1 == 1)
        .map(|f| f.0)
        .collect();
    assert_eq!(ticks, [2, 3, 4, 5]);
}

#[test]
fn aggro_above_compares_with_the_aggro_that_fades() {
    // A buff gives +4,000 on tick 1 (40 before the scale of ADR-103) and the aggro fades by 100
    // a tick; it is 4,000 - 100 (n - 1) on tick n, so it is above 3,000 until tick 10.
    let rules = json!([
        rule(always(), json!("self"), atk_up(10, 5), 1000),
        rule(
            json!({ "type": "aggro_above", "who": "self", "value": 3000 }),
            json!("self"),
            telegraph(),
            0
        ),
    ]);
    let stage = with_companions(vec![companion(50, hold(46_080, 90_000), rules)], vec![]);
    let mut e = engine(&stage);
    let events = play(&mut e, 20, |_| idle());
    let ticks: Vec<u32> = rule_firings_of(&events, 2)
        .iter()
        .filter(|f| f.1 == 1)
        .map(|f| f.0)
        .collect();
    assert_eq!(ticks, (2..=10).collect::<Vec<u32>>());
}

// --- Movement --------------------------------------------------------------------------------

fn companion_at(e: &Engine, n: u32) -> (i32, i32) {
    let s = e.snapshot();
    let c = s
        .entities
        .iter()
        .find(|x| x.id == id(n))
        .expect("the companion is there");
    (c.at.x.0, c.at.y.0)
}

#[test]
fn an_orbiting_companion_circles_the_main_character() {
    let orbit = |r: i32| json!({ "type": "orbit", "radius": r });
    let stage = with_companions(
        vec![
            companion(50, orbit(10_240), json!([])),
            companion(50, orbit(10_240), json!([])),
        ],
        vec![],
    );
    let mut e = engine(&stage);
    // Within a unit or two of the exact values: the table is interpolated and the radius is
    // 10,240 units, so one table unit of 1/65536 is 0.16 of a unit here.
    let near = |at: (i32, i32), dx: i32, dy: i32| {
        let (x, y) = (at.0 - START_X, at.1 - START_Y);
        assert!(
            (x - dx).abs() <= 2 && (y - dy).abs() <= 2,
            "{x},{y} is not near {dx},{dy}"
        );
    };
    e.step(idle());
    // 1.5 degrees on tick 1: 10240 * sin = 268.05, 10240 * cos = 10236.49. The second
    // companion starts a quarter turn on, at 91.5 degrees.
    near(companion_at(&e, 2), 10_236, 268);
    near(companion_at(&e, 3), -268, 10_236);
    for _ in 0..3 {
        e.step(idle());
    }
    // 6 degrees on tick 4: 10240 * sin = 1070.37, 10240 * cos = 10183.90.
    near(companion_at(&e, 2), 10_184, 1_070);
}

#[test]
fn an_orbit_stays_on_its_circle_and_follows_the_main_character() {
    let stage = with_companions(
        vec![companion(
            50,
            json!({ "type": "orbit", "radius": 10_240 }),
            json!([]),
        )],
        vec![],
    );
    let mut e = engine(&stage);
    for t in 0..400 {
        // 30 ticks of 768 units keep the player, and the circle round it, inside the field.
        e.step(if t < 30 { moving(-127, 0) } else { idle() });
        let (px, py) = player_at(&e);
        let (cx, cy) = companion_at(&e, 2);
        let d2 = i64::from(cx - px).pow(2) + i64::from(cy - py).pow(2);
        let r2 = 10_240i64.pow(2);
        // The table has whole degrees, so the radius is right to within a little.
        assert!((d2 - r2).abs() < r2 / 100, "tick {t}: {d2} vs {r2}");
    }
}

fn enemy_at(e: &Engine) -> (i32, i32) {
    let s = e.snapshot();
    let en = s
        .entities
        .iter()
        .find(|en| en.kind == EntityKind::Enemy)
        .expect("the enemy is there");
    (en.at.x.0, en.at.y.0)
}

/// One enemy of `movement`, spawned on tick 1 at `(x, y)`.
fn one_enemy(movement: Value, x: i32, y: i32) -> Engine {
    let waves = vec![wave(1, 1, 0, enemy(50, movement, json!([])), x, y)];
    engine(&quiet(with_companions(vec![], waves)))
}

#[test]
fn a_hold_enemy_walks_to_its_point_four_pixels_a_tick() {
    // 40 pixels straight below its spawn: ten ticks of 4 pixels (1,024 units), then it stays.
    let mut e = one_enemy(hold(46_080, 60_240), 46_080, 50_000);
    for tick in 1..=10 {
        e.step(idle());
        assert_eq!(enemy_at(&e), (46_080, 50_000 + 1_024 * tick), "tick {tick}");
    }
    for _ in 0..5 {
        e.step(idle());
        assert_eq!(enemy_at(&e), (46_080, 60_240));
    }
}

#[test]
fn a_hold_enemy_lands_exactly_on_its_point() {
    // 3,000 and 4,000 units away: 5,000 along the line, so four steps of 1,024 and one of 904.
    let mut e = one_enemy(hold(43_000, 54_000), 40_000, 50_000);
    e.step(idle());
    // 1,024 * 3 / 5 = 614.4 and 1,024 * 4 / 5 = 819.2, rounded toward zero.
    assert_eq!(enemy_at(&e), (40_614, 50_819));
    for _ in 0..4 {
        e.step(idle());
    }
    assert_eq!(enemy_at(&e), (43_000, 54_000));
}

#[test]
fn a_hold_companion_is_still_set_on_its_point() {
    let stage = with_companions(
        vec![companion(50, hold(20_000, 100_000), json!([]))],
        vec![],
    );
    let mut e = engine(&stage);
    e.step(idle());
    assert_eq!(companion_at(&e, 2), (20_000, 100_000));
}

#[test]
fn an_orbiting_enemy_moves_to_its_circle_and_then_keeps_it() {
    // It spawns 200 pixels above the main character; its circle is 40 pixels round it.
    let radius = 10_240;
    let mut e = one_enemy(
        json!({ "type": "orbit", "radius": radius }),
        START_X,
        START_Y - 51_200,
    );
    // The orbit step: 1.5 degrees of a 10,240 circle is a chord of 268 units; plus 4 pixels.
    let most = 268 + 1_024 + 2;
    let mut was = (START_X, START_Y - 51_200);
    let mut arrived = None;
    for tick in 1..=120 {
        e.step(idle());
        let at = enemy_at(&e);
        let moved = i64::from(at.0 - was.0).pow(2) + i64::from(at.1 - was.1).pow(2);
        assert!(
            moved <= i64::from(most).pow(2),
            "tick {tick}: {was:?} to {at:?}"
        );
        was = at;
        let (px, py) = player_at(&e);
        let d2 = i64::from(at.0 - px).pow(2) + i64::from(at.1 - py).pow(2);
        let r2 = i64::from(radius).pow(2);
        let on_circle = (d2 - r2).abs() < r2 / 100;
        if arrived.is_some() {
            assert!(on_circle, "tick {tick}: it left its circle");
        } else if on_circle {
            arrived = Some(tick);
        }
    }
    // 160 pixels to close at a little under 4 pixels a tick more than the circle turns.
    let arrived = arrived.expect("it reaches its circle");
    assert!((35..=60).contains(&arrived), "it arrived on tick {arrived}");
}

#[test]
fn a_following_companion_closes_to_its_distance_and_stops() {
    let stage = with_companions(
        vec![companion(
            50,
            json!({ "type": "follow", "distance": 5_120 }),
            json!([]),
        )],
        vec![],
    );
    let mut e = engine(&stage);
    for _ in 0..30 {
        e.step(moving(-127, 0));
    }
    for _ in 0..200 {
        e.step(idle());
    }
    let (px, py) = player_at(&e);
    let (cx, cy) = companion_at(&e, 2);
    // 5,120 units away, give or take a unit or two of rounding (squared, to stay in integers).
    let d2 = i64::from(cx - px).pow(2) + i64::from(cy - py).pow(2);
    assert!(
        (5_118i64.pow(2)..=5_122i64.pow(2)).contains(&d2),
        "settled with d^2 = {d2}"
    );
}

#[test]
fn a_holding_companion_stands_at_its_point() {
    let stage = with_companions(
        vec![companion(50, hold(30_000, 100_000), json!([]))],
        vec![],
    );
    let mut e = engine(&stage);
    e.step(idle());
    assert_eq!(companion_at(&e, 2), (30_000, 100_000));
}

#[test]
fn a_companion_cannot_leave_the_playfield() {
    let stage = with_companions(
        vec![companion(
            50,
            json!({ "type": "straight", "vx": 5_000, "vy": 0 }),
            json!([]),
        )],
        vec![],
    );
    let mut e = engine(&stage);
    for _ in 0..100 {
        e.step(idle());
    }
    assert_eq!(companion_at(&e, 2).0, 92_160);
}

#[test]
fn move_to_walks_in_equal_steps_and_lands_exactly() {
    let to = json!({ "x": 50_000, "y": 140_000 });
    let rules = json!([{ "when": always(), "target": "self", "once": true,
        "do": { "type": "move_to", "to": to, "ticks": 10 } }]);
    let stage = with_companions(vec![companion(50, hold(1, 1), rules)], vec![]);
    // The rule fires on tick 1 (the order starts), ticks 2 to 11 carry it out. Hold would pull
    // it back, so the order replaces the movement while it lasts.
    let mut e = engine(&stage);
    e.step(idle());
    assert_eq!(companion_at(&e, 2), (1, 1));
    let mut positions = Vec::new();
    for _ in 0..10 {
        e.step(idle());
        positions.push(companion_at(&e, 2));
    }
    assert_eq!(*positions.last().unwrap(), (50_000, 140_000));
    assert!(positions[..9].iter().all(|p| *p != (50_000, 140_000)));
    assert!(
        positions.windows(2).all(|w| w[0].0 < w[1].0),
        "closing in on x: {positions:?}"
    );
    // Back to holding afterwards.
    e.step(idle());
    assert_eq!(companion_at(&e, 2), (1, 1));
}

// --- Statuses --------------------------------------------------------------------------------

fn hit_damages(events: &[(u32, DomainEvent)], target: u32) -> Vec<u32> {
    events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Hit {
                target: t, damage, ..
            } if *t == id(target) => Some(*damage),
            _ => None,
        })
        .collect()
}

#[test]
fn atk_up_raises_the_damage_of_shots_fired_after_it() {
    // The boss (1) stands in the line of fire. A companion buffs the main character by 50% on
    // tick 1, after the first shot was fired (the main character acts first): 10, then 15s.
    let rules =
        json!([{ "when": always(), "target": "player", "do": atk_up(50, 1000), "once": true }]);
    let stage = stage_full(
        18_000,
        Some(boss_at(1_000, 25_600)),
        vec![],
        vec![companion(50, hold(0, 0), rules)],
    );
    let mut e = engine(&stage);
    let events = play(&mut e, 90, |t| firing(idle(), t - 1));
    let damages = hit_damages(&events, 1);
    assert_eq!(&damages[..4], [10, 15, 15, 15], "{damages:?}");
}

#[test]
fn applying_a_status_again_adds_a_stack_up_to_five_and_refreshes_it() {
    let rules = json!([rule(always(), json!("player"), atk_up(10, 30), 5)]);
    let stage = with_companions(vec![companion(50, hold(0, 0), rules)], vec![]);
    let mut e = engine(&stage);
    let events = play(&mut e, 45, |_| idle());
    let stacks: Vec<(u32, u16)> = events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::StatusApplied {
                target,
                status,
                stacks,
                duration_ticks,
            } if *target == id(0) && status.0 == "atk_up" && *duration_ticks == 30 => {
                Some((*t, *stacks))
            }
            _ => None,
        })
        .collect();
    // Every fifth tick from tick 1; five stacks at most.
    let expected: Vec<(u32, u16)> = [1u32, 6, 11, 16, 21, 26, 31, 36, 41]
        .into_iter()
        .zip([1, 2, 3, 4, 5, 5, 5, 5, 5])
        .collect();
    assert_eq!(stacks, expected);
}

#[test]
fn vulnerable_raises_the_damage_a_target_takes() {
    let hits = |rules: Value| {
        let stage = stage_full(
            18_000,
            Some(boss_at(5_000, 25_600)),
            vec![],
            vec![companion(50, hold(0, 0), rules)],
        );
        let mut e = engine(&stage);
        hit_damages(&play(&mut e, 120, |t| firing(idle(), t - 1)), 1)
    };
    let plain = hits(json!([]));
    let vulnerable = hits(
        json!([{ "when": always(), "target": "boss_first", "once": true,
        "do": cast(json!({ "type": "vulnerable", "value_pct": 100, "duration_ticks": 1000 })) }]),
    );
    assert!(
        plain.len() >= 2 && plain.iter().all(|d| *d == 10),
        "{plain:?}"
    );
    assert_eq!(vulnerable[..2], [20, 20], "{vulnerable:?}");
}

#[test]
fn slow_cuts_the_speed_of_the_main_character_and_of_enemies() {
    let slow = |pct: u32| cast(json!({ "type": "slow", "value_pct": pct, "duration_ticks": 1000 }));
    let rules = json!([{ "when": always(), "target": "player", "do": slow(50), "once": true }]);
    let mut e = engine(&stage_full(
        18_000,
        Some(far_boss()),
        vec![],
        vec![companion(50, hold(0, 0), rules)],
    ));
    e.step(moving(127, 0));
    assert_eq!(player_at(&e).0, START_X + SPEED);
    e.step(moving(127, 0));
    // Half speed from the tick after the cast.
    assert_eq!(player_at(&e).0, START_X + SPEED + SPEED / 2);

    // Stacks add up but never past 90%: one tenth of the speed is left.
    let rules = json!([rule(always(), json!("player"), slow(100), 0)]);
    let mut e = engine(&stage_full(
        18_000,
        Some(far_boss()),
        vec![],
        vec![companion(50, hold(0, 0), rules)],
    ));
    for _ in 0..20 {
        e.step(moving(127, 0));
    }
    let before = player_at(&e).0;
    e.step(moving(127, 0));
    assert_eq!(
        player_at(&e).0 - before,
        76,
        "10% of 768, truncated to whole units"
    );

    // An enemy running down at 2048 a tick, slowed by half, runs at 1024.
    let runner = enemy(
        50,
        json!({ "type": "straight", "vx": 0, "vy": 2048 }),
        json!([]),
    );
    let rules =
        json!([{ "when": always(), "target": "nearest_enemy", "do": slow(50), "once": true }]);
    let mut e = engine(&stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, runner, START_X, 0)],
        vec![companion(50, hold(46_080, 90_000), rules)],
    ));
    e.step(idle());
    let y1 = e
        .snapshot()
        .entities
        .iter()
        .find(|x| x.kind == EntityKind::Enemy)
        .unwrap()
        .at
        .y
        .0;
    e.step(idle());
    let y2 = e
        .snapshot()
        .entities
        .iter()
        .find(|x| x.kind == EntityKind::Enemy)
        .unwrap()
        .at
        .y
        .0;
    // Companions act before enemies, so the enemy is slowed on tick 1 already (the boss at
    // (0, 0) is farther from the companion than the enemy).
    assert_eq!((y1, y2 - y1), (1024, 1024));
}

#[test]
fn heals_stop_at_the_maximum_and_say_how_much_they_gave() {
    // An enemy hits the companion K (1,000 hp) once a tick for the default 100; the healer H (far
    // away) heals the lowest-hp ally by 500 whenever someone is hurt. K is hit, then healed back
    // by exactly the 100 hit points it lacks, every tick.
    let shooter = enemy(
        50,
        hold(46_080, 100_000),
        json!([rule(
            always(),
            json!("nearest_ally"),
            json!({
        "type": "attack", "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
            0
        )]),
    );
    let healer = json!([rule(
        json!({ "type": "hp_below", "who": "lowest_hp_ally", "pct": 100 }),
        json!("lowest_hp_ally"),
        cast(json!({ "type": "heal", "amount": 500 })),
        0
    )]);
    // The weapons are quiet, so that the shooter lives.
    let mut stage = quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![
            companion(1000, hold(46_080, 130_000), json!([])),
            companion(1000, hold(80_000, 147_456), healer),
        ],
    ));
    // The main character is an ally too: its 99 hit points, scaled, outlast the shooter's hits.
    stage.player.hp = 9_900;
    let mut e = engine(&stage);
    let events = play(&mut e, 150, |_| idle());
    let healed: Vec<(EntityId, u32)> = events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Healed { target, amount } => Some((*target, *amount)),
            _ => None,
        })
        .collect();
    assert!(healed.len() > 50, "{} heals", healed.len());
    assert!(
        healed.iter().all(|h| *h == (id(2), 100)),
        "every heal restores the 100 points missing"
    );
    assert!(
        e.snapshot().entities.iter().any(|x| x.id == id(2)),
        "K is still alive"
    );
}

// --- Aggro and companions in the line of fire ------------------------------------------------

#[test]
fn damage_dealt_raises_aggro_and_enemies_turn_on_the_ally_with_the_most() {
    // E1 stands in the main character's line of fire; E2, off to the side, shoots the ally
    // with the highest aggro. Before anyone has aggro that is the nearest ally, the companion K;
    // once the main character's shots land (the first on tick 59) that is the main character.
    let target = enemy(5_000, hold(START_X, 25_600), json!([]));
    let shooter = enemy(
        50,
        hold(70_000, 100_000),
        json!([rule(
            always(),
            json!("highest_aggro_ally"),
            json!({ "type": "attack", "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
            10,
        )]),
    );
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![
            wave(1, 1, 0, target, START_X, 25_600),
            wave(1, 1, 0, shooter, 70_000, 100_000),
        ],
        vec![companion(50, hold(60_000, 100_000), json!([]))],
    );
    let mut e = engine(&stage);
    let events = play(&mut e, 100, |t| firing(idle(), t - 1));
    // Entities: boss 1, K 2, target 3, shooter 4.
    let aimed_at: Vec<(u32, Option<EntityId>)> = rule_firings_of(&events, 4)
        .iter()
        .map(|f| (f.0, f.2))
        .collect();
    assert_eq!(aimed_at[0], (1, Some(id(2))), "{aimed_at:?}");
    assert!(
        aimed_at
            .iter()
            .any(|(t, who)| *t > 60 && *who == Some(id(0))),
        "after the shots land: {aimed_at:?}"
    );
}

#[test]
fn a_companion_that_is_hit_enough_dies_and_stops_being_selected() {
    let shooter = enemy(
        50,
        hold(46_080, 100_000),
        json!([rule(
            always(),
            json!("nearest_ally"),
            json!({
        "type": "attack", "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
            0
        )]),
    );
    let probe_rule = json!([probe(json!("nearest_ally"))]);
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![
            companion(3, hold(46_080, 130_000), json!([])),
            companion(50, hold(80_000, 147_456), probe_rule),
        ],
    );
    let mut e = engine(&stage);
    let events = play(&mut e, 120, |_| idle());
    let died: Vec<u32> = events
        .iter()
        .filter_map(|(t, ev)| {
            matches!(ev, DomainEvent::Died { entity } if *entity == id(2)).then_some(*t)
        })
        .collect();
    assert_eq!(died.len(), 1);
    assert!(e.snapshot().entities.iter().all(|x| x.id != id(2)));
    // The probe (3) first sees K (2) as its nearest ally, or the main character: after K's
    // death nothing it picks is K.
    for (t, _, target) in rule_firings_of(&events, 3) {
        if t > died[0] {
            assert_ne!(target, Some(id(2)), "tick {t}");
        }
    }
}

// --- Summons ---------------------------------------------------------------------------------

#[test]
fn an_enemy_summons_more_enemies_where_it_stands() {
    let summoner = enemy(
        50,
        hold(46_080, 50_000),
        json!([{ "when": always(), "target": "player", "do": summon(2), "once": true }]),
    );
    // Quiet weapons: the summoner stands straight above the main character.
    let mut e = engine(&quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, summoner, 46_080, 50_000)],
        vec![],
    )));
    e.step(idle());
    let s = e.snapshot();
    let ids: Vec<u32> = s.entities.iter().map(|x| x.id.0).collect();
    assert_eq!(
        ids,
        [1, 2, 3, 4],
        "boss, the summoner, then the two summoned"
    );
    // They appear where the summoner stands, and the three are pushed apart on that tick, at
    // most 4 pixels (1,024 units) each.
    let from = |v: &stella_rain_core::snapshot::EntityView, x: i32, y: i32| {
        (i64::from(v.at.x.0 - x).pow(2) + i64::from(v.at.y.0 - y).pow(2)).isqrt()
    };
    for v in &s.entities[1..] {
        assert!(from(v, 46_080, 50_000) <= 1_024, "{v:?}");
    }
    // The summoned start acting next tick: they walk to their point at (1000, 1000), 66,582
    // units away, at 4 pixels a tick, and keep to it side by side.
    for _ in 0..150 {
        e.step(idle());
    }
    let s = e.snapshot();
    for v in &s.entities[2..] {
        assert!(from(v, 1_000, 1_000) <= 4_096, "{v:?}");
    }
    assert!(from(&s.entities[2], s.entities[3].at.x.0, s.entities[3].at.y.0) >= 3_840);
}

#[test]
fn summons_respect_the_enemy_cap() {
    // 200 enemies on tick 1, one of which summons 8 more; later waves fill the rest. Never
    // more than 500 alive.
    let mut waves: Vec<Value> = (0..200)
        .map(|_| {
            wave(
                1,
                100,
                1,
                enemy(50, hold(46_080, 50_000), json!([])),
                46_080,
                50_000,
            )
        })
        .collect();
    waves[0] = wave(
        1,
        100,
        1,
        enemy(
            50,
            hold(46_080, 50_000),
            json!([{ "when": always(), "target": "player", "do": summon(8), "once": true }]),
        ),
        46_080,
        50_000,
    );
    let mut e = engine(&stage_full(18_000, Some(far_boss()), waves, vec![]));
    e.step(idle());
    let alive = |e: &Engine| {
        e.snapshot()
            .entities
            .iter()
            .filter(|x| x.kind == EntityKind::Enemy)
            .count()
    };
    assert_eq!(alive(&e), 208);
    for _ in 0..5 {
        e.step(idle());
        assert!(alive(&e) <= 500);
    }
    assert_eq!(alive(&e), 500);
}

#[test]
fn a_companion_cannot_summon_and_the_next_rule_gets_its_turn() {
    let rules = json!([
        { "when": always(), "target": "player", "do": summon(2) },
        probe(json!("player")),
    ]);
    let mut e = engine(&with_companions(
        vec![companion(50, hold(46_080, 90_000), rules)],
        vec![],
    ));
    let events = play(&mut e, 3, |_| idle());
    assert_eq!(
        rule_firings_of(&events, 2)
            .iter()
            .map(|f| f.1)
            .collect::<Vec<_>>(),
        [1, 1, 1]
    );
    assert_eq!(e.snapshot().entities.len(), 2, "no enemies appeared");
}

// --- Skills cast by companions -------------------------------------------------------------

#[test]
fn a_cast_shot_hits_for_its_own_damage() {
    let shot = json!({ "type": "shot", "damage": 40,
                       "pattern": { "preset": { "id": "aimed_single", "args": [2048] } } });
    let caster = companion(
        50,
        hold(46_080, 90_000),
        json!([rule(always(), json!("boss_first"), cast(shot), 1000)]),
    );
    // The weapons are quiet, so that every hit on the boss is the companion's.
    let mut e = engine(&quiet(stage_full(
        18_000,
        Some(boss_at(100_000, 25_600)),
        vec![],
        vec![caster],
    )));
    let events = play(&mut e, 80, |_| idle());
    assert_eq!(hit_damages(&events, 1), [40]);
}

#[test]
fn the_finished_run_still_ends_normally() {
    // Evades and companions do not change how a run ends.
    let mut e = engine(&stage_full(
        200,
        Some(boss_at(20, 25_600)),
        vec![],
        vec![companion(
            50,
            json!({ "type": "orbit", "radius": 5_000 }),
            json!([]),
        )],
    ));
    play(&mut e, 300, |t| {
        let evading = Input {
            evade: t % 7 == 0,
            ..idle()
        };
        firing(evading, t - 1)
    });
    assert_eq!(e.outcome(), Outcome::Cleared);
}
