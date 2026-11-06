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
    let mut c = Career {
        offer: SplitMix64::new(seed),
        ..Career::default()
    };
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

// --- the boss kill (ADR-SIM-7 point 5) ---------------------------------------------------------

/// A party of `n` in equipment mode whose tiers top out at Uncommon, Rare and Epic from levels 1,
/// 5 and 7, the members at `levels`; the pool holds one star of each of those rarities.
fn boss_party(n: u8, levels: [u32; 3]) -> (World, State) {
    let (mut w, mut s) = party(n);
    let stars = w.stage.stars.as_mut().expect("stars");
    let weights = |top| match top {
        Rarity::Uncommon => RarityWeights {
            uncommon: 1,
            ..RarityWeights::default()
        },
        Rarity::Rare => RarityWeights {
            rare: 1,
            ..RarityWeights::default()
        },
        _ => RarityWeights {
            epic: 1,
            ..RarityWeights::default()
        },
    };
    stars.tiers = [(1, Rarity::Uncommon), (5, Rarity::Rare), (7, Rarity::Epic)]
        .into_iter()
        .map(|(from_level, top)| Tier {
            from_level,
            weights: weights(top),
            random_rune_weight: None,
        })
        .collect();
    let rw = rewards(
        &[
            ("u", "uncommon", false, 10),
            ("r", "rare", false, 10),
            ("e", "epic", false, 10),
        ],
        false,
    );
    w.drops = DropTable::of(&rw);
    w.equipment_mode = true;
    let f = s.stars.as_mut().expect("stars");
    f.rarity = (Rarity::Common, Rarity::Supernova);
    f.pool = vec![0, 1, 2];
    f.rune_pool = FULL;
    for (i, level) in (0..n).zip(levels) {
        let progress = s.players.career_mut(slot(i)).progress.as_mut().unwrap();
        progress.level = level;
    }
    (w, s)
}

fn inventory(s: &State, i: u8) -> Vec<Held> {
    s.players.career(slot(i)).equipment.inventory.clone()
}

/// T10: a boss kill gives each member that is up personal drops at the rarity a boss offer would
/// have had; a downed or out member gets none; the boss's drops come before a pickup's.
#[test]
fn a_boss_kill_gives_each_up_member_personal_drops() {
    use super::party_fix_tests::{down, out};
    let (w, mut s) = boss_party(3, [5, 7, 3]);
    down(&mut s, 1);
    out(&mut s, 2);
    let mut again = s.clone();
    stars::boss_killed(&w, &mut s);
    // Among those up the highest level is 5: the Rare tier, the place 1 (or a rune).
    let mine = inventory(&s, 0);
    assert_eq!(mine.len(), usize::from(drops::BOSS_DROPS));
    assert!(matches!(mine[0], Held::Star(1) | Held::Rune(_)), "{mine:?}");
    assert!(inventory(&s, 1).is_empty() && inventory(&s, 2).is_empty());
    assert!(
        s.players
            .career(slot(0))
            .offers
            .as_ref()
            .unwrap()
            .due
            .is_empty()
    );
    // A pickup the same tick comes after.
    drop_star(&mut again, 0, Rarity::Rare);
    stars::boss_killed(&w, &mut again);
    stars::pick_up(&w, &mut again, slot(0), &mut Vec::new());
    let both = inventory(&again, 0);
    assert_eq!(both.len(), usize::from(drops::BOSS_DROPS) + 1);
    assert_eq!(both[..mine.len()], mine[..]);
    // Nobody up, nobody gets one.
    let (w, mut s) = boss_party(2, [7, 7, 1]);
    down(&mut s, 0);
    down(&mut s, 1);
    stars::boss_killed(&w, &mut s);
    assert!(inventory(&s, 0).is_empty() && inventory(&s, 1).is_empty());
}

/// T11: an equipment boss kill and a pickup offer nothing, solo or in a party.
#[test]
fn an_equipment_boss_kill_offers_nothing() {
    for n in [1, 3] {
        let (w, mut s) = boss_party(n, [7, 7, 7]);
        drop_star(&mut s, 0, Rarity::Epic);
        stars::boss_killed(&w, &mut s);
        stars::pick_up(&w, &mut s, slot(0), &mut Vec::new());
        for i in 0..n {
            let career = s.players.career(slot(i));
            let o = career.offers.as_ref().expect("offers");
            assert!(o.due.is_empty() && o.offer.is_none(), "n {n} slot {i}");
            assert_eq!(
                career.equipment.inventory.len(),
                usize::from(drops::BOSS_DROPS) + 1
            );
        }
    }
}

// --- the joiner (ADR-SAVE-7 points 2 and 4) ----------------------------------------------------

