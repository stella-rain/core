//! Rating the candidates of an open offer (ADR-REPL-16): the categories of a reward, the
//! expected gain in the main weapon's damage and the stats a pick moves. A read-only query on
//! the state as it is; nothing here is hashed, saved or recorded (ADR-SIM-17), and it is not part
//! of the snapshot (ADR-BRDG-13): call it once when an offer opens, not every frame.
//!
//! **The pick is run, not described.** Each candidate is taken with `stars::take` on a clone of
//! the player, so the set tier it completes is applied by the rules that apply it, and the stats
//! before and after are read from the two clones. A random rune is drawn from a generator seeded
//! 0 and dropped: it moves no stat shown and its draw leaks nothing (ADR-SIM-16).
//!
//! **The damage model** (`Rate`) has no tunable number (ADR-CONT-13), so no content key. It
//! rates the Main weapon's reference swing: the `main` chain of its combo (each step's `main`
//! after the first), a tap at the first level, no charge or hold. For each `Active` phase it
//! takes the damage of one hit through the formulas the rules use (`levels::attack_of`,
//! `attacks::swing_damage` with the type bonuses `attacks::capped_type_bonus` holds), the
//! expected crit (`(100 - c) * d + crit_damage(d) * c`, the roll being a hash that has the
//! chance `c` in the long run) and the echo (`100 + e` percent), and divides the sum by the
//! length of all the phases with the attack speed applied (`attacks::phase_ticks`, with its
//! tick rounding). The gain is the ratio of the two rates, in basis points rounded half up.
//!
//! **Its limits**, all the same on both sides of the ratio or left out on purpose: a neutral
//! target (no defence, match-up, status or party-size factor, which multiply alike and cancel);
//! one hit per active phase and one target; the echo's "the target stood" condition is dropped;
//! orbits, stage skills, runes, kill bursts, range, rune haste and transient atk-up statuses are
//! not rated (`dps_bp` is `None` for a pick that changes only those, but their stat changes show).

use crate::attack::DamageType;
use crate::attack::{AttackPhase, PhaseKind};
use crate::constellations;
use crate::id::PlayerIdx;
use crate::rewards::{EffectEntry, RewardEffect, Rewards};
use crate::rewards::{MAX_CRIT_CHANCE_TOTAL_PCT, MAX_CRIT_DAMAGE_TOTAL_PCT, MAX_ECHO_HIT_PCT};
use crate::rng::SplitMix64;
use crate::snapshot::{Category, OfferRating, StatChange, StatKind};

use super::state::{Offers, PlayerState, Progress, State};
use super::world::World;
use super::{attacks, combat, levels, stars};

/// The effect kinds that change a term the model rates.
fn rated(effect: RewardEffect) -> bool {
    matches!(
        effect,
        RewardEffect::AttackPct
            | RewardEffect::AttackSpeedPct
            | RewardEffect::CritChancePct
            | RewardEffect::CritDamagePct
            | RewardEffect::EchoHitPct
            | RewardEffect::TypeBonusPct
            | RewardEffect::AttackTypeBonusPct
            | RewardEffect::ElementBonusPct
    )
}

/// The damage rate of the reference swing, as a fraction: `damage` over `ticks`.
struct Rate {
    damage: u128,
    ticks: u128,
}

