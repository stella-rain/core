//! Healing (a stage with `heal`): a boss kill heals the main character `boss_pct` percent of its
//! most hit points, and a regular enemy's kill may drop a heal pickup, which the main character
//! takes on touch for `amount` hit points. Both heal as far as the most allows (`Healed`).
//!
//! The drop draws from the run's one generator (ADR-033, the same stream as the falling stars):
//! in the collisions, after the stars' drops of the tick, each regular kill in entity-ID order
//! draws once, `drop_one_in` being the range, and drops a pickup where the enemy fell when the
//! draw is 0 and the map holds fewer than `cap`. The draw happens whether or not the map has
//! room, so the stream advances once per regular kill and nothing else. An elite drops none. A
//! boss kill draws nothing. Then the main character takes every pickup inside its pickup circle,
//! in the order they dropped. Nothing heals a main character at 0 hit points.

use super::Run;
use crate::engine::{HealPickup, PlayerState, State};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::validate::limits::MAX_HEAL_AMOUNT;

use super::{PLAYER_ID, overlaps, widened};

/// The boss died this tick: it heals the main character `boss_pct` percent of its most.
pub(super) fn boss_killed(stage: &Run, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.heal else {
        return;
    };
    if s.player.hp == 0 {
        return;
    }
    let most = u64::from(s.player.max_hp);
    let want = most * u64::from(cfg.boss_pct) / 100;
    heal(&mut s.player, want, events);
}

/// Stars and pickups for the regular enemies at 0 hp, before they are removed (see the module
/// doc): one draw each.
pub(super) fn drop_from_defeated(stage: &Run, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.heal else {
        return;
    };
    let State {
        rng,
        enemies,
        next_id,
        heal_pickups,
        ..
    } = s;
    let Some(pickups) = heal_pickups.as_mut() else {
        return;
    };
    for e in enemies
        .iter()
        .filter(|e| e.hp == 0 && !e.burst && e.elite.is_none())
    {
        let drops = rng.below(cfg.drop_one_in) == 0;
        if !drops || pickups.len() >= usize::from(cfg.cap) {
            continue;
        }
        let id = EntityId(*next_id);
        *next_id += 1;
        pickups.push(HealPickup { id, at: e.at });
        events.push(DomainEvent::HealDropped { pickup: id });
    }
}

/// The main character takes the pickups inside its circle, in the order they dropped.
pub(super) fn pick_up(stage: &Run, s: &mut State, events: &mut Vec<DomainEvent>) {
    let Some(cfg) = &stage.heal else {
        return;
    };
    if s.player.hp == 0 {
        return;
    }
    let at = s.player.at;
    let radius = widened(cfg.pickup_radius.0, s.player.pickup_radius_pct);
    let taken = {
        let Some(pickups) = s.heal_pickups.as_mut() else {
            return;
        };
        let (taken, kept): (Vec<HealPickup>, Vec<HealPickup>) = std::mem::take(pickups)
            .into_iter()
            .partition(|p| overlaps(at, radius, p.at, 0));
        *pickups = kept;
        taken
    };
    for _ in taken {
        heal(&mut s.player, u64::from(cfg.amount), events);
    }
}

/// The enemies at 0 hp this tick that give credit (not a self-destruct) count toward the
/// heal-per-kills rewards: every `kill_heal_every` kills the main character heals `kill_heal`
/// hit points, held to `MAX_HEAL_AMOUNT`, once for each interval counted. A main character at 0
/// hit points heals nothing but still counts.
pub(super) fn kills_heal(s: &mut State, events: &mut Vec<DomainEvent>) {
    let every = s.player.kill_heal_every;
    if every == 0 {
        return;
    }
    let kills = s.enemies.iter().filter(|e| e.hp == 0 && !e.burst).count();
    let counted = u64::from(s.player.kills_counted) + u64::try_from(kills).unwrap_or(u64::MAX);
    let heals = counted / u64::from(every);
    s.player.kills_counted = u32::try_from(counted % u64::from(every)).unwrap_or(0);
    if heals == 0 || s.player.hp == 0 {
        return;
    }
    let each = u64::from(s.player.kill_heal.min(MAX_HEAL_AMOUNT));
    heal(&mut s.player, each * heals, events);
}

