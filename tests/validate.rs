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

fn shot(emitters: Value) -> Value {
    json!({ "inline": emitters })
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
    at_limit_and_over(MAX_SKILL_SLOTS, "$.player.skills", |n| {
        let mut v = b.clone();
        let skills: Vec<Value> = (0..n)
            .map(|i| {
                let mut s = b.pointer("/player/skills/0").unwrap().clone();
                s["slot"] = json!(i % 3 + 1);
                s
            })
            .collect();
        v["player"]["skills"] = Value::Array(skills);
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
        v["player"]["shot"] = shot(json!([node]));
        v
    };
    assert_valid(&nested(MAX_PATTERN_DEPTH as usize - 1));
    let issue = first_issue(&nested(MAX_PATTERN_DEPTH as usize));
    assert!(issue.message.contains("levels deep"), "{issue:?}");
    assert!(
        issue.path.starts_with("$.player.shot.inline[0]"),
        "{issue:?}"
    );

    let waits = |n: usize| {
        let mut v = b.clone();
        v["player"]["shot"] = shot(Value::Array(vec![json!({ "type": "wait", "ticks": 1 }); n]));
        v
    };
    assert_valid(&waits(MAX_PATTERN_NODES as usize));
    let issue = first_issue(&waits(MAX_PATTERN_NODES as usize + 1));
    assert_eq!(issue.path, "$.player.shot.inline[64]");
    assert!(issue.message.contains("64 nodes"), "{issue:?}");

    // Nodes are counted across the whole tree, branches included.
    let branches = |n: usize| {
        let leaf = |_: usize| json!({ "type": "rotate", "angle": 0, "body": [fire(json!(1))] });
        let mut v = b.clone();
        v["player"]["shot"] = shot(Value::Array((0..n).map(leaf).collect()));
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
        v["player"]["shot"] = shot(json!([fire(expr_with_nodes(n))]));
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
            v["player"]["shot"] = shot(json!([node]));
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
        bad("/player/skills/1/charges", json!(0)),
        "$.player.skills[1].charges"
    );
    assert_eq!(
        bad("/player/skills/0/skill/damage", json!(MAX_DAMAGE + 1)),
        "$.player.skills[0].skill.damage"
    );
    assert_eq!(
        bad("/player/skills/1/skill/value_pct", json!(MAX_BUFF_PCT + 1)),
        "$.player.skills[1].skill.value_pct"
    );
    assert_eq!(
        bad("/player/skills/1/skill/duration_ticks", json!(0)),
        "$.player.skills[1].skill.duration_ticks"
    );
}

#[test]
fn skill_slots_are_one_to_three_and_unique() {
    let b = base();
    let slots = |a: u8, c: u8| {
        let mut v = b.clone();
        v["player"]["skills"][0]["slot"] = json!(a);
        v["player"]["skills"][1]["slot"] = json!(c);
        v
    };
    assert_valid(&slots(1, 2));
    assert_valid(&slots(3, 1));
    assert_eq!(first_issue(&slots(2, 2)).path, "$.player.skills[1].slot");
    assert_eq!(first_issue(&slots(0, 2)).path, "$.player.skills[0].slot");
    assert_eq!(first_issue(&slots(1, 4)).path, "$.player.skills[1].slot");
    let mut none = b.clone();
    none["player"]["skills"] = json!([]);
    assert_valid(&none);
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
            "/boss/phases/1/timeline/1/attack/inline/0/then/0/body/0/bullet",
            "orb_huge"
        )
        .path,
        "$.boss.phases[1].timeline[1].attack.inline[0].then[0].body[0].bullet"
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
        v["player"]["shot"] = json!({ "preset": { "id": id, "args": args } });
        v
    };
    assert_valid(&with("twin_shot", json!([])));
    assert_valid(&with("aimed_single", json!([512])));
    assert_eq!(
        first_issue(&with("aimed_single", json!([]))).path,
        "$.player.shot.preset.args"
    );
    assert_eq!(
        first_issue(&with("twin_shot", json!([1]))).path,
        "$.player.shot.preset.args"
    );
    assert_eq!(
        first_issue(&with("aimed_single", json!([MAX_NUMBER + 1]))).path,
        "$.player.shot.preset.args[0].value"
    );
    let many = Value::Array(vec![json!(0); MAX_PRESET_ARGS + 1]);
    assert_eq!(
        first_issue(&with("twin_shot", many)).path,
        "$.player.shot.preset.args"
    );
    assert_eq!(
        first_issue(&with("not_a_preset", json!([]))).path,
        "$.player.shot.preset.id"
    );
}

#[test]
fn inline_attacks_take_no_arguments() {
    let mut v = base();
    v["player"]["shot"] = shot(json!([fire(json!({ "arg": 0 }))]));
    let issue = first_issue(&v);
    assert_eq!(issue.path, "$.player.shot.inline[0].speed");
    assert!(issue.message.contains("no such argument"));
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
