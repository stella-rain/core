//! The validator (ADR-020): every budget is tested at its limit and one past it, registry IDs
//! must exist for the stage's `sim_version`, and errors say where without repeating the file.

use serde_json::{Value, json};
use stella_rain_core::registry::{self, Availability, Entry};
use stella_rain_core::rng::SplitMix64;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{
    Error, Issue, limits::*, parse_and_validate, validate, validate_with,
};

const STAGE: &str = include_str!("fixtures/stage_v0.json");

fn base() -> Value {
    serde_json::from_str(STAGE).unwrap()
}

fn set(v: &mut Value, pointer: &str, new: Value) {
    *v.pointer_mut(pointer)
        .unwrap_or_else(|| panic!("{pointer} exists in the fixture")) = new;
}

/// Adds `key` to the object at `pointer`.
fn set_new(v: &mut Value, pointer: &str, key: &str, new: Value) {
    v.pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("{pointer} is an object in the fixture"))
        .insert(key.to_owned(), new);
}

fn stage(v: &Value) -> Stage {
    serde_json::from_value(v.clone()).expect("the test stage parses")
}

fn issues(v: &Value) -> Vec<Issue> {
    match validate(&stage(v)) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues,
        Err(other) => panic!("unexpected {other:?}"),
    }
}

fn assert_valid(v: &Value) {
    let found = issues(v);
    assert!(found.is_empty(), "expected a valid stage, found {found:?}");
}

/// The first problem found; panics if the stage is valid.
fn first_issue(v: &Value) -> Issue {
    issues(v)
        .into_iter()
        .next()
        .expect("expected the stage to be rejected")
}

/// `build(max)` is valid and `build(max + 1)` is rejected at `path`.
fn at_limit_and_over(max: usize, path: &str, build: impl Fn(usize) -> Value) {
    assert_valid(&build(max));
    let issue = first_issue(&build(max + 1));
    assert_eq!(issue.path, path, "{issue:?}");
}

fn repeated(v: &Value, pointer: &str, n: usize) -> Value {
    Value::Array(vec![v.pointer(pointer).unwrap().clone(); n])
}

/// The attack of the fixture's wave enemy, where the attack tests put theirs.
fn enemy_attack(v: &mut Value) -> &mut Value {
    &mut v["waves"][0]["enemy"]["rules"][0]["do"]["attack"]
}

/// An attack of one active phase that spawns `emitters`.
fn spawning(emitters: Value) -> Value {
    json!({ "inline": [{ "kind": "active", "ticks": 1, "spawn": emitters }] })
}

fn fire(speed: Value) -> Value {
    json!({ "type": "fire", "speed": speed, "direction": { "type": "absolute", "angle": 0 } })
}

/// An expression with exactly `n` nodes (n >= 1, n != 2).
fn expr_with_nodes(n: u32) -> Value {
    fn chain(leaves: u32, last: Value) -> Value {
        if leaves == 1 {
            last
        } else {
            json!({ "add": [1, chain(leaves - 1, last)] })
        }
    }
    if n % 2 == 1 {
        chain(n.div_ceil(2), json!(1))
    } else {
        chain((n - 2) / 2, json!({ "clamp": [1, 1, 1] }))
    }
}

#[test]
fn the_fixture_is_valid() {
    let stage = parse_and_validate(STAGE.as_bytes()).expect("valid");
    assert_eq!(stage.id, "placeholder-golem");
}

