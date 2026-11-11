//! Every `State` type: the whole state of a run, hashed by `StableHash` (`state_hash.rs`, ADR-SIM-3).

use crate::attack::DamageType;
use crate::engine::Outcome;
use crate::fixed::Point;
use crate::id::{ContentId, EntityId};
use crate::kinds::KindTable;
use crate::match_ups::MatchUps;
use crate::rewards::Rarity;
use crate::rng::{SplitMix64, TAG_ATTACK, TAG_HORDE, TAG_LOOT, TAG_SPAWN, derive};
use crate::run_stats::StatEffect;
use crate::runes::Runes;
use crate::sim::RangedPool;
use std::sync::Arc;

mod attack_run;
mod foes;
mod orbs_zones;
mod player_state;
mod players;
mod status;

pub(crate) use attack_run::*;
pub(crate) use foes::*;
pub(crate) use orbs_zones::*;
pub(crate) use player_state::*;
pub(crate) use players::*;
pub(crate) use status::*;

/// Firings or casts left when there is no limit.
pub(crate) const UNLIMITED: u32 = u32::MAX;

/// The run's generators, one per purpose, so a change to one kind of draw leaves the others
/// where they were (ADR-SIM-2). Each is `rng::derive(seed, tag)`; the hash writes them in this
/// order. The offer generator is per player slot, in `Career::offer` (ADR-SIM-7): the hash and
/// savepoint format 1 write slot 0's between `loot` and `horde`, where it was.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Streams {
    /// The angle of a wave's ring spawn.
    pub spawn: SplitMix64,
    /// An attack's random numbers.
    pub attack: SplitMix64,
    /// Heal drops, star drops and a star's rarity.
    pub loot: SplitMix64,
    /// The horde's boss and elite draws.
    pub horde: SplitMix64,
}

impl Streams {
    pub fn new(seed: u64) -> Self {
        Self {
            spawn: derive(seed, TAG_SPAWN),
            attack: derive(seed, TAG_ATTACK),
            loot: derive(seed, TAG_LOOT),
            horde: derive(seed, TAG_HORDE),
        }
    }

    /// The four states, in hash order (the offer generator is each career's).
    #[cfg(test)]
    pub fn states(&self) -> [u64; 4] {
        [
            self.spawn.state(),
            self.attack.state(),
            self.loot.state(),
            self.horde.state(),
        ]
    }
}

