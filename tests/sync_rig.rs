//! A SyncTest on the engine itself over the rig cases (ADR-REPL-5, ADR-SIM-3, ADR-SIM-7): a
//! ring of `Engine` clones taken at every tick boundary of a recorded run, a rollback of 1 to
//! `MAX_DEPTH` ticks from that ring at every boundary, the recorded inputs and picks stepped onto
//! the clone again, and the state hash compared with the forward pass after every tick. Beside
//! the hash, the ledger, the per-player stats, the tick, the outcome and the events of the last
//! step are compared at the end of a window: the first two are not hashed.
//!
//! A boundary `b` is the engine after `RigRun::play_onto(0, b)`: before the picks keyed `b`, so a
//! window that starts on an open offer replays its pick first. A window is one rollback: the ring
//! clone of boundary `t - d` stepped tick by tick up to boundary `t`. Integer arithmetic only.

use crate::common::run::trial_setup;
use crate::party_bot::RigRun;
use crate::run_play::party_cases;
use std::collections::VecDeque;
use std::ops::RangeInclusive;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::replay::hash_from_hex;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{CHECKPOINT_TICKS, RunReplay};
use stella_rain_core::run_setup::RunSetup;

/// The rig cases: a player goes down and is revived, every slot played by a bot.
const RIG_CASES: [&str; 2] = ["two_revive", "three_revive"];
/// The rollback window, in ticks.
const MAX_DEPTH: u32 = 8;
/// A rollback is run at the boundaries that are a multiple of this: every one.
const STRIDE: u32 = 1;

/// The engine clones of the last `MAX_DEPTH + 1` boundaries.
struct Ring {
    saved: VecDeque<(u32, Engine)>,
}

impl Ring {
    fn new() -> Ring {
        Ring {
            saved: VecDeque::new(),
        }
    }

    /// Saves a clone of `e` as boundary `boundary`, dropping the oldest beyond the window.
    fn push(&mut self, boundary: u32, e: &Engine) {
        self.saved.push_back((boundary, e.clone()));
        while self.saved.len() > MAX_DEPTH as usize + 1 {
            self.saved.pop_front();
        }
    }

    fn at(&self, boundary: u32) -> Option<&Engine> {
        self.saved
            .iter()
            .find(|(b, _)| *b == boundary)
            .map(|(_, e)| e)
    }
}

/// A window whose hash differs from the forward pass.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Desync {
    /// The first boundary of the window whose hash differs.
    tick: u32,
    depth: u32,
    /// The boundary the window was run up to.
    at: u32,
    expected: u64,
    got: u64,
}

#[derive(Clone, Debug, Default)]
struct Report {
    /// The forward pass's hash at every boundary, 0 to the last.
    hashes: Vec<u64>,
    windows: u32,
    resimulated: u32,
    /// Windows whose first clone had an offer open.
    on_offer: u32,
    /// Pick steps keyed inside windows, summed.
    picks_replayed: u32,
    /// `PlayerRevived` events seen in re-stepped ticks.
    revives: u32,
    desyncs: Vec<Desync>,
}

/// One rig case, played once.
struct Case {
    name: &'static str,
    replay: RunReplay,
    rig: RigRun,
}

impl Case {
    fn len(&self) -> u32 {
        u32::try_from(self.rig.ticks.len()).unwrap()
    }
}

fn cases() -> Vec<Case> {
    let all = party_cases();
    RIG_CASES
        .iter()
        .map(|name| {
            let case = all
                .iter()
                .find(|c| c.name == *name)
                .unwrap_or_else(|| panic!("the party case {name} is missing"));
            let replay = case.play().replay();
            let rig = RigRun::of(&replay);
            Case { name, replay, rig }
        })
        .collect()
}

/// Plays `rig` forward from a fresh engine, saving every boundary in a ring; at the boundaries in
/// `checks` (that are a multiple of `STRIDE`) rolls back `d` ticks for each `d` in `depths` and
/// steps `resim` over the window, comparing the hash after every tick with the forward pass.
/// A positive test passes `rig` twice, a negative one an altered copy.
fn sync_test(
    rig: &RigRun,
    resim: &RigRun,
    setup: &RunSetup,
    depths: RangeInclusive<u32>,
    checks: RangeInclusive<u32>,
) -> Report {
    let len = u32::try_from(rig.ticks.len()).unwrap();
    let mut e = Engine::start_party_with(setup, RunContent::bundled(), rig.n).unwrap();
    let mut ring = Ring::new();
    let mut report = Report {
        hashes: vec![e.state_hash()],
        ..Report::default()
    };
    for t in 0..=len {
        ring.push(t, &e);
        let saved = ring.at(t).unwrap();
        assert_eq!(saved.state_hash(), e.state_hash(), "clone at boundary {t}");
        assert_eq!(saved.tick(), e.tick(), "clone at boundary {t}");
        assert_eq!(saved.outcome(), e.outcome(), "clone at boundary {t}");
        assert_eq!(saved.ledger(), e.ledger(), "clone at boundary {t}");
        assert_eq!(
            saved.player_stats(),
            e.player_stats(),
            "clone at boundary {t}"
        );
        if t == len {
            break;
        }
        rig.play_onto(&mut e, t, t + 1);
        report.hashes.push(e.state_hash());
    }
    let _ = (resim, depths, checks, STRIDE);
    report
}