impl Rate {
    /// The rate for the stats `progress` and `player` hold; `None` without a weapon or an active
    /// phase to rate.
    fn of(
        world: &World,
        s: &State,
        progress: &Option<Progress>,
        player: &PlayerState,
    ) -> Option<Rate> {
        let atk = levels::attack_of(progress.as_ref(), world.player.atk);
        let speed = levels::attack_speed_of(progress.as_ref(), s.max_attack_speed_pct);
        let crit = u128::from(player.crit_chance_pct.min(MAX_CRIT_CHANCE_TOTAL_PCT));
        let echo = u128::from(player.echo_hit_pct.min(MAX_ECHO_HIT_PCT));
        let types = attacks::capped_type_bonus(&s.match_ups, &player.type_bonus_pct);
        let (mut damage, mut ticks) = (0u128, 0u128);
        let mut step = world.weapon()?.combo.get(false);
        while let Some(st) = step {
            let phases: &[AttackPhase] =
                &world.attacks[usize::from(world.attack_index(&st.attack).0)].phases;
            for (i, phase) in phases.iter().enumerate() {
                ticks += u128::from(attacks::phase_ticks(phases, speed, i));
                if phase.kind != PhaseKind::Active {
                    continue;
                }
                let hit =
                    attacks::swing_damage(phase, atk, 100, 0, Some(&types), world.tuning.defence_k);
                let crit_hit = combat::crit_damage(
                    hit,
                    player.crit_damage_pct.min(MAX_CRIT_DAMAGE_TOTAL_PCT),
                    world.tuning.crit_damage_base_pct,
                );
                damage +=
                    ((100 - crit) * u128::from(hit) + u128::from(crit_hit) * crit) * (100 + echo);
            }
            step = st.next.get(false);
        }
        (damage > 0 && ticks > 0).then_some(Rate { damage, ticks })
    }

    /// The gain of `after` over `self`, in basis points, rounded half up; never below 0.
    fn gain_bp(&self, after: &Rate) -> u32 {
        let num = after.damage * self.ticks * 10_000;
        let den = self.damage * after.ticks;
        let ratio = (num * 2 + den) / (2 * den);
        u32::try_from(ratio.saturating_sub(10_000)).unwrap_or(u32::MAX)
    }
}

/// The stats of a build that a pick can move, in `StatKind` order.
fn sheet(
    s: &State,
    progress: &Option<Progress>,
    player: &PlayerState,
    regen: u32,
    offers: &Offers,
) -> Vec<(StatKind, u32)> {
    use crate::run_stats::StatEffect as Total;
    let total = |effect: Total| progress.as_ref().map_or(0, |p| p.totals[effect as usize]);
    let mut out = vec![
        (StatKind::MaxHp, player.max_hp),
        (StatKind::Regen, regen),
        (StatKind::EvadePct, total(Total::EvadePct)),
        (StatKind::SpeedPct, total(Total::SpeedPct)),
        (StatKind::XpPct, total(Total::XpPct)),
        (StatKind::AttackPct, total(Total::AttackPct)),
        (
            StatKind::AttackSpeedPct,
            levels::attack_speed_of(progress.as_ref(), s.max_attack_speed_pct),
        ),
        (
            StatKind::CritChancePct,
            player.crit_chance_pct.min(MAX_CRIT_CHANCE_TOTAL_PCT),
        ),
        (
            StatKind::CritDamagePct,
            player.crit_damage_pct.min(MAX_CRIT_DAMAGE_TOTAL_PCT),
        ),
        (
            StatKind::EchoHitPct,
            player.echo_hit_pct.min(MAX_ECHO_HIT_PCT),
        ),
    ];
    let types = attacks::capped_type_bonus(&s.match_ups, &player.type_bonus_pct);
    out.extend(
        DamageType::ALL
            .iter()
            .zip(types)
            .map(|(ty, pct)| (StatKind::TypeBonus(*ty), pct)),
    );
    out.extend([
        (StatKind::RangePct, player.range_pct),
        (StatKind::PickupRadiusPct, player.pickup_radius_pct),
        (StatKind::SkillCooldownPct, player.skill_cooldown_pct),
        (StatKind::RuneHastePct, player.rune_haste_pct),
        (StatKind::EvadeCharges, u32::from(player.evade_charges)),
        (StatKind::KillHeal, player.kill_heal),
        (StatKind::BurstPct, player.burst_pct),
        (
            StatKind::Orbs,
            player.orbits.iter().map(|o| u32::from(o.orbs)).sum(),
        ),
        (StatKind::Rerolls, offers.rerolls_left),
        (StatKind::Banishes, offers.banishes_left),
        (
            StatKind::OfferCandidates,
            u32::from(offers.extra_candidates),
        ),
    ]);
    out
}

