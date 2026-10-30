//! The checked-in JSON Schema matches the types (ADR-018). After changing a type, regenerate:
//! `UPDATE_SCHEMA=1 cargo test --test integration schema::`, and commit the diff with the change.

use std::collections::BTreeSet;
use std::path::Path;

#[test]
fn schema_bundle_and_ref_stubs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
    let bundle_path = dir.join("content.schema.json");
    assert!(
        bundle_path.exists(),
        "schema/content.schema.json must exist as the schema bundle"
    );
    let bundle_text = std::fs::read_to_string(&bundle_path).expect("read content.schema.json");
    let bundle: serde_json::Value =
        serde_json::from_str(&bundle_text).expect("bundle is valid JSON");
    assert_eq!(
        bundle.get("$schema").and_then(|v| v.as_str()),
        Some("https://json-schema.org/draft/2020-12/schema"),
        "bundle must declare draft 2020-12 schema"
    );
    let defs = bundle
        .get("$defs")
        .and_then(|v| v.as_object())
        .expect("bundle must contain $defs object");

    for entry in std::fs::read_dir(&dir).expect("read schema dir") {
        let entry = entry.expect("valid entry");
        let name = entry.file_name().into_string().expect("utf8 file name");
        if !name.ends_with(".schema.json") || name == "content.schema.json" {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).expect("read schema stub");
        let stub: serde_json::Value = serde_json::from_str(&text).expect("stub is valid JSON");
        assert_eq!(
            stub.get("$schema").and_then(|v| v.as_str()),
            Some("https://json-schema.org/draft/2020-12/schema"),
            "schema stub {name} must declare draft 2020-12"
        );
        let ref_str = stub
            .get("$ref")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("schema stub {name} must contain a $ref"));
        let target = ref_str
            .strip_prefix("content.schema.json#/$defs/")
            .unwrap_or_else(|| {
                panic!("schema stub {name} $ref must point to content.schema.json#/$defs/..., got: {ref_str}")
            });
        assert!(
            defs.contains_key(target),
            "bundle $defs does not contain {target} referenced by {name}"
        );
    }
}

#[test]
fn every_validate_limit_const_appears_in_schema_bundle() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let validate_rs = std::fs::read_to_string(manifest_dir.join("src/validate.rs"))
        .expect("read src/validate.rs");
    let limits_start = validate_rs
        .find("pub mod limits {")
        .expect("pub mod limits found");
    let limits_end = validate_rs[limits_start..]
        .find("\n}")
        .expect("limits block end found")
        + limits_start;
    let limits_block = &validate_rs[limits_start..limits_end];
    let body = &validate_rs[limits_end..];

    let mut used_consts = Vec::new();
    for line in limits_block.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("pub const MAX_") && !trimmed.starts_with("pub const MIN_") {
            continue;
        }
        let after_const = trimmed.strip_prefix("pub const ").unwrap();
        let name = after_const.split(':').next().unwrap().trim();
        if name == "MAX_STAGE_BYTES" || name == "MAX_ISSUES" {
            continue;
        }
        let is_used = body
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == name);
        if !is_used {
            continue;
        }

        let expr = trimmed
            .split('=')
            .nth(1)
            .unwrap()
            .trim_end_matches(';')
            .trim();
        let mut val: i64 = 1;
        for factor in expr.split('*') {
            let f = factor.trim();
            let factor_val: i64 = if f == "DAMAGE_SCALE" {
                100
            } else {
                f.replace('_', "")
                    .parse()
                    .unwrap_or_else(|e| panic!("failed to parse factor '{f}' in {name}: {e}"))
            };
            val *= factor_val;
        }
        used_consts.push((name.to_string(), val));
    }

    assert!(
        !used_consts.is_empty(),
        "must find used constants in validate.rs"
    );

    let bundle_path = manifest_dir.join("schema/content.schema.json");
    assert!(
        bundle_path.exists(),
        "schema/content.schema.json must exist"
    );
    let bundle_text = std::fs::read_to_string(&bundle_path).expect("read content.schema.json");
    let bundle: serde_json::Value =
        serde_json::from_str(&bundle_text).expect("bundle is valid JSON");

    let mut schema_bounds = BTreeSet::new();
    fn collect_bounds(value: &serde_json::Value, bounds: &mut BTreeSet<i64>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    if matches!(
                        k.as_str(),
                        "maximum"
                            | "minimum"
                            | "maxLength"
                            | "minLength"
                            | "maxItems"
                            | "minItems"
                            | "exclusiveMaximum"
                            | "exclusiveMinimum"
                    ) {
                        if let Some(n) = v.as_i64() {
                            bounds.insert(n);
                        } else if let Some(n) = v.as_u64() {
                            bounds.insert(i64::try_from(n).unwrap());
                        }
                    }
                    collect_bounds(v, bounds);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    collect_bounds(item, bounds);
                }
            }
            _ => {}
        }
    }
    collect_bounds(&bundle, &mut schema_bounds);

    let mut missing = Vec::new();
    for (name, val) in &used_consts {
        if !schema_bounds.contains(val) {
            missing.push((name.clone(), *val));
        }
    }

    assert!(
        missing.is_empty(),
        "constants used in validate.rs missing from schema bundle bounds: {missing:?}"
    );
}

