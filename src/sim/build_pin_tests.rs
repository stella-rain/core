//! Pins for the build `take` writes (ADR-SIM-3, ADR-SIM-2 point 2, ADR-REPL-5): what each take
//! leaves in the player, the offers and the career, written out by hand from a small rewards
//! file, and a differential against a verbatim copy of the one-way `take` of before the build was
//! recomputed. They pass before and after the move; the expected values are worked out from the
//! file below, not copied from a run.

use super::build::Build;
use super::state::{Career, Offers, PlayerIdx, PlayerState, Progress, StarField, State};
use super::{World, initial_state, levels, orbit, player, runes, skill, stars};
use crate::attack::DamageType;
use crate::constellations::{self, Constellations};
use crate::rewards::{
    EffectEntry, MAX_BURST_DEPTH, MAX_BURST_PCT, MAX_BURST_RADIUS_PX, MAX_CRIT_CHANCE_TOTAL_PCT,
    MAX_CRIT_DAMAGE_TOTAL_PCT, MAX_ECHO_HIT_PCT, MAX_OFFER_CANDIDATES, ORBIT_SET_BASE, Rarity,
    RewardEffect, Rewards,
};
use crate::rng::SplitMix64;
use crate::run_content::RunContent;
use crate::run_stats::StatEffect;
use crate::rune::Rune;
use crate::sim::{RunDifficulty, RunPool};
use crate::version::Versions;
use serde_json::{Value, json};

/// The places of the hand-built file.
const ATK: u16 = 0;
const HP: u16 = 1;
const CRIT: u16 = 2;
const CRITD: u16 = 3;
const ECHO: u16 = 4;
const HEAL_A: u16 = 5;
const HEAL_B: u16 = 6;
const BURST_A: u16 = 7;
const BURST_B: u16 = 8;
const BDEPTH: u16 = 9;
const RANGE: u16 = 10;
const PICKUP: u16 = 11;
const COOL: u16 = 12;
const HASTE: u16 = 13;
const PYRO: u16 = 14;
const ELEM: u16 = 15;
const REGEN: u16 = 16;
const CAND: u16 = 17;
const REROLL: u16 = 18;
const EVADE: u16 = 19;
const RUNE: u16 = 20;
const N1: u16 = 21;
const N2: u16 = 22;

fn rarities() -> Value {
    json!([
        { "rarity": "common", "lumine_cost": 1 },
        { "rarity": "uncommon", "lumine_cost": 2 },
        { "rarity": "rare", "lumine_cost": 3 },
        { "rarity": "epic", "lumine_cost": 4 },
        { "rarity": "legendary", "lumine_cost": 6 },
        { "rarity": "supernova", "lumine_cost": 10 },
    ])
}

fn star(id: &str, limit: u8, effects: Vec<Value>) -> Value {
    let mut star = json!({ "id": id, "rarity": "common", "effects": effects });
    // A random rune has no limit (it leaves the offers once every rune is held).
    if limit > 0 {
        star["limit"] = json!(limit);
    }
    star
}

fn named(id: &str, name: &str) -> Value {
    json!({ "id": id, "rarity": "epic", "named": true, "star": name, "limit": 1,
        "effects": [{ "effect": "attack_pct", "amount": 1 }] })
}

fn fx(effect: &str, amount: u32) -> Value {
    json!({ "effect": effect, "amount": amount })
}

fn heal(amount: u32, kills: u32) -> Value {
    json!({ "effect": "heal_per_kills", "amount": amount, "kills": kills })
}

fn burst(amount: u32, radius: u32, depth: u32) -> Value {
    json!({ "effect": "kill_burst", "amount": amount, "radius": radius, "depth": depth })
}

/// The hand-built rewards, in the order of the place constants, with the set `duo` of the two
/// named stars: tier 1 (one member) heals 100 every 10 kills and bursts 10 % over 40 px; tier 2
/// (both) heals the same 100 but every 3 kills, and bursts 15 % over 90 px.
fn hand() -> Rewards {
    let list = vec![
        star("atk", 3, vec![fx("attack_pct", 5)]),
        star("hp", 20, vec![fx("max_hp", 100)]),
        star("crit", 2, vec![fx("crit_chance_pct", 40)]),
        star("critd", 2, vec![fx("crit_damage_pct", 200)]),
        star("echo", 2, vec![fx("echo_hit_pct", 60)]),
        star("heal_a", 2, vec![heal(300, 10)]),
        star("heal_b", 2, vec![heal(200, 4)]),
        star("burst_a", 2, vec![burst(20, 100, 2)]),
        star("burst_b", 2, vec![burst(30, 50, 1)]),
        star("bdepth", 5, vec![fx("burst_depth", 2)]),
        star("range", 2, vec![fx("range_pct", 10)]),
        star("pickup", 2, vec![fx("pickup_radius_pct", 200)]),
        star("cool", 2, vec![fx("skill_cooldown_pct", 50)]),
        star("haste", 2, vec![fx("rune_haste", 10)]),
        star(
            "pyro",
            2,
            vec![json!({ "effect": "type_bonus_pct", "type": "pyro", "amount": 7 })],
        ),
        star("elem", 2, vec![fx("element_bonus_pct", 3)]),
        star("regen", 2, vec![fx("regen", 2)]),
        star("cand", 5, vec![fx("offer_candidates", 1)]),
        star("reroll", 2, vec![fx("rerolls", 2), fx("banishes", 1)]),
        star("evade", 2, vec![fx("evade_charge", 1)]),
        star("rune", 0, vec![json!({ "effect": "random_rune" })]),
        named("n1", "Alpha"),
        named("n2", "Beta"),
    ];
    let file = json!({ "rarities": rarities(), "rewards": list });
    let rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the hand-built rewards validate");
    let sets = json!({ "constellations": [{
        "id": "duo", "name": "The duo", "members": ["n1", "n2"],
        "tiers": [
            { "threshold": 1, "effects": [heal(100, 10), burst(10, 40, 1)] },
            { "threshold": 2, "effects": [heal(100, 3), burst(15, 90, 1)] },
        ] }] });
    let sets = Constellations::parse_with(sets.to_string().as_bytes(), Versions::CURRENT, &rewards)
        .expect("the duo validates");
    rewards.with_constellations(sets)
}

