//! The savepoint determinism proof (ADR-SAVE-4, ADR-SIM-3): for every run case of the replay
//! corpus in the current tree, a savepoint taken at a cut, restored and run to the end gives the
//! hashes, the ledger, the tallies, the snapshot and the replay of the uninterrupted run. The
//! same holds for every run case of the frozen `v1/` tree: a recorder at `sim_version` 1 writes
//! the schema 1 file and takes the PICKS steps through its bench-gated seam. `the_proof_keeps_its_floor`
//! counts the verdicts `prove` returns per tree. `check_run` is
//! generic over the restore, so the meta-tests can hand it restores that are wrong on purpose and
//! see it name them.
//!
//! The stage cases are not here: a stage engine has no `RunSetup`, so it has no savepoint file,
//! and the state codec behind it is crate-private (the library's own `savepoint_tests` cover
//! the stage cases through it).

use crate::common;
use crate::common::run_steps::{self, RunSteps, Step};
use crate::corpus_pins::{case_dirs, run_case_pack};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use stella_rain_core::engine::Outcome;
use stella_rain_core::input::Input;
use stella_rain_core::ledger::RunLedger;
use stella_rain_core::pack::{ContentFiles, Pack};
use stella_rain_core::rng::{TAG_ATTACK, derive};
use stella_rain_core::run_replay::{RunRecorder, RunReplay, RunReplayError};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::Snapshot;

type Plain = (u32, Vec<(String, u8)>, Vec<String>, Vec<(String, u8)>);

const TREES: [&str; 2] = ["current", "v1"];

/// One run case of one tree, read.
struct Case {
    label: String,
    recorded: RunReplay,
    pack: Pack,
    steps: RunSteps,
}

fn tree_dir(tree: &str) -> PathBuf {
    crate::data_root::data_root()
        .join("tests/corpus")
        .join(tree)
}

fn load_case(tree: &str, name: &str) -> Case {
    let dir = tree_dir(tree).join("runs").join(name);
    let bytes = fs::read(dir.join("run.replay")).unwrap();
    let file = RunReplay::from_bytes(&bytes).unwrap();
    let pack = run_case_pack(&dir, &bytes).unwrap();
    let steps = RunSteps::of(&file);
    Case {
        label: format!("{tree}/{name}"),
        recorded: file,
        pack,
        steps,
    }
}

/// What a run shows at one tick, to compare a restored run with the original. The ledger and the
/// tally are not in the state hash, so this is what guards them.
#[derive(Clone, Debug, PartialEq)]
struct Mark {
    tick: u32,
    hash: u64,
    offer_open: bool,
    outcome: Outcome,
    ledger: RunLedger,
    tally: Plain,
    snapshot: Snapshot,
}

fn mark(rec: &RunRecorder) -> Mark {
    let e = rec.engine();
    let t = e.tally();
    let mut taken: Vec<(String, u8)> = t.taken().map(|(id, n)| (id.to_string(), n)).collect();
    taken.sort();
    let mut tiers: Vec<(String, u8)> = t.tiers().map(|(id, n)| (id.to_string(), n)).collect();
    tiers.sort();
    Mark {
        tick: e.tick(),
        hash: e.state_hash(),
        offer_open: e.offer_open(),
        outcome: e.outcome(),
        ledger: e.ledger().clone(),
        tally: (
            t.kills(),
            taken,
            t.named().iter().map(ToString::to_string).collect(),
            tiers,
        ),
        snapshot: e.snapshot(),
    }
}

/// The first field in which two marks of the same tick differ.
fn first_diff(a: &Mark, b: &Mark) -> Option<&'static str> {
    if a.tick != b.tick {
        Some("tick")
    } else if a.offer_open != b.offer_open {
        Some("offer_open")
    } else {
        same_end(a, b).or((a.hash != b.hash).then_some("hash"))
    }
}

/// The first of the ledger, the tally, the outcome and the snapshot in which two marks differ:
/// the state the hash does not see, and what the player sees.
fn same_end(a: &Mark, b: &Mark) -> Option<&'static str> {
    if a.ledger != b.ledger {
        Some("ledger")
    } else if a.tally != b.tally {
        Some("tally")
    } else if a.outcome != b.outcome {
        Some("outcome")
    } else if a.snapshot != b.snapshot {
        Some("snapshot")
    } else {
        None
    }
}

