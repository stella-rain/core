//! Run stats (ADR-079 §4, ADR-067): the stats a stat point raises are content, read from JSON
//! as untrusted bytes (ADR-020): a size limit before parsing, strict parsing, the current
//! versions only and every field bounded. The bundled file is the default; other bytes (a
//! server's, later) run the same way and change the run's content hash.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::id::ContentId;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    RunRecorder, RunReplayError, content_sha256, content_sha256_with, verify_run, verify_run_with,
};
use stella_rain_core::run_stats::{
    MAX_ATTACK_SPEED_PCT, MAX_DESCRIPTION_CHARS, MAX_LIMIT, MAX_RUN_STATS_BYTES, MAX_STATS,
    MAX_STEP, MAX_TITLE_CHARS, RunStats, StatEffect, capped_attack_speed_pct,
};
use stella_rain_core::validate::Error;

use crate::common;

use common::run::{run_script, trial_setup};

fn stat(id: &str) -> Value {
    json!({
        "id": id,
        "effect": "speed_pct",
        "step": 5,
        "limit": 3,
        "title": "Speed",
        "description": "Move faster"
    })
}

fn file(stats: Vec<Value>) -> Value {
    json!({ "schema_version": 0, "sim_version": 0, "stats": stats })
}

fn parse(v: &Value) -> Result<RunStats, Error> {
    RunStats::parse(v.to_string().as_bytes())
}

/// One valid stat with `field` set to `value`.
fn one_with(field: &str, value: Value) -> Value {
    let mut s = stat("speed");
    s[field] = value;
    file(vec![s])
}