fn stage() -> World {
    let mut w = World::for_test(RunContent::bundled().trial().clone());
    w.equipment_rules = crate::sim::slots::bundled_rules().clone();
    w
}

/// A solo state whose offers hold the first `n` rewards of the file.
fn start(stage: &World, n: u16) -> State {
    let pool = RunPool {
        rewards: (0..n).collect(),
        extra_rerolls: 0,
        difficulty: RunDifficulty {
            hp_pct: 100,
            damage_pct: 100,
            rarity: (Rarity::Common, Rarity::Supernova),
        },
    };
    initial_state(
        stage,
        RunContent::bundled(),
        pool,
        1,
        crate::kinds::KindTable::of_stage(stage).unwrap(),
    )
}

/// `take` with its parts split out of `s`, for slot `me`.
fn take_for(
    stage: &World,
    rewards: &Rewards,
    s: &mut State,
    me: PlayerIdx,
    i: u16,
) -> stars::Taken {
    let State { stars, players, .. } = s;
    let (player, career) = players.both_mut(me);
    let Career {
        progress,
        offer: rng,
        offers,
        regen,
        ..
    } = career;
    let f = stars.as_ref().expect("a star field");
    let o = offers.as_mut().expect("star offers");
    stars::take(stage, rewards, rng, progress, player, regen, f, o, i)
}

/// What the build holds, and what its picks counted, as a take leaves it. The expected values
/// are kept relative to the start, so a change of the trial stage's start does not move them.
#[derive(Clone, Debug, PartialEq, Default)]
struct Snap {
    range: u32,
    pickup: u32,
    cool: u32,
    haste: u32,
    kill_heal: u32,
    kill_heal_every: u32,
    crit: u32,
    critd: u32,
    echo: u32,
    burst_pct: u32,
    burst_radius: u32,
    burst_depth: u32,
    burst_depth_extra: u32,
    type_bonus: [u32; DamageType::ALL.len()],
    regen: u32,
    extra_candidates: u8,
    attack: u32,
    max_hp_more: u32,
    hp_more: u32,
    rerolls_more: u32,
    banishes_more: u32,
    set_count: u8,
    runes: usize,
}

struct Base {
    totals: Vec<u32>,
    regen: u32,
    hp: u32,
    max_hp: u32,
    rerolls: u32,
    banishes: u32,
}

fn base_of(s: &State) -> Base {
    let career = s.players.career(PlayerIdx::SOLO);
    let p = &s.players[PlayerIdx::SOLO];
    let o = career.offers.as_ref().expect("offers");
    Base {
        totals: career.progress.as_ref().expect("progress").totals.to_vec(),
        regen: career.regen,
        hp: p.hp,
        max_hp: p.max_hp,
        rerolls: o.rerolls_left,
        banishes: o.banishes_left,
    }
}

fn snap(s: &State, b: &Base) -> Snap {
    let career = s.players.career(PlayerIdx::SOLO);
    let p = &s.players[PlayerIdx::SOLO];
    let o = career.offers.as_ref().expect("offers");
    let totals = &career.progress.as_ref().expect("progress").totals;
    Snap {
        range: p.range_pct,
        pickup: p.pickup_radius_pct,
        cool: p.skill_cooldown_pct,
        haste: p.rune_haste_pct,
        kill_heal: p.kill_heal,
        kill_heal_every: p.kill_heal_every,
        crit: p.crit_chance_pct,
        critd: p.crit_damage_pct,
        echo: p.echo_hit_pct,
        burst_pct: p.burst_pct,
        burst_radius: p.burst_radius,
        burst_depth: p.burst_depth,
        burst_depth_extra: p.burst_depth_extra,
        type_bonus: p.type_bonus_pct,
        regen: career.regen - b.regen,
        extra_candidates: o.extra_candidates,
        attack: totals[StatEffect::AttackPct as usize] - b.totals[StatEffect::AttackPct as usize],
        max_hp_more: p.max_hp - b.max_hp,
        hp_more: p.hp - b.hp,
        rerolls_more: o.rerolls_left - b.rerolls,
        banishes_more: o.banishes_left - b.banishes,
        set_count: o.set_counts.first().copied().unwrap_or(0),
        runes: p.runes.len(),
    }
}

fn bonus(pyro: u32, elements: u32) -> [u32; DamageType::ALL.len()] {
    let mut out = [0; DamageType::ALL.len()];
    for (pct, ty) in out.iter_mut().zip(DamageType::ALL) {
        *pct = if ty.is_physical() { 0 } else { elements };
        if ty == DamageType::Pyro {
            *pct += pyro;
        }
    }
    out
}

/// Takes the star at `place` and checks the state against `want`, changed by `change`.
struct Walk {
    stage: World,
    rewards: Rewards,
    s: State,
    base: Base,
    want: Snap,
}

impl Walk {
    fn new(n: u16) -> Walk {
        let stage = stage();
        let s = start(&stage, n);
        let base = base_of(&s);
        let want = snap(&s, &base);
        assert_eq!(want, Snap::default(), "a run starts with an empty build");
        Walk {
            stage,
            rewards: hand(),
            s,
            base,
            want,
        }
    }

