//! The session-level SyncTest: sessions that exchange inputs over a mesh of fixed or jittered
//! lateness agree with a straight run at every confirmed tick, whatever they had to predict.

mod support;

use stella_rain_net::session::Wait;
use support::{Mesh, Xorshift, script, slot, straight};

const STOP: u32 = 612;
const DELAY: u32 = 2;
const WINDOW: u32 = 4;

#[test]
fn constant_lateness_agrees_with_the_straight_run() {
    for n in [2u8, 3] {
        let s = script(n, STOP, DELAY, None);
        let oracle = straight(&s, n);
        for k in 0..=5u32 {
            let lat = DELAY + k;
            let mut mesh = Mesh::new(&s, n, DELAY, WINDOW, STOP, Box::new(move |_, _, _| lat));
            for _ in 0..1200 {
                if mesh.done() {
                    break;
                }
                mesh.round();
                mesh.check(&oracle);
            }
            assert!(mesh.done(), "n {n} k {k}: not done after 1200 rounds");
            let ctx = format!("n {n} k {k}");
            match k {
                0..=2 => {
                    assert_eq!(mesh.waits(None), 0, "{ctx}");
                    assert_eq!(mesh.max_rolled(), k, "{ctx}");
                }
                3..=4 => assert!(mesh.max_rolled() <= k, "{ctx}"),
                _ => {
                    assert!(mesh.waits(Some(Wait::Window)) >= 1, "{ctx}");
                    assert!(mesh.max_rolled() <= WINDOW, "{ctx}");
                }
            }
            if k >= 1 {
                assert!(mesh.total_rolled() > 0, "{ctx}");
                assert!(mesh.rolled_over_an_action(), "{ctx}");
            }
            let c = mesh.sessions.iter().map(|s| s.confirmed()).min().unwrap();
            assert!(c >= 600, "{ctx}: confirmed {c}");
            for session in &mesh.sessions {
                assert_eq!(session.hash_after(c), Some(oracle[c as usize]), "{ctx}");
            }
        }
    }
}

/// The tick of the first scripted action of `who`.
fn first_action(s: &support::Script, who: usize) -> u32 {
    let at = s.eff.iter().position(|row| row[who].action.is_some());
    u32::try_from(at.expect("the script holds an action")).unwrap()
}

#[test]
fn an_action_that_arrives_three_ticks_late_rolls_back_three_and_is_not_lost() {
    let s = script(2, STOP, DELAY, None);
    let oracle = straight(&s, 2);
    let m = first_action(&s, 1);
    let lat = DELAY + 3;
    let mut mesh = Mesh::new(&s, 2, DELAY, WINDOW, STOP, Box::new(move |_, _, _| lat));
    mesh.run(1200);
    assert!(mesh.done());
    let at_arrival = mesh.calls[0]
        .iter()
        .find(|c| c.tick_before == m + 3 && c.out.waiting.is_none())
        .expect("a call at the arrival");
    assert_eq!(at_arrival.out.rolled_back, 3);
    let c = mesh.sessions[0].confirmed();
    assert!(c >= 600);
    assert_eq!(mesh.sessions[0].hash_after(c), Some(oracle[c as usize]));
    // The same run with that action stripped is a different run: a lost action would show.
    let mut stripped = s.clone();
    stripped.eff[m as usize][1].action = None;
    assert_ne!(
        straight(&stripped, 2)[m as usize + 1],
        oracle[m as usize + 1]
    );
}

#[test]
fn a_mutated_input_delivered_late_makes_the_sync_test_fail() {
    let s = script(2, STOP, DELAY, None);
    let oracle = straight(&s, 2);
    let flips = 300u32..320;
    let mut mesh = Mesh::new(&s, 2, DELAY, WINDOW, STOP, Box::new(|_, _, _| DELAY + 2));
    mesh.tamper = Some(Box::new(move |from, _to, frame| {
        if from == 1 {
            for (t, input) in (frame.start_tick..).zip(&mut frame.inputs) {
                if flips.contains(&t) {
                    input.main = !input.main;
                }
            }
        }
    }));
    mesh.run(1200);
    let c = mesh.sessions[0].confirmed();
    assert_ne!(mesh.sessions[0].hash_after(c), Some(oracle[c as usize]));
}

#[test]
fn jittered_messages_and_a_leave_still_agree_with_the_straight_run() {
    const TICKS: u32 = 900;
    const LEAVE: u32 = 300;
    let s = script(3, TICKS, DELAY, Some((slot(2), LEAVE)));
    let oracle = straight(&s, 3);
    for seed in [7u64, 99] {
        let mut rng = Xorshift::new(seed);
        let lateness = Box::new(move |_, _, _| DELAY + u32::try_from(rng.below(5)).unwrap());
        let mut mesh = Mesh::new(&s, 3, DELAY, WINDOW, TICKS, lateness).leaver(2, LEAVE);
        for _ in 0..2400 {
            if mesh.done() {
                break;
            }
            mesh.round();
            mesh.check(&oracle);
        }
        assert!(mesh.done(), "seed {seed}");
        let crossed = mesh.calls[..2].iter().flatten().any(|c| {
            c.out.rolled_back > 0
                && c.tick_before - c.out.rolled_back <= LEAVE
                && LEAVE < c.tick_before
        });
        assert!(crossed, "seed {seed}: no rollback crossed the leave");
        let c = mesh.sessions[..2]
            .iter()
            .map(|s| s.confirmed())
            .min()
            .unwrap();
        assert!(c >= TICKS - 12, "seed {seed}: confirmed {c}");
        for session in &mesh.sessions[..2] {
            assert_eq!(
                session.hash_after(c),
                Some(oracle[c as usize]),
                "seed {seed}"
            );
        }
    }
}
