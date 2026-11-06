//! Rating the candidates of an open offer (ADR-REPL-5): the categories of a reward, the
//! expected gain in the main weapon's damage and the stats a pick moves. A read-only query on
//! the state as it is; nothing here is hashed, saved or recorded (ADR-SIM-3), and it is not part
//! of the snapshot (ADR-BRDG-4): call it once when an offer opens, not every frame.
//!
//! **The pick is run, not described.** Each candidate is taken with `stars::take` on a clone of
//! the player, so the set tier it completes is applied by the rules that apply it, and the stats
//! before and after are read from the two clones. A random rune is drawn from a generator seeded
//! 0 and dropped: it moves no stat shown and its draw leaks nothing (ADR-SIM-2).
//!
//! **A swap is run, not described.** `swap_ratings` rates one star against each slot it could go
//! in (ADR-SIM-7): the equip or replace is made by `slots::equip` / `slots::replace` on a clone of
//! the player, so the build, the grants, the set counts and the slot's level come from the live
//! rules and no rule or per-level number is copied here. The model is the offer model's below;
//! `dps_bp` is signed (a replace can lower the damage). Same read-only, not-in-the-snapshot
//! rule: call it when a window opens or an item arrives, not every frame; it clones the
//! player's equipment once per slot. It is empty for a run without slots, a rune, a star the
//! file lacks and a star no slot takes.
//!
//! **The damage model** (`Rate`) has no tunable number (ADR-CONT-3), so no content key. It
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
use crate::snapshot::{Category, OfferRating, StatChange, StatKind, SwapItem, SwapRating};

use super::equipment::{EquipSlot, Equipment, Held};
use super::state::{Career, Offers, PlayerState, Progress, State};
use super::world::World;
use super::{attacks, combat, levels, slots, stars};

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

    /// The change from `self` to `after`, in basis points, the ratio rounded half up: positive
    /// for a gain, negative for a loss.
    fn delta_bp(&self, after: &Rate) -> i32 {
        let num = after.damage * self.ticks * 10_000;
        let den = self.damage * after.ticks;
        let ratio = (num * 2 + den) / (2 * den);
        let delta = i128::try_from(ratio).unwrap_or(i128::MAX) - 10_000;
        i32::try_from(delta).unwrap_or(if delta < 0 { i32::MIN } else { i32::MAX })
    }

    /// The gain of `after` over `self`, in basis points, rounded half up; never below 0.
    fn gain_bp(&self, after: &Rate) -> u32 {
        u32::try_from(self.delta_bp(after).max(0)).unwrap_or(0)
    }
}

/// The stats of a build that a pick can move, in `StatKind` order. Without `offers` (a stage
/// without a stars section) the three stats that live in them are left out.
fn sheet(
    s: &State,
    progress: &Option<Progress>,
    player: &PlayerState,
    regen: u32,
    offers: Option<&Offers>,
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
    ]);
    if let Some(offers) = offers {
        out.extend([
            (StatKind::Rerolls, offers.rerolls_left),
            (StatKind::Banishes, offers.banishes_left),
            (
                StatKind::OfferCandidates,
                u32::from(offers.extra_candidates),
            ),
        ]);
    }
    out
}

/// The distinct categories of `effects`, in the order they first appear.
fn categories_of_effects(effects: &[EffectEntry]) -> Vec<Category> {
    let mut categories: Vec<Category> = Vec::new();
    for e in effects {
        let category = category_of(e.effect);
        if !categories.contains(&category) {
            categories.push(category);
        }
    }
    categories
}

/// The stats the two sheets (of one build, before and after) disagree on.
fn diff_sheets(before: &[(StatKind, u32)], after: &[(StatKind, u32)]) -> Vec<StatChange> {
    before
        .iter()
        .zip(after)
        .filter(|(b, a)| b.1 != a.1)
        .map(|(b, a)| StatChange {
            stat: b.0,
            before: b.1,
            after: a.1,
        })
        .collect()
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
    let before_sheet = sheet(s, &career.progress, held, career.regen, Some(offers));
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
            let categories = categories_of_effects(&effects);
            let dps_bp = if effects.iter().any(|e| rated(e.effect)) {
                let after = Rate::of(world, s, &progress, &player);
                before
                    .as_ref()
                    .zip(after.as_ref())
                    .map(|(b, a)| b.gain_bp(a))
            } else {
                None
            };
            let after_sheet = sheet(s, &progress, &player, regen, Some(&taken_by));
            let changes = diff_sheets(&before_sheet, &after_sheet);
            OfferRating {
                categories,
                dps_bp,
                set_tier: taken.tiers.iter().map(|&(_, tier)| tier).max(),
                changes,
            }
        })
        .collect()
}