    fn take(&mut self, place: u16, change: impl FnOnce(&mut Snap)) {
        take_for(
            &self.stage,
            &self.rewards,
            &mut self.s,
            PlayerIdx::SOLO,
            place,
        );
        change(&mut self.want);
        assert_eq!(
            snap(&self.s, &self.base),
            self.want,
            "after taking place {place}"
        );
    }
}

/// Every build number after each take of a fixed sequence, worked out from `hand()`: sums, the
/// caps (crit chance 75, crit damage 300, echo 100, burst radius 240, burst depth 5), the
/// shortest heal interval, the chain of the set's tiers and the grants.
#[test]
fn take_builds_what_it_built_before() {
    let mut w = Walk::new(23);
    let room = u8::try_from(MAX_OFFER_CANDIDATES - w.stage.tuning.offer_size).unwrap();
    // The attack and hit points: sums; the player at full hit points is healed as many as the most rises.
    w.take(ATK, |e| e.attack = 5);
    w.take(HP, |e| (e.max_hp_more, e.hp_more) = (100, 100));
    w.take(HP, |e| (e.max_hp_more, e.hp_more) = (200, 200));
    // Crit chance 40 twice (80) caps at 75; crit damage 200 twice at 300; echo 60 twice at 100.
    w.take(CRIT, |e| e.crit = 40);
    w.take(CRIT, |e| e.crit = MAX_CRIT_CHANCE_TOTAL_PCT);
    w.take(CRITD, |e| e.critd = 200);
    w.take(CRITD, |e| e.critd = MAX_CRIT_DAMAGE_TOTAL_PCT);
    w.take(ECHO, |e| e.echo = 60);
    w.take(ECHO, |e| e.echo = MAX_ECHO_HIT_PCT);
    // Heals add their amounts and keep the shortest interval.
    w.take(HEAL_A, |e| (e.kill_heal, e.kill_heal_every) = (300, 10));
    w.take(HEAL_B, |e| (e.kill_heal, e.kill_heal_every) = (500, 4));
    // Bursts add percent and radius, the chain is the deepest; burst depth rewards cap at 5.
    w.take(BURST_A, |e| {
        (e.burst_pct, e.burst_radius, e.burst_depth) = (20, 100, 2)
    });
    w.take(BURST_B, |e| {
        (e.burst_pct, e.burst_radius, e.burst_depth) = (50, 150, 2)
    });
    w.take(BDEPTH, |e| e.burst_depth_extra = 2);
    w.take(BDEPTH, |e| e.burst_depth_extra = 4);
    w.take(BDEPTH, |e| e.burst_depth_extra = MAX_BURST_DEPTH);
    // Plain sums, stored uncapped (the caps are applied where they are read).
    w.take(RANGE, |e| e.range = 10);
    w.take(RANGE, |e| e.range = 20);
    w.take(PICKUP, |e| e.pickup = 200);
    w.take(PICKUP, |e| e.pickup = 400);
    w.take(COOL, |e| e.cool = 50);
    w.take(COOL, |e| e.cool = 100);
    w.take(HASTE, |e| e.haste = 10);
    w.take(PYRO, |e| e.type_bonus = bonus(7, 0));
    w.take(PYRO, |e| e.type_bonus = bonus(14, 0));
    w.take(ELEM, |e| e.type_bonus = bonus(14, 3));
    w.take(REGEN, |e| e.regen = 2);
    w.take(REGEN, |e| e.regen = 4);
    // Five offer-candidates rewards stop at the room under the offer-size limit.
    for n in 1..=5u8 {
        w.take(CAND, |e| e.extra_candidates = n.min(room));
    }
    // Grants: rerolls and banishes, an evade charge, a rune drawn.
    w.take(REROLL, |e| (e.rerolls_more, e.banishes_more) = (2, 1));
    w.take(EVADE, |_| {});
    w.take(RUNE, |e| e.runes = 1);
    // The set: one member reaches tier 1 (heal 100 / 10 kills, burst 10 % over 40 px), the second
    // tier 2, whose heal has the same amount and so adds nothing, and whose burst adds the
    // difference, 5 % over 50 px, to a radius capped at 240.
    w.take(N1, |e| {
        e.set_count = 1;
        e.attack += 1;
        e.kill_heal = 600;
        e.burst_pct = 60;
        e.burst_radius = 190;
    });
    w.take(N2, |e| {
        e.set_count = 2;
        e.attack += 1;
        e.burst_pct = 65;
        e.burst_radius = MAX_BURST_RADIUS_PX;
    });
    // A burst percent past the cap holds at it.
    assert!(w.want.burst_pct <= MAX_BURST_PCT);
    let o = w.s.players.career(PlayerIdx::SOLO).offers.as_ref().unwrap();
    assert_eq!(o.taken[usize::from(CAND)], 5);
    assert_eq!(o.taken[usize::from(N1)], 1);
}

/// The set alone, in either order: the second tier's heal has the same amount and a shorter
/// interval, which the first tier's kept (the delta drops an entry whose amount does not rise),
/// and its burst adds the difference.
#[test]
fn a_tier_that_does_not_raise_an_amount_leaves_the_rest_of_its_entry_out() {
    for order in [[N1, N2], [N2, N1]] {
        let mut w = Walk::new(23);
        w.take(order[0], |e| {
            e.set_count = 1;
            e.attack = 1;
            e.kill_heal = 100;
            e.kill_heal_every = 10;
            e.burst_pct = 10;
            e.burst_radius = 40;
            e.burst_depth = 1;
        });
        w.take(order[1], |e| {
            e.set_count = 2;
            e.attack = 2;
            e.burst_pct = 15;
            e.burst_radius = 90;
        });
    }
}

