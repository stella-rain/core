//! The pass and the orb clear of the hit-box pass.

use crate::attack::PhaseKind;
use crate::event::{DomainEvent, HitKind};
use crate::fixed::Fx;
use crate::sim::state::{PlayerIdx, State};

use super::{
    Line, Swing, active_swings, covers, hit_companions_side, hit_enemy_side, hit_players_side,
    hits_of, sort_swings, zone_swings,
};
use crate::sim::World;
use crate::sim::grid::Grid;
use crate::sim::scratch::{RunEnd, Scratch};
use crate::sim::{Blame, Shot, enemy_grid, hurt_player};

/// Every active hit box hurts what it covers. `grid` is the enemy grid, built here if a
/// friendly hit box needs it and left for the orbs after.
// Kept out of `sim::step`, so the tick-cost gate measures the rules, not the inliner
// (ADR-SIM-5).
#[inline(never)]
pub(in crate::sim) fn resolve(
    stage: &World,
    s: &mut State,
    grid: &mut Option<Grid>,
    events: &mut Vec<DomainEvent>,
) {
    let mut swings = std::mem::take(&mut s.scratch.swings);
    let mut found = std::mem::take(&mut s.scratch.found);
    let mut bits = std::mem::take(&mut s.scratch.found_bits);
    active_swings(stage, s, &mut swings);
    sort_swings(&mut swings);
    // The zones land after the hit boxes, in the order they were placed.
    zone_swings(s, &mut swings);
    if !s.orbs.is_empty() {
        for me in s.players.indices() {
            break_orbs(stage, s, me, &swings, events);
        }
        if stage.has_gauge && stage.tuning.orb_clear_radius > 0 {
            clear_orbs(stage, s, events);
        }
    }
    land(stage, s, grid, &swings, (&mut bits, &mut found), events);
    if swings.iter().any(|w| w.once) {
        super::attacks::retain_runs(s, |r| r.homing.is_none() || r.hits.is_empty());
        s.move_index.rebuild(stage, &s.runs);
    }
    s.scratch.swings = swings;
    s.scratch.found = found;
    s.scratch.found_bits = bits;
}

/// The cage beams, then every swing, one after another over the runs the pass collected them
/// from.
pub(super) fn land(
    stage: &World,
    s: &mut State,
    grid: &mut Option<Grid>,
    swings: &[Swing],
    (bits, found): (&mut Vec<u64>, &mut Vec<usize>),
    events: &mut Vec<DomainEvent>,
) {
    // A run that ends in the pass (a player down, a hold broken, a stun) is ended after the
    // last swing, so the places the swings hold stay valid (ADR-SIM-7).
    s.scratch.ends_pinned = Some(s.runs.len());
    if s.next_orb_group > 0 {
        // A beam's `last_hit` is shared, so a beam hurts at most one player per `rehit_ticks`,
        // the lowest slot first.
        for me in s.players.indices() {
            cage_beams(stage, s, me, events);
        }
    }
    let (gauge, tempo) = (stage.has_gauge, stage.has_tempo);
    for w in swings {
        if w.friendly {
            let grid = grid.get_or_insert_with(|| enemy_grid(stage, s));
            hit_enemy_side(stage, s, grid, w, (bits, found), events);
            if (gauge || tempo) && w.combo {
                // The enemies this swing hit now, each counted once: the weapon gauge's fill and
                // the combo tempo's stack.
                let tick = s.tick;
                let n = u32::try_from(hits_of(s, w).iter().filter(|h| h.1 == tick).count())
                    .unwrap_or(u32::MAX);
                if gauge {
                    super::player::gauge_from_hits(stage, s, w.owner, n, events);
                }
                if tempo {
                    super::player::tempo_from_hits(stage, s, w.owner, n, events);
                }
            }
        } else {
            hit_players_side(stage, s, w, events);
            hit_companions_side(stage, s, w, events);
        }
    }
    apply_ends(stage, s);
}

/// Ends the runs that ended during the pass, in the order they were asked for, among the runs
/// that existed when it began: a run a hit started in the pass is not touched.
pub(in crate::sim) fn apply_ends(stage: &World, s: &mut State) {
    let pinned = s.scratch.ends_pinned.take().unwrap_or(s.runs.len());
    if s.scratch.ends_pending.is_empty() {
        return;
    }
    let ends = std::mem::take(&mut s.scratch.ends_pending);
    let (tick, mut at) = (s.tick, 0);
    s.runs.retain_mut(|r| {
        let place = at;
        at += 1;
        let ended = place < pinned
            && ends.iter().any(|e| match *e {
                RunEnd::Slot(owner, slot) => r.owner == owner && r.slot == slot,
                RunEnd::All(owner) | RunEnd::Cancel(owner) => r.owner == owner,
            });
        if ended {
            // A cancel cools a channelled run's token, once however many ends asked.
            let cancelled = ends.contains(&RunEnd::Cancel(r.owner));
            if let Some(pool) = cancelled
                .then(|| super::attacks::cancelled_token(stage, r))
                .flatten()
            {
                super::attacks::cool_into(&mut s.ranged_cooling, tick, stage, pool);
            }
            Scratch::recycle_hits(&mut s.scratch.hit_lists, std::mem::take(&mut r.hits));
        }
        !ended
    });
    s.move_index.rebuild(stage, &s.runs);
    let mut ends = ends;
    ends.clear();
    s.scratch.ends_pending = ends;
}

