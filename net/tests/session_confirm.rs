//! `Session::confirm` (confirm without stepping) and the opt-in log of confirmed ticks.

mod support;

use stella_rain_core::input::Input;
use stella_rain_net::session::Session;
use stella_rain_net::wire::WireInput;
use support::{Script, engine, frame_of, script, slot, straight};

const DELAY: u32 = 2;

/// A session for slot 0 of `n` that has played `ticks` calls of `sc`, the remote slots' inputs
/// given `DELAY` ahead of each call (slot `leaver`'s only below its leave tick).
fn drive(sc: &Script, n: u8, ticks: u32, log: bool) -> Session {
    let mut s = Session::new(engine(n), stella_rain_net::session::Config::new(slot(0)))
        .expect("a valid config");
    if log {
        s.keep_confirmed();
    }
    for k in 0..ticks {
        give(&mut s, sc, n, k + DELAY);
        let input = sc
            .eff
            .get((k + DELAY) as usize)
            .map_or_else(Input::default, |r| r[0]);
        let out = s.advance(&input).expect("advance");
        assert!(out.waiting.is_none(), "call {k} waited");
        for p in 1..n {
            s.ack(slot(p), s.next_tick(slot(0))).expect("ack");
        }
    }
    s
}

/// Gives every remote slot's inputs through tick `to`, and a leave when its stamp is near.
fn give(s: &mut Session, sc: &Script, n: u8, to: u32) {
    for p in 1..n {
        let end = sc
            .leaves
            .iter()
            .find(|(_, w)| *w == slot(p))
            .map_or(to, |(lt, _)| to.min(lt - 1));
        let from = s.next_tick(slot(p));
        if from <= end && (from as usize) < sc.eff.len() {
            let end = end.min(sc.eff.len() as u32 - 1);
            s.receive_inputs(slot(p), &frame_of(sc, p, from, end))
                .expect("a frame");
        }
        if let Some((lt, _)) = sc.leaves.iter().find(|(_, w)| *w == slot(p)) {
            if s.tick() + 2 >= *lt {
                s.leave(slot(p), *lt).expect("leave");
            }
        }
    }
}

#[test]
fn confirm_confirms_late_inputs_without_stepping() {
    let sc = script(2, 612, DELAY, None);
    let oracle = straight(&sc, 2);
    let mut s = Session::new(engine(2), stella_rain_net::session::Config::new(slot(0)))
        .expect("a valid config");
    s.receive_inputs(slot(1), &frame_of(&sc, 1, 0, 2))
        .expect("frame");
    for k in 0..6u32 {
        let out = s
            .advance(&sc.eff[(k + DELAY) as usize][0])
            .expect("advance");
        assert!(out.waiting.is_none());
        s.ack(slot(1), s.next_tick(slot(0))).expect("ack");
    }
    assert_eq!(s.tick(), 6);
    assert!(s.confirmed() < 4, "confirmed {}", s.confirmed());
    assert!(
        (3..6).any(|t| sc.eff[t][1] != sc.eff[2][1]),
        "the script must contradict the prediction"
    );
    s.receive_inputs(slot(1), &frame_of(&sc, 1, 3, 5))
        .expect("late frame");
    let rolled = s.confirm().expect("confirm");
    assert!(rolled > 0, "a contradicted prediction is re-simulated");
    assert_eq!(s.tick(), 6, "confirm does not step");
    assert_eq!(s.confirmed(), 6);
    assert_eq!(s.hash_after(6), Some(oracle[6]));
}

#[test]
fn confirm_without_a_remote_peer_confirms_the_tick() {
    let mut s = Session::new(engine(1), stella_rain_net::session::Config::new(slot(0)))
        .expect("a valid config");
    for _ in 0..5 {
        s.advance(&Input::default()).expect("advance");
    }
    assert_eq!(s.confirm(), Ok(0));
    assert_eq!(s.confirmed(), s.tick());
    assert_eq!(s.tick(), 5);
}

#[test]
fn the_log_holds_one_entry_per_confirmed_tick() {
    for (n, leave) in [(2u8, None), (3, Some((slot(2), 300)))] {
        let sc = script(n, 612, DELAY, leave);
        let oracle = straight(&sc, n);
        let mut s = drive(&sc, n, 612, true);
        give(&mut s, &sc, n, 611);
        s.confirm().expect("confirm");
        assert_eq!(s.confirmed(), 612, "n {n}");
        let log = s.take_confirmed();
        assert_eq!(log.len(), 612, "n {n}");
        for (t, c) in log.iter().enumerate() {
            let t32 = u32::try_from(t).expect("short");
            assert_eq!(c.tick, t32);
            assert_eq!(c.hash, oracle[t + 1], "tick {t}");
            let want_leaves =
                leave.map_or(0u8, |(who, lt)| if lt == t32 { 1 << who.get() } else { 0 });
            assert_eq!(c.leaves, want_leaves, "tick {t}");
            for p in 0..3usize {
                let want = if p < usize::from(n) {
                    WireInput::from_input(&sc.eff[t][p])
                } else {
                    WireInput::default()
                };
                assert_eq!(c.inputs[p], want, "tick {t} slot {p}");
            }
        }
        assert!(s.take_confirmed().is_empty());
    }
}

#[test]
fn without_keep_confirmed_the_log_stays_empty() {
    let sc = script(2, 100, DELAY, None);
    let mut s = drive(&sc, 2, 100, false);
    give(&mut s, &sc, 2, 99);
    s.confirm().expect("confirm");
    assert!(s.confirmed() > 90);
    assert!(s.take_confirmed().is_empty());
}
