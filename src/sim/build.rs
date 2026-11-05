//! The build of a player: what the stars held add up to (ADR-SIM-3, ADR-REPL-5).
//!
//! A pure function of the rewards file and how many of each star is held, so the stars a player
//! holds can be counted from an inventory as well as from the picks of an offer. Nothing here is
//! hashed or saved.

use crate::attack::DamageType;
use crate::rewards::Rewards;
use crate::run_stats::StatEffect;

use super::World;

/// What the stars held add up to, in the player's own numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)] // `take` uses it from the next commit
pub(super) struct Build {
    pub range_pct: u32,
    pub pickup_radius_pct: u32,
    pub skill_cooldown_pct: u32,
    pub kill_heal: u32,
    pub kill_heal_every: u32,
    pub crit_chance_pct: u32,
    pub crit_damage_pct: u32,
    pub echo_hit_pct: u32,
    pub burst_pct: u32,
    pub burst_radius: u32,
    pub burst_depth: u32,
    pub burst_depth_extra: u32,
    pub rune_haste_pct: u32,
    pub regen: u32,
    pub type_bonus_pct: [u32; DamageType::ALL.len()],
    pub extra_candidates: u8,
    /// The stars' share of `Progress::totals`, in `StatEffect::ALL` order.
    pub stats: [u32; StatEffect::ALL.len()],
}

impl Build {
    /// The build of the stars held: `held[p]` times the star at place `p` of `rewards`.
    #[allow(dead_code)]
    pub(super) fn of(_stage: &World, _rewards: &Rewards, _held: &[u8]) -> Build {
        Build::default()
    }
}