#[test]
fn the_byte_limit_applies_before_parsing() {
    let pad = |total: usize| {
        let mut bytes = STAGE.as_bytes().to_vec();
        bytes.resize(total, b' ');
        bytes
    };
    assert!(parse_and_validate(&pad(MAX_STAGE_BYTES)).is_ok());
    assert_eq!(
        parse_and_validate(&pad(MAX_STAGE_BYTES + 1)),
        Err(Error::TooLarge {
            size: MAX_STAGE_BYTES + 1,
            limit: MAX_STAGE_BYTES
        })
    );
    // Not even JSON, but too big: reported as too big, never parsed.
    assert!(matches!(
        parse_and_validate(&vec![b'x'; MAX_STAGE_BYTES + 1]),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn newer_versions_are_reported_before_the_strict_parse() {
    for (field, name) in [("schema_version", "schema"), ("sim_version", "sim")] {
        let mut v = base();
        v[field] = json!(1);
        // A newer game may have fields this one does not know.
        v["field_from_the_future"] = json!(true);
        let err = parse_and_validate(v.to_string().as_bytes()).unwrap_err();
        assert!(
            matches!(err, Error::UnsupportedVersion { .. }),
            "{name}: {err:?}"
        );
        assert!(err.to_string().contains("newer version of the game"));
    }
    // The same stage at version 0 with an unknown field is a parse error.
    let mut v = base();
    v["field_from_the_future"] = json!(true);
    assert!(matches!(
        parse_and_validate(v.to_string().as_bytes()),
        Err(Error::Parse { .. })
    ));
}

#[test]
fn parse_errors_say_where_and_never_repeat_the_file() {
    let mut v = base();
    v["ZZZ_FIELD_NAME"] = json!(1);
    let err = parse_and_validate(v.to_string().as_bytes()).unwrap_err();
    assert!(matches!(err, Error::Parse { .. }));
    assert!(!err.to_string().contains("ZZZ_FIELD_NAME"), "{err}");
    assert!(matches!(
        parse_and_validate(b"{ not json"),
        Err(Error::Parse { line: 1, .. })
    ));
    assert!(matches!(parse_and_validate(b""), Err(Error::Parse { .. })));
}

#[test]
fn deeply_nested_json_is_refused_not_overflowed() {
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(matches!(
        parse_and_validate(deep.as_bytes()),
        Err(Error::Parse { .. })
    ));
}

#[test]
fn stage_budgets_hold_at_the_limit_and_fail_one_past() {
    let b = base();
    at_limit_and_over(MAX_WAVES, "$.waves", |n| {
        let mut v = b.clone();
        v["waves"] = repeated(&b, "/waves/0", n);
        v
    });
    at_limit_and_over(MAX_COMPANIONS, "$.companions", |n| {
        let mut v = b.clone();
        v["companions"] = repeated(&b, "/companions/0", n);
        v
    });
    at_limit_and_over(MAX_PHASES, "$.boss.phases", |n| {
        let mut v = b.clone();
        let mut phases = vec![b.pointer("/boss/phases/0").unwrap().clone(); n - 1];
        phases.push(b.pointer("/boss/phases/1").unwrap().clone());
        v["boss"]["phases"] = Value::Array(phases);
        v
    });
    at_limit_and_over(MAX_RULES, "$.companions[0].rules", |n| {
        let mut v = b.clone();
        v["companions"][0]["rules"] = repeated(&b, "/companions/0/rules/0", n);
        v
    });
    at_limit_and_over(MAX_RULES, "$.boss.phases[0].rules", |n| {
        let mut v = b.clone();
        v["boss"]["phases"][0]["rules"] = repeated(&b, "/boss/phases/0/rules/0", n);
        v
    });
    at_limit_and_over(MAX_TIMELINE_STEPS, "$.boss.phases[0].timeline", |n| {
        let mut v = b.clone();
        v["boss"]["phases"][0]["timeline"] =
            Value::Array(vec![json!({ "type": "wait", "ticks": 1 }); n]);
        v
    });
    at_limit_and_over(MAX_BOSS_PARTS, "$.boss.parts", |n| {
        let mut v = b.clone();
        let parts: Vec<Value> = (0..n)
            .map(|i| {
                let mut p = b.pointer("/boss/parts/0").unwrap().clone();
                if i > 0 {
                    p["id"] = json!(format!("part_{i}"));
                }
                p
            })
            .collect();
        // The first part keeps the ID the fixture's rules name.
        v["boss"]["parts"] = Value::Array(parts);
        v
    });
}

#[test]
fn pattern_depth_and_node_budgets() {
    let b = base();
    // `levels` rotates wrapped round one fire: the fire sits at depth levels + 1.
    let nested = |levels: usize| {
        let mut node = fire(json!(256));
        for _ in 0..levels {
            node = json!({ "type": "rotate", "angle": 0, "body": [node] });
        }
        let mut v = b.clone();
        *enemy_attack(&mut v) = spawning(json!([node]));
        v
    };
    assert_valid(&nested(MAX_PATTERN_DEPTH as usize - 1));
    let issue = first_issue(&nested(MAX_PATTERN_DEPTH as usize));
    assert!(issue.message.contains("levels deep"), "{issue:?}");
    assert!(
        issue
            .path
            .starts_with("$.waves[0].enemy.rules[0].do.attack.inline[0].spawn[0]"),
        "{issue:?}"
    );

    let waits = |n: usize| {
        let mut v = b.clone();
        *enemy_attack(&mut v) =
            spawning(Value::Array(vec![json!({ "type": "wait", "ticks": 1 }); n]));
        v
    };
    assert_valid(&waits(MAX_PATTERN_NODES as usize));
    let issue = first_issue(&waits(MAX_PATTERN_NODES as usize + 1));
    assert_eq!(
        issue.path,
        "$.waves[0].enemy.rules[0].do.attack.inline[0].spawn[64]"
    );
    assert!(issue.message.contains("64 nodes"), "{issue:?}");

    // Nodes are counted across the whole tree, branches included.
    let branches = |n: usize| {
        let leaf = |_: usize| json!({ "type": "rotate", "angle": 0, "body": [fire(json!(1))] });
        let mut v = b.clone();
        *enemy_attack(&mut v) = spawning(Value::Array((0..n).map(leaf).collect()));
        v
    };
    assert_valid(&branches(32));
    assert!(first_issue(&branches(33)).message.contains("64 nodes"));
}

#[test]
fn expressions_have_at_most_sixteen_nodes() {
    let b = base();
    let with = |n: u32| {
        let mut v = b.clone();
        *enemy_attack(&mut v) = spawning(json!([fire(expr_with_nodes(n))]));
        v
    };
    for n in [1, 3, 15, MAX_EXPR_NODES] {
        assert_valid(&with(n));
    }
    for n in [MAX_EXPR_NODES + 1, MAX_EXPR_NODES + 2, 200] {
        let found = issues(&with(n));
        assert_eq!(found.len(), 1, "n = {n}: {found:?}");
        assert!(found[0].message.contains("16 nodes"), "{found:?}");
    }
}

#[test]
fn constant_repeat_counts_are_at_most_one_hundred() {
    let b = base();
    for kind in ["repeat", "ring", "spread"] {
        let with = |count: Value| {
            let mut node = json!({ "type": kind, "count": count, "body": [] });
            match kind {
                "repeat" => node["interval_ticks"] = json!(1),
                "spread" => node["angle"] = json!(0),
                _ => {}
            }
            let mut v = b.clone();
            *enemy_attack(&mut v) = spawning(json!([node]));
            v
        };
        assert_valid(&with(json!(MAX_REPEAT)));
        assert_valid(&with(json!(1)));
        for bad in [0, MAX_REPEAT + 1, -1, i32::MIN] {
            let issue = first_issue(&with(json!(bad)));
            assert!(issue.path.contains(".count"), "{kind} {bad}: {issue:?}");
        }
        // Computed counts are the engine's to clamp.
        assert_valid(&with(json!({ "add": [1, 5000] })));
    }
}

#[test]
fn text_lengths_count_characters_not_bytes() {
    let b = base();
    let title = |n: usize| {
        let mut v = b.clone();
        v["title"] = json!("é".repeat(n));
        v
    };
    assert_valid(&title(MAX_TITLE_CHARS));
    assert_eq!(first_issue(&title(MAX_TITLE_CHARS + 1)).path, "$.title");
    assert_eq!(first_issue(&title(0)).path, "$.title");
    let description = |n: usize| {
        let mut v = b.clone();
        v["description"] = json!("日".repeat(n));
        v
    };
    assert_valid(&description(MAX_DESCRIPTION_CHARS));
    assert_valid(&description(0));
    assert_eq!(
        first_issue(&description(MAX_DESCRIPTION_CHARS + 1)).path,
        "$.description"
    );
}

#[test]
fn stage_ids_are_slugs() {
    let b = base();
    let with = |id: &str| {
        let mut v = b.clone();
        v["id"] = json!(id);
        v
    };
    for ok in ["a", "frost-cavern", "0-9", &"a".repeat(MAX_STAGE_ID_CHARS)] {
        assert_valid(&with(ok));
    }
    let too_long = "a".repeat(MAX_STAGE_ID_CHARS + 1);
    for bad in [
        "", "A", "a_b", "../x", "a/b", "a b", "-a", "a-", "é", "a.json", "a\u{0}", &too_long,
    ] {
        assert_eq!(first_issue(&with(bad)).path, "$.id", "{bad:?}");
    }
}

#[test]
fn ranges_keep_numbers_inside_the_loadout() {
    let b = base();
    let bad = |pointer: &str, value: Value| {
        let mut v = b.clone();
        set(&mut v, pointer, value);
        first_issue(&v).path
    };
    let ok = |pointer: &str, value: Value| {
        let mut v = b.clone();
        set(&mut v, pointer, value);
        assert_valid(&v);
    };
    ok("/player/hp", json!(MAX_PLAYER_HP));
    assert_eq!(bad("/player/hp", json!(0)), "$.player.hp");
    assert_eq!(bad("/player/hp", json!(MAX_PLAYER_HP + 1)), "$.player.hp");
    assert_eq!(bad("/player/atk", json!(0)), "$.player.atk");
    assert_eq!(bad("/boss/hp", json!(MAX_BOSS_HP + 1)), "$.boss.hp");
    assert_eq!(bad("/companions/0/hp", json!(0)), "$.companions[0].hp");
    assert_eq!(
        bad("/companions/0/radius", json!(0)),
        "$.companions[0].radius"
    );
    assert_eq!(
        bad("/companions/0/radius", json!(MAX_RADIUS + 1)),
        "$.companions[0].radius"
    );
    // Defence (ADR-056): 0 to MAX_DEFENCE on everything that can be hurt. The fixture has
    // none, so it is added to each owner.
    for (owner, path) in [
        ("/player", "$.player.defence"),
        ("/boss", "$.boss.defence"),
        ("/boss/parts/0", "$.boss.parts[0].defence"),
        ("/companions/0", "$.companions[0].defence"),
        ("/waves/0/enemy", "$.waves[0].enemy.defence"),
    ] {
        let with = |defence: u32| {
            let mut v = b.clone();
            v.pointer_mut(owner).unwrap()["defence"] = json!(defence);
            v
        };
        assert_valid(&with(0));
        assert_valid(&with(MAX_DEFENCE));
        assert_eq!(first_issue(&with(MAX_DEFENCE + 1)).path, path);
    }
    assert_eq!(bad("/length_ticks", json!(0)), "$.length_ticks");
    ok("/length_ticks", json!(MAX_TICKS));
    assert_eq!(bad("/length_ticks", json!(MAX_TICKS + 1)), "$.length_ticks");
    assert_eq!(bad("/waves/0/count", json!(0)), "$.waves[0].count");
    assert_eq!(
        bad("/waves/0/count", json!(MAX_WAVE_COUNT + 1)),
        "$.waves[0].count"
    );
    assert_eq!(
        bad("/waves/0/every_ticks", json!(0)),
        "$.waves[0].every_ticks"
    );
    assert_eq!(bad("/waves/0/at_tick", json!(18000)), "$.waves[0].at_tick");
    ok("/waves/0/at_tick", json!(17999));
    assert_eq!(bad("/waves/0/spawn/x", json!(-16385)), "$.waves[0].spawn.x");
    ok("/waves/0/spawn/x", json!(-16384));
    assert_eq!(
        bad("/waves/0/spawn/y", json!(163840 + 16385)),
        "$.waves[0].spawn.y"
    );
    // Without a map, a point beyond the portrait field is out; on a wide map it is in.
    assert_eq!(
        bad("/waves/0/spawn/x", json!(524_288)),
        "$.waves[0].spawn.x"
    );
    let mut wide = b.clone();
    wide["map"] = json!({ "width": 1_048_576, "height": 262_144 });
    set(&mut wide, "/waves/0/spawn/x", json!(1_048_576 + 16_384));
    assert_valid(&wide);
    set(&mut wide, "/waves/0/spawn/x", json!(1_048_576 + 16_385));
    assert_eq!(first_issue(&wide).path, "$.waves[0].spawn.x");
    assert_eq!(
        bad("/companions/0/movement/radius", json!(MAX_LEN + 1)),
        "$.companions[0].movement.radius"
    );
    assert_eq!(
        bad("/companions/0/rules/0/when/pct", json!(101)),
        "$.companions[0].rules[0].when.pct"
    );
    assert_eq!(
        bad("/companions/0/rules/0/cooldown_ticks", json!(MAX_TICKS + 1)),
        "$.companions[0].rules[0].cooldown_ticks"
    );
    assert_eq!(
        bad("/boss/phases/1/rules/1/charges", json!(0)),
        "$.boss.phases[1].rules[1].charges"
    );
    let shot = |damage: u32| {
        json!({ "type": "shot", "damage": damage,
                "pattern": { "preset": { "id": "spread_5", "args": [] } } })
    };
    ok("/companions/0/rules/0/do/skill", shot(MAX_DAMAGE));
    assert_eq!(
        bad("/companions/0/rules/0/do/skill", shot(MAX_DAMAGE + 1)),
        "$.companions[0].rules[0].do.skill.damage"
    );
    assert_eq!(
        bad(
            "/companions/0/rules/1/do/skill/value_pct",
            json!(MAX_BUFF_PCT + 1)
        ),
        "$.companions[0].rules[1].do.skill.value_pct"
    );
    assert_eq!(
        bad("/companions/0/rules/1/do/skill/duration_ticks", json!(0)),
        "$.companions[0].rules[1].do.skill.duration_ticks"
    );
}

#[test]
fn an_evade_is_bounded_and_required() {
    let b = base();
    let with = |key: &str, value: i64| {
        let mut v = b.clone();
        set(&mut v, &format!("/player/evade/{key}"), json!(value));
        v
    };
    for (key, lo, hi) in [
        ("distance", 0, i64::from(MAX_EVADE_DISTANCE)),
        ("ticks", 1, i64::from(MAX_EVADE_TICKS)),
        ("invulnerable_ticks", 0, i64::from(MAX_EVADE_TICKS)),
        ("cooldown_ticks", 0, i64::from(MAX_TICKS)),
    ] {
        let path = format!("$.player.evade.{key}");
        assert_valid(&with(key, lo));
        assert_valid(&with(key, hi));
        assert_eq!(first_issue(&with(key, hi + 1)).path, path);
        if key == "distance" || key == "ticks" {
            assert_eq!(first_issue(&with(key, lo - 1)).path, path);
        }
    }
    let mut none = b.clone();
    none["player"].as_object_mut().unwrap().remove("evade");
    assert!(matches!(
        parse_and_validate(none.to_string().as_bytes()),
        Err(Error::Parse { .. })
    ));
}

#[test]
fn evade_charges_are_one_to_three_and_the_most_is_not_below_the_start() {
    let with = |charges: Option<u8>, max: Option<u8>| {
        let mut v = base();
        if let Some(n) = charges {
            set_new(&mut v, "/player/evade", "charges", json!(n));
        }
        if let Some(n) = max {
            set_new(&mut v, "/player/evade", "max_charges", json!(n));
        }
        v
    };
    assert_valid(&with(None, None));
    assert_valid(&with(Some(1), None));
    assert_valid(&with(Some(MAX_EVADE_CHARGES), None));
    assert_valid(&with(Some(1), Some(MAX_EVADE_CHARGES)));
    assert_valid(&with(None, Some(2)));
    let path = |v: &Value| first_issue(v).path;
    assert_eq!(path(&with(Some(0), None)), "$.player.evade.charges");
    assert_eq!(
        path(&with(Some(MAX_EVADE_CHARGES + 1), None)),
        "$.player.evade.charges"
    );
    assert_eq!(
        path(&with(Some(1), Some(MAX_EVADE_CHARGES + 1))),
        "$.player.evade.max_charges"
    );
    assert_eq!(path(&with(Some(2), Some(1))), "$.player.evade.max_charges");
}

#[test]
fn registry_ids_must_exist_with_the_right_kind() {
    let b = base();
    let bad = |pointer: &str, id: &str| {
        let mut v = b.clone();
        set(&mut v, pointer, json!(id));
        first_issue(&v)
    };
    // Well formed but not in the registry.
    let issue = bad("/player/base", "zzz_secret");
    assert_eq!(issue.path, "$.player.base");
    assert!(!format!("{issue:?}").contains("zzz_secret"));
    // Exists, but as another kind.
    assert_eq!(bad("/player/base", "ice").path, "$.player.base");
    assert_eq!(
        bad("/waves/0/enemy/palette", "bat").path,
        "$.waves[0].enemy.palette"
    );
    assert_eq!(
        bad("/boss/parts/0/asset", "vulnerable").path,
        "$.boss.parts[0].asset"
    );
    // Not a valid ID at all.
    for malformed in ["", "Bat", "bat-2", "has space", "a".repeat(33).as_str()] {
        let issue = bad("/boss/base", malformed);
        assert_eq!(issue.path, "$.boss.base", "{malformed:?}");
        assert!(issue.message.contains("not a valid content ID"));
    }
    assert_eq!(
        bad("/boss/phases/1/rules/2/when/status", "no_such_status").path,
        "$.boss.phases[1].rules[2].when.status"
    );
    assert_eq!(
        bad(
            "/boss/phases/1/timeline/1/attack/inline/0/spawn/0/then/0/body/0/bullet",
            "orb_huge"
        )
        .path,
        "$.boss.phases[1].timeline[1].attack.inline[0].spawn[0].then[0].body[0].bullet"
    );
}

#[test]
fn ids_newer_than_the_stage_are_rejected_and_deprecated_ones_load() {
    let b = base();
    let with = |change: fn(&mut Entry)| -> Vec<Entry> {
        let mut entries = registry::ENTRIES.to_vec();
        entries
            .iter_mut()
            .filter(|e| e.id == "golem")
            .for_each(change);
        entries
    };
    let parsed = stage(&b);
    let newer = with(|e| e.introduced = 1);
    match validate_with(&parsed, &newer) {
        Err(Error::Invalid(found)) => {
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].path, "$.boss.base");
            assert!(found[0].message.contains("newer than this stage"));
        }
        other => panic!("{other:?}"),
    }
    let deprecated = with(|e| e.availability = Availability::Deprecated);
    assert_eq!(validate_with(&parsed, &deprecated), Ok(()));
}

#[test]
fn presets_take_the_arguments_the_registry_says() {
    let b = base();
    let with = |id: &str, args: Value| {
        let mut v = b.clone();
        *enemy_attack(&mut v) = json!({ "preset": { "id": id, "args": args } });
        v
    };
    assert_valid(&with("twin_shot", json!([])));
    assert_valid(&with("aimed_single", json!([512])));
    assert_eq!(
        first_issue(&with("aimed_single", json!([]))).path,
        "$.waves[0].enemy.rules[0].do.attack.preset.args"
    );
    assert_eq!(
        first_issue(&with("twin_shot", json!([1]))).path,
        "$.waves[0].enemy.rules[0].do.attack.preset.args"
    );
    assert_eq!(
        first_issue(&with("aimed_single", json!([MAX_NUMBER + 1]))).path,
        "$.waves[0].enemy.rules[0].do.attack.preset.args[0].value"
    );
    let many = Value::Array(vec![json!(0); MAX_PRESET_ARGS + 1]);
    assert_eq!(
        first_issue(&with("twin_shot", many)).path,
        "$.waves[0].enemy.rules[0].do.attack.preset.args"
    );
    assert_eq!(
        first_issue(&with("not_a_preset", json!([]))).path,
        "$.waves[0].enemy.rules[0].do.attack.preset.id"
    );
}

#[test]
fn inline_attacks_take_no_arguments() {
    let mut v = base();
    *enemy_attack(&mut v) = spawning(json!([fire(json!({ "arg": 0 }))]));
    let issue = first_issue(&v);
    assert_eq!(
        issue.path,
        "$.waves[0].enemy.rules[0].do.attack.inline[0].spawn[0].speed"
    );
    assert!(issue.message.contains("no such argument"));
}

/// One phase of an inline attack that spawns one bullet, with `extra` fields set on it.
fn phase(extra: Value) -> Value {
    let mut p = json!({ "kind": "active", "ticks": 1, "spawn": [fire(json!(256))] });
    for (k, v) in extra.as_object().expect("an object") {
        p[k] = v.clone();
    }
    p
}

fn with_phases(phases: Value) -> Value {
    let mut v = base();
    *enemy_attack(&mut v) = json!({ "inline": phases });
    v
}

#[test]
fn an_inline_attack_is_a_list_of_phases() {
    assert_valid(&with_phases(json!([phase(json!({}))])));
    assert_valid(&with_phases(json!([phase(json!({ "move_pct": 100 }))])));
    // A list of emitter nodes is no longer an attack (ADR-055).
    let mut nodes = base();
    *enemy_attack(&mut nodes) = json!({ "inline": [fire(json!(256))] });
    assert!(serde_json::from_value::<Stage>(nodes).is_err());

    let issue = first_issue(&with_phases(json!([])));
    assert_eq!(issue.path, "$.waves[0].enemy.rules[0].do.attack.inline");
    assert!(issue.message.contains("at least one phase"), "{issue:?}");
}

#[test]
fn an_attack_has_at_most_max_attack_phases() {
    at_limit_and_over(
        MAX_ATTACK_PHASES,
        "$.waves[0].enemy.rules[0].do.attack.inline",
        |n| with_phases(Value::Array(vec![phase(json!({})); n])),
    );
}

#[test]
fn a_phase_lasts_1_to_max_phase_ticks_and_moves_0_to_100_percent() {
    let at = |key: &str| format!("$.waves[0].enemy.rules[0].do.attack.inline[0].{key}");
    for kind in ["wind_up", "active", "recovery"] {
        assert_valid(&with_phases(json!([phase(json!({ "kind": kind }))])));
    }
    assert_valid(&with_phases(json!([phase(
        json!({ "ticks": MAX_PHASE_TICKS })
    )])));
    assert_valid(&with_phases(json!([phase(json!({ "move_pct": 0 }))])));
    for (extra, key) in [
        (json!({ "ticks": 0 }), "ticks"),
        (json!({ "ticks": MAX_PHASE_TICKS + 1 }), "ticks"),
        (json!({ "move_pct": 101 }), "move_pct"),
    ] {
        assert_eq!(
            first_issue(&with_phases(json!([phase(extra)]))).path,
            at(key)
        );
    }
}

/// Hit boxes, their damage and re-hits are bounded (ADR-055), at the limit and one past it.
#[test]
fn hit_boxes_damage_and_rehits_are_bounded() {
    let at = |key: &str| format!("$.waves[0].enemy.rules[0].do.attack.inline[0].{key}");
    let half = 180 * 256;
    let r = MAX_RADIUS;
    let valid = [
        json!({ "hit_box": { "type": "circle", "radius": 1 } }),
        json!({ "hit_box": { "type": "circle", "radius": r,
                             "offset": { "forward": r, "right": -r } } }),
        json!({ "hit_box": { "type": "arc", "radius": r, "half_angle": 0 } }),
        json!({ "hit_box": { "type": "arc", "radius": r, "half_angle": half } }),
        json!({ "damage": 1 }),
        json!({ "damage": MAX_DAMAGE }),
        json!({ "kind": "wind_up", "hit_box": { "type": "circle", "radius": 256 } }),
        json!({ "kind": "recovery", "hit_box": { "type": "circle", "radius": 256 } }),
        json!({ "hit_box": { "type": "circle", "radius": 256 }, "rehit_ticks": 1 }),
        json!({ "hit_box": { "type": "circle", "radius": 256 },
                "rehit_ticks": MAX_PHASE_TICKS }),
    ];
    for extra in valid {
        assert_valid(&with_phases(json!([phase(extra)])));
    }
    let circle = |radius: i32, forward: i32, right: i32| {
        json!({ "hit_box": { "type": "circle", "radius": radius,
                             "offset": { "forward": forward, "right": right } } })
    };
    let invalid = [
        (circle(0, 0, 0), at("hit_box.radius")),
        (circle(r + 1, 0, 0), at("hit_box.radius")),
        (circle(256, r + 1, 0), at("hit_box.offset.forward")),
        (circle(256, 0, -r - 1), at("hit_box.offset.right")),
        (
            json!({ "hit_box": { "type": "arc", "radius": 256, "half_angle": -1 } }),
            at("hit_box.half_angle"),
        ),
        (
            json!({ "hit_box": { "type": "arc", "radius": 256, "half_angle": half + 1 } }),
            at("hit_box.half_angle"),
        ),
        (json!({ "damage": 0 }), at("damage")),
        (json!({ "damage": MAX_DAMAGE + 1 }), at("damage")),
        (
            json!({ "hit_box": { "type": "circle", "radius": 256 }, "rehit_ticks": 0 }),
            at("rehit_ticks"),
        ),
        (
            json!({ "hit_box": { "type": "circle", "radius": 256 },
                    "rehit_ticks": MAX_PHASE_TICKS + 1 }),
            at("rehit_ticks"),
        ),
        // A re-hit needs something to hit with: an active phase with a hit box.
        (json!({ "rehit_ticks": 10 }), at("rehit_ticks")),
        (
            json!({ "kind": "wind_up", "hit_box": { "type": "circle", "radius": 256 },
                    "rehit_ticks": 10 }),
            at("rehit_ticks"),
        ),
    ];
    for (extra, path) in invalid {
        let issue = first_issue(&with_phases(json!([phase(extra.clone())])));
        assert_eq!(issue.path, path, "{extra}");
    }
}

#[test]
fn boss_phases_move_forward_and_end_on_a_condition() {
    let b = base();
    let mut last_has_until = b.clone();
    last_has_until["boss"]["phases"][1]["until"] = json!({ "type": "always" });
    assert_eq!(first_issue(&last_has_until).path, "$.boss.phases[1].until");

    let mut first_lacks_until = b.clone();
    first_lacks_until["boss"]["phases"][0]
        .as_object_mut()
        .unwrap()
        .remove("until");
    assert_eq!(
        first_issue(&first_lacks_until).path,
        "$.boss.phases[0].until"
    );

    let mut none = b.clone();
    none["boss"]["phases"] = json!([]);
    assert_eq!(first_issue(&none).path, "$.boss.phases");

    let mut early_loop = b.clone();
    early_loop["boss"]["phases"][0]["timeline"] =
        json!([{ "type": "loop" }, { "type": "wait", "ticks": 1 }]);
    assert_eq!(
        first_issue(&early_loop).path,
        "$.boss.phases[0].timeline[0]"
    );

    let mut transitions = b.clone();
    transitions["boss"]["phases"][1]["transition"] = Value::Array(vec![
        json!({ "type": "clear_bullets" });
        MAX_TRANSITIONS + 1
    ]);
    assert_eq!(
        first_issue(&transitions).path,
        "$.boss.phases[1].transition"
    );

    // Without a boss there are no parts to name.
    let mut no_boss = b.clone();
    no_boss.as_object_mut().unwrap().remove("boss");
    assert_valid(&no_boss);
    set(
        &mut no_boss,
        "/companions/0/rules/0/when",
        json!({ "type": "part_broken", "part": "horn_left" }),
    );
    assert_eq!(
        first_issue(&no_boss).path,
        "$.companions[0].rules[0].when.part"
    );
}

#[test]
fn parts_are_unique_and_conditions_may_only_name_existing_ones() {
    let b = base();
    let mut dup = b.clone();
    dup["boss"]["parts"] = repeated(&b, "/boss/parts/0", 2);
    assert_eq!(first_issue(&dup).path, "$.boss.parts[1].id");

    let mut missing = b.clone();
    set(
        &mut missing,
        "/boss/phases/0/rules/0/when/part",
        json!("horn_right"),
    );
    assert_eq!(
        first_issue(&missing).path,
        "$.boss.phases[0].rules[0].when.part"
    );

    let mut selector = b.clone();
    set(
        &mut selector,
        "/boss/phases/1/rules/0/target",
        json!({ "part": "horn_right" }),
    );
    assert_eq!(
        first_issue(&selector).path,
        "$.boss.phases[1].rules[0].target.part"
    );

    let mut badly_named = b.clone();
    set(&mut badly_named, "/boss/parts/0/id", json!("Horn Left"));
    assert_eq!(first_issue(&badly_named).path, "$.boss.parts[0].id");
}

#[test]
fn a_summoned_agent_cannot_summon() {
    let b = base();
    let summon = b
        .pointer("/boss/phases/1/rules/2/do")
        .expect("the fixture summons")
        .clone();
    let mut v = b.clone();
    v.pointer_mut("/boss/phases/1/rules/2/do/agent")
        .and_then(Value::as_object_mut)
        .expect("the summoned agent is an object")
        .insert(
            "rules".into(),
            json!([{ "when": { "type": "always" }, "target": "self", "do": summon }]),
        );
    let issue = first_issue(&v);
    assert_eq!(issue.path, "$.boss.phases[1].rules[2].do.agent.rules[0].do");
    assert!(issue.message.contains("cannot summon"));
}

/// A stage's map (core#87): each side from `MIN_MAP_SIDE` to `MAX_MAP_SIDE`, and the start on
/// the map itself, not in its margin.
#[test]
fn a_map_is_bounded_and_its_start_is_on_it() {
    let with_map = |map: Value| {
        let mut v = base();
        v["map"] = map;
        v
    };
    let side = 262_144;
    assert_valid(&with_map(json!({ "width": side, "height": side })));
    assert_valid(&with_map(
        json!({ "width": MAX_MAP_SIDE, "height": MAX_MAP_SIDE }),
    ));
    for (map, path) in [
        (
            json!({ "width": MAX_MAP_SIDE + 1, "height": side }),
            "$.map.width",
        ),
        (
            json!({ "width": side, "height": MAX_MAP_SIDE + 1 }),
            "$.map.height",
        ),
        (
            json!({ "width": MIN_MAP_SIDE - 1, "height": side }),
            "$.map.width",
        ),
        (
            json!({ "width": side, "height": MIN_MAP_SIDE - 1 }),
            "$.map.height",
        ),
        (
            json!({ "width": side, "height": side, "start": { "x": side + 1, "y": 0 } }),
            "$.map.start.x",
        ),
        (
            json!({ "width": side, "height": side, "start": { "x": 0, "y": -1 } }),
            "$.map.start.y",
        ),
    ] {
        assert_eq!(first_issue(&with_map(map.clone())).path, path, "{map}");
    }
    // The smallest side is accepted (the fixture's points then fall outside, reported later).
    let smallest = issues(&with_map(
        json!({ "width": MIN_MAP_SIDE, "height": MIN_MAP_SIDE }),
    ));
    assert!(
        smallest.iter().all(|i| !i.path.starts_with("$.map")),
        "{smallest:?}"
    );
    assert_valid(&with_map(
        json!({ "width": side, "height": side, "start": { "x": side, "y": side } }),
    ));
    // Unknown fields are rejected, as everywhere in the stage.
    let mut v = with_map(json!({ "width": side, "height": side, "depth": 1 }));
    assert!(matches!(
        parse_and_validate(&serde_json::to_vec(&v).unwrap()),
        Err(Error::Parse { .. })
    ));
    v["map"] = json!({ "width": side });
    assert!(matches!(
        parse_and_validate(&serde_json::to_vec(&v).unwrap()),
        Err(Error::Parse { .. })
    ));
}

#[test]
fn problems_are_capped_and_say_so() {
    let b = base();
    let mut v = b.clone();
    let mut wave = b.pointer("/waves/0").unwrap().clone();
    wave["count"] = json!(0);
    v["waves"] = Value::Array(vec![wave; 100]);
    let found = issues(&v);
    assert_eq!(found.len(), MAX_ISSUES + 1);
    assert!(found.last().unwrap().message.contains("more problems"));
}

#[test]
fn errors_print_one_line_per_problem() {
    let mut v = base();
    v["player"]["hp"] = json!(0);
    v["title"] = json!("");
    let text = validate(&stage(&v)).unwrap_err().to_string();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.lines().all(|l| l.starts_with("$.")));
}

