//! Held steps (ADR-084 point 6), worked out by hand. The main character stands at (46080,
//! 147456) facing up the field, with the far boss in the distance. The held step's own attack
//! spawns one orb at speed 0 on tick 1 (its first tick) and has its recovery from tick 3, so
//! the hold begins on tick 3 if `main` is still held then. The loop spawns one orb on each
//! of its starts: on the hold's first tick, then every 3 ticks while held. A release spawns one
//! orb on its first tick. Orbs at speed 0 stay where they spawn, so the orbs alive
//! count how many spawned.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::{DomainEvent, Press};
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::HoldView;
use stella_rain_core::stage::Stage;

/// One zone `speed` ahead along the aim, spawned by an active phase of one tick; it stays for
/// 200 ticks.
fn orb(speed: i32) -> Value {
    json!({ "type": "zone", "shape": { "type": "circle", "radius": 512 },
            "offset": { "forward": speed }, "direction": { "type": "aimed", "offset": 0 },
            "wind_up_ticks": 0, "active_ticks": 200 })
}

/// The held step's own attack: an orb at speed 0 on its first tick, then its recovery.
fn own() -> Value {
    json!({ "inline": [
        { "kind": "active", "ticks": 2, "spawn": [orb(0)] },
        { "kind": "recovery", "ticks": 4 }
    ] })
}

/// The loop: three ticks of an orb at `speed` on its first tick.
fn spin(speed: i32) -> Value {
    json!({ "inline": [{ "kind": "active", "ticks": 3, "spawn": [orb(speed)] }] })
}

/// The release: an orb at `speed` on its first tick, then a recovery of six.
fn slam(speed: i32) -> Value {
    json!({ "inline": [
        { "kind": "active", "ticks": 1, "spawn": [orb(speed)] },
        { "kind": "recovery", "ticks": 6 }
    ] })
}

/// A held step that holds for `max_ticks`, its loop and release at the given speeds, with
/// `next` for the presses that follow it.
fn held_step(max_ticks: u32, loop_speed: i32, release_speed: i32, next: Value) -> Value {
    json!({
        "attack": own(),
        "hold": { "loop": spin(loop_speed), "release": slam(release_speed), "max_ticks": max_ticks },
        "next": next
    })
}

/// The Main weapon whose one step is `step`, reached by `main`.
fn stage(step: Value) -> Stage {
    armed(
        stage_with(18_000, Some(far_boss()), vec![]),
        weapon_combo(json!({ "main": step })),
    )
}

/// Everything one run showed, tick by tick.
#[derive(Default)]
struct Trace {
    events: Vec<(u32, DomainEvent)>,
    /// The friendly orbs each tick spawned: `(tick, how many)`.
    spawns: Vec<(u32, usize)>,
    /// The hold in the snapshot after each tick, the first at index 0.
    holds: Vec<Option<HoldView>>,
}

impl Trace {
    fn hold_at(&self, tick: u32) -> Option<HoldView> {
        self.holds[tick as usize - 1]
    }

    fn releases(&self) -> Vec<(u32, u32)> {
        self.events
            .iter()
            .filter_map(|(t, ev)| match ev {
                DomainEvent::HoldReleased { ticks } => Some((*t, *ticks)),
                _ => None,
            })
            .collect()
    }

    fn breaks(&self) -> Vec<u32> {
        ticks_of(&self.events, |ev| *ev == DomainEvent::HoldBroken)
    }

    fn spawned_on(&self) -> Vec<u32> {
        self.spawns.iter().map(|(t, _)| *t).collect()
    }
}

/// Runs `inputs` one tick each on a fresh engine of `stage`, returning the engine and the trace.
fn run(stage: &Stage, inputs: impl IntoIterator<Item = Input>) -> (Engine, Trace) {
    let mut e = engine(stage);
    let mut trace = Trace::default();
    let mut alive = 0;
    for input in inputs {
        e.step(input);
        let tick = e.tick();
        let snap = e.snapshot();
        let now = snap.zones.iter().filter(|z| z.friendly).count();
        if now > alive {
            trace.spawns.push((tick, now - alive));
        }
        alive = now;
        trace.holds.push(snap.players[0].hold);
        trace
            .events
            .extend(e.events().iter().map(|ev| (tick, ev.clone())));
    }
    (e, trace)
}

