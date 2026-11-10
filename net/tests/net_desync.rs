//! The checkpoint exchange, the stop on desync that every peer reaches, and the report that
//! rebuilds the run through the recorder.

mod support;

use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::verify_run;
use stella_rain_net::session::party::{NetError, Phase, StopReason};
use stella_rain_net::session::report::ReportError;
use support::peers::{Peers, ROUND_LIMIT, flip};
use support::simnet::Impair;
use support::{Script, equipment_setup, script, slot, straight};

fn clean(latency: u32) -> Impair {
    Impair::clean(latency)
}

fn tampered(n: u8, impair: Impair, seed: u64) -> (Peers, Vec<u64>) {
    let sc = script(n, 612, 2, None);
    let oracle = straight(&sc, n);
    let mut peers = Peers::new(sc, n, seed, impair);
    peers.set_tamper(flip(1, 0, 300..340));
    (peers, oracle)
}

fn hex(h: u64) -> String {
    format!("{h:016x}")
}

#[test]
fn converging_runs_compare_checkpoints_and_never_stop() {
    for n in [2u8, 3] {
        let sc = script(n, 612, 2, None);
        let oracle = straight(&sc, n);
        let mut peers = Peers::new(sc, n, 1, clean(3));
        assert!(peers.run(&oracle));
        for (i, s) in peers.sessions.iter().enumerate() {
            assert!(s.stats().checkpoints_compared >= 9, "n {n} session {i}");
            assert_ne!(s.phase(), Phase::Stopped, "n {n} session {i}");
            assert!(s.desync_report().is_none());
        }
    }
}

#[test]
fn a_divergence_stops_both_peers_on_the_same_checkpoint() {
    let (mut peers, oracle) = tampered(2, clean(3), 1);
    assert!(peers.run_with(|_| {}), "not stopped");
    assert_eq!(
        peers.sessions[0].stop_reason(),
        Some(StopReason::Desync {
            tick: 360,
            slot: slot(1)
        })
    );
    assert!(
        matches!(
            peers.sessions[1].stop_reason(),
            Some(StopReason::Desync { tick: 360, slot: s }) if s == slot(0)
        ) || matches!(
            peers.sessions[1].stop_reason(),
            Some(StopReason::PeerDesync { slot: s, tick: 360 }) if s == slot(0)
        )
    );
    for s in &mut peers.sessions {
        assert_eq!(s.phase(), Phase::Stopped);
        assert!(s.session().tick() <= 420);
        assert_eq!(s.advance(&Input::default()), Err(NetError::Stopped));
    }
    let r1 = peers.sessions[1].desync_report().expect("a report");
    assert_eq!(r1.local_hash, Some(oracle[360]));
    let r0 = peers.sessions[0].desync_report().expect("a report");
    assert_ne!(r0.local_hash, Some(oracle[360]));
    assert_eq!(r0.remote_hash, Some(oracle[360]));
}

#[test]
fn a_tamper_towards_one_of_three_stops_all_three() {
    let (mut peers, _) = tampered(3, clean(3), 1);
    assert!(peers.run_with(|_| {}));
    for (i, s) in peers.sessions.iter().enumerate() {
        assert_eq!(s.phase(), Phase::Stopped, "session {i}");
        match s.stop_reason() {
            Some(StopReason::Desync { tick, .. } | StopReason::PeerDesync { tick, .. }) => {
                assert_eq!(tick, 360, "session {i}");
            }
            other => panic!("session {i}: {other:?}"),
        }
    }
}

#[test]
fn a_bye_stops_a_peer_whose_checkpoints_never_arrive() {
    for (seed, loss) in [(0, 0), (1, 300), (2, 300)] {
        let sc = script(3, 612, 2, None);
        let oracle = straight(&sc, 3);
        let impair = Impair { loss, ..clean(3) };
        let mut peers = Peers::new(sc, 3, seed.max(1), impair).drop_checkpoints_to(2);
        peers.set_tamper(flip(1, 0, 300..340));
        let _ = &oracle;
        assert!(
            peers.run_with(|_| {}),
            "seed {seed}: round limit {ROUND_LIMIT}"
        );
        assert_eq!(peers.sessions[2].stats().checkpoints_compared, 0);
        assert!(matches!(
            peers.sessions[2].stop_reason(),
            Some(StopReason::PeerDesync { .. })
        ));
    }
}

