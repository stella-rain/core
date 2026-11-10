//! The build of a player: what the stars held add up to (ADR-SIM-3, ADR-REPL-5).
//!
//! A pure function of the rewards file and the stars held, each with its slot's level, so the stars a player
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
    MAX_CRIT_DAMAGE_TOTAL_PCT, MAX_ECHO_HIT_PCT, MAX_ORBITS, MAX_ORBS, ORBIT_SET_BASE,
    RewardEffect, Rewards,
};
use crate::run_stats::StatEffect;
use crate::sim::state::{Offers, OrbitState, PlayerState, Progress, SkillSlot};
use crate::stage_skill::MAX_SKILL_LEVELS;
use crate::validate::limits::MAX_EVADE_CHARGES;

use super::slots::level_scaled;
use super::{World, levels, player};

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
        let count = |p: usize| usize::from(held.get(p).copied().unwrap_or(0));
        let mut b = Build::default();
        for (p, reward) in rewards.rewards().iter().enumerate() {
            for _ in 0..count(p) {
                b.add(stage, &reward.effects, 1);
            }
        }
        b.add_tiers(stage, rewards, count);
        b
    }

    /// The build of the stars equipped, each `(place, level)`: a star's own amounts scale with
    /// its slot's level (`slots::level_scaled`, level 1 is the base), a set's tier deltas do not.
    pub(super) fn of_levelled(stage: &World, rewards: &Rewards, stars: &[(u16, u8)]) -> Build {
        let mut b = Build::default();
        for &(place, level) in stars {
            if let Some(reward) = rewards.rewards().get(usize::from(place)) {
                b.add(stage, &reward.effects, level);
            }
        }
        b.add_tiers(stage, rewards, |p| {
            stars.iter().filter(|s| usize::from(s.0) == p).count()
        });
        b
    }

    /// Adds the chain of tier deltas of each set, from how many of each member `count` says are held.
    fn add_tiers(&mut self, stage: &World, rewards: &Rewards, count: impl Fn(usize) -> usize) {
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
                    self.add(stage, &constellations::tier_delta(lower, tier), 1);
                }
            }
        }
    }

    /// Adds what `effects` build, once. The grants (a skill, an orbit, an evade charge, a draw,
    /// rerolls and banishes) are not a build: they are `stars::grant`'s, and listed here under
    /// the one arm so that a new `RewardEffect` fails to compile until it is placed.
    fn add(&mut self, stage: &World, effects: &[EffectEntry], level: u8) {
        for entry in effects {
            // Every effect but the random rune's has an amount (`rewards::validate`). The slot's
            // level scales it, except the counts (`BurstDepth`, `OfferCandidates`), which use
            // `base`; the intervals, radius and depth are other fields and are not scaled.
            let base = entry.amount.unwrap_or(0);
            let amount = level_scaled(&stage.equipment_rules, base, level);
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
                        .saturating_add(base)
                        .min(MAX_BURST_DEPTH)
                }
                RewardEffect::Regen => self.regen = self.regen.saturating_add(amount),
                // Equipment mode keeps the Growth effects as stats (their `base`, not level-scaled,
                // with no cap here); the room under the offer size is the reader's.
                RewardEffect::Rerolls | RewardEffect::Banishes | RewardEffect::OfferCandidates => {
                    if let Some(stat) = entry.effect.growth_stat() {
                        let total = &mut self.stats[stat as usize];
                        *total = total.saturating_add(base);
                    }
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
    /// A fall (an unequip) takes the difference off the mixed fields: `regen` and the totals
    /// fall by the stars' share, `MaxHp` through `levels::remove_effect` (no heal, hit points
    /// kept within the most, never 0). A write can raise one stat and lower another.
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
        // The stars' share is part of `regen`: a rise adds the difference, a fall takes it off.
        *regen = regen
            .saturating_add(more_regen.saturating_sub(before.regen))
            .saturating_sub(before.regen.saturating_sub(more_regen));
        if let Some(p) = progress.as_mut() {
            for (stat, (&now, &was)) in StatEffect::ALL.iter().zip(stats.iter().zip(&before.stats))
            {
                if now > was {
                    levels::add_effect(p, player, *stat, now - was);
                } else if now < was {
                    levels::remove_effect(p, player, *stat, was - now);
                }
            }
        }
    }
}

/// What the stars equipped grant besides the build, as a function of the set (equipment mode):
/// the stage skill, the orbits, the evade charges. The derived
/// value does not depend on the order the stars were equipped in, only on their slot order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Grants {
    /// The first equipped star with a stage skill, in slot order: the skill's place in the stage
    /// skills and its level (the entry's amount, held to the skill's levels).
    skill: Option<(u16, u8)>,
    /// Every equipped orbit, in slot order: the key (the star's place, or `ORBIT_SET_BASE` plus a
    /// set's place for a tier's orbit) and its orbs.
    orbits: Vec<(u16, u8)>,
    /// The evade charges the stars add to the stage's own.
    evade_charges: u32,
}

impl Grants {
    /// The skill the grants hold: its place in the stage skills and its level.
    pub(super) fn skill(&self) -> Option<(u16, u8)> {
        self.skill
    }