// --- No input makes the validator panic -------------------------------------------------

fn number_pointers(v: &Value, here: String, out: &mut Vec<String>) {
    match v {
        Value::Number(_) => out.push(here),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                number_pointers(item, format!("{here}/{i}"), out);
            }
        }
        Value::Object(map) => {
            for (k, item) in map {
                number_pointers(item, format!("{here}/{k}"), out);
            }
        }
        _ => {}
    }
}

#[test]
fn extreme_numbers_never_panic() {
    // Overflow checks are on in every profile (ADR-019), so an unchecked `abs()` or `+` on
    // `i32::MIN` in the validator would panic here.
    let b = base();
    let mut pointers = Vec::new();
    number_pointers(&b, String::new(), &mut pointers);
    assert!(pointers.len() > 60, "{} numbers", pointers.len());
    let extremes: [Value; 9] = [
        json!(i32::MIN),
        json!(i32::MAX),
        json!(u32::MAX),
        json!(u32::MAX as u64 + 1),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(-1),
        json!(0),
        json!(1),
    ];
    let mut rng = SplitMix64::new(0x5EED);
    let (mut parsed, mut rejected) = (0u32, 0u32);
    for _ in 0..4000 {
        let mut v = b.clone();
        for _ in 0..=rng.below(3) {
            let p = &pointers[rng.below(pointers.len() as u32) as usize];
            let e = extremes[rng.below(extremes.len() as u32) as usize].clone();
            set(&mut v, p, e);
        }
        if let Ok(s) = serde_json::from_value::<Stage>(v) {
            parsed += 1;
            if validate(&s).is_err() {
                rejected += 1;
            }
        }
    }
    // The mutations reach the validator and most are caught by it.
    assert!(parsed > 1000, "{parsed} parsed");
    assert!(rejected > parsed / 2, "{rejected} of {parsed} rejected");
}

