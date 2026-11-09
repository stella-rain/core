//! `Engine::tally`: the run's HUD tallies (kills, rewards taken, named stars, constellation tiers)
//! as the events said them. A resumed run reports the tally of the same run played straight, and
//! the tally equals a fold of the events the way the app counts them.

use crate::common::run_steps::{self, RunSteps, Step};
use crate::common::{self, *};
use crate::corpus::runs_dir;
use crate::corpus_pins::case_pack;
use stella_rain_core::pack::Pack;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome, Tally};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::run_replay::{RunRecorder, RunReplay};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::EntityKind;

type Plain = (u32, Vec<(String, u8)>, Vec<String>, Vec<(String, u8)>);

/// A view as values to compare: kills, taken and tiers sorted by ID, names in event order.
fn plain(t: Tally<'_>) -> Plain {
    let mut taken: Vec<(String, u8)> = t.taken().map(|(id, n)| (id.to_string(), n)).collect();
    taken.sort();
    let mut tiers: Vec<(String, u8)> = t.tiers().map(|(id, n)| (id.to_string(), n)).collect();
    tiers.sort();
    (
        t.kills(),
        taken,
        t.named().iter().map(ToString::to_string).collect(),
        tiers,
    )
}

/// What `play.gd` counts from the events, the definition the tally keeps.
#[derive(Clone, Default, PartialEq, Debug)]
struct Fold {
    kills: u32,
    taken: BTreeMap<String, u8>,
    named: Vec<String>,
    tiers: BTreeMap<String, u8>,
    elite_kills: u32,
}

impl Fold {
    /// Adds the events; true when any of them counts.
    fn add(&mut self, events: &[DomainEvent]) -> bool {
        let mut counted = false;
        for e in events {
            match e {
                DomainEvent::Died { class, elite, .. } if *class == EntityKind::Enemy => {
                    self.kills += 1;
                    self.elite_kills += u32::from(*elite);
                    counted = true;
                }
                DomainEvent::RewardTaken {
                    reward,
                    taken,
                    star,
                    ..
                } => {
                    self.taken.insert(reward.to_string(), *taken);
                    if let Some(name) = star.as_ref().filter(|n| !n.is_empty()) {
                        self.named.push(name.to_string());
                    }
                    counted = true;
                }
                DomainEvent::Constellation { set, tier } => {
                    self.tiers.insert(set.to_string(), *tier);
                    counted = true;
                }
                _ => {}
            }
        }
        counted
    }

    fn plain(&self) -> Plain {
        (
            self.kills,
            self.taken.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            self.named.clone(),
            self.tiers.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        )
    }
}

fn case_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(runs_dir())
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A run the tally tests walk: a corpus case (equipment mode) or the offers-era `time_limit` run,
/// which the test records itself with the offers path (only the offers path emits `RewardTaken`
/// and `Constellation`, so only it has a taken reward, a named star or a tier to count).
struct Run {
    name: String,
    replay: RunReplay,
    pack: Pack,
}

/// The name the offers-era `time_limit` run goes by in these tests.
const OFFERS_TIME_LIMIT: &str = "offers time_limit";

fn offers_time_limit() -> Run {
    Run {
        name: OFFERS_TIME_LIMIT.into(),
        replay: crate::corpus::offers_time_limit_run(),
        pack: Pack::bundled().clone(),
    }
}

fn corpus_runs() -> Vec<Run> {
    case_names()
        .into_iter()
        .map(|name| {
            let case: PathBuf = runs_dir().join(&name);
            let replay =
                RunReplay::from_bytes(&fs::read(case.join("run.replay")).unwrap()).unwrap();
            Run {
                pack: case_pack(&case).unwrap(),
                replay,
                name,
            }
        })
        .collect()
}

/// The corpus cases and the offers-era `time_limit` run.
fn all_runs() -> Vec<Run> {
    let mut runs = corpus_runs();
    runs.push(offers_time_limit());
    runs
}