/// A breakable enemy orb that a hit box of the main character covers is removed, with no burst,
/// and reported once (core Z2, C24). It is gone before the orbs land, so it hurts nobody this
/// tick.
pub(super) fn break_orbs(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    swings: &[Swing],
    events: &mut Vec<DomainEvent>,
) {
    let owner = s.players.id(me);
    let mine = |w: &&Swing| w.friendly && w.owner == owner;
    s.orbs.retain(|b| {
        if !b.breakable {
            return true;
        }
        let hit = swings
            .iter()
            .filter(mine)
            .any(|w| covers(w, b.at, stage.tuning.orb_radius));
        if hit {
            events.push(DomainEvent::OrbBroken {
                owner: b.owner,
                at: b.at,
                player: Some(me),
            });
        }
        !hit
    });
}

/// The full-gauge smash's clear: on the first active tick of a player's full-gauge attack every
/// breakable orb within the run rules' `orb_clear_radius` of that player's centre (edge included)
/// is removed, with the same event as a broken orb and no burst. The attack is told by its
/// compiled index, which the pack check keeps apart from every attack of the base tree. One
/// player's clear is the party's. Zones and unbreakable orbs stay.
fn clear_orbs(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    let radius = i64::from(stage.tuning.orb_clear_radius);
    for me in s.players.indices() {
        let Some(crate::expertise::Gauge {
            spend: crate::expertise::GaugeSpend::FullSmash { attack },
            ..
        }) = stage.gauge_of(s, me)
        else {
            continue;
        };
        let (owner, attack) = (s.players.id(me), stage.attack_index(attack, None));
        let starting = s.runs.iter().any(|r| {
            r.owner == owner
                && r.slot == super::player::COMBO_SLOT
                && r.attack == attack
                && r.tick == 0
                && first_active(stage, r)
        });
        if !starting {
            continue;
        }
        let from = s.players[me].at;
        s.orbs.retain(|b| {
            let (dx, dy) = (
                i64::from(b.at.x.0) - i64::from(from.x.0),
                i64::from(b.at.y.0) - i64::from(from.y.0),
            );
            let cleared = b.breakable && dx * dx + dy * dy <= radius * radius;
            if cleared {
                events.push(DomainEvent::OrbBroken {
                    owner: b.owner,
                    at: b.at,
                    player: Some(me),
                });
            }
            !cleared
        });
    }
}

/// Whether `r` is in the first active phase of its attack (the one that clears its hit list).
fn first_active(stage: &World, r: &super::state::AttackRun) -> bool {
    let kind = |i: usize| super::attacks::phase(stage, r, i).kind;
    kind(r.phase as usize) == PhaseKind::Active
        && (r.phase == 0 || kind(r.phase as usize - 1) != PhaseKind::Active)
}

/// The beams of the orbs' cages hurt the main character (Z3a-3), a beam at most once per its
/// `rehit_ticks`; an orb broken just above is gone, and its beams with it. Each beam is a line
/// from one orb to its neighbour, tested like a hit box line; its last hit is kept on the orb it
/// starts from.
pub(super) fn cage_beams(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    events: &mut Vec<DomainEvent>,
) {
    for i in 0..s.orbs.len() {
        let p = &s.players[me];
        // An invulnerable player is only looked at while a perfect-evade window is open: the tick
        // cost of every other evade stays what it was.
        if p.hp == 0 || (p.invulnerable > 0 && p.combat.perfect_window == 0) {
            return;
        }
        let Some(j) = super::beam_to(&s.orbs, i) else {
            continue;
        };
        let (from, to) = (&s.orbs[i], &s.orbs[j]);
        let from_program = from.program;
        let Some(link) = from.link else {
            continue;
        };
        if link.last_hit.is_some_and(|t| s.tick - t < link.rehit_ticks) {
            continue;
        }
        let dx = i64::from(to.at.x.0) - i64::from(from.at.x.0);
        let dy = i64::from(to.at.y.0) - i64::from(from.at.y.0);
        let length = i32::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i32::MAX);
        let line = Line::new(super::attacks::normalize(dx, dy), Fx(length), link.width);
        let d = (
            i128::from(p.at.x.0) - i128::from(from.at.x.0),
            i128::from(p.at.y.0) - i128::from(from.at.y.0),
        );
        if !line.touches(d, stage.tuning.player_radius) {
            continue;
        }
        if p.invulnerable > 0 {
            // The beam is stopped by the invulnerability: a near miss, or nothing at all.
            if p.combat.perfect_window > 0 {
                super::player::near_miss(stage, s, me, events);
            }
            return;
        }
        let shot = Shot {
            at: p.at,
            owner: from.owner,
            friendly: false,
            damage: link.damage,
            damage_type: from.damage_type,
            breaks_charge: from.breaks_charge,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
            dot: false,
        };
        if let Some(l) = s.orbs[i].link.as_mut() {
            l.last_hit = Some(s.tick);
        }
        let by = Blame::of_program(HitKind::Link, from_program);
        hurt_player(stage, s, me, super::aided(stage, s, shot), by, events);
    }
}