/// The whole state of a run. Everything here is in the state hash (`StableHash`, in
/// `state_hash.rs`) except the fields that impl marks `_  // not hashed: <why>`: content, derived
/// values and buffers.
#[derive(Debug)]
pub(crate) struct State {
    /// Buffers reused from tick to tick; not part of the state (not hashed, not cloned).
    pub scratch: crate::sim::Scratch,
    /// The run's kinds, fixed at its start (core#271).
    pub kinds: Arc<KindTable>,
    pub tick: u32,
    pub rng: Streams,
    pub outcome: Outcome,
    /// The players, by slot (`Players::id` is a slot's entity number now): one now (ADR-SIM-7).
    pub players: Players,
    pub boss: Option<BossState>,
    /// The allies that act on their own, in entity-ID order.
    pub companions: Vec<AgentState>,
    /// In spawn order, which is entity-ID order.
    pub enemies: Vec<AgentState>,
    /// Attacks that are being carried out, oldest first (ADR-SIM-4 tier 3).
    pub tasks: Vec<AttackTask>,
    /// Phased attacks in progress, in the order they started.
    pub runs: Vec<AttackRun>,
    /// What `runs` leave each attacker's speed; updated wherever `runs` changes.
    pub move_index: MoveIndex,
    /// The ranged tokens cooling (RA1, DS83): the tick each is free on and its pool, sorted by
    /// both. At most `MAX_RANGED_ATTACKERS`: a pool grants no more tokens than the ones neither
    /// held nor cooling.
    pub ranged_cooling: Vec<(u32, RangedPool)>,
    /// In spawn order.
    pub orbs: Vec<OrbState>,
    /// Zones placed and not yet over, in the order they were placed (core Z1).
    pub zones: Vec<ZoneState>,
    /// Enemies spawned so far, per wave.
    pub spawned: Vec<u16>,
    /// The next entity number. It starts at `MAX_PLAYERS`: the numbers 0 to 2 are the player slots,
    /// present or not (ADR-SIM-7). The hash and the savepoint write it, like every entity number,
    /// as it is.
    pub next_id: u32,
    /// Cage groups taken so far (Z3a-3).
    pub next_orb_group: u32,
    /// Falling stars and offers; `None` in a stage without stars.
    pub stars: Option<StarField>,
    /// Heal pickups on the map, in the order they dropped; `None` in a stage without healing.
    pub heal_pickups: Option<Vec<HealPickup>>,
    /// What each match-up does to a hit: content, fixed for the run and covered by
    /// the content hash, so the state hash leaves it out.
    pub match_ups: MatchUps,
    /// What each on-hit rune does when it fires: content, as the match-ups are.
    pub runes: Arc<Runes>,
    /// The stun rules: content, as the match-ups are.
    pub stun: crate::run_stats::StunRules,
    /// The cap of the attack speed total (app#298): content, as the stun rules are.
    pub max_attack_speed_pct: u32,
    /// The damage an attack does when an enemy or the boss makes it and the attack names none:
    /// the run stats file's, content as the stun rules are.
    pub enemy_attack_damage: u32,
    /// The cap of Rune Haste (app#298): the rewards file's, held to the sum of the picks where
    /// it applies.
    pub max_rune_haste_pct: u32,
    /// The bosses of a `time_limit` run, as they come; `None` in any other run.
    pub horde: Option<Horde>,
    /// The percents the run's difficulty scales enemies' hit points and attacks' damage by
    ///: fixed for the run, part of the setup, and hashed with it.
    pub hp_pct: u32,
    pub damage_pct: u32,
    /// The run's seed, which the critical-hit rolls are hashed from: they draw nothing from
    /// `rng`, so a run without crit chance spends the same random numbers as before.
    pub crit_seed: u64,
    /// The echoes of last tick's hits, waiting for this tick's `collide`, in the order the hits
    /// landed; at most `MAX_ECHOES_PER_TICK`.
    pub echoes: Vec<Echo>,
}

