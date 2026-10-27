//! Experience, levels and stat points (ADR-079 §2, §4; DS18), in a stage with `levels`.
//!
//! Defeated enemies give their wave's experience as they are removed in the collisions. Each
//! level reached raises a run stat (ADR-067) by its step, and adds the step to its effect's
//! total, in one of two ways. By default the first stat of the run's priority below its limit
//! rises at once, with no pause (`StatRaised`). With `pick_stats` the level queues a stat offer
//! instead: it opens in `open_offers` after the boss and expertise offers, and `Engine::choose`
//! takes one of the stats below their limit, in the run stats file's order, with no reroll, skip
//! or banish. With every stat at its limit a level gives and opens nothing. What a total does:
//! `MaxHp` that many more hit points at most and
//! as many healed at once; `AttackPct` that percent more damage; `AttackSpeedPct` that percent
//! faster combo steps (`attacks::faster`); `EvadePct` that percent shorter evade cooldowns, at
//! least 1 tick; `SpeedPct` that
//! percent more speed; `XpPct` that percent more experience from each tick's kills. A reward
//! taken from an offer adds its amount to its effect's total the same way (`add_effect`).
//! Percentages round down.

use super::World;
use crate::engine::{AgentSource, PlayerState, Progress, State};
use crate::event::DomainEvent;
use crate::id::ContentId;
use crate::run_stats::{self, RunStats, StatEffect};
use crate::snapshot::{OfferOption, StatOfferView};

/// The highest level; past it experience is no longer counted (ADR-020: it bounds the points).
pub(crate) const MAX_LEVEL: u32 = 100;
/// The experience of an enemy whose wave gives none, or that no wave spawned (a summon).
pub(super) const DEFAULT_XP: u32 = 1;

/// The experience level `level` needs to reach the next; 0 at `MAX_LEVEL` or without levels.
pub(crate) fn xp_next(stage: &World, level: u32) -> u32 {
    let Some(levels) = &stage.levels else {
        return 0;
    };
    if level >= MAX_LEVEL {
        return 0;
    }
    let i = usize::try_from(level.saturating_sub(1)).unwrap_or(usize::MAX);
    levels.xp.get(i).or(levels.xp.last()).copied().unwrap_or(0)
}

/// Experience for the enemies at 0 hp, before they are removed.
pub(super) fn credit_defeated(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    if s.progress.is_none() {
        return;
    }
    let xp = s
        .enemies
        .iter()
        .filter(|e| e.hp == 0 && !e.burst)
        .map(|e| match e.key.source {
            AgentSource::Wave => usize::try_from(e.key.index)
                .ok()
                .and_then(|i| stage.waves.get(i))
                .and_then(|w| w.xp)
                .unwrap_or(stage.tuning.default_xp),
            _ => stage.tuning.default_xp,
        })
        .fold(0u32, u32::saturating_add);
    gain(stage, s, xp, events);
}

fn gain(stage: &World, s: &mut State, xp: u32, events: &mut Vec<DomainEvent>) {
    let xp = raised(xp, total(s, StatEffect::XpPct));
    let Some(p) = s.progress.as_mut() else {
        return;
    };
    if xp == 0 || p.level >= MAX_LEVEL {
        return;
    }
    p.xp = p.xp.saturating_add(xp);
    loop {
        let need = xp_next(stage, p.level);
        if need == 0 || p.xp < need {
            break;
        }
        p.xp -= need;
        p.level += 1;
        events.push(DomainEvent::LeveledUp { level: p.level });
        if p.pick_stats {
            p.offers_due = p.offers_due.saturating_add(1);
        } else if let Some(at) = first_below_limit(p) {
            raise(p, &mut s.player, at, events);
        }
    }
    if p.level >= MAX_LEVEL {
        p.xp = 0;
    }
}

/// Sets how the run's level-ups are spent: `priority` (run stat IDs the setup validated; an
/// unknown or repeated one is ignored) first, then the stats it leaves out in their own order.
pub(crate) fn set_policy(
    s: &mut State,
    run_stats: &RunStats,
    priority: &[ContentId],
    pick_stats: bool,
) {
    let Some(p) = s.progress.as_mut() else {
        return;
    };
    let named = |id: &ContentId| run_stats.stats().iter().position(|st| st.id == *id);
    let mut order: Vec<u8> = Vec::with_capacity(run_stats.stats().len());
    for at in priority
        .iter()
        .filter_map(named)
        .chain(0..run_stats.stats().len())
    {
        let at = u8::try_from(at).unwrap_or(u8::MAX);
        if !order.contains(&at) {
            order.push(at);
        }
    }
    p.priority = order;
    p.pick_stats = pick_stats;
}

/// The place of the first stat in the priority that is below its limit.
fn first_below_limit(p: &Progress) -> Option<usize> {
    p.priority
        .iter()
        .map(|&i| usize::from(i))
        .find(|&i| below_limit(p, i))
}

