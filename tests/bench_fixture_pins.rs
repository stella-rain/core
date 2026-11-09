//! The eight bench cases (`bench_fixtures`, ADR-SIM-5) build the engine the benches count. Their
//! pins live in `tests/bench_pins.toml`: the state hash before and after the measured tick(s),
//! and a digest of the case's stage and of its measured input, so that BS2d's bench job can tell
//! a case it may compare (the same `before`, `stage` and `input`) from one that changed. A pin
//! asserts an output, so it lives in `tests/`, not in the bench input: a change that moves a hash
//! and touches `src/` is then not refused by the bench guard (core#360).
//!
//! `UPDATE_BENCH_PINS=1 cargo test bench_fixture_pins` rewrites the file (only the exact value
//! `1` does, and CI refuses it); without it a mismatch fails and prints every computed value of the
//! case, so a re-pin is one paste or one env run.
//!
//! The values are those of the benches' own case modules (built on `tests/common/`) when the
//! fixtures were written: the fixtures must build the same engine, or the instruction counts move.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};
use stella_rain_core::bench_fixtures::equip_case::{self, EQUIP_PEAK_TICK, ROLLBACK_ACTIONS};
use stella_rain_core::bench_fixtures::party_case::{self, PEAK_TICK_3P, ROLLBACK_TICKS};
use stella_rain_core::bench_fixtures::worst_minute_case::{
    MINUTE_START, MINUTE_TICKS, Run, input_at, worst_minute_window,
};
use stella_rain_core::bench_fixtures::{self, Case, hit_case, time_limit_case, worst_case};
use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::content_sha256_with;
use stella_rain_core::stage::Stage;
use stella_rain_net::save::CloneRing;

#[path = "../benches/session_case/mod.rs"]
#[allow(dead_code)]
mod session_case;

fn pins_path() -> std::path::PathBuf {
    crate::data_root::data_root().join("tests/bench_pins.toml")
}
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
#   input  the first 16 hex of SHA-256 over the measured input's JSON; for a minute case over
#          the start tick, the tick count and the JSON of every played `Input` (`input_at`)
#          (`stella-rain/bench-input/v3`; the tick cases keep `v2`)
# Hashes are strings: TOML integers are i64 and these u64 values overflow it.
# A case is comparable when `before`, `stage` and `input` are equal; `after` is pinned only.
# Written by `UPDATE_BENCH_PINS=1 cargo test bench_fixture_pins` (only the exact value `1`; CI
# refuses it); BS2d's bench job reads it from base and head.
";

/// What a case measures: one tick of a built engine, or some ticks of a run.
enum Kind {
    Tick(fn() -> Case, fn() -> Stage),
    Run(fn() -> Run, u32),
    /// One tick of a three-player engine (`party_case::tick`).
    Party(fn() -> party_case::Case, fn() -> Stage),
    /// The saved engine of the rollback bench (`party_case::rollback`).
    Rollback(fn() -> Engine),
    /// The saved engine of the equipment rollback bench (`equip_case::rollback_equip`).
    EquipRollback(fn() -> Engine),
    /// One rolling-back `Session::advance` of the equipment run at a window
    /// (`session_case::rollback`).
    SessionRollback(u32),
    /// One `CloneRing` save of the saved engine (`session_case::save_once`).
    RingSave,
}

struct Spec {
    id: &'static str,
    bench: &'static str,
    /// The mode the case's content is built for; part of the stage digest.
    mode: &'static str,
    kind: Kind,
}