/// A rating for each candidate of `me`'s open star or boss offer, in the offer's order; empty
/// without one, or while an expertise or stat offer is open (those are not rated).
pub(crate) fn offer_ratings(
    world: &World,
    rewards: &Rewards,
    s: &State,
    me: PlayerIdx,
) -> Vec<OfferRating> {
    let career = s.players.career(me);
    let expertise_open = career.expertise.as_ref().is_some_and(|g| g.offer.is_some());
    let (Some(f), Some(offers)) = (s.stars.as_ref(), career.offers.as_ref()) else {
        return Vec::new();
    };
    let Some(open) = offers.offer.as_ref() else {
        return Vec::new();
    };
    if expertise_open || super::stat_offer_open(s, me) {
        return Vec::new();
    }
    let held = &s.players[me];
    let before = Rate::of(world, s, &career.progress, held);
    let before_sheet = sheet(s, &career.progress, held, career.regen, offers);
    open.candidates
        .iter()
        .map(|&i| {
            let mut progress = career.progress.clone();
            let mut player = held.clone();
            let mut regen = career.regen;
            let mut taken_by = offers.clone();
            let taken = stars::take(
                world,
                rewards,
                &mut SplitMix64::new(0),
                &mut progress,
                &mut player,
                &mut regen,
                f,
                &mut taken_by,
                i,
            );
            let reward = &rewards.rewards()[usize::from(i)];
            let sets = rewards.constellations().sets();
            // What the rules apply for a tier: its difference from the tier below.
            let tier_effects = taken.tiers.iter().flat_map(|&(set, tier)| {
                let tiers = &sets[set].tiers;
                let at = usize::from(tier) - 1;
                constellations::tier_delta(at.checked_sub(1).map(|lower| &tiers[lower]), &tiers[at])
            });
            let effects: Vec<EffectEntry> =
                reward.effects.iter().cloned().chain(tier_effects).collect();
            let mut categories: Vec<Category> = Vec::new();
            for e in &effects {
                let category = category_of(e.effect);
                if !categories.contains(&category) {
                    categories.push(category);
                }
            }
            let dps_bp = if effects.iter().any(|e| rated(e.effect)) {
                let after = Rate::of(world, s, &progress, &player);
                before
                    .as_ref()
                    .zip(after.as_ref())
                    .map(|(b, a)| b.gain_bp(a))
            } else {
                None
            };
            let after_sheet = sheet(s, &progress, &player, regen, &taken_by);
            let changes = before_sheet
                .iter()
                .zip(&after_sheet)
                .filter(|(b, a)| b.1 != a.1)
                .map(|(b, a)| StatChange {
                    stat: b.0,
                    before: b.1,
                    after: a.1,
                })
                .collect();
            OfferRating {
                categories,
                dps_bp,
                set_tier: taken.tiers.iter().map(|&(_, tier)| tier).max(),
                changes,
            }
        })
        .collect()
}