/// Refusals and clamps, each as `take` has always had them.
#[test]
fn take_keeps_its_clamps() {
    let stage = stage();
    let rewards = hand();
    // A stage without levels ignores the stats.
    let mut s = start(&stage, 23);
    s.players.career_mut(PlayerIdx::SOLO).progress = None;
    let before = s.players[PlayerIdx::SOLO].max_hp;
    take_for(&stage, &rewards, &mut s, PlayerIdx::SOLO, HP);
    assert_eq!(s.players[PlayerIdx::SOLO].max_hp, before);
    // A player at 0 hit points gets the most, not the heal.
    let mut s = start(&stage, 23);
    s.players[PlayerIdx::SOLO].hp = 0;
    let max = s.players[PlayerIdx::SOLO].max_hp;
    take_for(&stage, &rewards, &mut s, PlayerIdx::SOLO, HP);
    assert_eq!(s.players[PlayerIdx::SOLO].max_hp, max + 100);
    assert_eq!(s.players[PlayerIdx::SOLO].hp, 0);
    // A random rune with every rune held draws nothing.
    let mut s = start(&stage, 23);
    for rune in Rune::ALL {
        s.players[PlayerIdx::SOLO]
            .runes
            .push(crate::sim::state::HeldRune {
                rune,
                cooldown_left: 0,
            });
    }
    let rng = s.players.career(PlayerIdx::SOLO).offer.clone();
    take_for(&stage, &rewards, &mut s, PlayerIdx::SOLO, RUNE);
    assert_eq!(s.players.career(PlayerIdx::SOLO).offer, rng);
    // Every hit points star at its stack limit adds the exact sum.
    let mut s = start(&stage, 23);
    let max = s.players[PlayerIdx::SOLO].max_hp;
    for _ in 0..20 {
        take_for(&stage, &rewards, &mut s, PlayerIdx::SOLO, HP);
    }
    assert_eq!(s.players[PlayerIdx::SOLO].max_hp, max + 2_000);
}

/// The same pick by slot 1 of a party leaves slots 0 and 2 as they were.
#[test]
fn a_take_by_one_slot_leaves_the_others_alone() {
    let (stage, mut s) = super::party_fix_tests::party(3);
    let content = RunContent::bundled();
    let slot = super::party_fix_tests::slot;
    let (before0, before2) = (
        (
            s.players[slot(0)].clone(),
            s.players.career(slot(0)).clone(),
        ),
        (
            s.players[slot(2)].clone(),
            s.players.career(slot(2)).clone(),
        ),
    );
    let at = content
        .rewards()
        .rewards()
        .iter()
        .position(|r| {
            r.effects
                .iter()
                .any(|e| e.effect == RewardEffect::CritChancePct)
        })
        .and_then(|p| u16::try_from(p).ok())
        .expect("a crit reward");
    take_for(&stage, content.rewards(), &mut s, slot(1), at);
    assert!(s.players[slot(1)].crit_chance_pct > 0);
    assert_eq!(
        (
            s.players[slot(0)].clone(),
            s.players.career(slot(0)).clone()
        ),
        before0
    );
    assert_eq!(
        (
            s.players[slot(2)].clone(),
            s.players.career(slot(2)).clone()
        ),
        before2
    );
}

/// Picks in a fixed shuffled order from a seed, each star up to its stack limit.
fn shuffled(rewards: &Rewards, seed: u64) -> Vec<u16> {
    let mut rng = SplitMix64::new(seed);
    let mut picks = Vec::new();
    for (p, r) in rewards.rewards().iter().enumerate() {
        for _ in 0..r.limit.unwrap_or(1) {
            picks.push(u16::try_from(p).unwrap());
        }
    }
    for i in (1..picks.len()).rev() {
        let j = usize::try_from(rng.below(u32::try_from(i + 1).unwrap())).unwrap();
        picks.swap(i, j);
    }
    picks
}

/// `take` and the one-way reference agree on the whole state after every pick.
fn agree(stage: &World, rewards: &Rewards, picks: &[u16], n: u16) {
    let mut a = start(stage, n);
    let mut b = a.clone();
    for &i in picks {
        take_for(stage, rewards, &mut a, PlayerIdx::SOLO, i);
        {
            let State { stars, players, .. } = &mut b;
            let (player, career) = players.both_mut(PlayerIdx::SOLO);
            let Career {
                progress,
                offer: rng,
                offers,
                regen,
                ..
            } = career;
            let f = stars.as_ref().unwrap();
            let o = offers.as_mut().unwrap();
            reference_take(stage, rewards, rng, progress, player, regen, f, o, i);
        }
        assert_eq!(
            super::state_hash(&a),
            super::state_hash(&b),
            "after place {i}"
        );
        assert_eq!(
            a.players[PlayerIdx::SOLO],
            b.players[PlayerIdx::SOLO],
            "after place {i}"
        );
        assert_eq!(
            a.players.career(PlayerIdx::SOLO),
            b.players.career(PlayerIdx::SOLO)
        );
    }
}

#[test]
fn take_equals_the_one_way_reference() {
    let stage = stage();
    let bundled = RunContent::bundled().rewards();
    let n = u16::try_from(bundled.rewards().len()).unwrap();
    for seed in [1, 2, 3] {
        agree(&stage, bundled, &shuffled(bundled, seed), n);
    }
    let own = hand();
    for seed in [4, 5, 6] {
        agree(&stage, &own, &shuffled(&own, seed), 23);
    }
    let in_order: Vec<u16> = (0..23).collect();
    agree(&stage, &own, &in_order, 23);
}