/// A solo state whose slot 0 holds two stars, a rune, Starlight, a tree point and a Lumine slot.
fn rich() -> State {
    use super::equipment::{EquipSlot, SlotKind};
    let (_, mut s) = party(1);
    let e = &mut s.players.career_mut(slot(0)).equipment;
    e.inventory = vec![Held::Star(1), Held::Star(2), Held::Rune(Rune::Thunder)];
    e.starlight = 5;
    e.tree_points = 1;
    e.lumine = Some(EquipSlot {
        kind: SlotKind::Lumine,
        held: Some(Held::Star(3)),
        level: 1,
        expertise: 0,
    });
    s
}

/// T12: a party member starts with an empty inventory and no Starlight or tree points.
#[test]
fn a_party_member_starts_with_an_empty_inventory() {
    let s = rich();
    let players = super::state::Players::party(3, &s.players, s.crit_seed);
    let mine = players.career(slot(0));
    assert_eq!(mine.equipment.inventory.len(), 3);
    assert_eq!(
        (mine.equipment.starlight, mine.equipment.tree_points),
        (5, 1)
    );
    for i in 1..3 {
        let e = &players.career(slot(i)).equipment;
        assert!(e.inventory.is_empty(), "slot {i}");
        assert_eq!((e.starlight, e.tree_points), (0, 0), "slot {i}");
        assert_eq!(
            e.lumine, mine.equipment.lumine,
            "slot {i} keeps the Lumine slot"
        );
        assert_eq!(e.slots, mine.equipment.slots);
    }
}

/// T13: a joiner, and a rejoiner, start empty too.
#[test]
fn a_joiner_starts_with_an_empty_inventory() {
    use crate::engine::{Engine, Run};
    use crate::run_setup::{BuildMode, RunSetup};
    let s = rich();
    let mut players = super::state::Players::party(2, &s.players, s.crit_seed);
    players.leave(slot(1));
    players.join(slot(1), &s.players.template(), s.crit_seed);
    let e = &players.career(slot(1)).equipment;
    assert!(e.inventory.is_empty() && e.starlight == 0 && e.tree_points == 0);
    assert!(e.lumine.is_some());

    let setup = RunSetup {
        build_mode: BuildMode::Equipment,
        ..RunSetup::default()
    };
    let mut e = Engine::start(&setup).expect("starts");
    if let Run::V2 { state, .. } = e.run_mut() {
        state.players.career_mut(slot(0)).equipment.inventory = vec![Held::Star(0); 2];
    }
    for round in 0..2 {
        e.join(slot(1)).expect("joins");
        let Run::V2 { state, .. } = e.run() else {
            panic!("the live rules")
        };
        assert!(inventory(state, 1).is_empty(), "round {round}");
        assert_eq!(inventory(state, 0).len(), 2);
        e.leave(slot(1)).expect("leaves");
    }
}

// --- the snapshot (ADR-SIM-3 point 2, ADR-SIM-7) -----------------------------------------------

