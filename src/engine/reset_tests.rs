//! An engine restored to its tick-0 copy with `Clone::clone_from` is the engine a constructor
//! builds (ADR-NET-3 points 8 and 9): the state hash, every field the hash does not cover, a later
//! join, and the ticks that follow. Over every corpus stage, run and party, at party sizes 1 to 3,
//! `sim_version` 1 included.

use std::path::Path;

use super::clone_from_tests::shown;
use super::savepoint_tests::{data_root, pack_of};
use super::*;
use crate::replay::Replay;
use crate::run_replay::RunReplay;
use crate::step_log::RangeStep;
use crate::validate::parse_and_validate;

/// The most ticks one restore point plays, so that the debug-profile run stays short.
const TICK_CAP: u32 = 1_200;

#[derive(Clone, Copy)]
enum Step {
    Tick([Input; 3]),
    Picks([Option<u32>; 3]),
    Actions([Option<EquipAction>; 3]),
    Join(PlayerIdx),
    Leave(PlayerIdx),
}

fn spread<T: Copy + Default>(mask: u8, dense: &[T]) -> [T; 3] {
    let mut out = [T::default(); 3];
    for (slot, value) in (0..3).filter(|s| mask >> s & 1 == 1).zip(dense) {
        out[slot] = *value;
    }
    out
}

fn slot(n: u8) -> PlayerIdx {
    PlayerIdx::new(n).expect("a slot of the log")
}

/// The party a run replay starts with and its steps, either schema.
fn steps_of(replay: &RunReplay) -> (u8, Vec<Step>) {
    let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
    let party = u8::try_from(range.present.count_ones()).unwrap();
    let mut mask = range.present;
    let mut out = Vec::new();
    for step in &range.steps {
        match step {
            RangeStep::Ticks { run, inputs } => {
                let all = spread(mask, inputs);
                out.extend((0..*run).map(|_| Step::Tick(all)));
            }
            RangeStep::Picks { picks } => out.push(Step::Picks(spread(mask, picks))),
            RangeStep::Actions { actions } => out.push(Step::Actions(spread(mask, actions))),
            RangeStep::Join { slot: s } => {
                mask |= 1 << s;
                out.push(Step::Join(slot(*s)));
            }
            RangeStep::Leave { slot: s } => {
                mask &= !(1 << s);
                out.push(Step::Leave(slot(*s)));
            }
        }
    }
    (party, out)
}

fn apply(engine: &mut Engine, step: Step) -> Result<(), String> {
    match step {
        Step::Tick(inputs) => engine.step(&inputs),
        Step::Picks(picks) => engine.v1_pick_step(&picks).map_err(|e| format!("{e:?}"))?,
        Step::Actions(actions) => {
            for (n, action) in actions.iter().enumerate() {
                if let Some(action) = *action {
                    let who = slot(u8::try_from(n).unwrap());
                    engine
                        .check_action(who, action)
                        .and_then(|()| engine.act(who, action))
                        .map_err(|e| format!("action of slot {n}: {e:?}"))?;
                }
            }
        }
        Step::Join(s) => engine.join(s).map_err(|e| format!("{e:?}"))?,
        Step::Leave(s) => engine.leave(s).map_err(|e| format!("{e:?}"))?,
    }
    Ok(())
}

/// Applies the steps from `next` on up to and including the next tick; false, and nothing
/// applied, when no tick is left.
fn advance(engine: &mut Engine, steps: &[Step], next: &mut usize) -> Result<bool, String> {
    let Some(tick) = steps[*next..]
        .iter()
        .position(|s| matches!(s, Step::Tick(_)))
    else {
        return Ok(false);
    };
    for step in &steps[*next..=*next + tick] {
        apply(engine, *step)?;
    }
    *next += tick + 1;
    Ok(true)
}

/// One engine a restore is tried on: how to build it, and what it plays.
struct Case {
    name: String,
    build: Box<dyn Fn() -> Engine>,
    steps: Vec<Step>,
    /// 1 for a `sim_version` 1 engine.
    version: u32,
}

