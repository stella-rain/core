//! Attacks (ADR-036 tier 3): emitter trees carried out over time, expressions, presets and
//! their inline copies, and the runtime caps. An enemy standing at (46080, 50000) starts an
//! attack straight above the main character, so "aimed" means straight down; the orbs that
//! come out are read from the snapshot, and their velocities from where they first appear (one
//! move after they were fired).

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack_presets::AttackPresets;
use stella_rain_core::presets;
use stella_rain_core::registry::{self, Kind};
use stella_rain_core::rng::{TAG_ATTACK, derive};
use stella_rain_core::stage::Stage;
use stella_rain_core::trig::{PER_DEGREE, PER_TURN};
use stella_rain_core::validate::validate_attack;
use stella_rain_core::version::Versions;

/// Where the attackers stand: straight above the main character, 340 pixels, inside the leash's
/// reach (LS1).
const OX: i32 = near_start(0, -340).0;
const OY: i32 = near_start(0, -340).1;
const DEG: i32 = PER_DEGREE;

/// An orb as it first shows up: the tick, and where it is after its first move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    tick: u32,
    x: i32,
    y: i32,
}

impl Seen {
    /// Its velocity, taking it to have been fired from the enemy.
    fn v(&self) -> (i32, i32) {
        (self.x - OX, self.y - OY)
    }
}

/// A projectile with nothing to show but its flight: an orb that lives 100 ticks and bursts
/// into nothing (BR1c: the geometry of the emitters is read from orbs).
fn fire(speed: Value, direction: Value) -> Value {
    json!({ "type": "orb", "speed": speed, "direction": direction, "ticks": 100 })
}

fn aimed() -> Value {
    json!({ "type": "aimed", "offset": 0 })
}

fn absolute(angle: i32) -> Value {
    json!({ "type": "absolute", "angle": angle })
}

fn down(speed: i32) -> Value {
    fire(json!(speed), absolute(0))
}

fn wait(ticks: i32) -> Value {
    json!({ "type": "wait", "ticks": ticks })
}

fn repeat(count: Value, interval: Value, body: Vec<Value>) -> Value {
    json!({ "type": "repeat", "count": count, "interval_ticks": interval, "body": body })
}

fn ring(count: i32, body: Vec<Value>) -> Value {
    json!({ "type": "ring", "count": count, "body": body })
}

fn spread(count: i32, angle: i32, body: Vec<Value>) -> Value {
    json!({ "type": "spread", "count": count, "angle": angle, "body": body })
}

fn rotate(angle: i32, body: Vec<Value>) -> Value {
    json!({ "type": "rotate", "angle": angle, "body": body })
}

fn add(a: Value, b: Value) -> Value {
    json!({ "add": [a, b] })
}

fn mul(a: Value, b: Value) -> Value {
    json!({ "mul": [a, b] })
}

/// `v`, written as a sum, so that the validator, which range-checks constants, lets it through
/// and the engine's own clamps are what get tested.
fn calc(v: i32) -> Value {
    add(json!(v), json!(0))
}

/// An enemy that starts `attack` once, on its first tick, at the main character.
fn shooter(attack: Value) -> Value {
    enemy(
        50,
        hold(OX, OY),
        json!([{ "when": always(), "target": "player", "once": true,
                 "do": { "type": "attack", "attack": attack } }]),
    )
}

/// The weapons are quiet, so that every orb is the enemy's.
fn attack_stage(attack: Value) -> Stage {
    quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter(attack), OX, OY)],
        vec![],
    ))
}

/// Runs `ticks` ticks and lists every orb the attack made, in the order they appeared. The
/// orbs must all still be flying (short runs, slow orbs).
fn shots_of(stage: &Stage, ticks: u32) -> Vec<Seen> {
    let mut e = engine(stage);
    let mut seen = 0;
    let mut out = Vec::new();
    for _ in 0..ticks {
        e.step(&[idle()]);
        let orbs = e.snapshot().orbs;
        // Orbs that have left the field are gone from the list; start again from there.
        seen = seen.min(orbs.len());
        out.extend(orbs[seen..].iter().map(|b| Seen {
            tick: e.tick(),
            x: b.at.x.0,
            y: b.at.y.0,
        }));
        seen = orbs.len();
    }
    out
}

