//! The savepoint determinism proof (ADR-SAVE-3, ADR-SIM-3): for every run case of the replay
//! corpus, in the current tree and in the frozen `v1/`, a savepoint taken at a cut, restored and
//! run to the end gives the hashes, the ledger, the tallies, the snapshot and the replay of the
//! uninterrupted run. `check_run` is generic over the restore, so the meta-tests can hand it
//! restores that are wrong on purpose and see it name them.
//!
//! The stage cases are not here: a stage engine has no `RunSetup`, so it has no savepoint file,
//! and the state codec behind it is crate-private (the library's own `savepoint_tests` cover
//! the stage cases through it).

use crate::corpus_pins::{case_dirs, run_case_pack};
use std::fs;
use std::path::{Path, PathBuf};

use stella_rain_core::engine::Outcome;
use stella_rain_core::input::Input;
use stella_rain_core::ledger::RunLedger;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::{RunRecorder, RunReplay, RunReplayError, TickCommand};
use stella_rain_core::snapshot::Snapshot;

type Plain = (u32, Vec<(String, u8)>, Vec<String>, Vec<(String, u8)>);

const TREES: [&str; 2] = ["current", "v1"];

/// One run case of one tree, read.
struct Case {
    label: String,
    recorded: RunReplay,
    pack: Pack,
    inputs: Vec<Input>,
    commands: Vec<TickCommand>,
}

fn tree_dir(tree: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(tree)
}

fn load_case(tree: &str, name: &str) -> Case {
    let dir = tree_dir(tree).join("runs").join(name);
    let bytes = fs::read(dir.join("run.replay")).unwrap();
    let recorded = RunReplay::from_bytes(&bytes).unwrap();
    let pack = run_case_pack(&dir, &bytes).unwrap();
    let inputs = recorded.unpack_inputs(recorded.ticks).unwrap();
    let commands = recorded.unpack_commands().unwrap();
    Case {
        label: format!("{tree}/{name}"),
        recorded,
        pack,
        inputs,
        commands,
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
    let p = &s.player;
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

/// Applies the commands at `tick` from `at` on; returns where the next ones start.
fn apply_commands(
    rec: &mut RunRecorder,
    commands: &[TickCommand],
    mut at: usize,
    tick: u32,
) -> Result<usize, String> {
    while at < commands.len() && commands[at].tick == tick {
        rec.choose(commands[at].index)
            .map_err(|e| format!("choose at tick {tick}: {e:?}"))?;
        at += 1;
    }
    Ok(at)
}

/// Plays the case from `rec` at tick `from` to its last tick, commands before each input as the
/// run did, calling `visit(t, rec)` right after tick `t` and before the commands that follow it.
fn play_from(
    case: &Case,
    mut rec: RunRecorder,
    from: u32,
    mut visit: impl FnMut(u32, &RunRecorder) -> Result<(), String>,
) -> Result<RunRecorder, String> {
    let mut at = case.commands.partition_point(|c| c.tick < from);
    for i in from..case.recorded.ticks {
        at = apply_commands(&mut rec, &case.commands, at, i)?;
        rec.step(case.inputs[i as usize])
            .map_err(|e| format!("{}: step {}: {e:?}", case.label, i + 1))?;
        visit(i + 1, &rec)?;
    }
    apply_commands(&mut rec, &case.commands, at, case.recorded.ticks)?;
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
            let refused = restored.step(Input::default());
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

fn prove(tree: &str, name: &str, sweep: bool) {
    let case = load_case(tree, name);
    let r = reference(&case);
    let at = if sweep { sweep_cuts(&r) } else { cuts(&r) };
    assert!(!at.is_empty(), "{}: no cuts", case.label);
    check_run(&case, &r, &at, &real_restore).unwrap_or_else(|e| panic!("{e}"));
}

macro_rules! run_cases {
    ($($name:ident),* $(,)?) => {
        const RUN_CASES: &[&str] = &[$(stringify!($name)),*];
        $(
            mod $name {
                #[test]
                fn current() {
                    super::prove("current", stringify!($name), false);
                }

                #[test]
                fn v1() {
                    super::prove("v1", stringify!($name), false);
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
    tutorial,
);

/// The run cases of each tree, as (offer open, enemy mid-attack, player mid-attack) seen
/// somewhere in the uninterrupted run. A re-bless that moves one fails the guard below.
const FEATURES: [(&str, bool, bool, bool); 9] = [
    ("easy", true, true, true),
    ("elites", true, true, true),
    ("rainy", true, true, true),
    ("stage_skill", true, true, true),
    ("time_limit", true, true, true),
    ("time_limit_kill", true, false, true),
    ("time_limit_overtime", true, false, true),
    ("trial", true, true, true),
    ("tutorial", true, true, true),
];

#[test]
fn the_case_lists_are_the_directories() {
    for tree in TREES {
        let dirs = case_dirs(&tree_dir(tree).join("runs"));
        let names: Vec<String> = dirs
            .iter()
            .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        let mut listed: Vec<&str> = RUN_CASES.to_vec();
        listed.sort_unstable();
        assert_eq!(
            names, listed,
            "{tree}: the run cases changed; list the new case in run_cases! and FEATURES"
        );
    }
    let features: Vec<&str> = FEATURES.iter().map(|f| f.0).collect();
    assert_eq!(features, RUN_CASES, "FEATURES lists the cases in order");
}

#[test]
fn the_cuts_cover_an_open_offer_and_a_mid_attack() {
    for (name, offer, enemy, player) in FEATURES {
        let case = load_case("current", name);
        let r = reference(&case);
        let seen = (
            r.offer.iter().any(|f| *f),
            r.enemy_mid.iter().any(|f| *f),
            r.player_mid.iter().any(|f| *f),
        );
        assert_eq!(
            seen,
            (offer, enemy, player),
            "the corpus changed: update FEATURES, the suite no longer covers an open offer or a \
             mid-attack in {name}"
        );
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
        rec.step(Input::default()).unwrap();
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
    let trial = reference(&load_case("current", "trial"));
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

#[test]
fn a_restore_that_forgets_an_open_offer_is_named() {
    let (case, r) = tutorial();
    let cut = (1..r.ticks()).find(|t| r.offer[*t as usize - 1]).unwrap();
    let answered = |rec: &RunRecorder, _: &[u8], _: &RunReplay, _: &Pack| {
        let mut rec = rec.clone();
        rec.choose(0).unwrap();
        rec
    };
    let err = check_run(&case, &r, &[cut], &answered).unwrap_err();
    assert!(err.contains(&format!("tutorial cut {cut}")), "{err}");
    assert!(err.contains("offer_open"), "{err}");
}