#[test]
fn damaged_files_never_panic() {
    let original = STAGE.as_bytes();
    let mut rng = SplitMix64::new(0xF11E);
    for _ in 0..3000 {
        let mut bytes = original.to_vec();
        for _ in 0..=rng.below(3) {
            let len = bytes.len() as u32;
            let at = rng.below(len) as usize;
            match rng.below(4) {
                0 => bytes[at] = rng.below(256) as u8,
                1 => bytes.truncate(at),
                2 => {
                    let end = (at + rng.below(64) as usize).min(bytes.len());
                    bytes.drain(at..end);
                }
                _ => {
                    let end = (at + rng.below(64) as usize).min(bytes.len());
                    let copy = bytes[at..end].to_vec();
                    bytes.splice(at..at, copy);
                }
            }
            if bytes.is_empty() {
                break;
            }
        }
        let _ = parse_and_validate(&bytes);
    }
}

// --- Weapons (ADR-070) ----------------------------------------------------------------------

/// A combo step making `attack`.
fn step(attack: Value) -> Value {
    json!({ "attack": attack })
}

fn slash() -> Value {
    json!({ "preset": { "id": "slash" } })
}

/// A charge of three levels, at 20, 50 and 80 of 90 ticks, with `edit` applied.
fn charge(edit: impl FnOnce(&mut Value)) -> Value {
    let level = |at: u32, perfect: u32| {
        json!({ "at_ticks": at, "perfect_ticks": perfect, "perfect_pct": 50,
                "attack": { "preset": { "id": "thrust" } } })
    };
    let mut c = json!({ "levels": [level(20, 8), level(50, 8), level(80, 8)], "max_ticks": 90 });
    edit(&mut c);
    c
}