    /// The grants of `equipped`, the places of the stars equipped in slot order (the Lumine slot
    /// last), and of the tiers their sets reach. A random rune in a bundle grants nothing in
    /// equipment mode: runes come from drops.
    pub(super) fn of(rewards: &Rewards, equipped: &[u16]) -> Grants {
        let mut g = Grants::default();
        for &place in equipped {
            if let Some(reward) = rewards.rewards().get(usize::from(place)) {
                g.add(rewards, place, &reward.effects);
            }
        }
        let sets = rewards.constellations();
        for (set, c) in sets.sets().iter().enumerate() {
            let members = c
                .members
                .iter()
                .filter(|m| {
                    rewards
                        .index_of(m)
                        .and_then(|p| u16::try_from(p).ok())
                        .is_some_and(|p| equipped.contains(&p))
                })
                .count();
            for (t, tier) in c.tiers.iter().enumerate() {
                if usize::from(tier.threshold) <= members {
                    let lower = t.checked_sub(1).map(|p| &c.tiers[p]);
                    let delta = constellations::tier_delta(lower, tier);
                    // A tier's orbit is keyed on its set (`Rewards::orbit_def`).
                    let key = ORBIT_SET_BASE + u16::try_from(set).unwrap_or(0);
                    g.add(rewards, key, &delta);
                }
            }
        }
        g
    }

    /// Adds what `effects` grant, once. One arm per `RewardEffect` with no `_`, so a new kind
    /// fails to compile until it is placed (the build's are `Build::add`'s).
    fn add(&mut self, rewards: &Rewards, key: u16, effects: &[EffectEntry]) {
        for entry in effects {
            // Every effect but the random rune's has an amount (`rewards::validate`).
            let amount = entry.amount.unwrap_or(0);
            match entry.effect {
                RewardEffect::StageSkill if self.skill.is_none() => {
                    let index = entry
                        .skill
                        .as_ref()
                        .and_then(|id| rewards.stage_skills().index_of(id));
                    if let Some(index) = index
                        && let Some(def) = rewards.stage_skills().get(index)
                    {
                        let most = u32::try_from(def.levels.len().min(MAX_SKILL_LEVELS))
                            .unwrap_or(u32::MAX);
                        self.skill = Some((
                            u16::try_from(index).unwrap_or(u16::MAX),
                            u8::try_from(amount.min(most)).unwrap_or(u8::MAX),
                        ));
                    }
                }
                RewardEffect::Orbit => {
                    let orbs = u8::try_from(amount).unwrap_or(u8::MAX);
                    if let Some(o) = self.orbits.iter_mut().find(|o| o.0 == key) {
                        o.1 = o.1.saturating_add(orbs).min(MAX_ORBS);
                    } else if self.orbits.len() < MAX_ORBITS {
                        self.orbits.push((key, orbs.min(MAX_ORBS)));
                    }
                }
                RewardEffect::EvadeCharge => {
                    self.evade_charges = self.evade_charges.saturating_add(amount)
                }
                // A second skill star: the first in slot order holds the slot.
                RewardEffect::StageSkill
                // Runes come from drops in equipment mode: nothing is drawn, no stream moves.
                | RewardEffect::RandomRune
                // The build's (`Build::add`).
                | RewardEffect::MaxHp
                | RewardEffect::AttackPct
                | RewardEffect::AttackSpeedPct
                | RewardEffect::EvadePct
                | RewardEffect::SpeedPct
                | RewardEffect::XpPct
                | RewardEffect::RangePct
                | RewardEffect::TypeBonusPct
                | RewardEffect::AttackTypeBonusPct
                | RewardEffect::ElementBonusPct
                | RewardEffect::RuneHaste
                | RewardEffect::Regen
                | RewardEffect::PickupRadiusPct
                | RewardEffect::SkillCooldownPct
                | RewardEffect::HealPerKills
                | RewardEffect::OfferCandidates
                | RewardEffect::Rerolls
                | RewardEffect::Banishes
                | RewardEffect::CritChancePct
                | RewardEffect::CritDamagePct
                | RewardEffect::EchoHitPct
                | RewardEffect::KillBurst
                | RewardEffect::BurstDepth => {}
            }
        }
    }

    /// Writes the change from `before` to `after` into the player and its offers. A field that
    /// did not change is not touched. The skill keeps its cooldown while its index is unchanged
    /// and a new one arrives with `entry_cooldown` (0 is ready); an orbit still equipped keeps
    /// its run state and a new one arrives ready; the evade charges follow the stage's own plus
    /// the stars', held to its most, and a fall never takes the charges below the stage's own.
    pub(super) fn write(
        before: &Grants,
        after: &Grants,
        stage: &World,
        player: &mut PlayerState,
        entry_cooldown: u32,
    ) {
        if before.skill != after.skill {
            let held = player.skill;
            player.skill = after.skill.map(|(index, level)| SkillSlot {
                index,
                level,
                cooldown_left: held
                    .filter(|h| h.index == index)
                    .map_or(entry_cooldown, |h| h.cooldown_left),
            });
        }
        if before.orbits != after.orbits {
            let held = std::mem::take(&mut player.orbits);
            player.orbits = after
                .orbits
                .iter()
                .map(|&(reward, orbs)| {
                    held.iter().find(|h| h.reward == reward).map_or(
                        OrbitState {
                            reward,
                            orbs,
                            round: 0,
                            cooldown_left: 0,
                            angle: 0,
                        },
                        |h| OrbitState { orbs, ..*h },
                    )
                })
                .collect();
        }
        if before.evade_charges != after.evade_charges {
            let evade = &stage.player.evade;
            let base = u32::from(evade.charges);
            let most = u32::from(evade.most_charges().min(MAX_EVADE_CHARGES)).max(base);
            let cap = |stars: u32| base.saturating_add(stars).min(most);
            let (was, now) = (cap(before.evade_charges), cap(after.evade_charges));
            if now > was {
                player::add_evade_charges(stage, player, now - was);
            } else if now < was {
                let held = player.evade_charges;
                let lowered = held.saturating_sub(u8::try_from(was - now).unwrap_or(u8::MAX));
                let floor = held.min(u8::try_from(base).unwrap_or(u8::MAX));
                player.evade_charges = lowered.max(floor);
            }
        }
    }
}
