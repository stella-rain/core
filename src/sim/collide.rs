//! The hit loop of the live rules: orb movement, collision, the enemy grid and damage.

use super::damage::{self};
use super::entity::{Who, scale_damage, status_pct};
use super::grid::Grid;
use super::{
    Blame, Shot, World, buffer_of, combat, damageable, entity, expertise, heal, hit_boxes,
    hit_companions_side, hit_players_side, levels, profile, runes, slots, stardust, stars, state,
    stun,
};
use crate::attack::MatchUp;
use crate::event::{DomainEvent, HitKind};
use crate::fixed::{Fx, Point};
use crate::sim::state::PlayerIdx;
use crate::sim::state::{Smash, State};

/// The radius the enemy grid is built for (`Grid::rebuild`'s probe radius); it sets the cell side
/// with the largest enemy, so it stays what the friendly orbs' radius was.
const GRID_PROBE_RADIUS: i32 = Fx::px(2).0;

/// Moves every orb a tick, and drops those that leave: one whose lifetime is up leaves
/// before it moves, so an orb with `n` ticks is on the map for `n` ticks (core#147).
pub(super) fn move_orbs(stage: &World, s: &mut State) {
    let holds = stage.map().holds();
    s.orbs.retain_mut(|b| {
        if b.life == 0 {
            false
        } else {
            b.at.x = b.at.x.saturating_add(b.vx);
            b.at.y = b.at.y.saturating_add(b.vy);
            b.life -= 1;
            if let Some(n) = b.stop.as_mut().filter(|n| **n > 0) {
                *n -= 1;
                if *n == 0 {
                    b.vx = Fx(0);
                    b.vy = Fx(0);
                }
            }
            holds(b.at)
        }
    });
}

/// The orb that the beam from orb `i` runs to, if it is live: both are alive, in one group, and
/// neither is still flying toward its halt. A ring of two has one beam, from slot 0.
pub(crate) fn beam_to(orbs: &[crate::sim::state::OrbState], i: usize) -> Option<usize> {
    let from = orbs.get(i)?;
    let link = from.link?;
    if link.of < 2 || (link.of == 2 && link.slot == 1) {
        return None;
    }
    let flying = |b: &crate::sim::state::OrbState| b.stop.is_some_and(|n| n > 0);
    if flying(from) {
        return None;
    }
    let next = (link.slot + 1) % link.of;
    let j = orbs.iter().position(|b| {
        b.link
            .is_some_and(|l| l.group == link.group && l.slot == next)
    })?;
    (!flying(&orbs[j])).then_some(j)
}

/// Two circles overlap (touching counts), in 64-bit fixed point.
pub(super) fn overlaps(a: Point, ra: i32, b: Point, rb: i32) -> bool {
    let dx = i64::from(a.x.0) - i64::from(b.x.0);
    let dy = i64::from(a.y.0) - i64::from(b.y.0);
    let r = i64::from(ra) + i64::from(rb);
    dx * dx + dy * dy <= r * r
}

/// The percent a pickup circle of `me` is widened by: `pickup_pct_of` for its player.
pub(super) fn pickup_pct(s: &State, me: PlayerIdx) -> u32 {
    pickup_pct_of(s.players.career(me).progress.as_ref(), &s.players[me])
}

/// The percent a pickup circle of `player` is widened by: the stars' share plus the
/// `PickupRadiusPct` total of the run stats in `progress` (0 without levels), saturating, held to
/// `MAX_PICKUP_RADIUS_TOTAL_PCT`. What the snapshot and the offer rating show is this number.
pub(crate) fn pickup_pct_of(
    progress: Option<&state::Progress>,
    player: &state::PlayerState,
) -> u32 {
    let stat = progress.map_or(0, |p| {
        p.totals[crate::run_stats::StatEffect::PickupRadiusPct as usize]
    });
    player
        .pickup_radius_pct
        .saturating_add(stat)
        .min(crate::rewards::MAX_PICKUP_RADIUS_TOTAL_PCT)
}

/// A pickup circle of `radius` widened by `pct` percent, the percent held to
/// `MAX_PICKUP_RADIUS_TOTAL_PCT` (the pickup radius rewards).
pub(super) fn widened(radius: i32, pct: u32) -> i32 {
    let pct = i64::from(pct.min(crate::rewards::MAX_PICKUP_RADIUS_TOTAL_PCT));
    i32::try_from(i64::from(radius) * (100 + pct) / 100).unwrap_or(i32::MAX)
}

