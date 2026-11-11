//! Pins and tests for the drops of a star taken and of a boss kill (ADR-SIM-2 point 2, ADR-SIM-3
//! point 2, ADR-SIM-5 point 8, ADR-SIM-7). The offers-mode pins pass before and after the
//! equipment-mode rules land: the expected values are worked out here, not copied from a run.

use super::drops::{self, DropTable};
use super::equipment::Held;
use super::party_fix_tests::{party, slot};
use super::starlight::bundled_rules;
use super::state::{Career, StarField, StarState, State};
use super::{World, stars};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::limits::MAX_INVENTORY;
use crate::rewards::{Rarity, Rewards};
use crate::rng::SplitMix64;
use crate::rune::Rune;
use crate::stage::{RarityWeights, Tier};
use crate::version::Versions;
use serde_json::{Value, json};

/// Tiers whose highest rarity is `Rare` for every level; every rarity allowed.
pub(super) fn tiered(n: u8) -> (World, State) {
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
pub(super) fn drop_star(s: &mut State, me: u8, rarity: Rarity) {
    let at = s.players[slot(me)].at;
    let id = EntityId(s.next_id);
    s.next_id += 1;
    s.stars
        .as_mut()
        .expect("stars")
        .on_map
        .push(StarState { id, at, rarity });
}

// --- the roll (ADR-SIM-2 point 2, ADR-SIM-5 point 8) -------------------------------------------

/// One reward of the hand-built file: id, rarity, whether it is a named star, draw weight.
type Spec = (&'static str, &'static str, bool, u16);

pub(super) fn rewards(list: &[Spec], with_rune: bool) -> Rewards {
    rewards_in(list, with_rune, &[])
}

/// `rewards`, with the bundled equipment rules and the category of each star by position
/// (`categories[i]`; a star past the list has none).
pub(super) fn rewards_in(list: &[Spec], with_rune: bool, categories: &[&str]) -> Rewards {
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
        .enumerate()
        .map(|(i, &(id, rarity, named, weight))| {
            let mut r = json!({ "id": id, "rarity": rarity, "effect": "attack_pct",
                "amount": 1, "limit": if named { 1 } else { 2 }, "weight": weight });
            if let Some(c) = categories.get(i) {
                r["category"] = json!(c);
            }
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
    let mut file = json!({ "rarities": rarities, "rewards": rewards });
    if !categories.is_empty() {
        file["equipment"] = serde_json::to_value(bundled_rules()).expect("the rules serialise");
    }
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
pub(super) fn after(rng: &SplitMix64, n: u32) -> SplitMix64 {
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
    pct: &[u32],
    rarity: Rarity,
) -> Option<Held> {
    let before = c.equipment.inventory.len();
    drops::roll(table, f, c, share, pct, rarity);
    (c.equipment.inventory.len() > before).then(|| c.equipment.inventory.pop().expect("one"))
}

const FULL: u8 = crate::sim::runes::FULL_POOL;

/// What the stream gives a roll, replayed by hand from the weights: the kind first (a rune below
/// the share), then the rune's `below(n)` or the star's `below(total)` (ADR-SIM-2 point 2).
pub(super) fn expected(rng: &SplitMix64, runes: &[Rune], places: &[(u16, u32)]) -> Held {
    let mut r = rng.clone();
    if r.below(1000) < bundled_rules().rune_share_permille {
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
    w.drops = DropTable::of(&rw, bundled_rules());
    w.equipment_rules = bundled_rules().clone();
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
    let table = DropTable::of(&rw, bundled_rules());
    let f = field(3, FULL);
    let start = SplitMix64::new(77);
    // A star: below(1000), below(total).
    let mut c = career(77);
    drops::roll(&table, &f, &mut c, 0, &[], Rarity::Rare);
    assert_eq!(c.offer.state(), after(&start, 2).state());
    // A rune: below(1000), below(n).
    let mut c = career(77);
    drops::roll(&table, &f, &mut c, 1000, &[], Rarity::Rare);
    assert!(matches!(c.equipment.inventory[..], [Held::Rune(_)]));
    assert_eq!(c.offer.state(), after(&start, 2).state());
    // Nothing eligible at the rarity or below: the kind draw only.
    let mut c = career(77);
    drops::roll(&table, &f, &mut c, 0, &[], Rarity::Uncommon);
    assert!(c.equipment.inventory.is_empty());
    assert_eq!(c.offer.state(), after(&start, 1).state());
    // Room: the inventory never grows past what `reserve` gave it.
    let mut c = career(5);
    let room = c.equipment.inventory.capacity();
    for _ in 0..2_000 {
        drops::roll(
            &table,
            &f,
            &mut c,
            bundled_rules().rune_share_permille,
            &[],
            Rarity::Rare,
        );
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
    let table = DropTable::of(&rw, bundled_rules());
    let f = field(3, FULL);
    let slot_with = |held| super::equipment::EquipSlot {
        kind: super::equipment::SlotKind::Category(0),
        held: Some(held),
        level: 1,
        expertise: 0,
    };
    let count = |c: &mut Career, place| {
        (0..2_000)
            .filter(|_| rolled(&table, &f, c, 0, &[], Rarity::Epic) == Some(Held::Star(place)))
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
        drops::roll(&table, &f, &mut c, 0, &[], Rarity::Epic);
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
    let table = DropTable::of(&rw, bundled_rules());
    let f = field(2, FULL);
    let mut c = career(9);
    for _ in 0..200 {
        assert_eq!(
            rolled(&table, &f, &mut c, 0, &[], Rarity::Legendary),
            Some(Held::Star(1))
        );
    }
    let mut f = field(2, FULL);
    f.rarity = (Rarity::Rare, Rarity::Supernova);
    let rw = rewards(&[("c", "common", false, 10)], false);
    let table = DropTable::of(&rw, bundled_rules());
    let mut c = career(9);
    let start = c.offer.clone();
    assert_eq!(rolled(&table, &f, &mut c, 0, &[], Rarity::Epic), None);
    assert_eq!(c.offer.state(), after(&start, 1).state());
}

/// T7: the rune share, from the run's rune pool.
#[test]
fn a_rune_drops_with_the_share_from_the_rune_pool() {
    let rw = rewards(&[("a", "common", false, 10)], true);
    let table = DropTable::of(&rw, bundled_rules());
    let pair = [Rune::ALL[0], Rune::ALL[3]];
    let mask = pair.iter().fold(0u8, |m, r| m | 1 << r.index());
    let f = field(2, mask);
    let mut c = career(31);
    let mut replay = c.offer.clone();
    let (mut runes, mut seen) = (0, [false; 2]);
    for _ in 0..10_000 {
        let rune_roll = replay.below(1000) < bundled_rules().rune_share_permille;
        let got = rolled(
            &table,
            &f,
            &mut c,
            bundled_rules().rune_share_permille,
            &[],
            Rarity::Common,
        );
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
            rolled(&table, &f, &mut c, 0, &[], Rarity::Common),
            Some(Held::Star(0))
        ));
    }
}

/// T7: the drop weight scales the weight of every star of its category, named or not, by the
/// formula, and no star of another category.
#[test]
fn drop_weight_scales_every_star_of_its_category() {
    let f = field(2, FULL);
    // Star 0 and star 1 weigh 10 each; the member's percent is for attack (category 0) only.
    for (cats, first_is_scaled) in [
        (["attack", "defence"], true),
        (["defence", "attack"], false),
    ] {
        let rw = rewards_in(
            &[("n", "epic", true, 10), ("o", "epic", false, 10)],
            false,
            &cats,
        );
        let table = DropTable::of(&rw, bundled_rules());
        for (pct, scaled_draws) in [(0u32, 10u32), (100, 20), (40, 14)] {
            // The scaled star draws `scaled_draws` of `total`; the other 10.
            let total = scaled_draws + 10;
            let mut c = career(11);
            let mut replay = c.offer.clone();
            for _ in 0..10_000 {
                replay.below(1000);
                let first_scaled =
                    replay.below(total) < if first_is_scaled { scaled_draws } else { 10 };
                let got = rolled(&table, &f, &mut c, 0, &[pct, 0, 0], Rarity::Epic);
                assert_eq!(got == Some(Held::Star(0)), first_scaled, "pct {pct}");
            }
        }
    }
    // A weight of 1 at 50 floors to 1; a star outside the member's categories keeps its own.
    let rw = rewards_in(
        &[("n", "epic", true, 1), ("o", "epic", false, 1)],
        false,
        &["attack", "defence"],
    );
    let table = DropTable::of(&rw, bundled_rules());
    let mut c = career(12);
    let mut replay = c.offer.clone();
    for _ in 0..1_000 {
        replay.below(1000);
        let first = replay.below(2) < 1;
        assert_eq!(
            rolled(&table, &f, &mut c, 0, &[50, 0, 0], Rarity::Epic) == Some(Held::Star(0)),
            first
        );
    }
}

/// T8: the inventory has a bound, not a cap: past it the draws are made and nothing is added.
#[test]
fn the_inventory_has_a_bound_not_a_cap() {
    let rw = rewards(&[("a", "common", false, 10)], false);
    let table = DropTable::of(&rw, bundled_rules());
    let f = field(1, FULL);
    let mut c = career(3);
    c.equipment.inventory = vec![Held::Star(0); MAX_INVENTORY - 1];
    drops::roll(&table, &f, &mut c, 0, &[], Rarity::Common);
    assert_eq!(c.equipment.inventory.len(), MAX_INVENTORY);
    let before = c.offer.clone();
    drops::roll(&table, &f, &mut c, 0, &[], Rarity::Common);
    assert_eq!(c.equipment.inventory.len(), MAX_INVENTORY);
    assert_eq!(c.offer.state(), after(&before, 2).state());
}

/// T9: the starting rune is in play at tick 0 and draws nothing (it is not in the inventory).
#[test]
fn the_starting_rune_is_equipped_at_tick_0() {
    use crate::engine::{Engine, Run};
    use crate::run_setup::RunSetup;
    let setup = RunSetup {
        starting_rune: Some("thunder".into()),
        ..RunSetup::default()
    };
    let state = |e: &Engine| match e.run() {
        Run::V2 { state, .. } => state.clone(),
        Run::V1 { .. } => panic!("the live rules"),
    };
    let eq = state(&Engine::start(&setup).expect("starts"));
    let me = slot(0);
    assert!(eq.players[me].runes.iter().any(|h| h.rune == Rune::Thunder));
    assert!(eq.players.career(me).equipment.inventory.is_empty());
}

/// T17 (setup part): the perk set's `drop_weight` reaches the world; the bundled sets carry 25 each;
/// the effect is a no-op elsewhere.
#[test]
fn the_perk_drop_weight_reaches_the_world() {
    use crate::engine::{Engine, Run};
    use crate::perk_sets::PerkSets;
    use crate::run_content::RunContent;
    use crate::run_setup::RunSetup;
    let live = |e: &Engine| match e.run() {
        Run::V2 { world, .. } => world.drop_weight_pct.clone(),
        Run::V1 { .. } => panic!("the live rules"),
    };
    let setup = RunSetup {
        ..RunSetup::default()
    };
    // The bundled default set carries drop weight 25 (vanguard, duelist and scout each weigh one
    // category); `duelist`, the default, weighs attack.
    assert_eq!(
        live(&Engine::start(&setup).expect("starts")),
        vec![25, 0, 0]
    );
    let file = json!({ "perk_sets": [{ "id": "lucky",
        "effects": [{ "effect": "drop_weight", "amount": 40, "category": "attack" }] }] });
    let sets = PerkSets::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("valid");
    let content = RunContent::bundled().clone().with_perk_sets(sets);
    let lucky = RunSetup {
        perk_set: "lucky".into(),
        ..setup
    };
    assert_eq!(
        live(&Engine::start_with(&lucky, &content).expect("starts")),
        vec![40, 0, 0]
    );
}

/// The player's `drop_weights` add to the perk set's own percent per category; with neither side
/// weighing anything the vector stays empty.
#[test]
fn the_players_drop_weights_add_to_the_perk_sets() {
    use crate::engine::{Engine, Run};
    use crate::perk_sets::PerkSets;
    use crate::run_content::RunContent;
    use crate::run_setup::RunSetup;
    let live = |setup: &RunSetup, content: &RunContent| match Engine::start_with(setup, content)
        .expect("starts")
        .run()
    {
        Run::V2 { world, .. } => world.drop_weight_pct.clone(),
        Run::V1 { .. } => panic!("the live rules"),
    };
    let weighted = |base: &RunSetup, weights: Value| -> RunSetup {
        let mut v = serde_json::to_value(base).expect("serialises");
        v["drop_weights"] = weights;
        serde_json::from_value(v).expect("reads")
    };
    let content = RunContent::bundled();
    let duelist = RunSetup::default();
    assert_eq!(live(&duelist, content), vec![25, 0, 0]);
    let both = weighted(&duelist, json!({ "attack": 100 }));
    assert_eq!(live(&both, content), vec![125, 0, 0]);
    let scholar = RunSetup {
        perk_set: "scholar".into(),
        ..RunSetup::default()
    };
    assert_eq!(live(&scholar, content), Vec::<u32>::new());
    let own = weighted(&scholar, json!({ "defence": 40 }));
    assert_eq!(live(&own, content), vec![0, 40, 0]);
    // A set's amount is at most 100 and the setup's at most the pack's range: the sum adds up.
    let file = json!({ "perk_sets": [{ "id": "big", "effects": [
        { "effect": "drop_weight", "amount": 100, "category": "attack" }] }] });
    let sets = PerkSets::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("valid");
    let big = RunSetup {
        perk_set: "big".into(),
        ..RunSetup::default()
    };
    let content = content.clone().with_perk_sets(sets);
    let summed = weighted(&big, json!({ "attack": 100 }));
    assert_eq!(live(&summed, &content), vec![200, 0, 0]);
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
    w.drops = DropTable::of(&rw, bundled_rules());
    w.equipment_rules = bundled_rules().clone();
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

pub(super) fn inventory(s: &State, i: u8) -> Vec<Held> {
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
    assert_eq!(mine.len(), usize::from(bundled_rules().boss_drops));
    assert!(matches!(mine[0], Held::Star(1) | Held::Rune(_)), "{mine:?}");
    assert!(inventory(&s, 1).is_empty() && inventory(&s, 2).is_empty());
    // A pickup the same tick comes after.
    drop_star(&mut again, 0, Rarity::Rare);
    stars::boss_killed(&w, &mut again);
    stars::pick_up(&w, &mut again, slot(0), &mut Vec::new());
    let both = inventory(&again, 0);
    assert_eq!(both.len(), usize::from(bundled_rules().boss_drops) + 1);
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
            assert!(career.offers.is_some(), "n {n} slot {i}");
            assert_eq!(
                career.equipment.inventory.len(),
                usize::from(bundled_rules().boss_drops) + 1
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
    use crate::run_setup::RunSetup;
    let s = rich();
    let mut players = super::state::Players::party(2, &s.players, s.crit_seed);
    players.leave(slot(1));
    players.join(slot(1), &s.players.template(), s.crit_seed);
    let e = &players.career(slot(1)).equipment;
    assert!(e.inventory.is_empty() && e.starlight == 0 && e.tree_points == 0);
    assert!(e.lumine.is_some());

    let setup = RunSetup {
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

    /// T12 / P3: the stat adds to the stars' share, saturating, and the cap holds the sum: the
    /// number the sim reads is at most 300.
    #[test]
    fn pickup_pct_adds_the_stat_to_the_stars_share() {
        let (_, mut s) = party(1);
        s.players[slot(0)].pickup_radius_pct = 100;
        set_total(&mut s, StatEffect::PickupRadiusPct, 50);
        assert_eq!(super::super::pickup_pct(&s, slot(0)), 150);
        s.players[slot(0)].pickup_radius_pct = u32::MAX;
        set_total(&mut s, StatEffect::PickupRadiusPct, u32::MAX);
        assert_eq!(super::super::pickup_pct(&s, slot(0)), 300);
        assert_eq!(
            super::super::widened(RADIUS, u32::MAX),
            RADIUS * 4,
            "held to 300 percent more"
        );
    }

    /// The snapshot's pickup number for slot 0 of `s`.
    fn shown(w: &World, s: &State) -> u32 {
        let content = crate::run_content::RunContent::bundled();
        super::super::snapshot::build(w, content, s).players[0].pickup_radius_pct
    }

    /// P1: the snapshot shows the stars' share plus the stat.
    #[test]
    fn the_snapshot_shows_stars_plus_stat() {
        let (w, mut s) = party(1);
        s.players[slot(0)].pickup_radius_pct = 100;
        set_total(&mut s, StatEffect::PickupRadiusPct, 30);
        assert_eq!(shown(&w, &s), 130);
    }

    /// P2: the number shown is the number the sim uses, the cap included.
    #[test]
    fn the_snapshot_total_is_capped() {
        let (w, mut s) = party(1);
        s.players[slot(0)].pickup_radius_pct = 250;
        set_total(&mut s, StatEffect::PickupRadiusPct, 100);
        assert_eq!(shown(&w, &s), 300);
        assert_eq!(super::super::pickup_pct(&s, slot(0)), 300);
        assert_eq!(
            super::super::widened(RADIUS, shown(&w, &s)),
            super::super::widened(RADIUS, super::super::pickup_pct(&s, slot(0)))
        );
        assert!(star_taken(&w, &s, RADIUS * 4), "300 percent more is taken");
        assert!(!star_taken(&w, &s, RADIUS * 4 + 1), "one unit past is not");
    }

    /// P5: an offers run shows the stars' share, as before.
    #[test]
    fn offers_mode_shows_the_stars_share() {
        let (w, mut s) = party(1);
        s.players[slot(0)].pickup_radius_pct = 40;
        assert_eq!(shown(&w, &s), 40);
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

    fn star_taken(w: &World, s: &State, dx: i32) -> bool {
        let mut w = w.clone();
        w.stage.stars.as_mut().unwrap().pickup_radius = Fx(RADIUS);
        let mut s = s.clone();
        let id = EntityId(s.next_id);
        s.next_id += 1;
        let at = beside(&s, dx);
        s.stars.as_mut().unwrap().on_map.push(StarState {
            id,
            at,
            rarity: Rarity::Common,
        });
        drops::pick_up(&w, &mut s, slot(0), &mut Vec::new());
        s.stars.unwrap().on_map.is_empty()
    }

    /// T13: a total of 50 widens a circle of `R` to 1.5 `R`, touching counts, for the heal
    /// pickups and the stars.
    #[test]
    fn a_pickup_stat_widens_each_pickup() {
        let near = RADIUS * 3 / 2;
        let (hw, mut hs) = heal_fixture();
        let (sw, mut ss) = party(1);
        let cases: [(&str, Taken); 2] = [("heal", heal_taken), ("star", star_taken)];
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