/// `main` pressed on the ticks where `held` says, and no other button.
fn pressing(main: impl Fn(u32) -> bool) -> impl Iterator<Item = Input> {
    (1..).map(move |t| Input {
        main: main(t),
        ..idle()
    })
}

fn friendly(e: &Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .zones
        .iter()
        .filter(|z| z.friendly)
        .map(|z| (z.at.x.0 - START_X, z.at.y.0 - START_Y))
        .collect()
}

#[test]
fn past_its_own_attack_the_loop_spins_while_held_and_let_go_releases() {
    let (_, t) = run(
        &stage(held_step(180, 0, 0, json!({}))),
        pressing(|t| t <= 7).take(12),
    );
    // The own attack on 1; the hold on 3 and its loop again on 6; the release on 8.
    assert_eq!(t.spawned_on(), [1, 3, 6, 8]);
    assert_eq!(t.releases(), [(8, 4)]);
    assert_eq!(t.hold_at(2), None, "the own attack has no hold yet");
    assert_eq!(
        t.hold_at(3),
        Some(HoldView {
            held: 0,
            max_ticks: 180
        })
    );
    assert_eq!(t.hold_at(4).map(|h| h.held), Some(1));
    assert_eq!(t.hold_at(8), None, "the release ends the hold");
}

#[test]
fn reaching_max_ticks_releases_by_itself_after_the_last_loop() {
    let (_, t) = run(
        &stage(held_step(9, 0, 0, json!({}))),
        pressing(|_| true).take(15),
    );
    // Held 9 ticks from 3: the loop on 9 runs to 12, when the release comes.
    assert_eq!(t.spawned_on(), [1, 3, 6, 9, 12]);
    assert_eq!(t.releases(), [(12, 9)]);
    assert_eq!(t.hold_at(11).map(|h| h.held), Some(8));
}

#[test]
fn letting_go_before_the_recovery_is_a_plain_step() {
    // Let go on tick 3, the first tick of the recovery: no hold, and the recovery plays.
    let (_, t) = run(
        &stage(held_step(180, 0, 0, json!({}))),
        pressing(|t| t <= 2).take(10),
    );
    assert_eq!(t.spawned_on(), [1]);
    assert!(t.releases().is_empty());
    assert!(t.holds.iter().all(Option::is_none));
}

#[test]
fn a_press_during_the_hold_is_taken_after_the_release() {
    let next = json!({ "smash": { "attack": own() } });
    let inputs = (1..).take(12).map(|t| Input {
        main: t <= 7,
        smash: t == 5,
        ..idle()
    });
    let (_, t) = run(&stage(held_step(180, 0, 0, next)), inputs);
    // The press on 5 waits; the release on 8 starts its recovery on 9, which the press takes.
    let subs = ticks_of(&t.events, |ev| {
        matches!(
            ev,
            DomainEvent::ComboStep {
                press: Press::Smash,
                ..
            }
        )
    });
    assert_eq!(subs, [9]);
    assert_eq!(t.releases(), [(8, 4)]);
}

#[test]
fn an_evade_breaks_the_hold_with_no_release_and_the_combo_starts_again() {
    let inputs = (1..=10).map(|t| Input {
        main: true,
        evade: t == 5,
        ..idle()
    });
    let (e, t) = run(&stage(held_step(180, 0, 0, json!({}))), inputs);
    assert_eq!(t.breaks(), [5]);
    assert_eq!(
        ticks_of(&t.events, |ev| *ev == DomainEvent::ComboReset),
        [5]
    );
    assert!(t.releases().is_empty(), "no release");
    assert_eq!(t.spawned_on(), [1, 3], "the loop on 3 is the last attack");
    assert_eq!(e.snapshot().players[0].hold, None);
}

