//! The version 0 types (ADR-018, ADR-035): files round-trip, unknown fields are rejected, and
//! the input record says nothing about the device.

use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{ContentId, EntityId};
use stella_rain_core::input::Input;
use stella_rain_core::replay::Replay;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate;
use stella_rain_core::version::{SCHEMA_VERSION, SIM_VERSION};

const STAGE: &str = include_str!("fixtures/stage_v0.json");

fn parse(json: &str) -> Result<Stage, serde_json::Error> {
    serde_json::from_str(json)
}

/// The fixture with `pointer`'s object given one more field.
fn with_extra_field(pointer: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(STAGE).unwrap();
    value
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("{pointer} exists in the fixture"))
        .as_object_mut()
        .unwrap_or_else(|| panic!("{pointer} is an object"))
        .insert("unexpected".into(), serde_json::Value::Bool(true));
    value.to_string()
}

#[test]
fn versions_are_zero_until_the_freeze() {
    assert_eq!((SCHEMA_VERSION, SIM_VERSION), (0, 0));
    let stage = parse(STAGE).unwrap();
    assert_eq!(
        (stage.schema_version, stage.sim_version),
        (SCHEMA_VERSION, SIM_VERSION)
    );
}

#[test]
fn stage_with_every_tier_round_trips() {
    let stage = parse(STAGE).unwrap();
    let again = parse(&serde_json::to_string(&stage).unwrap()).unwrap();
    assert_eq!(stage, again);
    let boss = stage.boss.as_ref().unwrap();
    assert_eq!(boss.phases.len(), 2);
    assert_eq!(boss.parts[0].id, ContentId::from("horn_left"));
}

#[test]
fn unknown_fields_are_rejected_at_every_level() {
    for pointer in [
        "",
        "/player",
        "/player/evade",
        "/companions/0/rules/0/do/skill",
        "/companions/0/rules/0",
        "/companions/0/rules/0/when",
        "/companions/0/movement",
        "/waves/0/spawn",
        "/boss/parts/0",
        "/boss/phases/0",
        "/boss/phases/0/timeline/0",
        "/boss/phases/0/timeline/1/attack/inline/0",
        "/boss/phases/0/timeline/1/attack/inline/0/spawn/0",
        "/boss/phases/0/timeline/1/attack/inline/0/spawn/0/body/0/body/0/direction",
        "/boss/phases/1/transition/1",
    ] {
        let err = parse(&with_extra_field(pointer))
            .err()
            .unwrap_or_else(|| panic!("an extra field at {pointer:?} is rejected"));
        assert!(
            err.to_string().contains("unknown field"),
            "{pointer:?}: {err}"
        );
    }
}

#[test]
fn missing_required_fields_are_rejected() {
    let mut value: serde_json::Value = serde_json::from_str(STAGE).unwrap();
    value.as_object_mut().unwrap().remove("seed");
    assert!(parse(&value.to_string()).is_err());
}

#[test]
fn input_record_holds_the_stick_four_buttons_and_two_switches_only() {
    // ADR-071: named for what they do, never for a device, a button or a key.
    let input = Input {
        move_x: -127,
        move_y: 64,
        main: true,
        smash: false,
        evade: true,
        skill: false,
        assist: true,
        auto: false,
    };
    let value = serde_json::to_value(input).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "assist", "auto", "evade", "main", "move_x", "move_y", "skill", "smash"
        ]
    );
    assert_eq!(serde_json::from_value::<Input>(value).unwrap(), input);
    let all = r#""move_x":0,"move_y":0,"main":false,"smash":false,"evade":false,"skill":false,"assist":false,"auto":false"#;
    assert!(serde_json::from_str::<Input>(&format!("{{{all}}}")).is_ok());
    for bad in [
        format!(r#"{{{all},"device":"touch"}}"#),
        format!(r#"{{{all},"charge":true}}"#),
        format!(r#"{{{all},"fire":true}}"#),
        format!("{{{}}}", all.replace(r#""move_x":0"#, r#""move_x":128"#)),
        r#"{"move_x":0,"move_y":0,"main":false,"smash":false,"evade":false}"#.to_string(),
    ] {
        assert!(serde_json::from_str::<Input>(&bad).is_err(), "{bad}");
    }
}

#[test]
fn events_are_type_and_data() {
    let event = DomainEvent::RuleFired {
        agent: EntityId(7),
        rule: 1,
        target: Some(EntityId(0)),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(
        json,
        r#"{"type":"RuleFired","data":{"agent":7,"rule":1,"target":0}}"#
    );
    assert_eq!(serde_json::from_str::<DomainEvent>(&json).unwrap(), event);
    assert_eq!(
        serde_json::to_string(&DomainEvent::StageCleared).unwrap(),
        r#"{"type":"StageCleared"}"#
    );
    assert!(
        serde_json::from_str::<DomainEvent>(r#"{"type":"PhaseChanged","data":{"phase":1,"x":0}}"#)
            .is_err()
    );
}

#[test]
fn replay_round_trips_and_rejects_unknown_fields() {
    let replay = Replay {
        schema_version: 1,
        sim_version: 1,
        seed: 48213,
        stage_sha256: "0".repeat(64),
        ticks: 18000,
        final_hash: "9f3a0c12aa55bb66".into(),
        inputs: "AQAAAA==".into(),
    };
    let json = serde_json::to_string(&replay).unwrap();
    assert_eq!(serde_json::from_str::<Replay>(&json).unwrap(), replay);
    let extra = json.replacen('{', r#"{"unexpected":true,"#, 1);
    assert!(serde_json::from_str::<Replay>(&extra).is_err());
}

#[test]
fn versions_are_one_from_the_freeze() {
    // Version 1 is the frozen first version (ADR-REPL-7); version 0 is refused.
    assert_eq!((SCHEMA_VERSION, SIM_VERSION), (1, 1));
    let stage = parse(STAGE).unwrap();
    assert_eq!(
        (stage.schema_version, stage.sim_version),
        (SCHEMA_VERSION, SIM_VERSION)
    );
}

/// The fixture with its two versions set.
fn at_versions(schema_version: u32, sim_version: u32) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_str(STAGE).unwrap();
    value["schema_version"] = schema_version.into();
    value["sim_version"] = sim_version.into();
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn a_stage_at_version_0_is_refused() {
    // ADR-REPL-7 point 8: no migration from version 0.
    for (schema_version, sim_version) in [(0, 0), (SCHEMA_VERSION, 0), (0, SIM_VERSION)] {
        assert_eq!(
            validate::parse_and_validate(&at_versions(schema_version, sim_version)),
            Err(validate::Error::UnsupportedVersion {
                schema_version,
                sim_version
            })
        );
    }
}
