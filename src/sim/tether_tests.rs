//! The soft tether (ADR-SIM-7): it pulls the up players farthest from the party's centre back
//! past `TETHER_RANGE`, never moves a dashing, downed or out player, and costs solo nothing.

use std::sync::Arc;

use super::player::{TETHER_PULL_DIV, TETHER_RANGE, tether};
use super::state::{PlayerIdx, State};
use super::{World, schedule};
use crate::engine::{Engine, Run};
use crate::fixed::{Fx, Point};
use crate::input::Input;
use crate::run_setup::RunSetup;
use crate::stage::Map;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

fn px(x: i32, y: i32) -> Point {
    Point {
        x: Fx::px(x),
        y: Fx::px(y),
    }
}

/// A run of `n` players on an 8000 by 8000 px map with no enemies, each at the given place.
fn start(n: u8, at: &[Point]) -> Engine {
    let setup = RunSetup {
        seed: 20_261_105,
        ..RunSetup::default()
    };
    let mut e = Engine::start_party(&setup, n).expect("a party");
    let (world, s) = live_mut(&mut e);
    world.stage.map = Some(Map {
        width: Fx::px(8000),
        height: Fx::px(8000),
        start: None,
        facing: None,
    });
    s.enemies.clear();
    s.companions.clear();
    for (i, p) in at.iter().enumerate() {
        s.players[slot(u8::try_from(i).unwrap())].at = *p;
    }
    e
}

