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
    DEFAULT_MAX_ATTACK_SPEED_PCT, MAX_ATTACK_SPEED_PCT, MAX_HP_STEP, MAX_LIMIT,
    MAX_RUN_STATS_BYTES, MAX_STATS, MAX_STEP, MAX_STUN_IMMUNE_TICKS, RunStats, StatEffect,
    capped_attack_speed_pct,
};
use stella_rain_core::validate::Error;
use stella_rain_core::validate::limits::DAMAGE_SCALE;
use stella_rain_core::version::Versions;

use crate::common;

use common::run::{run_script, trial_setup};

fn stat(id: &str) -> Value {
    json!({
        "id": id,
        "effect": "speed_pct",
        "step": 5,
        "limit": 3
    })
}

fn file(stats: Vec<Value>) -> Value {
    json!({
        "stats": stats,
        "stun": { "immune_ticks": 60, "elite_pct": 50 },
    })
}

/// One valid stat with the stun rules `stun` in place of the bundled ones.
fn with_stun(stun: Value) -> Value {
    let mut v = file(vec![stat("speed")]);
    v["stun"] = stun;
    v
}

fn parse_at(v: &Value, versions: Versions) -> Result<RunStats, Error> {
    RunStats::parse(v.to_string().as_bytes(), versions)
}

fn parse(v: &Value) -> Result<RunStats, Error> {
    parse_at(v, Versions::CURRENT)
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
        .map(|s| (s.id.as_str(), s.effect, s.step, s.limit))
        .collect();
    assert_eq!(
        got,
        [
            ("max_hp", StatEffect::MaxHp, 100 * DAMAGE_SCALE, 5),
            ("attack", StatEffect::AttackPct, 10, 5),
            ("attack_speed", StatEffect::AttackSpeedPct, 8, 5),
            ("evade", StatEffect::EvadePct, 10, 5),
            ("speed", StatEffect::SpeedPct, 8, 5),
        ]
    );
    assert_eq!(
        RunStats::parse(common::bundled_file("run_stats"), Versions::CURRENT).as_ref(),
        Ok(RunStats::bundled())
    );
}

#[test]
fn a_valid_file_parses_in_its_order() {
    let stats = parse(&file(vec![stat("b"), stat("a")])).unwrap();
    let ids: Vec<&ContentId> = stats.stats().iter().map(|s| &s.id).collect();
    assert_eq!(ids, [&ContentId::from("b"), &ContentId::from("a")]);
}

#[test]
fn a_file_over_the_limit_is_not_parsed() {
    let big = vec![b' '; MAX_RUN_STATS_BYTES + 1];
    assert_eq!(
        RunStats::parse(&big, Versions::CURRENT),
        Err(Error::TooLarge {
            size: MAX_RUN_STATS_BYTES + 1,
            limit: MAX_RUN_STATS_BYTES
        })
    );
    let mut padded = file(vec![stat("speed")]).to_string().into_bytes();
    padded.resize(MAX_RUN_STATS_BYTES, b' ');
    assert!(
        RunStats::parse(&padded, Versions::CURRENT).is_ok(),
        "at the limit it parses"
    );
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
    assert!(matches!(
        RunStats::parse(b"{", Versions::CURRENT),
        Err(Error::Parse { .. })
    ));
}

