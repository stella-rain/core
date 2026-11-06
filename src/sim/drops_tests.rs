//! Pins and tests for the drops of a star taken and of a boss kill (ADR-SIM-2 point 2, ADR-SIM-3
//! point 2, ADR-SIM-5 point 8, ADR-SIM-7). The offers-mode pins pass before and after the
//! equipment-mode rules land: the expected values are worked out here, not copied from a run.

use super::drops::{self, DropTable, RUNE_SHARE_PERMILLE};
use super::equipment::Held;
use super::party_fix_tests::{party, slot};
use super::state::{Career, StarField, StarState, State};
use super::{World, stars};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::limits::MAX_INVENTORY;
use crate::rewards::{Rarity, Rewards};
use crate::rng::SplitMix64;
use crate::rune::Rune;
use crate::snapshot::OfferKind;
use crate::stage::{RarityWeights, Tier};
use crate::version::Versions;
use serde_json::{Value, json};

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

/// T2: the mode is a fact of the setup, and a restore rebuilds it.
#[test]
fn the_mode_is_set_by_the_setup() {
    use crate::engine::{Engine, Run};
    use crate::run_setup::{BuildMode, RunSetup};
    let live = |e: &Engine| match e.run() {
        Run::V2 { world, .. } => world.equipment_mode,
        Run::V1 { .. } => panic!("the live rules"),
    };
    let equipment = RunSetup {
        build_mode: BuildMode::Equipment,
        ..RunSetup::default()
    };
    assert!(live(&Engine::start(&equipment).expect("starts")));
    assert!(!live(&Engine::start(&RunSetup::default()).expect("starts")));
}

// --- the roll (ADR-SIM-2 point 2, ADR-SIM-5 point 8) -------------------------------------------

/// One reward of the hand-built file: id, rarity, whether it is a named star, draw weight.
type Spec = (&'static str, &'static str, bool, u16);

fn rewards(list: &[Spec], with_rune: bool) -> Rewards {
    let rarities: Vec<Value> = [
        ("common", 1),
        ("uncommon", 2),
        ("rare", 3),
        ("epic", 4),
        ("legendary", 6),
        ("supernova", 10),
    ]
    .iter()
    .map(|(r, c)| json!({ "rarity": r, "lumine_cost": c }))
    .collect();
    let mut rewards: Vec<Value> = list
        .iter()
        .map(|&(id, rarity, named, weight)| {
            let mut r = json!({ "id": id, "rarity": rarity, "effect": "attack_pct",
                "amount": 1, "limit": if named { 1 } else { 2 }, "weight": weight });
            if named {
                r["named"] = json!(true);
                r["star"] = json!(format!("Star {id}"));
            }
            r
        })
        .collect();
    if with_rune {
        rewards.push(json!({ "id": "rune", "rarity": "common", "effect": "random_rune" }));
    }
    let file = json!({ "rarities": rarities, "rewards": rewards });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the rewards validate")
}

fn field(places: u16, rune_pool: u8) -> StarField {
    StarField {
        misses: 0,
        on_map: Vec::new(),
        rarity: (Rarity::Common, Rarity::Supernova),
        rune_pool,
        pool: (0..places).collect(),
    }
}

fn career(seed: u64) -> Career {
    let mut c = Career::default();
    c.offer = SplitMix64::new(seed);
    c.equipment.inventory = Vec::with_capacity(MAX_INVENTORY);
    c
}

/// The stream after `n` draws of any bound.
fn after(rng: &SplitMix64, n: u32) -> SplitMix64 {
    let mut c = rng.clone();
    for _ in 0..n {
        c.below(2);
    }
    c
}

/// One roll; what it added, taken off again so the next roll sees the same inventory.
fn rolled(
    table: &DropTable,
    f: &StarField,
    c: &mut Career,
    share: u32,
    pct: u32,
    rarity: Rarity,
) -> Option<Held> {
    let before = c.equipment.inventory.len();
    drops::roll(table, f, c, share, pct, rarity);
    (c.equipment.inventory.len() > before).then(|| c.equipment.inventory.pop().expect("one"))
}

const FULL: u8 = crate::sim::runes::FULL_POOL;

