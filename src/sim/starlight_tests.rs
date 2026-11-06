//! Tests for Starlight (ADR-SIM-3, ADR-SIM-5 point 8, ADR-SIM-7): the placeholder values, extract,
//! the boss bonus and craft. The expected values are worked out here, not copied from a run.

use super::equipment::{Equipment, Held};
use super::starlight::{
    self, StarlightError, bundled_rules, craft_price, craft_table, extract_table, extract_value,
};
use crate::rewards::{Rarity, Rewards};
use crate::rune::Rune;
use crate::version::Versions;
use serde_json::{Value, json};

/// Places 0 to 5: one unnamed star of each rarity, in `Rarity::ALL` order.
const COMMON: u16 = 0;
const RARE: u16 = 2;
const EPIC: u16 = 3;
/// Place 6: a named epic star. Place 7: the lone random rune.
const NAMED: u16 = 6;
const RANDOM_RUNE: u16 = 7;

fn rewards() -> Rewards {
    let rarities: Vec<Value> = [
        "common",
        "uncommon",
        "rare",
        "epic",
        "legendary",
        "supernova",
    ]
    .iter()
    .enumerate()
    .map(|(i, r)| json!({ "rarity": r, "lumine_cost": i + 1 }))
    .collect();
    let mut list: Vec<Value> = [
        "common",
        "uncommon",
        "rare",
        "epic",
        "legendary",
        "supernova",
    ]
    .iter()
    .map(|r| {
        json!({ "id": format!("s_{r}"), "rarity": r, "effect": "attack_pct",
                "amount": 1, "limit": 2 })
    })
    .collect();
    list.push(
        json!({ "id": "named", "rarity": "epic", "effect": "attack_pct", "amount": 1,
        "limit": 1, "named": true, "star": "Named" }),
    );
    list.push(json!({ "id": "rune", "rarity": "common", "effect": "random_rune" }));
    let file = json!({ "rarities": rarities, "rewards": list });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the rewards validate")
}

fn with(items: &[Held]) -> Equipment {
    Equipment {
        inventory: items.to_vec(),
        ..Equipment::default()
    }
}

#[test]
fn the_prices_rise_with_rarity() {
    let rw = rewards();
    for w in extract_table().windows(2) {
        assert!(w[0] < w[1]);
    }
    for w in craft_table().windows(2) {
        assert!(w[0] < w[1]);
    }
    assert!(extract_table()[0] > 0 && craft_table()[0] > 0);
    for r in Rarity::ALL {
        let value = extract_value(bundled_rules(), &rw, Held::Star(u16::from(r.index())));
        assert!(craft_price(bundled_rules(), r) > value, "{r:?}");
    }
    assert!(bundled_rules().rune_extract > 0 && bundled_rules().boss_bonus > 0);
}

#[test]
fn extract_gives_the_rarity_s_value() {
    let rw = rewards();
    for r in Rarity::ALL {
        let i = usize::from(r.index());
        let mut e = with(&[Held::Star(u16::from(r.index()))]);
        assert_eq!(starlight::extract(bundled_rules(), &rw, &mut e, 0), Ok(()));
        assert_eq!(e.starlight, extract_table()[i]);
        assert!(e.inventory.is_empty());
    }
}

#[test]
fn extract_gives_a_rune_the_flat_value() {
    let rw = rewards();
    for rune in Rune::ALL {
        let mut e = with(&[Held::Rune(rune)]);
        starlight::extract(bundled_rules(), &rw, &mut e, 0).expect("a rune extracts");
        assert_eq!(e.starlight, bundled_rules().rune_extract);
    }
}

#[test]
fn extract_removes_only_that_item_and_adds_up() {
    let rw = rewards();
    let rune = Held::Rune(Rune::ALL[0]);
    let mut e = with(&[Held::Star(COMMON), Held::Star(EPIC), rune]);
    starlight::extract(bundled_rules(), &rw, &mut e, 1).unwrap();
    assert_eq!(e.inventory, vec![Held::Star(COMMON), rune]);
    assert_eq!(e.starlight, extract_table()[3]);
    starlight::extract(bundled_rules(), &rw, &mut e, 1).unwrap();
    assert_eq!(e.inventory, vec![Held::Star(COMMON)]);
    assert_eq!(
        e.starlight,
        extract_table()[3] + bundled_rules().rune_extract
    );
    assert!(e.slots.is_empty() && e.lumine.is_none() && e.tree_points == 0);
    // No overflow: the balance saturates.
    let mut e = with(&[Held::Star(COMMON)]);
    e.starlight = u32::MAX - 1;
    starlight::extract(bundled_rules(), &rw, &mut e, 0).unwrap();
    assert_eq!(e.starlight, u32::MAX);
}

