//! The gameplay observables pin (ADR-SIM-7). One digest per case over everything a
//! player or the app can read from a run: the events of every tick, the outcome, the snapshot
//! (every tick of a stage case, every `SNAPSHOT_STRIDE`-th tick and the last of a run or bench
//! case), then the ledger and the kill tally at the end. It leaves out the state hash and the
//! savepoint bytes only: `hashes.bin`, `final_hash`, `bench_pins.toml` and the savepoint pin hold
//! those. A change of how entities are numbered must leave every digest as it is, and a change
//! that moves one without adding an observable is a gameplay change.
//!
//! Entity IDs are compared through `map`: the live rules number the player slots 0 to 2 and the
//! other entities from 3, so an ID of 3 or more is mapped down by `ID_GAP` (the gap of a party of
//! one) to the number it had when the slots were not reserved. Every ID is an `EntityId`, whose
//! `Debug` text is `EntityId(n)`, so one pass over the text of each value finds all of them.
//!
//! On a mismatch the test prints every computed digest. A new case directory without a pin
//! fails; there is no switch that rewrites the pins.

use std::fmt::{Debug, Write as _};
use std::path::Path;

use sha2::{Digest, Sha256};
use stella_rain_core::bench_fixtures::worst_minute_case::{Run, input_at};
use stella_rain_core::bench_fixtures::{self, Case, hit_case, time_limit_case, worst_case};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::replay::Replay;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::validate::parse_and_validate;

use crate::common::run_steps::{self, RunSteps, Step};
use crate::corpus::current_dir;
use crate::corpus_pins::{case_dirs, case_pack, run_case_pack};

/// How many entity numbers the player slots reserve beyond the one slot a solo party holds.
const ID_GAP: u32 = 2;
/// The snapshot is digested on every this-many-th tick of a run or bench case, and on the last.
const SNAPSHOT_STRIDE: u32 = 30;
/// The ticks a bench tick case plays after its warm-up.
const BENCH_TICKS: u32 = 120;
/// The ticks of the dense minute case.
const MINUTE_PLAYED: u32 = 600;

/// An entity ID as the pre-reservation numbering had it.
fn map(n: u32) -> u32 {
    if n < 3 { n } else { n - ID_GAP }
}

/// `text` with every `EntityId(n)` rewritten to `EntityId(map(n))`: the numbers the version 2
/// rules gave an entity before the player slots were reserved.
pub(crate) fn rewrite_ids(text: &str) -> String {
    const TAG: &str = "EntityId(";
    let mut rest = text;
    let mut mapped = String::new();
    while let Some(at) = rest.find(TAG) {
        let (before, tail) = rest.split_at(at + TAG.len());
        mapped.push_str(before);
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let n: u32 = tail[..digits].parse().expect("an EntityId holds a number");
        write!(mapped, "{}", map(n)).expect("a String takes writes");
        rest = &tail[digits..];
    }
    mapped.push_str(rest);
    mapped
}

