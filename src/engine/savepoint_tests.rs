//! `encode_state` and `decode_state` (ADR-SIM-3): the whole run state round-trips, on a hand-built
//! state and on every state of the replay corpus; a decode checks its lengths and indices
//! against a fresh engine and refuses damaged bytes without panicking.

// The indices a test sets are the length of a table of a few hundred entries at most.
#![allow(clippy::cast_possible_truncation)]

use std::path::Path;

#[allow(dead_code, reason = "shared with tests/, which uses more of it")]
pub(super) mod data_root {
    include!("../../tests/data_root.rs");
}

use super::hash_coverage_tests::{engine, full_state};
use super::run::v1::State;
use super::*;
use crate::id::EntityId;
use crate::input::EquipAction;
use crate::pack::Pack;
use crate::replay::Replay;
use crate::run_replay::RunReplay;
use crate::savepoint::{DecodeError, encode_state};
use crate::step_log::RangeStep;
use crate::validate::parse_and_validate;

type Decoded = Result<(State, RunLedger, Counted), DecodeError>;

/// `decode_state` of an engine that runs version 1, the rules these tests edit.
fn decode_state(bytes: &[u8], fresh: &Engine) -> Decoded {
    let (saved, ledger, counted, _) = crate::savepoint::decode_state(bytes, fresh)?;
    Ok((saved.v1(), ledger, counted))
}

/// An engine holding the full state, a ledger and tallies with something in each field.
fn saved() -> Engine {
    let mut e = engine();
    let full = full_state(&e);
    e.run.set_state(full);
    // The index is derived from the runs; the fixture's was lowered by hand.
    let Run::V1 { world, state, .. } = &mut e.run else {
        unreachable!("a test engine runs version 1")
    };
    state.move_index.rebuild(world, &state.runs);
    e.ledger.add_kill("kills.giant_sword.enemy".into());
    e.ledger.add_kill("rune_taken.burn".into());
    e.ledger.add_bestiary_bit(&ContentId::from("slime"), 3);
    e.ledger.picks.push(crate::ledger::StarPick {
        rarity: crate::rewards::Rarity::Rare,
        name: Some("Vega".into()),
    });
    e.ledger.picks.push(crate::ledger::StarPick {
        rarity: crate::rewards::Rarity::Common,
        name: None,
    });
    e.ledger.add_skill_use(ContentId::from("dash"));
    e.counted.kills = 5;
    e.counted.named.push("Vega".into());
    e
}

/// `edit` applied to a copy of the full state, encoded, and decoded against the unedited one.
fn decode_edited(edit: impl FnOnce(&mut State)) -> Decoded {
    let fresh = saved();
    let mut edited = fresh.clone();
    edit(&mut edited.run);
    decode_state(&encode_state(&edited), &fresh)
}

/// What a decode gives, as the original engine's `Debug` text of the same parts.
fn debug_of(s: &State) -> String {
    format!(
        "{:?} {:?} {:?} {:?} {:?}",
        s.kinds, s.match_ups, s.runes, s.stun, s.move_index
    )
}

#[test]
fn round_trip_hand_built() {
    let e = saved();
    let bytes = encode_state(&e);
    let (state, ledger, counted) = decode_state(&bytes, &e).unwrap();
    let (_, _, orig, orig_ledger, orig_counted) = {
        let (a, b, c, d, f) = e.savepoint_parts();
        (a, b, c, d, f)
    };
    assert_eq!(ledger, *orig_ledger);
    assert_eq!(counted, *orig_counted);
    assert_eq!(debug_of(&state), debug_of(orig));
    let mut back = e.clone();
    back.run.set_state(state);
    assert_eq!(back.state_hash(), e.state_hash());
}