/// A snapshot with an enemy strictly inside an attack phase.
fn enemy_mid_attack(s: &Snapshot) -> bool {
    s.entities.iter().any(|e| {
        e.attack
            .as_ref()
            .is_some_and(|a| 0 < a.tick && a.tick + 1 < a.ticks)
    })
}

/// A snapshot with the player in a combo, a charge, a hold or a skill.
fn player_mid_attack(s: &Snapshot) -> bool {
    let p = &s.players[0];
    !p.combo.presses.is_empty() || p.charge.is_some() || p.hold.is_some() || p.skill.is_some()
}

/// The uninterrupted run, once per case: what is true after each tick.
struct Reference {
    /// `hashes[t - 1]` is the state hash after tick `t`, before the commands that follow it.
    hashes: Vec<u64>,
    offer: Vec<bool>,
    enemy_mid: Vec<bool>,
    player_mid: Vec<bool>,
    end: Mark,
}

impl Reference {
    fn ticks(&self) -> u32 {
        u32::try_from(self.hashes.len()).unwrap()
    }
    fn hash(&self, tick: u32) -> u64 {
        self.hashes[tick as usize - 1]
    }
}

/// Plays the case from `rec` at tick `from` to its last step, the picks before each tick as
/// the run made them, calling `visit(t, rec)` right after tick `t` and before the steps that
/// follow it.
fn play_from(
    case: &Case,
    mut rec: RunRecorder,
    from: u32,
    mut visit: impl FnMut(u32, &RunRecorder) -> Result<(), String>,
) -> Result<RunRecorder, String> {
    for step in &case.steps.steps()[case.steps.at(from)..] {
        run_steps::apply_to(&mut rec, step);
        if matches!(step, Step::Tick(_)) {
            visit(rec.engine().tick(), &rec)?;
        }
    }
    Ok(rec)
}

fn fresh(case: &Case) -> RunRecorder {
    RunRecorder::start_with_pack(&case.recorded.setup, &case.pack).unwrap()
}