#[test]
fn extract_refuses_a_place_past_the_inventory() {
    let rw = rewards();
    let mut e = with(&[Held::Star(COMMON)]);
    let before = e.clone();
    assert_eq!(
        starlight::extract(bundled_rules(), &rw, &mut e, 1),
        Err(StarlightError::NoItem { item: 1 })
    );
    assert_eq!(e, before);
    let mut e = Equipment::default();
    assert_eq!(
        starlight::check_extract(&e, 0),
        Err(StarlightError::NoItem { item: 0 })
    );
    assert!(starlight::extract(bundled_rules(), &rw, &mut e, u32::MAX).is_err());
}

#[test]
fn a_star_outside_the_file_extracts_for_nothing() {
    let rw = rewards();
    assert_eq!(
        extract_value(bundled_rules(), &rw, Held::Star(RANDOM_RUNE + 50)),
        0
    );
}

// --- craft -------------------------------------------------------------------------------------

fn rich(starlight: u32) -> Equipment {
    Equipment {
        starlight,
        ..Equipment::default()
    }
}

#[test]
fn craft_spends_the_price_and_adds_the_star() {
    let rw = rewards();
    let pool = [COMMON, RARE];
    let mut e = rich(100);
    e.inventory.push(Held::Star(EPIC));
    starlight::craft(bundled_rules(), &rw, &pool, &mut e, 1).expect("a craft");
    assert_eq!(e.starlight, 100 - craft_table()[2]);
    assert_eq!(e.inventory, vec![Held::Star(EPIC), Held::Star(RARE)]);
    // An unnamed star twice gives two copies.
    let mut e = rich(100);
    starlight::craft(bundled_rules(), &rw, &pool, &mut e, 0).unwrap();
    starlight::craft(bundled_rules(), &rw, &pool, &mut e, 0).unwrap();
    assert_eq!(e.inventory, vec![Held::Star(COMMON); 2]);
    assert_eq!(e.starlight, 100 - 2 * craft_table()[0]);
}

#[test]
fn craft_refuses_what_it_cannot_take() {
    let rw = rewards();
    let pool = [COMMON, RARE, 99, RANDOM_RUNE];
    let refused = |e: &Equipment, star: u32, want: StarlightError| {
        let mut e2 = e.clone();
        assert_eq!(
            starlight::check_craft(bundled_rules(), &rw, &pool, e, star),
            Err(want)
        );
        assert_eq!(
            starlight::craft(bundled_rules(), &rw, &pool, &mut e2, star),
            Err(want)
        );
        assert_eq!(&e2, e);
    };
    let e = rich(10_000);
    refused(&e, 4, StarlightError::NoStar { star: 4 });
    refused(&e, u32::MAX, StarlightError::NoStar { star: u32::MAX });
    refused(&e, 2, StarlightError::NoStar { star: 2 });
    refused(&e, 3, StarlightError::NotCraftable { star: 3 });
    let e = rich(craft_table()[2] - 1);
    refused(
        &e,
        1,
        StarlightError::NotEnoughStarlight {
            need: craft_table()[2],
            have: craft_table()[2] - 1,
        },
    );
    let mut e = rich(craft_table()[2]);
    starlight::craft(bundled_rules(), &rw, &pool, &mut e, 1).expect("exactly the price");
    assert_eq!(e.starlight, 0);
    let mut full = rich(10_000);
    full.inventory = vec![Held::Star(COMMON); crate::limits::MAX_INVENTORY];
    refused(&full, 0, StarlightError::InventoryFull);
    starlight::extract(bundled_rules(), &rw, &mut full, 0).unwrap();
    starlight::craft(bundled_rules(), &rw, &pool, &mut full, 0).expect("room again");
}

#[test]
fn craft_of_a_held_named_star_is_refused() {
    use super::equipment::{EquipSlot, SlotKind};
    let rw = rewards();
    let pool = [NAMED, COMMON];
    let slot = |held| EquipSlot {
        kind: SlotKind::Category(0),
        held: Some(held),
        level: 1,
        expertise: 0,
    };
    let already = StarlightError::AlreadyHeld { star: 0 };
    let mut e = rich(10_000);
    assert!(!starlight::holds_named(&rw, &e, NAMED));
    starlight::check_craft(bundled_rules(), &rw, &pool, &e, 0).expect("not held: craftable");
    e.inventory.push(Held::Star(NAMED));
    assert_eq!(
        starlight::check_craft(bundled_rules(), &rw, &pool, &e, 0),
        Err(already)
    );
    assert!(starlight::holds_named(&rw, &e, NAMED));
    let mut e = rich(10_000);
    e.slots.push(slot(Held::Star(NAMED)));
    assert_eq!(
        starlight::check_craft(bundled_rules(), &rw, &pool, &e, 0),
        Err(already)
    );
    let mut e = rich(10_000);
    e.lumine = Some(EquipSlot {
        kind: SlotKind::Lumine,
        ..slot(Held::Star(NAMED))
    });
    assert_eq!(
        starlight::check_craft(bundled_rules(), &rw, &pool, &e, 0),
        Err(already)
    );
    // An unnamed star crafts whether held or not.
    let mut e = rich(10_000);
    e.inventory.push(Held::Star(COMMON));
    e.slots.push(slot(Held::Star(COMMON)));
    assert!(!starlight::holds_named(&rw, &e, COMMON));
    starlight::check_craft(bundled_rules(), &rw, &pool, &e, 1).expect("an unnamed star");
}

