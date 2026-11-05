//! The equipment action is refused by every engine until a rule lands (ADR-SIM-7): `check_action`
//! says why, reads nothing it should not and changes nothing, and `step` ignores the field.

use crate::id::PlayerIdx;
use crate::input::{EquipAction, Input};
use crate::run_setup::RunSetup;

use super::*;

const ALL: [EquipAction; 5] = [
    EquipAction::Equip { item: 1, slot: 2 },
    EquipAction::Replace { item: 3, slot: 7 },
    EquipAction::Extract { item: 0 },
    EquipAction::Craft { star: 9 },
    EquipAction::TreePick { option: 4 },
];

fn setup() -> RunSetup {
    RunSetup {
        seed: 20_261_105,
        pick_stats: true,
        ..RunSetup::default()
    }
}

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

#[test]
fn every_action_is_not_built_yet() {
    let e = Engine::start(&setup()).unwrap();
    for action in ALL {
        assert_eq!(
            e.check_action(PlayerIdx::SOLO, action),
            Err(ActionError::NotBuilt),
            "{action:?}"
        );
    }
}

#[test]
fn a_slot_outside_the_party_is_no_player() {
    let e = Engine::start_party(&setup(), 2).unwrap();
    for action in ALL {
        assert_eq!(e.check_action(slot(1), action), Err(ActionError::NotBuilt));
        assert_eq!(
            e.check_action(slot(2), action),
            Err(ActionError::NoPlayer { player: slot(2) })
        );
    }
}

#[test]
fn the_check_is_read_only() {
    let e = Engine::start(&setup()).unwrap();
    let before = e.state_hash();
    for action in ALL {
        let _ = e.check_action(PlayerIdx::SOLO, action);
    }
    assert_eq!(e.state_hash(), before);
}

#[test]
fn step_does_not_read_the_action() {
    let mut with = Engine::start(&setup()).unwrap();
    let mut without = Engine::start(&setup()).unwrap();
    let acting = Input {
        action: Some(ALL[1]),
        ..Input::default()
    };
    for _ in 0..100 {
        with.step(&[acting, Input::default(), Input::default()]);
        without.step(&[Input::default(); 3]);
        assert_eq!(with.state_hash(), without.state_hash());
    }
}

#[test]
fn the_first_version_refuses_too() {
    let e = Engine::start(&RunSetup {
        schema_version: 1,
        ..setup()
    })
    .unwrap();
    assert_eq!(
        e.check_action(PlayerIdx::SOLO, ALL[0]),
        Err(ActionError::NotBuilt)
    );
}
