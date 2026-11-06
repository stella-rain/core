//! Experience, levels and stat points (ADR-REPL-5; DS18), in a stage with `levels`.
//!
//! Defeated enemies give their wave's experience, to the players who are up, as they are removed
//! in the collisions. Each level reached raises a run stat (ADR-CONT-1) by its step, and adds
//! the step to its effect's total, in one of two ways. By default the first stat of the run's
//! priority below its limit rises at once, with no pause (`StatRaised`). With `pick_stats` the level queues a stat offer
//! instead: it opens in `open_offers` after the boss and expertise offers, and `Engine::choose`
//! takes one of the stats below their limit, in the run stats file's order, with no reroll, skip
//! or banish. With every stat at its limit a level gives and opens nothing. What a total does:
//! `MaxHp` that many more hit points at most and
//! as many healed at once, unless the player is at 0 hit points (a revive alone gets it up);
//! `AttackPct` that percent more damage; `AttackSpeedPct` that percent
//! faster combo steps (`attacks::faster`); `EvadePct` that percent shorter evade cooldowns, at
//! least 1 tick; `SpeedPct` that
//! percent more speed; `XpPct` that percent more experience from each tick's kills. A reward
//! taken from an offer adds its amount to its effect's total the same way (`add_effect`).
//! Percentages round down.

use super::World;
use crate::event::DomainEvent;
use crate::id::ContentId;
use crate::run_stats::{self, RunStats, StatEffect};
use crate::sim::state::PlayerIdx;
use crate::sim::state::{AgentSource, PlayerState, Progress, State};
use crate::snapshot::{OfferOption, StatOfferView};

/// The highest level; past it experience is no longer counted (ADR-SVC-1: it bounds the points).
pub(crate) const MAX_LEVEL: u32 = 100;

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
    // Every player that is up earns it, each raised by its own `XpPct` (ADR-SIM-7 point 5).
    for me in s.players.indices() {
        if s.players[me].is_up() && s.players.career(me).progress.is_some() {
            gain(stage, s, me, xp, events);
        }
    }
}