#[test]
fn craft_and_extract_draw_nothing_and_allocate_nothing() {
    use super::party_fix_tests::{party, slot};
    use super::{check_craft, craft_action, extract_action};
    let rw = rewards();
    let (_, mut s) = party(1);
    let me = slot(0);
    s.stars.as_mut().expect("stars").pool = vec![COMMON, RARE];
    {
        let e = &mut s.players.career_mut(me).equipment;
        e.inventory.reserve(crate::limits::MAX_INVENTORY);
        e.inventory.push(Held::Star(EPIC));
    }
    let offer = s.players.career(me).offer.clone();
    let cap = s.players.career(me).equipment.inventory.capacity();
    extract_action(bundled_rules(), &rw, &mut s, me, 0).expect("extract");
    assert_eq!(
        check_craft(bundled_rules(), &rw, &s, me, 1),
        Err(StarlightError::NotEnoughStarlight {
            need: craft_table()[2],
            have: extract_table()[3],
        })
    );
    s.players.career_mut(me).equipment.starlight = 1000;
    craft_action(bundled_rules(), &rw, &mut s, me, 1).expect("craft");
    assert_eq!(s.players.career(me).offer, offer);
    assert_eq!(s.players.career(me).equipment.inventory.capacity(), cap);
}

/// T8: the Starlight values are the pack's: a pack that says 3, 9 and 4 gives and takes those.
#[test]
fn the_values_are_the_rules() {
    use crate::rewards::RarityStarlight;
    let rw = rewards();
    let mut r = bundled_rules().clone();
    r.rarities = Rarity::ALL
        .into_iter()
        .map(|rarity| RarityStarlight {
            rarity,
            extract: 3,
            craft_price: 9,
        })
        .collect();
    r.rune_extract = 4;
    assert_eq!(extract_value(&r, &rw, Held::Star(0)), 3);
    assert_eq!(extract_value(&r, &rw, Held::Rune(Rune::Thunder)), 4);
    for rarity in Rarity::ALL {
        assert_eq!(craft_price(&r, rarity), 9);
    }
}

// --- the boss bonus ----------------------------------------------------------------------------

mod boss {
    use super::super::equipment::Equipment;
    use super::super::party_fix_tests::{down, out, party, slot};
    use super::super::starlight::bundled_rules;
    use super::super::state::State;
    use super::super::{World, stars};

    fn balances(s: &State) -> Vec<u32> {
        s.players
            .indices()
            .map(|me| s.players.career(me).equipment.starlight)
            .collect()
    }

    /// T8: the boss bonus is the pack's: 7 pays 7.
    #[test]
    fn the_bonus_is_the_rules() {
        let (mut w, mut s) = equipment_party(2);
        w.equipment_rules.boss_bonus = 7;
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![7; 2]);
    }

    fn equipment_party(n: u8) -> (World, State) {
        let (mut w, s) = party(n);
        w.equipment_mode = true;
        w.equipment_rules = bundled_rules().clone();
        (w, s)
    }

    #[test]
    fn a_boss_kill_gives_each_player_who_is_up_the_bonus() {
        let (w, mut s) = equipment_party(3);
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![bundled_rules().boss_bonus; 3]);
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![2 * bundled_rules().boss_bonus; 3]);
    }

    #[test]
    fn a_downed_or_out_player_gets_no_bonus() {
        let (w, mut s) = equipment_party(3);
        down(&mut s, 1);
        out(&mut s, 2);
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![bundled_rules().boss_bonus, 0, 0]);
        down(&mut s, 0);
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![bundled_rules().boss_bonus, 0, 0]);
    }

    #[test]
    fn the_bonus_does_not_need_a_drop_tier_and_offers_mode_gets_none() {
        let (mut w, mut s) = equipment_party(2);
        w.stage.stars.as_mut().expect("stars").tiers.clear();
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![bundled_rules().boss_bonus; 2]);
        // Saturates at the top.
        s.players.career_mut(slot(0)).equipment.starlight = u32::MAX - 1;
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s)[0], u32::MAX);
        // An offers run pays nothing and keeps the equipment empty.
        let (w, mut s) = party(2);
        stars::boss_killed(&w, &mut s);
        for me in s.players.indices() {
            assert_eq!(s.players.career(me).equipment, Equipment::default());
        }
    }
}
