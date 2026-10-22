//! Falling stars and their offers (ADR-079 §2, §3, ADR-068), in a stage with `stars`.
//!
//! In the collisions, after the defeated enemies' experience, each defeated enemy in entity-ID
//! order may drop a star where it fell. With the map full of stars it drops none, and the pity
//! does not count it. Otherwise it drops one for sure once `pity` kills in a row have dropped
//! none, and on a draw of the drop chance before that. A dropped star's rarity is drawn from the
//! weights of the drop table's tier for the player's level, after this tick's experience. An
//! elite (ADR-068) drops one for sure, without a draw and leaving the pity as it was, from the
//! next tier up, or from the last tier. Then the main character takes every star inside its
//! pickup circle, in the order they dropped.
//!
//! A boss kill makes the boss offer due, ahead of the stars taken that tick, at the highest
//! rarity of the drop table's tier for the player's level after this tick's experience. It
//! opens even though the kill clears the stage, and the stars' offers due then open after it.
//!
//! At the end of a tick the run survives or clears, the first offer due opens, after the boss offer and
//! the expertise levels' (`expertise`). It draws `offer_size` candidates (`OFFER_SIZE`, one more for each
//! offer-candidates reward taken, at most 4) from the run's pool at the star's rarity, then from the rarities below while too few are left. It leaves out banished rewards, rewards at their limit and rewards it already
//! holds, and the random rune once the Main weapon holds every rune (ADR-099), whose rune is
//! drawn only when it is taken. An offer with no candidate does not open, and the next one due is tried. While an
//! offer is open the world stops. A choice (`choose`) takes a candidate, rerolls one, skips (a
//! star's offer only), or banishes one; when it closes the offer, the next one due opens at
//! once (`super::choose`). Every draw is from the run's generator, in this order.

use crate::attack::DamageType;
use crate::engine::{
    Due, HeldRune, OfferState, PlayerState, Progress, StarField, StarState, State,
};
use crate::event::DomainEvent;
use crate::id::{ContentId, EntityId};
use crate::rewards::{
    EffectEntry, MAX_BURST_DEPTH, MAX_BURST_PCT, MAX_BURST_RADIUS_PX, MAX_CRIT_CHANCE_TOTAL_PCT,
    MAX_CRIT_DAMAGE_TOTAL_PCT, MAX_ECHO_HIT_PCT, MAX_OFFER_CANDIDATES, Rarity, RewardEffect,
    Rewards,
};
use crate::rng::SplitMix64;
use crate::rune::Rune;
use crate::snapshot::{CandidateView, OfferKind, OfferOption, OfferView};
use crate::stage::{RarityWeights, Stage, Stars, Tier};
use crate::validate::limits::MAX_STARS_ON_MAP;

use super::{levels, orbit, overlaps, player, runes, skill, widened};

/// The candidates an offer holds (a tuning value, ADR-079 §3).
pub(crate) const OFFER_SIZE: usize = 3;
/// The candidates a star's offer holds: the base, and one more for each offer-candidates reward
/// taken, at most `MAX_OFFER_CANDIDATES`.
fn offer_size(f: &StarField) -> usize {
    (OFFER_SIZE + usize::from(f.extra_candidates)).min(MAX_OFFER_CANDIDATES)
}

/// The drop chance is out of this.
const PER_MILLE: u32 = 1000;

/// Stars for the enemies at 0 hp, before they are removed.
pub(super) fn drop_from_defeated(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.stars else {
        return;
    };
    let Some(tier) = tier_of(cfg, s) else {
        return;
    };
    let cap = usize::from(cfg.cap.min(MAX_STARS_ON_MAP));
    // An elite's star draws from the next tier up, or from the last.
    let up = cfg
        .tiers
        .iter()
        .position(|t| std::ptr::eq(t, tier))
        .and_then(|i| cfg.tiers.get(i + 1))
        .unwrap_or(tier);
    let fallen: Vec<_> = s
        .enemies
        .iter()
        .filter(|e| e.hp == 0 && !e.burst)
        .map(|e| (e.at, e.elite.is_some()))
        .collect();
    let State {
        rng,
        stars,
        next_id,
        ..
    } = s;
    let Some(f) = stars.as_mut() else {
        return;
    };
    for (at, elite) in fallen {
        if f.on_map.len() >= cap {
            continue;
        }
        // An elite drops its star for sure, and leaves the pity as it was.
        if !elite {
            let drops = f.misses >= u32::from(cfg.pity)
                || rng.below(PER_MILLE) < u32::from(cfg.drop_per_mille);
            if !drops {
                f.misses = f.misses.saturating_add(1);
                continue;
            }
            f.misses = 0;
        }
        let rarity = draw_rarity(rng, &if elite { up } else { tier }.weights, f.rarity);
        let id = EntityId(*next_id);
        *next_id += 1;
        f.on_map.push(StarState { id, at, rarity });
        events.push(DomainEvent::StarDropped { star: id, rarity });
    }
}