/// What the stream gives a roll, replayed by hand from the weights: the kind first (a rune below
/// the share), then the rune's `below(n)` or the star's `below(total)` (ADR-SIM-2 point 2).
fn expected(rng: &SplitMix64, runes: &[Rune], places: &[(u16, u32)]) -> Held {
    let mut r = rng.clone();
    if r.below(1000) < RUNE_SHARE_PERMILLE {
        let n = u32::try_from(runes.len()).expect("few");
        return Held::Rune(runes[usize::try_from(r.below(n)).expect("small")]);
    }
    let total: u32 = places.iter().map(|p| p.1).sum();
    let mut left = r.below(total);
    for &(place, weight) in places {
        if left < weight {
            return Held::Star(place);
        }
        left -= weight;
    }
    unreachable!("below the total")
}

/// T3: a star taken gives every member that is not out a roll of their own, in slot order, and
/// nobody an offer.
#[test]
fn a_star_taken_gives_every_member_a_roll_in_slot_order() {
    let (mut w, mut s) = tiered(3);
    let rw = rewards(
        &[
            ("a", "rare", false, 10),
            ("b", "rare", false, 20),
            ("c", "rare", false, 30),
        ],
        false,
    );
    w.drops = DropTable::of(&rw);
    w.equipment_mode = true;
    s.stars.as_mut().unwrap().pool = vec![0, 1, 2];
    s.stars.as_mut().unwrap().rune_pool = FULL;
    drop_star(&mut s, 0, Rarity::Rare);
    let before = s.clone();
    let mut ev = Vec::new();
    stars::pick_up(&w, &mut s, slot(0), &mut ev);
    assert_eq!(ev.len(), 1, "StarTaken once: {ev:?}");
    assert!(matches!(ev[0], DomainEvent::StarTaken { .. }));
    for i in 0..3 {
        let (was, now) = (before.players.career(slot(i)), s.players.career(slot(i)));
        assert!(
            now.offers.as_ref().unwrap().due.is_empty(),
            "slot {i} offered nothing"
        );
        let want = expected(&was.offer, &Rune::ALL, &[(0, 10), (1, 20), (2, 30)]);
        assert_eq!(now.equipment.inventory, vec![want], "slot {i}");
        assert_eq!(
            now.offer.state(),
            after(&was.offer, 2).state(),
            "slot {i} drew twice"
        );
    }
    assert_eq!(
        s.rng, before.rng,
        "the loot stream is not touched by a roll"
    );
}

/// T4: the draws of a roll, from its own stream only.
#[test]
fn a_roll_draws_the_pinned_number_from_its_own_stream() {
    let rw = rewards(&[("a", "rare", false, 10), ("b", "rare", false, 20)], true);
    let table = DropTable::of(&rw);
    let f = field(3, FULL);
    let start = SplitMix64::new(77);
    // A star: below(1000), below(total).
    let mut c = career(77);
    drops::roll(&table, &f, &mut c, 0, 0, Rarity::Rare);
    assert_eq!(c.offer.state(), after(&start, 2).state());
    // A rune: below(1000), below(n).
    let mut c = career(77);
    drops::roll(&table, &f, &mut c, 1000, 0, Rarity::Rare);
    assert!(matches!(c.equipment.inventory[..], [Held::Rune(_)]));
    assert_eq!(c.offer.state(), after(&start, 2).state());
    // Nothing eligible at the rarity or below: the kind draw only.
    let mut c = career(77);
    drops::roll(&table, &f, &mut c, 0, 0, Rarity::Uncommon);
    assert!(c.equipment.inventory.is_empty());
    assert_eq!(c.offer.state(), after(&start, 1).state());
    // Room: the inventory never grows past what `reserve` gave it.
    let mut c = career(5);
    let room = c.equipment.inventory.capacity();
    for _ in 0..2_000 {
        drops::roll(&table, &f, &mut c, RUNE_SHARE_PERMILLE, 0, Rarity::Rare);
    }
    assert_eq!(c.equipment.inventory.capacity(), room);
    assert_eq!(c.equipment.inventory.len(), MAX_INVENTORY);
}