/// The members of each set that are equipped, in the file's set order.
fn members_equipped(rewards: &Rewards, equipment: &Equipment) -> Vec<u8> {
    let places: Vec<u16> = slots::equipped_levels(equipment)
        .iter()
        .map(|s| s.0)
        .collect();
    rewards
        .constellations()
        .sets()
        .iter()
        .map(|set| {
            let n = set
                .members
                .iter()
                .filter(|m| {
                    rewards
                        .index_of(m)
                        .is_some_and(|p| places.iter().any(|&q| usize::from(q) == p))
                })
                .count();
            u8::try_from(n).unwrap_or(u8::MAX)
        })
        .collect()
}

/// What the rules apply for the tiers `from + 1 ..= to` of `set` (each is its difference from
/// the tier below).
fn tier_effects(rewards: &Rewards, set: usize, from: u8, to: u8) -> Vec<EffectEntry> {
    let tiers = &rewards.constellations().sets()[set].tiers;
    (from..to)
        .flat_map(|at| {
            let at = usize::from(at);
            constellations::tier_delta(at.checked_sub(1).map(|lower| &tiers[lower]), &tiers[at])
        })
        .collect()
}

/// The slot `index` addresses: the slots, then the Lumine slot.
fn slot_at(equipment: &Equipment, index: usize) -> Option<&EquipSlot> {
    equipment.slots.get(index).or_else(|| {
        equipment
            .lumine
            .as_ref()
            .filter(|_| index == equipment.slots.len())
    })
}

