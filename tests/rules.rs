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
use stella_rain_core::version::{NEWEST_READ_SCHEMA_VERSION, SIM_VERSION, Versions};

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
                schema_version: NEWEST_READ_SCHEMA_VERSION + 1,
                sim_version: SIM_VERSION + 1
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
fn the_pack_has_nineteen_slots_and_rules_is_required() {
    assert_eq!(ContentFiles::LAYOUT.len(), 19);
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
        // The bundled pack holds no threat colour table: that slot stays empty.
        if slot.name != "rules" {
            if let Some(bytes) = bundled.get(slot.name) {
                files.insert(slot.name, bytes.to_vec());
            }
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
        // `spawn_margin` at px(1024) needs lands out to its spawn edge:
        // `the_leash_lands_at_or_beyond_the_spawn_edge`.
        ("ranged_token_cooldown_ticks", 0),
        ("ranged_token_cooldown_ticks", 600),
        ("orbit_step", 1),
        ("orbit_step", 2560),
        ("boss_arrival_above", 0),
        ("boss_arrival_above", px(1024)),
        ("default_xp", 0),
        ("default_xp", 1000),
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
fn the_leash_lands_at_or_beyond_the_spawn_edge() {
    // The spawn edge is the view's half (420 x 180 px) plus `spawn_margin` (64 px bundled):
    // 123,904 and 62,464, which the bundled lands equal.
    let rules = Rules::bundled();
    assert_eq!(rules.leash_land_x(), Fx(123_904));
    assert_eq!(rules.leash_land_y(), Fx(62_464));
    assert_eq!(
        paths(Rules::parse(
            &with("leash_land_x", 123_903),
            Versions::CURRENT
        )),
        ["$.leash_land_x"]
    );
    assert_eq!(
        paths(Rules::parse(
            &with("leash_land_y", 62_463),
            Versions::CURRENT
        )),
        ["$.leash_land_y"]
    );
    // A wider margin moves the edge past the bundled lands (66.4 px).
    assert_eq!(
        paths(Rules::parse(
            &with("spawn_margin", 17_000),
            Versions::CURRENT
        )),
        ["$.leash_land_x", "$.leash_land_y"]
    );
    // The largest margin loads with lands (and reaches) out to its edge: 1,444 x 1,204 px.
    let r = with_all(&[
        ("spawn_margin", 1024 * 256),
        ("leash_land_x", 1444 * 256),
        ("leash_land_y", 1204 * 256),
        ("leash_reach_x", 1444 * 256),
        ("leash_reach_y", 1204 * 256),
    ]);
    assert!(Rules::parse(&r, Versions::CURRENT).is_ok());
    // A land already out of range is one issue, not two; a margin out of range is the margin's.
    assert_eq!(
        paths(Rules::parse(&with("leash_land_x", 0), Versions::CURRENT)),
        ["$.leash_land_x"]
    );
    assert_eq!(
        paths(Rules::parse(
            &with("spawn_margin", 1025 * 256),
            Versions::CURRENT
        )),
        ["$.spawn_margin"]
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
        slow.step(&[
            common::moving(127, 0),
            Default::default(),
            Default::default(),
        ]);
        fast.step(&[
            common::moving(127, 0),
            Default::default(),
            Default::default(),
        ]);
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
            e.step(&[common::idle(), Default::default(), Default::default()]);
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

#[test]
fn the_bundled_timer_is_300_and_its_bounds_are_checked() {
    use stella_rain_core::rules::MAX_OFFER_TIMER_TICKS;
    assert_eq!(Rules::bundled().offer_timer_ticks(), 300);
    assert_eq!(MAX_OFFER_TIMER_TICKS, 3600);
    let bundled: serde_json::Value =
        serde_json::from_slice(stella_rain_core::pack::ContentFiles::bundled_file("rules"))
            .unwrap();
    // At sim_version 2 the key is deprecated; the frozen version 1 rules still read it.
    let versions = Versions {
        schema_version: 1,
        sim_version: 1,
    };
    let with = |n: u32| {
        let mut v = bundled.clone();
        v["offer_timer_ticks"] = serde_json::json!(n);
        Rules::parse(v.to_string().as_bytes(), versions)
    };
    assert!(with(1).is_ok());
    assert!(with(3600).is_ok());
    for bad in [0, 3601] {
        let msg = format!("{:?}", with(bad).unwrap_err());
        assert!(msg.contains("offer_timer_ticks"), "{msg}");
    }
}

const V1: Versions = Versions {
    schema_version: 1,
    sim_version: 1,
};

fn bundled_value() -> Value {
    serde_json::from_slice(ContentFiles::bundled_file("rules")).unwrap()
}

#[test]
fn offer_size_is_checked_at_its_bounds_at_version_1() {
    for (n, ok) in [(0, false), (1, true), (4, true), (5, false)] {
        let result = Rules::parse(&with("offer_size", n), V1);
        if ok {
            assert_eq!(
                result.unwrap().offer_size(),
                u8::try_from(n).unwrap(),
                "{n}"
            );
        } else {
            assert_eq!(paths(result), ["$.offer_size"], "{n}");
        }
    }
}

#[test]
fn a_version_2_file_refuses_a_non_default_offer_key() {
    let issues = |fields: &[(&str, i64)]| paths(Rules::parse(&with_all(fields), Versions::CURRENT));
    assert_eq!(issues(&[("offer_size", 4)]), ["$.offer_size"]);
    assert_eq!(
        issues(&[("offer_timer_ticks", 600)]),
        ["$.offer_timer_ticks"]
    );
    assert_eq!(
        issues(&[("offer_size", 4), ("offer_timer_ticks", 600)]),
        ["$.offer_size", "$.offer_timer_ticks"]
    );
    let mut pack: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    pack["rules"]["offer_size"] = json!(4);
    assert!(matches!(
        Pack::from_bytes(pack.to_string().as_bytes()),
        Err(PackError::File { file: "rules", .. })
    ));
}

#[test]
fn the_offer_keys_are_optional_and_their_defaults_are_accepted() {
    let mut v = bundled_value();
    v["offer_size"] = json!(3);
    v["offer_timer_ticks"] = json!(300);
    Rules::parse(v.to_string().as_bytes(), Versions::CURRENT).expect("the defaults load");
    let object = v.as_object_mut().unwrap();
    object.remove("offer_size");
    object.remove("offer_timer_ticks");
    let r = Rules::parse(v.to_string().as_bytes(), Versions::CURRENT).expect("absent loads");
    assert_eq!((r.offer_size(), r.offer_timer_ticks()), (3, 300));
}

#[test]
fn version_1_still_reads_the_offer_keys() {
    let r = Rules::parse(
        &with_all(&[("offer_size", 4), ("offer_timer_ticks", 600)]),
        V1,
    )
    .unwrap();
    assert_eq!((r.offer_size(), r.offer_timer_ticks()), (4, 600));
}

#[test]
fn the_bundled_file_leaves_out_the_offer_keys() {
    let v = bundled_value();
    assert!(v.get("offer_size").is_none() && v.get("offer_timer_ticks").is_none());
    let r = Rules::bundled();
    assert_eq!((r.offer_size(), r.offer_timer_ticks()), (3, 300));
}

// --- Flinch, control repeats and vulnerable after a stun (dormant keys) --------------------------

/// The eight keys these rules add, each named by one value in range.
fn poise_keys() -> Value {
    json!({
        "poise_damage_a": 5,
        "poise_damage_b": 50,
        "flinch_ticks": 20,
        "flinch_immune_ticks": 40,
        "control_repeat_pcts": [100, 50, 25],
        "control_reset_ticks": 300,
        "control_immune_ticks": 120,
        "vulnerable_pcts": { "stun": 20 },
    })
}

/// The bundled file with `patch` merged over it, as bytes.
fn patched(patch: &Value) -> Vec<u8> {
    let mut v = bundled_value();
    for (key, value) in patch.as_object().unwrap() {
        v[key] = value.clone();
    }
    v.to_string().into_bytes()
}

/// The bundled file with `key` set to `value` after every key of `poise_keys` is named.
fn with_key(key: &str, value: Value) -> Vec<u8> {
    let mut all = poise_keys();
    all[key] = value;
    patched(&all)
}

/// The bundled file with only `keys` of `poise_keys`, then `key` set to `value`.
fn with_only(keys: &[&str], key: &str, value: Value) -> Vec<u8> {
    let all = poise_keys();
    let mut chosen = serde_json::Map::new();
    for k in keys {
        chosen.insert((*k).into(), all[*k].clone());
    }
    chosen.insert(key.into(), value);
    patched(&Value::Object(chosen))
}

#[test]
fn c1_the_bundled_file_names_none_of_the_poise_keys() {
    let v = bundled_value();
    for key in poise_keys().as_object().unwrap().keys() {
        assert!(v.get(key).is_none(), "{key}");
    }
}

#[test]
fn c2_every_bound_is_checked_at_its_key() {
    // (key, in range, first values out of range, the path of the issue)
    let cases: [(&str, Value, Vec<Value>, &str); 9] = [
        (
            "flinch_ticks",
            json!(60),
            vec![json!(0), json!(61)],
            "$.flinch_ticks",
        ),
        (
            "flinch_immune_ticks",
            json!(120),
            vec![json!(121)],
            "$.flinch_immune_ticks",
        ),
        (
            "poise_damage_a",
            json!(1000),
            vec![json!(1001)],
            "$.poise_damage_a",
        ),
        (
            "poise_damage_b",
            json!(1000),
            vec![json!(1001)],
            "$.poise_damage_b",
        ),
        (
            "control_repeat_pcts",
            json!([100, 100, 100, 100, 100, 100, 100, 100]),
            vec![
                json!([]),
                json!([100, 100, 100, 100, 100, 100, 100, 100, 100]),
            ],
            "$.control_repeat_pcts",
        ),
        (
            "control_reset_ticks",
            json!(600),
            vec![json!(0), json!(601)],
            "$.control_reset_ticks",
        ),
        (
            "control_immune_ticks",
            json!(600),
            vec![json!(0), json!(601)],
            "$.control_immune_ticks",
        ),
        (
            "vulnerable_pcts",
            json!({ "stun": 1000 }),
            vec![json!({ "stun": 1001 })],
            "$.vulnerable_pcts.stun",
        ),
        (
            "vulnerable_pcts",
            json!({ "stun": 1 }),
            vec![json!({ "stun": 0 })],
            "$.vulnerable_pcts.stun",
        ),
    ];
    for (key, ok, bad, path) in cases {
        assert!(
            Rules::parse(&with_key(key, ok.clone()), Versions::CURRENT).is_ok(),
            "{key} {ok}"
        );
        for b in bad {
            assert_eq!(
                paths(Rules::parse(&with_key(key, b.clone()), Versions::CURRENT)),
                [path],
                "{key} {b}"
            );
        }
    }
    for (value, path) in [
        (json!([100, 0]), "$.control_repeat_pcts[1]"),
        (json!([100, 101]), "$.control_repeat_pcts[1]"),
    ] {
        assert_eq!(
            paths(Rules::parse(
                &with_key("control_repeat_pcts", value),
                Versions::CURRENT
            )),
            [path]
        );
    }
}

#[test]
fn c3_a_key_without_its_partner_is_refused() {
    let cur = Versions::CURRENT;
    // The immunity needs the flinch.
    assert_eq!(
        paths(Rules::parse(
            &patched(&json!({ "flinch_immune_ticks": 40 })),
            cur
        )),
        ["$.flinch_immune_ticks"]
    );
    // The reset and the immunity need the list, and the list needs both.
    assert_eq!(
        paths(Rules::parse(
            &patched(&json!({ "control_reset_ticks": 300 })),
            cur
        )),
        ["$.control_reset_ticks"]
    );
    assert_eq!(
        paths(Rules::parse(
            &patched(&json!({ "control_immune_ticks": 120 })),
            cur
        )),
        ["$.control_immune_ticks"]
    );
    for named in [
        &["control_reset_ticks"][..],
        &["control_immune_ticks"][..],
        &[][..],
    ] {
        let bytes = with_only(named, "control_repeat_pcts", json!([100]));
        assert_eq!(
            paths(Rules::parse(&bytes, cur)),
            ["$.control_repeat_pcts"],
            "{named:?}"
        );
    }
    // The partners together load.
    assert!(Rules::parse(&patched(&poise_keys()), cur).is_ok());
}

#[test]
fn c4_a_pack_at_sim_version_1_refuses_every_key() {
    for (key, value) in poise_keys().as_object().unwrap() {
        let bytes = patched(&json!({ key: value }));
        let Err(Error::Invalid(issues)) = Rules::parse(&bytes, V1) else {
            panic!("{key} not refused at sim_version 1");
        };
        let at = format!("$.{key}");
        let hit = issues.iter().find(|i| i.path.starts_with(&at));
        assert!(
            hit.is_some_and(|i| i.message.contains("needs sim_version 2")),
            "{key}: {issues:?}"
        );
    }
}

#[test]
fn c5_a_file_naming_every_key_is_within_the_size_limit_and_parses() {
    let bytes = patched(&poise_keys());
    assert!(bytes.len() <= MAX_RULES_BYTES, "{}", bytes.len());
    Rules::parse(&bytes, Versions::CURRENT).expect("parses");
}
// --- The stagger gauge's keys (dormant until content names them) --------------------------------

/// The keys the stagger gauge adds to the rules file, each named by one value in range.
fn stagger_keys() -> Value {
    json!({
        "staggered_ticks": 20,
        "stagger_smash": 40,
        "stagger_values": { "stun": 30, "pull": 20, "knockback": 10 },
        "stagger_refill_pcts": [100, 50],
        "stagger_drain_delay_ticks": 10,
        "stagger_drain_per_tick": 5,
        "stagger_damage_pct": 50,
        "vulnerable_pcts": { "stagger": 20 },
    })
}

#[test]
fn r1_the_bundled_file_names_none_of_the_stagger_keys() {
    let v = bundled_value();
    for key in stagger_keys().as_object().unwrap().keys() {
        assert!(v.get(key).is_none(), "{key}");
    }
    let rules = Rules::bundled();
    assert_eq!(rules.staggered_ticks(), 0);
    assert_eq!(rules.stagger_smash(), 0);
    assert_eq!(rules.stagger_values(), [0, 0, 0]);
    assert!(rules.stagger_refill_pcts().is_empty());
    assert_eq!(rules.stagger_drain(), (0, 0));
    assert_eq!(rules.stagger_damage_pct(), 0);
    assert_eq!(rules.vulnerable_stagger_pct(), 0);
}

/// The bundled file with `key` set to `value` after every stagger key is named.
fn stagger_with(key: &str, value: Value) -> Vec<u8> {
    let mut all = stagger_keys();
    all[key] = value;
    patched(&all)
}

#[test]
fn r2_every_stagger_bound_is_checked_at_its_key() {
    // (key, in range, first values out of range, the path of the issue)
    let cases: Vec<(&str, Value, Vec<Value>, &str)> = vec![
        (
            "staggered_ticks",
            json!(600),
            vec![json!(0), json!(601)],
            "$.staggered_ticks",
        ),
        (
            "stagger_smash",
            json!(10_000),
            vec![json!(10_001)],
            "$.stagger_smash",
        ),
        (
            "stagger_values",
            json!({ "pull": 10_000 }),
            vec![json!({ "pull": 0 }), json!({ "pull": 10_001 })],
            "$.stagger_values.pull",
        ),
        (
            "stagger_values",
            json!({ "stun": 1 }),
            vec![json!({ "stun": 0 })],
            "$.stagger_values.stun",
        ),
        (
            "stagger_values",
            json!({ "knockback": 1 }),
            vec![json!({ "knockback": 10_001 })],
            "$.stagger_values.knockback",
        ),
        (
            "stagger_refill_pcts",
            json!([100, 100, 100, 100, 100, 100, 100, 100]),
            vec![
                json!([]),
                json!([100, 100, 100, 100, 100, 100, 100, 100, 100]),
            ],
            "$.stagger_refill_pcts",
        ),
        (
            "stagger_drain_delay_ticks",
            json!(600),
            vec![json!(0), json!(601)],
            "$.stagger_drain_delay_ticks",
        ),
        (
            "stagger_drain_per_tick",
            json!(10_000),
            vec![json!(0), json!(10_001)],
            "$.stagger_drain_per_tick",
        ),
        (
            "stagger_damage_pct",
            json!(1000),
            vec![json!(0), json!(1001)],
            "$.stagger_damage_pct",
        ),
        (
            "vulnerable_pcts",
            json!({ "stagger": 1000 }),
            vec![json!({ "stagger": 0 }), json!({ "stagger": 1001 })],
            "$.vulnerable_pcts.stagger",
        ),
    ];
    for (key, ok, bad, path) in cases {
        assert!(
            Rules::parse(&stagger_with(key, ok.clone()), Versions::CURRENT).is_ok(),
            "{key} {ok}"
        );
        for b in bad {
            assert_eq!(
                paths(Rules::parse(
                    &stagger_with(key, b.clone()),
                    Versions::CURRENT
                )),
                [path],
                "{key} {b}"
            );
        }
    }
    for (value, path) in [
        (json!([100, 0]), "$.stagger_refill_pcts[1]"),
        (json!([100, 101]), "$.stagger_refill_pcts[1]"),
    ] {
        assert_eq!(
            paths(Rules::parse(
                &stagger_with("stagger_refill_pcts", value),
                Versions::CURRENT
            )),
            [path]
        );
    }
}

#[test]
fn r3_one_drain_key_alone_is_refused_at_that_key() {
    let cur = Versions::CURRENT;
    for key in ["stagger_drain_delay_ticks", "stagger_drain_per_tick"] {
        let one = patched(&json!({ key: stagger_keys()[key].clone() }));
        assert_eq!(
            paths(Rules::parse(&one, cur)),
            [format!("$.{key}")],
            "{key}"
        );
    }
    let both = patched(&json!({
        "stagger_drain_delay_ticks": 10,
        "stagger_drain_per_tick": 5,
    }));
    assert!(Rules::parse(&both, cur).is_ok());
}

#[test]
fn r4_a_pack_at_sim_version_1_refuses_every_stagger_key() {
    for (key, value) in stagger_keys().as_object().unwrap() {
        let bytes = patched(&json!({ key: value }));
        let Err(Error::Invalid(issues)) = Rules::parse(&bytes, V1) else {
            panic!("{key} not refused at sim_version 1");
        };
        let at = format!("$.{key}");
        let hit = issues.iter().find(|i| i.path.starts_with(&at));
        assert!(
            hit.is_some_and(|i| i.message.contains("needs sim_version 2")),
            "{key}: {issues:?}"
        );
    }
}

#[test]
fn r5_a_file_naming_every_stagger_key_is_within_the_size_limit_and_parses() {
    let mut all = poise_keys();
    for (k, v) in stagger_keys().as_object().unwrap() {
        if k == "vulnerable_pcts" {
            all["vulnerable_pcts"]["stagger"] = v["stagger"].clone();
        } else {
            all[k] = v.clone();
        }
    }
    let bytes = patched(&all);
    assert!(bytes.len() <= MAX_RULES_BYTES, "{}", bytes.len());
    Rules::parse(&bytes, Versions::CURRENT).expect("parses");
}

// --- The perfect evade's five keys (ADR-CONT-3, ADR-SVC-1) ---------------------------------------

const EVADE_KEYS: [&str; 5] = [
    "perfect_evade_window_ticks",
    "perfect_evade_slow_ticks",
    "perfect_evade_cooldown_ticks",
    "perfect_evade_reach",
    "perfect_evade_slow_every",
];

/// The five keys at `[window, slow, cooldown, reach, every]` set on the bundled file.
fn evade_with(values: [i64; 5]) -> Vec<u8> {
    let fields: Vec<(&str, i64)> = EVADE_KEYS.into_iter().zip(values).collect();
    with_all(&fields)
}

const EVADE_OK: [i64; 5] = [10, 60, 300, 200 * 256, 2];

#[test]
fn the_five_perfect_evade_keys_load_at_their_bounds() {
    let px = |n: i64| n * 256;
    for values in [
        EVADE_OK,
        [1, 1, 1, px(1), 2],
        [30, 300, 3600, px(420), 8],
        [10, 60, 60, px(200), 2],
    ] {
        Rules::parse(&evade_with(values), Versions::CURRENT)
            .unwrap_or_else(|e| panic!("{values:?} refused: {e:?}"));
    }
}

#[test]
fn each_perfect_evade_key_is_refused_one_past_its_bound_at_its_own_path() {
    let px = |n: i64| n * 256;
    let cases: [(usize, i64); 10] = [
        (0, 0),
        (0, 31),
        (1, 0),
        (1, 301),
        (2, 3601),
        (3, 0),
        (3, px(420) + 1),
        (4, 1),
        (4, 9),
        (2, 59), // below the slow of 60
    ];
    for (key, value) in cases {
        let mut values = EVADE_OK;
        values[key] = value;
        assert_eq!(
            paths(Rules::parse(&evade_with(values), Versions::CURRENT)),
            [format!("$.{}", EVADE_KEYS[key])],
            "{} = {value}",
            EVADE_KEYS[key]
        );
    }
}

#[test]
fn some_but_not_all_perfect_evade_keys_is_refused_naming_each_missing_key() {
    for present in 1..5 {
        let mut v: Value = serde_json::from_slice(&evade_with(EVADE_OK)).unwrap();
        for key in &EVADE_KEYS[present..] {
            v.as_object_mut().unwrap().remove(*key);
        }
        let want: Vec<String> = EVADE_KEYS[present..]
            .iter()
            .map(|k| format!("$.{k}"))
            .collect();
        assert_eq!(
            paths(Rules::parse(v.to_string().as_bytes(), Versions::CURRENT)),
            want,
            "{present} present"
        );
    }
}

#[test]
fn the_perfect_evade_keys_need_sim_version_2() {
    let v1 = Versions {
        sim_version: 1,
        ..Versions::CURRENT
    };
    for key in EVADE_KEYS {
        let got = paths(Rules::parse(&evade_with(EVADE_OK), v1));
        assert!(got.contains(&format!("$.{key}")), "{key}: {got:?}");
    }
}

#[test]
fn the_bundled_file_names_no_perfect_evade_key() {
    let v: Value = serde_json::from_slice(common::bundled_file("rules")).unwrap();
    for key in EVADE_KEYS {
        assert!(v.get(key).is_none(), "{key}");
    }
    assert!(Rules::bundled().perfect_evade().is_none());
}

#[test]
fn a_loaded_perfect_evade_reads_back_clamped_to_the_caps() {
    let rules = Rules::parse(&evade_with(EVADE_OK), Versions::CURRENT).unwrap();
    let got = rules.perfect_evade().expect("all five keys");
    assert_eq!(
        (got.window_ticks, got.slow_ticks, got.cooldown_ticks),
        (10, 60, 300)
    );
    assert_eq!((got.reach, got.slow_every), (200 * 256, 2));
}

#[test]
fn the_perfect_evade_keys_change_the_derived_content_hash() {
    let base = RunContent::bundled().clone();
    let changed = run_content_with(
        Rules::parse(&evade_with(EVADE_OK), Versions::CURRENT).expect("the keys load"),
    );
    for mode in ["trial", "time_limit", "tutorial"] {
        assert_ne!(
            content_sha256_with(mode, &base),
            content_sha256_with(mode, &changed),
            "{mode}"
        );
    }
}