fn inline(nodes: Vec<Value>, ticks: u32) -> Vec<Seen> {
    shots_of(
        &attack_stage(json!({ "inline": spawned(json!(nodes)) })),
        ticks,
    )
}

fn velocities(shots: &[Seen]) -> Vec<(i32, i32)> {
    shots.iter().map(Seen::v).collect()
}

/// How fast each orb goes, whichever way: the length of its velocity.
fn speeds(shots: &[Seen]) -> Vec<i32> {
    shots
        .iter()
        .map(|s| {
            let (x, y) = s.v();
            let len = (i64::from(x).pow(2) + i64::from(y).pow(2))
                .unsigned_abs()
                .isqrt();
            i32::try_from(len).unwrap()
        })
        .collect()
}

fn ticks(shots: &[Seen]) -> Vec<u32> {
    shots.iter().map(|s| s.tick).collect()
}

/// Within `tolerance` of `expected`, component by component.
fn near(actual: &[(i32, i32)], expected: &[(i32, i32)], tolerance: i32) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{actual:?} against {expected:?}"
    );
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a.0 - e.0).abs() <= tolerance && (a.1 - e.1).abs() <= tolerance,
            "{actual:?} is not near {expected:?}"
        );
    }
}

// --- Directions ------------------------------------------------------------------------------

#[test]
fn aimed_points_at_the_target_and_turns_by_its_offset() {
    let aimed_by = |offset: i32| {
        velocities(&inline(
            vec![fire(
                json!(768),
                json!({ "type": "aimed", "offset": offset }),
            )],
            1,
        ))
    };
    assert_eq!(aimed_by(0), [(0, 768)]);
    // Positive angles turn from straight down toward the right.
    assert_eq!(aimed_by(90 * DEG), [(768, 0)]);
    assert_eq!(aimed_by(-90 * DEG), [(-768, 0)]);
    assert_eq!(aimed_by(180 * DEG), [(0, -768)]);
    near(&aimed_by(30 * DEG), &[(384, 665)], 1);
}

#[test]
fn aimed_follows_the_target_not_the_enemys_line() {
    // The main character a little to the right: the orb leans that way.
    let mut e = engine(&attack_stage(
        json!({ "inline": spawned(json!([fire(json!(768), aimed())])) }),
    ));
    e.step(&[moving(127, 0)]);
    let orb = e.snapshot().orbs[0];
    assert!(orb.at.x.0 > OX, "{orb:?}");
}

#[test]
fn absolute_ignores_the_target() {
    let v = |angle: i32| velocities(&inline(vec![fire(json!(768), absolute(angle))], 1));
    assert_eq!(v(0), [(0, 768)]);
    assert_eq!(v(90 * DEG), [(768, 0)]);
    assert_eq!(v(270 * DEG), [(-768, 0)]);
    assert_eq!(v(PER_TURN), [(0, 768)], "a full turn is no turn");
}

#[test]
fn relative_turns_from_the_direction_of_the_parent() {
    // An attack that a rule started has straight down as its parent direction.
    let v = |angle: i32| {
        velocities(&inline(
            vec![fire(
                json!(768),
                json!({ "type": "relative", "angle": angle }),
            )],
            1,
        ))
    };
    assert_eq!(v(0), [(0, 768)]);
    assert_eq!(v(90 * DEG), [(768, 0)]);
}

#[test]
fn sequential_turns_from_the_last_orb() {
    let step = |degrees: i32| {
        fire(
            json!(768),
            json!({ "type": "sequential", "step": degrees * DEG }),
        )
    };
    // Down, then each orb a quarter turn on: right, up, left, down.
    let shots = inline(vec![step(90), step(90), step(90), step(90)], 1);
    assert_eq!(
        velocities(&shots),
        [(768, 0), (0, -768), (-768, 0), (0, 768)]
    );
}