/// An inline Main weapon whose root branches are `combo`, with `edit` applied to it.
fn main_weapon(combo: Value, edit: impl FnOnce(&mut Value)) -> Value {
    let mut w = json!({ "combo": combo, "window_ticks": 20, "assist_range": 0 });
    edit(&mut w);
    json!({ "inline": w })
}

fn with_weapon(role: &str, weapon: Value) -> Value {
    let mut v = base();
    v["player"][role] = weapon;
    v
}

fn with_main(combo: Value, edit: impl FnOnce(&mut Value)) -> Value {
    with_weapon("main", main_weapon(combo, edit))
}

fn with_charge(edit: impl FnOnce(&mut Value)) -> Value {
    let mut charged = step(slash());
    charged["charge"] = charge(edit);
    with_main(json!({ "main": charged }), |_| {})
}

fn with_pull(distance: i32, ticks: u32) -> Value {
    with_weapon(
        "sub",
        json!({ "inline": { "bonus": { "type": "pull", "distance": distance, "ticks": ticks } } }),
    )
}

const CHARGE: &str = "$.player.main.inline.combo.main.charge";

/// A tree of `n` slashes, breadth first: step `i` leads to `2i + 1` by `main` and `2i + 2` by
/// `sub`, so 33 steps are only 6 deep.
fn tree(n: usize) -> Value {
    fn node(i: usize, n: usize) -> Value {
        let mut s = step(slash());
        for (key, child) in [("main", 2 * i + 1), ("sub", 2 * i + 2)] {
            if child < n {
                s["next"][key] = node(child, n);
            }
        }
        s
    }
    json!({ "main": node(0, n) })
}