/// The drop table's tier for the player's level.
fn tier_of<'a>(cfg: &'a Stars, s: &State) -> Option<&'a Tier> {
    let level = s.progress.as_ref().map_or(1, |p| p.level);
    cfg.tiers
        .iter()
        .rev()
        .find(|t| t.from_level <= level)
        .or(cfg.tiers.first())
}

/// The boss died this tick: its offer is due first, at the highest rarity of the player's tier.
pub(super) fn boss_killed(stage: &Stage, s: &mut State) {
    let Some(rarity) = stage
        .stars
        .as_ref()
        .and_then(|cfg| tier_of(cfg, s))
        .and_then(|t| t.weights.highest())
    else {
        return;
    };
    if let Some(f) = s.stars.as_mut() {
        // Inside the difficulty's range (ADR-110).
        let rarity = rarity.clamp(f.rarity.0, f.rarity.1);
        let kind = OfferKind::Boss;
        f.due.insert(0, Due { kind, rarity });
    }
}

/// A rarity drawn by `weights`, leaving out those outside `range`, the difficulty's (ADR-110):
/// the lowest of the range when it leaves no weight. The generator is drawn once either way.
fn draw_rarity(rng: &mut SplitMix64, weights: &RarityWeights, range: (Rarity, Rarity)) -> Rarity {
    let weight = |rarity: Rarity| match (range.0..=range.1).contains(&rarity) {
        true => u32::from(weights.get(rarity)),
        false => 0,
    };
    let total: u32 = Rarity::ALL.into_iter().map(weight).sum();
    let mut left = rng.below(total.max(1));
    for rarity in Rarity::ALL {
        let w = weight(rarity);
        if left < w {
            return rarity;
        }
        left -= w;
    }
    range.0
}

/// The main character takes the stars inside its pickup circle; their offers are due.
pub(super) fn pick_up(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.stars else {
        return;
    };
    if s.player.hp == 0 {
        return;
    }
    let at = s.player.at;
    let pickup_pct = s.player.pickup_radius_pct;
    let Some(f) = s.stars.as_mut() else {
        return;
    };
    let stars = std::mem::take(&mut f.on_map);
    for star in stars {
        if overlaps(at, widened(cfg.pickup_radius.0, pickup_pct), star.at, 0) {
            f.due.push(Due {
                kind: OfferKind::Star,
                rarity: star.rarity,
            });
            events.push(DomainEvent::StarTaken {
                star: star.id,
                rarity: star.rarity,
            });
        } else {
            f.on_map.push(star);
        }
    }
}

/// Opens the first offer due that has a candidate, unless an offer is open; with `boss_only`,
/// only while the first due is the boss offer.
pub(super) fn open_due(
    rewards: &Rewards,
    s: &mut State,
    events: &mut Vec<DomainEvent>,
    boss_only: bool,
) {
    if super::offer_open(s) {
        return;
    }
    let State {
        rng, stars, player, ..
    } = s;
    let Some(f) = stars.as_mut() else {
        return;
    };
    let held_runes = &player.runes;
    while f.offer.is_none()
        && f.due
            .first()
            .is_some_and(|d| !boss_only || d.kind == OfferKind::Boss)
    {
        let Due { kind, rarity } = f.due.remove(0);
        let size = offer_size(f);
        let mut candidates = Vec::with_capacity(size);
        while candidates.len() < size {
            match draw(rewards, f, held_runes, rng, rarity, &candidates) {
                Some(i) => candidates.push(i),
                None => break,
            }
        }
        if candidates.is_empty() {
            continue;
        }
        events.push(DomainEvent::OfferOpened {
            kind,
            rarity,
            candidates: candidates.iter().map(|i| id(rewards, *i)).collect(),
        });
        f.offer = Some(OfferState {
            kind,
            rarity,
            candidates,
        });
    }
}

