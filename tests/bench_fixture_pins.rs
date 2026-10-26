//! The six bench cases (`bench_fixtures`, ADR-SIM-6) build the engine the benches count. Their
//! pins live in `tests/bench_pins.toml`: the state hash before and after the measured tick(s),
//! and a digest of the case's stage and of its measured input, so that BS2d's bench job can tell
//! a case it may compare (the same `before`, `stage` and `input`) from one that changed. A pin
//! asserts an output, so it lives in `tests/`, not in the bench input: a change that moves a hash
//! and touches `src/` is then not refused by the bench guard (core#360).
//!
//! `UPDATE_BENCH_PINS=1 cargo test bench_fixture_pins` rewrites the file (only the exact value
//! `1` does, and CI refuses it); without it a mismatch fails and prints every computed value of the case, so a re-pin is one paste or one env run.
//!
//! The values are those of the benches' own case modules (built on `tests/common/`) when the
//! fixtures were written: the fixtures must build the same engine, or the instruction counts move.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};
use stella_rain_core::bench_fixtures::worst_minute_case::{
    MINUTE_START, MINUTE_TICKS, OFFER_CHOICE, Run, input_at,
};
use stella_rain_core::bench_fixtures::{self, Case, hit_case, time_limit_case, worst_case};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::content_sha256_with;
use stella_rain_core::stage::Stage;

const PINS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/bench_pins.toml");
const PINS: &str = include_str!("bench_pins.toml");
const BUDGETS: &str = include_str!("../benches/budgets.toml");

const HEADER: &str = "\
# The bench cases' pins (BS2c). One table per case, keyed by the case id (the `#[bench::<id>]`
# name in `benches/`), in the order of `benches/`.
#   bench  the bench target (a key of `benches/budgets.toml`)
#   before the state hash before the measured tick(s), 16 lowercase hex
#   after  the state hash after them
#   stage  the first 16 hex of SHA-256 over the stage's JSON and the run content's hash
#          (`stella-rain/bench-stage/v1`; the minute cases have no stage part: the content
#          hash and `before` cover their stage)
#   input  the first 16 hex of SHA-256 over the measured input's JSON, or over the tick count
#          for a minute case (`stella-rain/bench-input/v1`)
# Hashes are strings: TOML integers are i64 and these u64 values overflow it.
# A case is comparable when `before`, `stage` and `input` are equal; `after` is pinned only.
# Written by `UPDATE_BENCH_PINS=1 cargo test bench_fixture_pins` (only the exact value `1`; CI
# refuses it); BS2d's bench job reads it from base and head.
";

/// What a case measures: one tick of a built engine, or some ticks of a run.
enum Kind {
    Tick(fn() -> Case, fn() -> Stage),
    Run(fn() -> Run, u32),
}

struct Spec {
    id: &'static str,
    bench: &'static str,
    kind: Kind,
}

/// The cases, in the order of `benches/`.
fn specs() -> [Spec; 6] {
    [
        Spec {
            id: "worst_tick",
            bench: "worst_tick",
            kind: Kind::Tick(bench_fixtures::worst_tick, worst_case::stage),
        },
        Spec {
            id: "hit_tick",
            bench: "hit_tick",
            kind: Kind::Tick(bench_fixtures::hit_tick, hit_case::stage),
        },
        Spec {
            id: "hit_tick_spread",
            bench: "hit_tick",
            kind: Kind::Tick(bench_fixtures::hit_tick_spread, hit_case::spread_stage),
        },
        Spec {
            id: "time_limit_tick",
            bench: "time_limit_tick",
            kind: Kind::Tick(bench_fixtures::time_limit_tick, time_limit_case::stage),
        },
        Spec {
            id: "worst_minute",
            bench: "worst_minute",
            kind: Kind::Run(bench_fixtures::worst_minute, MINUTE_TICKS),
        },
        Spec {
            id: "worst_minute_peak",
            bench: "worst_minute_peak",
            kind: Kind::Run(bench_fixtures::worst_minute_peak, 1),
        },
    ]
}

/// The values of one case, as the file holds them.
struct Pins {
    bench: String,
    before: String,
    after: String,
    stage: String,
    input: String,
}