pub(super) fn gain(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    xp: u32,
    events: &mut Vec<DomainEvent>,
) {
    let xp = raised(xp, total(s, me, StatEffect::XpPct));
    let (player, career) = s.players.both_mut(me);
    let Some(p) = career.progress.as_mut() else {
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
        // The equipment build never queues a stat offer: the point goes by the priority.
        if p.pick_stats && !stage.equipment_mode {
            p.offers_due = p.offers_due.saturating_add(1);
        } else if let Some(at) = first_below_limit(p) {
            raise(p, player, at, events);
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
    me: PlayerIdx,
    run_stats: &RunStats,
    priority: &[ContentId],
    pick_stats: bool,
) {
    let Some(p) = s.players.career_mut(me).progress.as_mut() else {
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

/// Opens the next stat offer due to `me`, unless `me` has an offer open. Levels due while every
/// stat is at its limit give nothing.
pub(super) fn open_due(s: &mut State, me: PlayerIdx, events: &mut Vec<DomainEvent>) {
    if super::offer_open_for(s, me) {
        return;
    }
    let Some(p) = s.players.career_mut(me).progress.as_mut() else {
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

/// The options of `me`'s open stat offer: a take of each stat below its limit.
pub(super) fn options(s: &State, me: PlayerIdx) -> Vec<OfferOption> {
    let career = s.players.career(me);
    career.progress.as_ref().map_or_else(Vec::new, |p| {
        (0..open_stats(p).len())
            .map(|i| OfferOption::Take {
                candidate: u8::try_from(i).unwrap_or(u8::MAX),
            })
            .collect()
    })
}

/// Takes `option`, one of `options`: raises that stat and closes the offer.
pub(super) fn choose(
    s: &mut State,
    me: PlayerIdx,
    option: OfferOption,
    events: &mut Vec<DomainEvent>,
) {
    let (player, career) = s.players.both_mut(me);
    let Some(p) = career.progress.as_mut() else {
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

/// The stat offer `me` has open, for the snapshot.
pub(crate) fn offer_view(s: &State, me: PlayerIdx) -> Option<StatOfferView> {
    let career = s.players.career(me);
    let p = career.progress.as_ref().filter(|p| p.offer_open)?;
    Some(StatOfferView {
        options: open_stats(p)
            .iter()
            .map(|&i| p.stats[i].id.clone())
            .collect(),
    })
}

/// Adds `amount` to the total of `effect`; `MaxHp` also raises the most hit points and heals
/// as many, unless the player is at 0 hit points.
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
        // Only a player that is up is healed: a player at 0 hit points gets up by a revive alone
        // (ADR-SIM-7), in a solo run too.
        if player.hp > 0 {
            player.hp = player.hp.saturating_add(amount).min(player.max_hp);
        }
    }
}

/// Takes `amount` off the total of `effect` (an unequip); `MaxHp` also lowers the most hit points,
/// never below 1 (0 is the downed state), and keeps the hit points within it: no heal, no loss
/// beyond the clamp, and a player at 0 hit points stays at 0.
pub(super) fn remove_effect(
    p: &mut Progress,
    player: &mut PlayerState,
    effect: StatEffect,
    amount: u32,
) {
    let total = &mut p.totals[effect as usize];
    *total = total.saturating_sub(amount);
    if effect == StatEffect::MaxHp {
        player.max_hp = player.max_hp.saturating_sub(amount).max(1);
        player.hp = player.hp.min(player.max_hp);
    }
}

pub(super) fn total(s: &State, me: PlayerIdx, effect: StatEffect) -> u32 {
    let career = s.players.career(me);
    career
        .progress
        .as_ref()
        .map_or(0, |p| p.totals[effect as usize])
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
pub(super) fn attack(s: &State, me: PlayerIdx, atk: u32) -> u32 {
    attack_of(s.players.career(me).progress.as_ref(), atk)
}

/// `atk` with the points `progress` holds spent on attack: `attack` for any one `Progress`.
#[inline]
pub(super) fn attack_of(progress: Option<&Progress>, atk: u32) -> u32 {
    raised(
        atk,
        progress.map_or(0, |p| p.totals[StatEffect::AttackPct as usize]),
    )
}

/// The main character's attack speed, in percent, with the points spent on it.
pub(super) fn attack_speed(s: &State, me: PlayerIdx) -> u32 {
    attack_speed_of(
        s.players.career(me).progress.as_ref(),
        s.max_attack_speed_pct,
    )
}

/// The attack speed, in percent, the points `progress` holds give, held to `cap`:
/// `attack_speed` for any one `Progress`.
#[inline]
pub(super) fn attack_speed_of(progress: Option<&Progress>, cap: u32) -> u32 {
    run_stats::capped_attack_speed_pct(
        progress.map_or(0, |p| p.totals[StatEffect::AttackSpeedPct as usize]),
        cap,
    )
}

/// The evade's cooldown with the points spent on evading.
pub(super) fn evade_cooldown(s: &State, me: PlayerIdx, ticks: u32) -> u32 {
    shortened(ticks, total(s, me, StatEffect::EvadePct))
}

/// A speed cap with the points spent on speed.
pub(super) fn speed(s: &State, me: PlayerIdx, cap: i64) -> i64 {
    cap * (100 + i64::from(total(s, me, StatEffect::SpeedPct))) / 100
}

#[cfg(test)]
mod tests {
    //! The helpers read the career of the player they are given (ADR-SIM-7).

    use super::*;
    use crate::bench_fixtures::worst_case;
    use crate::engine::Run;
    use crate::sim::state::{Career, Players};

    fn party() -> (World, State, PlayerIdx) {
        let e = worst_case::warmed_up();
        let Run::V2 { world, state } = e.run() else {
            panic!("the live rules")
        };
        let mut s = state.clone();
        let p = s.players[PlayerIdx::SOLO].clone();
        let mut progress = s
            .players
            .career(PlayerIdx::SOLO)
            .progress
            .clone()
            .expect("levels");
        progress.totals = [0; StatEffect::ALL.len()];
        s.players = Players::with_mask(0b011, p);
        s.players.career_mut(PlayerIdx::SOLO).progress = Some(progress.clone());
        let one = PlayerIdx::new(1).unwrap();
        *s.players.career_mut(one) = Career {
            progress: Some(progress),
            ..Career::default()
        };
        (world.clone(), s, one)
    }

    #[test]
    fn the_helpers_read_the_player_they_are_given() {
        let (_, mut s, one) = party();
        let t = |s: &mut State, e: StatEffect| {
            s.players.career_mut(one).progress.as_mut().unwrap().totals[e as usize] = 50;
        };
        t(&mut s, StatEffect::AttackPct);
        t(&mut s, StatEffect::EvadePct);
        t(&mut s, StatEffect::SpeedPct);
        assert_eq!(attack(&s, one, 100), 150);
        assert_eq!(attack(&s, PlayerIdx::SOLO, 100), 100);
        assert_eq!(evade_cooldown(&s, one, 100), 50);
        assert_eq!(evade_cooldown(&s, PlayerIdx::SOLO, 100), 100);
        assert_eq!(speed(&s, one, 100), 150);
        assert_eq!(speed(&s, PlayerIdx::SOLO, 100), 100);
        t(&mut s, StatEffect::AttackSpeedPct);
        assert!(attack_speed(&s, one) > attack_speed(&s, PlayerIdx::SOLO));
    }

    #[test]
    fn experience_goes_to_the_player_it_is_given() {
        let (world, mut s, one) = party();
        let before = s.players.career(PlayerIdx::SOLO).progress.clone();
        gain(&world, &mut s, one, 1, &mut Vec::new());
        assert_eq!(s.players.career(PlayerIdx::SOLO).progress, before);
        let mine = s.players.career(one).progress.as_ref().unwrap();
        assert!(mine.xp > 0 || mine.level > 1);
    }
}
