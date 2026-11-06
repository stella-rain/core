//! Tests for Starlight (ADR-SIM-3, ADR-SIM-5 point 8, ADR-SIM-7): the placeholder values, extract,
//! the boss bonus and craft. The expected values are worked out here, not copied from a run.

use super::equipment::{Equipment, Held};
use super::starlight::{
    self, BOSS_BONUS, CRAFT_PRICE, EXTRACT_VALUE, RUNE_EXTRACT, StarlightError, craft_price,
    extract_value,
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
    let rarities: Vec<Value> = ["common", "uncommon", "rare", "epic", "legendary", "supernova"]
        .iter()
        .enumerate()
        .map(|(i, r)| json!({ "rarity": r, "lumine_cost": i + 1 }))
        .collect();
    let mut list: Vec<Value> = ["common", "uncommon", "rare", "epic", "legendary", "supernova"]
        .iter()
        .map(|r| {
            json!({ "id": format!("s_{r}"), "rarity": r, "effect": "attack_pct",
                "amount": 1, "limit": 2 })
        })
        .collect();
    list.push(json!({ "id": "named", "rarity": "epic", "effect": "attack_pct", "amount": 1,
        "limit": 1, "named": true, "star": "Named" }));
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
    for w in EXTRACT_VALUE.windows(2) {
        assert!(w[0] < w[1]);
    }
    for w in CRAFT_PRICE.windows(2) {
        assert!(w[0] < w[1]);
    }
    assert!(EXTRACT_VALUE[0] > 0 && CRAFT_PRICE[0] > 0);
    for r in Rarity::ALL {
        let value = extract_value(&rw, Held::Star(u16::from(r.index())));
        assert!(craft_price(r) > value, "{r:?}");
    }
    assert!(RUNE_EXTRACT > 0);
    assert!(BOSS_BONUS > 0);
}

#[test]
fn extract_gives_the_rarity_s_value() {
    let rw = rewards();
    for r in Rarity::ALL {
        let i = usize::from(r.index());
        let mut e = with(&[Held::Star(u16::from(r.index()))]);
        assert_eq!(starlight::extract(&rw, &mut e, 0), Ok(()));
        assert_eq!(e.starlight, EXTRACT_VALUE[i]);
        assert!(e.inventory.is_empty());
    }
}

#[test]
fn extract_gives_a_rune_the_flat_value() {
    let rw = rewards();
    for rune in Rune::ALL {
        let mut e = with(&[Held::Rune(rune)]);
        starlight::extract(&rw, &mut e, 0).expect("a rune extracts");
        assert_eq!(e.starlight, RUNE_EXTRACT);
    }
}

#[test]
fn extract_removes_only_that_item_and_adds_up() {
    let rw = rewards();
    let rune = Held::Rune(Rune::ALL[0]);
    let mut e = with(&[Held::Star(COMMON), Held::Star(EPIC), rune]);
    starlight::extract(&rw, &mut e, 1).unwrap();
    assert_eq!(e.inventory, vec![Held::Star(COMMON), rune]);
    assert_eq!(e.starlight, EXTRACT_VALUE[3]);
    starlight::extract(&rw, &mut e, 1).unwrap();
    assert_eq!(e.inventory, vec![Held::Star(COMMON)]);
    assert_eq!(e.starlight, EXTRACT_VALUE[3] + RUNE_EXTRACT);
    assert!(e.slots.is_empty() && e.lumine.is_none() && e.tree_points == 0);
    // No overflow: the balance saturates.
    let mut e = with(&[Held::Star(COMMON)]);
    e.starlight = u32::MAX - 1;
    starlight::extract(&rw, &mut e, 0).unwrap();
    assert_eq!(e.starlight, u32::MAX);
}

#[test]
fn extract_refuses_a_place_past_the_inventory() {
    let rw = rewards();
    let mut e = with(&[Held::Star(COMMON)]);
    let before = e.clone();
    assert_eq!(
        starlight::extract(&rw, &mut e, 1),
        Err(StarlightError::NoItem { item: 1 })
    );
    assert_eq!(e, before);
    let mut e = Equipment::default();
    assert_eq!(
        starlight::check_extract(&e, 0),
        Err(StarlightError::NoItem { item: 0 })
    );
    assert_eq!(starlight::extract(&rw, &mut e, u32::MAX).is_err(), true);
}

#[test]
fn a_star_outside_the_file_extracts_for_nothing() {
    let rw = rewards();
    assert_eq!(extract_value(&rw, Held::Star(RANDOM_RUNE + 50)), 0);
    let _ = (RARE, NAMED);
}

// --- the boss bonus ----------------------------------------------------------------------------

mod boss {
    use super::super::equipment::Equipment;
    use super::super::party_fix_tests::{down, out, party, slot};
    use super::super::starlight::BOSS_BONUS;
    use super::super::state::State;
    use super::super::{World, stars};

    fn balances(s: &State) -> Vec<u32> {
        s.players
            .indices()
            .map(|me| s.players.career(me).equipment.starlight)
            .collect()
    }

    fn equipment_party(n: u8) -> (World, State) {
        let (mut w, s) = party(n);
        w.equipment_mode = true;
        (w, s)
    }

    #[test]
    fn a_boss_kill_gives_each_player_who_is_up_the_bonus() {
        let (w, mut s) = equipment_party(3);
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![BOSS_BONUS; 3]);
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![2 * BOSS_BONUS; 3]);
    }

    #[test]
    fn a_downed_or_out_player_gets_no_bonus() {
        let (w, mut s) = equipment_party(3);
        down(&mut s, 1);
        out(&mut s, 2);
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![BOSS_BONUS, 0, 0]);
        down(&mut s, 0);
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![BOSS_BONUS, 0, 0]);
    }

    #[test]
    fn the_bonus_does_not_need_a_drop_tier_and_offers_mode_gets_none() {
        let (mut w, mut s) = equipment_party(2);
        w.stage.stars.as_mut().expect("stars").tiers.clear();
        stars::boss_killed(&w, &mut s);
        assert_eq!(balances(&s), vec![BOSS_BONUS; 2]);
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