impl Pins {
    fn fields(&self) -> [(&'static str, &str); 5] {
        [
            ("bench", &self.bench),
            ("before", &self.before),
            ("after", &self.after),
            ("stage", &self.stage),
            ("input", &self.input),
        ]
    }

    fn table(&self, id: &str) -> String {
        let mut out = format!("[{id}]\n");
        for (key, value) in self.fields() {
            writeln!(out, "{key} = \"{value}\"").expect("a String takes writes");
        }
        out
    }
}

fn hash_hex(hash: u64) -> String {
    format!("{hash:016x}")
}

/// The first 16 hex of the SHA-256 over `parts`.
fn digest(parts: &[&[u8]]) -> String {
    let mut sha = Sha256::new();
    for part in parts {
        sha.update(part);
    }
    sha.finalize()[..8]
        .iter()
        .fold(String::new(), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("a String takes writes");
            hex
        })
}

fn content_hash(engine: &Engine) -> [u8; 32] {
    content_sha256_with("time_limit", engine.content()).expect("the time_limit content hashes")
}

/// The digest of a minute case's input: the start tick, the tick count, the option an open offer
/// takes and every `Input` the script gives for the ticks `start..start + ticks`.
fn minute_input_digest(start: u32, ticks: u32, script: impl Fn(u32) -> Input) -> String {
    let mut played = Vec::new();
    for tick in start..start + ticks {
        serde_json::to_writer(&mut played, &script(tick)).expect("the input serializes");
        played.push(b'\n');
    }
    digest(&[
        b"stella-rain/bench-input/v2\n",
        b"start:",
        start.to_string().as_bytes(),
        b"ticks:",
        ticks.to_string().as_bytes(),
        b"offer:",
        OFFER_CHOICE.to_string().as_bytes(),
        b"\n",
        &played,
    ])
}

/// Builds the case, runs its measured ticks and gives every value it pins.
fn measure(spec: &Spec) -> Pins {
    let (before, after, stage, input);
    match &spec.kind {
        Kind::Tick(case, stage_of) => {
            let Case {
                mut engine,
                input: tick_input,
            } = case();
            before = hash_hex(engine.state_hash());
            let stage_json = serde_json::to_vec(&stage_of()).expect("the stage serializes");
            stage = digest(&[
                b"stella-rain/bench-stage/v1\n",
                &stage_json,
                &content_hash(&engine),
            ]);
            let input_json = serde_json::to_vec(&tick_input).expect("the input serializes");
            input = digest(&[b"stella-rain/bench-input/v2\n", &input_json]);
            engine.step(tick_input);
            after = hash_hex(engine.state_hash());
        }
        Kind::Run(run, ticks) => {
            let mut run = run();
            before = hash_hex(run.engine.state_hash());
            stage = digest(&[b"stella-rain/bench-stage/v1\n", &content_hash(&run.engine)]);
            input = minute_input_digest(run.engine.tick(), *ticks, input_at);
            run.play(*ticks);
            after = hash_hex(run.engine.state_hash());
        }
    }
    Pins {
        bench: spec.bench.to_string(),
        before,
        after,
        stage,
        input,
    }
}

/// Whether the pins are rewritten: only the exact value `1` turns it on, and never in CI.
fn update_mode(update: Option<&str>, ci: bool) -> bool {
    if update != Some("1") {
        return false;
    }
    assert!(
        !ci,
        "UPDATE_BENCH_PINS=1 is refused when CI is set: re-pin locally and commit the file"
    );
    true
}

#[expect(clippy::disallowed_methods, reason = "test knob")]
fn updating() -> bool {
    let update = std::env::var("UPDATE_BENCH_PINS").ok();
    update_mode(update.as_deref(), std::env::var_os("CI").is_some())
}

fn pinned_file() -> toml::Table {
    PINS.parse().expect("tests/bench_pins.toml is TOML")
}

