//! The version 0 types (ADR-018, ADR-035): files round-trip, unknown fields are rejected, and
//! the input record says nothing about the device.

use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{ContentId, EntityId};
use stella_rain_core::input::Input;
use stella_rain_core::replay::ReplayHeader;
use stella_rain_core::stage::Stage;
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
        "/player/skills/0",
        "/player/skills/0/skill",
        "/companions/0/rules/0",
        "/companions/0/rules/0/when",
        "/companions/0/movement",
        "/waves/0/spawn",
        "/boss/parts/0",
        "/boss/phases/0",
        "/boss/phases/0/timeline/0",
        "/boss/phases/0/timeline/1/attack/inline/0",
        "/boss/phases/0/timeline/1/attack/inline/0/body/0/body/0/direction",
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
fn input_record_holds_movement_focus_fire_and_held_slot_only() {
    let input = Input {
        dx: -768,
        dy: 256,
        focus: true,
        fire: true,
        held: 2,
    };
    let value = serde_json::to_value(input).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["dx", "dy", "fire", "focus", "held"]);
    assert_eq!(serde_json::from_value::<Input>(value).unwrap(), input);
    assert!(
        serde_json::from_str::<Input>(
            r#"{"dx":0,"dy":0,"focus":false,"fire":false,"held":0,"device":"touch"}"#
        )
        .is_err()
    );
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
fn replay_header_round_trips() {
    let header = ReplayHeader {
        schema_version: 0,
        sim_version: 0,
        seed: 48213,
        stage_sha256: "0".repeat(64),
        ticks: 18000,
        final_hash: "9f3a0c12aa55bb66".into(),
    };
    let json = serde_json::to_string(&header).unwrap();
    assert_eq!(serde_json::from_str::<ReplayHeader>(&json).unwrap(), header);
}
