//! A session with no remote peer, and the config checks.

mod support;

use stella_rain_core::input::Input;
use stella_rain_net::session::Wait;
use stella_rain_net::session::{
    Config, ConfigError, DEFAULT_DELAY, DEFAULT_WINDOW, MAX_DELAY, MAX_WINDOW, Session,
};
use stella_rain_net::wire::WireInput;
use support::{CountingSave, Script, engine, frame_of, script, slot, straight};

fn config(delay: u32, window: u32) -> Config {
    Config {
        delay,
        window,
        ..Config::new(slot(0))
    }
}

#[test]
fn the_config_is_checked_at_the_start() {
    let mut stepped = engine(1);
    stepped.step(&[Input::default(); 3]);
    assert_eq!(
        Session::new(stepped, Config::new(slot(0))).err(),
        Some(ConfigError::NotAtStart)
    );
    let local_third = Config::new(slot(2));
    assert_eq!(
        Session::new(engine(2), local_third).err(),
        Some(ConfigError::LocalAbsent)
    );
    assert_eq!(
        Session::new(engine(1), config(2, MAX_WINDOW + 1)).err(),
        Some(ConfigError::WindowTooLarge)
    );
    assert_eq!(
        Session::new(engine(1), config(MAX_DELAY + 1, 4)).err(),
        Some(ConfigError::DelayTooLarge)
    );
    for (delay, window) in [(0, 0), (MAX_DELAY, MAX_WINDOW), (2, 4)] {
        assert!(Session::new(engine(1), config(delay, window)).is_ok());
    }
}

#[test]
fn the_default_config_is_delay_two_window_four() {
    let c = Config::new(slot(0));
    assert_eq!((c.delay, c.window), (2, 4));
    assert_eq!((DEFAULT_DELAY, DEFAULT_WINDOW), (2, 4));
    assert_eq!(c.local, slot(0));
}

