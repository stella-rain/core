//! The stage skills file (ADR-075, ADR-098): content of its own beside the rewards file that
//! names its skills, read from JSON as untrusted bytes (ADR-020): a size limit before parsing,
//! strict parsing, the current versions only and every field bounded. A skill has an icon, a
//! title and a description, and each level a cooldown and a list of effects, at most one of
//! each building block and at most `MAX_SKILL_EFFECTS`.

use serde_json::{Value, json};
use stella_rain_core::id::ContentId;
use stella_rain_core::run_stats::{MAX_DESCRIPTION_CHARS, MAX_TITLE_CHARS};
use stella_rain_core::stage_skill::{
    MAX_PULL, MAX_SHIELD, MAX_SKILL_AHEAD, MAX_SKILL_COOLDOWN_TICKS, MAX_SKILL_DURATION_TICKS,
    MAX_SKILL_EFFECTS, MAX_SKILL_LEVELS, MAX_SKILL_PCT, MAX_SKILL_RADIUS, MAX_STAGE_SKILLS,
    MAX_STAGE_SKILLS_BYTES, SkillEffect, StageSkills,
};
use stella_rain_core::validate::Error;

fn self_status() -> Value {
    json!({ "type": "self_status", "status": "atk_up", "value_pct": 20, "duration_ticks": 120 })
}

fn shield() -> Value {
    json!({ "type": "shield", "amount": 5, "duration_ticks": 60 })
}

fn pull() -> Value {
    json!({ "type": "pull", "radius": 4096, "strength": 256, "duration_ticks": 60 })
}

fn area_status() -> Value {
    json!({ "type": "area_status", "status": "vulnerable", "radius": 4096, "value_pct": 20,
            "duration_ticks": 60 })
}

fn level(effects: Value) -> Value {
    json!({ "cooldown_ticks": 600, "effects": effects })
}

fn skill(id: &str) -> Value {
    json!({ "id": id, "icon": "icon_war_cry", "title": "Aura", "description": "Hit harder",
            "levels": [level(json!([self_status()]))] })
}

fn file(skills: Vec<Value>) -> Value {
    json!({ "schema_version": 0, "sim_version": 0, "stage_skills": skills })
}

fn one() -> Value {
    file(vec![skill("aura")])
}

fn parse(v: &Value) -> Result<StageSkills, Error> {
    StageSkills::parse(v.to_string().as_bytes())
}

/// `one` with its one level's effects replaced.
fn effects(list: Value) -> Value {
    let mut v = one();
    v["stage_skills"][0]["levels"][0]["effects"] = list;
    v
}

/// `one` with its one level's one effect replaced.
fn effect(e: Value) -> Value {
    effects(json!([e]))
}

