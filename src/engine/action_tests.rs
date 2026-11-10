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
        assert_eq!(
            refused(&e, equip(0, 10)),
            ActionError::WrongSlot { slot: 10 }
        );
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
        // Extract, Craft and the tree pick are built (the tests below): no point is held yet.
        assert_eq!(
            refused(&e, EquipAction::TreePick { option: 0 }),
            ActionError::NoTreePoint
        );
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

    const FIVE: [EquipAction; 5] = [
        EquipAction::Equip { item: 0, slot: 5 },
        EquipAction::Replace { item: 0, slot: 5 },
        EquipAction::Extract { item: 0 },
        EquipAction::Craft { star: 0 },
        EquipAction::TreePick { option: 0 },
    ];

    fn hurt(e: &mut Engine, player: u8, out: bool) {
        if let Run::V2 { state, .. } = e.run_mut() {
            state.players[slot(player)].hp = 0;
            state.players[slot(player)].out = out;
        }
    }

    /// L1: a downed member's window is locked for every kind of action; an up member's is not.
    #[test]
    fn a_downed_member_is_refused_every_action() {
        let mut e = started(false, 2);
        hurt(&mut e, 1, false);
        for a in FIVE {
            assert_eq!(
                e.check_action(slot(1), a),
                Err(ActionError::Downed),
                "{a:?}"
            );
            assert_ne!(e.check_action(slot(0), a), Err(ActionError::Downed));
        }
        assert_eq!(e.check_action(slot(0), equip(0, 5)), Ok(()));
    }

    /// L2: the window opens again on the revive.
    #[test]
    fn the_window_opens_on_the_revive() {
        let mut e = started(false, 2);
        hurt(&mut e, 1, false);
        let hash = e.state_hash();
        assert_eq!(e.act(slot(1), equip(0, 5)), Err(ActionError::Downed));
        assert_eq!(e.state_hash(), hash, "a refusal changes nothing");
        let idle = Input::default();
        let hold = Input {
            interact: true,
            ..Input::default()
        };
        for _ in 0..200 {
            e.step(&[hold, idle, idle]);
            if e.check_action(slot(1), equip(0, 5)).is_ok() {
                break;
            }
        }
        assert_eq!(e.check_action(slot(1), equip(0, 5)), Ok(()));
        assert_eq!(e.act(slot(1), equip(0, 5)), Ok(()));
        assert_eq!(
            equipment_of(&e, 1).slots[5].held,
            Some(Held::Star(place(&e, "max_hp_common")))
        );
    }

    /// L3: an out member's window is locked too, with its own refusal.
    #[test]
    fn an_out_member_is_refused() {
        let mut e = started(false, 3);
        hurt(&mut e, 2, true);
        for a in FIVE {
            assert_eq!(e.check_action(slot(2), a), Err(ActionError::Out), "{a:?}");
        }
        let kept = equipment_of(&e, 2).inventory.len();
        assert_eq!(kept, 7, "it keeps what it holds");
    }

    /// L4: the order of the refusals is the player, then the lock, then the rule.
    #[test]
    fn the_refusal_order_is_player_then_lock_then_rule() {
        let mut e = started(false, 2);
        hurt(&mut e, 1, false);
        let bad = equip(99, 5);
        assert_eq!(
            e.check_action(slot(2), bad),
            Err(ActionError::NoPlayer { player: slot(2) })
        );
        assert_eq!(e.check_action(slot(1), bad), Err(ActionError::Downed));
        assert_eq!(
            e.check_action(slot(0), bad),
            Err(ActionError::NoItem { item: 99 })
        );
    }

    /// L5: a solo player is never downed: at 0 hit points the run is over.
    #[test]
    fn a_solo_player_is_never_downed() {
        let mut e = started(false, 1);
        assert_eq!(e.check_action(PlayerIdx::SOLO, equip(0, 5)), Ok(()));
        hurt(&mut e, 0, false);
        e.step(&[Input::default(); 3]);
        assert_ne!(e.outcome(), Outcome::Running);
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
    use crate::sim::starlight::{bundled_rules, craft_table, extract_table};

    fn set_starlight(e: &mut Engine, player: u8, n: u32) {
        match e.run_mut() {
            Run::V2 { state, .. } => {
                state.players.career_mut(slot(player)).equipment.starlight = n;
            }
            Run::V1 { .. } => panic!("the live rules"),
        }
    }

    fn pool(e: &Engine) -> Vec<u16> {
        match e.run() {
            Run::V2 { state, .. } => state.stars.as_ref().expect("stars").pool.clone(),
            Run::V1 { .. } => panic!("the live rules"),
        }
    }

    /// The index in the setup's star pool of the reward `id`.
    fn pool_index(e: &Engine, id: &str) -> u32 {
        let at = place(e, id);
        let i = pool(e).iter().position(|&p| p == at).expect("in the pool");
        u32::try_from(i).unwrap()
    }

    fn extract(item: u32) -> EquipAction {
        EquipAction::Extract { item }
    }

    fn craft(star: u32) -> EquipAction {
        EquipAction::Craft { star }
    }

    /// T13: `check_action` refuses each extract and craft reason, and reads only.
    #[test]
    fn check_action_extract_and_craft() {
        // The default pool leaves the random rune out (it has no category), so the pool names it.
        let mut setup = equipment(false);
        setup.star_pool = Some(vec!["max_hp_common".into(), "random_rune".into()]);
        let mut e = Engine::start_party(&setup, 1).unwrap();
        give(&mut e, 0);
        let me = PlayerIdx::SOLO;
        let hash = e.state_hash();
        let refused = |e: &Engine, a: EquipAction| e.check_action(me, a).unwrap_err();
        assert_eq!(refused(&e, extract(7)), ActionError::NoItem { item: 7 });
        assert_eq!(e.check_action(me, extract(0)), Ok(()));
        let n = u32::try_from(pool(&e).len()).unwrap();
        assert_eq!(refused(&e, craft(n)), ActionError::NoStar { star: n });
        let rune = pool_index(&e, "random_rune");
        assert_eq!(
            refused(&e, craft(rune)),
            ActionError::NotCraftable { star: rune }
        );
        let hp = pool_index(&e, "max_hp_common");
        assert_eq!(
            refused(&e, craft(hp)),
            ActionError::NotEnoughStarlight {
                need: craft_table()[0],
                have: 0
            }
        );
        assert_eq!(
            e.check_action(slot(2), extract(0)),
            Err(ActionError::NoPlayer { player: slot(2) })
        );
        assert_eq!(e.state_hash(), hash, "the checks change nothing");
        set_starlight(&mut e, 0, 1000);
        assert_eq!(e.check_action(me, craft(hp)), Ok(()));
        match e.run_mut() {
            Run::V2 { state, .. } => {
                let inv = &mut state.players.career_mut(me).equipment.inventory;
                inv.resize(crate::limits::MAX_INVENTORY, Held::Rune(Rune::Thunder));
            }
            Run::V1 { .. } => panic!("the live rules"),
        }
        assert_eq!(refused(&e, craft(hp)), ActionError::InventoryFull);
        assert_eq!(e.check_action(me, extract(0)), Ok(()));
    }

    /// T14: `act` makes an extract and a craft and changes nothing on a refusal.
    #[test]
    fn act_makes_extract_and_craft() {
        let mut e = started(false, 1);
        let me = PlayerIdx::SOLO;
        let hash = e.state_hash();
        assert_eq!(
            e.act(me, extract(99)),
            Err(ActionError::NoItem { item: 99 })
        );
        assert_eq!(e.state_hash(), hash);
        assert_eq!(e.act(me, extract(0)), Ok(()));
        let eq = equipment_of(&e, 0);
        assert_eq!(eq.inventory.len(), 6);
        assert_eq!(eq.starlight, extract_table()[0], "max_hp_common is Common");
        let hp = pool_index(&e, "max_hp_common");
        set_starlight(&mut e, 0, 1000);
        assert_eq!(e.act(me, craft(hp)), Ok(()));
        let eq = equipment_of(&e, 0);
        assert_eq!(
            eq.inventory.last(),
            Some(&Held::Star(place(&e, "max_hp_common")))
        );
        assert_eq!(eq.starlight, 1000 - craft_table()[0]);

        // A displaced star comes back as an inventory item and extracts.
        let mut e = started(false, 1);
        e.act(me, equip(0, 5)).unwrap();
        e.act(me, replace(0, 5)).unwrap();
        let eq = equipment_of(&e, 0);
        let hp = Held::Star(place(&e, "max_hp_common"));
        let at = eq
            .inventory
            .iter()
            .position(|h| *h == hp)
            .expect("displaced");
        let last = u32::try_from(at).unwrap();
        assert_eq!(e.act(me, extract(last)), Ok(()));
        assert_eq!(equipment_of(&e, 0).starlight, extract_table()[0]);

        // In a party only the acting slot changes.
        let mut e = started(false, 2);
        let zero = equipment_of(&e, 0);
        assert_eq!(e.act(slot(1), extract(0)), Ok(()));
        assert_eq!(equipment_of(&e, 0), zero);
        assert_eq!(equipment_of(&e, 1).starlight, extract_table()[0]);
    }

    /// T15: Starlight ends with the run: a new engine and a joiner start at 0.
    #[test]
    fn starlight_ends_with_the_run() {
        let mut e = started(false, 1);
        e.act(PlayerIdx::SOLO, extract(0)).unwrap();
        assert!(equipment_of(&e, 0).starlight > 0);
        let fresh = Engine::start(&equipment(false)).unwrap();
        assert_eq!(equipment_of(&fresh, 0).starlight, 0);
        set_starlight(&mut e, 0, 500);
        e.join(slot(1)).unwrap();
        assert_eq!(equipment_of(&e, 1).starlight, 0);
        assert_eq!(equipment_of(&e, 0).starlight, 500);
    }

    /// T17: each new refusal reads well in a replay error.
    #[test]
    fn the_refusals_read_well() {
        use crate::run_replay::RunReplayError;
        let text = |error| RunReplayError::ActionRefused { tick: 12, error }.to_string();
        assert_eq!(
            text(ActionError::NoStar { star: 4 }),
            "an equipment action on tick 12 crafts star 4, which is not in the star pool"
        );
        assert_eq!(
            text(ActionError::NotCraftable { star: 5 }),
            "an equipment action on tick 12 crafts star 5, which cannot be crafted"
        );
        assert_eq!(
            text(ActionError::AlreadyHeld { star: 6 }),
            "an equipment action on tick 12 crafts star 6, which the player already holds"
        );
        assert_eq!(
            text(ActionError::NotEnoughStarlight {
                need: 100,
                have: 30
            }),
            "an equipment action on tick 12 crafts a star for 100 Starlight, \
             but the player has 30"
        );
        assert_eq!(
            text(ActionError::InventoryFull),
            "an equipment action on tick 12 crafts a star into a full inventory"
        );
    }
    /// Gives `player` a star in the inventory, or in category slot 0, or in the Lumine slot.
    fn put(e: &mut Engine, player: u8, held: Held, into: &str) {
        match e.run_mut() {
            Run::V2 { state, .. } => {
                let eq = &mut state.players.career_mut(slot(player)).equipment;
                match into {
                    "inventory" => eq.inventory.push(held),
                    "slot" => eq.slots[0].held = Some(held),
                    "lumine" => eq.lumine.as_mut().expect("a Lumine slot").held = Some(held),
                    _ => panic!("{into}"),
                }
            }
            Run::V1 { .. } => panic!("the live rules"),
        }
    }

    /// A solo equipment run whose Lumine slot holds `lumine` (a bundled star) at `level` and
    /// `expertise`, with `items` as the whole inventory.
    fn lumine_run(lumine: &str, level: u8, expertise: u32, items: &[Held]) -> Engine {
        let mut e = Engine::start(&RunSetup {
            lumine: 10,
            lumine_star: Some(lumine.into()),
            ..equipment(false)
        })
        .unwrap();
        match e.run_mut() {
            Run::V2 { state, .. } => {
                let eq = &mut state.players.career_mut(PlayerIdx::SOLO).equipment;
                let slot = eq.lumine.as_mut().expect("a Lumine slot");
                (slot.level, slot.expertise) = (level, expertise);
                eq.inventory = items.to_vec();
            }
            Run::V1 { .. } => panic!("the live rules"),
        }
        e
    }

    fn place_of(id: &str) -> u16 {
        let rewards = crate::run_content::RunContent::bundled().rewards();
        u16::try_from(rewards.index_of(&id.into()).expect("bundled star")).unwrap()
    }

    fn lumine_of(e: &Engine) -> (Option<Held>, u8, u32) {
        let s = equipment_of(e, 0).lumine.expect("a Lumine slot");
        (s.held, s.level, s.expertise)
    }

    /// T5 (Lumine): a replace of the Lumine star by one that shares no option halves the level and
    /// resets the kills; a same-option replace keeps both; equip, rune and slot rules hold.
    #[test]
    fn a_non_switch_replace_of_the_lumine_star_halves_the_level() {
        let me = PlayerIdx::SOLO;
        let (mintaka, sirius) = (
            Held::Star(place_of("star_mintaka")),
            Held::Star(place_of("star_sirius")),
        );
        let items = [
            Held::Star(place_of("attack_common")),
            Held::Star(place_of("xp_common")),
            sirius,
            Held::Rune(Rune::Thunder),
        ];
        let mut e = lumine_run("star_mintaka", 6, 40, &items);
        assert_eq!(lumine_of(&e), (Some(mintaka), 6, 40));
        assert_eq!(e.act(me, replace(2, 12)), Ok(()));
        assert_eq!(lumine_of(&e), (Some(sirius), 3, 0));
        assert_eq!(equipment_of(&e, 0).inventory[2], mintaka);

        // The Lumine slot is filled, takes no rune, and is addressed one past the slots only.
        let e = lumine_run("star_mintaka", 6, 40, &items);
        let refused = |a| e.check_action(me, a).unwrap_err();
        assert_eq!(refused(equip(0, 12)), ActionError::SlotFilled { slot: 12 });
        assert_eq!(refused(replace(3, 12)), ActionError::WrongSlot { slot: 12 });
        assert_eq!(refused(replace(0, 13)), ActionError::NoSlot { slot: 13 });
        let plain = started(false, 1);
        assert_eq!(
            plain.check_action(me, replace(0, 12)).unwrap_err(),
            ActionError::NoSlot { slot: 12 }
        );

        // A same-option star of another grade is a switch: the level and the kills stay.
        let mut e = lumine_run(
            "max_hp_common",
            6,
            40,
            &[Held::Star(place_of("max_hp_rare"))],
        );
        assert_eq!(e.act(me, replace(0, 12)), Ok(()));
        assert_eq!(
            lumine_of(&e),
            (Some(Held::Star(place_of("max_hp_rare"))), 6, 40)
        );
    }

    /// T8 (Lumine): the displaced Lumine star extracts and crafts; the equipped one is never an
    /// item, and a craft of a star the Lumine slot holds is refused.
    #[test]
    fn the_displaced_lumine_star_is_extractable() {
        let me = PlayerIdx::SOLO;
        let items = [
            Held::Star(place_of("attack_common")),
            Held::Star(place_of("star_sirius")),
        ];
        let mut e = lumine_run("star_mintaka", 6, 40, &items);
        let mintaka_pool = pool_index(&e, "star_mintaka");
        set_starlight(&mut e, 0, 1000);
        assert_eq!(
            e.check_action(me, craft(mintaka_pool)),
            Err(ActionError::AlreadyHeld { star: mintaka_pool }),
            "the Lumine slot holds it"
        );
        assert_eq!(
            e.check_action(me, extract(2)),
            Err(ActionError::NoItem { item: 2 }),
            "the equipped star is no item"
        );
        e.act(me, replace(1, 12)).unwrap();
        assert_eq!(
            e.check_action(me, craft(mintaka_pool)),
            Err(ActionError::AlreadyHeld { star: mintaka_pool }),
            "now it sits in the inventory"
        );
        let before = lumine_of(&e);
        assert_eq!(e.act(me, extract(1)), Ok(()));
        let eq = equipment_of(&e, 0);
        assert_eq!(eq.starlight, 1000 + extract_table()[3], "epic");
        assert_eq!(lumine_of(&e), before, "the Lumine slot is untouched");
        let n = u32::try_from(eq.inventory.len()).unwrap();
        assert_eq!(
            e.check_action(me, extract(n)),
            Err(ActionError::NoItem { item: n })
        );
        assert_eq!(e.act(me, craft(mintaka_pool)), Ok(()));
        assert_eq!(
            equipment_of(&e, 0).inventory.last(),
            Some(&Held::Star(place_of("star_mintaka")))
        );
    }

    /// T18: the snapshot shows the balance and the prices; an offers run shows neither.
    #[test]
    fn the_snapshot_shows_the_balance_and_the_prices() {
        use crate::rewards::Rarity;
        let mut e = started(false, 1);
        e.act(PlayerIdx::SOLO, extract(0)).unwrap();
        let snap = e.snapshot();
        assert_eq!(snap.players[0].starlight, extract_table()[0]);
        let view = snap.starlight.as_ref().expect("equipment mode");
        assert_eq!(view.prices.len(), 6);
        for (i, (price, rarity)) in view.prices.iter().zip(Rarity::ALL).enumerate() {
            assert_eq!(price.rarity, rarity);
            assert_eq!(price.extract, extract_table()[i]);
            assert_eq!(price.craft, craft_table()[i]);
        }
        assert_eq!(view.rune_extract, bundled_rules().rune_extract);
        assert_eq!(view.boss_bonus, bundled_rules().boss_bonus);
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(
            json["players"][0]["starlight"],
            serde_json::json!(extract_table()[0])
        );
        assert!(json["starlight"]["prices"].is_array());
        // Offers mode: no key at all.
        let offers = Engine::start(&setup()).unwrap();
        let json = serde_json::to_value(offers.snapshot()).unwrap();
        assert!(json.get("starlight").is_none());
        assert!(json["players"][0].get("starlight").is_none());
        assert!(json["players"][0].get("pool_held").is_none());
    }

    /// T20: the snapshot marks which pool stars a player holds, by the test a craft uses.
    #[test]
    fn the_snapshot_marks_a_held_star() {
        let pooled = RunSetup {
            star_pool: Some(vec!["star_altair".into(), "max_hp_common".into()]),
            ..equipment(true)
        };
        let fresh = || Engine::start_party(&pooled, 2).unwrap();
        let held = |e: &Engine, p: usize| e.snapshot().players[p].pool_held.clone();
        let agree = |e: &mut Engine| {
            set_starlight(e, 0, 100_000);
            set_starlight(e, 1, 100_000);
            for (p, me) in [slot(0), slot(1)].into_iter().enumerate() {
                let marks = held(e, p);
                assert_eq!(marks.len(), 2);
                for (i, mark) in marks.iter().enumerate() {
                    let refused = e.check_action(me, craft(u32::try_from(i).unwrap()))
                        == Err(ActionError::AlreadyHeld {
                            star: u32::try_from(i).unwrap(),
                        });
                    assert_eq!(*mark, refused, "player {p} star {i}");
                }
            }
        };
        let e = fresh();
        let altair = Held::Star(place(&e, "star_altair"));
        let common = Held::Star(place(&e, "max_hp_common"));
        let mut e = fresh();
        assert_eq!((held(&e, 0), held(&e, 1)), (vec![false; 2], vec![false; 2]));
        agree(&mut e);
        for into in ["inventory", "slot", "lumine"] {
            let mut e = fresh();
            put(&mut e, 0, altair, into);
            assert_eq!(held(&e, 0), [true, false], "{into}");
            assert_eq!(held(&e, 1), [false, false], "{into}");
            agree(&mut e);
        }
        // An unnamed star held does not mark, and a craft of it is accepted.
        let mut e = fresh();
        put(&mut e, 0, common, "inventory");
        assert_eq!(held(&e, 0), [false, false]);
        agree(&mut e);
        assert_eq!(e.check_action(slot(0), craft(1)), Ok(()));
        // Extracting the named star clears the mark.
        let mut e = fresh();
        put(&mut e, 0, altair, "inventory");
        let at = u32::try_from(
            equipment_of(&e, 0)
                .inventory
                .iter()
                .position(|h| *h == altair)
                .unwrap(),
        )
        .unwrap();
        e.act(slot(0), extract(at)).unwrap();
        assert_eq!(held(&e, 0), [false, false]);
        agree(&mut e);
        assert_eq!(e.check_action(slot(0), craft(0)), Ok(()));
        let json = serde_json::to_value(e.snapshot()).unwrap();
        assert_eq!(
            json["players"][0]["pool_held"],
            serde_json::json!([false, false])
        );
    }
}