fn dirs(tree: &Path, kind: &str) -> Vec<std::path::PathBuf> {
    let Ok(read) = std::fs::read_dir(tree.join(kind)) else {
        return Vec::new();
    };
    let mut v: Vec<_> = read.map(|d| d.unwrap().path()).collect();
    v.sort();
    v
}

fn name_of(tree: &str, kind: &str, dir: &Path) -> String {
    format!(
        "{tree}/{kind}/{}",
        dir.file_name().unwrap().to_string_lossy()
    )
}

/// Every engine of the matrix: the corpus stages at party sizes 1 to 3 (version 1 and the forced
/// rules of the `current/` stages at 1), the run cases at 1 to 3 (version 1 at 1) and the party
/// cases at the party they start with.
fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    for tree in ["current", "v1"] {
        let tree_dir = data_root::data_root().join("tests/corpus").join(tree);
        for dir in dirs(&tree_dir, "stages") {
            let stage =
                parse_and_validate(&std::fs::read(dir.join("stage.json")).unwrap()).unwrap();
            let recorded =
                Replay::from_bytes(&std::fs::read(dir.join("run.replay")).unwrap()).unwrap();
            let steps: Vec<Step> = recorded
                .unpack_inputs(stage.length_ticks)
                .unwrap()
                .into_iter()
                .map(|i| Step::Tick([i, Input::default(), Input::default()]))
                .collect();
            let pack = std::sync::Arc::new(pack_of(&tree_dir, &dir));
            let sizes: &[u8] = if tree == "current" { &[1, 2, 3] } else { &[] };
            for &n in sizes {
                let (stage, pack) = (stage.clone(), pack.clone());
                out.push(Case {
                    name: format!("{} party {n}", name_of(tree, "stages", &dir)),
                    build: Box::new(move || {
                        Engine::forced(2, || Engine::new_with(&stage, pack.content()).unwrap())
                            .into_party(n)
                    }),
                    steps: steps.clone(),
                    version: 2,
                });
            }
            // The frozen rules: the version 1 tree, and the current stages run as 1.
            let (stage, pack) = (stage.clone(), pack.clone());
            out.push(Case {
                name: format!("{} v1", name_of(tree, "stages", &dir)),
                build: Box::new(move || {
                    Engine::forced(1, || Engine::new_with(&stage, pack.content()).unwrap())
                }),
                steps,
                version: 1,
            });
        }
        for dir in dirs(&tree_dir, "runs") {
            let recorded =
                RunReplay::from_bytes(&std::fs::read(dir.join("run.replay")).unwrap()).unwrap();
            let (_, steps) = steps_of(&recorded);
            let pack = std::sync::Arc::new(pack_of(&tree_dir, &dir));
            let version = recorded.setup.sim_version;
            // A party above one is refused under `sim_version` 1.
            // A solo log's equipment actions name items a party's drops would not hold.
            let acts = steps.iter().any(|s| matches!(s, Step::Actions(_)));
            let sizes: &[u8] = if version == 1 || acts {
                &[1]
            } else {
                &[1, 2, 3]
            };
            for &n in sizes {
                let (setup, pack) = (recorded.setup.clone(), pack.clone());
                out.push(Case {
                    name: format!("{} party {n}", name_of(tree, "runs", &dir)),
                    build: Box::new(move || {
                        Engine::start_party_with(&setup, pack.content(), n).unwrap()
                    }),
                    steps: steps.clone(),
                    version,
                });
            }
        }
        for dir in dirs(&tree_dir, "parties") {
            let recorded =
                RunReplay::from_bytes(&std::fs::read(dir.join("run.replay")).unwrap()).unwrap();
            let (party, steps) = steps_of(&recorded);
            let pack = pack_of(&tree_dir, &dir);
            let setup = recorded.setup.clone();
            out.push(Case {
                name: format!("{} party {party}", name_of(tree, "parties", &dir)),
                build: Box::new(move || {
                    Engine::start_party_with(&setup, pack.content(), party).unwrap()
                }),
                steps,
                version: setup_version(&recorded),
            });
        }
    }
    out
}

fn setup_version(replay: &RunReplay) -> u32 {
    replay.setup.sim_version
}

/// What a case's checks visited.
#[derive(Default, Debug)]
struct Seen {
    engines: u32,
    restores: u32,
    ticks: u64,
}