/// T5: a named star never drops while its player holds it, wherever it is held.
#[test]
fn a_unique_star_never_drops_while_held() {
    let rw = rewards(
        &[
            ("n", "epic", true, 10),
            ("a", "epic", false, 10),
            ("b", "epic", false, 10),
        ],
        false,
    );
    let table = DropTable::of(&rw);
    let f = field(3, FULL);
    let slot_with = |held| super::equipment::EquipSlot {
        kind: super::equipment::SlotKind::Category(0),
        held: Some(held),
        level: 1,
        expertise: 0,
    };
    let count = |c: &mut Career, place| {
        (0..2_000)
            .filter(|_| rolled(&table, &f, c, 0, 0, Rarity::Epic) == Some(Held::Star(place)))
            .count()
    };
    let named = Held::Star(0);
    let mut c = career(1);
    c.equipment.inventory.push(named);
    assert_eq!(count(&mut c, 0), 0, "in the inventory");
    assert!(count(&mut c, 1) > 0 && count(&mut c, 2) > 0);
    let mut c = career(2);
    c.equipment.lumine = Some(slot_with(named));
    assert_eq!(count(&mut c, 0), 0, "in the Lumine slot");
    let mut c = career(3);
    c.equipment.slots.push(slot_with(named));
    assert_eq!(count(&mut c, 0), 0, "in a slot");
    let mut c = career(4);
    assert!(count(&mut c, 0) > 0, "not held, it drops");
    // Ordinary stars repeat.
    let mut c = career(6);
    for _ in 0..40 {
        drops::roll(&table, &f, &mut c, 0, 0, Rarity::Epic);
    }
    let a = c
        .equipment
        .inventory
        .iter()
        .filter(|h| **h == Held::Star(1))
        .count();
    assert!(a >= 2, "the same ordinary star twice: {a}");
}

/// T6: the rarity falls to the rarest below with a reward; none, no drop and no second draw.
#[test]
fn rarity_falls_to_the_rarest_below_with_a_reward() {
    let rw = rewards(
        &[("c", "common", false, 10), ("e", "epic", false, 10)],
        false,
    );
    let table = DropTable::of(&rw);
    let f = field(2, FULL);
    let mut c = career(9);
    for _ in 0..200 {
        assert_eq!(
            rolled(&table, &f, &mut c, 0, 0, Rarity::Legendary),
            Some(Held::Star(1))
        );
    }
    let mut f = field(2, FULL);
    f.rarity = (Rarity::Rare, Rarity::Supernova);
    let rw = rewards(&[("c", "common", false, 10)], false);
    let table = DropTable::of(&rw);
    let mut c = career(9);
    let start = c.offer.clone();
    assert_eq!(rolled(&table, &f, &mut c, 0, 0, Rarity::Epic), None);
    assert_eq!(c.offer.state(), after(&start, 1).state());
}

/// T7: the rune share, from the run's rune pool.
#[test]
fn a_rune_drops_with_the_share_from_the_rune_pool() {
    let rw = rewards(&[("a", "common", false, 10)], true);
    let table = DropTable::of(&rw);
    let pair = [Rune::ALL[0], Rune::ALL[3]];
    let mask = pair.iter().fold(0u8, |m, r| m | 1 << r.index());
    let f = field(2, mask);
    let mut c = career(31);
    let mut replay = c.offer.clone();
    let (mut runes, mut seen) = (0, [false; 2]);
    for _ in 0..10_000 {
        let rune_roll = replay.below(1000) < RUNE_SHARE_PERMILLE;
        let got = rolled(&table, &f, &mut c, RUNE_SHARE_PERMILLE, 0, Rarity::Common);
        if rune_roll {
            replay.below(2);
        } else {
            replay.below(10);
        }
        match got {
            Some(Held::Rune(r)) => {
                assert!(rune_roll);
                runes += 1;
                seen[pair.iter().position(|p| *p == r).expect("one of the two")] = true;
            }
            Some(Held::Star(p)) => assert!(!rune_roll && p == 0, "never the random rune"),
            None => panic!("a drop"),
        }
        assert_eq!(c.offer.state(), replay.state());
    }
    assert!(runes > 0 && seen == [true, true]);
    let mut c = career(31);
    for _ in 0..500 {
        assert!(matches!(
            rolled(&table, &f, &mut c, 0, 0, Rarity::Common),
            Some(Held::Star(0))
        ));
    }
}

