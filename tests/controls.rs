//! The controls of ADR-071 as far as they run so far: the input record's movement, the
//! persistent facing (ADR-069), the Main weapon's attack on a press of `main`, along the facing,
//! and the evade. Every number here is worked out by hand from the rules' constants.

mod common;

use common::*;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::Facing;
use stella_rain_core::trig::SCALE;

fn evading(move_x: i8, move_y: i8) -> Input {
    Input {
        move_x,
        move_y,
        evade: true,
        ..Input::default()
    }
}

fn facing(e: &stella_rain_core::engine::Engine) -> (i32, i32) {
    let Facing { x, y } = e.snapshot().player.facing;
    (x, y)
}

/// Friendly bullets alive, with their velocities.
fn shots(e: &stella_rain_core::engine::Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .bullets
        .iter()
        .filter(|b| b.friendly)
        .map(|b| (b.vx.0, b.vy.0))
        .collect()
}

// --- Movement --------------------------------------------------------------------------------

#[test]
fn a_full_stick_moves_at_the_speed_cap_and_less_moves_in_proportion() {
    let mut e = running(vec![]);
    e.step(moving(127, 0));
    assert_eq!(player_at(&e), (START_X + SPEED, START_Y));
    // 64 / 127 of the cap: 64 × 768 / 127 = 387.02, truncated.
    e.step(moving(64, 0));
    assert_eq!(player_at(&e), (START_X + SPEED + 387, START_Y));
    e.step(moving(0, -1));
    assert_eq!(player_at(&e), (START_X + SPEED + 387, START_Y - 6));
}

#[test]
fn a_diagonal_is_no_faster_than_a_straight_line() {
    let mut e = running(vec![]);
    // Length √(127² + 127²) = 179.6, rounded up to 180: 127 × 768 / 180 = 541.9 on each axis.
    e.step(moving(127, 127));
    let (x, y) = player_at(&e);
    assert_eq!((x, y), (START_X + 541, START_Y + 541));
    let (dx, dy) = (x - START_X, y - START_Y);
    assert!(dx * dx + dy * dy <= SPEED * SPEED);
}

#[test]
fn minus_128_moves_like_minus_127() {
    let mut a = running(vec![]);
    let mut b = running(vec![]);
    a.step(moving(-128, -128));
    b.step(moving(-127, -127));
    assert_eq!(player_at(&a), player_at(&b));
    a.step(moving(-128, 0));
    b.step(moving(-127, 0));
    assert_eq!(player_at(&a), player_at(&b));
}

// --- Facing ----------------------------------------------------------------------------------

#[test]
fn the_facing_starts_up_the_field_and_follows_the_last_movement() {
    let mut e = running(vec![]);
    assert_eq!(facing(&e), (0, -SCALE));
    e.step(moving(127, 0));
    assert_eq!(facing(&e), (SCALE, 0));
    // Standing still keeps it.
    e.step(idle());
    e.step(idle());
    assert_eq!(facing(&e), (SCALE, 0));
    // (-3, 4) has length 5: (-0.6, 0.8) × 65,536, truncated.
    e.step(moving(-3, 4));
    assert_eq!(facing(&e), (-39_321, 52_428));
    // A tiny stick still turns it fully: 65,536² / ⌊√2 × 65,536⌋ = 46,341.03.
    e.step(moving(1, 1));
    assert_eq!(facing(&e), (46_341, 46_341));
}

// --- The main shot ---------------------------------------------------------------------------

#[test]
fn the_main_shot_attacks_on_each_press_of_main() {
    // The test stage's Main weapon: one bullet at 8 pixels a tick, a one-tick step. Held, it
    // fires once; let go and pressed again, it fires again.
    let mut e = running(vec![]);
    e.step(idle());
    assert!(shots(&e).is_empty(), "nothing without a press");
    for _ in 0..5 {
        e.step(pressing_main());
    }
    assert_eq!(shots(&e), [(0, -2048)]);
    e.step(idle());
    e.step(pressing_main());
    assert_eq!(shots(&e).len(), 2);
}

#[test]
fn the_main_shot_goes_along_the_facing() {
    let mut e = running(vec![]);
    // Turning left on the tick of the press.
    e.step(Input {
        main: true,
        ..moving(-127, 0)
    });
    assert_eq!(shots(&e), [(-2048, 0)]);
}

// --- Evade -----------------------------------------------------------------------------------

#[test]
fn an_evade_dashes_its_distance_over_its_ticks_and_ignores_the_stick_meanwhile() {
    let mut e = running(vec![]);
    let step = EVADE_DISTANCE / EVADE_TICKS as i32;
    e.step(evading(127, 0));
    assert_eq!(player_at(&e), (START_X + step, START_Y));
    for k in 2..=EVADE_TICKS as i32 {
        e.step(moving(-127, 0));
        assert_eq!(
            player_at(&e),
            (START_X + k * step, START_Y),
            "dash tick {k}"
        );
    }
    e.step(moving(-127, 0));
    assert_eq!(player_at(&e), (START_X + EVADE_DISTANCE - SPEED, START_Y));
}

#[test]
fn an_evade_without_movement_goes_along_the_facing() {
    let mut e = running(vec![]);
    e.step(evading(0, 0));
    for _ in 1..EVADE_TICKS {
        e.step(idle());
    }
    assert_eq!(player_at(&e), (START_X, START_Y - EVADE_DISTANCE));
}

#[test]
fn an_evade_is_invulnerable_from_its_first_tick() {
    let mut e = running(vec![]);
    assert_eq!(e.snapshot().player.invulnerable, 0);
    e.step(evading(127, 0));
    assert_eq!(e.snapshot().player.invulnerable, EVADE_INVULNERABLE);
    e.step(idle());
    assert_eq!(e.snapshot().player.invulnerable, EVADE_INVULNERABLE - 1);
}

#[test]
fn the_evade_cooldown_counts_from_its_start() {
    let mut e = running(vec![]);
    let mut evaded = Vec::new();
    // Pressed on tick 1, then on every other tick: the press on tick 41 is the first ready one.
    for tick in 1..=EVADE_COOLDOWN + 3 {
        e.step(if tick % 2 == 1 {
            evading(127, 0)
        } else {
            idle()
        });
        if e.events().contains(&DomainEvent::Evaded) {
            evaded.push(tick);
        }
    }
    assert_eq!(evaded, [1, EVADE_COOLDOWN + 1]);
}

#[test]
fn a_held_button_presses_once() {
    // ADR-071: buttons are sent held, and only a button held now and not on the tick before is a
    // press, so holding the evade through its cooldown never evades again.
    let mut e = running(vec![]);
    let seen = run_until(&mut e, evading(127, 0), EVADE_COOLDOWN * 3, |_| false);
    assert_eq!(ticks_of(&seen, |ev| matches!(ev, DomainEvent::Evaded)), [1]);
    e.step(idle());
    let seen = run_until(&mut e, evading(127, 0), 1, |_| false);
    assert_eq!(
        ticks_of(&seen, |ev| matches!(ev, DomainEvent::Evaded)).len(),
        1
    );
}

#[test]
fn the_snapshot_shows_the_evade() {
    let mut e = running(vec![]);
    let view = e.snapshot().player.evade;
    assert_eq!(
        (view.ready, view.recovering, view.dash_left),
        (1, vec![], 0)
    );
    e.step(evading(127, 0));
    let view = e.snapshot().player.evade;
    assert_eq!(
        (view.ready, view.recovering, view.dash_left),
        (0, vec![EVADE_COOLDOWN], EVADE_TICKS - 1)
    );
}