// --- the tree pick in an equipment run (ADR-SIM-7, ADR-REPL-5) ------------------------------------

mod tree_runs {
    use super::*;
    use crate::run_setup::BuildMode;

    fn equipment() -> RunSetup {
        RunSetup {
            build_mode: BuildMode::Equipment,
            ..setup()
        }
    }

    fn pick(option: u32) -> EquipAction {
        EquipAction::TreePick { option }
    }

    /// Gives player `p` the gauge level `level`, the levels passed.
    fn reach(e: &mut Engine, p: u8, level: u32) {
        let Run::V2 { state, .. } = e.run_mut() else {
            panic!("the live rules");
        };
        let g = state
            .players
            .career_mut(slot(p))
            .expertise
            .as_mut()
            .expect("a gauge");
        g.level = level;
    }

    /// T16: `check_action` for a tree pick: each refusal in its order, and reads only.
    #[test]
    fn check_action_tree_pick() {
        let mut e = Engine::start(&equipment()).unwrap();
        let hash = e.state_hash();
        assert_eq!(
            e.check_action(slot(1), pick(0)),
            Err(ActionError::NoPlayer { player: slot(1) })
        );
        assert_eq!(
            e.check_action(PlayerIdx::SOLO, pick(0)),
            Err(ActionError::NoTreePoint)
        );
        reach(&mut e, 0, 3);
        let hash_held = e.state_hash();
        assert_ne!(hash, hash_held);
        assert_eq!(e.check_action(PlayerIdx::SOLO, pick(2)), Ok(()));
        assert_eq!(
            e.check_action(PlayerIdx::SOLO, pick(3)),
            Err(ActionError::NoTreeOption {
                option: 3,
                options: 3
            })
        );
        assert_eq!(e.state_hash(), hash_held, "the check changes nothing");
        // An offers run and a sim_version 1 run have no tree.
        let offers = Engine::start(&setup()).unwrap();
        assert_eq!(
            offers.check_action(PlayerIdx::SOLO, pick(0)),
            Err(ActionError::NotBuilt)
        );
        let v1 = Engine::start(&RunSetup {
            schema_version: 1,
            ..setup()
        })
        .unwrap();
        assert_eq!(
            v1.check_action(PlayerIdx::SOLO, pick(0)),
            Err(ActionError::NotBuilt)
        );
    }