/// `text` with `, offer: None` put back before the closing ` }` of every `ProgressView { .. }`
/// that has no top-level `offer:` field: the field left the snapshot, and the digests were
/// recorded when it was there.
pub(crate) fn restore_removed_fields(text: &str) -> String {
    const TAG: &str = "ProgressView {";
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(TAG) {
        let (before, tail) = rest.split_at(at + TAG.len());
        out.push_str(before);
        // Find the matching `}`, noting whether a top-level `offer:` field is there.
        let (mut depth, mut quoted, mut escaped) = (1u32, false, false);
        let mut end = tail.len();
        for (i, c) in tail.char_indices() {
            if quoted {
                match (escaped, c) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, '"') => quoted = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => quoted = true,
                '{' | '[' | '(' => depth += 1,
                '}' | ']' | ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &tail[..end];
        let has_offer = body.starts_with(" offer: ") || {
            let (mut depth, mut found) = (0u32, false);
            let bytes = body.as_bytes();
            for (i, b) in bytes.iter().enumerate() {
                match b {
                    b'{' | b'[' | b'(' => depth += 1,
                    b'}' | b']' | b')' => depth = depth.saturating_sub(1),
                    b',' if depth == 0 && body[i..].starts_with(", offer: ") => found = true,
                    _ => {}
                }
            }
            found
        };
        out.push_str(body.strip_suffix(' ').unwrap_or(body));
        if !has_offer {
            out.push_str(", offer: None");
        }
        if body.ends_with(' ') {
            out.push(' ');
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// `text` without the fields added to views, while they are empty (every added feature is
/// dormant): `TreeReadyView.kinds`, `TreeStatsView.grants` (the style kinds) and
/// `threat_colour` of `AttackView` and `ZoneView` (the threat colours), `gauge`,
/// `spirit`, `counter`, `tempo`, `trigger`, `rally` and `perfect_evade` of `PlayerView` (the weapon gauge, the spirit level, the counter window, the combo tempo, the giant trigger, the rally and the perfect evade), `stagger` and
/// `stagger_max` of `EntityView` (the stagger gauge), and `repeat: false` of a `Hit` event (the
/// step repeat), `counterable: false` of an `AttackView` and `super_armour` of a `PlayerView`
/// (the counter and super armour; the armour flag is derived from the charge and the running
/// smash step, so it reads true in a shipped run while a smash swings, whichever value, it is
/// no gameplay: only a `breaks_charge` hit reads it, and no shipped phase has one). The digests were
/// recorded before the fields existed, and an empty one changes no gameplay. `OrbBroken`'s
/// `player`, the last field of the one event that has one, is dropped too: it names who broke
/// the orb, and the event's other fields and the tick it comes on are still digested. A view that names
/// a kind or a colour keeps its field and so moves the digest.
pub(crate) fn drop_empty_added_fields(text: &str) -> String {
    text.replace(", kinds: [] }", " }")
        .replace(", counterable: false }", " }")
        .replace(", super_armour: false", "")
        .replace(", super_armour: true", "")
        .replace(", grants: [] }", " }")
        .replace(", threat_colour: None }", " }")
        .replace(", repeat: false, player:", ", player:")
        .replace(", stagger: None, stagger_max: None }", " }")
        .replace(
            ", gauge: None, spirit: None, counter: None, tempo: None, trigger: None, rally: None, perfect_evade: None",
            "",
        )
        .replace(", player: Some(First) }", " }")
        .replace(", player: Some(Second) }", " }")
        .replace(", player: Some(Third) }", " }")
}

/// Feeds `text` to `sha` with its entity IDs mapped.
fn feed_mapped(sha: &mut Sha256, text: &str) {
    let text = drop_empty_added_fields(text);
    sha.update(rewrite_ids(&restore_removed_fields(&text)).as_bytes());
    sha.update(b"\n");
}

fn feed(sha: &mut Sha256, value: &impl Debug) {
    feed_mapped(sha, &format!("{value:?}"));
}

/// What one tick adds to the digest.
fn observe_tick(sha: &mut Sha256, engine: &Engine, snapshot: bool) {
    sha.update(engine.tick().to_le_bytes());
    feed(sha, &engine.events());
    feed(sha, &engine.outcome());
    if snapshot {
        feed(sha, &engine.snapshot());
    }
}

/// What the end of a case adds.
fn observe_end(sha: &mut Sha256, engine: &Engine) {
    feed(sha, engine.ledger());
    sha.update(engine.tally().kills().to_le_bytes());
}

fn hex16(sha: Sha256) -> String {
    sha.finalize()[..8]
        .iter()
        .fold(String::new(), |mut hex, b| {
            write!(hex, "{b:02x}").expect("a String takes writes");
            hex
        })
}

fn sampled(tick: u32, last: bool) -> bool {
    last || tick % SNAPSHOT_STRIDE == 0
}

fn bench_tick_case(case: fn() -> Case, input_of: fn(u32) -> Input) -> String {
    let Case { mut engine, .. } = case();
    let mut sha = Sha256::new();
    for i in 0..BENCH_TICKS {
        let input = input_of(engine.tick() + 1);
        engine.step(&[input, Default::default(), Default::default()]);
        observe_tick(
            &mut sha,
            &engine,
            sampled(engine.tick(), i + 1 == BENCH_TICKS),
        );
    }
    observe_end(&mut sha, &engine);
    hex16(sha)
}

fn minute_case() -> String {
    let mut run = Run::fast_forwarded(0);
    let mut sha = Sha256::new();
    for i in 0..MINUTE_PLAYED {
        let input = input_at(run.engine.tick());
        run.engine
            .step(&[input, Default::default(), Default::default()]);
        observe_tick(
            &mut sha,
            &run.engine,
            sampled(run.engine.tick(), i + 1 == MINUTE_PLAYED),
        );
    }
    observe_end(&mut sha, &run.engine);
    hex16(sha)
}

/// A case's name and the function that plays it.
type Named = (&'static str, fn() -> String);

fn bench_digests() -> Vec<(String, String)> {
    let cases: Vec<Named> = vec![
        ("worst_tick", || {
            bench_tick_case(bench_fixtures::worst_tick, worst_case::input)
        }),
        ("hit_tick", || {
            bench_tick_case(bench_fixtures::hit_tick, hit_case::input)
        }),
        ("hit_tick_spread", || {
            bench_tick_case(bench_fixtures::hit_tick_spread, hit_case::input)
        }),
        ("time_limit_tick", || {
            bench_tick_case(bench_fixtures::time_limit_tick, time_limit_case::input)
        }),
        ("worst_minute", minute_case),
    ];
    in_parallel(cases)
}

fn in_parallel(cases: Vec<Named>) -> Vec<(String, String)> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = cases
            .iter()
            .map(|(name, run)| (name, scope.spawn(run)))
            .collect();
        handles
            .into_iter()
            .map(|(name, handle)| {
                (
                    format!("bench/{name}"),
                    handle.join().expect("the case runs"),
                )
            })
            .collect()
    })
}

fn stage_digest(case: &Path) -> String {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let stage_bytes = std::fs::read(case.join("stage.json")).expect("stage.json");
    let stage = parse_and_validate(&stage_bytes).expect("the stage parses");
    let recorded = Replay::from_bytes(&std::fs::read(case.join("run.replay")).expect("run.replay"))
        .expect("the replay parses");
    let inputs = recorded.unpack_inputs(stage.length_ticks).expect("inputs");
    let pack = case_pack(case).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut engine = Engine::new_with(&stage, pack.content()).expect("the stage starts");
    let mut sha = Sha256::new();
    for input in &inputs {
        engine.step(&[*input, Default::default(), Default::default()]);
        observe_tick(&mut sha, &engine, true);
    }
    observe_end(&mut sha, &engine);
    hex16(sha)
}

fn run_digest(case: &Path) -> String {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let bytes = std::fs::read(case.join("run.replay")).expect("run.replay");
    let recorded = RunReplay::from_bytes(&bytes).expect("the run replay parses");
    let pack = run_case_pack(case, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut engine = run_steps::start(&recorded, pack.content());
    let steps = RunSteps::of(&recorded);
    let played = steps.ticks;
    let mut sha = Sha256::new();
    for step in steps.steps() {
        if engine.tick() >= played {
            break;
        }
        run_steps::apply(&mut engine, step);
        if matches!(step, Step::Tick(_)) {
            observe_tick(
                &mut sha,
                &engine,
                sampled(engine.tick(), engine.tick() == played),
            );
        }
    }
    observe_end(&mut sha, &engine);
    hex16(sha)
}

fn dir_digests(sub: &str, digest: fn(&Path) -> String) -> Vec<(String, String)> {
    let dirs = case_dirs(&current_dir().join(sub));
    assert!(!dirs.is_empty(), "no cases in {sub}");
    std::thread::scope(|scope| {
        let handles: Vec<_> = dirs
            .iter()
            .map(|dir| {
                let name = dir.file_name().unwrap().to_string_lossy().into_owned();
                (format!("{sub}/{name}"), scope.spawn(move || digest(dir)))
            })
            .collect();
        handles
            .into_iter()
            .map(|(name, h)| (name, h.join().expect("the case runs")))
            .collect()
    })
}

/// The pins: first 16 hex of the digest, one per case. Set from the first run on the code before
/// the entity numbers moved; they are hand-written and move only with a new observable.
const PINS: &[(&str, &str)] = &[
    ("bench/hit_tick", "84e781ed85a26a66"),
    ("bench/hit_tick_spread", "280b626db0770637"),
    ("bench/time_limit_tick", "739405859ee4f707"),
    ("bench/worst_minute", "a1b9b4a0a7079014"),
    ("bench/worst_tick", "f7615e32fdd91f74"),
    ("runs/easy", "61bd66634c00214a"),
    ("runs/elites", "0e9d871c7d318c31"),
    ("runs/rainy", "76c02eef3bc455dd"),
    ("runs/stage_skill", "e560d0df31d08c84"),
    ("runs/time_limit", "570adca54e774559"),
    ("runs/time_limit_kill", "7c928e9542da8420"),
    ("runs/time_limit_overtime", "d5c1cc710ee0fe42"),
    ("runs/trial", "a564357c714a0451"),
    ("runs/trial_kinds", "9c04299f028b0568"),
    ("runs/tutorial", "beb18841e5fc4644"),
    ("runs/tutorial_opening", "3194c62bb4c75399"),
    ("stages/agents", "9eba992e81c1061d"),
    ("stages/assist", "bc7ec48a962a8b7c"),
    ("stages/boss", "d0a4b766d2a70ea4"),
    ("stages/clear", "0391deae5ea50761"),
    ("stages/dash", "b4e48c58d746ce2d"),
    ("stages/evade", "7ae1ea0434b63872"),
    ("stages/fail", "d131ada2820d1aa9"),
    ("stages/landscape", "295a2b596497a93c"),
    ("stages/leash_edge", "0592ca55c4027b9f"),
    ("stages/melee", "6f0108d5ca5bf6d5"),
    ("stages/orb_cage", "b5d7930fd7e34671"),
    ("stages/orbs", "9895492022635383"),
    ("stages/ranged_cap", "089be77e0e72f7d1"),
    ("stages/timeout", "ff31edf8c0ebd0ed"),
    ("stages/warn_anchor", "caf50889a40491ad"),
    ("stages/zones", "3267106e794011ca"),
];

fn check(computed: Vec<(String, String)>) {
    let mut wrong = String::new();
    for (name, digest) in &computed {
        match PINS.iter().find(|(pinned, _)| pinned == name) {
            Some((_, pin)) if pin == digest => {}
            Some((_, pin)) => {
                writeln!(wrong, "  {name}: pinned {pin}, computed {digest}").unwrap();
            }
            None => writeln!(wrong, "  {name}: no pin, computed {digest}").unwrap(),
        }
    }
    let mut all = String::new();
    for (name, digest) in &computed {
        writeln!(all, "    (\"{name}\", \"{digest}\"),").unwrap();
    }
    assert!(
        wrong.is_empty(),
        "the gameplay observables moved:\n{wrong}computed:\n{all}"
    );
}

#[test]
fn the_bench_cases_observables_are_pinned() {
    check(bench_digests());
}

#[test]
fn the_corpus_stage_cases_observables_are_pinned() {
    check(dir_digests("stages", stage_digest));
}

#[test]
fn the_corpus_run_cases_observables_are_pinned() {
    check(dir_digests("runs", run_digest));
}

#[test]
fn every_pin_names_a_case() {
    let mut names: Vec<String> = bench_names();
    for sub in ["stages", "runs"] {
        for dir in case_dirs(&current_dir().join(sub)) {
            names.push(format!(
                "{sub}/{}",
                dir.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    for (pinned, _) in PINS {
        assert!(names.iter().any(|n| n == pinned), "{pinned} is not a case");
    }
}

fn bench_names() -> Vec<String> {
    [
        "worst_tick",
        "hit_tick",
        "hit_tick_spread",
        "time_limit_tick",
        "worst_minute",
    ]
    .iter()
    .map(|n| format!("bench/{n}"))
    .collect()
}

#[test]
fn the_id_map_leaves_the_slots_and_moves_the_rest_by_the_gap() {
    assert_eq!(map(0), 0);
    assert_eq!(map(2), 2);
    assert_eq!(map(3), 3 - ID_GAP);
    let before = format!("x EntityId({}) y Some(EntityId(0))", 3 + ID_GAP);
    assert_eq!(rewrite_ids(&before), "x EntityId(3) y Some(EntityId(0))");
}

#[test]
fn the_removed_offer_field_is_restored_for_the_digest() {
    let without = "a ProgressView { level: 1, xp: 0, xp_next: 5, spent: [] } b";
    assert_eq!(
        restore_removed_fields(without),
        "a ProgressView { level: 1, xp: 0, xp_next: 5, spent: [], offer: None } b"
    );
    let with = "a ProgressView { level: 1, xp: 0, xp_next: 5, spent: [], offer: None } b";
    assert_eq!(restore_removed_fields(with), with);
    let open = "ProgressView { level: 1, spent: [], offer: Some(OfferView { kind: Star }) }";
    assert_eq!(restore_removed_fields(open), open);
    assert_eq!(restore_removed_fields("no view here"), "no view here");
}