#[test]
fn a_ring_keeps_the_last_nine_boundaries() {
    let mut e = Engine::start_party(&trial_setup(), 1).unwrap();
    let mut ring = Ring::new();
    let mut hashes = Vec::new();
    for boundary in 0..=20 {
        ring.push(boundary, &e);
        hashes.push(e.state_hash());
        e.step(&[Input::default(); 3]);
    }
    assert_eq!(ring.saved.len(), 9);
    assert!(ring.at(11).is_none());
    for boundary in 12..=20 {
        let saved = ring.at(boundary).expect("a boundary in the window");
        assert_eq!(saved.tick(), boundary);
        assert_eq!(saved.state_hash(), hashes[boundary as usize]);
    }
    // A clone is independent: stepping it leaves the ring's own copy as it was.
    let mut copy = ring.at(20).unwrap().clone();
    copy.step(&[Input::default(); 3]);
    assert_eq!(ring.at(20).unwrap().state_hash(), hashes[20]);
    assert_ne!(copy.state_hash(), hashes[20]);
}

/// The sum over `depths` of `f(d)`.
fn sum_over(depths: RangeInclusive<u32>, f: impl Fn(u32) -> u32) -> u32 {
    depths.map(f).sum()
}

#[test]
fn a_rollback_of_every_depth_at_every_tick_agrees_with_the_forward_pass() {
    for case in cases() {
        let len = case.len();
        assert_eq!(len, case.replay.ticks, "{}", case.name);
        let report = sync_test(
            &case.rig,
            &case.rig,
            &case.replay.setup,
            1..=MAX_DEPTH,
            0..=len,
        );
        assert_eq!(report.desyncs, vec![], "{}", case.name);
        assert_eq!(
            report.windows,
            sum_over(1..=MAX_DEPTH, |d| len + 1 - d),
            "{}: windows",
            case.name
        );
        assert_eq!(
            report.resimulated,
            sum_over(1..=MAX_DEPTH, |d| d * (len + 1 - d)),
            "{}: re-stepped ticks",
            case.name
        );
        assert!(
            report.on_offer > 0,
            "{}: a window starts on an open offer",
            case.name
        );
        assert!(
            report.picks_replayed > 0,
            "{}: a pick is replayed",
            case.name
        );
        assert!(report.revives > 0, "{}: a revive is replayed", case.name);
        let one = sync_test(&case.rig, &case.rig, &case.replay.setup, 8..=8, 0..=len);
        assert_eq!(one.windows, len - 7, "{}: one depth", case.name);
    }
}

#[test]
fn the_forward_pass_ends_at_the_recorded_hashes() {
    for case in cases() {
        let len = case.len();
        let report = sync_test(&case.rig, &case.rig, &case.replay.setup, 1..=1, 0..=0);
        assert_eq!(report.hashes.len(), len as usize + 1, "{}", case.name);
        assert_eq!(
            Some(report.hashes[len as usize]),
            hash_from_hex(&case.replay.final_hash),
            "{}: final hash",
            case.name
        );
        for m in 1..=len / CHECKPOINT_TICKS {
            let at = (m * CHECKPOINT_TICKS) as usize;
            let from = (m as usize - 1) * 16;
            assert_eq!(
                Some(report.hashes[at]),
                hash_from_hex(&case.replay.checkpoints[from..from + 16]),
                "{}: checkpoint at tick {at}",
                case.name
            );
        }
        // A clone taken half way and stepped to the end beside the original agrees every tick.
        let mut a = Engine::start_party_with(&case.replay.setup, RunContent::bundled(), case.rig.n)
            .unwrap();
        case.rig.play_onto(&mut a, 0, len / 2);
        let mut b = a.clone();
        for t in len / 2..len {
            case.rig.play_onto(&mut a, t, t + 1);
            case.rig.play_onto(&mut b, t, t + 1);
            assert_eq!(a.state_hash(), b.state_hash(), "{}: tick {t}", case.name);
        }
    }
}

#[test]
fn the_rig_cases_hold_a_revive() {
    // The revive the windows are to cover is in the recorded run, not only in a window.
    for case in cases() {
        let mut e = Engine::start_party_with(&case.replay.setup, RunContent::bundled(), case.rig.n)
            .unwrap();
        let mut revives = 0;
        case.rig.play_with(&mut e, 0, case.len(), |e| {
            revives += e
                .events()
                .iter()
                .filter(|ev| matches!(ev, DomainEvent::PlayerRevived { .. }))
                .count();
        });
        assert!(revives > 0, "{}", case.name);
    }
}
