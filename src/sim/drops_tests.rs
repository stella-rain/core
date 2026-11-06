//! Pins and tests for the drops of a star taken and of a boss kill (ADR-SIM-2 point 2, ADR-SIM-3
//! point 2, ADR-SIM-5 point 8, ADR-SIM-7). The offers-mode pins pass before and after the
//! equipment-mode rules land: the expected values are worked out here, not copied from a run.

use super::party_fix_tests::{party, slot};
use super::state::{StarState, State};
use super::{World, stars};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::rewards::Rarity;
use crate::snapshot::OfferKind;
use crate::stage::{RarityWeights, Tier};

/// Tiers whose highest rarity is `Rare` for every level; every rarity allowed.
fn tiered(n: u8) -> (World, State) {
    let (mut w, mut s) = party(n);
    let stars = w.stage.stars.as_mut().expect("stars");
    let weights = RarityWeights {
        rare: 1,
        ..RarityWeights::default()
    };
    stars.tiers = vec![Tier {
        from_level: 1,
        weights,
        random_rune_weight: None,
    }];
    s.stars.as_mut().expect("stars").rarity = (Rarity::Common, Rarity::Supernova);
    (w, s)
}

/// A star of `rarity` lying on slot `me`'s feet.
fn drop_star(s: &mut State, me: u8, rarity: Rarity) {
    let at = s.players[slot(me)].at;
    let id = EntityId(s.next_id);
    s.next_id += 1;
    s.stars
        .as_mut()
        .expect("stars")
        .on_map
        .push(StarState { id, at, rarity });
}

/// S1: in offers mode a star taken and a boss kill only queue offers; no inventory is touched and
/// no offer stream moves (ADR-SIM-2 point 2).
#[test]
fn offers_mode_picks_are_unchanged() {
    let (w, mut s) = tiered(3);
    drop_star(&mut s, 0, Rarity::Epic);
    let before = s.clone();
    let mut ev = Vec::new();
    stars::boss_killed(&w, &mut s);
    stars::pick_up(&w, &mut s, slot(0), &mut ev);
    for i in 0..3 {
        let (was, now) = (before.players.career(slot(i)), s.players.career(slot(i)));
        assert!(now.equipment.inventory.is_empty(), "slot {i}");
        assert_eq!(now.offer, was.offer, "slot {i} draws nothing");
        let due = &now.offers.as_ref().expect("offers").due;
        assert_eq!(due[0].kind, OfferKind::Boss, "slot {i}");
        assert_eq!(due[0].rarity, Rarity::Rare, "slot {i}");
        let taken = usize::from(i == 0);
        assert_eq!(due.len(), 1 + taken, "slot {i}");
    }
    let due = &s.players.career(slot(0)).offers.as_ref().unwrap().due;
    assert_eq!(due[1].kind, OfferKind::Star);
    assert_eq!(due[1].rarity, Rarity::Epic);
    assert!(s.stars.as_ref().unwrap().on_map.is_empty());
    assert!(matches!(ev[..], [DomainEvent::StarTaken { .. }]));
}