/// The derived fields of the player, `extra_candidates` and the stars' share of the totals and
/// `regen` equal the build of the stars held.
fn assert_build_is_held(stage: &World, rewards: &Rewards, s: &State, base: &Base, why: &str) {
    let career = s.players.career(PlayerIdx::SOLO);
    let p = &s.players[PlayerIdx::SOLO];
    let o = career.offers.as_ref().expect("offers");
    let b = Build::of(stage, rewards, &o.taken);
    let totals = &career.progress.as_ref().expect("progress").totals;
    let mut stats = [0; StatEffect::ALL.len()];
    for (i, stat) in stats.iter_mut().enumerate() {
        *stat = totals[i] - base.totals[i];
    }
    let got = Build {
        range_pct: p.range_pct,
        pickup_radius_pct: p.pickup_radius_pct,
        skill_cooldown_pct: p.skill_cooldown_pct,
        kill_heal: p.kill_heal,
        kill_heal_every: p.kill_heal_every,
        crit_chance_pct: p.crit_chance_pct,
        crit_damage_pct: p.crit_damage_pct,
        echo_hit_pct: p.echo_hit_pct,
        burst_pct: p.burst_pct,
        burst_radius: p.burst_radius,
        burst_depth: p.burst_depth,
        burst_depth_extra: p.burst_depth_extra,
        rune_haste_pct: p.rune_haste_pct,
        regen: career.regen - base.regen,
        type_bonus_pct: p.type_bonus_pct,
        extra_candidates: o.extra_candidates,
        stats,
    };
    assert_eq!(got, b, "{why}");
}

#[test]
fn the_derived_fields_equal_the_build_of_what_is_held() {
    let stage = stage();
    assert_eq!(
        Build::of(&stage, &hand(), &[]),
        Build::default(),
        "nothing held, nothing built"
    );
    let own = hand();
    let bundled = RunContent::bundled().rewards();
    let n = u16::try_from(bundled.rewards().len()).unwrap();
    let in_order: Vec<u16> = (0..23).collect();
    let runs: Vec<(&Rewards, u16, Vec<u16>)> = vec![
        (&own, 23, in_order),
        (&own, 23, shuffled(&own, 4)),
        (&own, 23, shuffled(&own, 5)),
        (bundled, n, shuffled(bundled, 1)),
        (bundled, n, shuffled(bundled, 2)),
    ];
    for (rewards, n, picks) in runs {
        let mut s = start(&stage, n);
        let base = base_of(&s);
        for i in picks {
            take_for(&stage, rewards, &mut s, PlayerIdx::SOLO, i);
            assert_build_is_held(&stage, rewards, &s, &base, &format!("after place {i}"));
        }
    }
}

/// Four stars taken in every order leave the same state.
#[test]
fn the_order_of_the_picks_does_not_change_the_build() {
    let stage = stage();
    let rewards = hand();
    let four = [CRIT, HEAL_A, BURST_A, N1];
    let mut orders: Vec<Vec<u16>> = vec![Vec::new()];
    for _ in 0..four.len() {
        let mut next_orders = Vec::new();
        for o in &orders {
            for p in four.iter().filter(|p| !o.contains(*p)) {
                let mut next = o.clone();
                next.push(*p);
                next_orders.push(next);
            }
        }
        orders = next_orders;
    }
    assert_eq!(orders.len(), 24);
    let mut first: Option<Snap> = None;
    for order in orders {
        let mut s = start(&stage, 23);
        let base = base_of(&s);
        for i in order {
            take_for(&stage, &rewards, &mut s, PlayerIdx::SOLO, i);
        }
        let got = snap(&s, &base);
        assert_build_is_held(&stage, &rewards, &s, &base, "the four");
        assert_eq!(*first.get_or_insert(got.clone()), got);
    }
}

/// The build of a set's tiers is the chain of deltas: the second tier's heal does not raise an
/// amount, so its shorter interval does not enter; its burst adds the difference.
#[test]
fn the_build_of_the_tiers_is_the_chain_of_deltas() {
    let stage = stage();
    let rewards = hand();
    let held = |places: &[u16]| {
        let mut h = vec![0u8; 23];
        for p in places {
            h[usize::from(*p)] += 1;
        }
        h
    };
    let one = Build::of(&stage, &rewards, &held(&[N1]));
    let two = Build::of(&stage, &rewards, &held(&[N1, N2]));
    assert_eq!(two, Build::of(&stage, &rewards, &held(&[N2, N1])));
    assert_eq!(one, Build::of(&stage, &rewards, &held(&[N2])));
    assert_eq!((one.kill_heal, one.kill_heal_every), (100, 10));
    assert_eq!((two.kill_heal, two.kill_heal_every), (100, 10));
    assert_eq!(
        (one.burst_pct, one.burst_radius, one.burst_depth),
        (10, 40, 1)
    );
    assert_eq!(
        (two.burst_pct, two.burst_radius, two.burst_depth),
        (15, 90, 1)
    );
    assert_eq!(two.stats[StatEffect::AttackPct as usize], 2);
}