#[test]
fn shape_is_checked() {
    let progress = |f: fn(&mut Progress)| decode_edited(|s| f(s.progress.as_mut().unwrap()));
    let stars = |f: fn(&mut StarField)| decode_edited(|s| f(s.stars.as_mut().unwrap()));
    assert!(matches!(
        decode_edited(|s| s.spawned.push(0)).map(drop),
        Err(DecodeError::Invalid(
            "spawned length differs from the content"
        ))
    ));
    assert!(matches!(
        progress(|p| p.spent.push(0)).map(drop),
        Err(DecodeError::Invalid(
            "spent length differs from the content"
        ))
    ));
    assert!(matches!(
        progress(|p| p.spent.clear()).map(drop),
        Err(DecodeError::Invalid(
            "spent length differs from the content"
        ))
    ));
    assert!(matches!(
        stars(|f| f.taken.push(0)).map(drop),
        Err(DecodeError::Invalid(
            "star taken length differs from the content"
        ))
    ));
    assert!(matches!(
        stars(|f| {
            f.taken.pop();
        })
        .map(drop),
        Err(DecodeError::Invalid(
            "star taken length differs from the content"
        ))
    ));
    assert!(matches!(
        decode_edited(|s| s.heal_pickups = None).map(drop),
        Err(DecodeError::Invalid(
            "heal_pickups presence differs from the content"
        ))
    ));
    assert!(matches!(
        decode_edited(|s| s.horde = None).map(drop),
        Err(DecodeError::Invalid(
            "horde presence differs from the content"
        ))
    ));
    assert!(matches!(
        decode_edited(|s| s.max_attack_speed_pct += 1).map(drop),
        Err(DecodeError::Invalid(
            "max_attack_speed_pct differs from the content"
        ))
    ));
    // The tallies' lengths come from the rewards file too.
    let fresh = saved();
    let mut edited = fresh.clone();
    edited.counted.taken.push(0);
    assert!(matches!(
        decode_state(&encode_state(&edited), &fresh).map(drop),
        Err(DecodeError::Invalid(
            "counted taken length differs from the content"
        ))
    ));
}

/// One saved index: how to set it to a given value, and what it indexes.
struct Index {
    message: &'static str,
    set: fn(&mut State, usize),
    /// The length of what it indexes, in an engine.
    len: fn(&Engine) -> usize,
}

fn indices() -> Vec<Index> {
    use crate::kinds::KindIndex;
    use crate::sim_v1::{AttackIndex, BossIndex, DefIndex, ProgramIndex};
    vec![
        Index {
            message: "agent def out of range",
            set: |s, n| s.enemies[0].def = DefIndex(n as u16),
            len: |e| e.run.world().defs.len(),
        },
        Index {
            message: "agent kind out of range",
            set: |s, n| s.enemies[0].kind = KindIndex(n as u16),
            len: |e| e.run.kinds.len(),
        },
        Index {
            message: "boss index out of range",
            set: |s, n| s.boss.as_mut().unwrap().boss = BossIndex(n as u16),
            len: |e| e.run.world().bosses.len(),
        },
        Index {
            message: "boss kind out of range",
            set: |s, n| s.boss.as_mut().unwrap().kind = KindIndex(n as u16),
            len: |e| e.run.kinds.len(),
        },
        Index {
            message: "part kind out of range",
            set: |s, n| s.boss.as_mut().unwrap().parts[0].kind = KindIndex(n as u16),
            len: |e| e.run.kinds.len(),
        },
        Index {
            message: "run attack out of range",
            set: |s, n| s.runs[0].attack = AttackIndex(n as u16),
            len: |e| e.run.world().attacks.len(),
        },
        Index {
            message: "task program out of range",
            set: |s, n| s.tasks[0].program = ProgramIndex(n as u32),
            len: |e| e.run.world().emitters.len(),
        },
        Index {
            message: "orb program out of range",
            set: |s, n| s.orbs[0].program = ProgramIndex(n as u32),
            len: |e| e.run.world().emitters.len(),
        },
        Index {
            message: "zone program out of range",
            set: |s, n| s.zones[0].program = ProgramIndex(n as u32),
            len: |e| e.run.world().emitters.len(),
        },
        Index {
            message: "hook program out of range",
            set: |s, n| s.orbs[0].hook.as_mut().unwrap().program = ProgramIndex(n as u32),
            len: |e| e.run.world().emitters.len(),
        },
        Index {
            message: "orbit reward out of range",
            set: |s, n| s.player.orbits[0].reward = n as u16,
            len: |e| e.content.rewards().rewards().len(),
        },
        Index {
            message: "skill index out of range",
            set: |s, n| s.player.skill.as_mut().unwrap().index = n as u16,
            len: |e| e.content.rewards().stage_skills().skills().len(),
        },
        Index {
            message: "star pool entry out of range",
            set: |s, n| s.stars.as_mut().unwrap().pool[0] = n as u16,
            len: |e| e.content.rewards().rewards().len(),
        },
        Index {
            message: "offer candidate out of range",
            set: |s, n| s.stars.as_mut().unwrap().offer.as_mut().unwrap().candidates[0] = n as u16,
            len: |e| e.content.rewards().rewards().len(),
        },
    ]
}