#[test]
fn a_newer_file_is_unsupported() {
    let mut v = file(vec![stat("speed")]);
    // A newer file may have fields this game does not know.
    v["stats"][0]["extra"] = json!(1);
    assert_eq!(
        parse_at(
            &v,
            Versions {
                schema_version: 0,
                sim_version: 1
            }
        ),
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
fn a_hit_point_step_has_the_scaled_bound_and_the_others_keep_theirs() {
    // ADR-103: a point of Max HP is up to 100 times a percentage's step.
    assert_eq!(MAX_HP_STEP, MAX_STEP * 100);
    let hp = |step: u32| {
        let mut v = one_with("effect", json!("max_hp"));
        v["stats"][0]["step"] = json!(step);
        v
    };
    assert!(parse(&hp(MAX_HP_STEP)).is_ok());
    invalid_at(&hp(MAX_HP_STEP + 1), "$.stats[0].step");
    invalid_at(&one_with("step", json!(MAX_HP_STEP)), "$.stats[0].step");
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
fn text_fields_are_refused_as_unknown() {
    assert!(parse(&one_with("title", json!("Speed"))).is_err());
    assert!(parse(&one_with("description", json!("Move faster"))).is_err());
}

#[test]
fn the_hash_is_of_the_bytes() {
    let a = file(vec![stat("speed")]).to_string();
    let spaced = format!("{a} ");
    assert_eq!(
        RunStats::parse(a.as_bytes(), Versions::CURRENT)
            .unwrap()
            .sha256(),
        RunStats::parse(a.as_bytes(), Versions::CURRENT)
            .unwrap()
            .sha256()
    );
    assert_ne!(
        RunStats::parse(a.as_bytes(), Versions::CURRENT)
            .unwrap()
            .sha256(),
        RunStats::parse(spaced.as_bytes(), Versions::CURRENT)
            .unwrap()
            .sha256()
    );
}

// --- Other stats in a run ----------------------------------------------------------------------

/// The bundled stats with the speed step doubled.
fn faster() -> RunStats {
    let mut v: Value = serde_json::from_slice(common::bundled_file("run_stats")).unwrap();
    v["stats"][4]["step"] = json!(16);
    RunStats::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap()
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
    // Expertise options have no stack limit, so the total is bounded here instead; the cap is
    // the file's, held to the code ceiling.
    assert_eq!(capped_attack_speed_pct(0, 500), 0);
    assert_eq!(capped_attack_speed_pct(500, 500), 500);
    assert_eq!(capped_attack_speed_pct(501, 500), 500);
    assert_eq!(capped_attack_speed_pct(u32::MAX, 500), 500);
    assert_eq!(capped_attack_speed_pct(300, 300), 300);
    assert_eq!(
        capped_attack_speed_pct(u32::MAX, u32::MAX),
        MAX_ATTACK_SPEED_PCT,
        "an unvalidated cap is held to the ceiling"
    );
}

#[test]
fn the_attack_speed_cap_is_the_files_and_the_code_ceiling_bounds_it() {
    assert_eq!(RunStats::bundled().max_attack_speed_pct(), 500, "core#298");
    assert_eq!(DEFAULT_MAX_ATTACK_SPEED_PCT, 500, "a file naming none");
    let with_cap = |cap: Value| {
        let mut v = file(vec![stat("a")]);
        v["max_attack_speed_pct"] = cap;
        parse(&v)
    };
    let none = parse(&file(vec![stat("a")])).unwrap();
    assert_eq!(none.max_attack_speed_pct(), DEFAULT_MAX_ATTACK_SPEED_PCT);
    assert_eq!(
        with_cap(json!(300)).unwrap().max_attack_speed_pct(),
        300,
        "read from the file"
    );
    assert!(with_cap(json!(MAX_ATTACK_SPEED_PCT)).is_ok());
    assert!(with_cap(json!(MAX_ATTACK_SPEED_PCT + 1)).is_err());
    assert!(with_cap(json!(0)).is_err());
}

#[test]
fn the_stun_rules_are_bounded_and_the_bundled_ones_are_the_placeholders() {
    // ADR-085 points 4 and 6: the immunity after a stun and the elite's share of it, run rules
    // in the run stats file (ADR-067), both bounded (ADR-020).
    let bundled = RunStats::bundled().stun();
    assert_eq!((bundled.immune_ticks, bundled.elite_pct), (60, 50));
    assert!(parse(&with_stun(json!({ "immune_ticks": 0, "elite_pct": 0 }))).is_ok());
    assert!(
        parse(&with_stun(json!({
            "immune_ticks": MAX_STUN_IMMUNE_TICKS,
            "elite_pct": 100,
        })))
        .is_ok()
    );
    invalid_at(
        &with_stun(json!({ "immune_ticks": MAX_STUN_IMMUNE_TICKS + 1, "elite_pct": 50 })),
        "$.stun.immune_ticks",
    );
    invalid_at(
        &with_stun(json!({ "immune_ticks": 60, "elite_pct": 101 })),
        "$.stun.elite_pct",
    );
}