/// A verbatim copy of `take` and `apply_effects` as they were before the build was recomputed
/// (test code only, kept while the offers path exists).
#[allow(clippy::too_many_arguments)]
fn reference_take(
    stage: &World,
    rewards: &Rewards,
    rng: &mut SplitMix64,
    progress: &mut Option<Progress>,
    player: &mut PlayerState,
    regen: &mut u32,
    f: &StarField,
    o: &mut Offers,
    i: u16,
) -> stars::Taken {
    let at = usize::from(i);
    o.taken[at] = o.taken[at].saturating_add(1);
    let reward = &rewards.rewards()[at];
    let rune = reference_apply(
        stage,
        rewards,
        rng,
        progress,
        player,
        regen,
        f,
        o,
        i,
        &reward.effects,
    );
    let mut tiers = Vec::new();
    let sets = rewards.constellations();
    if let Some(set) = sets.set_of(&reward.id) {
        let c = &sets.sets()[set];
        let held = c
            .members
            .iter()
            .filter(|m| rewards.index_of(m).is_some_and(|p| o.taken[p] > 0))
            .count();
        let held = u8::try_from(held).unwrap_or(u8::MAX);
        let before = o.set_counts.get(set).copied().unwrap_or(0);
        if let Some(count) = o.set_counts.get_mut(set) {
            *count = held;
        }
        for (t, tier) in c.tiers.iter().enumerate() {
            if before < tier.threshold && tier.threshold <= held {
                let lower = t.checked_sub(1).map(|p| &c.tiers[p]);
                let delta = constellations::tier_delta(lower, tier);
                // A tier's orbit is keyed on its set (`Rewards::orbit_def`).
                let key = ORBIT_SET_BASE + u16::try_from(set).unwrap_or(0);
                reference_apply(
                    stage, rewards, rng, progress, player, regen, f, o, key, &delta,
                );
                tiers.push((set, u8::try_from(t + 1).unwrap_or(u8::MAX)));
            }
        }
    }
    stars::Taken { rune, tiers }
}

/// Does what `effects` do, once: the effects of the reward at `i` of the file (an orbit keys on
/// that place), or of a constellation's tier.
#[allow(clippy::too_many_arguments)]
fn reference_apply(
    stage: &World,
    rewards: &Rewards,
    rng: &mut SplitMix64,
    progress: &mut Option<Progress>,
    player: &mut PlayerState,
    regen: &mut u32,
    f: &StarField,
    o: &mut Offers,
    i: u16,
    effects: &[EffectEntry],
) -> Option<Rune> {
    let mut rune = None;
    for entry in effects {
        // Every effect but the random rune's has an amount (`rewards::validate`).
        let amount = entry.amount.unwrap_or(0);
        match entry.effect {
            RewardEffect::StageSkill => {
                let index = entry
                    .skill
                    .as_ref()
                    .and_then(|id| rewards.stage_skills().index_of(id));
                if let Some(index) = index {
                    skill::take(rewards, player, index, amount);
                }
            }
            RewardEffect::EvadeCharge => player::add_evade_charges(stage, player, amount),
            RewardEffect::Orbit => orbit::take(player, i, amount),
            RewardEffect::RangePct => player.range_pct = player.range_pct.saturating_add(amount),
            RewardEffect::PickupRadiusPct => {
                player.pickup_radius_pct = player.pickup_radius_pct.saturating_add(amount)
            }
            RewardEffect::SkillCooldownPct => {
                player.skill_cooldown_pct = player.skill_cooldown_pct.saturating_add(amount)
            }
            RewardEffect::HealPerKills => {
                // Every pick adds its amount and heals at the shortest interval among them.
                let every = entry.kills.unwrap_or(0).max(1);
                player.kill_heal = player.kill_heal.saturating_add(amount);
                player.kill_heal_every = match player.kill_heal_every {
                    0 => every,
                    held => held.min(every),
                };
            }
            RewardEffect::CritChancePct => {
                player.crit_chance_pct = player
                    .crit_chance_pct
                    .saturating_add(amount)
                    .min(MAX_CRIT_CHANCE_TOTAL_PCT)
            }
            RewardEffect::CritDamagePct => {
                player.crit_damage_pct = player
                    .crit_damage_pct
                    .saturating_add(amount)
                    .min(MAX_CRIT_DAMAGE_TOTAL_PCT)
            }
            RewardEffect::EchoHitPct => {
                player.echo_hit_pct = player
                    .echo_hit_pct
                    .saturating_add(amount)
                    .min(MAX_ECHO_HIT_PCT)
            }
            RewardEffect::KillBurst => {
                let (radius, depth) = (entry.radius.unwrap_or(0), entry.depth.unwrap_or(0));
                player.burst_pct = player.burst_pct.saturating_add(amount).min(MAX_BURST_PCT);
                player.burst_radius = player
                    .burst_radius
                    .saturating_add(radius)
                    .min(MAX_BURST_RADIUS_PX);
                player.burst_depth = player.burst_depth.max(depth).min(MAX_BURST_DEPTH);
            }
            RewardEffect::BurstDepth => {
                player.burst_depth_extra = player
                    .burst_depth_extra
                    .saturating_add(amount)
                    .min(MAX_BURST_DEPTH)
            }
            RewardEffect::Regen => *regen = regen.saturating_add(amount),
            // The perks' counters (`loadout::perk`), added to for the rest of the run.
            RewardEffect::OfferCandidates => {
                let room =
                    u8::try_from(MAX_OFFER_CANDIDATES - stage.tuning.offer_size).unwrap_or(u8::MAX);
                let more = u8::try_from(amount).unwrap_or(u8::MAX);
                o.extra_candidates = o.extra_candidates.saturating_add(more).min(room);
            }
            RewardEffect::Rerolls => o.rerolls_left = o.rerolls_left.saturating_add(amount),
            RewardEffect::Banishes => o.banishes_left = o.banishes_left.saturating_add(amount),
            RewardEffect::RuneHaste => {
                player.rune_haste_pct = player.rune_haste_pct.saturating_add(amount)
            }
            RewardEffect::RandomRune => rune = runes::draw(rng, player, f.rune_pool),
            RewardEffect::TypeBonusPct
            | RewardEffect::AttackTypeBonusPct
            | RewardEffect::ElementBonusPct => {
                for (ty, pct) in DamageType::ALL.iter().zip(&mut player.type_bonus_pct) {
                    let applies = match entry.effect {
                        RewardEffect::AttackTypeBonusPct => ty.is_physical(),
                        RewardEffect::ElementBonusPct => !ty.is_physical(),
                        _ => entry.damage_type == Some(*ty),
                    };
                    if applies {
                        *pct = pct.saturating_add(amount);
                    }
                }
            }
            effect => {
                if let (Some(effect), Some(p)) = (effect.stat(), progress.as_mut()) {
                    levels::add_effect(p, player, effect, amount);
                }
            }
        }
    }
    rune
}

