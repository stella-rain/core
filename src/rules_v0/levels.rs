//! Experience, levels and stat points (ADR-079 §2, §4), in a stage with `levels`.
//!
//! Defeated enemies give their wave's experience as they are removed in the collisions. Each
//! level reached gives one stat point and opens no offer; points wait, unspent, until the run
//! ends. Spending one, between ticks, raises a run stat (ADR-067) by its step, and adds the
//! step to its effect's total. What a total does: `MaxHp` that many more hit points at most and
//! as many healed at once; `AttackPct` that percent more damage; `AttackSpeedPct` that percent
//! faster combo steps (`attacks::faster`); `EvadePct` that percent shorter evade cooldowns, at
//! least 1 tick; `SpeedPct` that
//! percent more speed; `XpPct` that percent more experience from each tick's kills. A reward
//! taken from an offer adds its amount to its effect's total the same way (`add_effect`).
//! Percentages round down.

use super::Run;
use crate::engine::{AgentSource, PlayerState, Progress, State};
use crate::event::DomainEvent;
use crate::run_stats::{self, RunStat, StatEffect};

/// The highest level; past it experience is no longer counted (ADR-020: it bounds the points).
pub(crate) const MAX_LEVEL: u32 = 100;
/// The experience of an enemy whose wave gives none, or that no wave spawned (a summon).
const DEFAULT_XP: u32 = 1;

/// The experience level `level` needs to reach the next; 0 at `MAX_LEVEL` or without levels.
pub(crate) fn xp_next(stage: &Run, level: u32) -> u32 {
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
pub(super) fn credit_defeated(stage: &Run, s: &mut State, events: &mut Vec<DomainEvent>) {
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
                .unwrap_or(DEFAULT_XP),
            _ => DEFAULT_XP,
        })
        .fold(0u32, u32::saturating_add);
    gain(stage, s, xp, events);
}

fn gain(stage: &Run, s: &mut State, xp: u32, events: &mut Vec<DomainEvent>) {
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
        p.points += 1;
        events.push(DomainEvent::LeveledUp { level: p.level });
    }
    if p.level >= MAX_LEVEL {
        p.xp = 0;
    }
}

/// Spends a point on the run stat `stat` at `index` (checked by `Engine::spend`).
pub(crate) fn spend(s: &mut State, index: usize, stat: &RunStat, events: &mut Vec<DomainEvent>) {
    let Some(p) = s.progress.as_mut() else {
        return;
    };
    let Some(spent) = p.spent.get_mut(index) else {
        return;
    };
    *spent = spent.saturating_add(1);
    let spent = *spent;
    p.points -= 1;
    add_effect(p, &mut s.player, stat.effect, stat.step);
    events.push(DomainEvent::StatRaised {
        stat: stat.id.clone(),
        spent,
    });
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