#[test]
fn a_ring_spreads_its_rounds_round_a_full_turn() {
    let shots = inline(vec![ring(4, vec![down(768)])], 1);
    assert_eq!(
        velocities(&shots),
        [(0, 768), (768, 0), (0, -768), (-768, 0)]
    );

    let twelve = velocities(&inline(vec![ring(8, vec![down(768)])], 1));
    assert_eq!(twelve.len(), 8);
    // Eight different directions of the same speed that cancel out.
    for (x, y) in &twelve {
        let speed2 = i64::from(*x).pow(2) + i64::from(*y).pow(2);
        assert!((speed2 - 768i64.pow(2)).abs() < 3_000, "{x},{y}");
    }
    let (sx, sy): (i32, i32) = twelve.iter().fold((0, 0), |a, v| (a.0 + v.0, a.1 + v.1));
    assert!(sx.abs() <= 8 && sy.abs() <= 8, "{sx},{sy}");
    let mut distinct = twelve.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 8);
}

#[test]
fn a_spread_fans_its_rounds_evenly_across_its_angle_round_the_aim() {
    let fan = |count: i32, angle: i32| {
        velocities(&inline(
            vec![spread(count, angle * DEG, vec![fire(json!(768), aimed())])],
            1,
        ))
    };
    // Three orbs over 90 degrees: -45, 0, +45 from straight down. 768 / sqrt(2) = 543.
    near(&fan(3, 90), &[(-543, 543), (0, 768), (543, 543)], 1);
    near(&fan(2, 60), &[(-384, 665), (384, 665)], 1);
    // A single orb goes straight at the target, whatever the angle.
    assert_eq!(fan(1, 90), [(0, 768)]);
    assert_eq!(fan(5, 0), [(0, 768); 5]);
}

#[test]
fn rotate_turns_everything_in_its_body_and_they_add_up() {
    let v = |nodes: Vec<Value>| velocities(&inline(nodes, 1));
    assert_eq!(v(vec![rotate(90 * DEG, vec![down(768)])]), [(768, 0)]);
    assert_eq!(
        v(vec![rotate(
            90 * DEG,
            vec![rotate(90 * DEG, vec![down(768)])]
        )]),
        [(0, -768)]
    );
    // Not for what is outside it.
    assert_eq!(v(vec![rotate(90 * DEG, vec![]), down(768)]), [(0, 768)]);
    // Turns also carry a ring or a spread round: 90 + 0/90/180/270.
    let shots = v(vec![rotate(90 * DEG, vec![ring(4, vec![down(768)])])]);
    assert_eq!(shots, [(768, 0), (0, -768), (-768, 0), (0, 768)]);
    // `sequential` goes on from the last orb and ignores the turns round it.
    let seq = fire(json!(768), json!({ "type": "sequential", "step": 0 }));
    assert_eq!(v(vec![rotate(90 * DEG, vec![seq])]), [(0, 768)]);
}

#[test]
fn a_negative_speed_goes_the_other_way() {
    assert_eq!(velocities(&inline(vec![down(-500)], 1)), [(0, -500)]);
}

// --- Time ------------------------------------------------------------------------------------

#[test]
fn repeat_runs_its_body_again_after_the_interval_from_the_end_of_the_last_round() {
    let shots = inline(vec![repeat(json!(3), json!(5), vec![down(768)])], 20);
    assert_eq!(ticks(&shots), [1, 6, 11]);
    // With a wait in the body the interval starts after it.
    let shots = inline(
        vec![repeat(json!(3), json!(5), vec![down(768), wait(2)])],
        30,
    );
    assert_eq!(ticks(&shots), [1, 8, 15]);
    // No interval: all at once.
    assert_eq!(
        ticks(&inline(
            vec![repeat(json!(3), json!(0), vec![down(768)])],
            5
        )),
        [1, 1, 1]
    );
}

