//! The named stars' combat rewards (core#NS2b, app#336): critical hits, echoes of hits and kill
//! bursts. They act on the main character's own hits only: a hit of a rune, of a
//! damage-over-time status, an echo and a burst neither crit nor echo.
//!
//! A run without any of the rewards runs none of this: every entry point returns at once, the
//! state writes nothing for them to the hash and no random number is drawn.

use crate::attack::MatchUp;
use crate::engine::{Echo, PlayerState};
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::rewards::{
    MAX_CRIT_CHANCE_TOTAL_PCT, MAX_CRIT_DAMAGE_TOTAL_PCT, MAX_ECHO_HIT_PCT, MAX_ECHOES_PER_TICK,
};
use crate::rng::SplitMix64;

use super::{PLAYER_ID, Shot, State};

/// Whether any combat reward is held: what the state hash writes them for.
pub(super) fn held(p: &PlayerState) -> bool {
    p.crit_chance_pct > 0 || p.crit_damage_pct > 0 || p.echo_hit_pct > 0 || p.burst_pct > 0
}

/// Whether `b`, a hit on the foe `id` that has `hp` hit points before it, is a critical hit.
/// Only the main character's own hits on enemies, the boss and its parts roll, and only with
/// crit chance held. The roll is a hash of the run's seed, the tick, the target, its hit points
/// and where the shot landed, so two hits on one target in one tick differ and no draw is
/// taken from the run's generator.
pub(super) fn crits(s: &State, b: &Shot, id: EntityId, hp: u32, foe: bool) -> bool {
    let chance = s.player.crit_chance_pct.min(MAX_CRIT_CHANCE_TOTAL_PCT);
    if chance == 0 || b.chain || b.rune.is_some() || !b.friendly || b.owner != PLAYER_ID || !foe {
        return false;
    }
    let parts = [
        u64::from(s.tick),
        u64::from(id.0),
        u64::from(hp),
        u64::from(b.at.x.0 as u32),
        u64::from(b.at.y.0 as u32),
    ];
    let mut h = s.crit_seed;
    for part in parts {
        h = SplitMix64::new(h ^ part).next_u64();
    }
    h % 100 < u64::from(chance)
}

/// `damage` of a critical hit: twice as much, and `bonus_pct` percent more (held to
/// `MAX_CRIT_DAMAGE_TOTAL_PCT`).
pub(super) fn crit_damage(damage: u32, bonus_pct: u32) -> u32 {
    let pct = 200 + u64::from(bonus_pct.min(MAX_CRIT_DAMAGE_TOTAL_PCT));
    u32::try_from(u64::from(damage) * pct / 100).unwrap_or(u32::MAX)
}

/// Queues the echo of a hit that landed: `dealt` hit points on the foe `target`, which stayed
/// standing. Only the main character's own hits echo, as only they crit; an echo, a burst, a
/// rune's hit and a damage-over-time tick do not. The echo is the reward's percent of what the
/// hit dealt, at least 1, and is dealt a tick later, at the start of `collide`. The queue holds
/// `MAX_ECHOES_PER_TICK`: the hits after that, in the order they landed, echo nothing.
pub(super) fn queue_echo(s: &mut State, b: &Shot, target: EntityId, dealt: u32, foe: bool) {
    let pct = s.player.echo_hit_pct.min(MAX_ECHO_HIT_PCT);
    if pct == 0
        || dealt == 0
        || b.chain
        || b.rune.is_some()
        || !b.friendly
        || b.owner != PLAYER_ID
        || !foe
        || s.echoes.len() >= MAX_ECHOES_PER_TICK
    {
        return;
    }
    let damage = u32::try_from(u64::from(dealt) * u64::from(pct) / 100)
        .unwrap_or(u32::MAX)
        .max(1);
    s.echoes.push(Echo {
        source: b.owner,
        target,
        damage,
    });
}