#[test]
fn checked_in_schema_is_current() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
    #[expect(clippy::disallowed_methods, reason = "test knob")]
    let update = std::env::var_os("UPDATE_SCHEMA").is_some();
    let mut stale = Vec::new();
    for (name, json) in stella_rain_core::schema::all() {
        let path = dir.join(name);
        if update {
            if std::fs::read_to_string(&path).ok().as_deref() != Some(json.as_str()) {
                std::fs::write(&path, &json).unwrap();
            }
        } else if std::fs::read_to_string(&path).ok().as_deref() != Some(json.as_str()) {
            stale.push(name);
        }
    }
    // A file for a type that no longer exists is stale too.
    let known: Vec<&str> = stella_rain_core::schema::all()
        .iter()
        .map(|(n, _)| *n)
        .collect();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        assert!(
            known.contains(&name.as_str()),
            "schema/{name} belongs to no type: delete it"
        );
    }
    assert!(
        stale.is_empty(),
        "stale schema files {stale:?}: run `UPDATE_SCHEMA=1 cargo test --test integration schema::`"
    );
}

#[test]
fn a_player_hit_names_its_source() {
    use stella_rain_core::event::{DomainEvent, HitKind, HitSource};
    use stella_rain_core::id::EntityId;
    let hit = |attack: Option<std::sync::Arc<str>>| DomainEvent::PlayerHit {
        hp_left: 1,
        source: HitSource {
            kind: HitKind::Link,
            owner: EntityId(5),
            attack,
        },
    };
    assert_eq!(
        serde_json::to_value(hit(None)).unwrap(),
        serde_json::json!({ "type": "PlayerHit",
            "data": { "hp_left": 1, "source": { "kind": "link", "owner": 5 } } })
    );
    assert_eq!(
        serde_json::to_string(&hit(Some("x".into()))).unwrap(),
        r#"{"type":"PlayerHit","data":{"hp_left":1,"source":{"kind":"link","owner":5,"attack":"x"}}}"#
    );
    let extra =
        r#"{"type":"PlayerHit","data":{"hp_left":1,"source":{"kind":"zone","owner":5,"x":1}}}"#;
    assert!(serde_json::from_str::<DomainEvent>(extra).is_err());
    let schema: serde_json::Value = stella_rain_core::schema::all()
        .iter()
        .find(|(n, _)| *n == "content.schema.json")
        .map(|(_, j)| serde_json::from_str(j).unwrap())
        .unwrap();
    let one_of = schema["$defs"]["DomainEvent"]["oneOf"].as_array().unwrap();
    let variant = one_of
        .iter()
        .find(|v| v["properties"]["type"]["const"] == "PlayerHit")
        .unwrap();
    assert_eq!(
        variant["properties"]["data"]["required"],
        serde_json::json!(["hp_left", "source"])
    );
}

