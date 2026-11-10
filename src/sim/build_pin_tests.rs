//! Pins for the build the equipment writes (ADR-SIM-3, ADR-SIM-2 point 2, ADR-REPL-5): the build
//! of the held counts, and what a rise or a fall of it leaves in the player and the career,
//! worked out by hand from a small rewards file, not copied from a run.

use super::build::Build;
use super::state::{PlayerIdx, State};
use super::{World, initial_state};
use crate::constellations::Constellations;
use crate::rewards::{MAX_CRIT_CHANCE_TOTAL_PCT, Rarity, Rewards};
use crate::run_content::RunContent;
use crate::run_stats::StatEffect;
use crate::sim::{RunDifficulty, RunPool};
use crate::version::Versions;
use serde_json::{Value, json};

/// The places of the hand-built file.
const ATK: u16 = 0;
const HP: u16 = 1;
const CRIT: u16 = 2;
const HEAL_A: u16 = 5;
const BURST_A: u16 = 7;
const BDEPTH: u16 = 9;
const RANGE: u16 = 10;
const PYRO: u16 = 14;
const REGEN: u16 = 16;
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

/// The clamps of the write: a stage without levels ignores the stats, a player at 0 hit points
/// gets the most and not the heal, and every hit points star at its stack limit adds the exact sum.
#[test]
fn a_build_write_keeps_its_clamps() {
    let stage = stage();
    let rewards = hand();
    // A stage without levels ignores the stats.
    let mut s = start(&stage, 23);
    s.players.career_mut(PlayerIdx::SOLO).progress = None;
    let before = s.players[PlayerIdx::SOLO].max_hp;
    write_change(&stage, &rewards, &mut s, &[], &[HP]);
    assert_eq!(s.players[PlayerIdx::SOLO].max_hp, before);
    // A player at 0 hit points gets the most, not the heal.
    let mut s = start(&stage, 23);
    s.players[PlayerIdx::SOLO].hp = 0;
    let max = s.players[PlayerIdx::SOLO].max_hp;
    write_change(&stage, &rewards, &mut s, &[], &[HP]);
    assert_eq!(s.players[PlayerIdx::SOLO].max_hp, max + 100);
    assert_eq!(s.players[PlayerIdx::SOLO].hp, 0);
    // Every hit points star at its stack limit adds the exact sum.
    let mut s = start(&stage, 23);
    let max = s.players[PlayerIdx::SOLO].max_hp;
    write_change(&stage, &rewards, &mut s, &[], &[HP; 20]);
    assert_eq!(s.players[PlayerIdx::SOLO].max_hp, max + 2_000);
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
    Build::write(
        &before,
        &after,
        player,
        &mut career.progress,
        &mut career.regen,
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
    // Counts are not scaled: burst depth.
    assert_eq!(
        at10(BDEPTH).burst_depth_extra,
        at1(BDEPTH).burst_depth_extra
    );
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
        Build::write(
            &before,
            &after,
            player,
            &mut career.progress,
            &mut career.regen,
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