#[test]
fn indices_are_checked() {
    let fresh = saved();
    for index in indices() {
        let len = (index.len)(&fresh);
        assert!(
            len > 0,
            "{}: the full state's engine indexes something",
            index.message
        );
        assert_eq!(
            decode_edited(|s| (index.set)(s, len)).map(drop),
            Err(DecodeError::Invalid(index.message)),
            "{} at {len}",
            index.message
        );
        assert!(
            decode_edited(|s| (index.set)(s, len - 1)).is_ok(),
            "{} at {}",
            index.message,
            len - 1
        );
    }
}

#[test]
fn a_run_phase_past_its_attack_is_refused_before_the_move_index_is_built() {
    let fresh = saved();
    let phases = fresh.run.world().attacks[usize::from(fresh.run.runs[0].attack.0)]
        .phases
        .len();
    assert_eq!(
        decode_edited(|s| s.runs[0].phase = phases as u32).map(drop),
        Err(DecodeError::Invalid("run phase out of range"))
    );
    assert!(decode_edited(|s| s.runs[0].phase = phases as u32 - 1).is_ok());
}

/// The first byte at which two encodings differ.
fn first_diff(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .expect("they differ")
}

fn encoded(edit: impl FnOnce(&mut State)) -> Vec<u8> {
    let mut e = saved();
    edit(&mut e.run);
    encode_state(&e)
}

#[test]
fn bytes_are_checked() {
    let fresh = saved();
    let empty = engine();
    let bare = {
        let mut e = saved();
        e.run.progress = None;
        e.run.stars = None;
        e
    };
    for (name, e) in [("full", &fresh), ("fresh", &empty), ("bare", &bare)] {
        let bytes = encode_state(e);
        assert!(decode_state(&bytes, e).is_ok() || name == "bare", "{name}");
        for n in 0..bytes.len() {
            let r = decode_state(&bytes[..n], e);
            assert!(
                matches!(r, Err(DecodeError::Truncated | DecodeError::Invalid(_))),
                "{name}: a prefix of {n} bytes decoded"
            );
        }
    }
    let mut bytes = encode_state(&fresh);
    bytes.push(0);
    assert_eq!(
        decode_state(&bytes, &fresh).map(drop),
        Err(DecodeError::Invalid("trailing bytes"))
    );

    // A byte patched in an otherwise valid encoding: found as the first byte two encodings of
    // the same field with different values differ at.
    let patch = |a: Vec<u8>, b: Vec<u8>, bytes: &[u8]| {
        let mut out = a.clone();
        let at = first_diff(&a, &b);
        out[at..at + bytes.len()].copy_from_slice(bytes);
        out
    };
    let long = patch(
        encoded(|s| s.ranged_cooling.clear()),
        encoded(|s| s.ranged_cooling.truncate(1)),
        &[0xff; 4],
    );
    assert_eq!(
        decode_state(&long, &fresh).map(drop),
        Err(DecodeError::Invalid("list too long"))
    );
    use crate::rewards::Rarity;
    let outcome = patch(
        encoded(|s| s.outcome = Outcome::Running),
        encoded(|s| s.outcome = Outcome::Failed),
        &[3],
    );
    assert!(matches!(
        decode_state(&outcome, &fresh),
        Err(DecodeError::Invalid(_))
    ));
    let rarity = patch(
        encoded(|s| s.stars.as_mut().unwrap().rarity.0 = Rarity::Common),
        encoded(|s| s.stars.as_mut().unwrap().rarity.0 = Rarity::Uncommon),
        &[6],
    );
    assert!(matches!(
        decode_state(&rarity, &fresh),
        Err(DecodeError::Invalid(_))
    ));
    let boolean = patch(
        encoded(|s| s.progress.as_mut().unwrap().pick_stats = false),
        encoded(|s| s.progress.as_mut().unwrap().pick_stats = true),
        &[2],
    );
    assert_eq!(
        decode_state(&boolean, &fresh).map(drop),
        Err(DecodeError::Invalid("bool"))
    );
}