#[test]
fn counts_are_clamped_and_empty_loops_do_nothing() {
    // The rounds are counted in zones, of which 128 may be alive; orbs are capped at 8 an owner.
    let mark = || {
        json!({ "type": "zone", "shape": { "type": "circle", "radius": 256 },
                "wind_up_ticks": 100, "active_ticks": 100 })
    };
    let marks = |nodes: Vec<Value>| {
        let mut e = engine(&attack_stage(json!({ "inline": spawned(json!(nodes)) })));
        e.step(&[idle()]);
        e.step(&[idle()]);
        e.snapshot().zones.len()
    };
    let n = |count: i32| marks(vec![repeat(calc(count), json!(0), vec![mark()])]);
    assert_eq!((n(0), n(1), n(100)), (0, 1, 100));
    // A count that an expression makes is clamped: negative to nothing, huge to 100.
    assert_eq!((n(-5), n(101), n(5_000)), (0, 100, 100));
    let big = mul(json!(1_000_000), json!(1_000_000));
    assert_eq!(
        marks(vec![repeat(big.clone(), json!(0), vec![mark()])]),
        100
    );
    let rings = |count: i32| {
        marks(vec![
            json!({ "type": "ring", "count": calc(count), "body": [mark()] }),
        ])
    };
    assert_eq!((rings(0), rings(-3), rings(7)), (0, 0, 7));
    assert_eq!(marks(vec![repeat(json!(5), json!(0), vec![])]), 0);
    assert_eq!(marks(vec![rotate(0, vec![]), spread(3, 0, vec![])]), 0);
}

#[test]
fn wait_pauses_the_attack() {
    let shots = inline(vec![down(768), wait(10), down(768)], 20);
    assert_eq!(ticks(&shots), [1, 11]);
    // Zero and negative waits do not pause; a wait is clamped to a stage's length.
    let negative = json!({ "type": "wait", "ticks": calc(-4) });
    assert_eq!(
        ticks(&inline(vec![down(768), wait(0), negative, down(768)], 3)),
        [1, 1]
    );
}

#[test]
fn loop_index_phase_time_and_arithmetic() {
    let vy = |nodes: Vec<Value>, ticks: u32| {
        velocities(&inline(nodes, ticks))
            .iter()
            .map(|v| v.1)
            .collect::<Vec<_>>()
    };
    // Rounds 0 to 3 of a repeat.
    let body = fire(mul(json!("loop_index"), json!(100)), absolute(0));
    assert_eq!(
        vy(vec![repeat(json!(4), json!(1), vec![body])], 6),
        [0, 100, 200, 300]
    );
    // The innermost loop's round, with a ring in a repeat.
    let inner = fire(mul(json!("loop_index"), json!(100)), absolute(0));
    // (The ring turns the orbs, so their speeds are read from the length of the velocity.)
    let shots = inline(
        vec![repeat(json!(2), json!(0), vec![ring(3, vec![inner])])],
        2,
    );
    let got = speeds(&shots);
    assert_eq!(got.len(), 6);
    for (g, want) in got.iter().zip([0, 100, 200, 0, 100, 200]) {
        assert!((g - want).abs() <= 1, "{got:?}");
    }
    // Outside any loop the round is 0.
    assert_eq!(
        vy(
            vec![fire(add(json!(50), json!("loop_index")), absolute(0))],
            1
        ),
        [50]
    );
    // Ticks since the attack started: 0, 4 and 8 on ticks 1, 5 and 9.
    let timed = fire(
        add(json!(100), mul(json!("phase_time"), json!(10))),
        absolute(0),
    );
    assert_eq!(
        vy(vec![repeat(json!(3), json!(4), vec![timed])], 12),
        [100, 140, 180]
    );
}

#[test]
fn expressions_follow_integer_rules() {
    let speed = |e: Value| velocities(&inline(vec![fire(e, absolute(0))], 1))[0].1;
    assert_eq!(speed(json!({ "add": [300, 200] })), 500);
    assert_eq!(speed(json!({ "sub": [100, 300] })), -200);
    assert_eq!(speed(json!({ "mul": [30, 20] })), 600);
    // Division truncates toward zero, and by zero gives 0.
    assert_eq!(speed(json!({ "div": [7, 2] })), 3);
    assert_eq!(speed(json!({ "div": [-7, 2] })), -3);
    assert_eq!(speed(json!({ "div": [5, 0] })), 0);
    assert_eq!(speed(json!({ "min": [5, 9] })), 5);
    assert_eq!(speed(json!({ "max": [5, 9] })), 9);
    assert_eq!(speed(json!({ "clamp": [500, 0, 300] })), 300);
    assert_eq!(speed(json!({ "clamp": [-5, 0, 300] })), 0);
    assert_eq!(speed(json!({ "clamp": [150, 0, 300] })), 150);
    // Bounds the wrong way round: the upper one wins.
    assert_eq!(speed(json!({ "clamp": [5, 10, 3] })), 3);
    // Arithmetic saturates rather than wrapping: 10^12 stays enormous, so the minimum is 700.
    let huge = json!({ "mul": [1_000_000, 1_000_000] });
    assert_eq!(speed(json!({ "min": [huge.clone(), 700] })), 700);
    assert_eq!(speed(json!({ "max": [{ "sub": [0, huge] }, -700] })), -700);
}