impl Clone for State {
    fn clone(&self) -> State {
        let State {
            scratch,
            kinds,
            tick,
            rng,
            outcome,
            players,
            boss,
            companions,
            enemies,
            tasks,
            runs,
            move_index,
            ranged_cooling,
            orbs,
            zones,
            spawned,
            next_id,
            next_orb_group,
            stars,
            heal_pickups,
            match_ups,
            runes,
            stun,
            max_attack_speed_pct,
            enemy_attack_damage,
            max_rune_haste_pct,
            horde,
            hp_pct,
            damage_pct,
            crit_seed,
            echoes,
        } = self;
        State {
            scratch: scratch.clone(),
            kinds: kinds.clone(),
            tick: *tick,
            rng: rng.clone(),
            outcome: *outcome,
            players: players.clone(),
            boss: boss.clone(),
            companions: companions.clone(),
            enemies: enemies.clone(),
            tasks: tasks.clone(),
            runs: runs.clone(),
            move_index: move_index.clone(),
            ranged_cooling: ranged_cooling.clone(),
            orbs: orbs.clone(),
            zones: zones.clone(),
            spawned: spawned.clone(),
            next_id: *next_id,
            next_orb_group: *next_orb_group,
            stars: stars.clone(),
            heal_pickups: heal_pickups.clone(),
            match_ups: *match_ups,
            runes: runes.clone(),
            stun: *stun,
            max_attack_speed_pct: *max_attack_speed_pct,
            enemy_attack_damage: *enemy_attack_damage,
            max_rune_haste_pct: *max_rune_haste_pct,
            horde: horde.clone(),
            hp_pct: *hp_pct,
            damage_pct: *damage_pct,
            crit_seed: *crit_seed,
            echoes: echoes.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &State) {
        let State {
            scratch,
            kinds,
            tick,
            rng,
            outcome,
            players,
            boss,
            companions,
            enemies,
            tasks,
            runs,
            move_index,
            ranged_cooling,
            orbs,
            zones,
            spawned,
            next_id,
            next_orb_group,
            stars,
            heal_pickups,
            match_ups,
            runes,
            stun,
            max_attack_speed_pct,
            enemy_attack_damage,
            max_rune_haste_pct,
            horde,
            hp_pct,
            damage_pct,
            crit_seed,
            echoes,
        } = self;
        let State {
            scratch: s_scratch,
            kinds: s_kinds,
            tick: s_tick,
            rng: s_rng,
            outcome: s_outcome,
            players: s_players,
            boss: s_boss,
            companions: s_companions,
            enemies: s_enemies,
            tasks: s_tasks,
            runs: s_runs,
            move_index: s_move_index,
            ranged_cooling: s_ranged_cooling,
            orbs: s_orbs,
            zones: s_zones,
            spawned: s_spawned,
            next_id: s_next_id,
            next_orb_group: s_next_orb_group,
            stars: s_stars,
            heal_pickups: s_heal_pickups,
            match_ups: s_match_ups,
            runes: s_runes,
            stun: s_stun,
            max_attack_speed_pct: s_max_attack_speed_pct,
            enemy_attack_damage: s_enemy_attack_damage,
            max_rune_haste_pct: s_max_rune_haste_pct,
            horde: s_horde,
            hp_pct: s_hp_pct,
            damage_pct: s_damage_pct,
            crit_seed: s_crit_seed,
            echoes: s_echoes,
        } = source;
        scratch.clone_from(s_scratch);
        kinds.clone_from(s_kinds);
        *tick = *s_tick;
        rng.clone_from(s_rng);
        *outcome = *s_outcome;
        players.clone_from(s_players);
        boss.clone_from(s_boss);
        companions.clone_from(s_companions);
        enemies.clone_from(s_enemies);
        tasks.clone_from(s_tasks);
        runs.clone_from(s_runs);
        move_index.clone_from(s_move_index);
        ranged_cooling.clone_from(s_ranged_cooling);
        orbs.clone_from(s_orbs);
        zones.clone_from(s_zones);
        spawned.clone_from(s_spawned);
        *next_id = *s_next_id;
        *next_orb_group = *s_next_orb_group;
        stars.clone_from(s_stars);
        heal_pickups.clone_from(s_heal_pickups);
        *match_ups = *s_match_ups;
        runes.clone_from(s_runes);
        *stun = *s_stun;
        *max_attack_speed_pct = *s_max_attack_speed_pct;
        *enemy_attack_damage = *s_enemy_attack_damage;
        *max_rune_haste_pct = *s_max_rune_haste_pct;
        horde.clone_from(s_horde);
        *hp_pct = *s_hp_pct;
        *damage_pct = *s_damage_pct;
        *crit_seed = *s_crit_seed;
        echoes.clone_from(s_echoes);
    }
}

/// One hit's echo, queued when the hit lands and dealt a tick later (core#NS2b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Echo {
    pub source: EntityId,
    pub target: EntityId,
    /// Hit points taken, already the hit's percent of its damage.
    pub damage: u32,
    /// The type of the hit it echoes, which the echo's `Hit` event reports.
    pub damage_type: Option<DamageType>,
}

/// How far a `time_limit` run's bosses have come (core#212): the times, and which pool entries
/// are used up. What the bosses are is `World::horde` and `World::bosses`, fixed for the run.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Horde {
    pub boss_ticks: Vec<u32>,
    /// The boss times at which the boss calls no elites (core#336).
    pub no_call_ticks: Vec<u32>,
    /// The mid-bosses' hit points by slot, in percent (core#336).
    pub mid_boss_hp_pct: Vec<u32>,
    /// Bosses that have come, mid-bosses and the final one.
    pub spawned: u32,
    /// One per pool entry: drawn already.
    pub used: Vec<bool>,
    /// The spawn director's, per wave (DIR1): the ticks the wave's schedule has slid, one for
    /// each tick a spawn that was due found the crowd at its target. Zero for a wave never held.
    pub held: Vec<u32>,
}