fn below_limit(p: &Progress, at: usize) -> bool {
    match (p.spent.get(at), p.stats.get(at)) {
        (Some(spent), Some(rule)) => *spent < rule.limit,
        _ => false,
    }
}

/// The places of the stats below their limit, in the run stats' order.
fn open_stats(p: &Progress) -> Vec<usize> {
    (0..p.stats.len()).filter(|&i| below_limit(p, i)).collect()
}

/// Raises the run stat at `at` (below its limit) by its step.
fn raise(p: &mut Progress, player: &mut PlayerState, at: usize, events: &mut Vec<DomainEvent>) {
    let Some(rule) = p.stats.get(at) else {
        return;
    };
    let (id, effect, step) = (rule.id.clone(), rule.effect, rule.step);
    let Some(spent) = p.spent.get_mut(at) else {
        return;
    };
    *spent = spent.saturating_add(1);
    let spent = *spent;
    add_effect(p, player, effect, step);
    events.push(DomainEvent::StatRaised { stat: id, spent });
}

/// Opens the next stat offer due, unless an offer is open. Levels due while every stat is at its
/// limit give nothing.
pub(super) fn open_due(s: &mut State, events: &mut Vec<DomainEvent>) {
    if super::offer_open(s) {
        return;
    }
    let Some(p) = s.progress.as_mut() else {
        return;
    };
    if p.offers_due == 0 {
        return;
    }
    let options = open_stats(p);
    if options.is_empty() {
        p.offers_due = 0;
        return;
    }
    p.offers_due -= 1;
    p.offer_open = true;
    events.push(DomainEvent::StatOfferOpened {
        options: options.iter().map(|&i| p.stats[i].id.clone()).collect(),
    });
}

/// The open stat offer's options: a take of each stat below its limit.
pub(super) fn options(s: &State) -> Vec<OfferOption> {
    s.progress.as_ref().map_or_else(Vec::new, |p| {
        (0..open_stats(p).len())
            .map(|i| OfferOption::Take {
                candidate: u8::try_from(i).unwrap_or(u8::MAX),
            })
            .collect()
    })
}

/// Takes `option`, one of `options`: raises that stat and closes the offer.
pub(super) fn choose(s: &mut State, option: OfferOption, events: &mut Vec<DomainEvent>) {
    let State {
        progress, player, ..
    } = s;
    let Some(p) = progress.as_mut() else {
        return;
    };
    let OfferOption::Take { candidate } = option else {
        return;
    };
    if let Some(&at) = open_stats(p).get(usize::from(candidate)) {
        p.offer_open = false;
        raise(p, player, at, events);
    }
}

/// The open stat offer, for the snapshot.
pub(crate) fn offer_view(s: &State) -> Option<StatOfferView> {
    let p = s.progress.as_ref().filter(|p| p.offer_open)?;
    Some(StatOfferView {
        options: open_stats(p)
            .iter()
            .map(|&i| p.stats[i].id.clone())
            .collect(),
    })
}

/// Adds `amount` to the total of `effect`; `MaxHp` also raises the most hit points and heals
/// as many.
pub(super) fn add_effect(
    p: &mut Progress,
    player: &mut PlayerState,
    effect: StatEffect,
    amount: u32,
) {
    let total = &mut p.totals[effect as usize];
    *total = total.saturating_add(amount);
    if effect == StatEffect::MaxHp {
        player.max_hp = player.max_hp.saturating_add(amount);
        player.hp = player.hp.saturating_add(amount).min(player.max_hp);
    }
}

fn total(s: &State, effect: StatEffect) -> u32 {
    s.progress.as_ref().map_or(0, |p| p.totals[effect as usize])
}

fn raised(value: u32, pct: u32) -> u32 {
    let raised = u64::from(value) * (100 + u64::from(pct)) / 100;
    u32::try_from(raised).unwrap_or(u32::MAX)
}

fn shortened(ticks: u32, pct: u32) -> u32 {
    let kept = u64::from(ticks) * u64::from(100u32.saturating_sub(pct)) / 100;
    u32::try_from(kept).unwrap_or(u32::MAX).max(1).min(ticks)
}

/// The main character's attack stat with the points spent on attack.
pub(super) fn attack(s: &State, atk: u32) -> u32 {
    raised(atk, total(s, StatEffect::AttackPct))
}

/// The main character's attack speed, in percent, with the points spent on it.
pub(super) fn attack_speed(s: &State) -> u32 {
    run_stats::capped_attack_speed_pct(total(s, StatEffect::AttackSpeedPct), s.max_attack_speed_pct)
}

/// The evade's cooldown with the points spent on evading.
pub(super) fn evade_cooldown(s: &State, ticks: u32) -> u32 {
    shortened(ticks, total(s, StatEffect::EvadePct))
}

/// A speed cap with the points spent on speed.
pub(super) fn speed(s: &State, cap: i64) -> i64 {
    cap * (100 + i64::from(total(s, StatEffect::SpeedPct))) / 100
}