// ---- The replay corpus ----

/// How to start a case's engine again, and what it plays.
struct Case {
    name: String,
    start: Box<dyn Fn() -> Engine>,
    steps: Vec<Step>,
}

/// One step of a solo case: a world tick, or a pick or an equipment action, which do not advance
/// the world (ADR-REPL-5).
#[derive(Clone, Copy)]
enum Step {
    Tick(Input),
    Pick(u32),
    Act(EquipAction),
}

/// The steps of a solo run, either schema, read through `step_range` (a pick or an action keeps
/// its place before the tick it was made on).
fn steps_of(replay: &RunReplay) -> Vec<Step> {
    let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
    assert_eq!(range.present, 1, "a solo run");
    let mut out = Vec::new();
    for step in range.steps {
        match step {
            RangeStep::Ticks { run, inputs } => {
                out.extend((0..run).map(|_| Step::Tick(inputs[0])));
            }
            RangeStep::Picks { picks } => out.push(Step::Pick(picks[0].unwrap())),
            RangeStep::Actions { actions } => out.push(Step::Act(actions[0].unwrap())),
            RangeStep::Join { .. } | RangeStep::Leave { .. } => panic!("a solo run"),
        }
    }
    out
}

fn apply_step(engine: &mut Engine, step: Step) {
    match step {
        Step::Tick(input) => engine.step(&[input, Default::default(), Default::default()]),
        Step::Pick(i) => crate::engine::pick(engine, crate::id::PlayerIdx::SOLO, i).unwrap(),
        Step::Act(a) => engine.act(crate::id::PlayerIdx::SOLO, a).unwrap(),
    }
}

/// Applies the steps from `next` on up to and including the next tick; false, and nothing
/// applied, when the case holds no more ticks.
fn advance(engine: &mut Engine, steps: &[Step], next: &mut usize) -> bool {
    let Some(tick) = steps[*next..]
        .iter()
        .position(|s| matches!(s, Step::Tick(_)))
    else {
        return false;
    };
    for step in &steps[*next..=*next + tick] {
        apply_step(engine, *step);
    }
    *next += tick + 1;
    true
}

fn pack_of(tree: &Path, case: &Path) -> Pack {
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(case.join("case.json")).unwrap()).unwrap();
    let pin = json["pack_sha256"].as_str().unwrap();
    match std::fs::read(tree.join("packs").join(format!("{pin}.json"))) {
        Ok(bytes) => Pack::from_bytes(&bytes).unwrap(),
        Err(_) => Pack::bundled().clone(),
    }
}

fn cases(tree: &str) -> Vec<Case> {
    let tree_dir = data_root::data_root().join("tests/corpus").join(tree);
    let mut out = Vec::new();
    let dirs = |kind: &str| -> Vec<std::path::PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(tree_dir.join(kind))
            .unwrap()
            .map(|d| d.unwrap().path())
            .collect();
        v.sort();
        v
    };
    for dir in dirs("stages") {
        let name = format!(
            "{tree}/stages/{}",
            dir.file_name().unwrap().to_string_lossy()
        );
        let stage = parse_and_validate(&std::fs::read(dir.join("stage.json")).unwrap()).unwrap();
        let recorded = Replay::from_bytes(&std::fs::read(dir.join("run.replay")).unwrap()).unwrap();
        let inputs = recorded.unpack_inputs(stage.length_ticks).unwrap();
        let pack = pack_of(&tree_dir, &dir);
        out.push(Case {
            name,
            // The codec read here is the frozen state's: the current cases, at 2, run as 1.
            start: Box::new(move || {
                Engine::forced(1, || Engine::new_with(&stage, pack.content()).unwrap())
            }),
            steps: inputs.into_iter().map(Step::Tick).collect(),
        });
    }
    for dir in dirs("runs") {
        let name = format!("{tree}/runs/{}", dir.file_name().unwrap().to_string_lossy());
        let recorded =
            RunReplay::from_bytes(&std::fs::read(dir.join("run.replay")).unwrap()).unwrap();
        let steps = steps_of(&recorded);
        let pack = pack_of(&tree_dir, &dir);
        let setup = recorded.setup.clone();
        out.push(Case {
            name,
            // The setup names its own rules: version 2 is the equipment mode, version 1 the frozen
            // offers rules.
            start: Box::new(move || Engine::start_with(&setup, pack.content()).unwrap()),
            steps,
        });
    }
    out
}

