//! The director, expertise and star state a run's progress ties together (ADR-SIM-3).

use crate::fixed::Point;
use crate::id::{ContentId, EntityId};
use crate::rewards::Rarity;
use crate::run_stats::StatEffect;

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
