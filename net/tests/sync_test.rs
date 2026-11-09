//! The session-level SyncTest: sessions that exchange inputs over a mesh of fixed or jittered
//! lateness agree with a straight run at every confirmed tick, whatever they had to predict.

mod support;

use stella_rain_net::session::Wait;
use support::{Mesh, script, straight};

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
    assert_eq!(
        mesh.sessions[0].hash_after(STOP),
        Some(oracle[STOP as usize])
    );
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
    let flip = 300u32;
    let mut mesh = Mesh::new(&s, 2, DELAY, WINDOW, STOP, Box::new(|_, _, _| DELAY + 2));
    mesh.tamper = Some(Box::new(move |from, _to, frame| {
        if from == 1
            && (frame.start_tick..)
                .zip(&mut frame.inputs)
                .any(|(t, _)| t == flip)
        {
            let at = (flip - frame.start_tick) as usize;
            frame.inputs[at].main = !frame.inputs[at].main;
        }
    }));
    mesh.run(1200);
    let c = mesh.sessions[0].confirmed();
    assert_ne!(mesh.sessions[0].hash_after(c), Some(oracle[c as usize]));
}