/// A chain of `n` slashes, each reached by `main`.
fn chain(n: usize) -> Value {
    let mut b = json!({});
    for _ in 0..n {
        let mut s = step(slash());
        if b.as_object().is_some_and(|o| !o.is_empty()) {
            s["next"] = b;
        }
        b = json!({ "main": s });
    }
    b
}

#[test]
fn a_weapon_preset_is_a_weapon_id() {
    let preset = |id: &str| with_weapon("sub", json!({ "preset": { "id": id } }));
    assert_valid(&preset("blade"));
    assert_valid(&preset("maul"));
    for id in ["slash", "no_such_weapon"] {
        assert_eq!(first_issue(&preset(id)).path, "$.player.sub.preset.id");
    }
    // A weapon preset takes no arguments.
    let args = with_weapon("main", json!({ "preset": { "id": "blade", "args": [] } }));
    assert!(serde_json::from_value::<Stage>(args).is_err());
}

#[test]
fn a_main_weapon_holds_combos_and_a_sub_weapon_a_bonus() {
    let sub = json!({ "inline": { "bonus": { "type": "pull", "distance": 0, "ticks": 1 } } });
    let main = main_weapon(chain(1), |_| {});
    assert_valid(&with_weapon("sub", sub.clone()));
    assert!(serde_json::from_value::<Stage>(with_weapon("main", sub)).is_err());
    assert!(serde_json::from_value::<Stage>(with_weapon("sub", main)).is_err());
    // A Main weapon without steps is valid: its presses do nothing.
    assert_valid(&with_main(json!({}), |_| {}));
}