/// A rating for the star `item` against each slot it could go in (the slot rules accept it as an
/// equip into an empty slot or a replace of an occupied one), in slot order. Each swap is made by
/// the rules' own `slots::equip` / `slots::replace` on a clone of the player, so the build, the
/// grants, the set counts and the slot's level are the live rules' and nothing is copied here.
/// Empty without slots, for an item that is not a star the rules can equip, or when no slot
/// takes it. Read-only: the state is not changed, hashed or recorded.
pub(crate) fn swap_ratings(
    world: &World,
    rewards: &Rewards,
    s: &State,
    me: PlayerIdx,
    item: SwapItem,
) -> Vec<SwapRating> {
    let career = s.players.career(me);
    if career.equipment.slots.is_empty() {
        return Vec::new();
    }
    let owned;
    let (base, index): (&Career, u32) = match item {
        SwapItem::Inventory(i) => {
            let held = usize::try_from(i)
                .ok()
                .and_then(|i| career.equipment.inventory.get(i));
            if !matches!(held, Some(Held::Star(_))) {
                return Vec::new();
            }
            (career, i)
        }
        SwapItem::Star(id) => {
            let Some(place) = rewards.index_of(&id).and_then(|p| u16::try_from(p).ok()) else {
                return Vec::new();
            };
            let mut c = career.clone();
            c.equipment.inventory.push(Held::Star(place));
            let at = u32::try_from(c.equipment.inventory.len() - 1).unwrap_or(u32::MAX);
            owned = c;
            (&owned, at)
        }
    };
    let held = &s.players[me];
    let sets = rewards.constellations().sets();
    let before = Rate::of(world, s, &career.progress, held);
    let before_sheet = sheet(
        s,
        &career.progress,
        held,
        career.regen,
        career.offers.as_ref(),
    );
    let before_sets = members_equipped(rewards, &career.equipment);
    let slot_count = base.equipment.slots.len() + usize::from(base.equipment.lumine.is_some());
    let mut out = Vec::new();
    for at in 0..slot_count {
        let slot = u32::try_from(at).unwrap_or(u32::MAX);
        let Some(target) = slot_at(&base.equipment, at) else {
            continue;
        };
        let replaces = target.held.is_some();
        if slots::check(
            &world.equipment_rules,
            rewards,
            &base.equipment,
            index,
            slot,
            replaces,
        )
        .is_err()
        {
            continue;
        }
        let switch = replaces && slots::is_switch(rewards, &base.equipment, index, slot);
        let displaced = target.held;
        let incoming = match base.equipment.inventory.get(index as usize) {
            Some(Held::Star(p)) => *p,
            _ => continue,
        };
        let mut c = base.clone();
        let mut player = held.clone();
        let made = if replaces {
            slots::replace(world, rewards, &mut player, &mut c, index, slot)
        } else {
            slots::equip(world, rewards, &mut player, &mut c, index, slot)
        };
        if made.is_err() {
            continue;
        }
        let after_sets = members_equipped(rewards, &c.equipment);
        let (mut set_gained, mut set_lost) = (None, None);
        let mut tier_effects_moved: Vec<EffectEntry> = Vec::new();
        let mut tier_effects_gained: Vec<EffectEntry> = Vec::new();
        for (set, def) in sets.iter().enumerate() {
            let was = def.tier_reached(before_sets[set]);
            let now = def.tier_reached(after_sets[set]);
            if now > was {
                set_gained = set_gained.max(Some(now));
                let gained = tier_effects(rewards, set, was, now);
                tier_effects_moved.extend(gained.iter().cloned());
                tier_effects_gained.extend(gained);
            } else if now < was {
                set_lost = set_lost.max(Some(was));
                tier_effects_moved.extend(tier_effects(rewards, set, now, was));
            }
        }
        let star_effects = |place: u16| -> &[EffectEntry] {
            rewards
                .rewards()
                .get(usize::from(place))
                .map_or(&[], |r| r.effects.as_slice())
        };
        let displaced_place = match displaced {
            Some(Held::Star(p)) => Some(p),
            _ => None,
        };
        let touches = star_effects(incoming)
            .iter()
            .chain(displaced_place.map_or(&[][..], star_effects))
            .chain(&tier_effects_moved)
            .any(|e| rated(e.effect));
        let dps_bp = if touches {
            let after = Rate::of(world, s, &c.progress, &player);
            before
                .as_ref()
                .zip(after.as_ref())
                .map(|(b, a)| b.delta_bp(a))
        } else {
            None
        };
        let effects: Vec<EffectEntry> = star_effects(incoming)
            .iter()
            .cloned()
            .chain(tier_effects_gained)
            .collect();
        let after_sheet = sheet(s, &c.progress, &player, c.regen, c.offers.as_ref());
        out.push(SwapRating {
            slot,
            replaces,
            switch,
            level_before: slot_at(&career.equipment, at).map_or(0, |t| t.level),
            level_after: slot_at(&c.equipment, at).map_or(0, |t| t.level),
            categories: categories_of_effects(&effects),
            dps_bp,
            set_gained,
            set_lost,
            changes: diff_sheets(&before_sheet, &after_sheet),
        });
    }
    out
}