#[test]
fn dividing_the_smallest_value_by_minus_one_saturates() {
    // -10^12 saturates to i32::MIN; MIN / -1 is 2^31, which saturates to i32::MAX rather than
    // wrapping back to MIN (and, unchecked, panicking). The minimum with 700 tells them apart.
    let speed = |e: Value| velocities(&inline(vec![fire(e, absolute(0))], 1))[0].1;
    let smallest = json!({ "mul": [-1_000_000, 1_000_000] });
    let flipped = json!({ "div": [smallest, -1] });
    assert_eq!(speed(json!({ "min": [flipped, 700] })), 700);
}

#[test]
fn rand_draws_from_the_runs_generator_left_to_right() {
    // The stage's seed is 7: the two draws of one expression are the attack stream's first two.
    let both = json!({ "add": [{ "rand": [0, 1000] }, { "rand": [0, 1000] }] });
    let got = velocities(&inline(vec![fire(both, absolute(0))], 1))[0].1;
    let mut rng = derive(7, TAG_ATTACK);
    let expected = i32::try_from(rng.below(1001) + rng.below(1001)).unwrap();
    assert_eq!(got, expected);

    // A single draw per orb, in the order the orbs are made, and in range.
    let body = fire(json!({ "rand": [100, 200] }), absolute(0));
    let speeds: Vec<i32> = velocities(&inline(vec![repeat(json!(8), json!(0), vec![body])], 1))
        .iter()
        .map(|v| v.1)
        .collect();
    assert!(speeds.iter().all(|s| (100..=200).contains(s)));
    let mut rng = derive(7, TAG_ATTACK);
    let again: Vec<i32> = (0..8)
        .map(|_| 100 + i32::try_from(rng.below(101)).unwrap())
        .collect();
    assert_eq!(speeds, again);
    // An empty range is its lower end.
    let degenerate = |lo: i32, hi: i32| {
        velocities(&inline(
            vec![fire(json!({ "rand": [lo, hi] }), absolute(0))],
            1,
        ))[0]
            .1
    };
    assert_eq!((degenerate(5, 5), degenerate(9, 3)), (5, 9));
}

#[test]
fn an_empty_range_takes_no_draw_and_fields_draw_in_order() {
    let first_two = || {
        let mut rng = derive(7, TAG_ATTACK);
        (
            i32::try_from(rng.below(1001)).unwrap(),
            i32::try_from(rng.below(1001)).unwrap(),
        )
    };
    // `rand(5, 5)` does not use up a draw: the next `rand` gets the generator's first.
    let sum = add(json!({ "rand": [5, 5] }), json!({ "rand": [0, 1000] }));
    let got = velocities(&inline(vec![fire(sum, absolute(0))], 1))[0].1;
    assert_eq!(got, 5 + first_two().0);
    // The speed is worked out before the direction: a draw for each, in that order. The angle
    // is a draw of 0 to 1000 units of 1/256 degree, and the speed another, so the orb's
    // speed is the first draw and its turn the second.
    let rolled = fire(
        json!({ "rand": [0, 1000] }),
        json!({ "type": "absolute", "angle": { "rand": [0, 1000] } }),
    );
    let shot = inline(vec![rolled], 1)[0].v();
    let (speed, angle) = first_two();
    // sin of a small angle: angle / 256 degrees * pi / 180 = angle * 6.82e-5 radians.
    let expected_x = i64::from(speed) * i64::from(angle) * 682 / 10_000_000;
    let x = i64::from(shot.0);
    assert!(
        (x - expected_x).abs() <= 3,
        "{shot:?} for speed {speed} and angle {angle}"
    );
    let length = (x.pow(2) + i64::from(shot.1).pow(2)).unsigned_abs().isqrt();
    assert!(
        i64::try_from(length).unwrap().abs_diff(i64::from(speed)) <= 2,
        "{shot:?} {speed}"
    );
}

