//! Experience, levels and reward offers (ADR-051 §2), in a stage with `levels`.
//!
//! Defeated enemies give their wave's experience as they are removed in the collisions. Each
//! level reached waits for its offer; at the end of the tick, if the run goes on, the first
//! waiting offer is drawn from the run's generator: up to `OFFER_SIZE` different upgrades, from
//! those below `MAX_STACKS` in `Upgrade::ALL` order, by a partial Fisher-Yates shuffle (one
//! `below` per option). With none left, the level passes without an offer. A choice applies its
//! upgrade, then the next waiting offer opens on the same tick.
//!
//! What a stack does (tuning values): `MaxHp` one more hit point at most and one healed;
//! `Attack` 10% more damage; `Haste` 8% shorter automatic-attack cooldowns; `Evade` a 10%
//! shorter evade cooldown; `Speed` 8% more speed. Percentages add up per stack and round down;
//! a cooldown stays at least 1 tick.

use crate::engine::{AgentSource, State};
use crate::event::DomainEvent;
use crate::stage::Stage;
use crate::upgrade::{MAX_STACKS, OFFER_SIZE, Upgrade};

/// The highest level; past it experience is no longer counted (ADR-020: it bounds the offers).
pub(crate) const MAX_LEVEL: u32 = 100;
/// The experience of an enemy whose wave gives none, or that no wave spawned (a summon).
const DEFAULT_XP: u32 = 1;

const ATTACK_PCT: u32 = 10;
const HASTE_PCT: u32 = 8;
const EVADE_PCT: u32 = 10;
const SPEED_PCT: u32 = 8;

/// The experience level `level` needs to reach the next; 0 at `MAX_LEVEL` or without levels.
pub(crate) fn xp_next(stage: &Stage, level: u32) -> u32 {
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
pub(super) fn credit_defeated(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    if s.progress.is_none() {
        return;
    }
    let xp = s
        .enemies
        .iter()
        .filter(|e| e.hp == 0)
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

fn gain(stage: &Stage, s: &mut State, xp: u32, events: &mut Vec<DomainEvent>) {
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
        p.pending += 1;
        events.push(DomainEvent::LeveledUp { level: p.level });
    }
    if p.level >= MAX_LEVEL {
        p.xp = 0;
    }
}

/// Opens the first waiting offer, if none is open; levels with nothing left to offer pass.
pub(super) fn open_offer(s: &mut State, events: &mut Vec<DomainEvent>) {
    let State { progress, rng, .. } = s;
    let Some(p) = progress.as_mut() else {
        return;
    };
    while p.offer.is_none() && p.pending > 0 {
        p.pending -= 1;
        let mut pool: Vec<Upgrade> = Upgrade::ALL
            .iter()
            .zip(p.stacks)
            .filter(|(_, n)| *n < MAX_STACKS)
            .map(|(u, _)| *u)
            .collect();
        if pool.is_empty() {
            continue;
        }
        let len = u32::try_from(pool.len()).unwrap_or(u32::MAX);
        let take = OFFER_SIZE.min(len);
        for i in 0..take {
            let j = i + rng.below(len - i);
            pool.swap(i as usize, j as usize);
        }
        pool.truncate(take as usize);
        events.push(DomainEvent::OfferOpened {
            options: pool.clone(),
        });
        p.offer = Some(pool);
    }
}

/// Applies option `index` of the open offer (checked by `Engine::choose`), then opens the next.
pub(crate) fn choose(s: &mut State, index: u32, events: &mut Vec<DomainEvent>) {
    let Some(upgrade) = s
        .progress
        .as_mut()
        .and_then(|p| p.offer.take())
        .and_then(|o| o.get(index as usize).copied())
    else {
        return;
    };
    if let Some(p) = s.progress.as_mut() {
        let n = &mut p.stacks[upgrade as usize];
        *n = n.saturating_add(1).min(MAX_STACKS);
    }
    if upgrade == Upgrade::MaxHp {
        let player = &mut s.player;
        player.max_hp = player.max_hp.saturating_add(1);
        player.hp = player.hp.saturating_add(1).min(player.max_hp);
    }
    events.push(DomainEvent::UpgradeChosen { upgrade });
    open_offer(s, events);
}

fn stacks(s: &State, upgrade: Upgrade) -> u32 {
    s.progress
        .as_ref()
        .map_or(0, |p| u32::from(p.stacks[upgrade as usize]))
}

fn raised(value: u32, pct: u32) -> u32 {
    let raised = u64::from(value) * (100 + u64::from(pct)) / 100;
    u32::try_from(raised).unwrap_or(u32::MAX)
}

fn shortened(ticks: u32, pct: u32) -> u32 {
    let kept = u64::from(ticks) * u64::from(100u32.saturating_sub(pct)) / 100;
    u32::try_from(kept).unwrap_or(u32::MAX).max(1).min(ticks)
}

/// The main character's attack stat with its `Attack` stacks.
pub(super) fn attack(s: &State, atk: u32) -> u32 {
    raised(atk, ATTACK_PCT * stacks(s, Upgrade::Attack))
}

/// An automatic attack's cooldown with the `Haste` stacks.
pub(super) fn auto_cooldown(s: &State, ticks: u32) -> u32 {
    shortened(ticks, HASTE_PCT * stacks(s, Upgrade::Haste))
}

/// The evade's cooldown with the `Evade` stacks.
pub(super) fn evade_cooldown(s: &State, ticks: u32) -> u32 {
    shortened(ticks, EVADE_PCT * stacks(s, Upgrade::Evade))
}

/// A speed cap with the `Speed` stacks.
pub(super) fn speed(s: &State, cap: i64) -> i64 {
    cap * (100 + i64::from(SPEED_PCT * stacks(s, Upgrade::Speed))) / 100
}
