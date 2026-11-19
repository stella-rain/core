//! `Session::restart`: the session goes back to tick 0 in place and behaves as a fresh one.

mod support;

use stella_rain_core::input::Input;
use stella_rain_net::save::Save;
use stella_rain_net::session::{Config, ConfirmedTick, Session, SessionError};
use stella_rain_net::wire::{InputFrame, WireInput};
use support::{CountingSave, Script, engine, frame_of, script, slot, straight};

fn config(delay: u32, window: u32) -> Config {
    Config {
        local: slot(0),
        delay,
        window,
    }
}

fn fresh(n: u8, cfg: Config, keep: bool) -> Session {
    let mut s = Session::new(engine(n), cfg).expect("a valid config");
    if keep {
        s.keep_confirmed();
    }
    s
}

/// Gives every remote slot's inputs through tick `to`, and a leave when its stamp is near.
fn give<S: Save>(s: &mut Session<S>, sc: &Script, n: u8, to: u32) {
    for p in 1..n {
        let end = sc
            .leaves
            .iter()
            .find(|(_, w)| *w == slot(p))
            .map_or(to, |(lt, _)| to.min(lt - 1));
        let from = s.next_tick(slot(p));
        let len = sc.ticks();
        if from <= end && from < len {
            s.receive_inputs(slot(p), &frame_of(sc, p, from, end.min(len - 1)))
                .expect("a frame");
        }
        if let Some((lt, _)) = sc.leaves.iter().find(|(_, w)| *w == slot(p)) {
            if s.tick() + 2 >= *lt {
                s.leave(slot(p), *lt).expect("leave");
            }
        }
    }
}

/// `calls` calls of `advance` on a session at tick 0, the remote inputs `lag` ticks late (0: in
/// time). Returns what each call did and the engine hash after it.
fn play<S: Save>(
    s: &mut Session<S>,
    sc: &Script,
    n: u8,
    calls: u32,
    lag: u32,
) -> Vec<(String, u64)> {
    let delay = s.config().delay;
    let mut out = Vec::new();
    for k in 0..calls {
        give(s, sc, n, (k + delay).saturating_sub(lag));
        let input = sc
            .eff
            .get((k + delay) as usize)
            .map_or_else(Input::default, |r| r[0]);
        let adv = s.advance(&input).expect("advance");
        for p in 1..n {
            s.ack(slot(p), s.next_tick(slot(0))).expect("ack");
        }
        out.push((format!("{adv:?}"), s.engine().state_hash()));
    }
    out
}

fn hashes(s: &Session) -> Vec<Option<u64>> {
    (0..=s.confirmed() + 1).map(|k| s.hash_after(k)).collect()
}

/// The values a restarted session shares with a fresh one.
fn assert_fresh(r: &Session, f: &Session, n: u8) {
    let delay = r.config().delay;
    assert_eq!((r.tick(), r.confirmed(), r.engine().tick()), (0, 0, 0));
    assert_eq!(r.engine().state_hash(), f.engine().state_hash());
    assert_eq!(r.hash_after(0), f.hash_after(0));
    assert!(r.hash_after(0).is_some());
    assert_eq!(r.hash_after(1), None);
    assert_eq!(r.next_tick(slot(0)), delay);
    for p in 1..n.max(2) {
        assert_eq!(r.next_tick(slot(p)), 0);
    }
    assert_eq!(
        format!("{:?}", r.local_frame(0)),
        format!("{:?}", f.local_frame(0))
    );
    assert!(r.local_frame(delay).is_none());
    assert_eq!(r.config(), f.config());
}

const SETTINGS: [(u8, u32, u32); 6] = [
    (1, 2, 4),
    (2, 2, 4),
    (3, 2, 4),
    (1, 1, 0),
    (2, 1, 0),
    (3, 1, 0),
];

#[test]
fn a_restarted_session_equals_a_fresh_one() {
    for (n, delay, window) in SETTINGS {
        let cfg = config(delay, window);
        let sc = script(n, 260, delay, None);
        let mut r = fresh(n, cfg, true);
        play(&mut r, &sc, n, 200, 0);
        let mut f = fresh(n, cfg, true);
        r.restart();
        assert_fresh(&r, &f, n);
        assert!(r.take_confirmed().is_empty());
        let a = play(&mut r, &sc, n, 200, 0);
        let b = play(&mut f, &sc, n, 200, 0);
        assert_eq!(a, b, "n {n} delay {delay} window {window}");
        assert_eq!(hashes(&r), hashes(&f));
        assert_eq!(r.take_confirmed(), f.take_confirmed());
    }
}

#[test]
fn a_restart_forgets_a_leave() {
    let cfg = config(2, 4);
    let leaving = script(3, 400, 2, Some((slot(2), 100)));
    let played = || {
        let mut s = fresh(3, cfg, false);
        play(&mut s, &leaving, 3, 150, 0);
        assert!(s.engine().present() >> 2 & 1 == 0, "slot 2 left");
        s.restart();
        s
    };
    assert!(played().leave(slot(2), 5).is_ok());
    let stays = script(3, 400, 2, None);
    let mut s = played();
    play(&mut s, &stays, 3, 150, 0);
    let oracle = straight(&stays, 3);
    for k in 0..=s.confirmed() {
        if let Some(h) = s.hash_after(k) {
            assert_eq!(h, oracle[k as usize], "tick {k}");
        }
    }
}

