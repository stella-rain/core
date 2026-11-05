//! The build of a player: what the stars held add up to (ADR-SIM-3, ADR-REPL-5).
//!
//! A pure function of the rewards file and how many of each star is held, so the stars a player
//! holds can be counted from an inventory as well as from the picks of an offer. It draws
//! nothing, reads no `State`, and nothing here is hashed or saved: `write` puts it into fields
//! the hash already covers.
//!
//! Why a recompute gives what adding one pick at a time gave, in any order: every number is a
//! `u32` (no float, so no order of summation); a sum is a `saturating_add` of non-negative
//! amounts, which is associative; a capped field is `min(CAP, sum)` at every step with `CAP`
//! below `u32::MAX`, which equals `min(CAP, sum of all)`; the deepest burst and the shortest heal
//! interval are a maximum and a minimum. A set's tiers are the exception that needs care:
//! `tier_delta` drops an entry whose amount does not rise, with its `kills` and `depth`, so the
//! build applies the chain of deltas (tier 1 from nothing, tier `t` from tier `t - 1`) and never
//! a tier's full totals.

use crate::attack::DamageType;
use crate::constellations;
use crate::rewards::{
    EffectEntry, MAX_BURST_DEPTH, MAX_BURST_PCT, MAX_BURST_RADIUS_PX, MAX_CRIT_CHANCE_TOTAL_PCT,
    MAX_CRIT_DAMAGE_TOTAL_PCT, MAX_ECHO_HIT_PCT, MAX_OFFER_CANDIDATES, RewardEffect, Rewards,
};
use crate::run_stats::StatEffect;
use crate::sim::state::{Offers, PlayerState, Progress};

use super::{World, levels};

