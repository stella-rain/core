//! The named stars' combat rewards (core#NS2b, app#336): critical hits, echoes of hits and kill
//! bursts. They act on the main character's own hits only: a hit of a rune, of a
//! damage-over-time status, an echo and a burst neither crit nor echo.
//!
//! A run without any of the rewards runs none of this: every entry point returns at once, the
//! state writes nothing for them to the hash and no random number is drawn.

use super::entity::Who;
use crate::attack::MatchUp;
use crate::engine::{Echo, PlayerState};
use crate::event::DomainEvent;
use crate::fixed::Fx;
use crate::id::EntityId;
use crate::rewards::{
    MAX_BURST_DEPTH, MAX_BURST_PCT, MAX_BURST_RADIUS_PX, MAX_BURSTS_PER_TICK,
    MAX_CRIT_CHANCE_TOTAL_PCT, MAX_CRIT_DAMAGE_TOTAL_PCT, MAX_ECHO_HIT_PCT, MAX_ECHOES_PER_TICK,
};
use crate::rng::SplitMix64;
use crate::stage::Stage;

use super::agents::agent_def;
use super::{PLAYER_ID, Shot, State, overlaps};

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

/// Kill bursts, after the tick's hits and echoes have landed and before the kills are credited.
/// Each enemy at 0 hit points that was defeated (not a self-destruct) bursts, in entity-ID
/// order: the other enemies within the burst's radius of it (the edge of their circle counts)
/// take the reward's percent of its maximum hit points (at least 1) as the main character's
/// damage, which defence and vulnerability change as for any hit but which neither crits nor
/// echoes. What a burst defeats bursts in the next round, in entity-ID order, down to the
/// reward's depth: a chain of at most `MAX_BURST_DEPTH` links. At most `MAX_BURSTS_PER_TICK`
/// bursts go off in a tick; a kill past that is still credited, it just does not burst. A
/// burst's kills are credited like any other (experience, drops, kills for a heal); the depth
/// is all that stops the chain. The boss and its parts are not hurt by a burst.
pub(super) fn bursts(stage: &Stage, s: &mut State, events: &mut Vec<DomainEvent>) {
    let p = &s.player;
    if p.burst_pct == 0 {
        return;
    }
    let pct = u64::from(p.burst_pct.min(MAX_BURST_PCT));
    let radius = i64::from(p.burst_radius.min(MAX_BURST_RADIUS_PX)) * i64::from(Fx::px(1).0);
    let radius = i32::try_from(radius).unwrap_or(i32::MAX);
    let depth = p.burst_depth.clamp(1, MAX_BURST_DEPTH);
    let fallen = |s: &State| -> Vec<usize> {
        (0..s.enemies.len())
            .filter(|&i| s.enemies[i].hp == 0 && !s.enemies[i].burst)
            .collect()
    };
    let mut round = fallen(s);
    let mut gone_off = 0;
    for link in 1..=depth {
        let mut next = Vec::new();
        for &i in &round {
            if gone_off == MAX_BURSTS_PER_TICK {
                return;
            }
            gone_off += 1;
            let (at, max_hp) = (s.enemies[i].at, s.enemies[i].max_hp);
            events.push(DomainEvent::KillBurst {
                at,
                radius,
                depth: u8::try_from(link).unwrap_or(u8::MAX),
            });
            let amount = u32::try_from(u64::from(max_hp) * pct / 100)
                .unwrap_or(u32::MAX)
                .max(1);
            let shot = Shot {
                at,
                owner: PLAYER_ID,
                friendly: true,
                damage: amount,
                damage_type: None,
                breaks_charge: false,
                smash: None,
                combo: false,
                rune: None,
                chain: true,
            };
            for j in 0..s.enemies.len() {
                let e = &s.enemies[j];
                if j == i
                    || e.hp == 0
                    || !overlaps(at, radius, e.at, agent_def(stage, e.key).radius.0)
                {
                    continue;
                }
                super::damage(stage, s, shot, Who::Enemy(j), events);
                if s.enemies[j].hp == 0 {
                    next.push(j);
                }
            }
        }
        round = next;
        round.sort_unstable();
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

    fn burst_state(pct: u32, radius: u32, depth: u32) -> State {
        let mut s = state();
        (
            s.player.burst_pct,
            s.player.burst_radius,
            s.player.burst_depth,
        ) = (pct, radius, depth);
        s
    }

    /// An enemy of `max_hp` at `x` pixels along the top edge, at `hp`.
    fn at_px(s: &mut State, id: u32, x: i32, hp: u32, max_hp: u32) {
        with_enemy(s, id, hp);
        let e = s.enemies.last_mut().unwrap();
        e.at = Point {
            x: Fx::px(x),
            y: Fx::px(0),
        };
        e.max_hp = max_hp;
    }

    fn run(s: &mut State) -> Vec<DomainEvent> {
        let stage = RunContent::bundled().trial().clone();
        let mut events = Vec::new();
        bursts(&stage, s, &mut events);
        events
    }

    fn depths(events: &[DomainEvent]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|e| match e {
                DomainEvent::KillBurst { depth, .. } => Some(*depth),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_run_without_the_reward_bursts_nothing() {
        let mut s = state();
        at_px(&mut s, 1, 0, 0, 100);
        at_px(&mut s, 2, 10, 50, 100);
        assert!(run(&mut s).is_empty());
        assert_eq!(s.enemies[1].hp, 50);
    }

    #[test]
    fn a_burst_takes_its_percent_of_max_hit_points_from_those_in_reach_only() {
        let mut s = burst_state(40, 100, 1);
        at_px(&mut s, 1, 0, 0, 100);
        at_px(&mut s, 2, 90, 500, 500);
        at_px(&mut s, 3, 400, 500, 500);
        let events = run(&mut s);
        assert_eq!(depths(&events), [1]);
        // 40 of 100; the one 400 px away is untouched. No defence in the trial's enemies.
        assert_eq!((s.enemies[1].hp, s.enemies[2].hp), (460, 500));
        assert_eq!(s.enemies[0].hp, 0, "the dead are not hurt again");
    }

    #[test]
    fn a_self_destruct_does_not_burst() {
        let mut s = burst_state(100, 240, 3);
        at_px(&mut s, 1, 0, 0, 100);
        s.enemies[0].burst = true;
        at_px(&mut s, 2, 10, 50, 100);
        assert!(run(&mut s).is_empty());
    }

    #[test]
    fn a_chain_stops_at_its_depth() {
        for depth in 1..=3u32 {
            let mut s = burst_state(100, 100, depth);
            at_px(&mut s, 1, 0, 0, 100);
            for n in 1..=4 {
                at_px(&mut s, 1 + n, 90 * i32::try_from(n).unwrap(), 10, 100);
            }
            let events = run(&mut s);
            let dead = s.enemies.iter().filter(|e| e.hp == 0).count();
            assert_eq!(dead, 1 + depth as usize, "depth {depth}");
            assert_eq!(depths(&events), (1..=depth as u8).collect::<Vec<_>>());
        }
    }

    #[test]
    fn at_most_eight_bursts_go_off_in_a_tick_and_the_earliest_ids_first() {
        let mut s = burst_state(100, 1, 3);
        // Ten fallen enemies far apart: each its own root.
        for n in 0..10 {
            at_px(&mut s, 1 + n, 1_000 * i32::try_from(n).unwrap(), 0, 100);
        }
        let events = run(&mut s);
        assert_eq!(depths(&events), [1; MAX_BURSTS_PER_TICK]);
        let places: Vec<i32> = events
            .iter()
            .filter_map(|e| match e {
                DomainEvent::KillBurst { at, .. } => Some(at.x.0 / 256),
                _ => None,
            })
            .collect();
        assert_eq!(places, (0..8).map(|n| 1_000 * n).collect::<Vec<_>>());
    }

    #[test]
    fn the_totals_are_held_to_their_bounds() {
        let mut s = burst_state(u32::MAX, u32::MAX, u32::MAX);
        at_px(&mut s, 1, 0, 0, 100);
        at_px(&mut s, 2, 239, 1_000, 1_000);
        at_px(&mut s, 3, 241 + 8, 1_000, 1_000);
        run(&mut s);
        // 100 percent of 100, 240 px (+8 px of the enemy's own circle) of reach.
        assert_eq!(s.enemies[1].hp, 900);
        assert_eq!(s.enemies[2].hp, 1_000);
    }
}