#[test]
fn orbs_start_where_the_attacks_owner_is_at_that_moment() {
    // The enemy walks down 100 a tick; its attack fires on tick 1 and again on tick 11.
    let walker = enemy(
        50,
        json!({ "type": "straight", "vx": 0, "vy": 100 }),
        json!([{ "when": always(), "target": "player", "once": true,
                 "do": { "type": "attack",
                         "attack": { "inline": spawned(json!([down(0), wait(10), down(0)])) } } }]),
    );
    let stage = quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, walker, OX, OY)],
        vec![],
    ));
    let shots = shots_of(&stage, 12);
    // Tick 1 the enemy has moved once; tick 11 ten times more.
    assert_eq!(
        shots.iter().map(|s| (s.tick, s.y)).collect::<Vec<_>>(),
        [(1, OY + 100), (11, OY + 1_100)]
    );
}

#[test]
fn the_seed_changes_the_draws() {
    let body = || fire(json!({ "rand": [0, 100_000] }), absolute(0));
    let mut a = attack_stage(
        json!({ "inline": spawned(json!([repeat(json!(8), json!(0), vec![body()])])) }),
    );
    let first = velocities(&shots_of(&a, 1));
    a.seed += 1;
    assert_ne!(first, velocities(&shots_of(&a, 1)));
}

// --- Caps ------------------------------------------------------------------------------------

#[test]
fn a_huge_attack_is_spread_over_ticks_by_the_step_budget() {
    // 100 rounds of 100 rounds of nothing, then an orb: 1 + 100 * 202 + 1 = 20,202 steps. At
    // 4,096 a tick that is five ticks, so the orb is made on tick 5, not tick 1.
    let inner = repeat(json!(100), json!(0), vec![wait(0)]);
    let shots = inline(
        vec![repeat(json!(100), json!(0), vec![inner]), down(768)],
        8,
    );
    assert_eq!(ticks(&shots), [5]);
    // One round of 100 is far inside the budget.
    let small = inline(
        vec![repeat(json!(100), json!(0), vec![wait(0)]), down(768)],
        3,
    );
    assert_eq!(ticks(&small), [1]);
}

// --- Presets and their inline copies ---------------------------------------------------------

fn hashes(stage: &Stage, ticks: u32) -> Vec<u64> {
    let mut e = engine(stage);
    (0..ticks)
        .map(|t| {
            e.step(&[moving(if t % 40 < 20 { 64 } else { -64 }, 0)]);
            e.state_hash()
        })
        .collect()
}

#[test]
fn a_preset_and_its_inline_copy_reach_the_same_hashes() {
    for (id, args) in [
        ("aimed_single", vec![15]),
        ("aimed_spread", vec![20]),
        ("spiral", vec![30]),
        ("spread_5", vec![]),
        ("twin_shot", vec![]),
    ] {
        let by_id = json!({ "preset": { "id": id, "args": args } });
        let apart = presets::take_apart(AttackPresets::bundled(), id, &args)
            .expect("a preset to take apart");
        let copy = json!({ "inline": apart });
        let rule = |attack: Value| {
            enemy(
                50,
                hold(OX, OY),
                json!([{ "when": always(), "target": "player", "cooldown_ticks": 60,
                         "do": { "type": "attack", "attack": attack } }]),
            )
        };
        let stage = |attack: Value| {
            quiet(stage_full(
                600,
                Some(far_boss()),
                vec![wave(1, 1, 0, rule(attack), OX, OY)],
                vec![],
            ))
        };
        let a = hashes(&stage(by_id), 200);
        let b = hashes(&stage(copy), 200);
        assert_eq!(a, b, "{id}");
    }
}