    /// T17: `act` makes the pick; a refused one changes nothing.
    #[test]
    fn act_makes_the_tree_pick() {
        let mut e = Engine::start(&equipment()).unwrap();
        reach(&mut e, 0, 3);
        let hash = e.state_hash();
        assert_eq!(
            e.act(PlayerIdx::SOLO, pick(5)),
            Err(ActionError::NoTreeOption {
                option: 5,
                options: 3
            })
        );
        assert_eq!(e.state_hash(), hash);
        assert!(e.snapshot().expertise.expect("a gauge").style.is_none());
        e.act(PlayerIdx::SOLO, pick(1)).unwrap();
        let view = e.snapshot().expertise.expect("a gauge");
        assert_eq!(view.style.as_ref().map(|s| s.as_str()), Some("tempo"));
        assert_ne!(e.state_hash(), hash);
        assert_eq!(
            e.act(PlayerIdx::SOLO, pick(0)),
            Err(ActionError::NoTreePoint)
        );
        reach(&mut e, 0, 6);
        e.act(PlayerIdx::SOLO, pick(0)).unwrap();
        assert_eq!(e.snapshot().expertise.expect("a gauge").path.len(), 1);
    }

    /// T19: the refusals read well in a replay error.
    #[test]
    fn the_tree_refusals_read_well() {
        use crate::run_replay::RunReplayError;
        let text = |error| RunReplayError::ActionRefused { tick: 12, error }.to_string();
        assert_eq!(
            text(ActionError::NoTreePoint),
            "an equipment action on tick 12 takes a tree pick, but no tree point is held"
        );
        assert_eq!(
            text(ActionError::NoTreeOption {
                option: 7,
                options: 3
            }),
            "an equipment action on tick 12 takes tree option 7, but the pick has 3 options"
        );
    }

