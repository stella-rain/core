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
use stella_rain_core::engine::{ChoiceError, Engine};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::replay::hash_from_hex;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{CHECKPOINT_TICKS, RunReplay};
use stella_rain_core::run_setup::RunSetup;

/// The rig cases: a player goes down and is revived, every slot played by a bot.
const RIG_CASES: [&str; 2] = ["two_revive", "three_revive"];
/// The rollback window, in ticks.
const MAX_DEPTH: u32 = 8;
/// A rollback is run at the boundaries that are a multiple of this, every depth at each: with 1
/// the module took 32 s, over the 30 s bar, so it is 2.
const STRIDE: u32 = 2;

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

fn case(name: &'static str) -> Case {
    let case = party_cases()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("the party case {name} is missing"));
    let replay = case.play().replay();
    let rig = RigRun::of(&replay);
    Case { name, replay, rig }
}

fn cases() -> Vec<Case> {
    RIG_CASES.iter().map(|name| case(name)).collect()
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
        if checks.contains(&t) && t.rem_euclid(STRIDE) == 0 {
            for d in depths.clone().filter(|d| *d <= t) {
                window(&mut report, &ring, resim, t, d);
            }
        }
        if t == len {
            break;
        }
        rig.play_onto(&mut e, t, t + 1);
        report.hashes.push(e.state_hash());
    }
    report
}