/// Plays a run through a recorder, calling `f(recorder, ordinal, counted)` after every step
/// and every choice, where `counted` says the last events changed the fold. Returns the fold.
fn walk(
    run: &Run,
    stop_at: Option<usize>,
    mut f: impl FnMut(&RunRecorder, usize, bool, &Fold),
) -> Fold {
    let recorded = &run.replay;
    let mut rec = RunRecorder::start_with_pack(&recorded.setup, &run.pack).unwrap();
    let mut fold = Fold::default();
    let mut ordinal = 0;
    for step in RunSteps::of(recorded).steps() {
        if matches!(step, Step::Tick(_))
            && (rec.engine().outcome() != Outcome::Running || rec.engine().offer_open())
        {
            break;
        }
        run_steps::apply_to(&mut rec, step);
        ordinal += 1;
        let counted = fold.add(rec.engine().events());
        f(&rec, ordinal, counted, &fold);
        if stop_at == Some(ordinal) {
            break;
        }
    }
    fold
}

#[test]
fn a_resumed_run_reports_the_tally_of_the_run_played_straight() {
    let runs = all_runs();
    for must in ["easy", "rainy", "time_limit", "trial", "tutorial"] {
        assert!(runs.iter().any(|r| r.name == must), "no run case {must}");
    }
    for run in &runs {
        let name = &run.name;
        // First pass: where the tally changes, and the first open offer and choice.
        let mut changes = Vec::new();
        let mut first_offer = None;
        let end = walk(run, None, |rec, ordinal, counted, _| {
            if counted {
                changes.push(ordinal);
            }
            if first_offer.is_none() && rec.engine().offer_open() {
                first_offer = Some(ordinal);
            }
        });
        let mut cuts: Vec<usize> = vec![1];
        cuts.extend(first_offer);
        let thin = 12;
        for k in 0..thin.min(changes.len()) {
            cuts.push(changes[k * changes.len() / thin.min(changes.len())]);
        }
        if let Some(last) = changes.last() {
            cuts.push(*last);
            cuts.push(*last + 1);
        }
        if let Some(c) = first_offer {
            cuts.push(c + 1);
        }
        cuts.sort_unstable();
        cuts.dedup();
        let pack = &run.pack;
        let mut compared = 0;
        for cut in cuts {
            let mut at = None;
            walk(run, Some(cut), |rec, ordinal, _, fold| {
                if ordinal == cut {
                    at = Some((rec.clone(), fold.plain()));
                }
            });
            let Some((rec, folded)) = at else { continue };
            if rec.engine().outcome() != Outcome::Running {
                continue;
            }
            let straight = plain(rec.engine().tally());
            assert_eq!(
                straight, folded,
                "{name}: straight tally vs the fold at {cut}"
            );
            let resumed = RunRecorder::resume_with_pack(&rec.replay(), pack).unwrap();
            assert_eq!(
                plain(resumed.engine().tally()),
                straight,
                "{name}: resumed at ordinal {cut}"
            );
            assert_eq!(resumed.engine().state_hash(), rec.engine().state_hash());
            compared += 1;
        }
        assert!(compared > 0, "{name}: no cut compared");
        if ["easy", "rainy", "time_limit", OFFERS_TIME_LIMIT].contains(&name.as_str()) {
            assert!(end.kills > 0, "{name}: no kills");
        }
        eprintln!(
            "tally resume: {name}: {compared} cuts, {} changes",
            changes.len()
        );
    }
}

#[test]
fn the_tally_equals_a_fold_of_the_events_on_every_corpus_run() {
    let mut cases = 0;
    for run in all_runs() {
        let name = &run.name;
        let mut compared = 0;
        let end = walk(&run, None, |rec, _, counted, fold| {
            if counted {
                assert_eq!(
                    plain(rec.engine().tally()),
                    fold.plain(),
                    "{name} at tick {}",
                    rec.engine().tick()
                );
                compared += 1;
            }
        });
        assert!(compared > 0 || end.kills == 0, "{name}");
        cases += 1;
    }
    assert!(cases >= 10, "{cases} run cases");
}

