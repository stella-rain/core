//! The perfect evade over whole ticks (ADR-SIM-8): the window at the press, the slow of the
//! enemies in the evader's reach and no one else's, the token a slowed ranged run frees, and a
//! run that never names the rule.

use super::state::{PlayerIdx, State};
use crate::bench_fixtures::worst_case;
use crate::engine::{Engine, Run};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use crate::rules::PerfectEvade;
use std::sync::Arc;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

const RULE: PerfectEvade = PerfectEvade {
    window_ticks: 10,
    slow_ticks: 60,
    cooldown_ticks: 300,
    reach: 200 * 256,
    slow_every: 2,
};

/// A warmed-up party of `n` that names the rule, every player hardy enough to outlast a minute.
fn rig(n: u8, rule: Option<PerfectEvade>) -> Engine {
    let mut e = worst_case::warmed_up().into_party(n);
    let Run::V2 { world, state } = e.run_mut() else {
        panic!("the live rules");
    };
    Arc::make_mut(world).tuning.perfect_evade = rule;
    for i in 0..n {
        let p = &mut state.players[slot(i)];
        p.hp = 1_000_000;
        p.max_hp = 1_000_000;
        p.invulnerable = 0;
    }
    e
}

fn st(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn idle(e: &mut Engine, ticks: u32) {
    for _ in 0..ticks {
        e.step(&[Input::default(); 3]);
    }
}

fn combat(s: &State, i: u8) -> (u16, u16, u16) {
    let c = &s.players[slot(i)].combat;
    (c.perfect_window, c.perfect_slow, c.perfect_cooldown)
}

/// The first two living enemies that do not lie hidden, by id.
fn two_enemies(s: &State) -> (EntityId, EntityId) {
    let mut live = s.enemies.iter().filter(|e| e.hp > 0 && !e.hidden());
    (live.next().expect("one").id, live.next().expect("two").id)
}

fn put(s: &mut State, id: EntityId, at: Point) {
    s.enemies.iter_mut().find(|e| e.id == id).expect("it").at = at;
}

fn age(s: &State, id: EntityId) -> u32 {
    s.enemies.iter().find(|e| e.id == id).expect("alive").age
}

fn px(x: i32, y: i32) -> Point {
    Point {
        x: Fx::px(x),
        y: Fx::px(y),
    }
}

#[test]
fn an_effective_press_opens_the_window_and_it_counts_down() {
    let mut e = rig(1, Some(RULE));
    let press = Input {
        evade: true,
        ..Input::default()
    };
    e.step(&[press, Input::default(), Input::default()]);
    assert_eq!(combat(st(&mut e), 0).0, 10);
    idle(&mut e, 1);
    assert_eq!(combat(st(&mut e), 0).0, 9);
    idle(&mut e, 9);
    assert_eq!(combat(st(&mut e), 0).0, 0);
}

#[test]
fn a_run_that_names_no_rule_never_sets_a_timer() {
    let mut e = rig(1, None);
    let press = Input {
        evade: true,
        ..Input::default()
    };
    for t in 0..100 {
        let input = if t % 2 == 0 { press } else { Input::default() };
        e.step(&[input, Input::default(), Input::default()]);
        assert_eq!(combat(st(&mut e), 0), (0, 0, 0));
        assert!(st(&mut e).scratch.slowed.is_empty());
    }
}

#[test]
fn an_enemy_in_the_evaders_reach_advances_every_other_tick_and_the_rest_do_not() {
    let run = |slow: u16| {
        // A small reach: the leash lands the far enemy about 9 px from where the near one stands.
        let rule = PerfectEvade {
            reach: 6 * 256,
            ..RULE
        };
        let mut e = rig(3, Some(rule));
        let (near, far) = two_enemies(st(&mut e));
        let s = st(&mut e);
        // Alone, so that the push-apart of a crowd does not carry them out of place.
        s.enemies.retain(|x| x.id == near || x.id == far);
        // One tick for the leash to settle them, then arrange them where they stand.
        idle(&mut e, 1);
        let s = st(&mut e);
        let home = s.enemies.iter().find(|x| x.id == near).expect("near").at;
        s.players[slot(0)].at = home;
        let away = Point {
            x: home.x,
            y: home.y.saturating_add(Fx::px(60)),
        };
        put(s, far, away);
        s.players[slot(1)].at = away;
        s.players[slot(2)].at = away;
        s.players[slot(0)].combat.perfect_slow = slow;
        s.players[slot(0)].combat.perfect_cooldown = if slow > 0 { 300 } else { 0 };
        let (a, b) = (age(s, near), age(s, far));
        idle(&mut e, 60);
        let s = st(&mut e);
        (age(s, near) - a, age(s, far) - b, s.players[slot(2)].combat)
    };
    let (slowed_near, slowed_far, slot2) = run(60);
    let (control_near, control_far, control_slot2) = run(0);
    assert_eq!(control_near, 60);
    assert_eq!(slowed_near, 30, "half the ticks in 60");
    assert_eq!(
        slowed_far, control_far,
        "out of every slowing player's reach"
    );
    assert_eq!(slot2, control_slot2, "the others' timers are their own");
}

#[test]
fn a_slow_the_party_has_not_started_by_a_downed_player_slows_nothing() {
    let mut e = rig(2, Some(RULE));
    let (near, _) = two_enemies(st(&mut e));
    let s = st(&mut e);
    let home = s.players[slot(0)].at;
    put(s, near, home);
    s.players[slot(0)].combat.perfect_slow = 60;
    s.players[slot(0)].combat.perfect_cooldown = 300;
    s.players[slot(0)].hp = 0;
    s.players[slot(0)].down_ticks = 0;
    let before = age(s, near);
    idle(&mut e, 10);
    assert_eq!(age(st(&mut e), near) - before, 10);
    assert_eq!(combat(st(&mut e), 0).2, 290, "its cooldown still counts");
}

#[test]
fn a_slowed_ranged_run_frees_its_token_and_runs_on() {
    let mut e = rig(3, Some(RULE));
    let s = st(&mut e);
    let (near, far) = {
        let first = s.runs[0].owner;
        let other = s
            .runs
            .iter()
            .map(|r| r.owner)
            .find(|o| *o != first)
            .expect("two attackers");
        (first, other)
    };
    let home = s.players[slot(0)].at;
    for r in &mut s.runs {
        if r.owner == near || r.owner == far {
            r.ranged = true;
            r.phase = 0;
            r.tick = 0;
        }
    }
    // Both foes in a spot of their own: the first in slot 0's reach, the other out of it.
    for (id, at) in [(near, home), (far, px(5000, 5000))] {
        if let Some(i) = s.enemies.iter().position(|e| e.id == id) {
            s.enemies[i].at = at;
        }
    }
    s.players[slot(0)].combat.perfect_slow = 60;
    s.players[slot(0)].combat.perfect_cooldown = 300;
    let cooling = s.ranged_cooling.len();
    idle(&mut e, 1);
    let s = st(&mut e);
    let mine: Vec<_> = s.runs.iter().filter(|r| r.owner == near).collect();
    assert!(!mine.is_empty(), "the run is still there");
    assert!(mine.iter().all(|r| !r.ranged), "its token is free");
    assert_eq!(s.ranged_cooling.len(), cooling, "no cooldown began");
}

#[test]
fn the_slowed_run_is_the_same_run_whether_it_is_cloned_mid_slow_or_not() {
    let mut e = rig(3, Some(RULE));
    let (near, _) = two_enemies(st(&mut e));
    let s = st(&mut e);
    let home = s.players[slot(0)].at;
    put(s, near, home);
    s.players[slot(0)].combat.perfect_slow = 60;
    s.players[slot(0)].combat.perfect_cooldown = 300;
    idle(&mut e, 20);
    let mut resumed = e.clone();
    for t in 0..40 {
        e.step(&[Input::default(); 3]);
        resumed.step(&[Input::default(); 3]);
        assert_eq!(
            e.state_hash(),
            resumed.state_hash(),
            "tick {t} after the clone"
        );
    }
}