/// The cases, in the order of `benches/`.
fn specs() -> [Spec; 12] {
    [
        Spec {
            id: "worst_tick",
            bench: "worst_tick",
            mode: "time_limit",
            kind: Kind::Tick(bench_fixtures::worst_tick, worst_case::stage),
        },
        Spec {
            id: "hit_tick",
            bench: "hit_tick",
            mode: "time_limit",
            kind: Kind::Tick(bench_fixtures::hit_tick, hit_case::stage),
        },
        Spec {
            id: "hit_tick_spread",
            bench: "hit_tick",
            mode: "time_limit",
            kind: Kind::Tick(bench_fixtures::hit_tick_spread, hit_case::spread_stage),
        },
        Spec {
            id: "time_limit_tick",
            bench: "time_limit_tick",
            mode: "time_limit",
            kind: Kind::Tick(bench_fixtures::time_limit_tick, time_limit_case::stage),
        },
        Spec {
            id: "worst_minute",
            bench: "worst_minute",
            mode: "time_limit",
            kind: Kind::Run(bench_fixtures::worst_minute, MINUTE_TICKS),
        },
        Spec {
            id: "worst_minute_peak",
            bench: "worst_minute_peak",
            mode: "time_limit",
            kind: Kind::Run(bench_fixtures::worst_minute_peak, 1),
        },
        Spec {
            id: "worst_tick_3p",
            bench: "worst_tick_3p",
            mode: "time_limit",
            kind: Kind::Party(party_case::worst_tick_3p, worst_case::stage),
        },
        Spec {
            id: "rollback",
            bench: "rollback",
            mode: "time_limit",
            kind: Kind::Rollback(party_case::saved),
        },
        Spec {
            id: "rollback_equip",
            bench: "rollback_equip",
            mode: "time_limit",
            kind: Kind::EquipRollback(equip_case::saved),
        },
        Spec {
            id: "rollback_session_w4",
            bench: "rollback_session",
            mode: "time_limit",
            kind: Kind::SessionRollback(stella_rain_net::session::DEFAULT_WINDOW),
        },
        Spec {
            id: "rollback_session_w16",
            bench: "rollback_session",
            mode: "time_limit",
            kind: Kind::SessionRollback(stella_rain_net::session::MAX_WINDOW),
        },
        Spec {
            id: "clone_ring_save",
            bench: "rollback_session",
            mode: "time_limit",
            kind: Kind::RingSave,
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

fn content_hash(engine: &Engine, mode: &str) -> [u8; 32] {
    content_sha256_with(mode, engine.content()).expect("the mode's content hashes")
}

/// The digest of a minute case's input: the start tick, the tick count
/// and every `Input` the script gives for the ticks `start..start + ticks`.
fn minute_input_digest<I: serde::Serialize>(
    start: u32,
    ticks: u32,
    script: impl Fn(u32) -> I,
) -> String {
    let mut played = Vec::new();
    for tick in start..start + ticks {
        serde_json::to_writer(&mut played, &script(tick)).expect("the input serializes");
        played.push(b'\n');
    }
    digest(&[
        b"stella-rain/bench-input/v3\n",
        b"start:",
        start.to_string().as_bytes(),
        b"ticks:",
        ticks.to_string().as_bytes(),
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
                &content_hash(&engine, spec.mode),
            ]);
            let input_json = serde_json::to_vec(&tick_input).expect("the input serializes");
            input = digest(&[b"stella-rain/bench-input/v2\n", &input_json]);
            engine.step(&[tick_input, Default::default(), Default::default()]);
            after = hash_hex(engine.state_hash());
        }
        Kind::Party(case, stage_of) => {
            let party_case::Case { mut engine, inputs } = case();
            before = hash_hex(engine.state_hash());
            let stage_json = serde_json::to_vec(&stage_of()).expect("the stage serializes");
            stage = digest(&[
                b"stella-rain/bench-stage/v1\n",
                &stage_json,
                &content_hash(&engine, spec.mode),
            ]);
            let input_json = serde_json::to_vec(&inputs).expect("the inputs serialize");
            input = digest(&[b"stella-rain/bench-input/v2\n", &input_json]);
            engine.step(&inputs);
            after = hash_hex(engine.state_hash());
        }
        Kind::Rollback(saved) => {
            let saved = saved();
            before = hash_hex(saved.state_hash());
            stage = digest(&[
                b"stella-rain/bench-stage/v1\n",
                &content_hash(&saved, spec.mode),
            ]);
            input = minute_input_digest(saved.tick(), ROLLBACK_TICKS, party_case::minute_inputs);
            let (engine, saved) = party_case::rollback(saved);
            std::mem::forget(saved);
            after = hash_hex(engine.state_hash());
        }
        Kind::EquipRollback(saved) => {
            let saved = saved();
            before = hash_hex(saved.state_hash());
            stage = digest(&[
                b"stella-rain/bench-stage/v1\n",
                &content_hash(&saved, spec.mode),
            ]);
            let mut played = Vec::new();
            for tick in saved.tick()..saved.tick() + ROLLBACK_TICKS {
                serde_json::to_writer(&mut played, &party_case::minute_inputs(tick))
                    .expect("the inputs serialize");
                played.push(b'\n');
            }
            let actions = serde_json::to_vec(&ROLLBACK_ACTIONS).expect("the actions serialize");
            input = digest(&[
                b"stella-rain/bench-input-equip/v1\n",
                &played,
                b"actions:",
                &actions,
            ]);
            let (engine, saved) = equip_case::rollback_equip(saved);
            std::mem::forget(saved);
            after = hash_hex(engine.state_hash());
        }
        Kind::SessionRollback(window) => {
            let prepared = session_case::driven(*window);
            before = hash_hex(prepared.session.engine().state_hash());
            stage = digest(&[
                b"stella-rain/bench-stage/v1\n",
                &content_hash(prepared.session.engine(), spec.mode),
            ]);
            let mut played = Vec::new();
            for tick in EQUIP_PEAK_TICK..=EQUIP_PEAK_TICK + window {
                serde_json::to_writer(&mut played, &party_case::minute_inputs(tick))
                    .expect("the inputs serialize");
                played.push(b'\n');
            }
            let actions = serde_json::to_vec(&ROLLBACK_ACTIONS).expect("the actions serialize");
            input = digest(&[
                b"stella-rain/bench-input-session/v1\n",
                &played,
                b"actions:",
                &actions,
                b"window:",
                window.to_string().as_bytes(),
            ]);
            let prepared = session_case::rollback(prepared);
            after = hash_hex(prepared.session.engine().state_hash());
            std::mem::forget(prepared);
        }
        Kind::RingSave => {
            let (ring, engine) = (
                CloneRing::new(session_case::RING_CAPACITY),
                equip_case::saved(),
            );
            before = hash_hex(engine.state_hash());
            stage = digest(&[
                b"stella-rain/bench-stage/v1\n",
                &content_hash(&engine, spec.mode),
            ]);
            input = digest(&[
                b"stella-rain/bench-input-session-save/v1\n",
                b"capacity:",
                session_case::RING_CAPACITY.to_string().as_bytes(),
            ]);
            let (ring, engine) = session_case::save_once((ring, engine));
            after = hash_hex(engine.state_hash());
            std::mem::forget((ring, engine));
        }
        Kind::Run(run, ticks) => {
            let mut run = run();
            before = hash_hex(run.engine.state_hash());
            stage = digest(&[
                b"stella-rain/bench-stage/v1\n",
                &content_hash(&run.engine, spec.mode),
            ]);
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
fn the_worst_tick_3p_is_the_benches_case() {
    check("worst_tick_3p");
}

#[test]
fn the_rollback_is_the_benches_case() {
    check("rollback");
}

#[test]
fn the_rollback_equip_is_the_benches_case() {
    check("rollback_equip");
}

#[test]
fn the_session_w4_is_the_benches_case() {
    check("rollback_session_w4");
}

#[test]
fn the_session_w16_is_the_benches_case() {
    check("rollback_session_w16");
}

#[test]
fn the_ring_save_is_the_benches_case() {
    check("clone_ring_save");
}

/// A straight engine from the saved one, `window` ticks played with the window's recorded
/// actions applied before the tick they come before.
fn corrected_run(window: u32) -> Engine {
    let mut direct = equip_case::saved();
    for i in 0..window {
        for (at, acts) in &ROLLBACK_ACTIONS {
            if *at == i {
                for (s, a) in acts.iter().enumerate() {
                    if let Some(a) = a {
                        let p = PlayerIdx::new(u8::try_from(s).unwrap()).unwrap();
                        direct.act(p, *a).unwrap();
                    }
                }
            }
        }
        direct.step(&party_case::minute_inputs(direct.tick()));
    }
    direct
}

/// S1: driven through its public calls, the session reproduces the saved engine of the case.
#[test]
fn the_session_setup_reaches_the_rollback_equip_saved_engine() {
    let session = session_case::at_peak(stella_rain_net::session::DEFAULT_WINDOW);
    assert_eq!(session.tick(), EQUIP_PEAK_TICK);
    assert_eq!(session.confirmed(), EQUIP_PEAK_TICK);
    assert_eq!(session.engine().tick(), EQUIP_PEAK_TICK);
    assert_eq!(
        session.engine().state_hash(),
        equip_case::saved().state_hash()
    );
}

/// S2: the measured advance re-simulates exactly the window and lands on the corrected run.
#[test]
fn the_session_rollback_re_simulates_the_window() {
    for w in [
        stella_rain_net::session::DEFAULT_WINDOW,
        stella_rain_net::session::MAX_WINDOW,
    ] {
        let mut p = session_case::driven(w);
        assert_eq!(p.session.tick(), EQUIP_PEAK_TICK + w);
        assert_eq!(p.session.confirmed(), EQUIP_PEAK_TICK);
        let advance = p.session.advance(&p.local).expect("advances");
        assert_eq!(advance.rolled_back, w);
        assert_eq!(advance.waiting, None);
        assert_eq!(p.session.tick(), EQUIP_PEAK_TICK + w + 1);
        assert_eq!(p.session.confirmed(), EQUIP_PEAK_TICK + w);
        assert_eq!(
            p.session.hash_after(EQUIP_PEAK_TICK + w),
            Some(corrected_run(w).state_hash()),
            "window {w}"
        );
    }
}

/// S3: at the largest window the corrected span crosses both recorded actions.
#[test]
fn the_session_window_equips_and_extracts() {
    let before = equip_case::saved().snapshot();
    let after = session_case::rollback(session_case::driven(stella_rain_net::session::MAX_WINDOW))
        .session
        .engine()
        .snapshot();
    for (b, a) in before.players.iter().zip(&after.players) {
        assert_eq!(a.inventory.len() + 2, b.inventory.len());
    }
}

/// S4: one save into an empty ring is one held clone, and the engine is unchanged.
#[test]
fn the_ring_save_is_one_clone() {
    let engine = equip_case::saved();
    let hash = engine.state_hash();
    let (ring, engine) =
        session_case::save_once((CloneRing::new(session_case::RING_CAPACITY), engine));
    assert_eq!(ring.held(), 1);
    assert_eq!(engine.state_hash(), hash);
}

fn hits_by(engine: &Engine, slot: u8) -> usize {
    use stella_rain_core::event::DomainEvent;
    let id = PlayerIdx::new(slot).expect("a slot").id();
    engine
        .events()
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::Hit { player: Some(p), .. } if *p == id))
        .count()
}

/// The three-player tick: all three present, slots 1 and 2 land their hits, and the crowd, the
/// zones and the orbs are at the caps `tests/worst_case.rs` checks for the solo case.
#[test]
fn the_party_tick_reaches_the_caps_with_every_slot_hitting() {
    let case = party_case::worst_tick_3p();
    assert_eq!(case.engine.players(), 3);
    assert_eq!(case.engine.present(), 0b111);
    let engine = party_case::tick(case);
    for slot in [1, 2] {
        assert!(hits_by(&engine, slot) >= 260, "slot {slot} hits");
    }
    let load = engine.load();
    assert_eq!((load.enemies, load.zones, load.orbs), (260, 128, 32));
}

/// Rolling back leaves the saved engine as it was, and the 8 ticks are the ones an engine
/// advanced from the same state plays.
#[test]
fn the_rollback_restores_and_plays_eight_ticks() {
    let saved = party_case::saved();
    assert_eq!(saved.players(), 3);
    assert_eq!(saved.tick(), PEAK_TICK_3P);
    assert!(!saved.offer_open());
    let before = saved.state_hash();
    let (played, saved) = party_case::rollback(saved);
    assert_eq!(saved.state_hash(), before);
    assert_eq!(played.tick(), PEAK_TICK_3P + ROLLBACK_TICKS);
    let mut direct = party_case::saved();
    for _ in 0..ROLLBACK_TICKS {
        direct.step(&party_case::minute_inputs(direct.tick()));
    }
    assert_eq!(played.state_hash(), direct.state_hash());
}

/// The scan of the 3-player dense minute finds the peak the rollback starts at.
#[test]
fn the_party_peak_is_where_the_scan_finds_it() {
    assert_eq!(party_case::window(), (MINUTE_START, PEAK_TICK_3P));
}

/// The minute fixtures run in equipment mode: the main character has equipment slots and no
/// level-up offer ever opens (ADR-SIM-5, ADR-SIM-7).
#[test]
fn the_minute_fixtures_run_in_equipment_mode_and_open_no_offer() {
    let mut run = bench_fixtures::worst_minute();
    assert!(
        !run.engine.snapshot().players[0].slots.is_empty(),
        "the worst minute is in equipment mode"
    );
    for _ in 0..600 {
        assert!(!run.engine.offer_open());
        run.play(1);
    }
    let mut party = party_case::saved();
    assert!(
        !party.snapshot().players[0].slots.is_empty(),
        "the party case is in equipment mode"
    );
    for _ in 0..600 {
        assert!(!party.offer_open());
        party.step(&party_case::minute_inputs(party.tick()));
    }
    let equip = equip_case::saved();
    assert!(
        !equip.snapshot().players[0].slots.is_empty(),
        "the equipment case is in equipment mode"
    );
    assert!(!equip.offer_open());
}

/// The stage fixtures start in equipment mode (ADR-SIM-5, ADR-SIM-7): the main character has
/// equipment slots, the worst tick's seven runes and Rune Haste are equipped, and no offer opens.
#[test]
fn the_stage_fixtures_run_in_equipment_mode() {
    use stella_rain_core::rune::Rune;
    use stella_rain_core::snapshot::{HeldView, SlotKindView};

    let worst = bench_fixtures::worst_tick().engine;
    let slots = &worst.snapshot().players[0].slots;
    let runes: Vec<Rune> = slots
        .iter()
        .filter(|s| s.kind == SlotKindView::Rune)
        .filter_map(|s| match s.held {
            Some(HeldView::Rune { rune }) => Some(rune),
            _ => None,
        })
        .collect();
    assert_eq!(runes, Rune::ALL, "every rune, in the rune slots");
    assert!(
        matches!(&slots[0].held, Some(HeldView::Star { id, .. }) if id.as_str() == "haste"),
        "Rune Haste in slot 0"
    );

    type Inputs = fn(u32) -> Input;
    let seven = Rune::ALL.len();
    let cases: [(&str, Engine, Inputs, usize); 5] = [
        (
            "worst_tick",
            bench_fixtures::worst_tick().engine,
            worst_case::input,
            seven,
        ),
        (
            "hit_tick",
            bench_fixtures::hit_tick().engine,
            hit_case::input,
            0,
        ),
        (
            "hit_tick_spread",
            bench_fixtures::hit_tick_spread().engine,
            hit_case::input,
            0,
        ),
        (
            "time_limit_tick",
            bench_fixtures::time_limit_tick().engine,
            time_limit_case::input,
            0,
        ),
        (
            "worst_tick_3p",
            party_case::worst_tick_3p().engine,
            worst_case::input,
            seven,
        ),
    ];
    for (name, mut engine, input, runes) in cases {
        let view = &engine.snapshot().players[0];
        assert!(!view.slots.is_empty(), "{name} is in equipment mode");
        let held_runes = view
            .slots
            .iter()
            .filter(|s| matches!(s.held, Some(HeldView::Rune { .. })))
            .count();
        assert_eq!(held_runes, runes, "{name} runes equipped");
        for _ in 0..120 {
            assert!(!engine.offer_open(), "{name} opens no offer");
            let tick = engine.tick() + 1;
            let mut inputs = [Input::default(), Input::default(), Input::default()];
            inputs[0] = input(tick);
            engine.step(&inputs);
        }
    }
}

/// E1: the equipment rollback's saved engine.
#[test]
fn the_equip_saved_engine() {
    let saved = equip_case::saved();
    assert_eq!(saved.players(), 3);
    assert_eq!(saved.present(), 0b111);
    assert!(
        !saved.snapshot().players[0].slots.is_empty(),
        "equipment mode"
    );
    assert!(!saved.offer_open());
    assert_eq!(saved.tick(), EQUIP_PEAK_TICK);
    for view in &saved.snapshot().players {
        assert!(view.inventory.len() >= 2, "every inventory holds two items");
    }
}

/// E2: the window's actions are accepted where they come, and equip two, extract two.
#[test]
fn the_window_equips_and_extracts() {
    let mut e = equip_case::saved();
    let before = e.snapshot();
    for i in 0..ROLLBACK_TICKS {
        for (at, acts) in &ROLLBACK_ACTIONS {
            if *at == i {
                for (s, a) in acts.iter().enumerate() {
                    if let Some(a) = a {
                        let p = PlayerIdx::new(u8::try_from(s).unwrap()).unwrap();
                        assert_eq!(e.check_action(p, *a), Ok(()), "slot {s} at {i}");
                        e.act(p, *a).unwrap();
                    }
                }
            }
        }
        e.step(&party_case::minute_inputs(e.tick()));
    }
    let after = e.snapshot();
    for (b, a) in before.players.iter().zip(&after.players) {
        assert_eq!(a.inventory.len() + 2, b.inventory.len());
        let filled = |v: &stella_rain_core::snapshot::PlayerView| {
            v.slots.iter().filter(|s| s.held.is_some()).count()
        };
        assert_eq!(filled(a), filled(b) + 1);
        assert!(a.starlight > b.starlight);
    }
}

/// E3: the rollback leaves the saved engine alone and equals a direct play.
#[test]
fn the_rollback_equip_matches_a_direct_play() {
    let saved = equip_case::saved();
    let before = saved.state_hash();
    let mut direct = saved.clone();
    for i in 0..ROLLBACK_TICKS {
        for (at, acts) in &ROLLBACK_ACTIONS {
            if *at == i {
                for (s, a) in acts.iter().enumerate() {
                    if let Some(a) = a {
                        let p = PlayerIdx::new(u8::try_from(s).unwrap()).unwrap();
                        direct.act(p, *a).unwrap();
                    }
                }
            }
        }
        direct.step(&party_case::minute_inputs(direct.tick()));
    }
    let (played, saved) = equip_case::rollback_equip(saved);
    assert_eq!(saved.state_hash(), before);
    assert_eq!(played.tick(), EQUIP_PEAK_TICK + ROLLBACK_TICKS);
    assert_eq!(played.state_hash(), direct.state_hash());
}

/// E4: the scan finds the peak.
#[test]
fn the_equip_peak_is_where_the_scan_finds_it() {
    assert_eq!(equip_case::window(), (MINUTE_START, EQUIP_PEAK_TICK));
}

/// The worst tick is one of the heaviest by `Load` that the scripted run plays; a tie is settled
/// by counting instructions.
#[test]
fn the_worst_tick_is_where_the_scan_finds_it() {
    let heaviest = worst_case::window();
    assert!(
        heaviest.contains(&(worst_case::WARMUP_TICKS + 1)),
        "the measured tick is not among the heaviest: {heaviest:?}"
    );
}

/// Asserts that `file` holds exactly the benches' cases, each with a bench of `budgets.toml` or
/// in `UNBUDGETED`.
fn assert_case_set(file: &toml::Table) {
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
            budgets.contains_key(spec.bench)
                || crate::bench_teardown::UNBUDGETED.contains(&spec.bench),
            "{}: bench = \"{}\" is not a key of benches/budgets.toml or in UNBUDGETED",
            spec.id,
            spec.bench
        );
    }
}

/// In update mode the writer test checks the file it wrote (the embedded `PINS` is the file
/// before the run, and tests of one binary run in parallel).
#[test]
fn the_pin_file_holds_exactly_the_benches_cases() {
    if updating() {
        return;
    }
    assert_case_set(&pinned_file());
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
    std::fs::write(pins_path(), &out).expect("tests/bench_pins.toml is writable");
    assert_case_set(&out.parse().expect("the written file is TOML"));
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
fn the_peak_cases_input_digest_covers_its_start_tick() {
    assert_eq!(
        input_of("worst_minute_peak"),
        minute_input_digest(worst_minute_window().1, 1, input_at)
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

/// Only the `time_limit` mode is built (`Engine::start_with`), and the stage digest covers the
/// mode: a case of another mode must name it, not pass as `time_limit`.
#[test]
fn every_case_is_a_time_limit_case() {
    for spec in specs() {
        assert_eq!(spec.mode, "time_limit", "{}", spec.id);
    }
}