/// A stage where an enemy 40 pixels above places a zone on the main character on tick 1,
/// hurting it on tick 9; `breaks` marks the zone's attack as one that breaks charges.
fn shot_stage(breaks: bool) -> Stage {
    let attack = json!({ "inline": [{ "kind": "active", "ticks": 1, "breaks_charge": breaks,
        "spawn": [{ "type": "zone", "shape": { "type": "circle", "radius": 3_072 },
                    "anchor": "target", "wind_up_ticks": 8, "active_ticks": 2 }] }] });
    let shooter = enemy(
        1000,
        hold(START_X, START_Y - 10_240),
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack", "attack": attack }),
            600
        )]),
    );
    armed(
        stage_with(
            18_000,
            Some(far_boss()),
            vec![wave(1, 1, 0, shooter, START_X, START_Y - 10_240)],
        ),
        weapon_combo(json!({ "main": held_step(180, 0, 0, json!({})) })),
    )
}

#[test]
fn a_hit_from_a_powerful_attack_breaks_the_hold_and_others_do_not() {
    let (e, t) = run(&shot_stage(true), pressing(|_| true).take(15));
    let hit = ticks_of(&t.events, |ev| matches!(ev, DomainEvent::PlayerHit { .. }));
    assert_eq!(hit, [9]);
    assert_eq!(t.breaks(), [9]);
    assert!(
        t.spawned_on().iter().all(|&s| s <= 9),
        "the loop ends with the hold"
    );
    assert_eq!(e.snapshot().players[0].hold, None);

    let (e, t) = run(&shot_stage(false), pressing(|_| true).take(15));
    assert!(t.breaks().is_empty());
    assert!(e.snapshot().players[0].hold.is_some());
}

#[test]
fn each_loop_faces_where_the_main_character_moves_when_it_starts() {
    let inputs = (1..=8).map(|t| Input {
        main: true,
        move_x: if t == 6 { 127 } else { 0 },
        ..idle()
    });
    let (e, _) = run(&stage(held_step(180, 1024, 0, json!({}))), inputs);
    // Orbs in spawn order: the own attack's (still), the loop on 3 facing up, the loop on 6
    // facing right after the stick turned on tick 6.
    let shots = friendly(&e);
    assert_eq!(shots.len(), 3);
    assert_eq!(shots[1].0, 0, "the first loop faces up");
    assert!(
        shots[2].0 > 0 && shots[2].1 == 0,
        "the second loop faces right"
    );
}

/// The main character's x after each of six ticks of the stick walking right, with `main`
/// pressed on every tick if `main`.
fn walk(main: bool) -> Vec<i32> {
    let mut e = engine(&stage(held_step(180, 0, 0, json!({}))));
    (0..6)
        .map(|_| {
            e.step(Input {
                main,
                move_x: 127,
                ..idle()
            });
            player_at(&e).0
        })
        .collect()
}

#[test]
fn holding_moves_the_main_character_at_the_hold_s_percentage() {
    // Ticks 4 to 5 are held with `move_pct` 50; a run with no attack walks at full speed.
    let mut step = held_step(180, 0, 0, json!({}));
    step["hold"]["move_pct"] = json!(50);
    let mut e = engine(&stage(step));
    let held: Vec<i32> = (0..6)
        .map(|_| {
            e.step(Input {
                main: true,
                move_x: 127,
                ..idle()
            });
            player_at(&e).0
        })
        .collect();
    let free = walk(false);
    let free_step = free[4] - free[3];
    assert!(free_step > 0);
    assert_eq!(held[4] - held[3], free_step / 2);
}

#[test]
fn the_giant_hammer_s_aaa_holds_while_a_stays_held() {
    // A on tick 1, A buffered on 3 (taken when A's recovery begins on 11), A buffered on 13
    // (taken when AA's recovery begins on 22): AAA's recovery begins on 36, still held, so it
    // holds from there on.
    let stage = armed(
        stage_with(18_000, Some(far_boss()), vec![]),
        json!({ "preset": { "id": "giant_hammer" } }),
    );
    let inputs = (1..=40).map(|t| Input {
        main: t == 1 || (t >= 3 && t != 12),
        ..idle()
    });
    let (_, t) = run(&stage, inputs);
    assert_eq!(t.hold_at(35), None, "AAA's own attack is still running");
    assert_eq!(
        t.hold_at(37),
        Some(HoldView {
            held: 1,
            max_ticks: 180
        })
    );
    assert!(t.releases().is_empty(), "still held");
}