#[test]
fn a_combo_step_s_attack_is_checked_where_it_is() {
    let v = with_main(chain(1), |w| {
        w["combo"]["main"]["next"]["sub"] = step(json!({ "preset": { "id": "no_such_attack" } }))
    });
    assert_eq!(
        first_issue(&v).path,
        "$.player.main.inline.combo.main.next.sub.attack.preset.id"
    );
    let dash = with_main(chain(1), |w| {
        w["dash"] = json!({ "sub": step(json!({ "preset": { "id": "blade" } })) })
    });
    assert_eq!(
        first_issue(&dash).path,
        "$.player.main.inline.dash.sub.attack.preset.id"
    );
}

#[test]
fn combo_limits_hold_at_the_limit_and_fail_one_past() {
    at_limit_and_over(MAX_COMBO_STEPS, "$.player.main.inline.combo", |n| {
        with_main(tree(n), |_| {})
    });
    // The dash branch counts toward the same total.
    let with_dash = |n: usize| with_main(tree(n), |w| w["dash"] = chain(1));
    assert_valid(&with_dash(MAX_COMBO_STEPS - 1));
    assert_eq!(
        first_issue(&with_dash(MAX_COMBO_STEPS)).path,
        "$.player.main.inline.combo"
    );
    let depth = MAX_COMBO_DEPTH as usize;
    assert_valid(&with_main(chain(depth), |_| {}));
    let deepest = format!(
        "$.player.main.inline.combo{}.main",
        ".main.next".repeat(depth)
    );
    assert_eq!(
        first_issue(&with_main(chain(depth + 1), |_| {})).path,
        deepest
    );
    for (key, max) in [
        ("window_ticks", MAX_COMBO_WINDOW_TICKS),
        ("dash_strike_ticks", MAX_DASH_STRIKE_TICKS),
    ] {
        assert_valid(&with_main(chain(1), |w| w[key] = json!(max)));
        assert_eq!(
            first_issue(&with_main(chain(1), |w| w[key] = json!(max + 1))).path,
            format!("$.player.main.inline.{key}")
        );
    }
}

