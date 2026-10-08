//! Agents (ADR-036 tier 1): rule lists, selectors, conditions, statuses, aggro, movement,
//! summons and the main character's skills (ADR-032). Small stages whose every tick can be
//! worked out by hand; the expected numbers come from the constants in `rules_v0`.

mod common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EntityKind;

// --- Builders --------------------------------------------------------------------------------

fn skill_slot(slot: u8, target: &str, skill: Value, cooldown_ticks: u32) -> Value {
    json!({ "slot": slot, "target": target, "skill": skill, "cooldown_ticks": cooldown_ticks })
}

fn held(slot: u8) -> Input {
    Input {
        held: slot,
        ..Input::default()
    }
}

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
    stage_full(18_000, Some(far_boss()), waves, companions, vec![])
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
        vec![],
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
        vec![],
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
        vec![],
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
        vec![],
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
        vec![],
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
        vec![],
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
    // An enemy shoots the companion K (10 hp) once per tick; K reacts when hit points are
    // below 100% (the first hit) and below 50% (after the sixth hit, at 4 hp).
    let shooter = enemy(
        50,
        hold(46_080, 100_000),
        json!([rule(
            always(),
            json!("nearest_ally"),
            json!({
        "type": "attack", "attack": { "preset": { "id": "twin_shot", "args": [] } } }),
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
        vec![companion(10, hold(46_080, 130_000), k_rules)],
        vec![],
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
    // Rule 0 fires the tick after the first hit (9 of 10 hp), rule 1 the tick after the
    // sixth (4 of 10 hp; 5 of 10 is not below 50%).
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
    // A buff gives +40 on tick 1; the aggro is 40 - (n - 1) on tick n, so it is above 30 until
    // tick 10.
    let rules = json!([
        rule(always(), json!("self"), atk_up(10, 5), 1000),
        rule(
            json!({ "type": "aggro_above", "who": "self", "value": 30 }),
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

#[test]
fn player_firing_needs_the_player_to_have_fired_for_min_ticks_in_a_row() {
    let stage = |when: Value| {
        with_companions(
            vec![companion(
                50,
                hold(46_080, 90_000),
                json!([rule(when, json!("self"), telegraph(), 0)]),
            )],
            vec![],
        )
    };
    // Fire on ticks 1 to 3, stop on 4, fire on 5 to 11.
    let input = |t: u32| {
        if (1..=3).contains(&t) || (5..=11).contains(&t) {
            firing()
        } else {
            idle()
        }
    };
    let run = |when: Value| {
        let mut e = engine(&stage(when));
        let events = play(&mut e, 14, input);
        rule_firings_of(&events, 2)
            .iter()
            .map(|f| f.0)
            .collect::<Vec<u32>>()
    };
    assert_eq!(
        run(json!({ "type": "player_firing", "min_ticks": 5 })),
        [9, 10, 11]
    );
    assert_eq!(
        run(json!({ "type": "player_firing" })),
        [1, 2, 3, 5, 6, 7, 8, 9, 10, 11]
    );
    assert_eq!(
        run(json!({ "type": "player_firing", "min_ticks": 1 })),
        [1, 2, 3, 5, 6, 7, 8, 9, 10, 11]
    );
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
    e.step(idle());
    // 1.5 degrees on tick 1 is the whole degree 1: sin 18 / cos 1024 (times 1024). The second
    // companion starts a quarter turn on: sin 1024, cos -18.
    assert_eq!(companion_at(&e, 2), (START_X + 10_240, START_Y + 180));
    assert_eq!(companion_at(&e, 3), (START_X - 180, START_Y + 10_240));
    for _ in 0..3 {
        e.step(idle());
    }
    // 6 degrees on tick 4: sin 107, cos 1018.
    assert_eq!(companion_at(&e, 2), (START_X + 10_180, START_Y + 1_070));
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
        e.step(if t < 30 { moving(-10_000, 0) } else { idle() });
        let (px, py) = player_at(&e);
        let (cx, cy) = companion_at(&e, 2);
        let d2 = i64::from(cx - px).pow(2) + i64::from(cy - py).pow(2);
        let r2 = 10_240i64.pow(2);
        // The table has whole degrees, so the radius is right to within a little.
        assert!((d2 - r2).abs() < r2 / 100, "tick {t}: {d2} vs {r2}");
    }
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
        e.step(moving(-10_000, 0));
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
        vec![],
    );
    let mut e = engine(&stage);
    let events = play(&mut e, 90, |_| firing());
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
            vec![],
        );
        let mut e = engine(&stage);
        hit_damages(&play(&mut e, 120, |_| firing()), 1)
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
        vec![],
    ));
    e.step(moving(10_000, 0));
    assert_eq!(player_at(&e).0, START_X + SPEED);
    e.step(moving(10_000, 0));
    // Half speed from the tick after the cast.
    assert_eq!(player_at(&e).0, START_X + SPEED + SPEED / 2);

    // Stacks add up but never past 90%: one tenth of the speed is left.
    let rules = json!([rule(always(), json!("player"), slow(100), 0)]);
    let mut e = engine(&stage_full(
        18_000,
        Some(far_boss()),
        vec![],
        vec![companion(50, hold(0, 0), rules)],
        vec![],
    ));
    for _ in 0..20 {
        e.step(moving(10_000, 0));
    }
    let before = player_at(&e).0;
    e.step(moving(10_000, 0));
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
        vec![],
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
    // An enemy hits the companion K (10 hp) once a tick; the healer H (far away) heals the
    // lowest-hp ally by 5 whenever someone is hurt. K is hit, then healed back by exactly the
    // 1 hit point it lacks, every tick.
    let shooter = enemy(
        50,
        hold(46_080, 100_000),
        json!([rule(
            always(),
            json!("nearest_ally"),
            json!({
        "type": "attack", "attack": { "preset": { "id": "twin_shot", "args": [] } } }),
            0
        )]),
    );
    let healer = json!([rule(
        json!({ "type": "hp_below", "who": "lowest_hp_ally", "pct": 100 }),
        json!("lowest_hp_ally"),
        cast(json!({ "type": "heal", "amount": 5 })),
        0
    )]);
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![
            companion(10, hold(46_080, 130_000), json!([])),
            companion(10, hold(80_000, 147_456), healer),
        ],
        vec![],
    );
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
        healed.iter().all(|h| *h == (id(2), 1)),
        "every heal restores the 1 point missing"
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
            json!({ "type": "attack", "attack": { "preset": { "id": "twin_shot", "args": [] } } }),
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
        vec![],
    );
    let mut e = engine(&stage);
    let events = play(&mut e, 100, |_| firing());
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
        "type": "attack", "attack": { "preset": { "id": "twin_shot", "args": [] } } }),
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
        vec![],
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
    let mut e = engine(&stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, summoner, 46_080, 50_000)],
        vec![],
        vec![],
    ));
    e.step(idle());
    let s = e.snapshot();
    let ids: Vec<u32> = s.entities.iter().map(|x| x.id.0).collect();
    assert_eq!(
        ids,
        [1, 2, 3, 4],
        "boss, the summoner, then the two summoned"
    );
    // They appear where the summoner stands, and start acting (and holding) next tick.
    assert_eq!(s.entities[2].at.x.0, 46_080);
    e.step(idle());
    let s = e.snapshot();
    assert_eq!((s.entities[2].at.x.0, s.entities[2].at.y.0), (1000, 1000));
}