/// T17 (roll part): the drop weight scales a named star's weight, by the formula.
#[test]
fn drop_weight_scales_a_named_star_for_the_member_who_has_it() {
    let rw = rewards(&[("n", "epic", true, 10), ("o", "epic", false, 10)], false);
    let table = DropTable::of(&rw);
    let f = field(2, FULL);
    for (pct, named_draws) in [(0u32, 10u32), (100, 20), (50, 15)] {
        let total = named_draws + 10;
        let mut c = career(11);
        let mut replay = c.offer.clone();
        for _ in 0..10_000 {
            replay.below(1000);
            let named = replay.below(total) < named_draws;
            let got = rolled(&table, &f, &mut c, 0, pct, Rarity::Epic);
            assert_eq!(got == Some(Held::Star(0)), named, "pct {pct}");
        }
    }
    // A weight of 1 at 50 floors to 1.
    let rw = rewards(&[("n", "epic", true, 1), ("o", "epic", false, 1)], false);
    let table = DropTable::of(&rw);
    let mut c = career(12);
    let mut replay = c.offer.clone();
    for _ in 0..1_000 {
        replay.below(1000);
        let named = replay.below(2) < 1;
        assert_eq!(
            rolled(&table, &f, &mut c, 0, 50, Rarity::Epic) == Some(Held::Star(0)),
            named
        );
    }
}

/// T8: the inventory has a bound, not a cap: past it the draws are made and nothing is added.
#[test]
fn the_inventory_has_a_bound_not_a_cap() {
    let rw = rewards(&[("a", "common", false, 10)], false);
    let table = DropTable::of(&rw);
    let f = field(1, FULL);
    let mut c = career(3);
    c.equipment.inventory = vec![Held::Star(0); MAX_INVENTORY - 1];
    drops::roll(&table, &f, &mut c, 0, 0, Rarity::Common);
    assert_eq!(c.equipment.inventory.len(), MAX_INVENTORY);
    let before = c.offer.clone();
    drops::roll(&table, &f, &mut c, 0, 0, Rarity::Common);
    assert_eq!(c.equipment.inventory.len(), MAX_INVENTORY);
    assert_eq!(c.offer.state(), after(&before, 2).state());
}

/// T9: the starting rune is in play at tick 0 and draws nothing (it is not in the inventory).
#[test]
fn the_starting_rune_is_equipped_at_tick_0() {
    use crate::engine::{Engine, Run};
    use crate::run_setup::{BuildMode, RunSetup};
    let setup = |build_mode| RunSetup {
        build_mode,
        starting_rune: Some("thunder".into()),
        ..RunSetup::default()
    };
    let state = |e: &Engine| match e.run() {
        Run::V2 { state, .. } => state.clone(),
        Run::V1 { .. } => panic!("the live rules"),
    };
    let eq = state(&Engine::start(&setup(BuildMode::Equipment)).expect("starts"));
    let offers = state(&Engine::start(&setup(BuildMode::Offers)).expect("starts"));
    let me = slot(0);
    assert!(eq.players[me].runes.iter().any(|h| h.rune == Rune::Thunder));
    assert!(eq.players.career(me).equipment.inventory.is_empty());
    assert_eq!(eq.players.career(me).offer, offers.players.career(me).offer);
    assert_eq!(eq.rng, offers.rng);
}

/// T17 (setup part): the perk set's `drop_weight` reaches the world; the bundled sets carry none;
/// the effect is a no-op elsewhere.
#[test]
fn the_perk_drop_weight_reaches_the_world() {
    use crate::engine::{Engine, Run};
    use crate::perk_sets::PerkSets;
    use crate::run_content::RunContent;
    use crate::run_setup::{BuildMode, RunSetup};
    let live = |e: &Engine| match e.run() {
        Run::V2 { world, .. } => world.drop_weight_pct,
        Run::V1 { .. } => panic!("the live rules"),
    };
    let setup = RunSetup {
        build_mode: BuildMode::Equipment,
        ..RunSetup::default()
    };
    assert_eq!(live(&Engine::start(&setup).expect("starts")), 0);
    let file = json!({ "perk_sets": [{ "id": "lucky",
        "effects": [{ "effect": "drop_weight", "amount": 40 }] }] });
    let sets = PerkSets::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("valid");
    let content = RunContent::bundled().clone().with_perk_sets(sets);
    let lucky = RunSetup {
        perk_set: "lucky".into(),
        ..setup
    };
    assert_eq!(
        live(&Engine::start_with(&lucky, &content).expect("starts")),
        40
    );
}