#[test]
fn errors_display() {
    for e in [
        ConfigError::NotAtStart,
        ConfigError::LocalAbsent,
        ConfigError::WindowTooLarge,
        ConfigError::DelayTooLarge,
    ] {
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn a_session_of_one_matches_a_straight_run_at_every_tick() {
    for delay in [0u32, 3] {
        let s = script(1, 300 + delay, delay, None);
        let oracle = straight(&s, 1);
        let mut session = Session::new(engine(1), config(delay, 4)).unwrap();
        for k in 0..300u32 {
            let input = s.eff[(k + delay) as usize][0];
            let out = session.advance(&input).unwrap();
            assert_eq!((out.rolled_back, out.waiting), (0, None), "call {k}");
            assert_eq!(session.tick(), k + 1);
            assert_eq!(session.tick(), session.engine().tick());
            assert_eq!(session.confirmed(), session.tick());
            assert_eq!(
                session.hash_after(session.tick()),
                Some(oracle[(k + 1) as usize]),
                "delay {delay}, call {k}"
            );
        }
    }
}

#[test]
fn the_first_delay_ticks_are_neutral_and_a_call_lands_delay_ticks_later() {
    let given = |k: u32| Input {
        main: true,
        move_x: 100,
        move_y: i8::try_from(k).unwrap(),
        ..Input::default()
    };
    let mut session = Session::new(engine(1), config(3, 4)).unwrap();
    for k in 0..4 {
        session.advance(&given(k)).unwrap();
    }
    // Ticks 0 to 2 were neutral; tick 3 stepped the input of call 0.
    let mut by_hand = engine(1);
    for _ in 0..3 {
        by_hand.step(&[Input::default(); 3]);
    }
    assert_eq!(session.hash_after(3), Some(by_hand.state_hash()));
    by_hand.step(&[given(0), Input::default(), Input::default()]);
    assert_eq!(session.hash_after(4), Some(by_hand.state_hash()));
}

#[test]
fn a_stick_axis_of_minus_128_steps_as_minus_127() {
    let run = |x: i8| {
        let mut session = Session::new(engine(1), config(0, 4)).unwrap();
        for k in 0..40u8 {
            let input = Input {
                move_x: x,
                move_y: i8::try_from(k).unwrap(),
                main: true,
                ..Input::default()
            };
            session.advance(&input).unwrap();
        }
        session.hash_after(40).unwrap()
    };
    assert_eq!(run(i8::MIN), run(-127));
    assert_ne!(run(-127), run(100), "the stick matters to the hash");
}

#[test]
fn local_frame_starts_where_asked_and_drops_what_is_confirmed() {
    let mut session = Session::new(engine(1), config(2, 4)).unwrap();
    for _ in 0..10 {
        session.advance(&Input::default()).unwrap();
    }
    // No remote peer: everything before the confirmed tick (10) is dropped.
    assert!(session.local_frame(0).is_none());
    let frame = session.local_frame(10).unwrap();
    assert_eq!(
        (frame.slot, frame.start_tick, frame.inputs.len()),
        (slot(0), 10, 2)
    );
    assert!(session.local_frame(12).is_none());
}

#[test]
fn a_session_saves_once_per_stepped_tick_and_never_loads_without_a_peer() {
    let counting = CountingSave::default();
    let mut session = Session::with_save(engine(1), config(2, 4), counting.clone()).unwrap();
    for _ in 0..50 {
        session.advance(&Input::default()).unwrap();
    }
    let log = counting.0.borrow();
    assert_eq!((log.saves, log.loads), (50, 0));
    assert!(log.max_held <= 5);
}

fn first_action(s: &Script, who: usize) -> usize {
    s.eff
        .iter()
        .position(|row| row[who].action.is_some())
        .expect("the script holds an action")
}

#[test]
fn a_predicted_remote_input_repeats_without_its_action_and_is_corrected_in_one_rollback() {
    let s = script(2, 400, 2, None);
    let oracle = straight(&s, 2);
    let m = first_action(&s, 1);
    assert!(m + 10 < 400);
    let m32 = u32::try_from(m).unwrap();
    let mut a = Session::new(engine(2), Config::new(slot(0))).unwrap();
    let call = |a: &mut Session, k: usize| {
        a.ack(slot(1), a.tick() + 2).unwrap();
        a.advance(&s.eff[k + 2][0]).unwrap()
    };
    // B's input for tick r is here before A's call r, up to its first action.
    for r in 0..=m {
        let r32 = u32::try_from(r).unwrap();
        a.receive_inputs(slot(1), &frame_of(&s, 1, r32, r32))
            .unwrap();
        assert_eq!(call(&mut a, r).waiting, None);
    }
    // Then nothing: four ticks are predicted, the fifth call waits.
    for r in m + 1..=m + 4 {
        assert_eq!(call(&mut a, r).waiting, None);
    }
    let held = a.tick();
    let out = call(&mut a, m + 5);
    assert_eq!((out.waiting, out.rolled_back), (Some(Wait::Window), 0));
    assert_eq!(a.tick(), held);
    // B's input at the action tick repeated, its action never.
    let mut predicted = s.clone();
    for row in &mut predicted.eff[m + 1..] {
        row[1] = Input {
            action: None,
            ..s.eff[m][1]
        };
    }
    assert_eq!(
        a.engine().state_hash(),
        straight(&predicted, 2)[m + 5],
        "A predicted B's last input without its action"
    );
    // The real inputs arrive in one frame: A re-simulates from the first tick that differs.
    let guess = WireInput::from_input(&s.eff[m][1]);
    let guess = WireInput {
        action: None,
        ..guess
    };
    let first_diff = (m + 1..=m + 4)
        .find(|t| WireInput::from_input(&s.eff[*t][1]) != guess)
        .map_or(0, |t| (m + 5) - t);
    a.receive_inputs(slot(1), &frame_of(&s, 1, m32 + 1, m32 + 4))
        .unwrap();
    let out = call(&mut a, m + 5);
    assert_eq!(out.waiting, None);
    assert_eq!(usize::try_from(out.rolled_back).unwrap(), first_diff);
    assert_eq!(a.hash_after(m32 + 5), Some(oracle[m + 5]));
}

#[test]
fn the_window_bounds_the_predicted_ticks_and_a_wait_changes_nothing() {
    let s = script(2, 100, 2, None);
    for window in [0u32, 1, 4, 16] {
        let counting = CountingSave::default();
        let config = Config {
            window,
            ..Config::new(slot(0))
        };
        let mut a = Session::with_save(engine(2), config, counting.clone()).unwrap();
        let mut stepped = 0;
        while a.advance(&s.eff[stepped + 2][0]).unwrap().waiting.is_none() {
            stepped += 1;
            assert!(stepped <= 20);
        }
        assert_eq!(stepped, window as usize, "window {window}");
        let (tick, hash, frame) = (a.tick(), a.engine().state_hash(), a.local_frame(0));
        let out = a.advance(&s.eff[stepped + 2][0]).unwrap();
        assert_eq!((out.waiting, out.rolled_back), (Some(Wait::Window), 0));
        assert_eq!(
            (a.tick(), a.engine().state_hash(), a.local_frame(0)),
            (tick, hash, frame)
        );
        // The missing inputs let the next call step.
        a.receive_inputs(slot(1), &frame_of(&s, 1, 0, window + 3))
            .unwrap();
        assert_eq!(a.advance(&s.eff[stepped + 2][0]).unwrap().waiting, None);
        assert!(
            counting.0.borrow().max_held <= window as usize + 1,
            "window {window}"
        );
    }
}