/// Restores `d` over from `pristine`. The proof's restore is `clone_from`; the meta test passes
/// a wrong one.
type Restore = dyn Fn(&mut Engine, &Engine);

fn same(what: &str, case: &str, tick: u32, a: &Engine, b: &Engine) -> Result<(), String> {
    if a.state_hash() != b.state_hash() {
        return Err(format!(
            "{case}: hash differs after a restore at tick {tick}: {} vs {}",
            a.state_hash(),
            b.state_hash()
        ));
    }
    if shown(a) != shown(b) {
        return Err(format!("{case}: shown differs ({what}) at tick {tick}"));
    }
    if a.saved_parts().1 != b.saved_parts().1 {
        return Err(format!("{case}: tally differs ({what}) at tick {tick}"));
    }
    Ok(())
}

/// The restore check on one case: play a copy to each sample tick, restore it to the tick-0 copy,
/// and compare it with an engine the constructor built.
fn check(case: &Case, stranger: &Engine, restore: &Restore, seen: &mut Seen) -> Result<(), String> {
    let name = case.name.as_str();
    let fresh = (case.build)();
    let pristine = (case.build)().clone();
    let pristine_shown = shown(&pristine);
    let last = u32::try_from(
        case.steps
            .iter()
            .filter(|s| matches!(s, Step::Tick(_)))
            .count(),
    )
    .unwrap();
    seen.engines += 1;
    // A slot that holds another run altogether, whose own template a joiner is built from.
    let mut d = stranger.clone();
    restore(&mut d, &pristine);
    seen.restores += 1;
    same("over a stranger", name, 0, &d, &fresh)?;
    let (mut a, mut b) = (d.clone(), fresh.clone());
    if case.version >= 2 {
        let two = slot(2);
        if a.present() & 4 != 0 {
            a.leave(two)
                .and_then(|()| b.leave(two))
                .map_err(|e| format!("{name}: join: {e:?}"))?;
        }
        a.join(two)
            .and_then(|()| b.join(two))
            .map_err(|e| format!("{name}: join: {e:?}"))?;
        same("join over a stranger", name, 0, &a, &b)?;
    }
    let mut d = pristine.clone();
    let mut points = vec![1, 60, last.min(TICK_CAP)];
    points.retain(|t| *t <= last);
    points.dedup();
    for point in points {
        let mut next = 0;
        while d.tick() < point {
            if !advance(&mut d, &case.steps, &mut next).map_err(|e| format!("{name}: {e}"))? {
                break;
            }
        }
        if point >= 60 && d.state_hash() == fresh.state_hash() {
            return Err(format!("{name}: the engine did not move by tick {point}"));
        }
        seen.ticks += u64::from(d.tick());
        restore(&mut d, &pristine);
        seen.restores += 1;
        same("restore", name, point, &d, &fresh)?;
        if shown(&pristine) != pristine_shown {
            return Err(format!("{name}: the restore changed its source (shown)"));
        }

        // A later join: the template a joiner is built from, and the slot's generation.
        if case.version >= 2 {
            let (mut a, mut b) = (d.clone(), fresh.clone());
            let two = slot(2);
            if a.present() & 4 != 0 {
                a.leave(two)
                    .and_then(|()| b.leave(two))
                    .map_err(|e| format!("{name}: join: {e:?}"))?;
            }
            a.join(two)
                .and_then(|()| b.join(two))
                .map_err(|e| format!("{name}: join: {e:?}"))?;
            same("join", name, point, &a, &b).map_err(|e| e.replace("shown", "join shown"))?;
        }

        // The restored engine and the fresh one play on together.
        let (mut next, mut ticks) = (0, 0);
        let mut f = fresh.clone();
        loop {
            let start = next;
            if ticks == 20
                || !advance(&mut d, &case.steps, &mut next).map_err(|e| format!("{name}: {e}"))?
            {
                break;
            }
            for step in &case.steps[start..next] {
                apply(&mut f, *step).map_err(|e| format!("{name}: {e}"))?;
            }
            ticks += 1;
            if d.state_hash() != f.state_hash() {
                return Err(format!(
                    "{name}: steps differ: hash at tick {} after a restore at {point}",
                    d.tick()
                ));
            }
        }
        d.clone_from(&pristine);
    }
    Ok(())
}