/// EVENT-ARC: an event that names a content ID serialises the ID as the plain string it was when
/// `ContentId` owned a `String`, and reads it back.
#[test]
fn the_events_that_name_an_id_keep_their_json() {
    use stella_rain_core::event::DomainEvent;
    use stella_rain_core::id::{ContentId, EntityId};
    let cases = [
        (
            DomainEvent::EliteSpawned {
                elite: EntityId(7),
                asset: ContentId::from("golem"),
                kind: None,
            },
            r#"{"type":"EliteSpawned","data":{"elite":7,"asset":"golem"}}"#,
        ),
        (
            DomainEvent::StatusApplied {
                target: EntityId(3),
                status: ContentId::from("stun"),
                stacks: 1,
                duration_ticks: 30,
            },
            r#"{"type":"StatusApplied","data":{"target":3,"status":"stun","stacks":1,"duration_ticks":30}}"#,
        ),
        (
            DomainEvent::RewardTaken {
                reward: ContentId::from("r"),
                taken: 1,
                rune: None,
                star: Some("S".into()),
            },
            r#"{"type":"RewardTaken","data":{"reward":"r","taken":1,"star":"S"}}"#,
        ),
        (
            DomainEvent::RewardTaken {
                reward: ContentId::from("r"),
                taken: 1,
                rune: None,
                star: None,
            },
            r#"{"type":"RewardTaken","data":{"reward":"r","taken":1}}"#,
        ),
        (
            DomainEvent::StatOfferOpened {
                options: vec![ContentId::from("attack"), ContentId::from("speed")],
            },
            r#"{"type":"StatOfferOpened","data":{"options":["attack","speed"]}}"#,
        ),
        (
            DomainEvent::PartBroken {
                part: ContentId::from("arm"),
            },
            r#"{"type":"PartBroken","data":{"part":"arm"}}"#,
        ),
    ];
    for (event, json) in cases {
        assert_eq!(serde_json::to_string(&event).unwrap(), json);
        assert_eq!(serde_json::from_str::<DomainEvent>(json).unwrap(), event);
    }
    assert_eq!(
        serde_json::to_string(&ContentId::from("x")).unwrap(),
        r#""x""#
    );
}

#[test]
fn preset_argument_bounds_in_schema() {
    use stella_rain_core::registry::{ENTRIES, Kind};

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
    let bundle_path = dir.join("content.schema.json");
    let bundle_text = std::fs::read_to_string(&bundle_path).expect("read content.schema.json");
    let bundle: serde_json::Value =
        serde_json::from_str(&bundle_text).expect("bundle is valid JSON");

    let preset_schema = &bundle["$defs"]["AttackRef"]["oneOf"][0]["properties"]["preset"];
    let all_of = preset_schema["allOf"]
        .as_array()
        .expect("AttackRef.preset must have allOf array");

    let bounded_presets: Vec<_> = ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Preset && !e.arg_bounds.is_empty())
        .collect();

    assert_eq!(
        all_of.len(),
        bounded_presets.len(),
        "allOf must have one entry per bounded preset"
    );

    for (entry, clause) in bounded_presets.iter().zip(all_of.iter()) {
        assert_eq!(
            clause["if"]["properties"]["id"]["const"], entry.id,
            "if.properties.id.const must match preset ID"
        );
        assert_eq!(
            clause["if"]["required"],
            serde_json::json!(["id"]),
            "if.required must be [\"id\"]"
        );
        let prefix_items = clause["then"]["properties"]["args"]["prefixItems"]
            .as_array()
            .expect("then.properties.args.prefixItems must be an array");
        assert_eq!(prefix_items.len(), entry.arg_bounds.len());
        for (b, item) in entry.arg_bounds.iter().zip(prefix_items.iter()) {
            assert_eq!(item["type"], "integer");
            assert_eq!(item["minimum"], b.min);
            assert_eq!(item["maximum"], b.max);
        }
        assert_eq!(
            clause["then"]["properties"]["args"]["minItems"],
            entry.arg_bounds.len()
        );
        assert_eq!(
            clause["then"]["properties"]["args"]["maxItems"],
            entry.arg_bounds.len()
        );
    }
}
