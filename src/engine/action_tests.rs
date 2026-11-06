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

// --- equip and replace in an equipment run (ADR-SIM-7) -----------------------------------------

mod equipment_runs {
    use super::*;
    use crate::run_setup::BuildMode;
    use crate::rune::Rune;
    use crate::sim::equipment::{Equipment, Held};

    fn equipment(lumine: bool) -> RunSetup {
        RunSetup {
            build_mode: BuildMode::Equipment,
            starting_rune: Some("thunder".into()),
            lumine: u8::from(lumine),
            lumine_star: lumine.then(|| "max_hp_common".into()),
            pick_stats: false,
            ..setup()
        }
    }

    fn place(e: &Engine, id: &str) -> u16 {
        u16::try_from(
            e.content
                .rewards()
                .index_of(&id.into())
                .expect("bundled star"),
        )
        .unwrap()
    }

    /// The inventory every test gives a player: index 0 `max_hp_common` (Defence), 1
    /// `max_hp_rare` (same option), 2 `attack_common` (Attack), 3 `xp_common` (Growth), 4 the
    /// lone random rune, 5 the starting rune again, 6 another rune.
    fn give(e: &mut Engine, player: u8) {
        let items = [
            Held::Star(place(e, "max_hp_common")),
            Held::Star(place(e, "max_hp_rare")),
            Held::Star(place(e, "attack_common")),
            Held::Star(place(e, "xp_common")),
            Held::Star(place(e, "random_rune")),
            Held::Rune(Rune::Thunder),
            Held::Rune(Rune::Bleed),
        ];
        match e.run_mut() {
            Run::V2 { state, .. } => state
                .players
                .career_mut(slot(player))
                .equipment
                .inventory
                .extend_from_slice(&items),
            Run::V1 { .. } => panic!("the live rules"),
        }
    }

    fn equipment_of(e: &Engine, player: u8) -> Equipment {
        match e.run() {
            Run::V2 { state, .. } => state.players.career(slot(player)).equipment.clone(),
            Run::V1 { .. } => panic!("the live rules"),
        }
    }

    fn started(lumine: bool, party: u8) -> Engine {
        let mut e = Engine::start_party(&equipment(lumine), party).unwrap();
        for p in 0..party {
            give(&mut e, p);
        }
        e
    }

    fn equip(item: u32, slot: u32) -> EquipAction {
        EquipAction::Equip { item, slot }
    }

    fn replace(item: u32, slot: u32) -> EquipAction {
        EquipAction::Replace { item, slot }
    }

    /// T20: `check_action` refuses each reason, and reads only.
    #[test]
    fn check_action_refuses_each_reason() {
        let mut e = started(false, 1);
        let me = PlayerIdx::SOLO;
        let hash = e.state_hash();
        let refused = |e: &Engine, a: EquipAction| e.check_action(me, a).unwrap_err();
        assert_eq!(refused(&e, equip(7, 5)), ActionError::NoItem { item: 7 });
        assert_eq!(refused(&e, equip(0, 12)), ActionError::NoSlot { slot: 12 });
        assert_eq!(refused(&e, equip(0, 13)), ActionError::NoSlot { slot: 13 });
        assert_eq!(
            refused(&e, replace(0, 5)),
            ActionError::SlotEmpty { slot: 5 }
        );
        assert_eq!(refused(&e, equip(2, 5)), ActionError::WrongSlot { slot: 5 });
        assert_eq!(refused(&e, equip(3, 0)), ActionError::WrongSlot { slot: 0 });
        assert_eq!(refused(&e, equip(0, 9)), ActionError::WrongSlot { slot: 9 });
        assert_eq!(refused(&e, equip(5, 0)), ActionError::WrongSlot { slot: 0 });
        assert_eq!(
            refused(&e, equip(4, 0)),
            ActionError::NotEquippable { item: 4 }
        );
        assert_eq!(refused(&e, equip(5, 10)), ActionError::RuneHeld { slot: 9 });
        assert_eq!(
            refused(&e, equip(6, 9)),
            ActionError::SlotFilled { slot: 9 }
        );
        assert_eq!(e.check_action(me, equip(0, 5)), Ok(()));
        assert_eq!(e.check_action(me, equip(6, 10)), Ok(()));
        for action in [
            EquipAction::Extract { item: 0 },
            EquipAction::Craft { star: 0 },
            EquipAction::TreePick { option: 0 },
        ] {
            assert_eq!(refused(&e, action), ActionError::NotBuilt, "{action:?}");
        }
        assert_eq!(e.state_hash(), hash, "the check changes nothing");

        e.act(me, equip(0, 5)).unwrap();
        assert_eq!(
            refused(&e, equip(0, 6)),
            ActionError::OptionHeld { slot: 5 }
        );
        assert_eq!(
            refused(&e, equip(0, 5)),
            ActionError::SlotFilled { slot: 5 }
        );

        // The Lumine slot is addressed one past the slots, when the run has one.
        let e = started(true, 1);
        assert_eq!(refused(&e, equip(0, 13)), ActionError::NoSlot { slot: 13 });
        assert_eq!(
            refused(&e, equip(2, 12)),
            ActionError::SlotFilled { slot: 12 }
        );
        assert_eq!(
            refused(&e, equip(0, 5)),
            ActionError::OptionHeld { slot: 12 }
        );
        assert_eq!(e.check_action(me, replace(1, 12)), Ok(()), "a switch");
    }

    /// T21: `act` applies what `check_action` accepts and changes nothing on a refusal.
    #[test]
    fn act_applies_what_check_action_accepts() {
        let mut e = started(false, 1);
        let me = PlayerIdx::SOLO;
        let hash = e.state_hash();
        for bad in [equip(99, 5), equip(0, 99), replace(0, 5), equip(2, 5)] {
            let wanted = e.check_action(me, bad);
            assert_eq!(e.act(me, bad), wanted);
            assert!(wanted.is_err());
            assert_eq!(e.state_hash(), hash, "{bad:?} changed nothing");
        }
        assert_eq!(
            e.act(slot(2), equip(0, 5)),
            Err(ActionError::NoPlayer { player: slot(2) })
        );
        assert_eq!(e.act(me, equip(0, 5)), Ok(()));
        assert_ne!(e.state_hash(), hash);
        let eq = equipment_of(&e, 0);
        assert_eq!(eq.inventory.len(), 6);
        assert!(eq.slots[5].held.is_some());

        // In a party only the acting slot changes.
        let mut e = started(false, 2);
        let (zero, one) = (equipment_of(&e, 0), equipment_of(&e, 1));
        assert_eq!(e.act(slot(1), equip(0, 5)), Ok(()));
        assert_eq!(equipment_of(&e, 0), zero);
        assert_ne!(equipment_of(&e, 1), one);

        // The step of actions applies each present slot's action in slot order, ignoring a
        // refused one.
        let mut e = started(false, 2);
        e.action_step(&[Some(equip(0, 5)), Some(equip(99, 5)), None]);
        assert!(equipment_of(&e, 0).slots[5].held.is_some());
        assert!(equipment_of(&e, 1).slots[5].held.is_none());
    }
}