#[test]
fn a_restart_forgets_acks_and_queued_inputs() {
    let cfg = config(2, 4);
    let sc = script(2, 400, 2, None);
    let played = || {
        let mut s = fresh(2, cfg, false);
        play(&mut s, &sc, 2, 150, 0);
        let from = s.next_tick(slot(1));
        s.receive_inputs(slot(1), &frame_of(&sc, 1, from, from + 2))
            .expect("a frame beyond the confirmed tick");
        s.restart();
        s
    };
    let mut s = played();
    assert_eq!(s.next_tick(slot(1)), 0);
    assert_eq!(s.ack(slot(1), 3), Err(SessionError::AckBeyondSent));
    assert_eq!(s.ack(slot(1), 2), Ok(()));
    let silent = |s: &mut Session| {
        (0..40)
            .map(|_| format!("{:?}", s.advance(&Input::default()).expect("advance")))
            .collect::<Vec<_>>()
    };
    let (mut r, mut f) = (played(), fresh(2, cfg, false));
    let (a, b) = (silent(&mut r), silent(&mut f));
    assert_eq!(a, b);
    assert!(a.iter().any(|x| x.contains("Window")), "{a:?}");
}

#[test]
fn a_restart_hands_out_the_confirmed_log() {
    let cfg = config(2, 4);
    let sc = script(2, 400, 2, None);
    let mut s = fresh(2, cfg, true);
    let mut twin = fresh(2, cfg, true);
    play(&mut s, &sc, 2, 150, 0);
    play(&mut twin, &sc, 2, 150, 0);
    let before = hashes(&s);
    let (confirmed, log) = (s.confirmed(), s.restart());
    assert_eq!(log, twin.take_confirmed());
    assert_eq!(log.len(), confirmed as usize);
    for (i, t) in log.iter().enumerate() {
        assert_eq!(t.tick as usize, i);
        if let Some(Some(h)) = before.get(i + 1) {
            assert_eq!(t.hash, *h, "tick {i}");
        }
    }
    assert!(!log.is_empty());
    assert!(s.restart().is_empty(), "a second restart in a row");
    // The next run's log starts at tick 0: keep stays on.
    play(&mut s, &sc, 2, 20, 0);
    let next: Vec<ConfirmedTick> = s.take_confirmed();
    assert_eq!(next.first().map(|t| t.tick), Some(0));

    // Predicted ticks are not in the log.
    let mut p = fresh(2, cfg, true);
    p.receive_inputs(slot(1), &frame_of(&sc, 1, 0, 8))
        .expect("frame");
    for k in 0..14u32 {
        let _ = p.advance(&sc.eff[(k + 2) as usize][0]);
        p.ack(slot(1), p.next_tick(slot(0))).expect("ack");
    }
    assert!(p.tick() > p.confirmed(), "{} {}", p.tick(), p.confirmed());
    let confirmed = p.confirmed();
    assert_eq!(p.restart().len(), confirmed as usize);

    // Keep off: nothing is logged.
    let mut off = fresh(2, cfg, false);
    play(&mut off, &sc, 2, 50, 0);
    assert!(off.restart().is_empty());
}

#[test]
fn restarting_twice_gives_the_same_run() {
    let cfg = config(2, 4);
    let sc = script(3, 200, 2, None);
    let mut s = fresh(3, cfg, false);
    let first = play(&mut s, &sc, 3, 120, 0);
    for run in 2..=3 {
        s.restart();
        assert_eq!(play(&mut s, &sc, 3, 120, 0), first, "run {run}");
    }
}

#[test]
fn a_restart_at_tick_zero_changes_nothing() {
    for (n, delay, window) in SETTINGS {
        let cfg = config(delay, window);
        let sc = script(n, 100, delay, None);
        let mut r = fresh(n, cfg, true);
        assert!(r.restart().is_empty());
        let mut f = fresh(n, cfg, true);
        assert_fresh(&r, &f, n);
        assert_eq!(play(&mut r, &sc, n, 60, 0), play(&mut f, &sc, n, 60, 0));
    }
}

#[test]
fn a_restart_clears_the_save_ring() {
    let cfg = config(2, 4);
    let sc = script(2, 300, 2, None);
    let save = CountingSave::default();
    let mut s = Session::with_save(engine(2), cfg, save.clone()).expect("a session");
    let first = play(&mut s, &sc, 2, 120, 3);
    assert!(
        first.iter().any(|(a, _)| !a.contains("rolled_back: 0")),
        "run 1 rolled back"
    );
    let loads = save.0.borrow().loads;
    assert!(loads > 0);
    s.restart();
    assert_eq!(save.0.borrow().held(), 0);

    let other = CountingSave::default();
    let mut f = Session::with_save(engine(2), cfg, other.clone()).expect("a session");
    let a = play(&mut s, &sc, 2, 120, 3);
    let b = play(&mut f, &sc, 2, 120, 3);
    assert_eq!(a, b);
    assert!(save.0.borrow().loads > loads, "run 2 rolled back too");
    assert_eq!(save.0.borrow().loads - loads, other.0.borrow().loads);
    assert!(save.0.borrow().max_held <= 5);
}

#[test]
fn a_restart_right_after_a_rollback_to_tick_zero_steps_normally() {
    let cfg = config(2, 4);
    let sc = script(2, 100, 2, None);
    let mut s = fresh(2, cfg, false);
    let _ = s.advance(&Input::default()).expect("advance");
    // A remote input for tick 0 that contradicts the neutral prediction.
    let late = InputFrame {
        run: 0,
        slot: slot(1),
        start_tick: 0,
        inputs: vec![WireInput::from_input(&Input {
            move_x: 100,
            ..Input::default()
        })],
    };
    s.receive_inputs(slot(1), &late).expect("frame");
    assert!(s.confirm().expect("confirm") > 0, "rolled back to tick 0");
    s.restart();
    let mut f = fresh(2, cfg, false);
    assert_eq!(play(&mut s, &sc, 2, 80, 0), play(&mut f, &sc, 2, 80, 0));
}