/// Heals `player` by `want` hit points, as far as its most allows, and reports what it healed.
fn heal(player: &mut PlayerState, want: u64, events: &mut Vec<DomainEvent>) {
    let before = player.hp;
    let healed = (u64::from(before) + want).min(u64::from(player.max_hp));
    player.hp = u32::try_from(healed).unwrap_or(player.max_hp);
    events.push(DomainEvent::Healed {
        target: PLAYER_ID,
        amount: player.hp.saturating_sub(before),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewards::Rarity;
    use crate::rules_v0::{RunDifficulty, RunPool, initial_state};
    use crate::run_content::RunContent;

    fn state() -> State {
        let stage = super::super::Run::for_test(RunContent::bundled().trial().clone());
        let pool = RunPool {
            rewards: Vec::new(),
            extra_rerolls: 0,
            difficulty: RunDifficulty {
                hp_pct: 100,
                damage_pct: 100,
                rarity: (Rarity::Common, Rarity::Supernova),
            },
        };
        let mut s = initial_state(
            &stage,
            RunContent::bundled(),
            pool,
            1,
            crate::kinds::KindTable::of_stage(&stage).unwrap(),
        );
        s.player.max_hp = 100_000;
        s.player.hp = 1_000;
        s
    }

    fn healed(events: &[DomainEvent]) -> Vec<u32> {
        events
            .iter()
            .filter_map(|e| match e {
                DomainEvent::Healed { amount, .. } => Some(*amount),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nothing_is_counted_or_healed_without_the_reward() {
        let mut s = state();
        let mut events = Vec::new();
        kills_heal(&mut s, &mut events);
        assert_eq!((s.player.kills_counted, healed(&events)), (0, vec![]));
    }

    /// `credited` defeated enemies and one that blew itself up, all at 0 hit points.
    fn defeated(s: &mut State, credited: u32) {
        use crate::engine::{AgentKey, AgentSource};
        use crate::fixed::Point;
        use crate::id::EntityId;
        use crate::rules_v0::agents::new_agent;
        let stage = super::super::Run::for_test(RunContent::bundled().trial().clone());
        let key = AgentKey {
            source: AgentSource::Wave,
            index: 0,
            rule: 0,
        };
        for n in 0..=credited {
            let mut e = new_agent(
                EntityId(100 + n),
                stage.defs.of_key(key),
                stage.defs.get(stage.defs.of_key(key)),
                key,
                Point::default(),
                0,
            );
            e.hp = 0;
            e.burst = n == credited;
            s.enemies.push(e);
        }
    }

    #[test]
    fn it_heals_every_n_kills_that_give_credit_and_carries_the_rest() {
        let mut s = state();
        (s.player.kill_heal, s.player.kill_heal_every) = (500, 3);
        let mut events = Vec::new();
        defeated(&mut s, 2);
        kills_heal(&mut s, &mut events);
        // Two kills and a self-destruct: two counted, no heal yet.
        assert_eq!((s.player.kills_counted, healed(&events)), (2, vec![]));
        s.enemies.clear();
        defeated(&mut s, 5);
        kills_heal(&mut s, &mut events);
        // Seven in all: two intervals of three, one carried.
        assert_eq!(healed(&events), vec![1_000]);
        assert_eq!((s.player.hp, s.player.kills_counted), (2_000, 1));
    }

    #[test]
    fn a_main_character_at_0_hit_points_counts_but_is_not_healed() {
        let mut s = state();
        (s.player.kill_heal, s.player.kill_heal_every) = (500, 1);
        s.player.hp = 0;
        let mut events = Vec::new();
        defeated(&mut s, 2);
        kills_heal(&mut s, &mut events);
        assert_eq!((s.player.hp, healed(&events)), (0, vec![]));
    }
}