pub(super) fn collide(stage: &World, s: &mut State, dots: bool, events: &mut Vec<DomainEvent>) {
    if dots {
        entity::tick_dots(stage, s, events);
    }
    // The enemies do not move while hit boxes land, so one grid, built for the first friendly hit
    // box, serves all of them; a tick without one never builds it.
    let mut grid: Option<Grid> = None;
    let boss_alive = s.boss.as_ref().is_some_and(|b| b.hp > 0);
    combat::deal_echoes(stage, s, events);
    combat::deal_repeats(stage, s, events);
    hit_boxes::resolve(stage, s, &mut grid, events);
    let mut consumed = std::mem::take(&mut s.scratch.consumed);
    consumed.clear();
    for i in 0..s.orbs.len() {
        let b = &s.orbs[i];
        let shot = Shot {
            at: b.at,
            owner: b.owner,
            friendly: false,
            damage: b.damage,
            damage_type: b.damage_type,
            breaks_charge: b.breaks_charge,
            smash: b.smash,
            combo: b.combo,
            rune: None,
            chain: false,
        };
        let by = Blame::of_program(HitKind::Attack, b.program);
        consumed.push(
            hit_players_side(stage, s, shot, by, events)
                || hit_companions_side(stage, s, shot, events),
        );
    }
    let mut flags = consumed.iter();
    s.orbs.retain(|_| !flags.next().copied().unwrap_or(false));
    s.scratch.consumed = consumed;
    if let Some(grid) = grid {
        s.scratch.grid = grid;
    }
    s.companions.retain(|c| c.hp > 0);
    // A kill's burst, kill-heal and expertise go to the killer; the bursts of all slots share the
    // tick's cap.
    let mut gone_off = 0;
    for me in s.players.indices() {
        combat::bursts(stage, s, me, &mut gone_off, events);
    }
    levels::credit_defeated(stage, s, events);
    // Stardust is shared: every player that is up earns every kill's base.
    for me in s.players.indices() {
        stardust::credit_defeated(stage, s, me);
    }
    stars::drop_from_defeated(stage, s, events);
    heal::drop_from_defeated(stage, s, events);
    for me in s.players.indices() {
        heal::kills_heal(s, me, events);
    }
    slots::count_kills(s);
    // The record is by enemy index, which no step between a hit and this removal changes.
    debug_assert!(s.scratch.killers.len() <= s.enemies.len());
    s.enemies.retain(|e| e.hp > 0);
    s.scratch.killers.clear();
    if boss_alive && s.boss.as_ref().is_some_and(|b| b.hp == 0) {
        stars::boss_killed(stage, s);
        for me in s.players.indices() {
            heal::boss_killed(stage, s, me, events);
            stardust::boss_killed(stage, s, me);
        }
    }
    // The stars and pick-ups on the map are shared: the lowest slot takes what its circle
    // covers, and the next finds it gone.
    for me in s.players.indices() {
        stars::pick_up(stage, s, me, events);
        heal::pick_up(stage, s, me, events);
    }
}

/// The enemies as circles, in index order, for friendly hit boxes to land on and for the
/// push-apart. It is built in the state's scratch grid, which the caller returns when it is done.
pub(super) fn enemy_grid(stage: &World, s: &mut State) -> Grid {
    let mut grid = std::mem::take(&mut s.scratch.grid);
    let circles = s
        .enemies
        .iter()
        .map(|e| (e.at, stage.defs.agent(e.def).radius.0));
    grid.rebuild(circles, GRID_PROBE_RADIUS);
    grid
}

/// Damage to an enemy, a companion or the boss: raised by its vulnerability, changed by its
/// match-up, reduced by its defence, reported, and credited to the shooter's aggro. Death is
/// reported too. A combo step's hit adds expertise points (`expertise`), puts the
/// damage-over-time runes' statuses on a target it leaves standing, and fires the on-hit runes
/// that are ready (`runes`). The hit points, the kill credit, the events and the aggro are
/// `damage::land`.
pub(super) fn damage(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    b: Shot,
    target: Who,
    events: &mut Vec<DomainEvent>,
) {
    // A shrine, or a mimic in disguise, is an object: nothing damages it.
    if !damageable(s, target) {
        return;
    }
    let vulnerable = status_pct(
        s.statuses(target),
        crate::sim::state::StatusKind::Vulnerable,
    );
    // C03: a staggered elite or boss takes more (a part takes its boss's).
    let vulnerable = if stage.has_stagger {
        stun::raised(stage, s, target, vulnerable)
    } else {
        vulnerable
    };
    let match_up = profile(stage, s, target).map_or(MatchUp::None, |p| p.match_up(b.damage_type));
    let id = s.id_of(target);
    // The common case, with no combat reward held, pays for one comparison (ADR-SIM-5).
    let crit = s.players[me].crit_chance_pct != 0
        && combat::crits(s, me, &b, id, s.hp(target), !State::is_ally(target));
    let base = if crit {
        combat::crit_damage(
            b.damage,
            s.players[me].crit_damage_pct,
            stage.tuning.crit_damage_base_pct,
        )
    } else {
        b.damage
    };
    let dealt = scale_damage(
        base,
        0,
        0,
        vulnerable,
        s.match_ups.pct(match_up),
        s.defence(target),
        stage.tuning.defence_k,
    );
    // The buffer is read only in a party, and for a player's own shot (ADR-SIM-7).
    let buffed_by =
        if !State::is_ally(target) && s.players.len() > 1 && s.players.of_id(b.owner).is_some() {
            buffer_of(s, me)
        } else {
            None
        };
    let how = damage::Landing {
        id,
        source: b.owner,
        damage_type: b.damage_type,
        match_up,
        rune: b.rune,
        crit,
        echo: false,
        repeat: false,
        buffed_by,
    };
    let dead = damage::land(stage, s, me, target, dealt, how, events).dead;
    if !dead && s.players[me].echo_hit_pct != 0 {
        combat::queue_echo(s, me, &b, id, dealt, !State::is_ally(target));
    }
    if b.combo {
        expertise::credit(stage, s, me, target, dead);
        if !dead && b.smash == Some(Smash::Bleed) {
            runes::apply_smash_bleed(s, me, target, events);
        }
        // A reap's burst may kill the target: what follows treats it as dead then.
        let mut dead = dead;
        if let Some(Smash::Reap { stacks, boss_pct }) = b.smash.filter(|_| !dead) {
            runes::reap_smash(stage, s, me, target, stacks, boss_pct, events);
            dead = !s.is_alive(target);
        }
        if let Some(Smash::Stun { ticks }) = b.smash.filter(|_| !dead) {
            stun::apply(stage, s, target, b.owner, ticks, events);
        }
        if !s.players[me].runes.is_empty() {
            if !dead {
                runes::apply_dots(s, me, target, events);
            }
            let at = s.at(target);
            let proc = super::player::combo_proc(stage, s, me);
            runes::on_hit(stage, s, me, id, at, events, proc);
        }
    }
}