impl Clone for Horde {
    fn clone(&self) -> Horde {
        let Horde {
            boss_ticks,
            no_call_ticks,
            mid_boss_hp_pct,
            spawned,
            used,
            held,
        } = self;
        Horde {
            boss_ticks: boss_ticks.clone(),
            no_call_ticks: no_call_ticks.clone(),
            mid_boss_hp_pct: mid_boss_hp_pct.clone(),
            spawned: *spawned,
            used: used.clone(),
            held: held.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &Horde) {
        let Horde {
            boss_ticks,
            no_call_ticks,
            mid_boss_hp_pct,
            spawned,
            used,
            held,
        } = self;
        let Horde {
            boss_ticks: s_boss_ticks,
            no_call_ticks: s_no_call_ticks,
            mid_boss_hp_pct: s_mid_boss_hp_pct,
            spawned: s_spawned,
            used: s_used,
            held: s_held,
        } = source;
        boss_ticks.clone_from(s_boss_ticks);
        no_call_ticks.clone_from(s_no_call_ticks);
        mid_boss_hp_pct.clone_from(s_mid_boss_hp_pct);
        *spawned = *s_spawned;
        used.clone_from(s_used);
        held.clone_from(s_held);
    }
}

/// The one expertise gauge of the one weapon, and its offers (ADR-REPL-5).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExpertiseGauge {
    /// From 0.
    pub level: u32,
    /// Points toward the next level; at most `next - 1` between ticks.
    pub points: u32,
    /// The points the next level needs.
    pub next: u32,
    /// What the range options taken add up to: every combo step's hit box that much wider,
    /// with the range rewards'.
    pub range_pct: u32,
    /// The weapon's style, a place among its three, once chosen at the level table's style
    /// level; held for the run.
    pub style: Option<u8>,
    /// The places of the style's choices taken at the upgrade and capstone levels, in level
    /// order.
    pub path: Vec<u8>,
}

impl Clone for ExpertiseGauge {
    fn clone(&self) -> ExpertiseGauge {
        let ExpertiseGauge {
            level,
            points,
            next,
            range_pct,
            style,
            path,
        } = self;
        ExpertiseGauge {
            level: *level,
            points: *points,
            next: *next,
            range_pct: *range_pct,
            style: *style,
            path: path.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &ExpertiseGauge) {
        let ExpertiseGauge {
            level,
            points,
            next,
            range_pct,
            style,
            path,
        } = self;
        let ExpertiseGauge {
            level: s_level,
            points: s_points,
            next: s_next,
            range_pct: s_range_pct,
            style: s_style,
            path: s_path,
        } = source;
        *level = *s_level;
        *points = *s_points;
        *next = *s_next;
        *range_pct = *s_range_pct;
        *style = *s_style;
        path.clone_from(s_path);
    }
}

/// The run part of the star field: falling stars and what the run's setup fixes for every
/// player's offers (ADR-REPL-5). What is left of each player's offers is per slot in `Offers`
/// (ADR-SIM-7); the live rules open none.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StarField {
    /// Kills in a row that dropped no star, for the pity.
    pub misses: u32,
    /// The stars waiting on the map, in the order they dropped (entity-ID order).
    pub on_map: Vec<StarState>,
    /// The rarities its stars and offers may have, lowest and highest: fixed for the
    /// run, part of the setup, and hashed with it.
    pub rarity: (Rarity, Rarity),
    /// The runes the random rune may draw, by `Rune::index`: fixed for the run, part
    /// of the setup, and hashed with it.
    pub rune_pool: u8,
    /// The run's pool: places in the rewards file, in its order, fixed for the run.
    pub pool: Vec<u16>,
}

impl Clone for StarField {
    fn clone(&self) -> StarField {
        let StarField {
            misses,
            on_map,
            rarity,
            rune_pool,
            pool,
        } = self;
        StarField {
            misses: *misses,
            on_map: on_map.clone(),
            rarity: *rarity,
            rune_pool: *rune_pool,
            pool: pool.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &StarField) {
        let StarField {
            misses,
            on_map,
            rarity,
            rune_pool,
            pool,
        } = self;
        let StarField {
            misses: s_misses,
            on_map: s_on_map,
            rarity: s_rarity,
            rune_pool: s_rune_pool,
            pool: s_pool,
        } = source;
        *misses = *s_misses;
        on_map.clone_from(s_on_map);
        *rarity = *s_rarity;
        *rune_pool = *s_rune_pool;
        pool.clone_from(s_pool);
    }
}

/// One player's star offers, what is left of them (ADR-REPL-5). A slot holds `Some` exactly when
/// the stage has stars (`State::stars`). The live rules open no offer, so the state holds none.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Offers {
    /// For each set of the run's constellations: the members held, counted after every take
    /// (core#NS3).
    pub set_counts: Vec<u8>,
}