#[test]
fn taking_a_preset_apart_writes_its_arguments_in_as_constants() {
    let apart = presets::take_apart(AttackPresets::bundled(), "aimed_single", &[20]).unwrap();
    assert_eq!(
        serde_json::to_value(&apart).unwrap(),
        spawned(
            json!([{ "type": "zone", "shape": { "type": "circle", "radius": 3_072 },
                         "anchor": "target", "offset": { "forward": 0, "right": 0 },
                         "direction": { "type": "aimed", "offset": 0 },
                         "wind_up_ticks": 20, "active_ticks": 2 }])
        )
    );
    assert_eq!(presets::highest_arg(&apart), None);
    // The wrong number of arguments, or no such preset.
    assert!(presets::take_apart(AttackPresets::bundled(), "aimed_single", &[]).is_none());
    assert!(presets::take_apart(AttackPresets::bundled(), "aimed_single", &[1, 2]).is_none());
    assert!(presets::take_apart(AttackPresets::bundled(), "twin_shot", &[1]).is_none());
    assert!(presets::take_apart(AttackPresets::bundled(), "no_such_preset", &[]).is_none());
}

#[test]
fn the_validator_holds_a_preset_to_the_budgets_of_an_inline_attack() {
    let phases: Vec<stella_rain_core::attack::AttackPhase> =
        serde_json::from_value(spawned(json!([
            { "type": "orb", "speed": { "arg": 1 }, "ticks": 100,
              "direction": { "type": "aimed", "offset": 0 } }
        ])))
        .unwrap();
    assert!(validate_attack(&phases, 2, registry::ENTRIES).is_ok());
    let issues = validate_attack(&phases, 1, registry::ENTRIES).unwrap_err();
    assert_eq!(issues[0].path, "$[0].spawn[0].speed");
}

// --- What the presets do ---------------------------------------------------------------------

/// The zones a preset places by tick `ticks`, as `(tick placed, direction)` with the direction
/// as 256 times its unit vector. Run with a warning long enough for none to end first.
fn preset_zones(id: &str, args: Vec<i32>, ticks: u32) -> Vec<(u32, (i32, i32))> {
    let stage = attack_stage(json!({ "preset": { "id": id, "args": args } }));
    let mut e = engine(&stage);
    let mut seen = 0;
    let mut out = Vec::new();
    for _ in 0..ticks {
        e.step(&[idle()]);
        let zones = e.snapshot().zones;
        out.extend(zones[seen..].iter().map(|z| {
            let to256 = |c: i32| c * 256 / stella_rain_core::trig::SCALE;
            (e.tick(), (to256(z.direction.x), to256(z.direction.y)))
        }));
        seen = zones.len();
    }
    out
}

#[test]
fn the_aimed_presets_fan_out_round_the_aim() {
    // One circle where the main character stands.
    let mut e = engine(&attack_stage(
        json!({ "preset": { "id": "aimed_single", "args": [20] } }),
    ));
    e.step(&[idle()]);
    let zones = e.snapshot().zones;
    assert_eq!(zones.len(), 1);
    let player = e.snapshot().players[0].at;
    assert_eq!((zones[0].at.x, zones[0].at.y), (player.x, player.y));
    // Seven lines over 30 degrees: 15 degrees either side of straight down.
    let fan: Vec<_> = preset_zones("aimed_spread", vec![600], 1)
        .iter()
        .map(|z| z.1)
        .collect();
    assert_eq!(fan.len(), 7);
    assert_eq!(fan[3], (0, 256));
    near(&fan[..1], &[(-66, 247)], 1);
    near(&fan[6..], &[(66, 247)], 1);
    // Five over 20 degrees; two 5 degrees apart.
    assert_eq!(preset_zones("spread_5", vec![], 1).len(), 5);
    let twin = preset_zones("twin_shot", vec![], 1);
    assert_eq!(twin.len(), 2);
    assert_eq!((twin[0].1).0, -(twin[1].1).0);
}