fn invalid_at(v: &Value, path: &str) {
    match parse(v) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

#[test]
fn the_bundled_stats_are_the_five_of_version_0() {
    let stats = RunStats::bundled().stats();
    let got: Vec<(&str, StatEffect, u32, u8)> = stats
        .iter()
        .map(|s| (s.id.0.as_str(), s.effect, s.step, s.limit))
        .collect();
    assert_eq!(
        got,
        [
            ("max_hp", StatEffect::MaxHp, 1, 5),
            ("attack", StatEffect::AttackPct, 10, 5),
            ("attack_speed", StatEffect::AttackSpeedPct, 8, 5),
            ("evade", StatEffect::EvadePct, 10, 5),
            ("speed", StatEffect::SpeedPct, 8, 5),
        ]
    );
    assert!(stats.iter().all(|s| !s.title.is_empty()));
    assert_eq!(
        RunStats::parse(RunStats::BUNDLED).as_ref(),
        Ok(RunStats::bundled())
    );
}

#[test]
fn a_valid_file_parses_in_its_order() {
    let stats = parse(&file(vec![stat("b"), stat("a")])).unwrap();
    let ids: Vec<&ContentId> = stats.stats().iter().map(|s| &s.id).collect();
    assert_eq!(ids, [&ContentId("b".into()), &ContentId("a".into())]);
}

#[test]
fn a_file_over_the_limit_is_not_parsed() {
    let big = vec![b' '; MAX_RUN_STATS_BYTES + 1];
    assert_eq!(
        RunStats::parse(&big),
        Err(Error::TooLarge {
            size: MAX_RUN_STATS_BYTES + 1,
            limit: MAX_RUN_STATS_BYTES
        })
    );
    let mut padded = file(vec![stat("speed")]).to_string().into_bytes();
    padded.resize(MAX_RUN_STATS_BYTES, b' ');
    assert!(RunStats::parse(&padded).is_ok(), "at the limit it parses");
}

#[test]
fn only_the_run_stats_format_parses() {
    for bad in [
        json!("not an object"),
        json!({ "schema_version": 0, "sim_version": 0 }),
        {
            let mut v = file(vec![stat("speed")]);
            v["extra"] = json!(1);
            v
        },
        one_with("extra", json!(1)),
        one_with("effect", json!("no_such_effect")),
        one_with("step", json!(-1)),
        one_with("limit", json!(256)),
    ] {
        assert!(
            matches!(parse(&bad), Err(Error::Parse { .. })),
            "{bad}: {:?}",
            parse(&bad)
        );
    }
    assert!(matches!(RunStats::parse(b"{"), Err(Error::Parse { .. })));
}

#[test]
fn a_newer_file_is_unsupported() {
    let mut v = file(vec![stat("speed")]);
    v["sim_version"] = json!(1);
    // A newer file may have fields this game does not know.
    v["stats"][0]["extra"] = json!(1);
    assert_eq!(
        parse(&v),
        Err(Error::UnsupportedVersion {
            schema_version: 0,
            sim_version: 1
        })
    );
}

#[test]
fn one_to_the_most_stats() {
    invalid_at(&file(vec![]), "$.stats");
    let at: Vec<Value> = (0..MAX_STATS).map(|i| stat(&format!("s{i}"))).collect();
    assert!(parse(&file(at)).is_ok());
    let past: Vec<Value> = (0..=MAX_STATS).map(|i| stat(&format!("s{i}"))).collect();
    invalid_at(&file(past), "$.stats");
}

#[test]
fn ids_are_well_formed_and_unique() {
    invalid_at(&file(vec![stat("Not An ID")]), "$.stats[0].id");
    invalid_at(&file(vec![stat("a"), stat("a")]), "$.stats[1].id");
    // Not registry entries: any well-formed ID is a stat's own.
    assert!(parse(&file(vec![stat("crit_chance")])).is_ok());
}

#[test]
fn steps_and_limits_are_within_bounds() {
    assert!(parse(&one_with("step", json!(MAX_STEP))).is_ok());
    invalid_at(&one_with("step", json!(0)), "$.stats[0].step");
    invalid_at(&one_with("step", json!(MAX_STEP + 1)), "$.stats[0].step");
    assert!(parse(&one_with("limit", json!(MAX_LIMIT))).is_ok());
    invalid_at(&one_with("limit", json!(0)), "$.stats[0].limit");
    invalid_at(&one_with("limit", json!(MAX_LIMIT + 1)), "$.stats[0].limit");
}

#[test]
fn titles_and_descriptions_are_bounded_text() {
    let title_at = "é".repeat(MAX_TITLE_CHARS);
    assert!(parse(&one_with("title", json!(title_at))).is_ok());
    invalid_at(&one_with("title", json!("")), "$.stats[0].title");
    invalid_at(
        &one_with("title", json!("t".repeat(MAX_TITLE_CHARS + 1))),
        "$.stats[0].title",
    );
    assert!(parse(&one_with("description", json!(""))).is_ok());
    let description_at = "d".repeat(MAX_DESCRIPTION_CHARS);
    assert!(parse(&one_with("description", json!(description_at))).is_ok());
    invalid_at(
        &one_with("description", json!("d".repeat(MAX_DESCRIPTION_CHARS + 1))),
        "$.stats[0].description",
    );
    invalid_at(&one_with("title", json!("a\u{7}b")), "$.stats[0].title");
}

#[test]
fn the_hash_is_of_the_bytes() {
    let a = file(vec![stat("speed")]).to_string();
    let spaced = format!("{a} ");
    assert_eq!(
        RunStats::parse(a.as_bytes()).unwrap().sha256(),
        RunStats::parse(a.as_bytes()).unwrap().sha256()
    );
    assert_ne!(
        RunStats::parse(a.as_bytes()).unwrap().sha256(),
        RunStats::parse(spaced.as_bytes()).unwrap().sha256()
    );
}

// --- Other stats in a run ----------------------------------------------------------------------

/// The bundled stats with the speed step doubled.
fn faster() -> RunStats {
    let mut v: Value = serde_json::from_slice(RunStats::BUNDLED).unwrap();
    v["stats"][4]["step"] = json!(16);
    RunStats::parse(v.to_string().as_bytes()).unwrap()
}

#[test]
fn the_content_hash_covers_the_run_stats() {
    assert_eq!(
        content_sha256_with("trial", RunContent::bundled()),
        content_sha256("trial")
    );
    assert_ne!(
        content_sha256_with(
            "trial",
            &RunContent::new(faster(), Rewards::bundled().clone())
        ),
        content_sha256("trial")
    );
}

#[test]
fn a_run_with_other_stats_verifies_only_with_those_stats() {
    let stats = faster();
    let content = RunContent::new(stats.clone(), Rewards::bundled().clone());
    let mut recorder = RunRecorder::start_with(&trial_setup(), &content).unwrap();
    for t in 0..50 {
        recorder.step(run_script(t)).unwrap();
    }
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
    assert_eq!(verify_run(&bytes), Err(RunReplayError::ContentChanged));
}

#[test]
fn an_engine_started_with_other_stats_uses_them() {
    let stats = faster();
    let e = Engine::start_with(
        &trial_setup(),
        &RunContent::new(stats.clone(), Rewards::bundled().clone()),
    )
    .unwrap();
    assert_eq!(e.run_stats(), &stats);
    assert_eq!(
        e.snapshot().progress.unwrap().spent.len(),
        stats.stats().len()
    );
}

#[test]
fn the_attack_speed_total_saturates_at_its_cap() {
    // Expertise options have no stack limit, so the total is bounded here instead.
    assert_eq!(capped_attack_speed_pct(0), 0);
    assert_eq!(
        capped_attack_speed_pct(MAX_ATTACK_SPEED_PCT),
        MAX_ATTACK_SPEED_PCT
    );
    assert_eq!(
        capped_attack_speed_pct(MAX_ATTACK_SPEED_PCT + 1),
        MAX_ATTACK_SPEED_PCT
    );
    assert_eq!(capped_attack_speed_pct(u32::MAX), MAX_ATTACK_SPEED_PCT);
}
