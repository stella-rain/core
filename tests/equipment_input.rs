//! The equipment action in the input and its record (ADR-SIM-7, ADR-REPL-5, ADR-REPL-3).
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::replay::pack_inputs;

const REPLACE37: EquipAction = EquipAction::Replace { item: 3, slot: 7 };

#[test]
fn an_input_without_an_action_writes_no_action_key() {
    let json = serde_json::to_string(&Input::default()).unwrap();
    assert!(!json.contains("action"), "{json}");
    let json = serde_json::to_string(&Input::picking(2)).unwrap();
    assert!(!json.contains("action"), "{json}");
    assert!(json.contains("\"pick\":2"), "{json}");
}

#[test]
fn an_action_round_trips_in_the_json() {
    let input = Input {
        action: Some(REPLACE37),
        ..Input::default()
    };
    let json = serde_json::to_string(&input).unwrap();
    assert!(
        json.contains("\"action\":{\"replace\":{\"item\":3,\"slot\":7}}"),
        "{json}"
    );
    assert_eq!(serde_json::from_str::<Input>(&json).unwrap(), input);
}

#[test]
fn an_action_parses_strictly() {
    let base = r#"{"move_x":0,"move_y":0,"main":false,"smash":false,"evade":false,"skill":false,"assist":false,"auto":false,"action":"#;
    let parse = |a: &str| serde_json::from_str::<Input>(&format!("{base}{a}}}"));
    assert_eq!(
        parse(r#"{"craft":{"star":9}}"#).unwrap().action,
        Some(EquipAction::Craft { star: 9 })
    );
    assert!(parse(r#"{"sell":{}}"#).is_err());
    assert!(parse(r#"{"equip":{"item":1}}"#).is_err());
    assert!(parse(r#"{"extract":{"item":1,"more":2}}"#).is_err());
}

#[test]
fn the_packed_stream_never_holds_an_action() {
    let with = Input {
        action: Some(REPLACE37),
        ..Input::default()
    };
    assert_eq!(pack_inputs(&[with]), pack_inputs(&[Input::default()]));
}