// --- a fall in the build (an unequip, ADR-SIM-3 point 2, ADR-SIM-7) -----------------------------

/// The held counts of the hand-built file with the stars at `places` held once.
fn held(places: &[u16]) -> Vec<u8> {
    let mut v = vec![0u8; 23];
    for &p in places {
        v[usize::from(p)] += 1;
    }
    v
}

/// Writes the change from the build of `from` to the build of `to` into slot 0 of `s`.
fn write_change(stage: &World, rewards: &Rewards, s: &mut State, from: &[u16], to: &[u16]) {
    let before = Build::of(stage, rewards, &held(from));
    let after = Build::of(stage, rewards, &held(to));
    let (player, career) = s.players.both_mut(PlayerIdx::SOLO);
    let o = career.offers.as_mut().expect("offers");
    Build::write(
        &before,
        &after,
        player,
        &mut career.progress,
        &mut career.regen,
        o,
    );
}

fn total(s: &State, stat: StatEffect) -> u32 {
    s.players
        .career(PlayerIdx::SOLO)
        .progress
        .as_ref()
        .expect("levels")
        .totals[stat as usize]
}

/// T4: a fall lowers the stat totals and the regeneration by the star's share; the perk's stays.
#[test]
fn a_fall_lowers_the_stat_totals_and_regen() {
    let (stage, rewards) = (stage(), hand());
    let mut s = start(&stage, 23);
    s.players.career_mut(PlayerIdx::SOLO).regen = 3; // a perk gave 3 first
    let (atk, hp) = (
        total(&s, StatEffect::AttackPct),
        total(&s, StatEffect::MaxHp),
    );
    write_change(&stage, &rewards, &mut s, &[], &[ATK, HP, REGEN]);
    assert_eq!(total(&s, StatEffect::AttackPct), atk + 5);
    assert_eq!(total(&s, StatEffect::MaxHp), hp + 100);
    assert_eq!(s.players.career(PlayerIdx::SOLO).regen, 5);
    write_change(&stage, &rewards, &mut s, &[ATK, HP, REGEN], &[]);
    assert_eq!(total(&s, StatEffect::AttackPct), atk);
    assert_eq!(total(&s, StatEffect::MaxHp), hp);
    assert_eq!(
        s.players.career(PlayerIdx::SOLO).regen,
        3,
        "the perk's share stays"
    );
}

/// T5: the most hit points fall with the clamp: no heal, never 0, a downed player stays down.
#[test]
fn max_hp_falls_with_the_hp_clamp() {
    let (stage, rewards) = (stage(), hand());
    let at = |s: &State| {
        (
            s.players[PlayerIdx::SOLO].max_hp,
            s.players[PlayerIdx::SOLO].hp,
        )
    };
    let mut s = start(&stage, 23);
    let (max, _) = at(&s);
    s.players[PlayerIdx::SOLO].hp = max;
    write_change(&stage, &rewards, &mut s, &[], &[HP]);
    assert_eq!(
        at(&s),
        (max + 100, max + 100),
        "a full player is healed by the star"
    );
    write_change(&stage, &rewards, &mut s, &[HP], &[]);
    assert_eq!(at(&s), (max, max), "back to the base");

    // A hurt player keeps its hit points when the star leaves.
    write_change(&stage, &rewards, &mut s, &[], &[HP]);
    s.players[PlayerIdx::SOLO].hp = 40.min(max);
    write_change(&stage, &rewards, &mut s, &[HP], &[]);
    assert_eq!(at(&s), (max, 40.min(max)));

    // The floor is 1: a star larger than what is left leaves 1.
    let mut s = start(&stage, 23);
    write_change(&stage, &rewards, &mut s, &[], &[HP]);
    s.players[PlayerIdx::SOLO].max_hp = 50;
    s.players[PlayerIdx::SOLO].hp = 50;
    write_change(&stage, &rewards, &mut s, &[HP], &[]);
    assert_eq!(at(&s), (1, 1));

    // A downed player stays at 0.
    let mut s = start(&stage, 23);
    write_change(&stage, &rewards, &mut s, &[], &[HP]);
    s.players[PlayerIdx::SOLO].hp = 0;
    write_change(&stage, &rewards, &mut s, &[HP], &[]);
    assert_eq!(at(&s), (max, 0));
}

/// T6: a replace can raise one stat and lower another in one write.
#[test]
fn a_replace_can_raise_one_stat_and_lower_another() {
    let (stage, rewards) = (stage(), hand());
    let mut s = start(&stage, 23);
    let (atk, hp) = (
        total(&s, StatEffect::AttackPct),
        total(&s, StatEffect::MaxHp),
    );
    write_change(&stage, &rewards, &mut s, &[], &[ATK]);
    write_change(&stage, &rewards, &mut s, &[ATK], &[HP]);
    assert_eq!(total(&s, StatEffect::AttackPct), atk);
    assert_eq!(total(&s, StatEffect::MaxHp), hp + 100);
}