/// Whether the reward at `i` may be offered: not banished, not in `held`, and not at its limit,
/// or for a random rune, while the Main weapon lacks a rune of `runes`.
fn eligible(
    rewards: &Rewards,
    f: &StarField,
    runes: &[HeldRune],
    i: u16,
    rarity: Rarity,
    held: &[u16],
) -> bool {
    let at = usize::from(i);
    rewards.rewards().get(at).is_some_and(|r| {
        let room = match (r.is_random_rune(), r.limit) {
            (true, _) => !runes::all_held(runes, f.rune_pool),
            (false, Some(limit)) => f.taken[at] < limit,
            (false, None) => false,
        };
        r.rarity == rarity && !f.banished[at] && room && !held.contains(&i)
    })
}

/// The rarest rarity from `top` down that has a reward to offer besides `held`.
fn first_rarity(
    rewards: &Rewards,
    f: &StarField,
    runes: &[HeldRune],
    top: Rarity,
    held: &[u16],
) -> Option<Rarity> {
    Rarity::ALL
        .iter()
        .rev()
        .copied()
        .filter(|r| *r <= top && *r >= f.rarity.0)
        .find(|r| {
            f.pool
                .iter()
                .any(|i| eligible(rewards, f, runes, *i, *r, held))
        })
}