#[test]
fn an_assist_range_is_0_to_its_limit() {
    let with_range = |r: i64| with_main(chain(1), |w| w["assist_range"] = json!(r));
    assert_valid(&with_range(0));
    assert_valid(&with_range(i64::from(MAX_ASSIST_RANGE)));
    let at = "$.player.main.inline.assist_range";
    assert_eq!(first_issue(&with_range(-1)).path, at);
    assert_eq!(
        first_issue(&with_range(i64::from(MAX_ASSIST_RANGE) + 1)).path,
        at
    );
}

#[test]
fn a_pull_is_0_to_its_distance_over_1_to_its_ticks() {
    assert_valid(&with_pull(0, 1));
    assert_valid(&with_pull(MAX_PULL_DISTANCE, MAX_PULL_TICKS));
    let at = |key: &str| format!("$.player.sub.inline.bonus.{key}");
    assert_eq!(first_issue(&with_pull(-1, 1)).path, at("distance"));
    assert_eq!(
        first_issue(&with_pull(MAX_PULL_DISTANCE + 1, 1)).path,
        at("distance")
    );
    assert_eq!(first_issue(&with_pull(0, 0)).path, at("ticks"));
    assert_eq!(
        first_issue(&with_pull(0, MAX_PULL_TICKS + 1)).path,
        at("ticks")
    );
}

#[test]
fn charge_levels_start_after_0_rise_strictly_and_come_before_max_ticks() {
    assert_valid(&with_charge(|_| {}));
    let at = |key: &str| format!("{CHARGE}.{key}");
    // Level 0 is the step's own attack, so level 1 comes after it.
    let first_at_0 = with_charge(|c| c["levels"][0]["at_ticks"] = json!(0));
    assert_eq!(first_issue(&first_at_0).path, at("levels[0].at_ticks"));
    // Level 2's window is emptied, so only the level order is wrong.
    let not_rising = with_charge(|c| {
        c["levels"][1]["perfect_ticks"] = json!(0);
        c["levels"][2]["at_ticks"] = json!(50);
    });
    assert_eq!(first_issue(&not_rising).path, at("levels[2].at_ticks"));
    let at_max = with_charge(|c| {
        c["levels"][2]["perfect_ticks"] = json!(0);
        c["levels"][2]["at_ticks"] = json!(90);
    });
    assert_eq!(first_issue(&at_max).path, at("levels[2].at_ticks"));
    let issue = first_issue(&with_charge(|c| c["levels"] = json!([])));
    assert_eq!(issue.path, at("levels"));
}

#[test]
fn a_perfect_window_ends_by_the_next_level_or_max_ticks() {
    let at = |key: &str| format!("{CHARGE}.{key}");
    // Level 1 is at 20 and level 2 at 50: a window of 30 ends exactly at the next level.
    assert_valid(&with_charge(|c| {
        c["levels"][0]["perfect_ticks"] = json!(30)
    }));
    assert_eq!(
        first_issue(&with_charge(|c| c["levels"][0]["perfect_ticks"] = json!(31))).path,
        at("levels[0].perfect_ticks")
    );
    // The last, at 80, ends by max_ticks, 90.
    assert_valid(&with_charge(|c| {
        c["levels"][2]["perfect_ticks"] = json!(10)
    }));
    assert_eq!(
        first_issue(&with_charge(|c| c["levels"][2]["perfect_ticks"] = json!(11))).path,
        at("levels[2].perfect_ticks")
    );
}

#[test]
fn charge_limits_hold_at_the_limit_and_fail_one_past() {
    let at = |key: &str| format!("{CHARGE}.{key}");
    let level = |i: u32| json!({ "at_ticks": i + 1, "attack": { "preset": { "id": "thrust" } } });
    at_limit_and_over(MAX_CHARGE_LEVELS, &at("levels"), |n| {
        with_charge(|c| c["levels"] = Value::Array((0..n as u32).map(level).collect()))
    });
    assert_valid(&with_charge(|c| c["max_ticks"] = json!(MAX_CHARGE_TICKS)));
    assert_eq!(
        first_issue(&with_charge(
            |c| c["max_ticks"] = json!(MAX_CHARGE_TICKS + 1)
        ))
        .path,
        at("max_ticks")
    );
    assert_valid(&with_charge(|c| {
        c["levels"][1]["perfect_pct"] = json!(MAX_PERFECT_PCT)
    }));
    assert_eq!(
        first_issue(&with_charge(|c| {
            c["levels"][1]["perfect_pct"] = json!(MAX_PERFECT_PCT + 1)
        }))
        .path,
        at("levels[1].perfect_pct")
    );
    assert_valid(&with_charge(|c| c["move_pct"] = json!(0)));
    assert_eq!(
        first_issue(&with_charge(|c| c["move_pct"] = json!(101))).path,
        at("move_pct")
    );
    // A charge has no cooldown (ADR-070).
    let cooldown = with_charge(|c| c["cooldown_ticks"] = json!(1));
    assert!(serde_json::from_value::<Stage>(cooldown).is_err());
    let perfect_attack = with_charge(|c| {
        c["levels"][0]["perfect_attack"] = json!({ "preset": { "id": "no_such_attack" } })
    });
    assert_eq!(
        first_issue(&perfect_attack).path,
        at("levels[0].perfect_attack.preset.id")
    );
}

#[test]
fn a_phase_may_break_charges() {
    let mut v = base();
    *enemy_attack(&mut v) = json!({ "inline": [phase(json!({ "breaks_charge": true }))] });
    assert_valid(&v);
}

#[test]
fn an_enemy_s_and_the_boss_s_expertise_points_are_capped() {
    for at in ["/waves/0", "/boss"] {
        let path = format!(
            "${}",
            at.replace("/waves/0", ".waves[0]")
                .replace("/boss", ".boss")
        );
        let mut v = base();
        set_new(
            &mut v,
            at,
            "expertise",
            json!({ "hit": MAX_EXPERTISE_PER_HIT, "kill": MAX_EXPERTISE_PER_KILL }),
        );
        assert_valid(&v);
        let mut over = v.clone();
        set(
            &mut over,
            &format!("{at}/expertise/hit"),
            json!(MAX_EXPERTISE_PER_HIT + 1),
        );
        assert_eq!(first_issue(&over).path, format!("{path}.expertise.hit"));
        let mut over = v.clone();
        set(
            &mut over,
            &format!("{at}/expertise/kill"),
            json!(MAX_EXPERTISE_PER_KILL + 1),
        );
        assert_eq!(first_issue(&over).path, format!("{path}.expertise.kill"));
        // Both numbers are given, and nothing else.
        let mut partial = base();
        set_new(&mut partial, at, "expertise", json!({ "hit": 1 }));
        assert!(serde_json::from_value::<Stage>(partial).is_err());
    }
}