// --- the build at a slot level (equipment mode, ADR-SIM-3 point 2) ------------------------------

use super::slots::level_scaled;

/// T6: at level 1 the build of the levelled stars is the build of the counts, sets included.
#[test]
fn the_build_at_level_1_is_the_build_of_the_counts() {
    let (stage, rewards) = (stage(), hand());
    let places = [
        ATK, ATK, HP, HEAL_A, BURST_A, REGEN, PYRO, N1, N2, RANGE, CRIT,
    ];
    let levelled: Vec<(u16, u8)> = places.iter().map(|&p| (p, 1)).collect();
    assert_eq!(
        Build::of_levelled(&stage, &rewards, &levelled),
        Build::of(&stage, &rewards, &held(&places))
    );
    // Level 0 reads as 1.
    let zero: Vec<(u16, u8)> = places.iter().map(|&p| (p, 0)).collect();
    assert_eq!(
        Build::of_levelled(&stage, &rewards, &zero),
        Build::of(&stage, &rewards, &held(&places))
    );
}

/// T7: a level raises the amounts and leaves the intervals, radius, depth, counts and tiers.
#[test]
fn a_level_raises_the_amounts_and_not_the_intervals() {
    let (stage, rewards) = (stage(), hand());
    let at10 = |p: u16| Build::of_levelled(&stage, &rewards, &[(p, 10)]);
    let at1 = |p: u16| Build::of_levelled(&stage, &rewards, &[(p, 1)]);
    assert_eq!(at10(ATK).stats[StatEffect::AttackPct as usize], 6);
    assert_eq!(at10(HP).stats[StatEffect::MaxHp as usize], 127);
    let heal = at10(HEAL_A);
    assert_eq!(
        heal.kill_heal,
        level_scaled(crate::sim::slots::bundled_rules(), 300, 10)
    );
    assert_eq!(heal.kill_heal_every, at1(HEAL_A).kill_heal_every);
    let burst = at10(BURST_A);
    assert_eq!(
        burst.burst_pct,
        level_scaled(crate::sim::slots::bundled_rules(), 20, 10)
    );
    assert_eq!(burst.burst_radius, at1(BURST_A).burst_radius);
    assert_eq!(burst.burst_depth, at1(BURST_A).burst_depth);
    // Counts are not scaled: burst depth and offer candidates.
    assert_eq!(
        at10(BDEPTH).burst_depth_extra,
        at1(BDEPTH).burst_depth_extra
    );
    assert_eq!(at10(CAND).extra_candidates, at1(CAND).extra_candidates);
    assert_eq!(
        at10(REGEN).regen,
        level_scaled(crate::sim::slots::bundled_rules(), 2, 10)
    );
    assert_eq!(at10(RANGE).range_pct, 12);
    // The caps still apply after the scaling.
    let two = Build::of_levelled(&stage, &rewards, &[(CRIT, 10), (CRIT, 10)]);
    assert_eq!(two.crit_chance_pct, MAX_CRIT_CHANCE_TOTAL_PCT);
    // A set's tier delta is the same at any level of its members.
    let (low, high) = (
        Build::of_levelled(&stage, &rewards, &[(N1, 1), (N2, 1)]),
        Build::of_levelled(&stage, &rewards, &[(N1, 10), (N2, 10)]),
    );
    assert_eq!(low.kill_heal, high.kill_heal);
    assert_eq!(low.kill_heal_every, high.kill_heal_every);
    assert_eq!(low.burst_radius, high.burst_radius);
}

/// T8: a level change rewrites the totals by the difference and heals a full player; a drop
/// returns them exactly.
#[test]
fn a_level_up_rewrites_the_player_s_numbers() {
    let (stage, rewards) = (stage(), hand());
    let mut s = start(&stage, 23);
    let (max, atk) = (
        s.players[PlayerIdx::SOLO].max_hp,
        total(&s, StatEffect::AttackPct),
    );
    let change = |s: &mut State, from: &[(u16, u8)], to: &[(u16, u8)]| {
        let before = Build::of_levelled(&stage, &rewards, from);
        let after = Build::of_levelled(&stage, &rewards, to);
        let (player, career) = s.players.both_mut(PlayerIdx::SOLO);
        let o = career.offers.as_mut().expect("offers");
        Build::write(
            &before,
            &after,
            player,
            &mut career.progress,
            &mut career.regen,
            o,
        );
    };
    change(&mut s, &[], &[(ATK, 1), (HP, 1)]);
    s.players[PlayerIdx::SOLO].hp = s.players[PlayerIdx::SOLO].max_hp;
    change(&mut s, &[(ATK, 1), (HP, 1)], &[(ATK, 10), (HP, 10)]);
    assert_eq!(total(&s, StatEffect::AttackPct), atk + 6);
    assert_eq!(s.players[PlayerIdx::SOLO].max_hp, max + 127);
    assert_eq!(s.players[PlayerIdx::SOLO].hp, max + 127);
    change(&mut s, &[(ATK, 10), (HP, 10)], &[(ATK, 1), (HP, 1)]);
    assert_eq!(total(&s, StatEffect::AttackPct), atk + 5);
    assert_eq!(s.players[PlayerIdx::SOLO].max_hp, max + 100);
    // A downed player stays at 0 hit points.
    s.players[PlayerIdx::SOLO].hp = 0;
    change(&mut s, &[(ATK, 1), (HP, 1)], &[(ATK, 10), (HP, 10)]);
    assert_eq!(s.players[PlayerIdx::SOLO].hp, 0);
}