/// T15: the snapshot lists each slot's inventory in drop order with the star's id and rarity, a
/// rune as itself; an offers-mode snapshot has no `inventory` key.
#[test]
fn the_snapshot_shows_the_inventory() {
    use crate::run_content::RunContent;
    use crate::snapshot::HeldView;
    let content = RunContent::bundled();
    let places = content.rewards().rewards();
    let pick = |rarity| {
        let at = places
            .iter()
            .position(|r| !r.is_random_rune() && r.rarity == rarity)
            .expect("one at each rarity");
        (u16::try_from(at).unwrap(), &places[at])
    };
    let (common, uncommon) = (pick(Rarity::Common), pick(Rarity::Uncommon));
    let (w, mut s) = party(2);
    let snap = super::snapshot::build(&w, content, &s);
    let text = serde_json::to_string(&snap).expect("serialises");
    assert!(!text.contains("inventory"), "offers mode shows none");
    for (i, got) in [(0u8, [common, uncommon]), (1, [uncommon, common])] {
        s.players.career_mut(slot(i)).equipment.inventory = vec![
            Held::Star(got[0].0),
            Held::Rune(Rune::Thunder),
            Held::Star(got[1].0),
        ];
    }
    let snap = super::snapshot::build(&w, content, &s);
    for (i, got) in [(0usize, [common, uncommon]), (1, [uncommon, common])] {
        let star = |p: (u16, &crate::rewards::Reward)| HeldView::Star {
            id: p.1.id.clone(),
            rarity: p.1.rarity,
        };
        assert_eq!(
            snap.players[i].inventory,
            vec![
                star(got[0]),
                HeldView::Rune {
                    rune: Rune::Thunder
                },
                star(got[1])
            ],
            "slot {i}"
        );
    }
    let text = serde_json::to_string(&snap.players[0].inventory).expect("serialises");
    assert!(
        text.contains(r#""kind":"rune""#) && text.contains(r#""kind":"star""#),
        "{text}"
    );
}

// --- the pickup radius stat (ADR-SIM-3) ---------------------------------------------------------

mod pickup_stat {
    use super::*;
    use crate::fixed::Fx;
    use crate::run_stats::StatEffect;
    use crate::stage::Healing;

    const RADIUS: i32 = 12_288;

    /// Whether a pickup at the given distance to the right is taken.
    type Taken = fn(&World, &State, i32) -> bool;

    fn set_total(s: &mut State, effect: StatEffect, v: u32) {
        s.players
            .career_mut(slot(0))
            .progress
            .as_mut()
            .expect("the stage has levels")
            .totals[effect as usize] = v;
    }

    /// T12: the stat adds to the stars' share, saturating, and the cap holds the sum.
    #[test]
    fn pickup_pct_adds_the_stat_to_the_stars_share() {
        let (_, mut s) = party(1);
        s.players[slot(0)].pickup_radius_pct = 100;
        set_total(&mut s, StatEffect::PickupRadiusPct, 50);
        assert_eq!(super::super::pickup_pct(&s, slot(0)), 150);
        s.players[slot(0)].pickup_radius_pct = u32::MAX;
        set_total(&mut s, StatEffect::PickupRadiusPct, u32::MAX);
        assert_eq!(super::super::pickup_pct(&s, slot(0)), u32::MAX);
        assert_eq!(
            super::super::widened(RADIUS, u32::MAX),
            RADIUS * 4,
            "held to 300 percent more"
        );
    }

    /// Slot 0 of `s` with a pickup `dx` to its right: the position to put it at.
    fn beside(s: &State, dx: i32) -> crate::fixed::Point {
        let mut at = s.players[slot(0)].at;
        at.x.0 += dx;
        at
    }

    fn heal_fixture() -> (World, State) {
        let (mut w, mut s) = party(1);
        w.stage.heal = Some(Healing {
            boss_pct: 0,
            drop_one_in: 1,
            amount: 1,
            cap: 4,
            pickup_radius: Fx(RADIUS),
        });
        s.heal_pickups = Some(Vec::new());
        (w, s)
    }

    fn heal_taken(w: &World, s: &State, dx: i32) -> bool {
        let mut s = s.clone();
        let id = EntityId(s.next_id);
        s.next_id += 1;
        let at = beside(&s, dx);
        s.heal_pickups
            .as_mut()
            .unwrap()
            .push(crate::sim::state::HealPickup { id, at });
        super::super::heal::pick_up(w, &mut s, slot(0), &mut Vec::new());
        s.heal_pickups.unwrap().is_empty()
    }

    fn star_taken(w: &World, s: &State, dx: i32, equipment: bool) -> bool {
        let mut w = w.clone();
        w.stage.stars.as_mut().unwrap().pickup_radius = Fx(RADIUS);
        w.equipment_mode = equipment;
        let mut s = s.clone();
        let id = EntityId(s.next_id);
        s.next_id += 1;
        let at = beside(&s, dx);
        s.stars.as_mut().unwrap().on_map.push(StarState {
            id,
            at,
            rarity: Rarity::Common,
        });
        if equipment {
            drops::pick_up(&w, &mut s, slot(0), &mut Vec::new());
        } else {
            stars::pick_up(&w, &mut s, slot(0), &mut Vec::new());
        }
        s.stars.unwrap().on_map.is_empty()
    }

    /// T13: a total of 50 widens a circle of `R` to 1.5 `R`, touching counts, for the heal
    /// pickups, the stars of offers mode and the stars of equipment mode.
    #[test]
    fn a_pickup_stat_widens_each_pickup() {
        let near = RADIUS * 3 / 2;
        let (hw, mut hs) = heal_fixture();
        let (sw, mut ss) = party(1);
        let cases: [(&str, Taken); 3] = [
            ("heal", heal_taken),
            ("offers star", |w, s, dx| star_taken(w, s, dx, false)),
            ("equipment star", |w, s, dx| star_taken(w, s, dx, true)),
        ];
        for (name, taken) in cases {
            let (w, s) = if name == "heal" {
                (&hw, &mut hs)
            } else {
                (&sw, &mut ss)
            };
            set_total(s, StatEffect::PickupRadiusPct, 0);
            assert!(!taken(w, s, near), "{name}: not taken at total 0");
            assert!(taken(w, s, RADIUS), "{name}: the circle itself");
            set_total(s, StatEffect::PickupRadiusPct, 50);
            assert!(taken(w, s, near), "{name}: touching counts at total 50");
            assert!(!taken(w, s, near + 1), "{name}: one past is not taken");
        }
    }

    /// T16: nothing shipped writes either stat, which is why no outcome moves: a run's
    /// totals for them start at 0 and the bundled content never raises them.
    #[test]
    fn both_totals_are_zero_in_a_shipped_run() {
        let (_, s) = party(1);
        let totals = &s.players.career(slot(0)).progress.as_ref().unwrap().totals;
        assert_eq!(totals[StatEffect::PickupRadiusPct as usize], 0);
        assert_eq!(totals[StatEffect::StardustPct as usize], 0);
        assert!(
            crate::run_stats::RunStats::bundled()
                .stats()
                .iter()
                .all(|r| !matches!(
                    r.effect,
                    StatEffect::PickupRadiusPct | StatEffect::StardustPct
                ))
        );
    }
}