/// Deals last tick's echoes, in the order they were queued, before this tick's hit boxes and
/// bullets land. An echo takes its hit points from the target it was queued on, if that is
/// still standing, and nothing else: it neither crits, echoes, fires a rune nor adds expertise.
/// What it defeats is credited with the rest of the tick's kills.
pub(super) fn deal_echoes(s: &mut State, events: &mut Vec<DomainEvent>) {
    if s.echoes.is_empty() {
        return;
    }
    for echo in std::mem::take(&mut s.echoes) {
        let Some(target) = s.find(echo.target).filter(|t| !State::is_ally(*t)) else {
            continue;
        };
        if s.hp(target) == 0 || !super::damageable(s, target) {
            continue;
        }
        let hp = s.hp_mut(target);
        *hp = hp.saturating_sub(echo.damage);
        let dead = *hp == 0;
        events.push(DomainEvent::Hit {
            source: echo.source,
            target: echo.target,
            damage: echo.damage,
            match_up: MatchUp::None,
            rune: None,
            crit: false,
            echo: true,
        });
        if dead {
            events.push(super::death_event(s, target, echo.target));
        }
        s.raise_aggro(
            echo.source,
            echo.damage.saturating_mul(super::AGGRO_PER_DAMAGE),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::Point;
    use crate::rewards::Rarity;
    use crate::rules_v0::{RunDifficulty, RunPool, initial_state};
    use crate::run_content::RunContent;

    fn state() -> State {
        let stage = RunContent::bundled().trial().clone();
        let pool = RunPool {
            rewards: Vec::new(),
            extra_rerolls: 0,
            upgrades: 0,
            difficulty: RunDifficulty {
                hp_pct: 100,
                damage_pct: 100,
                rarity: (Rarity::Common, Rarity::Supernova),
            },
        };
        initial_state(&stage, RunContent::bundled(), pool, 1)
    }

    fn shot(owner: EntityId) -> Shot {
        Shot {
            at: Point::default(),
            owner,
            friendly: true,
            damage: 100,
            damage_type: None,
            breaks_charge: false,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
        }
    }

    #[test]
    fn a_crit_damage_is_twice_the_damage_and_the_bonus() {
        assert_eq!(crit_damage(100, 0), 200);
        assert_eq!(crit_damage(100, 100), 300);
        assert_eq!(crit_damage(100, 300), 500);
        // Held to the total even if an unvalidated sum passes it.
        assert_eq!(crit_damage(100, u32::MAX), 500);
        assert_eq!(crit_damage(u32::MAX, 300), u32::MAX);
    }

    fn rolls(s: &mut State, b: Shot) -> usize {
        (0..1_000u32)
            .filter(|t| {
                s.tick = *t;
                crits(s, &b, EntityId(1), 500, true)
            })
            .count()
    }

    #[test]
    fn a_roll_draws_nothing_from_the_runs_generator() {
        let mut s = state();
        s.player.crit_chance_pct = 50;
        let before = s.rng.state();
        assert!(rolls(&mut s, shot(PLAYER_ID)) > 0);
        assert_eq!(s.rng.state(), before);
    }

    #[test]
    fn the_roll_follows_the_chance_and_only_the_main_characters_own_hits_on_foes() {
        let mut s = state();
        assert_eq!(rolls(&mut s, shot(PLAYER_ID)), 0, "no chance held");
        s.player.crit_chance_pct = 50;
        let half = rolls(&mut s, shot(PLAYER_ID));
        assert!((400..=600).contains(&half), "{half} of 1000");
        s.player.crit_chance_pct = u32::MAX;
        let most = rolls(&mut s, shot(PLAYER_ID));
        assert!((650..=850).contains(&most), "held to 75: {most}");
        let mut other = shot(PLAYER_ID);
        other.chain = true;
        assert_eq!(rolls(&mut s, other), 0, "a chained hit never crits");
        let mut rune = shot(PLAYER_ID);
        rune.rune = Some(crate::rune::Rune::ALL[0]);
        assert_eq!(rolls(&mut s, rune), 0, "nor a rune's");
        assert_eq!(rolls(&mut s, shot(EntityId(9))), 0, "nor another's hit");
        assert!(
            !crits(&s, &shot(PLAYER_ID), EntityId(1), 500, false),
            "nor on an ally"
        );
    }

    fn with_enemy(s: &mut State, id: u32, hp: u32) {
        use crate::engine::{AgentKey, AgentSource};
        use crate::rules_v0::agents::new_agent;
        let stage = RunContent::bundled().trial().clone();
        let key = AgentKey {
            source: AgentSource::Wave,
            index: 0,
            rule: 0,
        };
        let mut e = new_agent(
            EntityId(id),
            key,
            &stage.waves[0].enemy,
            Point::default(),
            0,
        );
        e.hp = hp;
        s.enemies.push(e);
    }

    #[test]
    fn echoes_are_queued_at_their_percent_and_capped_per_tick() {
        let mut s = state();
        queue_echo(&mut s, &shot(PLAYER_ID), EntityId(5), 100, true);
        assert!(s.echoes.is_empty(), "no echo reward held");
        s.player.echo_hit_pct = 35;
        queue_echo(&mut s, &shot(PLAYER_ID), EntityId(5), 100, true);
        queue_echo(&mut s, &shot(PLAYER_ID), EntityId(5), 1, true);
        assert_eq!(
            s.echoes.iter().map(|e| e.damage).collect::<Vec<_>>(),
            [35, 1],
            "35 percent, and at least 1"
        );
        let mut chained = shot(PLAYER_ID);
        chained.chain = true;
        queue_echo(&mut s, &chained, EntityId(5), 100, true);
        queue_echo(&mut s, &shot(EntityId(9)), EntityId(5), 100, true);
        queue_echo(&mut s, &shot(PLAYER_ID), EntityId(5), 100, false);
        queue_echo(&mut s, &shot(PLAYER_ID), EntityId(5), 0, true);
        assert_eq!(
            s.echoes.len(),
            2,
            "no echo of an echo, a stranger's hit or an ally"
        );
        for i in 0..100 {
            queue_echo(&mut s, &shot(PLAYER_ID), EntityId(i), 100, true);
        }
        assert_eq!(s.echoes.len(), MAX_ECHOES_PER_TICK);
        // The first to land are the ones kept.
        assert_eq!(s.echoes[2].target, EntityId(0));
    }

    #[test]
    fn echoes_are_dealt_in_order_once_and_dropped_when_their_target_is_gone() {
        let mut s = state();
        with_enemy(&mut s, 7, 50);
        for (target, damage) in [(7, 20), (8, 5), (7, 40)] {
            s.echoes.push(Echo {
                source: PLAYER_ID,
                target: EntityId(target),
                damage,
            });
        }
        let mut events = Vec::new();
        deal_echoes(&mut s, &mut events);
        assert!(s.echoes.is_empty());
        assert_eq!(s.enemies[0].hp, 0, "20 then 40 of 50");
        let types: Vec<_> = events
            .iter()
            .map(|e| match e {
                DomainEvent::Hit { damage, echo, .. } => (*damage, *echo),
                DomainEvent::Died { .. } => (0, false),
                other => panic!("{other:?}"),
            })
            .collect();
        // The echo of 5 on the missing enemy is dropped; the last one is dealt in full.
        assert_eq!(types, [(20, true), (40, true), (0, false)]);
        deal_echoes(&mut s, &mut events);
        assert_eq!(events.len(), 3, "nothing is dealt twice");
    }
}