#[test]
fn the_corpus_reaches_every_tally() {
    let mut sum = Fold::default();
    let mut named_max = 0;
    let mut tiers_seen = BTreeMap::new();
    for run in all_runs() {
        let name = &run.name;
        let mut last: Option<Plain> = None;
        let end = walk(&run, None, |rec, _, _, _| {
            last = Some(plain(rec.engine().tally()));
        });
        assert_eq!(last.unwrap(), end.plain(), "{name}: the tally at the end");
        sum.kills += end.kills;
        sum.elite_kills += end.elite_kills;
        named_max = named_max.max(end.named.len());
        sum.taken.extend(end.taken.clone());
        for (k, v) in &end.tiers {
            tiers_seen.insert(format!("{name}/{k}"), *v);
        }
        eprintln!(
            "tally: {name}: kills {}, elites {}, taken {}, named {}, tiers {:?}",
            end.kills,
            end.elite_kills,
            end.taken.len(),
            end.named.len(),
            end.tiers
        );
    }
    assert!(sum.kills > 0, "no kills in the corpus");
    assert!(sum.elite_kills > 0, "no elite kill in the corpus");
    assert!(!sum.taken.is_empty(), "no reward taken in the corpus");
    assert!(named_max > 0, "no named star in the corpus");
    assert!(!tiers_seen.is_empty(), "no tier in the corpus");
    // The offers-era `time_limit` run (the old case's seed) still names at least 11 stars.
    let tl = walk(&offers_time_limit(), None, |_, _, _, _| {});
    assert!(
        tl.named.len() >= 11,
        "time_limit names {} stars",
        tl.named.len()
    );
    // The corpus `time_limit` case is an equipment run: its tally holds the kills only, because
    // only the offers path emits `RewardTaken` and `Constellation`.
    let corpus_tl = corpus_runs()
        .into_iter()
        .find(|r| r.name == "time_limit")
        .unwrap();
    let tl = walk(&corpus_tl, None, |_, _, _, _| {});
    assert!(tl.kills > 0, "the corpus time_limit kills");
    assert_eq!(
        tl.elite_kills, TIME_LIMIT_ELITE_KILLS,
        "elite kills of the corpus time_limit"
    );
    assert!(
        tl.taken.is_empty() && tl.named.is_empty() && tl.tiers.is_empty(),
        "an equipment run counts no reward, star or tier"
    );
}

// --- built stages -------------------------------------------------------------------------------

/// The elites the corpus `time_limit` case kills (computed from the blessed file).
const TIME_LIMIT_ELITE_KILLS: u32 = 0;

const CX: i32 = 20_000;
const CY: i32 = 30_000;

/// A companion at the corner strikes the `foe` next to it once, in a one-tick strike of 10.
fn striker(foe: Value) -> stella_rain_core::stage::Stage {
    let strike = json!({ "type": "attack", "attack": { "inline": [{
        "kind": "active", "ticks": 1, "damage": 10,
        "hit_box": { "type": "circle", "radius": 4_096 } }] } });
    let rules = json!([{ "when": always(), "target": "nearest_enemy", "once": true,
                         "do": strike }]);
    stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, foe, CX + 2_560, CY)],
        vec![companion(50, hold(CX, CY), rules)],
    )
}