/// Saves `engine` at its tick and checks what a resume must give back.
fn check_savepoint(case: &Case, engine: &Engine, next: usize) {
    let at = format!("{} tick {}", case.name, engine.tick());
    let bytes = encode_state(engine);
    let fresh = (case.start)();
    let (state, ledger, counted) =
        decode_state(&bytes, &fresh).unwrap_or_else(|e| panic!("{at}: {e:?}"));
    let (_, _, orig, orig_ledger, orig_counted) = engine.savepoint_parts();
    // (e) the ledger and the tallies, with the indices the ledger rebuilds
    assert_eq!(&ledger, orig_ledger, "{at}: ledger");
    assert_eq!(&counted, orig_counted, "{at}: counted");
    // (c) the move index
    for run in &orig.runs {
        assert_eq!(
            state.move_index.get(run.owner),
            orig.move_index.get(run.owner),
            "{at}: move index"
        );
    }
    assert_eq!(state.move_index.get(EntityId(100)), 100, "{at}");
    // (d) the derived tables and the content copies
    assert_eq!(debug_of(&state), debug_of(orig), "{at}: derived");
    assert_eq!(
        format!("{:?}", state.progress.as_ref().map(|p| &p.stats)),
        format!("{:?}", orig.progress.as_ref().map(|p| &p.stats)),
        "{at}: stats"
    );
    assert_eq!(
        format!(
            "{:?}",
            state
                .horde
                .as_ref()
                .map(|h| (&h.boss_ticks, &h.no_call_ticks, &h.mid_boss_hp_pct))
        ),
        format!(
            "{:?}",
            orig.horde
                .as_ref()
                .map(|h| (&h.boss_ticks, &h.no_call_ticks, &h.mid_boss_hp_pct))
        ),
        "{at}: horde times"
    );
    let mut resumed = engine.clone();
    resumed.run.set_state(state);
    resumed.ledger = ledger;
    resumed.counted = counted;
    // (a) the hash, (f) the bytes, which hold every saved index field
    assert_eq!(resumed.state_hash(), engine.state_hash(), "{at}: hash");
    assert_eq!(encode_state(&resumed), bytes, "{at}: re-encoded bytes");
    // (g) both run on for 30 ticks
    let mut original = engine.clone();
    let (mut a, mut b) = (next, next);
    for i in 0..30 {
        if !advance(&mut original, &case.steps, &mut a) {
            break;
        }
        assert!(advance(&mut resumed, &case.steps, &mut b));
        assert_eq!(
            resumed.state_hash(),
            original.state_hash(),
            "{at}: {} ticks after the resume",
            i + 1
        );
    }
}

/// `check_savepoint` for a run of the live rules (an equipment run, whose state is not version
/// 1's): the decode swaps the saved parts into a fresh engine, which has the original's hash and
/// bytes and goes on 30 ticks as the original does.
fn check_live_savepoint(case: &Case, engine: &Engine, next: usize) {
    let at = format!("{} tick {}", case.name, engine.tick());
    let bytes = encode_state(engine);
    let fresh = (case.start)();
    let (state, ledger, counted, stats) =
        crate::savepoint::decode_state(&bytes, &fresh).unwrap_or_else(|e| panic!("{at}: {e:?}"));
    let mut resumed = fresh.with_saved(state, ledger, counted, stats);
    assert_eq!(resumed.state_hash(), engine.state_hash(), "{at}: hash");
    assert_eq!(encode_state(&resumed), bytes, "{at}: re-encoded bytes");
    let mut original = engine.clone();
    let (mut a, mut b) = (next, next);
    for i in 0..30 {
        if !advance(&mut original, &case.steps, &mut a) {
            break;
        }
        assert!(advance(&mut resumed, &case.steps, &mut b));
        assert_eq!(
            resumed.state_hash(),
            original.state_hash(),
            "{at}: {} ticks after the resume",
            i + 1
        );
    }
}