#[test]
fn the_spiral_turns_six_degrees_a_line_every_three_ticks() {
    // A warning of 600 ticks, so that no line has gone before the last is placed.
    let lines = preset_zones("spiral", vec![600], 125);
    assert_eq!(lines.len(), 40);
    let at: Vec<u32> = lines.iter().map(|l| l.0).collect();
    assert_eq!(at[..3], [1, 4, 7]);
    assert_eq!(at[39], 118);
    // The k-th line is turned 6 degrees * (k + 1) from straight down: x = 256 sin, y = 256 cos.
    let v: Vec<(i32, i32)> = lines.iter().map(|l| l.1).collect();
    near(&v[..1], &[(27, 255)], 2); // 6 degrees
    near(&v[14..15], &[(256, 0)], 2); // the 15th: 90 degrees, to the right
    near(&v[29..30], &[(0, -256)], 2); // the 30th: 180 degrees, up
    near(&v[39..40], &[(-222, -128)], 3); // the 40th: 240 degrees
}

// --- The fixture and the rest of the engine --------------------------------------------------

#[test]
fn an_orb_names_its_style() {
    let styled = json!({ "type": "orb", "speed": 768, "style": "orb_small", "ticks": 100,
                "direction": absolute(0) });
    let mut e = engine(&attack_stage(
        json!({ "inline": spawned(json!([styled, down(768)])) }),
    ));
    e.step(&[idle()]);
    let styles: Vec<u16> = e.snapshot().orbs.iter().map(|b| b.style).collect();
    assert_eq!(styles, [1, 0]);
    assert_eq!(registry::orb_style("orb_small"), 1);
    assert_eq!(registry::orb_style("no_such_orb"), 0);
}

// --- A run reads its own presets (CS1b-1) ------------------------------------------------------

fn slam_stage(attack: Value) -> Stage {
    let rule = enemy(
        50,
        hold(OX, OY),
        json!([{ "when": always(), "target": "player", "cooldown_ticks": 60,
                 "do": { "type": "attack", "attack": attack } }]),
    );
    quiet(stage_full(
        600,
        Some(far_boss()),
        vec![wave(1, 1, 0, rule, OX, OY)],
        vec![],
    ))
}

fn content_with_slam_wind_up(ticks: u32) -> stella_rain_core::run_content::RunContent {
    let mut file: Value = serde_json::from_slice(bundled_file("attack_presets")).unwrap();
    let list = file["presets"].as_array_mut().unwrap();
    let slam = list.iter_mut().find(|p| p["id"] == "slam").unwrap();
    slam["phases"][0]["ticks"] = json!(ticks);
    design_content().with_attack_presets(
        AttackPresets::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap(),
    )
}

fn hashes_with(stage: &Stage, content: &stella_rain_core::run_content::RunContent) -> Vec<u64> {
    let mut e = stella_rain_core::engine::Engine::new_with(stage, content).unwrap();
    (0..300)
        .map(|t| {
            e.step(&[moving(if t % 40 < 20 { 64 } else { -64 }, 0)]);
            e.state_hash()
        })
        .collect()
}

#[test]
fn a_preset_runs_as_the_run_s_own_content_defines_it() {
    let by_id = slam_stage(json!({ "preset": { "id": "slam" } }));
    let bundled = design_content();
    let other = content_with_slam_wind_up(24);
    let a = hashes_with(&by_id, &bundled);
    let b = hashes_with(&by_id, &other);
    assert_ne!(a, b, "the edited preset runs");
    // It equals the same stage with the edited preset written inline.
    let apart = presets::take_apart(other.attack_presets(), "slam", &[]).unwrap();
    let inline = slam_stage(json!({ "inline": apart }));
    assert_eq!(b, hashes_with(&inline, &bundled));
}

#[test]
fn every_registered_preset_is_in_the_pack_and_passes_the_validator() {
    let presets = AttackPresets::bundled();
    let registered: Vec<_> = registry::ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Preset)
        .collect();
    assert_eq!(
        registered.iter().map(|e| e.id).collect::<Vec<_>>(),
        presets.ids().map(|i| i.as_str()).collect::<Vec<_>>(),
        "the registry and the pack list the same presets in the same order"
    );
    for entry in registered {
        let def = presets.definition(entry.id).expect("a definition");
        validate_attack(def, entry.args, registry::ENTRIES)
            .unwrap_or_else(|issues| panic!("{}: {issues:?}", entry.id));
        let used = presets::highest_arg(def).map_or(0, |n| n + 1);
        assert_eq!(used, entry.args, "{}", entry.id);
    }
}