fn kills_after(stage: &stella_rain_core::stage::Stage, ticks: u32) -> (u32, u32) {
    let mut e = engine(stage);
    for _ in 0..ticks {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    let ledger = e.ledger();
    let ledgered: u32 = ledger
        .kills
        .iter()
        .filter(|(k, _)| k.ends_with(".enemy") || k.ends_with(".elite"))
        .map(|(_, n)| *n)
        .sum();
    (e.tally().kills(), ledgered)
}

#[test]
fn the_tally_counts_what_the_app_counts_and_the_ledger_does_not() {
    let plain_foe = enemy(10, hold(CX + 2_560, CY), json!([]));
    assert_eq!(
        kills_after(&striker(plain_foe), 3).0,
        1,
        "an ordinary enemy"
    );
    let mut elite = enemy(10, hold(CX + 2_560, CY), json!([]));
    elite["elite"] = json!({ "range": 256 });
    assert_eq!(kills_after(&striker(elite), 3).0, 1, "an elite");

    // A companion dying is not a kill.
    let shooter = enemy(
        50,
        hold(46_080, 100_000),
        json!([rule(
            always(),
            json!("nearest_ally"),
            json!({ "type": "attack",
                    "attack": { "preset": { "id": "aimed_single", "args": [15] } } }),
            0
        )]),
    );
    let stage = stage_full(
        18_000,
        Some(far_boss()),
        vec![wave(1, 1, 0, shooter, 46_080, 100_000)],
        vec![companion(3, hold(46_080, 130_000), json!([]))],
    );
    assert_eq!(kills_after(&stage, 120).0, 0, "a companion");

    // A boss dying is not a kill either.
    let boss = json!({ "base": "golem", "hp": 20, "radius": 8192,
                       "spawn": { "x": 46_080, "y": 60_000 }, "phases": [{}] });
    let stage = stage_full(18_000, Some(boss), vec![], vec![]);
    let mut e = engine(&stage);
    let mut died = false;
    for tick in 0..200 {
        e.step(&[
            common::firing(idle(), tick),
            Default::default(),
            Default::default(),
        ]);
        died |= e.events().iter().any(|ev| {
            matches!(
                ev,
                DomainEvent::Died {
                    class: EntityKind::Boss,
                    ..
                }
            )
        });
    }
    assert!(died, "the boss died");
    assert_eq!(e.tally().kills(), 0, "a boss");
}

fn carried_engine() -> Engine {
    let setup = RunSetup {
        carried: ["star_altair", "star_deneb"]
            .iter()
            .map(|id| stella_rain_core::run_setup::CarriedStar {
                reward: (*id).into(),
            })
            .collect(),
        lumine: 10,
        ..common::run::trial_setup()
    };
    Engine::start(&setup).unwrap()
}

#[test]
fn a_carried_star_is_held_but_not_taken() {
    let e = carried_engine();
    let held = &e.snapshot().constellations;
    assert!(
        held.iter().any(|c| c.held == 2),
        "the state holds the carried pair"
    );
    assert_eq!(e.tally().taken().count(), 0);
    assert_eq!(e.tally().tiers().count(), 0);
    assert_eq!(e.tally().named().len(), 0);
}

#[test]
fn a_cloned_engine_keeps_its_tally() {
    let mut last = None;
    let easy = corpus_runs()
        .into_iter()
        .find(|r| r.name == "easy")
        .unwrap();
    walk(&easy, None, |rec, _, _, _| {
        last = Some(rec.engine().clone())
    });
    let e = last.unwrap();
    let clone = e.clone();
    assert!(e.tally().kills() > 0, "nothing was killed in `easy`");
    assert_eq!(plain(clone.tally()), plain(e.tally()));
}

#[test]
fn tally_changes_nothing() {
    let setup = common::run::time_limit_setup();
    let mut e = Engine::start(&setup).unwrap();
    for _ in 0..1800 {
        e.step(&[Default::default(), Default::default(), Default::default()]);
    }
    let (hash, tick, events) = (e.state_hash(), e.tick(), e.events().to_vec());
    let ledger = e.ledger().clone();
    let (a, b) = (plain(e.tally()), plain(e.tally()));
    let _ = plain(e.tally());
    assert_eq!(a, b);
    assert_eq!(e.state_hash(), hash);
    assert_eq!(e.tick(), tick);
    assert_eq!(e.events(), events.as_slice());
    assert_eq!(*e.ledger(), ledger);
}