/// One rollback: the ring's clone of boundary `at - depth` stepped by `resim` up to boundary
/// `at`, the hash compared after every tick, and at the end of an equal window everything else
/// the engine holds that the hash does not.
fn window(report: &mut Report, ring: &Ring, resim: &RigRun, at: u32, depth: u32) {
    let from = at - depth;
    let mut c = ring.at(from).expect("the ring holds the window").clone();
    report.windows += 1;
    report.on_offer += u32::from(c.offer_open());
    let len = u32::try_from(resim.ticks.len()).unwrap();
    report.picks_replayed += u32::try_from(
        resim
            .picks
            .iter()
            .filter(|(key, _)| (from..at).contains(key) || (at == len && *key == len))
            .count(),
    )
    .unwrap();
    for k in from..at {
        resim.play_with(&mut c, k, k + 1, |e| {
            report.revives += u32::try_from(
                e.events()
                    .iter()
                    .filter(|ev| matches!(ev, DomainEvent::PlayerRevived { .. }))
                    .count(),
            )
            .unwrap();
        });
        report.resimulated += 1;
        let (expected, got) = (report.hashes[k as usize + 1], c.state_hash());
        if expected != got {
            report.desyncs.push(Desync {
                tick: k + 1,
                depth,
                at,
                expected,
                got,
            });
            return;
        }
    }
    let forward = ring.at(at).expect("the ring holds the boundary");
    assert_eq!(c.tick(), forward.tick(), "tick: depth {depth} at {at}");
    assert_eq!(
        c.outcome(),
        forward.outcome(),
        "outcome: depth {depth} at {at}"
    );
    assert_eq!(
        c.ledger(),
        forward.ledger(),
        "ledger: depth {depth} at {at}"
    );
    assert_eq!(
        c.player_stats(),
        forward.player_stats(),
        "player stats: depth {depth} at {at}"
    );
    assert_eq!(
        c.events(),
        forward.events(),
        "events: depth {depth} at {at}"
    );
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

/// The boundaries `d..=len` that are a multiple of `STRIDE`: the windows of depth `d`.
fn boundaries(d: u32, len: u32) -> u32 {
    len / STRIDE - (d - 1) / STRIDE
}

/// The boundaries in `from..=to` where windows are run.
fn strided(from: u32, to: u32) -> Vec<u32> {
    (from..=to).filter(|t| t.rem_euclid(STRIDE) == 0).collect()
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
            sum_over(1..=MAX_DEPTH, |d| boundaries(d, len)),
            "{}: windows",
            case.name
        );
        assert_eq!(
            report.resimulated,
            sum_over(1..=MAX_DEPTH, |d| d * boundaries(d, len)),
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
        assert_eq!(one.windows, boundaries(8, len), "{}: one depth", case.name);
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

/// A copy of `rig` with the input of `slot` at world tick `tick` changed by `f`.
fn with_input(rig: &RigRun, tick: u32, slot: usize, f: impl Fn(Input) -> Input) -> RigRun {
    let mut out = rig.clone();
    let input = &mut out.ticks[tick as usize][slot];
    *input = f(*input);
    out
}

/// A copy of `rig` with the first pick of `rig.picks[index]` set to `option`; returns the slot.
fn with_pick(rig: &RigRun, index: usize, option: u32) -> (RigRun, usize) {
    let mut out = rig.clone();
    let slot = out.picks[index]
        .1
        .iter()
        .position(Option::is_some)
        .expect("a pick step holds a pick");
    out.picks[index].1[slot] = Some(option);
    (out, slot)
}

/// The slot-0 steer an altered input takes: full right, or full left where it was full right.
fn steered(input: Input) -> Input {
    Input {
        move_x: if input.move_x == 127 { -127 } else { 127 },
        ..input
    }
}

/// Per case: the input tick `k` (slot 0's steer is altered), and the index of the pick in
/// `rig.picks` with the option it is altered to (the old option plus one, modulo the offer's
/// options). Found by scanning, not committed: ticks `k >= 100` upward for the first one whose
/// altered step changes the hash after it, and the picks in order for the first in a slot whose
/// offer has two or more options and whose altered hash differs. The scan ran over ticks 100 up
/// (both cases change at once, at 100) and over `rig.picks` from index 0 (a 10-option level-up
/// offer in slot 0 at ticks 108 and 106, option 0 taken, 1 chosen instead).
const NEG: [(&str, u32, usize, u32); 2] = [("two_revive", 100, 0, 1), ("three_revive", 100, 0, 1)];

fn neg_case(name: &str) -> (Case, u32, usize, u32) {
    let (name, k, i, alt) = *NEG.iter().find(|(n, ..)| *n == name).unwrap();
    (case(name), k, i, alt)
}

/// The hash at boundaries 0 to `upto` of `rig` played forward. An altered run is played only as
/// far as the alteration is looked at: past it, the recorded picks no longer fit its offers.
fn hashes_to(rig: &RigRun, setup: &RunSetup, upto: u32) -> Vec<u64> {
    let mut e = Engine::start_party_with(setup, RunContent::bundled(), rig.n).unwrap();
    let mut out = vec![e.state_hash()];
    for t in 0..upto {
        rig.play_onto(&mut e, t, t + 1);
        out.push(e.state_hash());
    }
    out
}

/// The boundaries where windows are run for an alteration at `step`: ten before it to ten after,
/// which holds every window that contains it and the windows on both sides that do not. The
/// forward pass still runs the whole case.
fn around(step: u32, len: u32) -> RangeInclusive<u32> {
    step.saturating_sub(10)..=(step + 10).min(len)
}

fn desync_boundaries(report: &Report) -> Vec<u32> {
    report.desyncs.iter().map(|d| d.at).collect()
}

#[test]
fn an_altered_input_is_caught_at_the_tick_it_takes_effect() {
    for name in RIG_CASES {
        let (case, k, ..) = neg_case(name);
        let (rig, setup, len) = (&case.rig, &case.replay.setup, case.len());
        let altered = with_input(rig, k, 0, steered);
        let h = hashes_to(rig, setup, k + 1);
        let h2 = hashes_to(&altered, setup, k + 1);
        assert_eq!(
            h2[..=k as usize],
            h[..=k as usize],
            "{name}: equal up to {k}"
        );
        assert_ne!(
            h2[k as usize + 1],
            h[k as usize + 1],
            "{name}: the step changes the hash"
        );

        let report = sync_test(rig, &altered, setup, 8..=8, around(k, len));
        let want = strided(k + 1, (k + 8).min(len));
        assert_eq!(desync_boundaries(&report), want, "{name}");
        assert!(
            report
                .desyncs
                .iter()
                .all(|d| d.tick == k + 1 && d.depth == 8),
            "{name}: {:?}",
            report.desyncs
        );

        let all = sync_test(rig, &altered, setup, 1..=8, around(k, len));
        let first = &all.desyncs[0];
        // The first window that holds tick `k` ends on the first boundary run after it.
        let at = (k + 1).next_multiple_of(STRIDE);
        assert_eq!(
            (first.tick, first.depth, first.at),
            (k + 1, at - k, at),
            "{name}"
        );
    }
}

#[test]
fn an_altered_pick_is_caught_at_the_tick_it_takes_effect() {
    for name in RIG_CASES {
        let (case, _, i, alt) = neg_case(name);
        let (rig, setup, len) = (&case.rig, &case.replay.setup, case.len());
        let p = rig.picks[i].0;
        let (altered, slot) = with_pick(rig, i, alt);
        assert_ne!(altered.picks[i], rig.picks[i], "{name}: the pick changes");
        // The alternative is a legal option of the offer open at that boundary.
        let mut e = Engine::start_party_with(setup, RunContent::bundled(), rig.n).unwrap();
        rig.play_onto(&mut e, 0, p);
        let player = PlayerIdx::new(u8::try_from(slot).unwrap()).unwrap();
        assert_eq!(e.check_pick(player, alt), Ok(()), "{name}: a legal option");
        assert!(matches!(
            e.check_pick(player, u32::MAX),
            Err(ChoiceError::OutOfRange { options, .. }) if options >= 2
        ));

        let h = hashes_to(rig, setup, p + 1);
        let h2 = hashes_to(&altered, setup, p + 1);
        assert_eq!(
            h2[..=p as usize],
            h[..=p as usize],
            "{name}: equal up to {p}"
        );
        assert_ne!(
            h2[p as usize + 1],
            h[p as usize + 1],
            "{name}: the pick changes the hash"
        );

        let report = sync_test(rig, &altered, setup, 8..=8, around(p, len));
        let want = strided(p + 1, (p + 8).min(len));
        assert_eq!(desync_boundaries(&report), want, "{name}");
        assert!(
            report.desyncs.iter().all(|d| d.tick == p + 1),
            "{name}: {:?}",
            report.desyncs
        );
    }
}