/// The category of one effect kind. One exhaustive `match`, no `_` arm: a new kind does not
/// compile until it has a category (ADR-CONT-3: derived, never stored in the content).
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

    // --- swap_ratings: the star rated against each slot it could go in -------------------------

    mod swaps {
        use super::super::*;
        use crate::constellations::Constellations;
        use crate::rewards::Rarity;
        use crate::run_content::RunContent;
        use crate::sim::equipment::Held;
        use crate::sim::{RunDifficulty, RunPool, initial_state, slots};
        use crate::snapshot::Category::{Attack, Defence};
        use crate::snapshot::{SwapItem, SwapRating};
        use crate::version::Versions;
        use serde_json::{Value, json};

        const ME: PlayerIdx = PlayerIdx::SOLO;

        struct Fixture {
            world: World,
            rewards: Rewards,
            s: State,
        }

        fn effect(kind: &str, amount: u32) -> Value {
            json!({ "effect": kind, "amount": amount })
        }

        fn star(id: &str, effects: Vec<Value>) -> Value {
            json!({ "id": id, "rarity": "common", "limit": 3, "effects": effects })
        }

        fn named(id: &str, effects: Vec<Value>) -> Value {
            json!({ "id": id, "rarity": "epic", "limit": 1, "named": true, "star": id,
                    "effects": effects })
        }

        /// The stars of the worked numbers.
        fn list() -> Vec<Value> {
            vec![
                star("atk20", vec![effect("attack_pct", 20)]),
                star("atk30", vec![effect("attack_pct", 30)]),
                star("atk5", vec![effect("attack_pct", 5)]),
                star("crit10", vec![effect("crit_chance_pct", 10)]),
                named("crit5", vec![effect("crit_chance_pct", 5)]),
                named("speed", vec![effect("speed_pct", 10)]),
                star("hp100", vec![effect("max_hp", 100)]),
                star("hp250", vec![effect("max_hp", 250)]),
                json!({ "id": "random_rune", "rarity": "common", "effects": [{ "effect": "random_rune" }] }),
            ]
        }

        fn duo() -> Value {
            json!({ "constellations": [{
                "id": "duo", "name": "The duo", "members": ["speed", "crit5"],
                "tiers": [{ "threshold": 2, "effects": [effect("attack_pct", 7)] }]
            }] })
        }

        fn rarities() -> Vec<Value> {
            [
                "common",
                "uncommon",
                "rare",
                "epic",
                "legendary",
                "supernova",
            ]
            .iter()
            .enumerate()
            .map(|(i, r)| json!({ "rarity": r, "lumine_cost": i + 1 }))
            .collect()
        }

        /// One active phase of 10 ticks reaching the enemies.
        fn swing() -> Value {
            json!([{ "kind": "active", "ticks": 10,
                     "hit_box": { "type": "circle", "offset": { "forward": 1_024 },
                                  "radius": 16_384 } }])
        }

        fn weapon(phases: Value) -> Value {
            json!({ "inline": {
                "combo": { "main": { "attack": { "inline": phases } } },
                "window_ticks": 30,
                "assist_range": 0
            } })
        }

        fn fixture_with(phases: Value, sets: Option<Value>, lumine: bool) -> Fixture {
            let mut file = json!({ "rarities": rarities(), "rewards": list() });
            crate::sim::starlight::with_equipment_rules(&mut file);
            let mut rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
                .expect("the test rewards validate");
            if let Some(v) = sets {
                let c = Constellations::parse_with(
                    v.to_string().as_bytes(),
                    Versions::CURRENT,
                    &rewards,
                )
                .expect("the test sets validate");
                rewards = rewards.with_constellations(c);
            }
            let mut stage = RunContent::bundled().trial().clone();
            stage.player.atk = 1000;
            stage.player.weapon = serde_json::from_value(weapon(phases)).expect("the weapon");
            let mut world = World::for_test(stage);
            world.equipment_rules = slots::bundled_rules().clone();
            let pool = RunPool {
                rewards: (0..u16::try_from(rewards.rewards().len()).unwrap()).collect(),
                extra_rerolls: 0,
                difficulty: RunDifficulty {
                    hp_pct: 100,
                    damage_pct: 100,
                    rarity: (Rarity::Common, Rarity::Supernova),
                },
            };
            let mut s = initial_state(
                &world,
                RunContent::bundled(),
                pool,
                1,
                crate::kinds::KindTable::of_stage(&world).unwrap(),
            );
            let eq = &mut s.players.career_mut(ME).equipment;
            eq.slots = slots::fresh_slots(slots::bundled_rules());
            if lumine {
                eq.lumine = Some(crate::sim::equipment::EquipSlot {
                    kind: crate::sim::equipment::SlotKind::Lumine,
                    held: None,
                    level: 0,
                    expertise: 0,
                });
            }
            Fixture { world, rewards, s }
        }

        fn fixture() -> Fixture {
            fixture_with(swing(), None, false)
        }

        impl Fixture {
            fn place(&self, id: &str) -> u16 {
                u16::try_from(self.rewards.index_of(&id.into()).expect(id)).unwrap()
            }

            /// Puts the star `id` into slot `at` at `level`, writing the change.
            fn equip(&mut self, at: usize, id: &str, level: u8) {
                let place = self.place(id);
                let (player, career) = self.s.players.both_mut(ME);
                let before = slots::equipped_levels(&career.equipment);
                career.equipment.slots[at].held = Some(Held::Star(place));
                career.equipment.slots[at].level = level;
                slots::refresh(
                    &self.world,
                    &self.rewards,
                    player,
                    career,
                    &before,
                    slots::Arrival::Swap,
                );
            }

            /// Puts the star `id` at the end of the inventory; its index.
            fn hold(&mut self, id: &str) -> u32 {
                let place = self.place(id);
                let inv = &mut self.s.players.career_mut(ME).equipment.inventory;
                inv.push(Held::Star(place));
                u32::try_from(inv.len() - 1).unwrap()
            }

            fn rate(&self, item: SwapItem) -> Vec<SwapRating> {
                swap_ratings(&self.world, &self.rewards, &self.s, ME, item)
            }

            fn rate_item(&self, i: u32) -> Vec<SwapRating> {
                self.rate(SwapItem::Inventory(i))
            }
        }

        fn slots_of(r: &[SwapRating]) -> Vec<u32> {
            r.iter().map(|x| x.slot).collect()
        }

        fn at(r: &[SwapRating], slot: u32) -> &SwapRating {
            r.iter().find(|x| x.slot == slot).expect("slot is rated")
        }

        fn change(stat: StatKind, before: u32, after: u32) -> StatChange {
            StatChange {
                stat,
                before,
                after,
            }
        }

        /// T1: an equip into an empty slot, against the Attack slots only.
        #[test]
        fn an_equip_into_an_empty_slot() {
            let mut f = fixture();
            let i = f.hold("atk20");
            let r = f.rate_item(i);
            assert_eq!(slots_of(&r), [0, 1, 2, 3, 4]);
            assert_eq!(
                *at(&r, 0),
                SwapRating {
                    slot: 0,
                    replaces: false,
                    switch: false,
                    level_before: 0,
                    level_after: 1,
                    categories: vec![Attack],
                    dps_bp: Some(2000),
                    set_gained: None,
                    set_lost: None,
                    changes: vec![change(StatKind::AttackPct, 0, 20)],
                }
            );
        }

        /// T2: a replace can lower the damage; the other slots are equips against the build
        /// that still holds the star.
        #[test]
        fn a_replace_is_signed() {
            let mut f = fixture();
            f.equip(0, "atk20", 1);
            let i = f.hold("crit10");
            let r = f.rate_item(i);
            let first = at(&r, 0);
            assert!(first.replaces && !first.switch);
            assert_eq!(first.dps_bp, Some(-833));
            assert_eq!(
                first.changes,
                [
                    change(StatKind::AttackPct, 20, 0),
                    change(StatKind::CritChancePct, 0, 10)
                ]
            );
            for slot in 1..=4 {
                let x = at(&r, slot);
                assert!(!x.replaces);
                assert_eq!(x.dps_bp, Some(1000));
            }
        }

        /// T3: a same-option switch keeps the whole level and is the only slot left.
        #[test]
        fn a_switch_keeps_the_level() {
            let mut f = fixture();
            f.equip(0, "atk20", 4);
            let i = f.hold("atk30");
            let r = f.rate_item(i);
            assert_eq!(slots_of(&r), [0]);
            let x = &r[0];
            assert!(x.replaces && x.switch);
            assert_eq!((x.level_before, x.level_after), (4, 4));
            // Lv4 scales both stars by `level_scaled`: 21 -> 32, so 132 / 121.
            assert_eq!(x.dps_bp, Some(909));

            // At Lv1 the amounts are the base: 120 -> 130.
            let mut f = fixture();
            f.equip(0, "atk20", 1);
            let i = f.hold("atk30");
            assert_eq!(f.rate_item(i)[0].dps_bp, Some(833));
        }

        /// T15: a replace keeps half the level (at least 1), a switch the whole level, and the
        /// rating reads what `slots::replace` leaves.
        #[test]
        fn replace_halves_and_switch_keeps_the_level() {
            for (level, halved) in [(10, 5), (9, 4), (3, 1), (2, 1), (1, 1)] {
                let mut f = fixture();
                f.equip(0, "atk20", level);
                let other = f.hold("crit10");
                let same = f.hold("atk30");
                let r = f.rate_item(other);
                let x = at(&r, 0);
                assert_eq!(
                    (x.level_before, x.level_after),
                    (level, halved),
                    "Lv{level}"
                );
                assert!(!x.switch);
                let r = f.rate_item(same);
                let x = at(&r, 0);
                assert_eq!((x.level_before, x.level_after), (level, level), "Lv{level}");
                assert!(x.switch);
                // The rule itself, on a second clone.
                let (mut player, mut career) =
                    (f.s.players[ME].clone(), f.s.players.career(ME).clone());
                slots::replace(&f.world, &f.rewards, &mut player, &mut career, other, 0).unwrap();
                assert_eq!(career.equipment.slots[0].level, halved);
            }
        }

        /// T16: the same swap at two levels differs by what `level_scaled` gives.
        #[test]
        fn the_per_level_change_comes_from_the_rules() {
            let bp = |level: u8| {
                let (before, after) = (
                    100 + slots::level_scaled(slots::bundled_rules(), 20, level),
                    100 + slots::level_scaled(slots::bundled_rules(), 30, level),
                );
                i32::try_from(
                    (10_000 * u64::from(after) * 2 + u64::from(before)) / (2 * u64::from(before)),
                )
                .unwrap()
                    - 10_000
            };
            let rated = |level: u8| {
                let mut f = fixture();
                f.equip(0, "atk20", level);
                let i = f.hold("atk30");
                f.rate_item(i)[0].dps_bp
            };
            assert_eq!(rated(3), Some(bp(3)));
            assert_eq!(rated(10), Some(bp(10)));
            // Worked by hand: 131/121 and 138/125.
            assert_eq!((rated(3), rated(10)), (Some(826), Some(1040)));
        }

        /// T17: at a level, the stat changes are the diff of really making the action.
        #[test]
        fn the_changes_equal_the_stat_diff_of_the_action() {
            let mut f = fixture();
            f.equip(0, "atk20", 7);
            let i = f.hold("crit10");
            let r = f.rate_item(i);
            let career = f.s.players.career(ME);
            let held = &f.s.players[ME];
            let before = sheet(
                &f.s,
                &career.progress,
                held,
                career.regen,
                career.offers.as_ref(),
            );
            let (mut player, mut c) = (held.clone(), career.clone());
            slots::replace(&f.world, &f.rewards, &mut player, &mut c, i, 0).unwrap();
            let after = sheet(&f.s, &c.progress, &player, c.regen, c.offers.as_ref());
            assert!(!at(&r, 0).changes.is_empty());
            assert_eq!(at(&r, 0).changes, diff_sheets(&before, &after));
            assert_eq!(at(&r, 0).level_after, 3);
        }

        /// T4: a slot the check refuses is absent, and so is every item that cannot be rated.
        #[test]
        fn refusals_are_absent() {
            let mut f = fixture();
            let i = f.hold("hp100");
            assert_eq!(slots_of(&f.rate_item(i)), [5, 6]);
            let mut f = fixture_with(swing(), None, true);
            let i = f.hold("hp100");
            assert_eq!(slots_of(&f.rate_item(i)), [5, 6, 12]);

            let mut f = fixture();
            let rr = f.hold("random_rune");
            assert!(f.rate_item(rr).is_empty());
            let eq = &mut f.s.players.career_mut(ME).equipment;
            eq.inventory.push(Held::Rune(crate::rune::Rune::Thunder));
            let rune = u32::try_from(eq.inventory.len() - 1).unwrap();
            assert!(f.rate_item(rune).is_empty());
            assert!(f.rate_item(99).is_empty());
            assert!(f.rate(SwapItem::Star("nope".into())).is_empty());
        }

        /// T5: a star not held rates like the same star held, on a clone.
        #[test]
        fn a_star_not_held_rates_as_held() {
            let held = {
                let mut f = fixture();
                let i = f.hold("atk20");
                f.rate_item(i)
            };
            let f = fixture();
            let r = f.rate(SwapItem::Star("atk20".into()));
            assert_eq!(r, held);
            assert!(f.s.players.career(ME).equipment.inventory.is_empty());
        }

        /// T6: a star with no modelled effect has no damage rating.
        #[test]
        fn a_star_that_is_not_damage() {
            let mut f = fixture();
            let h = f.s.players[ME].max_hp;
            let i = f.hold("hp100");
            let r = f.rate_item(i);
            let x = at(&r, 5);
            assert_eq!(x.dps_bp, None);
            assert_eq!(x.categories, [Defence]);
            assert_eq!(x.changes, [change(StatKind::MaxHp, h, h + 100)]);
        }

        /// T7: a displaced star that is not damage either.
        #[test]
        fn a_displaced_star_that_is_not_damage() {
            let mut f = fixture();
            f.equip(5, "hp100", 1);
            let h = f.s.players[ME].max_hp;
            let i = f.hold("hp250");
            let r = f.rate_item(i);
            let x = at(&r, 5);
            assert!(x.switch);
            assert_eq!(x.dps_bp, None);
            assert_eq!(x.changes, [change(StatKind::MaxHp, h, h + 150)]);
        }

        /// T8: a set tier gained or lost is run by the rules and rated.
        #[test]
        fn sets_gained_and_lost() {
            let mut f = fixture_with(swing(), Some(duo()), false);
            f.equip(7, "speed", 1);
            let i = f.hold("crit5");
            let r = f.rate_item(i);
            let x = at(&r, 0);
            assert_eq!(x.dps_bp, Some(1235));
            assert_eq!((x.set_gained, x.set_lost), (Some(1), None));
            assert!(x.changes.contains(&change(StatKind::AttackPct, 0, 7)));

            let mut f = fixture_with(swing(), Some(duo()), false);
            f.equip(7, "speed", 1);
            f.equip(0, "crit5", 1);
            let i = f.hold("atk5");
            let x = at(&f.rate_item(i), 0).clone();
            assert!(x.replaces && !x.switch);
            assert_eq!(x.dps_bp, Some(-654));
            assert_eq!((x.set_gained, x.set_lost), (None, Some(1)));
        }

        /// T9: a weapon with no active phase has no damage to rate.
        #[test]
        fn an_empty_weapon_rates_no_damage() {
            let phases = json!([{ "kind": "wind_up", "ticks": 10,
                "hit_box": { "type": "circle", "offset": { "forward": 1_024 },
                             "radius": 16_384 } }]);
            let mut f = fixture_with(phases, None, false);
            let i = f.hold("atk20");
            let r = f.rate_item(i);
            assert_eq!(r.len(), 5);
            assert!(r.iter().all(|x| x.dps_bp.is_none()));
            assert_eq!(at(&r, 0).changes, [change(StatKind::AttackPct, 0, 20)]);
        }

        /// T10: the state is not touched, and two calls agree.
        #[test]
        fn the_query_reads_and_changes_nothing() {
            let mut f = fixture_with(swing(), Some(duo()), false);
            f.equip(7, "speed", 1);
            let i = f.hold("crit5");
            f.hold("atk20");
            let hash = crate::sim::state_hash(&f.s);
            let (career, player) = (f.s.players.career(ME).clone(), f.s.players[ME].clone());
            let first = f.rate_item(i);
            assert_eq!(f.rate_item(i), first);
            assert_eq!(crate::sim::state_hash(&f.s), hash);
            let after = f.s.players.career(ME);
            assert_eq!(after.equipment, career.equipment);
            assert_eq!(after.progress, career.progress);
            assert_eq!(after.offers, career.offers);
            assert_eq!(after.expertise, career.expertise);
            assert_eq!(f.s.players[ME], player);
        }

        /// The signed core: the ratio rounds half up and a loss is negative.
        #[test]
        fn delta_bp_is_signed() {
            let rate = |damage| Rate { damage, ticks: 1 };
            assert_eq!(rate(100).delta_bp(&rate(120)), 2000);
            assert_eq!(rate(120).delta_bp(&rate(110)), -833);
            assert_eq!(rate(100).gain_bp(&rate(90)), 0);
        }
    }
}