/// Draws one reward from the pool at `top`, or the rarest rarity below it with one left,
/// leaving out `held`.
fn draw(
    rewards: &Rewards,
    f: &StarField,
    runes: &[HeldRune],
    rng: &mut SplitMix64,
    top: Rarity,
    held: &[u16],
) -> Option<u16> {
    let rarity = first_rarity(rewards, f, runes, top, held)?;
    let list: Vec<u16> = f
        .pool
        .iter()
        .copied()
        .filter(|i| eligible(rewards, f, runes, *i, rarity, held))
        .collect();
    // One draw over the weights, divided by their greatest common divisor: equal weights are
    // one each, so the draw is `below(len)` as it was before rewards had weights.
    let weight = |i: u16| u32::from(rewards.rewards()[usize::from(i)].weight).max(1);
    let step = list.iter().fold(0, |g, i| gcd(g, weight(*i)));
    let total: u32 = list.iter().map(|i| weight(*i) / step).sum();
    let mut at = rng.below(total);
    for i in &list {
        let w = weight(*i) / step;
        if at < w {
            return Some(*i);
        }
        at -= w;
    }
    None
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn id(rewards: &Rewards, i: u16) -> ContentId {
    rewards.rewards()[usize::from(i)].id.clone()
}

/// The options of the open offer, in index order; empty without one.
pub(crate) fn options(rewards: &Rewards, s: &State) -> Vec<OfferOption> {
    let Some((f, o)) = s.stars.as_ref().and_then(|f| Some((f, f.offer.as_ref()?))) else {
        return Vec::new();
    };
    let n = u8::try_from(o.candidates.len()).unwrap_or(u8::MAX);
    let mut out: Vec<OfferOption> = (0..n)
        .map(|candidate| OfferOption::Take { candidate })
        .collect();
    let runes = &s.player.runes;
    if f.rerolls_left > 0 && first_rarity(rewards, f, runes, o.rarity, &o.candidates).is_some() {
        out.extend((0..n).map(|candidate| OfferOption::Reroll { candidate }));
    }
    if o.kind == OfferKind::Star {
        out.push(OfferOption::Skip);
    }
    if f.banishes_left > 0 {
        out.extend((0..n).map(|candidate| OfferOption::Banish { candidate }));
    }
    out
}

/// Takes the reward at `i` of the file: counts it toward its stack limit and does what it does.
/// The rune a random rune drew, if it drew one. A pick from an open offer and a star the run
/// starts with (ADR-107 point 3) go through here alike.
#[allow(clippy::too_many_arguments)]
pub(super) fn take(
    stage: &Stage,
    rewards: &Rewards,
    rng: &mut SplitMix64,
    progress: &mut Option<Progress>,
    player: &mut PlayerState,
    regen: &mut u32,
    f: &mut StarField,
    i: u16,
) -> Option<Rune> {
    let at = usize::from(i);
    f.taken[at] = f.taken[at].saturating_add(1);
    let reward = &rewards.rewards()[at];
    apply_effects(
        stage,
        rewards,
        rng,
        progress,
        player,
        regen,
        f,
        i,
        &reward.effects,
    )
}

/// Does what `effects` do, once: the effects of the reward at `i` of the file (an orbit keys on
/// that place), or of a constellation's tier.
#[allow(clippy::too_many_arguments)]
fn apply_effects(
    stage: &Stage,
    rewards: &Rewards,
    rng: &mut SplitMix64,
    progress: &mut Option<Progress>,
    player: &mut PlayerState,
    regen: &mut u32,
    f: &mut StarField,
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
            RewardEffect::Regen => *regen = regen.saturating_add(amount),
            // The perks' counters (`loadout::perk`), added to for the rest of the run.
            RewardEffect::OfferCandidates => {
                let room = u8::try_from(MAX_OFFER_CANDIDATES - OFFER_SIZE).unwrap_or(u8::MAX);
                let more = u8::try_from(amount).unwrap_or(u8::MAX);
                f.extra_candidates = f.extra_candidates.saturating_add(more).min(room);
            }
            RewardEffect::Rerolls => f.rerolls_left = f.rerolls_left.saturating_add(amount),
            RewardEffect::Banishes => f.banishes_left = f.banishes_left.saturating_add(amount),
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

/// Carries out `option`, one of `options`, on the open offer.
pub(super) fn choose(
    stage: &Stage,
    rewards: &Rewards,
    s: &mut State,
    option: OfferOption,
    events: &mut Vec<DomainEvent>,
) {
    let State {
        rng,
        stars,
        progress,
        player,
        regen,
        ..
    } = s;
    let Some(f) = stars.as_mut() else {
        return;
    };
    let Some(mut o) = f.offer.take() else {
        return;
    };
    let open = match option {
        OfferOption::Take { candidate } => {
            let Some(&i) = o.candidates.get(usize::from(candidate)) else {
                return;
            };
            let rune = take(stage, rewards, rng, progress, player, regen, f, i);
            let at = usize::from(i);
            events.push(DomainEvent::RewardTaken {
                reward: rewards.rewards()[at].id.clone(),
                taken: f.taken[at],
                rune,
            });
            false
        }
        OfferOption::Reroll { candidate } => {
            let slot = usize::from(candidate);
            if let Some(new) = draw(rewards, f, &player.runes, rng, o.rarity, &o.candidates) {
                f.rerolls_left = f.rerolls_left.saturating_sub(1);
                o.candidates[slot] = new;
                events.push(DomainEvent::CandidateReplaced {
                    slot: candidate,
                    reward: id(rewards, new),
                });
            }
            true
        }
        OfferOption::Skip => {
            events.push(DomainEvent::OfferSkipped);
            false
        }
        OfferOption::Banish { candidate } => {
            let slot = usize::from(candidate);
            let Some(&i) = o.candidates.get(slot) else {
                return;
            };
            f.banished[usize::from(i)] = true;
            f.banishes_left = f.banishes_left.saturating_sub(1);
            events.push(DomainEvent::RewardBanished {
                reward: id(rewards, i),
            });
            match draw(rewards, f, &player.runes, rng, o.rarity, &o.candidates) {
                Some(new) => {
                    o.candidates[slot] = new;
                    events.push(DomainEvent::CandidateReplaced {
                        slot: candidate,
                        reward: id(rewards, new),
                    });
                }
                None => {
                    o.candidates.remove(slot);
                }
            }
            !o.candidates.is_empty()
        }
    };
    if open {
        f.offer = Some(o);
    }
}

/// What the frontend shows of the open offer.
pub(crate) fn offer_view(rewards: &Rewards, s: &State) -> Option<OfferView> {
    let f = s.stars.as_ref()?;
    let o = f.offer.as_ref()?;
    Some(OfferView {
        kind: o.kind,
        rarity: o.rarity,
        candidates: o
            .candidates
            .iter()
            .map(|i| {
                let r = &rewards.rewards()[usize::from(*i)];
                CandidateView {
                    reward: r.id.clone(),
                    rarity: r.rarity,
                    hidden: r.is_random_rune(),
                    named: r.named,
                    star: r.star.clone(),
                }
            })
            .collect(),
        options: options(rewards, s),
        rerolls_left: f.rerolls_left,
        banishes_left: f.banishes_left,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules_v0::{RunDifficulty, RunPool, initial_state};
    use crate::run_content::RunContent;

    const ALL: (Rarity, Rarity) = (Rarity::Common, Rarity::Supernova);

    fn weights(common: u16, rare: u16, legendary: u16) -> RarityWeights {
        RarityWeights {
            common,
            rare,
            legendary,
            ..RarityWeights::default()
        }
    }

    #[test]
    fn a_range_leaves_out_the_rarities_outside_it() {
        let w = weights(50, 35, 15);
        let draws = |range| -> Vec<Rarity> {
            let mut rng = SplitMix64::new(9);
            (0..400).map(|_| draw_rarity(&mut rng, &w, range)).collect()
        };
        let all = draws(ALL);
        assert!(all.contains(&Rarity::Common) && all.contains(&Rarity::Legendary));
        let capped = draws((Rarity::Common, Rarity::Rare));
        assert!(capped.iter().all(|r| *r <= Rarity::Rare));
        assert!(capped.contains(&Rarity::Rare));
        let raised = draws((Rarity::Rare, Rarity::Supernova));
        assert!(raised.iter().all(|r| *r >= Rarity::Rare));
    }

    #[test]
    fn a_range_that_leaves_no_weight_gives_its_lowest_rarity() {
        let mut rng = SplitMix64::new(1);
        let only_common = weights(100, 0, 0);
        assert_eq!(
            draw_rarity(&mut rng, &only_common, (Rarity::Rare, Rarity::Supernova)),
            Rarity::Rare
        );
    }

    #[test]
    fn a_range_that_leaves_every_weight_in_changes_no_draw() {
        let w = weights(50, 35, 15);
        let (mut a, mut b) = (SplitMix64::new(3), SplitMix64::new(3));
        for _ in 0..100 {
            let wide = draw_rarity(&mut a, &w, ALL);
            let exact = draw_rarity(&mut b, &w, (Rarity::Common, Rarity::Legendary));
            assert_eq!(wide, exact);
        }
    }

    #[test]
    fn a_boss_offer_at_the_tiers_highest_rarity_stays_inside_the_difficultys_range() {
        // Level 1's tier tops at supernova, and easy's range tops at rare: the boss offer is rare.
        // (A run-level test cannot reach it: the player would have to be at level 5 first.)
        let mut stage = RunContent::bundled().trial().clone();
        let st = stage.stars.as_mut().expect("the trial stage has stars");
        st.tiers = vec![Tier {
            from_level: 1,
            weights: RarityWeights {
                supernova: 1,
                ..RarityWeights::default()
            },
        }];
        let pool = RunPool {
            rewards: (0..3).collect(),
            extra_rerolls: 0,
            upgrades: 0,
            difficulty: RunDifficulty {
                hp_pct: 70,
                damage_pct: 75,
                rarity: (Rarity::Common, Rarity::Rare),
            },
        };
        let mut s = initial_state(&stage, RunContent::bundled(), pool, 1);
        boss_killed(&stage, &mut s);
        let due = &s.stars.as_ref().expect("a star field").due;
        assert_eq!(due.len(), 1, "{due:?}");
        assert_eq!(
            (due[0].kind, due[0].rarity),
            (OfferKind::Boss, Rarity::Rare)
        );
    }

    #[test]
    fn heal_per_kills_picks_add_their_amounts_and_heal_at_the_shortest_interval() {
        let heal = |id: &str, amount: u32, kills: u32| {
            serde_json::json!({ "id": id, "rarity": "epic", "named": true, "star": id,
                "limit": 1, "effects": [{ "effect": "heal_per_kills", "amount": amount,
                "kills": kills }], "title": "Heal", "description": "" })
        };
        let file = serde_json::json!({ "schema_version": 0, "sim_version": 0,
            "rewards": [heal("a", 300, 10), heal("b", 200, 4)] });
        let rewards = Rewards::parse(file.to_string().as_bytes()).expect("valid");
        let stage = RunContent::bundled().trial().clone();
        let pool = RunPool {
            rewards: vec![0, 1],
            extra_rerolls: 0,
            upgrades: 0,
            difficulty: RunDifficulty {
                hp_pct: 100,
                damage_pct: 100,
                rarity: ALL,
            },
        };
        let mut s = initial_state(&stage, RunContent::bundled(), pool, 1);
        let State {
            rng,
            stars,
            progress,
            player,
            regen,
            ..
        } = &mut s;
        let f = stars.as_mut().expect("a star field");
        take(&stage, &rewards, rng, progress, player, regen, f, 0);
        assert_eq!((player.kill_heal, player.kill_heal_every), (300, 10));
        take(&stage, &rewards, rng, progress, player, regen, f, 1);
        assert_eq!((player.kill_heal, player.kill_heal_every), (500, 4));
    }
}