fn reference(case: &Case) -> Reference {
    let (mut hashes, mut offer, mut enemy_mid, mut player_mid) = (vec![], vec![], vec![], vec![]);
    let rec = play_from(case, fresh(case), 0, |_, rec| {
        let e = rec.engine();
        let s = e.snapshot();
        hashes.push(e.state_hash());
        offer.push(e.offer_open());
        enemy_mid.push(enemy_mid_attack(&s));
        player_mid.push(player_mid_attack(&s));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        rec.replay(),
        case.recorded,
        "{}: the case does not replay as recorded",
        case.label
    );
    Reference {
        hashes,
        offer,
        enemy_mid,
        player_mid,
        end: mark(&rec),
    }
}

/// The ticks to cut at, `1 <= t < N`, sorted: both edges, six evenly spaced, the first and the
/// last with an offer open, the first and the middle of those strictly inside an enemy's attack
/// phase (the player's when no enemy has one).
fn cuts(r: &Reference) -> Vec<u32> {
    let n = r.ticks();
    let ticks_where =
        |flags: &[bool]| -> Vec<u32> { (1..=n).filter(|t| flags[*t as usize - 1]).collect() };
    let ends = |v: &[u32]| -> Vec<u32> { v.first().into_iter().chain(v.last()).copied().collect() };
    let mut out = vec![1, n - 1];
    out.extend((1..=6).map(|k| n * k / 7));
    out.extend(ends(&ticks_where(&r.offer)));
    let mut attack = ticks_where(&r.enemy_mid);
    if attack.is_empty() {
        attack = ticks_where(&r.player_mid);
    }
    out.extend(attack.first().copied());
    out.extend(attack.get(attack.len() / 2).copied());
    out.retain(|t| (1..n).contains(t));
    out.sort_unstable();
    out.dedup();
    out
}

/// Every 60th tick, the checkpoint ticks, below the last.
fn sweep_cuts(r: &Reference) -> Vec<u32> {
    (1..r.ticks()).filter(|t| t % 60 == 0).collect()
}

/// How a savepoint is turned back into a recorder: the original at the cut, the savepoint bytes,
/// the replay up to the cut and the pack.
type Restore<'a> = &'a dyn Fn(&RunRecorder, &[u8], &RunReplay, &Pack) -> RunRecorder;

fn real_restore(_: &RunRecorder, bytes: &[u8], prefix: &RunReplay, pack: &Pack) -> RunRecorder {
    RunRecorder::resume_from_savepoint(bytes, prefix, pack).unwrap()
}

/// Saves at every cut, restores with `restore`, runs the restored run to the end and compares
/// it with the uninterrupted one. The `Err` names the case, the cut and the first thing that
/// differs.
fn check_run(case: &Case, r: &Reference, cut_at: &[u32], restore: Restore) -> Result<(), String> {
    let label = &case.label;
    play_from(case, fresh(case), 0, |t, orig| {
        if cut_at.binary_search(&t).is_err() {
            return Ok(());
        }
        let fail = |what: String| Err(format!("{label} cut {t}: {what}"));
        let bytes = orig.save();
        let prefix = orig.replay();
        let restored = restore(orig, &bytes, &prefix, &case.pack);
        if restored.engine().tick() != t {
            return fail(format!(
                "diverges first on tick {}: the restored run is at tick {}, the cut was at {t}",
                restored.engine().tick(),
                restored.engine().tick()
            ));
        }
        if let Some(field) = first_diff(&mark(orig), &mark(&restored)) {
            return fail(format!("after the restore {field} differs"));
        }
        if restored.save() != bytes {
            return fail("the restored run saves other bytes".to_owned());
        }
        if restored.replay() != prefix {
            return fail("the restored replay is not the prefix".to_owned());
        }
        let mut restored = restored;
        if r.offer[t as usize - 1] {
            let refused =
                restored.step(&[Input::default(), Default::default(), Default::default()]);
            if !matches!(refused, Err(RunReplayError::OfferOpen { .. })) {
                return fail(format!("a step with the offer open gave {refused:?}"));
            }
        }
        let done = play_from(case, restored, t, |u, rec| {
            let got = rec.engine().state_hash();
            if got == r.hash(u) {
                Ok(())
            } else {
                fail(format!(
                    "diverges first on tick {u}: restored {got:016x}, reference {:016x}",
                    r.hash(u)
                ))
            }
        })?;
        if let Some(field) = first_diff(&r.end, &mark(&done)) {
            return fail(format!("at the end {field} differs"));
        }
        if done.replay().to_bytes() != case.recorded.to_bytes() {
            return fail("the finished replay is not the recorded one".to_owned());
        }
        Ok(())
    })
    .map(|_| ())
}

/// Run cases recorded after `v1/` was frozen: only `current/` holds them.
const CURRENT_ONLY: &[&str] = &["tutorial_opening", "trial_kinds"];

/// What the proof made of a case.
#[derive(Clone, Debug, PartialEq)]
enum Verdict {
    Proved,
    /// Not proved, and why: the floor counts it.
    Skipped(&'static str),
}

/// `None` for a case the tree does not hold (`CURRENT_ONLY` in `v1`). A `Skipped(reason)`
/// return is counted by the floor.
fn prove(tree: &str, name: &str, sweep: bool) -> Option<Verdict> {
    if tree == "v1" && CURRENT_ONLY.contains(&name) {
        return None;
    }
    let case = load_case(tree, name);
    let r = reference(&case);
    let at = if sweep { sweep_cuts(&r) } else { cuts(&r) };
    assert!(!at.is_empty(), "{}: no cuts", case.label);
    check_run(&case, &r, &at, &real_restore).unwrap_or_else(|e| panic!("{e}"));
    Some(Verdict::Proved)
}

type Memo = std::collections::BTreeMap<String, Arc<OnceLock<Option<Verdict>>>>;

/// `prove`'s verdict without the sweep, once per case: the per-case tests and the floor share
/// one run. A panic in `prove` leaves the cell empty, so each test that asks fails on it.
fn verdict(tree: &str, name: &str) -> Option<Verdict> {
    static MEMO: Mutex<Memo> = Mutex::new(Memo::new());
    let cell = MEMO
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(format!("{tree}/{name}"))
        .or_default()
        .clone();
    cell.get_or_init(|| prove(tree, name, false)).clone()
}

/// The proved and the skipped case names of a list of verdicts.
fn split(verdicts: &[(String, Verdict)]) -> (Vec<String>, Vec<String>) {
    let named = |proved: bool| {
        verdicts
            .iter()
            .filter(|(_, v)| (*v == Verdict::Proved) == proved)
            .map(|(n, _)| n.clone())
            .collect()
    };
    (named(true), named(false))
}

macro_rules! run_cases {
    ($($name:ident),* $(,)?) => {
        const RUN_CASES: &[&str] = &[$(stringify!($name)),*];
        $(
            mod $name {
                #[test]
                fn current() {
                    let v = super::verdict("current", stringify!($name));
                    assert!(!matches!(v, Some(super::Verdict::Skipped(_))), "{v:?}");
                }

                #[test]
                fn v1() {
                    let v = super::verdict("v1", stringify!($name));
                    assert!(!matches!(v, Some(super::Verdict::Skipped(_))), "{v:?}");
                }

                #[test]
                #[ignore = "full sweep, run by deep.yml"]
                fn sweep() {
                    for tree in super::TREES {
                        super::prove(tree, stringify!($name), true);
                    }
                }
            }
        )*
    };
}

run_cases!(
    easy,
    elites,
    rainy,
    stage_skill,
    time_limit,
    time_limit_kill,
    time_limit_overtime,
    trial,
    trial_kinds,
    tutorial,
    tutorial_opening,
);

/// The run cases of `current/`, as (offer open, enemy mid-attack, player mid-attack) seen
/// somewhere in the uninterrupted run. A re-bless that moves one fails the guard below. Every
/// current case is an equipment run (no offer opens), so the open offer is covered by the frozen
/// `v1/` runs: `tutorial()` below reads one.
const FEATURES: [(&str, bool, bool, bool); 11] = [
    ("easy", false, true, true),
    ("elites", false, true, true),
    ("rainy", false, true, true),
    ("stage_skill", false, true, true),
    ("time_limit", false, true, true),
    ("time_limit_kill", false, false, true),
    ("time_limit_overtime", false, false, true),
    ("trial", false, true, true),
    ("trial_kinds", false, true, true),
    ("tutorial", false, true, true),
    ("tutorial_opening", false, true, true),
];

#[test]
fn the_case_lists_are_the_directories() {
    for tree in TREES {
        let dirs = case_dirs(&tree_dir(tree).join("runs"));
        let names: Vec<String> = dirs
            .iter()
            .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        let mut listed: Vec<&str> = RUN_CASES
            .iter()
            .copied()
            .filter(|n| tree == "current" || !CURRENT_ONLY.contains(n))
            .collect();
        listed.sort_unstable();
        assert_eq!(
            names, listed,
            "{tree}: the run cases changed; list the new case in run_cases! and FEATURES"
        );
    }
    let features: Vec<&str> = FEATURES.iter().map(|f| f.0).collect();
    assert_eq!(features, RUN_CASES, "FEATURES lists the cases in order");
}

/// The floor of the proof per tree: (tree, proved at least, skipped at most). `current` proves
/// every case, and so does the frozen `v1/` (nine cases), counted from `prove`'s own verdicts.
const FLOOR: [(&str, usize, usize); 2] = [("current", 11, 0), ("v1", 9, 0)];

fn check_floor(tree: &str, proved: &[String], skipped: &[String]) -> Result<(), String> {
    let (min, max) = floor_of(tree);
    if proved.len() >= min && skipped.len() <= max {
        return Ok(());
    }
    Err(format!(
        "savepoint proof, {tree}: {} proved ({}), {} skipped ({}); the floor is at least {min} \
         proved and at most {max} skipped. A case that now skips must be proved, not skipped \
         (what does `prove` return early for?); a case \
         that no longer skips or a corpus bless that changed the case list: raise the floor in \
         FLOOR here and say so in the PR, never lower it to get green.",
        proved.len(),
        proved.join(", "),
        skipped.len(),
        skipped.join(", "),
    ))
}

fn names_of(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

fn floor_of(tree: &str) -> (usize, usize) {
    let f = FLOOR.iter().find(|f| f.0 == tree).unwrap();
    (f.1, f.2)
}

#[test]
fn a_proof_that_proves_nothing_fails() {
    assert_eq!(floor_of("current"), (11, 0));
    let err = check_floor("current", &[], &names_of(RUN_CASES)).unwrap_err();
    assert!(err.contains("savepoint proof, current"), "{err}");
    assert!(err.contains("0 proved"), "{err}");
    assert!(err.contains("11 skipped"), "{err}");
    assert!(err.contains("raise the floor"), "{err}");
}

#[test]
fn a_current_case_that_skips_fails() {
    let rest = names_of(&RUN_CASES[1..]);
    let err = check_floor("current", &rest, &names_of(&["easy"])).unwrap_err();
    assert!(err.contains("easy"), "{err}");
    assert!(err.contains("1 skipped"), "{err}");
}

#[test]
fn a_skipping_prove_is_counted() {
    let (proved, skipped) = split(&[
        ("easy".to_owned(), Verdict::Proved),
        ("trial".to_owned(), Verdict::Skipped("x")),
    ]);
    assert_eq!(proved, names_of(&["easy"]));
    assert_eq!(skipped, names_of(&["trial"]));
    let err = check_floor("v1", &proved, &skipped).unwrap_err();
    assert!(err.contains("1 skipped") && err.contains("trial"), "{err}");
}

fn frozen_names() -> Vec<String> {
    RUN_CASES
        .iter()
        .filter(|n| !CURRENT_ONLY.contains(n))
        .map(|n| (*n).to_owned())
        .collect()
}

#[test]
fn the_floor_holds_at_todays_counts() {
    let all = names_of(RUN_CASES);
    let nine = frozen_names();
    assert_eq!(nine.len(), 9);
    assert_eq!(floor_of("v1"), (9, 0));
    assert_eq!(check_floor("current", &all, &[]), Ok(()));
    assert_eq!(check_floor("v1", &nine, &[]), Ok(()));
    assert!(check_floor("v1", &[], &nine).is_err());
}

#[test]
fn the_proof_keeps_its_floor() {
    for tree in TREES {
        let verdicts: Vec<(String, Verdict)> = RUN_CASES
            .iter()
            .filter_map(|name| verdict(tree, name).map(|v| ((*name).to_owned(), v)))
            .collect();
        let (proved, skipped) = split(&verdicts);
        eprintln!("savepoint proof, {tree}: {proved:?} proved, {skipped:?} skipped");
        check_floor(tree, &proved, &skipped).unwrap_or_else(|e| panic!("{e}"));
        if tree == "v1" {
            // Every frozen run case: a swapped case is seen.
            assert_eq!(proved, frozen_names(), "the proved v1 cases");
        }
    }
}

#[test]
fn the_cuts_cover_an_open_offer_and_a_mid_attack() {
    let mut moved = Vec::new();
    for (name, offer, enemy, player) in FEATURES {
        let case = load_case("current", name);
        let r = reference(&case);
        let seen = (
            r.offer.iter().any(|f| *f),
            r.enemy_mid.iter().any(|f| *f),
            r.player_mid.iter().any(|f| *f),
        );
        if seen != (offer, enemy, player) {
            moved.push(format!(
                "    (\"{name}\", {}, {}, {}),",
                seen.0, seen.1, seen.2
            ));
            continue;
        }
        let at = cuts(&r);
        assert!(at.len() <= 12 && at.windows(2).all(|w| w[0] < w[1]));
        if offer {
            assert!(
                at.iter().any(|t| r.offer[*t as usize - 1]),
                "{name}: no cut with an offer open"
            );
        }
        let flags = if enemy { &r.enemy_mid } else { &r.player_mid };
        if enemy || player {
            assert!(
                at.iter().any(|t| flags[*t as usize - 1]),
                "{name}: no cut strictly inside an attack"
            );
        }
    }
    assert!(
        moved.is_empty(),
        "the corpus changed: update FEATURES, computed:\n{}",
        moved.join("\n")
    );
}

#[test]
fn the_cuts_are_deterministic() {
    let r = reference(&load_case("current", "tutorial"));
    assert_eq!(cuts(&r), cuts(&r));
    assert_eq!(
        cuts(&reference(&load_case("current", "tutorial"))),
        cuts(&r)
    );
}

// The checker against restores that are wrong on purpose: each must come back `Err`, naming the
// case, the cut and what differs.

fn tutorial() -> (Case, Reference) {
    let case = load_case("current", "tutorial");
    let r = reference(&case);
    (case, r)
}

#[test]
fn a_restore_that_runs_one_tick_too_far_is_named() {
    let (case, r) = tutorial();
    let too_far = |rec: &RunRecorder, _: &[u8], _: &RunReplay, _: &Pack| {
        let mut rec = rec.clone();
        rec.step(&[Input::default(), Default::default(), Default::default()])
            .unwrap();
        rec
    };
    let err = check_run(&case, &r, &[100], &too_far).unwrap_err();
    assert!(err.contains("current/tutorial cut 100"), "{err}");
    assert!(err.contains("diverges first on tick 101"), "{err}");
}

#[test]
fn a_restore_with_the_wrong_hash_is_named() {
    let (case, mut r) = tutorial();
    r.hashes[17 - 1] ^= 1;
    let err = check_run(&case, &r, &[10], &real_restore).unwrap_err();
    assert!(err.contains("tutorial cut 10"), "{err}");
    assert!(err.contains("diverges first on tick 17"), "{err}");
}

#[test]
fn a_restore_with_another_runs_ledger_is_seen_though_the_hash_matches() {
    let (_, tutorial_ref) = tutorial();
    let trial = reference(&load_case("current", "time_limit"));
    assert_eq!(same_end(&tutorial_ref.end, &trial.end), Some("ledger"));
    // The checker compares the state the hash does not hold, not the hash alone.
    let mut other = tutorial_ref.end.clone();
    other.ledger = trial.end.ledger.clone();
    assert_eq!(other.hash, tutorial_ref.end.hash);
    assert_eq!(first_diff(&tutorial_ref.end, &other), Some("ledger"));
}

#[test]
fn a_restore_with_other_tallies_is_seen() {
    let (_, r) = tutorial();
    let mut other = r.end.clone();
    other.tally.0 += 1;
    assert_eq!(other.hash, r.end.hash);
    assert_eq!(first_diff(&r.end, &other), Some("tally"));
}

// A run whose enemy attack evaluates `rand` expressions, so `rng.attack` moves. No corpus case
// does, so a restore that leaves that stream fresh would pass every case above.

/// The tutorial mode with its first bat rule's `slash` replaced by an inline attack with three
/// `rand` expressions: a frozen copy, so a change to the bundled tutorial moves nothing here.
const RAND_TUTORIAL: &[u8] = include_bytes!("fixtures/savepoint_rng/tutorial_rand_attack.json");
const RAND_SEED: u64 = 20_261_010;
const RAND_TICKS: u32 = 1500;
/// Before the first draw (ticks 121 to 300), then after draws with draws still to come.
const RAND_CUTS: [u32; 4] = [100, 300, 700, 1200];

fn rand_pack() -> Pack {
    let mut files = ContentFiles::bundled().clone();
    assert!(files.insert("tutorial", RAND_TUTORIAL.to_vec()));
    Pack::from_bytes(&files.export().unwrap()).unwrap()
}

/// The run recorded here, not stored: `RAND_TICKS` ticks of the run script.
fn rand_attack_case() -> (Case, Reference) {
    let pack = rand_pack();
    let setup = RunSetup {
        mode: "tutorial".into(),
        seed: RAND_SEED,
        ..RunSetup::default()
    };
    let mut rec = RunRecorder::start_with_pack(&setup, &pack).unwrap();
    for tick in 0..RAND_TICKS {
        rec.step(&[
            common::run::run_script(tick),
            Default::default(),
            Default::default(),
        ])
        .unwrap();
    }
    assert_eq!(rec.engine().outcome(), Outcome::Running);
    let recorded = rec.replay();
    let case = Case {
        label: "fixture/tutorial_rand_attack".to_owned(),
        steps: RunSteps::of(&recorded),
        recorded,
        pack,
    };
    let reference = reference(&case);
    (case, reference)
}

fn rand_cuts(r: &Reference) -> Vec<u32> {
    let mut at = cuts(r);
    at.extend(RAND_CUTS);
    at.retain(|t| (1..r.ticks()).contains(t));
    at.sort_unstable();
    at.dedup();
    at
}

#[test]
fn the_fixture_pack_loads_and_is_not_the_bundled_one() {
    let pack = rand_pack();
    assert_ne!(pack.sha256(), Pack::bundled().sha256());
    let text = std::str::from_utf8(RAND_TUTORIAL).unwrap();
    assert_eq!(text.matches("\"rand\"").count(), 3);
}

#[test]
fn the_fixture_run_draws_from_the_attack_stream_around_the_cut() {
    let (case, r) = rand_attack_case();
    let fresh_stream = derive(RAND_SEED, TAG_ATTACK).state().to_le_bytes();
    let holds = |bytes: &[u8]| bytes.windows(8).any(|w| w == fresh_stream);
    let mut saved = vec![];
    play_from(&case, fresh(&case), 0, |t, rec| {
        if RAND_CUTS.contains(&t) {
            saved.push((t, holds(&rec.save())));
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        saved,
        [(100, true), (300, false), (700, false), (1200, false)],
        "the fresh attack stream is in the savepoint only before the first draw"
    );
    assert!(
        r.enemy_mid[1200..].contains(&true),
        "no enemy attack after the last cut, so no draw follows it"
    );
}

#[test]
fn the_fixture_run_is_running_at_its_last_tick() {
    let (_, r) = rand_attack_case();
    assert_eq!(r.end.outcome, Outcome::Running);
    assert_eq!(r.ticks(), RAND_TICKS);
}

#[test]
fn the_rand_attack_run_restores_to_the_uninterrupted_run() {
    let (case, r) = rand_attack_case();
    check_run(&case, &r, &rand_cuts(&r), &real_restore).unwrap_or_else(|e| panic!("{e}"));
}

// The recorded form of a frozen run (ADR-REPL-5, ADR-SAVE-4): a `sim_version` 1 recorder writes
// the schema 1 file the run was frozen in, and its picks go in through the bench-gated seam.

fn frozen_file(name: &str) -> (RunReplay, Vec<u8>, Pack) {
    let dir = tree_dir("v1").join("runs").join(name);
    let bytes = fs::read(dir.join("run.replay")).unwrap();
    let file = RunReplay::from_bytes(&bytes).unwrap();
    let pack = run_case_pack(&dir, &bytes).unwrap();
    (file, bytes, pack)
}

/// A recorder for the frozen `name`, driven through its first `upto` steps.
fn driven(name: &str, upto: usize) -> (RunRecorder, RunSteps, Pack, RunReplay) {
    let (file, _, pack) = frozen_file(name);
    let steps = RunSteps::of(&file);
    let mut rec = RunRecorder::start_with_pack(&file.setup, &pack).unwrap();
    for step in &steps.steps()[..upto.min(steps.steps().len())] {
        run_steps::apply_to(&mut rec, step);
    }
    (rec, steps, pack, file)
}

fn is_picks(s: &Step) -> bool {
    matches!(s, Step::Picks(_))
}

#[test]
fn a_version_1_recorder_writes_the_frozen_file() {
    for name in frozen_names() {
        let (file, bytes, _) = frozen_file(&name);
        let (rec, ..) = driven(&name, usize::MAX >> 1);
        let written = rec.replay();
        assert_eq!(written.setup.schema_version, 1, "{name}");
        assert_eq!(written.to_bytes(), bytes, "{name}");
        assert_eq!(written, file, "{name}");
    }
}

/// The hex of the SHA-256 of the bytes.
fn digest(bytes: &[u8]) -> String {
    stella_rain_core::content_hash::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn a_version_1_savepoint_carries_the_prefix_schema() {
    use crate::common::run::trial_setup;
    use crate::run_play::play_run;
    use stella_rain_core::run_replay::Command;
    // A version 1 recorder at a cut of the frozen `trial`.
    let (file, _, pack) = frozen_file("trial");
    let steps = RunSteps::of(&file);
    let (mut rec, _, _, _) = driven("trial", 0);
    for step in &steps.steps()[..steps.at(300)] {
        run_steps::apply_to(&mut rec, step);
    }
    let prefix = rec.replay();
    let bytes = rec.save();
    assert_eq!(bytes[12..16], 1u32.to_le_bytes());
    assert!(RunRecorder::resume_from_savepoint(&bytes, &prefix, &pack).is_ok());
    // A version 2 recorder's savepoint is what it was before.
    let mut live = RunRecorder::start_with_pack(&trial_setup(), Pack::bundled()).unwrap();
    for c in play_run(300) {
        if let Command::Step(i) = c {
            live.step(&[i, Default::default(), Default::default()])
                .unwrap();
        }
    }
    let saved = live.save();
    assert_eq!(saved[12..16], 2u32.to_le_bytes());
    assert_eq!(
        digest(&saved),
        "4c858a07db5c47661c9b4c18a3096ba180350385f45cd58bb2b7c772c663b946"
    );
}

#[test]
fn a_version_1_prefix_with_picks_resumes_and_saves_again() {
    let (file, _, pack) = frozen_file("trial");
    let steps = RunSteps::of(&file);
    // The prefix that ends on the second pick.
    let second = steps
        .steps()
        .iter()
        .enumerate()
        .filter(|(_, s)| is_picks(s))
        .nth(1)
        .map(|(i, _)| i)
        .expect("trial holds two picks");
    let (rec, ..) = driven("trial", second + 1);
    let prefix = rec.replay();
    let mut resumed = RunRecorder::resume_with_pack(&prefix, &pack).unwrap();
    let from = resumed.engine().tick();
    for step in &steps.steps()[second + 1..] {
        run_steps::apply_to(&mut resumed, step);
        if resumed.engine().tick() == from + 120 {
            break;
        }
    }
    let replay = resumed.replay();
    let bytes = replay.to_bytes();
    assert_eq!(RunReplay::from_bytes(&bytes), Ok(replay.clone()));
    assert_eq!(replay.setup.schema_version, 1);
    assert_ne!(replay.commands, "");
    let got = RunSteps::of(&replay);
    assert!(got.steps().iter().filter(|s| is_picks(s)).count() >= 2);
    assert_eq!(got.steps(), &steps.steps()[..got.steps().len()]);
    // The savepoint of the resumed run restores too.
    let save = resumed.save();
    assert!(RunRecorder::resume_from_savepoint(&save, &replay, &pack).is_ok());
}

#[test]
fn a_recorder_resumed_on_an_open_offer_continues() {
    let (file, _, pack) = frozen_file("tutorial");
    let steps = RunSteps::of(&file);
    // The first tick that leaves an offer open, and the step that answers it.
    let (mut rec, ..) = driven("tutorial", 0);
    let mut at = 0;
    for (i, step) in steps.steps().iter().enumerate() {
        run_steps::apply_to(&mut rec, step);
        if rec.engine().offer_open() {
            at = i + 1;
            break;
        }
    }
    assert!(at > 0, "tutorial opens an offer");
    assert!(is_picks(&steps.steps()[at]), "the offer is answered next");
    let prefix = rec.replay();
    let save = rec.save();
    let resumed = [
        RunRecorder::resume_with_pack(&prefix, &pack).unwrap(),
        RunRecorder::resume_from_savepoint(&save, &prefix, &pack).unwrap(),
    ];
    for mut r in resumed {
        let held = [Input::default(); 3];
        assert!(matches!(
            r.step(&held),
            Err(RunReplayError::OfferOpen { .. })
        ));
        for step in &steps.steps()[at..] {
            run_steps::apply_to(&mut r, step);
        }
        assert_eq!(r.replay(), file);
    }
}

#[test]
fn the_pick_seam_refuses_and_records_nothing() {
    use crate::common::run::trial_setup;
    let open = |rec: &RunRecorder| (rec.replay(), rec.engine().state_hash());
    // No pick at all.
    let (mut rec, ..) = driven("tutorial", 0);
    let before = open(&rec);
    assert!(matches!(
        rec.v1_pick_step(&[None; 3]),
        Err(RunReplayError::BadInputs(_))
    ));
    // A pick with no offer open (tick 0, before the first level).
    assert!(matches!(
        rec.v1_pick_step(&[Some(0), None, None]),
        Err(RunReplayError::ChoiceRefused { .. })
    ));
    assert_eq!(open(&rec), before);
    // An index no offer holds, with the offer open.
    let (file, ..) = frozen_file("tutorial");
    let steps = RunSteps::of(&file);
    for step in steps.steps() {
        run_steps::apply_to(&mut rec, step);
        if rec.engine().offer_open() {
            break;
        }
    }
    assert!(rec.engine().offer_open());
    let before = open(&rec);
    assert!(matches!(
        rec.v1_pick_step(&[Some(99), None, None]),
        Err(RunReplayError::ChoiceRefused { .. })
    ));
    assert_eq!(open(&rec), before);
    // The live rules have no offer.
    let mut live = RunRecorder::start_with_pack(&trial_setup(), Pack::bundled()).unwrap();
    let before = open(&live);
    assert!(matches!(
        live.v1_pick_step(&[Some(0), None, None]),
        Err(RunReplayError::ChoiceRefused { .. })
    ));
    assert_eq!(open(&live), before);
}

#[test]
fn the_v1_cuts_cover_an_open_offer() {
    for name in ["tutorial", "trial"] {
        let case = load_case("v1", name);
        let r = reference(&case);
        let at = cuts(&r);
        assert!(
            at.iter().any(|t| r.offer[*t as usize - 1]),
            "{name}: no cut with an offer open"
        );
        assert!(case.steps.steps().iter().any(is_picks), "{name}");
    }
}