impl Clone for Offers {
    fn clone(&self) -> Offers {
        let Offers { set_counts } = self;
        Offers {
            set_counts: set_counts.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &Offers) {
        let Offers { set_counts } = self;
        let Offers {
            set_counts: s_set_counts,
        } = source;
        set_counts.clone_from(s_set_counts);
    }
}

/// A heal pickup on the map.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct HealPickup {
    pub id: EntityId,
    pub at: Point,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StarState {
    pub id: EntityId,
    pub at: Point,
    pub rarity: Rarity,
}

/// What a run stat does, from the run stats file.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct StatRule {
    pub id: ContentId,
    pub effect: StatEffect,
    pub step: u32,
    pub limit: u8,
}

/// Experience, levels and stat points (ADR-REPL-5; DS18).
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Progress {
    /// From 1.
    pub level: u32,
    /// Experience toward the next level.
    pub xp: u32,
    /// The run stats' rules, in the run stats' order: content, fixed for the run, not hashed.
    pub stats: Vec<StatRule>,
    /// Every stat's place in `stats`, in the order a level-up spends its point on them: the
    /// setup's stat priority, then the stats it leaves out in their own order.
    pub priority: Vec<u8>,
    /// Points spent on each run stat, in the run stats' order.
    pub spent: Vec<u8>,
    /// What the points spent add up to for each effect, in `StatEffect::ALL` order.
    pub totals: [u32; StatEffect::ALL.len()],
    /// What the weapon's stat set adds for each effect, in tenths of a percent (`MaxHp`'s set is
    /// whole and lives in `totals`), in `StatEffect::ALL` order. Only the set's writer
    /// (`expertise.rs`) touches it; a read adds `value / 10` to the whole percent in `totals`.
    pub set_tenths: [u32; StatEffect::ALL.len()],
}

impl Clone for Progress {
    fn clone(&self) -> Progress {
        let Progress {
            level,
            xp,
            stats,
            priority,
            spent,
            totals,
            set_tenths,
        } = self;
        Progress {
            level: *level,
            xp: *xp,
            stats: stats.clone(),
            priority: priority.clone(),
            spent: spent.clone(),
            totals: *totals,
            set_tenths: *set_tenths,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &Progress) {
        let Progress {
            level,
            xp,
            stats,
            priority,
            spent,
            totals,
            set_tenths,
        } = self;
        let Progress {
            level: s_level,
            xp: s_xp,
            stats: s_stats,
            priority: s_priority,
            spent: s_spent,
            totals: s_totals,
            set_tenths: s_set_tenths,
        } = source;
        *level = *s_level;
        *xp = *s_xp;
        stats.clone_from(s_stats);
        priority.clone_from(s_priority);
        spent.clone_from(s_spent);
        *totals = *s_totals;
        *set_tenths = *s_set_tenths;
    }
}

#[cfg(test)]
mod status_kind_tests {
    use super::StatusKind;

    #[test]
    fn every_status_kind_is_listed() {
        // An exhaustive match: a kind added without a number here does not compile, and one
        // without a place in `ALL` fails the assert below, so `Statuses` is sized for all.
        let number = |k: StatusKind| match k {
            StatusKind::AtkUp => 0,
            StatusKind::Vulnerable => 1,
            StatusKind::Slow => 2,
            StatusKind::Bleed => 3,
            StatusKind::Poison => 4,
            StatusKind::Burn => 5,
            StatusKind::Stun => 6,
        };
        let numbers: Vec<usize> = StatusKind::ALL.into_iter().map(number).collect();
        assert_eq!(numbers, (0..StatusKind::COUNT).collect::<Vec<_>>());
    }

    #[test]
    fn a_status_kind_is_its_own_id() {
        for (i, kind) in StatusKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, i);
            assert_eq!(kind.id().as_str(), kind.name());
        }
    }
}