/// What the stars held add up to, in the player's own numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Build {
    pub range_pct: u32,
    pub pickup_radius_pct: u32,
    pub skill_cooldown_pct: u32,
    pub kill_heal: u32,
    /// The shortest heal interval among the heal entries present, 0 while none.
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
    /// The build of the stars held: `held[p]` times the star at place `p` of `rewards` (a slice
    /// shorter than the file counts the rest as 0), and the chain of tier deltas of each set.
    pub(super) fn of(stage: &World, rewards: &Rewards, held: &[u8]) -> Build {
        let count = |p: usize| held.get(p).copied().unwrap_or(0);
        let mut b = Build::default();
        for (p, reward) in rewards.rewards().iter().enumerate() {
            for _ in 0..count(p) {
                b.add(stage, &reward.effects);
            }
        }
        let sets = rewards.constellations();
        for set in sets.sets() {
            let members = set
                .members
                .iter()
                .filter(|m| rewards.index_of(m).is_some_and(|p| count(p) > 0))
                .count();
            for (t, tier) in set.tiers.iter().enumerate() {
                if usize::from(tier.threshold) <= members {
                    let lower = t.checked_sub(1).map(|p| &set.tiers[p]);
                    b.add(stage, &constellations::tier_delta(lower, tier));
                }
            }
        }
        b
    }

    /// Adds what `effects` build, once. The grants (a skill, an orbit, an evade charge, a draw,
    /// rerolls and banishes) are not a build: they are `stars::grant`'s, and listed here under
    /// the one arm so that a new `RewardEffect` fails to compile until it is placed.
    fn add(&mut self, stage: &World, effects: &[EffectEntry]) {
        for entry in effects {
            // Every effect but the random rune's has an amount (`rewards::validate`).
            let amount = entry.amount.unwrap_or(0);
            match entry.effect {
                RewardEffect::RangePct => self.range_pct = self.range_pct.saturating_add(amount),
                RewardEffect::PickupRadiusPct => {
                    self.pickup_radius_pct = self.pickup_radius_pct.saturating_add(amount)
                }
                RewardEffect::SkillCooldownPct => {
                    self.skill_cooldown_pct = self.skill_cooldown_pct.saturating_add(amount)
                }
                RewardEffect::HealPerKills => {
                    // Every pick adds its amount and heals at the shortest interval among them.
                    let every = entry.kills.unwrap_or(0).max(1);
                    self.kill_heal = self.kill_heal.saturating_add(amount);
                    self.kill_heal_every = match self.kill_heal_every {
                        0 => every,
                        held => held.min(every),
                    };
                }
                RewardEffect::CritChancePct => {
                    self.crit_chance_pct = self
                        .crit_chance_pct
                        .saturating_add(amount)
                        .min(MAX_CRIT_CHANCE_TOTAL_PCT)
                }
                RewardEffect::CritDamagePct => {
                    self.crit_damage_pct = self
                        .crit_damage_pct
                        .saturating_add(amount)
                        .min(MAX_CRIT_DAMAGE_TOTAL_PCT)
                }
                RewardEffect::EchoHitPct => {
                    self.echo_hit_pct = self
                        .echo_hit_pct
                        .saturating_add(amount)
                        .min(MAX_ECHO_HIT_PCT)
                }
                RewardEffect::KillBurst => {
                    let (radius, depth) = (entry.radius.unwrap_or(0), entry.depth.unwrap_or(0));
                    self.burst_pct = self.burst_pct.saturating_add(amount).min(MAX_BURST_PCT);
                    self.burst_radius = self
                        .burst_radius
                        .saturating_add(radius)
                        .min(MAX_BURST_RADIUS_PX);
                    self.burst_depth = self.burst_depth.max(depth).min(MAX_BURST_DEPTH);
                }
                RewardEffect::BurstDepth => {
                    self.burst_depth_extra = self
                        .burst_depth_extra
                        .saturating_add(amount)
                        .min(MAX_BURST_DEPTH)
                }
                RewardEffect::Regen => self.regen = self.regen.saturating_add(amount),
                RewardEffect::OfferCandidates => {
                    let room = u8::try_from(MAX_OFFER_CANDIDATES - stage.tuning.offer_size)
                        .unwrap_or(u8::MAX);
                    let more = u8::try_from(amount).unwrap_or(u8::MAX);
                    self.extra_candidates = self.extra_candidates.saturating_add(more).min(room);
                }
                RewardEffect::RuneHaste => {
                    self.rune_haste_pct = self.rune_haste_pct.saturating_add(amount)
                }
                RewardEffect::TypeBonusPct
                | RewardEffect::AttackTypeBonusPct
                | RewardEffect::ElementBonusPct => {
                    for (ty, pct) in DamageType::ALL.iter().zip(&mut self.type_bonus_pct) {
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
                RewardEffect::MaxHp
                | RewardEffect::AttackPct
                | RewardEffect::AttackSpeedPct
                | RewardEffect::EvadePct
                | RewardEffect::SpeedPct
                | RewardEffect::XpPct => {
                    if let Some(stat) = entry.effect.stat() {
                        let total = &mut self.stats[stat as usize];
                        *total = total.saturating_add(amount);
                    }
                }
                RewardEffect::StageSkill
                | RewardEffect::EvadeCharge
                | RewardEffect::Orbit
                | RewardEffect::Rerolls
                | RewardEffect::Banishes
                | RewardEffect::RandomRune => {}
            }
        }
    }

    /// Writes the change from `before` to `after` into the run. The derived fields (the player's
    /// numbers and the extra candidates) are assigned from `after`; the mixed ones, which the
    /// level-ups, the perk and the gauge also add to (the stat totals, `regen`), take the
    /// difference. A stat goes through `levels::add_effect`, so `MaxHp` raises the most hit
    /// points and heals as it always did; a stage without levels ignores the stats.
    ///
    /// Lowering the mixed fields (an unequip) is not here: the assert below marks where that
    /// rule goes.
    pub(super) fn write(
        before: &Build,
        after: &Build,
        player: &mut PlayerState,
        progress: &mut Option<Progress>,
        regen: &mut u32,
        o: &mut Offers,
    ) {
        let Build {
            range_pct,
            pickup_radius_pct,
            skill_cooldown_pct,
            kill_heal,
            kill_heal_every,
            crit_chance_pct,
            crit_damage_pct,
            echo_hit_pct,
            burst_pct,
            burst_radius,
            burst_depth,
            burst_depth_extra,
            rune_haste_pct,
            regen: more_regen,
            type_bonus_pct,
            extra_candidates,
            stats,
        } = *after;
        debug_assert!(
            more_regen >= before.regen && stats.iter().zip(&before.stats).all(|(a, b)| a >= b),
            "a build that falls is the unequip rule's, not written here"
        );
        player.range_pct = range_pct;
        player.pickup_radius_pct = pickup_radius_pct;
        player.skill_cooldown_pct = skill_cooldown_pct;
        player.kill_heal = kill_heal;
        player.kill_heal_every = kill_heal_every;
        player.crit_chance_pct = crit_chance_pct;
        player.crit_damage_pct = crit_damage_pct;
        player.echo_hit_pct = echo_hit_pct;
        player.burst_pct = burst_pct;
        player.burst_radius = burst_radius;
        player.burst_depth = burst_depth;
        player.burst_depth_extra = burst_depth_extra;
        player.rune_haste_pct = rune_haste_pct;
        player.type_bonus_pct = type_bonus_pct;
        o.extra_candidates = extra_candidates;
        *regen = regen.saturating_add(more_regen.saturating_sub(before.regen));
        if let Some(p) = progress.as_mut() {
            for (stat, (&now, &was)) in StatEffect::ALL.iter().zip(stats.iter().zip(&before.stats))
            {
                if now > was {
                    levels::add_effect(p, player, *stat, now - was);
                }
            }
        }
    }
}