fn effect_at(field: &str) -> String {
    format!("$.stage_skills[0].levels[0].effects[0].{field}")
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
fn the_bundled_skills_are_one_effect_each_and_cover_every_block() {
    let bundled = StageSkills::bundled();
    assert_eq!(
        StageSkills::parse(StageSkills::BUNDLED).as_ref(),
        Ok(bundled)
    );
    let ids: Vec<&str> = bundled.skills().iter().map(|k| k.id.0.as_str()).collect();
    assert_eq!(ids, ["black_hole", "war_cry", "hex", "nova", "aegis"]);
    let mut blocks = Vec::new();
    for k in bundled.skills() {
        assert!(
            !k.title.is_empty() && !k.description.is_empty(),
            "{:?}",
            k.id
        );
        assert_eq!(k.icon, ContentId(format!("icon_{}", k.id.0)));
        for l in &k.levels {
            assert_eq!(l.effects.len(), 1, "{:?}", k.id);
        }
        blocks.push(k.levels[0].effects[0].block());
    }
    assert_eq!(
        blocks,
        ["pull", "self_status", "area_status", "attack", "shield"]
    );
    assert_eq!(
        bundled.index_of(&ContentId("hex".into())),
        Some(2),
        "a skill's place in the file"
    );
    assert_eq!(bundled.index_of(&ContentId("other".into())), None);
}

#[test]
fn a_level_holds_one_to_max_effects_at_most_one_of_each_block() {
    let attack = json!({ "type": "attack", "attack": { "preset": { "id": "slam" } } });
    let every = [pull(), area_status(), shield(), self_status(), attack];
    // The budget binds: fewer than the five blocks.
    assert!(MAX_SKILL_EFFECTS < every.len());
    assert!(parse(&effects(json!(every[..MAX_SKILL_EFFECTS]))).is_ok());
    invalid_at(
        &effects(json!(every[..=MAX_SKILL_EFFECTS])),
        "$.stage_skills[0].levels[0].effects",
    );
    invalid_at(&effects(json!([])), "$.stage_skills[0].levels[0].effects");
    invalid_at(
        &effects(json!([shield(), pull(), shield()])),
        "$.stage_skills[0].levels[0].effects[2]",
    );
    // Two status blocks are two blocks, whatever their statuses.
    assert!(parse(&effects(json!([self_status(), area_status()]))).is_ok());
}

#[test]
fn the_effects_parse_in_their_order() {
    let skills = parse(&effects(json!([shield(), pull()]))).unwrap();
    let blocks: Vec<&str> = skills.skills()[0].levels[0]
        .effects
        .iter()
        .map(SkillEffect::block)
        .collect();
    assert_eq!(blocks, ["shield", "pull"]);
}

#[test]
fn levels_may_hold_different_blocks() {
    let mut v = one();
    v["stage_skills"][0]["levels"] = json!([
        level(json!([self_status()])),
        level(json!([self_status(), shield()]))
    ]);
    assert!(parse(&v).is_ok());
}

#[test]
fn a_skill_has_one_to_max_levels() {
    let levels = |n: usize| {
        let mut v = one();
        v["stage_skills"][0]["levels"] = json!(vec![level(json!([shield()])); n]);
        v
    };
    assert!(parse(&levels(MAX_SKILL_LEVELS)).is_ok());
    invalid_at(&levels(0), "$.stage_skills[0].levels");
    invalid_at(&levels(MAX_SKILL_LEVELS + 1), "$.stage_skills[0].levels");
}

#[test]
fn ids_are_well_formed_and_unique_and_the_list_is_bounded() {
    invalid_at(
        &file(vec![skill("aura"), skill("aura")]),
        "$.stage_skills[1].id",
    );
    invalid_at(
        &file(vec![skill("aura"), skill("Bad")]),
        "$.stage_skills[1].id",
    );
    let many = |n: usize| file((0..n).map(|i| skill(&format!("s{i}"))).collect());
    assert!(parse(&many(MAX_STAGE_SKILLS)).is_ok());
    invalid_at(&many(MAX_STAGE_SKILLS + 1), "$.stage_skills");
    // A file may hold none: a pack with no skills of its own.
    assert!(parse(&many(0)).is_ok());
}

#[test]
fn the_icon_is_a_registered_asset_and_the_texts_are_bounded() {
    let with = |field: &str, value: Value| {
        let mut v = one();
        v["stage_skills"][0][field] = value;
        v
    };
    for icon in ["icon_black_hole", "icon_aegis"] {
        assert!(parse(&with("icon", json!(icon))).is_ok(), "{icon}");
    }
    invalid_at(
        &with("icon", json!("no_such_icon")),
        "$.stage_skills[0].icon",
    );
    invalid_at(&with("icon", json!("Bad Icon")), "$.stage_skills[0].icon");
    // An asset of the registry, not any ID: a status is not an icon.
    invalid_at(&with("icon", json!("atk_up")), "$.stage_skills[0].icon");
    let long = |n: usize| json!("x".repeat(n));
    assert!(parse(&with("title", long(MAX_TITLE_CHARS))).is_ok());
    invalid_at(&with("title", json!("")), "$.stage_skills[0].title");
    invalid_at(
        &with("title", long(MAX_TITLE_CHARS + 1)),
        "$.stage_skills[0].title",
    );
    assert!(parse(&with("description", json!(""))).is_ok());
    assert!(parse(&with("description", long(MAX_DESCRIPTION_CHARS))).is_ok());
    invalid_at(
        &with("description", long(MAX_DESCRIPTION_CHARS + 1)),
        "$.stage_skills[0].description",
    );
    invalid_at(
        &with("description", json!("a\u{7}b")),
        "$.stage_skills[0].description",
    );
}

#[test]
fn a_pull_s_numbers_are_bounded() {
    let pull = |radius: i64, strength: i64, ticks: i64| {
        effect(
            json!({ "type": "pull", "radius": radius, "strength": strength,
                       "duration_ticks": ticks }),
        )
    };
    let (r, p, d) = (
        i64::from(MAX_SKILL_RADIUS),
        i64::from(MAX_PULL),
        i64::from(MAX_SKILL_DURATION_TICKS),
    );
    assert!(parse(&pull(0, 0, 1)).is_ok());
    assert!(parse(&pull(r, p, d)).is_ok());
    invalid_at(&pull(-1, p, d), &effect_at("radius"));
    invalid_at(&pull(r + 1, p, d), &effect_at("radius"));
    invalid_at(&pull(r, -1, d), &effect_at("strength"));
    invalid_at(&pull(r, p + 1, d), &effect_at("strength"));
    invalid_at(&pull(r, p, 0), &effect_at("duration_ticks"));
    invalid_at(&pull(r, p, d + 1), &effect_at("duration_ticks"));
}

#[test]
fn the_status_blocks_name_a_status_and_their_numbers_are_bounded() {
    let pct = i64::from(MAX_SKILL_PCT);
    let d = i64::from(MAX_SKILL_DURATION_TICKS);
    let on_self = |status: &str, pct: i64, ticks: i64| {
        effect(
            json!({ "type": "self_status", "status": status, "value_pct": pct,
                       "duration_ticks": ticks }),
        )
    };
    assert!(parse(&on_self("atk_up", pct, d)).is_ok());
    // Any status is open to both blocks (ADR-098).
    assert!(parse(&on_self("slow", 1, 1)).is_ok());
    invalid_at(&on_self("no_such_status", 1, 1), &effect_at("status"));
    invalid_at(&on_self("pilot_a", 1, 1), &effect_at("status"));
    invalid_at(&on_self("atk_up", 0, 1), &effect_at("value_pct"));
    invalid_at(&on_self("atk_up", pct + 1, 1), &effect_at("value_pct"));
    invalid_at(&on_self("atk_up", 1, 0), &effect_at("duration_ticks"));
    invalid_at(&on_self("atk_up", 1, d + 1), &effect_at("duration_ticks"));
    let r = i64::from(MAX_SKILL_RADIUS);
    let area = |status: &str, radius: i64, pct: i64, ticks: i64| {
        effect(
            json!({ "type": "area_status", "status": status, "radius": radius,
                       "value_pct": pct, "duration_ticks": ticks }),
        )
    };
    assert!(parse(&area("vulnerable", r, pct, d)).is_ok());
    assert!(parse(&area("slow", 0, 1, 1)).is_ok());
    invalid_at(&area("no_such_status", 0, 1, 1), &effect_at("status"));
    invalid_at(&area("slow", -1, 1, 1), &effect_at("radius"));
    invalid_at(&area("slow", r + 1, 1, 1), &effect_at("radius"));
    invalid_at(&area("slow", 0, 0, 1), &effect_at("value_pct"));
    invalid_at(&area("slow", 0, pct + 1, 1), &effect_at("value_pct"));
    invalid_at(&area("slow", 0, 1, 0), &effect_at("duration_ticks"));
    invalid_at(&area("slow", 0, 1, d + 1), &effect_at("duration_ticks"));
}

#[test]
fn a_shield_s_numbers_are_bounded() {
    let shield = |amount: i64, ticks: i64| {
        effect(json!({ "type": "shield", "amount": amount, "duration_ticks": ticks }))
    };
    let (a, d) = (i64::from(MAX_SHIELD), i64::from(MAX_SKILL_DURATION_TICKS));
    assert!(parse(&shield(a, d)).is_ok());
    invalid_at(&shield(0, d), &effect_at("amount"));
    invalid_at(&shield(a + 1, d), &effect_at("amount"));
    invalid_at(&shield(a, 0), &effect_at("duration_ticks"));
    invalid_at(&shield(a, d + 1), &effect_at("duration_ticks"));
}

#[test]
fn an_attack_is_held_to_an_attack_s_budgets() {
    let attack = |attack: Value| effect(json!({ "type": "attack", "attack": attack }));
    let circle = json!([{ "kind": "active", "ticks": 10,
                          "hit_box": { "type": "circle", "radius": 8192 } }]);
    assert!(parse(&attack(json!({ "inline": circle }))).is_ok());
    assert!(parse(&attack(json!({ "preset": { "id": "slam" } }))).is_ok());
    invalid_at(
        &attack(json!({ "inline": [] })),
        &effect_at("attack.inline"),
    );
    invalid_at(
        &attack(json!({ "preset": { "id": "no_such_attack" } })),
        &effect_at("attack.preset.id"),
    );
}

#[test]
fn the_cooldown_and_the_distance_ahead_are_bounded() {
    let c = i64::from(MAX_SKILL_COOLDOWN_TICKS);
    for (cooldown, ok) in [(1, true), (c, true), (0, false), (c + 1, false)] {
        let mut v = one();
        v["stage_skills"][0]["levels"][0]["cooldown_ticks"] = json!(cooldown);
        if ok {
            assert!(parse(&v).is_ok(), "cooldown {cooldown}");
        } else {
            invalid_at(&v, "$.stage_skills[0].levels[0].cooldown_ticks");
        }
    }
    let a = i64::from(MAX_SKILL_AHEAD);
    for (ahead, ok) in [(0, true), (a, true), (-1, false), (a + 1, false)] {
        let mut v = one();
        v["stage_skills"][0]["ahead"] = json!(ahead);
        if ok {
            assert!(parse(&v).is_ok(), "ahead {ahead}");
        } else {
            invalid_at(&v, "$.stage_skills[0].ahead");
        }
    }
}

#[test]
fn a_file_over_the_limit_is_not_parsed() {
    let big = vec![b' '; MAX_STAGE_SKILLS_BYTES + 1];
    assert_eq!(
        StageSkills::parse(&big),
        Err(Error::TooLarge {
            size: MAX_STAGE_SKILLS_BYTES + 1,
            limit: MAX_STAGE_SKILLS_BYTES
        })
    );
    let mut padded = one().to_string().into_bytes();
    padded.resize(MAX_STAGE_SKILLS_BYTES, b' ');
    assert!(
        StageSkills::parse(&padded).is_ok(),
        "at the limit it parses"
    );
}

#[test]
fn only_the_stage_skills_format_parses() {
    let with_skill = |field: &str, value: Value| {
        let mut v = one();
        v["stage_skills"][0][field] = value;
        v
    };
    for bad in [
        json!("not an object"),
        json!({ "schema_version": 0, "sim_version": 0 }),
        {
            let mut v = one();
            v["extra"] = json!(1);
            v
        },
        with_skill("extra", json!(1)),
        with_skill("kind", json!("buff")),
        // A level's one `effect` of before is gone.
        with_skill(
            "levels",
            json!([{ "cooldown_ticks": 60, "effect": self_status() }]),
        ),
        effect(json!({ "type": "black_hole", "radius": 1, "pull": 1, "duration_ticks": 1 })),
        effect(json!({ "type": "shield", "amount": 1, "duration_ticks": 1, "extra": 1 })),
    ] {
        assert!(
            matches!(parse(&bad), Err(Error::Parse { .. })),
            "{bad}: {:?}",
            parse(&bad)
        );
    }
}

#[test]
fn a_newer_file_is_unsupported() {
    let mut v = one();
    v["sim_version"] = json!(1);
    v["stage_skills"][0]["extra"] = json!(1);
    assert_eq!(
        parse(&v),
        Err(Error::UnsupportedVersion {
            schema_version: 0,
            sim_version: 1
        })
    );
}
