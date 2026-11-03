//! Rating the candidates of an open offer (ADR-REPL-16): the categories of a reward, the
//! expected gain in the main weapon's damage and the stats a pick moves.

use crate::rewards::RewardEffect;
use crate::snapshot::Category;

/// The category of one effect kind. One exhaustive `match`, no `_` arm: a new kind does not
/// compile until it has a category (ADR-CONT-13: derived, never stored in the content).
#[allow(dead_code)]
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