#[test]
fn corpus_round_trip() {
    let (mut live, mut actions) = (0, 0);
    for tree in ["current", "v1"] {
        for case in cases(tree) {
            let mut engine = (case.start)();
            actions += case
                .steps
                .iter()
                .filter(|s| matches!(s, Step::Act(_)))
                .count();
            let mut next = 0;
            let last = case
                .steps
                .iter()
                .filter(|s| matches!(s, Step::Tick(_)))
                .count();
            for tick in 0..=last {
                let wants = tick == 0 || tick % 60 == 0 || tick == last || engine.offer_open();
                if wants && matches!(engine.run(), Run::V2 { .. }) {
                    live += 1;
                    check_live_savepoint(&case, &engine, next);
                } else if wants {
                    check_savepoint(&case, &engine, next);
                }
                if tick == last {
                    break;
                }
                assert!(advance(&mut engine, &case.steps, &mut next));
            }
        }
    }
    // Not vacuous: the equipment runs of `current/` go through the live round trip, actions
    // included.
    assert!(
        live > 100 && actions > 0,
        "{live} live checks, {actions} actions"
    );
}

#[test]
fn a_decoded_ledger_goes_on_counting_where_the_original_does() {
    let mut e = engine();
    // A kill key that sorts before the weapon's own shifts the kill indices (`add_kill`).
    e.ledger.add_kill("rune_taken.burn".into());
    e.ledger.add_kill("a_first_key".into());
    let (_, ledger, _) = decode_state(&encode_state(&e), &e).unwrap();
    let mut original = e.ledger.clone();
    let mut resumed = ledger;
    assert_eq!(resumed, original);
    for key in ["a_first_key", "kills.giant_sword.enemy", "b_new_key"] {
        original.add_kill(key.into());
        resumed.add_kill(key.into());
    }
    assert_eq!(resumed, original);
    assert_eq!(resumed.stat("kills.giant_sword.enemy"), 1);
}

/// The size of a savepoint at the worst ticks the benches count: a record for the PR, not a
/// gate. Each decodes against the same engine and encodes again to the same bytes.
#[cfg(feature = "bench")]
#[test]
fn encoded_size_of_the_bench_fixtures() {
    use crate::bench_fixtures::{self, offers_cases};
    use crate::savepoint::Encode;
    // The state read here is the frozen version's (`savepoint_parts`): build the cases at 1, in
    // offers mode (the equipped cases build the live rules only).
    let engines = Engine::forced(1, || {
        [
            ("worst_tick", offers_cases::worst_tick().engine),
            ("hit_tick", offers_cases::hit_tick().engine),
            ("time_limit_tick", offers_cases::time_limit_tick().engine),
            (
                "worst_minute_peak",
                bench_fixtures::worst_minute_peak().engine,
            ),
        ]
    });
    for (name, e) in engines {
        let (_, _, state, ledger, counted) = e.savepoint_parts();
        let size = |f: &dyn Fn(&mut crate::savepoint::Writer)| {
            let mut w = crate::savepoint::Writer::new();
            f(&mut w);
            w.finish().len()
        };
        eprintln!(
            "savepoint size {name}: state {} ledger {} counted {}",
            size(&|w| state.encode(w)),
            size(&|w| ledger.encode(w)),
            size(&|w| counted.encode(w)),
        );
        let bytes = encode_state(&e);
        let (s, l, c) = decode_state(&bytes, &e).unwrap_or_else(|err| panic!("{name}: {err:?}"));
        let mut back = e.clone();
        (*back.run, back.ledger, back.counted) = (s, l, c);
        assert_eq!(encode_state(&back), bytes, "{name}");
    }
}
