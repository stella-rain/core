//! The pack's `rules.json` (DIR1, DS103): the crowd target and the cap's headroom, the first
//! numbers of the simulation that live in the pack. The cap is derived from them and never
//! stored; the file is a required slot of the pack, and the derived content hash covers it.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::Fx;
use stella_rain_core::pack::{ContentFiles, Pack, PackError};
use stella_rain_core::rules::{MAX_RULES_BYTES, Rules};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::content_sha256_with;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{Error, limits};
use stella_rain_core::version::Versions;

/// The bundled file with the crowd's two numbers set.
fn file(crowd_target: u64, cap_headroom_pct: u64) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v["crowd_target"] = json!(crowd_target);
    v["cap_headroom_pct"] = json!(cap_headroom_pct);
    v.to_string().into_bytes()
}

/// The bundled file with one field set.
fn with(field: &str, value: i64) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v[field] = json!(value);
    v.to_string().into_bytes()
}

/// The bundled file with several fields set.
fn with_all(fields: &[(&str, i64)]) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    for (field, value) in fields {
        v[*field] = json!(*value);
    }
    v.to_string().into_bytes()
}

fn paths(result: Result<Rules, Error>) -> Vec<String> {
    match result {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

#[test]
fn the_bundled_file_gives_a_target_of_200_and_a_cap_of_260() {
    let rules = Rules::bundled();
    assert_eq!(rules.crowd_target(), 200);
    assert_eq!(rules.cap_headroom_pct(), 30);
    assert_eq!(rules.enemy_cap_for(200), 260);
    assert_eq!(
        Rules::parse(common::bundled_file("rules"), Versions::CURRENT).unwrap(),
        *rules
    );
}

#[test]
fn the_limits_are_the_engine_ceiling_and_the_most_any_target_can_be() {
    assert_eq!(limits::MAX_ENEMY_CAP, 500);
    assert_eq!(limits::MAX_CROWD_TARGET, 400);
}

#[test]
fn the_headroom_is_25_to_30_percent() {
    for bad in [24, 31] {
        assert_eq!(
            paths(Rules::parse(&file(200, bad), Versions::CURRENT)),
            ["$.cap_headroom_pct"]
        );
    }
    for good in [25, 30] {
        assert!(Rules::parse(&file(200, good), Versions::CURRENT).is_ok());
    }
}

#[test]
fn the_target_keeps_its_cap_within_the_ceiling() {
    for bad in [0, 385] {
        assert_eq!(
            paths(Rules::parse(&file(bad, 30), Versions::CURRENT)),
            ["$.crowd_target"]
        );
    }
    // 384 * 130 / 100 = 499.
    assert_eq!(
        Rules::parse(&file(384, 30), Versions::CURRENT)
            .unwrap()
            .enemy_cap_for(384),
        499
    );
    // 400 * 125 / 100 = 500.
    assert_eq!(
        Rules::parse(&file(400, 25), Versions::CURRENT)
            .unwrap()
            .enemy_cap_for(400),
        500
    );
    assert_eq!(
        paths(Rules::parse(&file(401, 25), Versions::CURRENT)),
        ["$.crowd_target"]
    );
}

#[test]
fn an_override_is_clamped_so_the_cap_stays_within_the_ceiling() {
    let rules = Rules::bundled();
    assert_eq!(rules.enemy_cap_for(100), 130);
    assert_eq!(rules.enemy_cap_for(384), 499);
    assert_eq!(rules.enemy_cap_for(400), 499);
    assert_eq!(rules.enemy_cap_for(u16::MAX), 499);
    let tight = Rules::parse(&file(200, 25), Versions::CURRENT).unwrap();
    assert_eq!(tight.enemy_cap_for(400), 500);
}

#[test]
fn unknown_fields_other_versions_and_big_files_are_refused() {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v["extra"] = 1.into();
    assert!(matches!(
        Rules::parse(v.to_string().as_bytes(), Versions::CURRENT),
        Err(Error::Parse { .. })
    ));
    let v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    assert!(matches!(
        Rules::parse(
            v.to_string().as_bytes(),
            Versions {
                schema_version: 0,
                sim_version: 1
            }
        ),
        Err(Error::UnsupportedVersion { .. })
    ));
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v["sim_version"] = 0.into();
    assert!(matches!(
        Rules::parse(v.to_string().as_bytes(), Versions::CURRENT),
        Err(Error::Parse { .. })
    ));
    let mut big = common::bundled_file("rules").to_vec();
    big.resize(MAX_RULES_BYTES + 1, b' ');
    assert!(matches!(
        Rules::parse(&big, Versions::CURRENT),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn the_pack_has_seventeen_slots_and_rules_is_required() {
    assert_eq!(ContentFiles::LAYOUT.len(), 17);
    let slot = ContentFiles::LAYOUT
        .iter()
        .find(|s| s.name == "rules")
        .expect("a rules slot");
    assert!(!slot.optional && !slot.in_modes);
}

#[test]
fn a_pack_without_rules_is_refused() {
    // Exporting names the slot.
    let bundled = ContentFiles::bundled();
    let mut files = ContentFiles::default();
    for slot in ContentFiles::LAYOUT {
        if slot.name != "rules" {
            files.insert(slot.name, bundled.get(slot.name).unwrap().to_vec());
        }
    }
    assert!(matches!(
        files.export(),
        Err(PackError::Missing { file: "rules" })
    ));
    // Reading a pack that lacks it fails closed.
    let mut pack: Value = serde_json::from_slice(&bundled.export().unwrap()).unwrap();
    pack.as_object_mut().unwrap().remove("rules");
    assert!(Pack::from_bytes(pack.to_string().as_bytes()).is_err());
}

#[test]
fn the_bundled_pack_holds_the_bundled_file_and_round_trips() {
    let bytes = ContentFiles::bundled().export().unwrap();
    let pack = Pack::from_bytes(&bytes).unwrap();
    assert_eq!(pack.content().rules(), Rules::bundled());
    let again = ContentFiles::from_pack(&bytes).unwrap();
    assert_eq!(again.get("rules").unwrap(), common::bundled_file("rules"));
    assert_eq!(again.export().unwrap(), bytes);
}

#[test]
fn replacing_the_rules_file_changes_the_derived_content_hash() {
    let base = RunContent::bundled().clone();
    let changed = base
        .clone()
        .with_rules(Rules::parse(&file(150, 30), Versions::CURRENT).unwrap());
    assert_eq!(changed.rules().crowd_target(), 150);
    for mode in ["trial", "time_limit", "tutorial"] {
        assert_ne!(
            content_sha256_with(mode, &base.clone().with_rules(Rules::bundled().clone())),
            content_sha256_with(mode, &changed),
            "{mode}"
        );
    }
}

#[test]
fn the_bundled_file_holds_the_designer_facing_rules() {
    let r = Rules::bundled();
    assert_eq!(r.player_speed(), Fx(768));
    assert_eq!(r.player_radius(), Fx(512));
    assert_eq!(r.orb_radius(), Fx(768));
    assert_eq!(r.hit_invulnerable_ticks(), 60);
    assert_eq!(r.aggro_per_damage(), 1);
    assert_eq!(r.aggro_per_buff(), 4000);
    assert_eq!(r.aggro_per_heal_point(), 2);
    assert_eq!(r.aggro_decay(), 100);
    assert_eq!(r.leash_reach_x(), Fx(215_040));
    assert_eq!(r.leash_reach_y(), Fx(92_160));
    assert_eq!(r.leash_land_x(), Fx(123_904));
    assert_eq!(r.leash_land_y(), Fx(62_464));
    assert_eq!(r.spawn_margin(), Fx(16_384));
    assert_eq!(r.ranged_token_cooldown_ticks(), 30);
    assert_eq!(r.ranged_zone_tokens(), 3);
    assert_eq!(r.ranged_line_tokens(), 1);
    assert_eq!(r.orbit_step(), 384);
    assert_eq!(r.boss_arrival_above(), Fx(30_720));
    assert_eq!(r.default_xp(), 1);
    assert_eq!(r.offer_size(), 3);
    assert_eq!(r.defence_k(), 10_000);
    assert_eq!(r.crit_damage_base_pct(), 200);
    assert_eq!(
        Rules::parse(common::bundled_file("rules"), Versions::CURRENT).unwrap(),
        *r
    );
}

#[test]
fn each_new_field_is_refused_outside_its_bound_at_its_path() {
    let px = |n: i64| n * 256;
    let refused: &[(&str, i64)] = &[
        ("player_speed", 0),
        ("player_speed", 4097),
        ("player_radius", 0),
        ("player_radius", 65_537),
        ("orb_radius", 0),
        ("orb_radius", 65_537),
        ("hit_invulnerable_ticks", 601),
        ("aggro_per_damage", 101),
        ("aggro_per_buff", i64::from(limits::MAX_AGGRO) + 1),
        ("aggro_per_heal_point", 101),
        ("aggro_decay", i64::from(limits::MAX_AGGRO) + 1),
        ("leash_reach_x", 0),
        ("leash_reach_x", px(4096) + 1),
        ("leash_reach_y", 0),
        ("leash_reach_y", px(4096) + 1),
        ("spawn_margin", -1),
        ("spawn_margin", px(1024) + 1),
        ("ranged_token_cooldown_ticks", 601),
        ("orbit_step", 0),
        ("orbit_step", 2561),
        ("boss_arrival_above", -1),
        ("boss_arrival_above", px(1024) + 1),
        ("default_xp", 1001),
        ("offer_size", 0),
        ("offer_size", 5),
        ("ranged_zone_tokens", 0),
        ("ranged_line_tokens", 0),
        ("defence_k", 0),
        ("defence_k", 1_000_001),
        ("crit_damage_base_pct", 99),
        ("crit_damage_base_pct", 1001),
    ];
    for (field, value) in refused {
        assert_eq!(
            paths(Rules::parse(&with(field, *value), Versions::CURRENT)),
            [format!("$.{field}")],
            "{field} = {value}"
        );
    }
}

#[test]
fn each_new_field_loads_at_its_bounds() {
    let px = |n: i64| n * 256;
    let max_aggro = i64::from(limits::MAX_AGGRO);
    let ok: &[(&str, i64)] = &[
        ("player_speed", 1),
        ("player_speed", 4096),
        ("player_radius", 1),
        ("player_radius", 65_536),
        ("orb_radius", 1),
        ("orb_radius", 65_536),
        ("hit_invulnerable_ticks", 0),
        ("hit_invulnerable_ticks", 600),
        ("aggro_per_damage", 0),
        ("aggro_per_damage", 100),
        ("aggro_per_buff", 0),
        ("aggro_per_buff", max_aggro),
        ("aggro_per_heal_point", 100),
        ("aggro_decay", 0),
        ("aggro_decay", max_aggro),
        ("leash_reach_x", px(4096)),
        ("leash_reach_y", px(4096)),
        ("spawn_margin", 0),
        ("spawn_margin", px(1024)),
        ("ranged_token_cooldown_ticks", 0),
        ("ranged_token_cooldown_ticks", 600),
        ("orbit_step", 1),
        ("orbit_step", 2560),
        ("boss_arrival_above", 0),
        ("boss_arrival_above", px(1024)),
        ("default_xp", 0),
        ("default_xp", 1000),
        ("offer_size", 1),
        ("offer_size", 4),
        ("defence_k", 1),
        ("defence_k", 1_000_000),
        ("crit_damage_base_pct", 100),
        ("crit_damage_base_pct", 1000),
    ];
    for (field, value) in ok {
        let bytes = with(field, *value);
        // The leash lands inside where it triggers: a reach at its least needs a smaller land.
        Rules::parse(&bytes, Versions::CURRENT)
            .unwrap_or_else(|e| panic!("{field} = {value} refused: {e:?}"));
    }
}

#[test]
fn the_leash_lands_inside_where_it_triggers() {
    // The land equal to the reach loads; one more is refused at the land's path.
    let r = with_all(&[("leash_land_x", 215_040)]);
    assert!(Rules::parse(&r, Versions::CURRENT).is_ok());
    let r = with_all(&[("leash_land_x", 215_041)]);
    assert_eq!(
        paths(Rules::parse(&r, Versions::CURRENT)),
        ["$.leash_land_x"]
    );
    let r = with_all(&[("leash_land_y", 92_161)]);
    assert_eq!(
        paths(Rules::parse(&r, Versions::CURRENT)),
        ["$.leash_land_y"]
    );
    assert_eq!(
        paths(Rules::parse(&with("leash_land_x", 0), Versions::CURRENT)),
        ["$.leash_land_x"]
    );
}

#[test]
fn the_two_token_pools_sum_to_at_most_the_ranged_ceiling() {
    let ok = with_all(&[("ranged_zone_tokens", 2), ("ranged_line_tokens", 2)]);
    assert!(Rules::parse(&ok, Versions::CURRENT).is_ok());
    let ok = with_all(&[("ranged_zone_tokens", 1), ("ranged_line_tokens", 1)]);
    assert!(Rules::parse(&ok, Versions::CURRENT).is_ok());
    let bad = with_all(&[("ranged_zone_tokens", 3), ("ranged_line_tokens", 2)]);
    assert_eq!(
        paths(Rules::parse(&bad, Versions::CURRENT)),
        ["$.ranged_line_tokens"]
    );
}

#[test]
fn a_missing_or_unknown_new_field_is_a_parse_error_and_no_message_quotes_the_file() {
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v.as_object_mut().unwrap().remove("player_speed");
    assert!(matches!(
        Rules::parse(v.to_string().as_bytes(), Versions::CURRENT),
        Err(Error::Parse { .. })
    ));
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v["a_marker_string_xyzzy"] = json!(1);
    assert!(matches!(
        Rules::parse(v.to_string().as_bytes(), Versions::CURRENT),
        Err(Error::Parse { .. })
    ));
    let bad = with_all(&[("player_speed", 0), ("leash_land_x", 999_999_999)]);
    let Err(Error::Invalid(issues)) = Rules::parse(&bad, Versions::CURRENT) else {
        panic!("not invalid");
    };
    assert_eq!(issues.len(), 2);
    let mut v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    v["player_speed"] = json!("a_marker_string_xyzzy");
    let Err(e) = Rules::parse(v.to_string().as_bytes(), Versions::CURRENT) else {
        panic!("a string is not a number");
    };
    assert!(!format!("{e:?} {e}").contains("xyzzy"));
}

fn rules_with(fields: &[(&str, i64)]) -> Rules {
    Rules::parse(&with_all(fields), Versions::CURRENT).unwrap()
}

fn run_content_with(rules: Rules) -> RunContent {
    // The attacks of the older tests (100 stored), so a hit does not end the run.
    common::design_content().with_rules(rules)
}

#[test]
fn a_new_field_changes_the_derived_content_hash() {
    let base = RunContent::bundled().clone();
    let changed = run_content_with(rules_with(&[("player_speed", 1536)]));
    for mode in ["trial", "time_limit", "tutorial"] {
        assert_ne!(
            content_sha256_with(mode, &base),
            content_sha256_with(mode, &changed),
            "{mode}"
        );
    }
}

// --- A run reads its own rules -----------------------------------------------------------------

fn engine_on(rules: Rules, stage: &Stage) -> Engine {
    Engine::new_with(stage, &run_content_with(rules)).expect("the test stage is valid")
}

#[test]
fn a_run_moves_at_the_speed_of_its_own_rules() {
    let stage = common::stage_with(18_000, Some(common::far_boss()), vec![]);
    let mut slow = engine_on(Rules::bundled().clone(), &stage);
    let mut fast = engine_on(rules_with(&[("player_speed", 1536)]), &stage);
    let mut first = None;
    for t in 0..20 {
        slow.step(common::moving(127, 0));
        fast.step(common::moving(127, 0));
        if t == 0 {
            first = Some((slow.state_hash(), fast.state_hash()));
        }
    }
    let (a, b) = (common::player_at(&slow).0, common::player_at(&fast).0);
    assert_eq!(a - common::START_X, 20 * 768);
    assert_eq!(b - common::START_X, 2 * (a - common::START_X));
    let (a1, b1) = first.unwrap();
    assert_ne!(a1, b1, "the hashes differ from tick 1");
}

#[test]
fn a_run_is_hurt_for_as_long_as_its_own_rules_say() {
    let attack = json!({ "inline": [{ "kind": "active", "ticks": 1, "damage": 1,
        "spawn": [{ "type": "zone", "shape": { "type": "circle", "radius": 3_072 },
                    "anchor": "target", "wind_up_ticks": 1, "active_ticks": 1 }] }] });
    let shooter = common::enemy(
        1000,
        common::hold(common::START_X, common::START_Y - 10_240),
        json!([{ "when": common::always(), "target": "player",
                 "do": { "type": "attack", "attack": attack }, "cooldown_ticks": 8 }]),
    );
    let stage = common::stage_full(
        18_000,
        Some(common::far_boss()),
        vec![common::wave(
            1,
            1,
            0,
            shooter,
            common::START_X,
            common::START_Y - 10_240,
        )],
        vec![],
    );
    let hits = |rules: Rules| {
        let mut e = engine_on(rules, &stage);
        let mut hits = 0;
        for _ in 0..45 {
            e.step(common::idle());
            hits += e
                .events()
                .iter()
                .filter(|ev| matches!(ev, DomainEvent::PlayerHit { .. }))
                .count();
        }
        hits
    };
    assert_eq!(hits(Rules::bundled().clone()), 1);
    assert!(hits(rules_with(&[("hit_invulnerable_ticks", 0)])) > 1);
}
