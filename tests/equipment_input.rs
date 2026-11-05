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

mod recorder {
    use super::REPLACE37;
    use stella_rain_core::input::{EquipAction, Input};
    use stella_rain_core::run_replay::{RunRecorder, RunReplayError, verify_run};
    use stella_rain_core::run_setup::RunSetup;

    fn start() -> RunRecorder {
        RunRecorder::start(&RunSetup::default()).unwrap()
    }

    #[test]
    fn a_refused_action_records_nothing() {
        let mut rec = start();
        let before = rec.replay();
        let err = rec.actions(&[Some(REPLACE37), None, None]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "an equipment action on tick 0 is not built yet"
        );
        assert!(matches!(err, RunReplayError::ActionRefused { tick: 0, .. }));
        assert_eq!(rec.replay(), before);
        assert_eq!(before.ticks, 0);
        rec.step(&[Input::default(); 3]).unwrap();
    }

    #[test]
    fn a_step_of_actions_holds_an_action() {
        let mut rec = start();
        let why = RunReplayError::BadInputs("a step of actions holds an action");
        assert_eq!(rec.actions(&[None, None, None]), Err(why.clone()));
        // An absent slot's entry is ignored.
        assert_eq!(rec.actions(&[None, Some(REPLACE37), None]), Err(why));
    }

    #[test]
    fn a_refused_action_in_a_step_is_ignored_like_a_refused_pick() {
        let acting = Input {
            action: Some(EquipAction::Extract { item: 1 }),
            ..Input::default()
        };
        let mut with = start();
        let mut idle = start();
        for _ in 0..10 {
            with.step(&[acting, Input::default(), Input::default()])
                .unwrap();
            idle.step(&[Input::default(); 3]).unwrap();
        }
        assert_eq!(with.replay(), idle.replay());
        verify_run(&with.replay().to_bytes()).unwrap();
    }
}
