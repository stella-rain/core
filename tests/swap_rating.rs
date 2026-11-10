//! `Engine::swap_ratings`: a star rated against each slot it could go in, with the rules' own
//! equip and replace run on a clone (ADR-SIM-7, ADR-REPL-5, ADR-BRDG-4). A read-only query: nothing
//! the hash, the snapshot or a savepoint covers moves (ADR-SIM-3).

use crate::common::run::trial_setup;
use crate::run_play::PartyCase;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_replay::RunRecorder;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::{HeldView, SwapItem, SwapRating};

/// The slots of the layout, and the Lumine slot after them.
const SLOTS: u32 = 13;

fn case(seed: u64, ticks: u32) -> PartyCase {
    PartyCase {
        name: "swap_rating",
        setup: RunSetup {
            mode: "tutorial".into(),
            seed,
            ..trial_setup()
        },
        party: 1,
        ticks,
        changes: vec![],
        bot: None,
        script: None,
    }
}

fn items(e: &Engine) -> u32 {
    u32::try_from(e.snapshot().players[0].inventory.len()).unwrap()
}

fn acting(a: EquipAction) -> [Input; 3] {
    [
        Input {
            action: Some(a),
            ..Input::default()
        },
        Input::default(),
        Input::default(),
    ]
}

/// A recording whose inventory holds a star that goes into a slot, and one more item after the
/// first equip, so that a replace is possible too.
fn recorded() -> RunRecorder {
    for seed in 1..=60u64 {
        let rec = case(seed, 3000).play();
        if rec.engine().outcome() != Outcome::Running {
            continue;
        }
        let e = rec.engine();
        let accepted = |i: u32| {
            (0..SLOTS).any(|s| {
                let a = EquipAction::Equip { item: i, slot: s };
                e.check_action(PlayerIdx::SOLO, a).is_ok()
            })
        };
        if (0..items(e)).any(accepted) {
            return rec;
        }
    }
    panic!("no seed of 1..=60 drops an item that equips");
}

fn is_star(e: &Engine, i: u32) -> bool {
    matches!(
        e.snapshot().players[0].inventory[i as usize],
        HeldView::Star { .. }
    )
}

/// T11: a query changes nothing the engine keeps or shows.
#[test]
fn rating_every_item_changes_nothing() {
    let rec = recorded();
    let e = rec.engine();
    let (hash, snapshot, saved) = (e.state_hash(), e.snapshot(), rec.save());
    for i in 0..items(e) {
        let first = e.swap_ratings(PlayerIdx::SOLO, SwapItem::Inventory(i));
        assert_eq!(
            e.swap_ratings(PlayerIdx::SOLO, SwapItem::Inventory(i)),
            first
        );
    }
    assert_eq!(e.state_hash(), hash);
    assert_eq!(e.snapshot(), snapshot);
    assert_eq!(rec.save(), saved);
}

/// T12: nothing to rate gives an empty list.
#[test]
fn the_empty_cases() {
    // An offers run has no slots.
    let offers = Engine::start(&RunSetup::default()).unwrap();
    assert!(
        offers
            .swap_ratings(PlayerIdx::SOLO, SwapItem::Inventory(0))
            .is_empty()
    );
    assert!(
        offers
            .swap_ratings(PlayerIdx::SOLO, SwapItem::Star("nope".into()))
            .is_empty()
    );
    // An equipment run with nothing in the inventory, and a player outside the party.
    let fresh = Engine::start(&RunSetup {
        ..RunSetup::default()
    })
    .unwrap();
    assert!(
        fresh
            .swap_ratings(PlayerIdx::SOLO, SwapItem::Inventory(0))
            .is_empty()
    );
    let rec = recorded();
    let other = PlayerIdx::new(1).unwrap();
    assert!(
        rec.engine()
            .swap_ratings(other, SwapItem::Inventory(0))
            .is_empty()
    );
    let past = items(rec.engine());
    assert!(
        rec.engine()
            .swap_ratings(PlayerIdx::SOLO, SwapItem::Inventory(past))
            .is_empty()
    );
}

/// T13: the slots rated are the slots the action API accepts, and the first of them, made for
/// real, puts the star where the rating said.
#[test]
fn the_rated_slots_are_the_slots_the_actions_accept() {
    let rec = recorded();
    let e = rec.engine();
    let mut first = None;
    for i in (0..items(e)).filter(|&i| is_star(e, i)) {
        let rated: Vec<u32> = e
            .swap_ratings(PlayerIdx::SOLO, SwapItem::Inventory(i))
            .iter()
            .map(|r| r.slot)
            .collect();
        let accepted: Vec<u32> = (0..SLOTS)
            .filter(|&slot| {
                [
                    EquipAction::Equip { item: i, slot },
                    EquipAction::Replace { item: i, slot },
                ]
                .iter()
                .any(|a| e.check_action(PlayerIdx::SOLO, *a).is_ok())
            })
            .collect();
        assert_eq!(rated, accepted, "item {i}");
        if first.is_none() && !accepted.is_empty() {
            first = Some((i, accepted[0]));
        }
    }
    let (item, slot) = first.expect("an item that goes somewhere");
    let want = e.snapshot().players[0].inventory[item as usize].clone();
    let before = e.snapshot();
    let mut probe = rec.clone();
    let action = if before.players[0]
        .slots
        .get(slot as usize)
        .is_some_and(|s| s.held.is_some())
    {
        EquipAction::Replace { item, slot }
    } else {
        EquipAction::Equip { item, slot }
    };
    probe.step(&acting(action)).unwrap();
    let held = probe.engine().snapshot().players[0]
        .slots
        .get(slot as usize)
        .and_then(|s| s.held.clone());
    assert_eq!(held, Some(want));
    assert_eq!(e.snapshot(), before, "the original is unchanged");
}

/// T14: the slots come in ascending order, and an equip's level before is what the slot has.
#[test]
fn slots_ascend_and_an_equip_has_no_level_before() {
    let rec = recorded();
    let e = rec.engine();
    let snapshot = e.snapshot();
    for i in (0..items(e)).filter(|&i| is_star(e, i)) {
        let r: Vec<SwapRating> = e.swap_ratings(PlayerIdx::SOLO, SwapItem::Inventory(i));
        assert!(r.windows(2).all(|w| w[0].slot < w[1].slot));
        for x in &r {
            let slot = snapshot.players[0].slots.get(x.slot as usize);
            assert_eq!(x.replaces, slot.is_some_and(|s| s.held.is_some()));
            if let Some(s) = slot {
                assert_eq!(x.level_before, s.level);
            }
        }
    }
}