#[test]
fn summons_respect_the_enemy_cap() {
    // 200 enemies on tick 1, one of which summons 8 more; later waves fill the rest. Never
    // more than 300 alive.
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
    let mut e = engine(&stage_full(18_000, Some(far_boss()), waves, vec![], vec![]));
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
        assert!(alive(&e) <= 300);
    }
    assert_eq!(alive(&e), 300);
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

// --- The main character's skills (ADR-032) ----------------------------------------------------

fn shot_skill(damage: u32) -> Value {
    json!({ "type": "shot", "damage": damage, "pattern": { "preset": { "id": "twin_shot", "args": [] } } })
}

fn skill_stage(skills: Vec<Value>) -> Engine {
    engine(&stage_full(
        18_000,
        Some(boss_at(100_000, 25_600)),
        vec![],
        vec![],
        skills,
    ))
}

fn casts(events: &[(u32, DomainEvent)]) -> Vec<(u32, u8, Option<EntityId>)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::SkillCast { slot, target } => Some((*t, *slot, *target)),
            _ => None,
        })
        .collect()
}

/// `held(slot)` on ticks `from..to`, nothing otherwise.
fn hold_between(slot: u8, from: u32, to: u32) -> impl Fn(u32) -> Input {
    move |t| {
        if (from..to).contains(&t) {
            held(slot)
        } else {
            idle()
        }
    }
}