fn live_mut(e: &mut Engine) -> (&mut World, &mut State) {
    match e.run_mut() {
        Run::V2 { world, state } => (
            Arc::get_mut(world).expect("the engine has not been cloned"),
            state,
        ),
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn state(e: &Engine) -> &State {
    match e.run() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn places(e: &Engine) -> Vec<Point> {
    let s = state(e);
    s.players.indices().map(|i| s.players[i].at).collect()
}

fn pull(e: &mut Engine) {
    let (world, s) = live_mut(e);
    tether(world, s);
}

fn down(e: &mut Engine, i: u8) {
    let p = &mut live_mut(e).1.players[slot(i)];
    p.hp = 0;
    p.down_ticks = 100;
}

fn step_clean(e: &mut Engine, n: u32, inputs: &[Input; 3]) {
    for _ in 0..n {
        e.step(inputs);
        let s = live_mut(e).1;
        s.enemies.clear();
        s.companions.clear();
    }
}

#[test]
fn the_constants_are_840_px_and_an_eighth() {
    assert_eq!(TETHER_RANGE, Fx::px(840));
    assert_eq!(TETHER_PULL_DIV, 8);
}

#[test]
fn at_range_nothing_moves() {
    let mut e = start(2, &[px(1000, 500), px(1840, 500)]);
    pull(&mut e);
    assert_eq!(places(&e), vec![px(1000, 500), px(1840, 500)]);
}

#[test]
fn past_range_both_are_pulled() {
    let mut e = start(2, &[px(1000, 500), px(1880, 500)]);
    pull(&mut e);
    assert_eq!(places(&e), vec![px(1005, 500), px(1875, 500)]);
}

#[test]
fn three_players_in_a_tie_pull_both_extremes() {
    let mut e = start(3, &[px(1000, 500), px(1450, 500), px(1900, 500)]);
    pull(&mut e);
    let step = 60 * 256 / 8;
    assert_eq!(
        places(&e),
        vec![
            Point {
                x: Fx(Fx::px(1000).0 + step),
                y: Fx::px(500)
            },
            px(1450, 500),
            Point {
                x: Fx(Fx::px(1900).0 - step),
                y: Fx::px(500)
            },
        ]
    );
}

/// The tether's move for `me`, written out with `i128`, apart from the code under test.
fn reference(at: &[Point], me: usize) -> (i64, i64) {
    let xs = at.iter().map(|p| i128::from(p.x.0));
    let ys = at.iter().map(|p| i128::from(p.y.0));
    let c2x = xs.clone().min().unwrap() + xs.max().unwrap();
    let c2y = ys.clone().min().unwrap() + ys.max().unwrap();
    let mut far = 0_i128;
    for a in at {
        for b in at {
            let (dx, dy) = (i128::from(a.x.0 - b.x.0), i128::from(a.y.0 - b.y.0));
            far = far.max(dx * dx + dy * dy);
        }
    }
    let range = i128::from(TETHER_RANGE.0);
    if far <= range * range {
        return (0, 0);
    }
    let step = (far.isqrt() - range) / i128::from(TETHER_PULL_DIV);
    let v = |p: &Point| (c2x - 2 * i128::from(p.x.0), c2y - 2 * i128::from(p.y.0));
    let d2 = |p: &Point| {
        let (x, y) = v(p);
        x * x + y * y
    };
    let top = at.iter().map(d2).max().unwrap();
    if d2(&at[me]) != top {
        return (0, 0);
    }
    let (x, y) = v(&at[me]);
    let l2 = (x * x + y * y).isqrt();
    if l2 == 0 {
        return (0, 0);
    }
    let step = step.min(l2 / 2);
    (
        i64::try_from(x * step / l2).unwrap(),
        i64::try_from(y * step / l2).unwrap(),
    )
}

#[test]
fn three_players_one_far_corner_pulls_only_that_one() {
    let at = [px(1000, 1000), px(1900, 1100), px(1300, 1400)];
    let mut e = start(3, &at);
    pull(&mut e);
    let now = places(&e);
    let (dx, dy) = reference(&at, 0);
    assert!(dx > 1800 && dx < 2000 && dy > 800 && dy < 900, "{dx} {dy}");
    assert_eq!(now[0].x.0 - at[0].x.0, i32::try_from(dx).unwrap());
    assert_eq!(now[0].y.0 - at[0].y.0, i32::try_from(dy).unwrap());
    assert_eq!(now[1], at[1]);
    assert_eq!(now[2], at[2]);
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: i32) -> i32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        i32::try_from((self.0 >> 33) % u64::try_from(below).unwrap()).unwrap()
    }
}

fn dist2(a: Point, b: (i64, i64)) -> i128 {
    let dx = i128::from(i64::from(a.x.0) * 2 - b.0);
    let dy = i128::from(i64::from(a.y.0) * 2 - b.1);
    dx * dx + dy * dy
}

#[test]
fn a_pull_never_passes_the_centre_and_has_no_slot_bias() {
    let mut rng = Lcg(7);
    for _ in 0..1000 {
        let at: Vec<Point> = (0..3)
            .map(|_| px(2000 + rng.next(4000), 2000 + rng.next(4000)))
            .collect();
        let c2 = |f: fn(&Point) -> i32| {
            i64::from(at.iter().map(f).min().unwrap()) + i64::from(at.iter().map(f).max().unwrap())
        };
        let centre = (c2(|p| p.x.0), c2(|p| p.y.0));
        let mut e = start(3, &at);
        pull(&mut e);
        let after = places(&e);
        for (before, now) in at.iter().zip(&after) {
            assert!(dist2(*now, centre) <= dist2(*before, centre));
            // Not past the centre: the doubled offset keeps its sign on each axis or is zero.
            let (bx, by) = (
                centre.0 - 2 * i64::from(before.x.0),
                centre.1 - 2 * i64::from(before.y.0),
            );
            let (nx, ny) = (
                centre.0 - 2 * i64::from(now.x.0),
                centre.1 - 2 * i64::from(now.y.0),
            );
            assert!(bx * nx >= 0 && by * ny >= 0);
        }
        let mut again = start(3, &at);
        pull(&mut again);
        assert_eq!(places(&again), after, "the same twice");
        let mut swapped_at = at.clone();
        swapped_at.swap(1, 2);
        let mut swapped = start(3, &swapped_at);
        pull(&mut swapped);
        let mut back = places(&swapped);
        back.swap(1, 2);
        assert_eq!(back, after, "no slot bias");
    }
}

#[test]
fn a_dash_is_never_blocked() {
    let mut e = start(2, &[px(1000, 500), px(1840, 500)]);
    let (distance, ticks) = {
        let ev = &live_mut(&mut e).0.stage.player.evade;
        (ev.distance, ev.ticks)
    };
    let mut inputs = [Input::default(); 3];
    inputs[1].evade = true;
    inputs[1].move_x = 127;
    step_clean(&mut e, 1, &inputs);
    inputs[1] = Input::default();
    step_clean(&mut e, ticks - 1, &inputs);
    let now = places(&e);
    assert_eq!(
        now[1],
        Point {
            x: Fx(Fx::px(1840).0 + distance.0),
            y: Fx::px(500)
        }
    );
    assert!(now[0].x > Fx::px(1000), "the partner is pulled");
}

#[test]
fn a_dashing_player_beyond_range_is_not_moved_but_its_partner_is() {
    let mut e = start(2, &[px(1000, 500), px(1900, 500)]);
    live_mut(&mut e).1.players[slot(1)].dash = Some(super::state::DashState {
        dir: super::state::Dir { x: 256, y: 0 },
        done: 0,
        left: 5,
    });
    pull(&mut e);
    let now = places(&e);
    assert_eq!(now[1], px(1900, 500));
    assert!(now[0].x > Fx::px(1000));
}

#[test]
fn downed_and_out_players_are_not_tethered_or_measured() {
    for out in [false, true] {
        let mut e = start(3, &[px(1000, 500), px(3000, 500), px(1100, 500)]);
        down(&mut e, 1);
        if out {
            live_mut(&mut e).1.players[slot(1)].down_ticks = 0;
            live_mut(&mut e).1.players[slot(1)].out = true;
        }
        pull(&mut e);
        assert_eq!(
            places(&e),
            vec![px(1000, 500), px(3000, 500), px(1100, 500)]
        );
    }
}

#[test]
fn fewer_than_two_up_do_nothing() {
    let mut e = start(2, &[px(1000, 500), px(3000, 500)]);
    down(&mut e, 1);
    pull(&mut e);
    assert_eq!(places(&e), vec![px(1000, 500), px(3000, 500)]);
    let mut solo = start(1, &[px(1000, 500)]);
    pull(&mut solo);
    assert_eq!(places(&solo), vec![px(1000, 500)]);
}

#[test]
fn the_schedule_runs_the_tether_between_advance_runs_and_player_update() {
    let names: Vec<&str> = schedule::TICK.iter().map(|(n, _)| *n).collect();
    let at = names.iter().position(|n| *n == "tether").expect("tether");
    assert_eq!(names[at - 1], "advance_runs");
    assert_eq!(names[at + 1], "player_update");
}

#[test]
fn a_revive_ring_stays_and_the_only_up_player_is_not_pulled() {
    let mut e = start(2, &[px(1000, 500), px(1900, 500)]);
    down(&mut e, 1);
    step_clean(&mut e, 100, &[Input::default(); 3]);
    assert_eq!(places(&e), vec![px(1000, 500), px(1900, 500)]);
}