#[test]
fn lost_duplicated_and_reordered_checkpoints_are_never_a_desync() {
    for seed in [1, 2] {
        let sc = script(3, 612, 2, None);
        let oracle = straight(&sc, 3);
        let impair = Impair {
            latency: 3,
            jitter: 3,
            loss: 300,
            reorder: 200,
            reorder_extra: 4,
            duplicate: 200,
        };
        let mut peers = Peers::new(sc, 3, seed, impair);
        assert!(peers.run(&oracle));
        for s in &peers.sessions {
            assert_ne!(s.phase(), Phase::Stopped);
        }
    }
}

fn actions_below(sc: &Script, ticks: u32) -> usize {
    sc.eff[..ticks as usize]
        .iter()
        .flat_map(|row| row.iter())
        .filter(|i| {
            matches!(
                i.action,
                Some(EquipAction::Equip { .. } | EquipAction::Extract { .. })
            )
        })
        .count()
}

#[test]
fn the_report_rebuilds_a_replay_that_verifies() {
    let (mut peers, oracle) = tampered(2, clean(3), 1);
    assert!(peers.run_with(|_| {}));
    let content = RunContent::bundled();
    let r1 = peers.sessions[1]
        .desync_report()
        .expect("a report")
        .replay(&equipment_setup(), content)
        .expect("a replay");
    assert_eq!(r1.ticks, 360);
    assert_eq!(r1.final_hash, hex(oracle[360]));
    let want: String = (1..=6).map(|k| hex(oracle[60 * k])).collect();
    assert_eq!(r1.checkpoints, want);
    verify_run(&r1.to_bytes()).expect("verifies");
    let range = r1.step_range(0, 360, u32::MAX).expect("a range");
    // `RangeStep` is not public API: read the steps through their `Debug` text.
    let text = format!("{:?}", range.steps);
    let actions = text.matches("Equip {").count() + text.matches("Extract {").count();
    assert_eq!(actions, actions_below(&peers.script, 360));
    let r0 = peers.sessions[0]
        .desync_report()
        .expect("a report")
        .replay(&equipment_setup(), content)
        .expect("a replay");
    verify_run(&r0.to_bytes()).expect("peer 0's own run verifies too");
    assert_eq!(r0.checkpoints[..5 * 16], r1.checkpoints[..5 * 16]);
    assert_ne!(r0.checkpoints[5 * 16..], r1.checkpoints[5 * 16..]);
}

#[test]
fn a_report_holds_the_whole_log_and_a_leave_and_refuses_another_setup() {
    let sc = script(3, 612, 2, Some((slot(2), 300)));
    let oracle = straight(&sc, 3);
    let mut peers = Peers::new(sc, 3, 1, clean(3)).leaver(2, 300);
    peers.set_tamper(flip(1, 0, 400..440));
    let _ = &oracle;
    assert!(peers.run_with(|_| {}));
    let report = peers.sessions[1].desync_report().expect("a report");
    assert_eq!(report.ticks.len(), report.tick as usize);
    assert_eq!(report.party, 3);
    let replay = report
        .replay(&equipment_setup(), RunContent::bundled())
        .expect("a replay");
    verify_run(&replay.to_bytes()).expect("verifies");
    let range = replay.step_range(0, replay.ticks, u32::MAX).expect("range");
    assert!(format!("{:?}", range.steps).contains("Leave { slot: 2 }"));
    let mut other = equipment_setup();
    other.seed += 1;
    assert_eq!(
        report.replay(&other, RunContent::bundled()).err(),
        Some(ReportError::HashDiffers)
    );
}