#[test]
fn releasing_a_held_skill_casts_it_at_its_target() {
    let mut e = skill_stage(vec![skill_slot(1, "boss_first", shot_skill(40), 100)]);
    // Pressed on tick 5, released on tick 10.
    let events = play(&mut e, 20, hold_between(1, 5, 10));
    assert_eq!(casts(&events), [(10, 1, Some(id(1)))]);
    let skill = &e.snapshot().player.skills[0];
    assert_eq!(
        (skill.slot, skill.ready, skill.cooldown_left),
        (1, false, 90)
    );
}

#[test]
fn a_cast_shot_hits_for_its_own_damage() {
    let mut e = skill_stage(vec![skill_slot(1, "boss_first", shot_skill(40), 100)]);
    let events = play(&mut e, 80, hold_between(1, 1, 3));
    assert_eq!(casts(&events).len(), 1);
    assert_eq!(hit_damages(&events, 1), [40]);
}

#[test]
fn a_skill_on_cooldown_cannot_be_held() {
    let mut e = skill_stage(vec![skill_slot(1, "boss_first", shot_skill(40), 100)]);
    // Cast on tick 3 (pressed 1, released 3); the cooldown of 100 ends on tick 103. A press at
    // tick 50 does nothing: no hold, no cast on release.
    let input = |t: u32| {
        if t < 3 || (50..60).contains(&t) {
            held(1)
        } else {
            idle()
        }
    };
    let events = play(&mut e, 110, input);
    assert_eq!(casts(&events).iter().map(|c| c.0).collect::<Vec<_>>(), [3]);
    assert!(e.snapshot().player.skills[0].ready);
}

#[test]
fn a_long_hold_casts_by_itself_after_thirty_ticks() {
    // A short cooldown, so that a second cast would be possible if holding on counted as a new press.
    let mut e = skill_stage(vec![skill_slot(1, "boss_first", shot_skill(40), 5)]);
    // Pressed on tick 10 and never let go: the cast is on tick 40, and holding on does not
    // start another.
    let events = play(&mut e, 120, hold_between(1, 10, 200));
    assert_eq!(casts(&events).iter().map(|c| c.0).collect::<Vec<_>>(), [40]);
    // Letting go and pressing again does: pressed on tick 130, auto-cast on tick 160.
    let events = play(&mut e, 80, |t| if t >= 130 { held(1) } else { idle() });
    assert_eq!(
        casts(&events).iter().map(|c| c.0).collect::<Vec<_>>(),
        [160]
    );
}

#[test]
fn switching_slots_releases_the_first_and_presses_the_second() {
    let mut e = skill_stage(vec![
        skill_slot(1, "boss_first", shot_skill(40), 1000),
        skill_slot(
            2,
            "player",
            json!({ "type": "atk_up", "value_pct": 10, "duration_ticks": 100 }),
            1000,
        ),
    ]);
    let input = |t: u32| match t {
        3..=9 => held(1),
        10..=19 => held(2),
        _ => idle(),
    };
    let events = play(&mut e, 30, input);
    assert_eq!(casts(&events), [(10, 1, Some(id(1))), (20, 2, Some(id(0)))]);
}