fn restore_by_clone_from(d: &mut Engine, pristine: &Engine) {
    d.clone_from(pristine);
}

#[test]
fn a_restored_engine_hashes_as_a_fresh_one() {
    let cases = cases();
    let mut seen = Seen::default();
    let (mut stages, mut runs, mut parties) = (0, 0, 0);
    let (mut v1_stage, mut v1_run) = (false, false);
    let mut sizes = std::collections::BTreeSet::new();
    for (i, case) in cases.iter().enumerate() {
        let stranger = (cases[(i + cases.len() - 1) % cases.len()].build)();
        check(case, &stranger, &restore_by_clone_from, &mut seen).unwrap_or_else(|e| panic!("{e}"));
        if case.name.contains("/stages/") {
            stages += 1;
            v1_stage |= case.version == 1;
        } else if case.name.contains("/runs/") {
            runs += 1;
            v1_run |= case.version == 1;
        } else {
            parties += 1;
        }
        if case.name.starts_with("current/") && case.version == 2 {
            sizes.insert(case.name.rsplit(' ').next().unwrap().to_string());
        }
    }
    eprintln!("reset: {} cases, {seen:?}", cases.len());
    // The stage and run cases: 32 stages, 23 runs, 19 parties, floors.
    assert!(stages >= 32, "{stages} stage engines");
    assert!(runs >= 23, "{runs} run engines");
    assert!(parties >= 19, "{parties} party engines");
    assert!(v1_stage && v1_run, "a frozen engine of each kind");
    assert!(sizes.len() >= 3, "party sizes {sizes:?}");
}

fn case_named(name: &str) -> Case {
    cases()
        .into_iter()
        .find(|c| c.name.contains(name))
        .unwrap_or_else(|| panic!("no case {name}"))
}

#[test]
fn the_check_names_what_a_wrong_restore_leaves() {
    let case = case_named("current/parties/three_time_limit");
    let mut seen = Seen::default();
    let stranger = (case_named("v1/stages/agents").build)();
    check(&case, &stranger, &restore_by_clone_from, &mut seen)
        .expect("the right restore is accepted");

    // A restore that leaves the engine one tick on: the hash.
    let one_on = |d: &mut Engine, p: &Engine| {
        d.clone_from(p);
        d.step(&[Input::default(); 3]);
    };
    let e = check(&case, &stranger, &one_on, &mut seen).unwrap_err();
    assert!(
        e.contains("hash") && e.contains("three_time_limit") && e.contains("tick"),
        "{e}"
    );

    // A restore that leaves a kill in the ledger: what the hash does not cover.
    let dirty = |d: &mut Engine, p: &Engine| {
        d.clone_from(p);
        d.ledger.add_kill("x".to_string());
    };
    let e = check(&case, &stranger, &dirty, &mut seen).unwrap_err();
    assert!(e.contains("shown") && e.contains("three_time_limit"), "{e}");

    // A restore that leaves slot 2 out of the party.
    let hole = |d: &mut Engine, p: &Engine| {
        d.clone_from(p);
        d.leave(slot(2)).unwrap();
    };
    let e = check(&case, &stranger, &hole, &mut seen).unwrap_err();
    assert!(e.contains("three_time_limit") && e.contains("tick"), "{e}");
}

#[test]
fn a_restore_does_not_change_its_source() {
    let case = case_named("current/parties/three_time_limit");
    let pristine = (case.build)();
    let before = shown(&pristine);
    let mut d = pristine.clone();
    let mut next = 0;
    while d.tick() < 60 {
        assert!(advance(&mut d, &case.steps, &mut next).unwrap());
    }
    d.clone_from(&pristine);
    assert_eq!(shown(&pristine), before, "after a restore");
    let mut next = 0;
    while d.tick() < 60 {
        assert!(advance(&mut d, &case.steps, &mut next).unwrap());
    }
    assert_eq!(
        shown(&pristine),
        before,
        "after 60 more ticks of the restored"
    );
}