/// Asserts the case against the file: every mismatch is listed, with the computed values.
fn check(id: &str) {
    if updating() {
        return;
    }
    let spec = specs()
        .into_iter()
        .find(|spec| spec.id == id)
        .expect("a known case");
    let computed = measure(&spec);
    let file = pinned_file();
    let pinned = file.get(id).and_then(toml::Value::as_table);
    let mut wrong = String::new();
    for (key, value) in computed.fields() {
        match pinned
            .and_then(|table| table.get(key))
            .and_then(toml::Value::as_str)
        {
            Some(pin) if pin == value => {}
            Some(pin) => writeln!(wrong, "  {key}: pinned {pin}, computed {value}")
                .expect("a String takes writes"),
            None => {
                writeln!(wrong, "  missing {key}: computed {value}").expect("a String takes writes")
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{id} is not the pinned bench case:\n{wrong}tests/bench_pins.toml would hold:\n{}",
        computed.table(id)
    );
}

#[test]
fn the_worst_tick_is_the_benches_case() {
    check("worst_tick");
}

#[test]
fn the_hit_tick_is_the_benches_case() {
    check("hit_tick");
}

#[test]
fn the_spread_hit_tick_is_the_benches_case() {
    check("hit_tick_spread");
}

#[test]
fn the_time_limit_tick_is_the_benches_case() {
    check("time_limit_tick");
}

// The two constants of the densest minute are re-pinned together with these hashes after any
// outcome change (`src/bench_fixtures/worst_minute_case.rs`).
#[test]
fn the_worst_minute_is_the_benches_case() {
    check("worst_minute");
}

#[test]
fn the_worst_minute_peak_tick_is_the_benches_case() {
    check("worst_minute_peak");
}

#[test]
fn the_pin_file_holds_exactly_the_benches_cases() {
    let file = pinned_file();
    let mut in_file: Vec<&str> = file.keys().map(String::as_str).collect();
    let mut in_code: Vec<&str> = specs().iter().map(|spec| spec.id).collect();
    in_file.sort_unstable();
    in_code.sort_unstable();
    assert_eq!(
        in_file, in_code,
        "the cases of tests/bench_pins.toml and of the code"
    );

    let budgets: toml::Table = BUDGETS.parse().expect("benches/budgets.toml is TOML");
    for spec in specs() {
        assert!(
            budgets.contains_key(spec.bench),
            "{}: bench = \"{}\" is not a key of benches/budgets.toml",
            spec.id,
            spec.bench
        );
    }
}

/// `UPDATE_BENCH_PINS=1 cargo test bench_fixture_pins` writes the whole file, in the order of
/// `benches/`, as `UPDATE_CORPUS` writes the corpus.
#[test]
fn bench_pins_are_written_on_request() {
    if !updating() {
        return;
    }
    let mut out = String::from(HEADER);
    for spec in specs() {
        out.push('\n');
        out.push_str(&measure(&spec).table(spec.id));
    }
    std::fs::write(PINS_PATH, out).expect("tests/bench_pins.toml is writable");
}

#[test]
fn only_the_value_one_turns_update_mode_on() {
    assert!(!update_mode(None, false));
    assert!(!update_mode(Some("0"), false));
    assert!(!update_mode(Some(""), false));
    assert!(!update_mode(Some("true"), false));
    assert!(!update_mode(Some("yes"), false));
    assert!(update_mode(Some("1"), false));
}

#[test]
fn a_set_ci_without_the_knob_is_not_update_mode() {
    assert!(!update_mode(None, true));
    assert!(!update_mode(Some("0"), true));
}

#[test]
#[should_panic(expected = "CI")]
fn update_mode_is_refused_in_ci() {
    update_mode(Some("1"), true);
}

fn input_of(id: &str) -> String {
    let spec = specs()
        .into_iter()
        .find(|spec| spec.id == id)
        .expect("a known case");
    measure(&spec).input
}

#[test]
fn the_minute_cases_input_digest_covers_the_played_input() {
    assert_eq!(
        input_of("worst_minute"),
        minute_input_digest(MINUTE_START, MINUTE_TICKS, input_at)
    );
}

#[test]
fn the_minute_input_digest_moves_with_the_start_tick() {
    assert_ne!(
        minute_input_digest(MINUTE_START, MINUTE_TICKS, input_at),
        minute_input_digest(MINUTE_START + 1, MINUTE_TICKS, input_at)
    );
}

#[test]
fn the_minute_input_digest_moves_with_the_script() {
    let flipped = |tick| Input {
        main: tick % 2 == 0,
        ..Input::default()
    };
    assert_ne!(
        minute_input_digest(MINUTE_START, MINUTE_TICKS, input_at),
        minute_input_digest(MINUTE_START, MINUTE_TICKS, flipped)
    );
}