    /// T20: a point waiting in one slot does not stop a party.
    #[test]
    fn a_waiting_point_does_not_stop_a_party() {
        let mut e = Engine::start_party(&equipment(), 2).unwrap();
        reach(&mut e, 0, 3);
        let tick = e.tick();
        e.step(&[Input::default(); 3]);
        assert_eq!(e.tick(), tick + 1);
        assert!(!e.offer_open());
        assert_eq!(
            e.check_action(slot(1), pick(0)),
            Err(ActionError::NoTreePoint)
        );
        assert_eq!(e.check_action(slot(0), pick(0)), Ok(()));
    }

    /// T21: the snapshot shows each player's own tree point, and drops it when it is taken.
    #[test]
    fn the_snapshot_shows_tree_ready() {
        use crate::snapshot::{ExpertiseOfferKind, TreeReadyView};
        let mut e = Engine::start_party(&equipment(), 2).unwrap();
        assert_eq!(e.snapshot().players[0].tree_ready, None);
        reach(&mut e, 0, 3);
        let want = TreeReadyView {
            kind: ExpertiseOfferKind::Style,
            level: 3,
            held: 1,
            options: vec!["edge".into(), "tempo".into(), "reach".into()],
        };
        let snap = e.snapshot();
        assert_eq!(snap.players[0].tree_ready.as_ref(), Some(&want));
        assert_eq!(snap.players[1].tree_ready, None, "each player's own");
        let json = serde_json::to_value(&snap.players[0]).unwrap();
        assert_eq!(
            json["tree_ready"],
            serde_json::json!({ "kind": "style", "level": 3, "held": 1,
                "options": ["edge", "tempo", "reach"] })
        );
        e.act(PlayerIdx::SOLO, pick(0)).unwrap();
        let snap = e.snapshot();
        assert_eq!(snap.players[0].tree_ready, None);
        assert!(
            serde_json::to_value(&snap.players[0])
                .unwrap()
                .get("tree_ready")
                .is_none()
        );
        // An offers run has none.
        let mut offers = Engine::start(&setup()).unwrap();
        reach(&mut offers, 0, 3);
        let json = serde_json::to_value(&offers.snapshot().players[0]).unwrap();
        assert!(json.get("tree_ready").is_none());
    }
}