/// The category of one effect kind. One exhaustive `match`, no `_` arm: a new kind does not
/// compile until it has a category (ADR-CONT-13: derived, never stored in the content).
pub(crate) fn category_of(effect: RewardEffect) -> Category {
    match effect {
        RewardEffect::AttackPct
        | RewardEffect::AttackSpeedPct
        | RewardEffect::CritChancePct
        | RewardEffect::CritDamagePct
        | RewardEffect::TypeBonusPct
        | RewardEffect::ElementBonusPct
        | RewardEffect::AttackTypeBonusPct
        | RewardEffect::SkillCooldownPct
        | RewardEffect::StageSkill => Category::Attack,
        RewardEffect::EchoHitPct
        | RewardEffect::KillBurst
        | RewardEffect::BurstDepth
        | RewardEffect::Orbit => Category::OnHit,
        RewardEffect::MaxHp
        | RewardEffect::Regen
        | RewardEffect::EvadePct
        | RewardEffect::EvadeCharge
        | RewardEffect::HealPerKills => Category::Defence,
        RewardEffect::SpeedPct | RewardEffect::RangePct | RewardEffect::PickupRadiusPct => {
            Category::Mobility
        }
        RewardEffect::RuneHaste | RewardEffect::RandomRune => Category::Rune,
        RewardEffect::XpPct
        | RewardEffect::Rerolls
        | RewardEffect::Banishes
        | RewardEffect::OfferCandidates => Category::Growth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewards::Rewards;
    use Category::*;

    /// Fails to compile on a new `RewardEffect` until `ALL` lists it.
    fn listed(effect: RewardEffect) -> usize {
        match effect {
            RewardEffect::MaxHp => 0,
            RewardEffect::AttackPct => 1,
            RewardEffect::AttackSpeedPct => 2,
            RewardEffect::EvadePct => 3,
            RewardEffect::SpeedPct => 4,
            RewardEffect::XpPct => 5,
            RewardEffect::StageSkill => 6,
            RewardEffect::EvadeCharge => 7,
            RewardEffect::Orbit => 8,
            RewardEffect::RangePct => 9,
            RewardEffect::TypeBonusPct => 10,
            RewardEffect::AttackTypeBonusPct => 11,
            RewardEffect::ElementBonusPct => 12,
            RewardEffect::RuneHaste => 13,
            RewardEffect::RandomRune => 14,
            RewardEffect::Regen => 15,
            RewardEffect::PickupRadiusPct => 16,
            RewardEffect::SkillCooldownPct => 17,
            RewardEffect::HealPerKills => 18,
            RewardEffect::OfferCandidates => 19,
            RewardEffect::Rerolls => 20,
            RewardEffect::Banishes => 21,
            RewardEffect::CritChancePct => 22,
            RewardEffect::CritDamagePct => 23,
            RewardEffect::EchoHitPct => 24,
            RewardEffect::KillBurst => 25,
            RewardEffect::BurstDepth => 26,
        }
    }

    #[test]
    fn all_lists_every_kind_once_in_order() {
        for (i, effect) in RewardEffect::ALL.iter().enumerate() {
            assert_eq!(listed(*effect), i);
        }
    }

    #[test]
    fn every_kind_has_the_category_the_table_gives_it() {
        let table: [(Category, &[RewardEffect]); 6] = [
            (
                Attack,
                &[
                    RewardEffect::AttackPct,
                    RewardEffect::AttackSpeedPct,
                    RewardEffect::CritChancePct,
                    RewardEffect::CritDamagePct,
                    RewardEffect::TypeBonusPct,
                    RewardEffect::ElementBonusPct,
                    RewardEffect::AttackTypeBonusPct,
                    RewardEffect::SkillCooldownPct,
                    RewardEffect::StageSkill,
                ],
            ),
            (
                OnHit,
                &[
                    RewardEffect::EchoHitPct,
                    RewardEffect::KillBurst,
                    RewardEffect::BurstDepth,
                    RewardEffect::Orbit,
                ],
            ),
            (
                Defence,
                &[
                    RewardEffect::MaxHp,
                    RewardEffect::Regen,
                    RewardEffect::EvadePct,
                    RewardEffect::EvadeCharge,
                    RewardEffect::HealPerKills,
                ],
            ),
            (
                Mobility,
                &[
                    RewardEffect::SpeedPct,
                    RewardEffect::RangePct,
                    RewardEffect::PickupRadiusPct,
                ],
            ),
            (Rune, &[RewardEffect::RuneHaste, RewardEffect::RandomRune]),
            (
                Growth,
                &[
                    RewardEffect::XpPct,
                    RewardEffect::Rerolls,
                    RewardEffect::Banishes,
                    RewardEffect::OfferCandidates,
                ],
            ),
        ];
        let mut seen = 0;
        for (category, effects) in table {
            for effect in effects {
                assert_eq!(category_of(*effect), category, "{effect:?}");
                seen += 1;
            }
        }
        assert_eq!(seen, RewardEffect::ALL.len());
    }

    #[test]
    fn every_bundled_reward_has_a_category() {
        for reward in Rewards::bundled().rewards() {
            assert!(!reward.effects.is_empty(), "{}", reward.id);
            for entry in &reward.effects {
                let _ = category_of(entry.effect);
            }
        }
    }
}