#[test]
fn charges_run_out_and_the_view_says_so() {
    let mut slot = skill_slot(
        1,
        "player",
        json!({ "type": "atk_up", "value_pct": 10, "duration_ticks": 5 }),
        0,
    );
    slot["charges"] = json!(2);
    let mut e = skill_stage(vec![slot]);
    assert_eq!(e.snapshot().player.skills[0].charges_left, Some(2));
    // Three quick presses and releases: two casts.
    let input = |t: u32| {
        if matches!(t, 2 | 5 | 8) {
            held(1)
        } else {
            idle()
        }
    };
    let events = play(&mut e, 12, input);
    assert_eq!(
        casts(&events).iter().map(|c| c.0).collect::<Vec<_>>(),
        [3, 6]
    );
    let view = &e.snapshot().player.skills[0];
    assert_eq!((view.ready, view.charges_left), (false, Some(0)));
}

#[test]
fn a_skill_with_nobody_to_aim_at_fizzles_and_costs_nothing() {
    let mut e = skill_stage(vec![skill_slot(
        1,
        "nearest_ally",
        json!({ "type": "heal", "amount": 5 }),
        100,
    )]);
    let events = play(&mut e, 10, hold_between(1, 2, 5));
    assert_eq!(casts(&events), []);
    assert!(e.snapshot().player.skills[0].ready);
}

#[test]
fn a_held_slot_that_does_not_exist_holds_nothing() {
    let mut e = skill_stage(vec![skill_slot(1, "boss_first", shot_skill(40), 100)]);
    for slot in [0u8, 2, 3, 4, 7, 255] {
        let events = play(&mut e, 10, hold_between(slot, 1, 5));
        assert_eq!(casts(&events), [], "slot {slot}");
    }
}

#[test]
fn a_skill_view_counts_down_its_cooldown() {
    let mut e = skill_stage(vec![skill_slot(1, "boss_first", shot_skill(40), 20)]);
    play(&mut e, 3, hold_between(1, 1, 3));
    let at_cast = e.snapshot().player.skills[0].cooldown_left;
    assert_eq!(at_cast, 20);
    play(&mut e, 5, |_| idle());
    assert_eq!(e.snapshot().player.skills[0].cooldown_left, 15);
    play(&mut e, 15, |_| idle());
    assert!(e.snapshot().player.skills[0].ready);
}

#[test]
fn a_cast_buff_raises_aggro_and_the_main_characters_damage() {
    let slot = skill_slot(
        1,
        "player",
        json!({ "type": "atk_up", "value_pct": 100, "duration_ticks": 1000 }),
        1000,
    );
    let mut e = engine(&stage_full(
        18_000,
        Some(boss_at(5_000, 25_600)),
        vec![],
        vec![],
        vec![slot],
    ));
    // Buff on tick 3; the shots fired since are doubled.
    let input = |t: u32| {
        if t < 3 {
            Input {
                held: 1,
                ..firing()
            }
        } else {
            firing()
        }
    };
    let events = play(&mut e, 90, input);
    let damages = hit_damages(&events, 1);
    assert_eq!(&damages[..3], [10, 20, 20], "{damages:?}");
}

#[test]
fn the_finished_run_still_ends_normally() {
    // Skills and companions do not change how a run ends.
    let mut e = engine(&stage_full(
        200,
        Some(boss_at(20, 25_600)),
        vec![],
        vec![companion(
            50,
            json!({ "type": "orbit", "radius": 5_000 }),
            json!([]),
        )],
        vec![skill_slot(
            1,
            "player",
            json!({ "type": "atk_up", "value_pct": 10, "duration_ticks": 50 }),
            10,
        )],
    ));
    play(&mut e, 300, |t| Input {
        held: (t % 7 == 0) as u8,
        ..firing()
    });
    assert_eq!(e.outcome(), Outcome::Cleared);
}
