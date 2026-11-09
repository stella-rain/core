//! A session with no remote peer, and the config checks.

mod support;

use stella_rain_core::input::Input;
use stella_rain_net::session::{
    Config, ConfigError, DEFAULT_DELAY, DEFAULT_WINDOW, MAX_DELAY, MAX_WINDOW, Session,
};
use support::{CountingSave, engine, script, slot, straight};

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
