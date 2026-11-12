//! Attack magnetism (ADR-CONT-3): a combo step whose phase names an `advance` slides the player
//! along the facing the step took, toward its current target, and no further than touching it.

use super::state::{PlayerIdx, State};
use crate::engine::{Engine, Run};
use crate::fixed::{Fx, Point};
use crate::input::Input;
use crate::limits::{MAX_ADVANCE_DISTANCE, MAX_ADVANCE_PER_TICK};
use crate::run_setup::RunSetup;
use std::sync::Arc;

const ME: PlayerIdx = PlayerIdx::SOLO;

fn st(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn dist(a: Point, b: Point) -> i64 {
    let (dx, dy) = (
        i64::from(a.x.0) - i64::from(b.x.0),
        i64::from(a.y.0) - i64::from(b.y.0),
    );
    i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).expect("fits")
}

/// One enemy settled where the leash puts it and the player `gap_px` to its left, with the
/// combo's first press made; the attack the press compiled names `advance` on every phase.
fn rig(gap_px: i32, advance_px: i32) -> Engine {
    let mut e = Engine::start_party(&RunSetup::default(), 1).expect("a run");
    while !st(&mut e).enemies.iter().any(|x| x.hp > 0 && !x.hidden()) {
        e.step(&[Input::default(); 3]);
    }
    {
        let s = st(&mut e);
        let keep = s
            .enemies
            .iter()
            .find(|x| x.hp > 0 && !x.hidden())
            .expect("one")
            .id;
        s.enemies.retain(|x| x.id == keep);
        let p = &mut s.players[ME];
        p.hp = 1_000_000;
        p.max_hp = 1_000_000;
    }
    e.step(&[Input::default(); 3]);
    let s = st(&mut e);
    let at = s.enemies[0].at;
    s.players[ME].at = Point {
        x: Fx(at.x.0.saturating_sub(Fx::px(gap_px).0)),
        y: at.y,
    };
    let press = Input {
        main: true,
        assist: true,
        ..Input::default()
    };
    e.step(&[press, Input::default(), Input::default()]);
    let Run::V2 { world, state } = e.run_mut() else {
        panic!("the live rules");
    };
    let attack = state
        .runs
        .iter()
        .find(|r| r.owner == state.players.id(ME) && r.slot == 0)
        .expect("the combo run")
        .attack;
    let w = Arc::make_mut(world);
    for p in &mut w.attacks[usize::from(attack.0)].phases {
        p.advance = Some(Fx::px(advance_px));
    }
    w.has_advance = true;
    e
}

/// The player's positions over the next `ticks` ticks with `assist` as given.
fn slide(e: &mut Engine, ticks: u32, assist: bool) -> Vec<Point> {
    let input = Input {
        assist,
        ..Input::default()
    };
    (0..ticks)
        .map(|_| {
            e.step(&[input; 3]);
            st(e).players[ME].at
        })
        .collect()
}

#[test]
fn the_player_slides_toward_the_target_a_bounded_share_a_tick() {
    let mut e = rig(150, MAX_ADVANCE_DISTANCE);
    let start = st(&mut e).players[ME].at;
    let path = slide(&mut e, 12, true);
    let cap = i64::from(Fx::px(MAX_ADVANCE_PER_TICK).0);
    let mut last = start;
    for at in &path {
        assert!(dist(last, *at) <= cap + 1, "no tick moves past the cap");
        last = *at;
    }
    assert!(
        path.last().expect("ticks").x.0 > start.x.0,
        "toward the target, on the facing"
    );
}

#[test]
fn the_slide_stops_where_the_circles_touch() {
    let mut e = rig(30, MAX_ADVANCE_DISTANCE);
    let radius = e_radius(&mut e);
    slide(&mut e, 30, true);
    let s = st(&mut e);
    let gap = dist(s.players[ME].at, s.enemies[0].at);
    assert!(
        gap >= radius - 256,
        "never into the target: {gap} against {radius}"
    );
}

fn e_radius(e: &mut Engine) -> i64 {
    let Run::V2 { world, state } = e.run_mut() else {
        panic!("the live rules");
    };
    let def = world.defs.agent(state.enemies[0].def).radius.0;
    i64::from(def) + i64::from(world.tuning.player_radius)
}

#[test]
fn no_target_no_slide() {
    let mut e = rig(150, MAX_ADVANCE_DISTANCE);
    let start = st(&mut e).players[ME].at;
    let path = slide(&mut e, 12, false);
    assert!(
        path.iter().all(|at| *at == start),
        "assist off: the player stays"
    );
}
