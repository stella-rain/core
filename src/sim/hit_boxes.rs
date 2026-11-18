//! Hit boxes: every active phase with a hit box hurts the targets inside it, in the
//! collision step, before orbs.
//!
//! - A hit box follows its attacker, or stays on the point an on-hit rune's attack fired at,
//!   and faces the way the attack faced when it started. Its
//!   offset is forward along that facing and to the attacker's right as seen from above: a
//!   quarter turn clockwise on the screen, so facing down the screen its right is the screen's
//!   left.
//! - A hit box is anchored at its attacker, or at the target's spot (where the target stood when
//!   the attack began); a line is a rectangle from its start along the facing turned by its
//!   angle. Zones (`zones`) are tested the same way, after the hit boxes, each target once.
//! - A circle hits a target whose circle overlaps it; an arc, a target whose circle overlaps
//!   the sector `half_angle` to each side of the facing. Integer only: squared distances in 64
//!   or 128 bits, and the arc's edges and angle from the sine table.
//! - Friendly hit boxes hit the boss, its parts and the enemies (found through the enemy grid),
//!   in entity-ID order; enemy hit boxes hit the present players in slot order, then the companions
//!   once. A hit box
//!   hits every target inside it (cleave), each once per active phase, or again `rehit_ticks`
//!   after its last hit. A homing missile's hits only the first, once, and the
//!   missile is gone once every hit box has landed. A target that cannot be hurt (invulnerable, dead) is not hit and not
//!   counted as hit.
//! - The orb clear: on the first active tick of a player's full-gauge attack, every breakable orb
//!   within the run rules' `orb_clear_radius` of that player is removed before the swings land
//!   (`clear_orbs`), reported as a broken orb, with no burst.
//! - Attackers resolve in entity-ID order, and every hit box active at the start of the step
//!   lands, even if its attacker dies earlier in the step: hits on the same tick trade.

mod area;
mod hit_sides;
mod land;

use super::{World, aided, attacks, beam_to, hitter, knock_enemy, player, pull_enemy, state, stun};
use crate::attack::{Anchor, DamageType, HitBox, Offset, PhaseKind, ZoneShape};
#[cfg(test)]
use crate::event::HitKind;
#[cfg(test)]
use crate::fixed::Fx;
use crate::fixed::Point;
use crate::id::EntityId;
use crate::rune::Rune;
#[cfg(test)]
use crate::sim::state::PlayerIdx;
use crate::sim::state::{Dir, Smash, State};
pub(super) use area::{Area, centre};
use area::{Line, Sector, circle, covers};
use hit_sides::{hit_companions_side, hit_enemy_side, hit_players_side, hits_of};
#[cfg(test)]
pub(super) use land::apply_ends;
pub(super) use land::resolve;
#[cfg(test)]
use land::{break_orbs, cage_beams, land};

use super::entity::Who;
#[cfg(test)]
use super::scratch::RunEnd;
#[cfg(test)]
use super::{Blame, Shot};

/// What a swing is made by, and so where the targets it has hit are kept.
#[derive(Clone, Copy)]
enum Source {
    /// The attack run at this place in `State::runs`.
    Run(usize),
    /// The zone at this place in `State::zones` (core Z1).
    Zone(usize),
}

/// One hit box or zone that is active this tick.
pub(super) struct Swing {
    source: Source,
    owner: EntityId,
    slot: u8,
    friendly: bool,
    area: Area,
    damage: u32,
    damage_type: Option<DamageType>,
    rehit_ticks: Option<u32>,
    breaks_charge: bool,
    smash: Option<Smash>,
    combo: bool,
    /// The poise damage the hit phase names, if any (`AttackPhase::poise_damage`).
    poise_damage: Option<u32>,
    /// The stagger the hit phase names, if any (`AttackPhase::stagger`).
    stagger: Option<u32>,
    rune: Option<Rune>,
    /// A homing missile's: one hit, then nothing.
    once: bool,
}

impl Swing {
    /// The place of its source in `State::runs` or `State::zones`.
    fn place(&self) -> usize {
        match self.source {
            Source::Run(i) | Source::Zone(i) => i,
        }
    }
}

/// Puts the tick's hit boxes in owner, then slot order, in place: the stable sort allocates a
/// scratch buffer the size of the crowd's swings, and a steady tick allocates nothing (T1b). The
/// place is the tie-break, so the order is total. It equals the stable sort's for `active_swings`'
/// output, which lists the runs in `State::runs` order, so tied swings come in ascending place
/// (ADR-SIM-3); a caller that lists them in another order gets place order instead. Run before
/// `zone_swings`: a run's and a zone's place share one index space, so every swing here is a run.
fn sort_swings(swings: &mut [Swing]) {
    debug_assert!(swings.iter().all(|w| matches!(w.source, Source::Run(_))));
    swings.sort_unstable_by_key(|w| (w.owner.0, w.slot, w.place()));
}

fn active_swings(stage: &World, s: &State, swings: &mut Vec<Swing>) {
    swings.clear();
    for (i, r) in s.runs.iter().enumerate() {
        let p = super::attacks::phase(stage, r, r.phase as usize);
        let (PhaseKind::Active, Some(h)) = (p.kind, p.hit_box()) else {
            continue;
        };
        let Some(owner) = s.find(r.owner) else {
            continue;
        };
        let from = origin_of(s, r, h, owner);
        let area = area_of(h, from, r.facing);
        swings.push(Swing {
            source: Source::Run(i),
            owner: r.owner,
            slot: r.slot,
            friendly: r.friendly,
            area,
            damage: super::attacks::phase_damage(stage, r),
            damage_type: p.damage_type,
            rehit_ticks: p.rehit_ticks,
            breaks_charge: p.breaks_charge,
            smash: r.smash,
            combo: super::attacks::is_combo(s, r),
            poise_damage: p.poise_damage,
            stagger: p.stagger,
            rune: super::runes::of_run(r),
            once: r.homing.is_some(),
        });
    }
}

/// The point run `r`'s hit box `h` is centred from: the target's `spot` for a target-anchored
/// box that has one, else the run's own `at`, else `owner`'s position.
#[inline(always)]
pub(super) fn origin_of(s: &State, r: &super::state::AttackRun, h: &HitBox, owner: Who) -> Point {
    match (h.anchor(), r.spot) {
        (Anchor::Target, Some(spot)) => spot,
        _ => r.at.unwrap_or_else(|| s.at(owner)),
    }
}

/// The area of hit box `h` centred from `from` and facing `facing`.
#[inline(always)]
pub(super) fn area_of(h: &HitBox, from: Point, facing: Dir) -> Area {
    let at_offset = |offset: &Offset| centre(from, facing, offset.forward.0, offset.right.0);
    match h {
        HitBox::Circle { offset, radius, .. } => circle(at_offset(offset), facing, radius.0, None),
        HitBox::Arc {
            offset,
            radius,
            half_angle,
            ..
        } => circle(
            at_offset(offset),
            facing,
            radius.0,
            Sector::new(facing, *half_angle),
        ),
        HitBox::Line {
            offset,
            angle,
            length,
            width,
            ..
        } => Line::new(super::attacks::rotate(facing, *angle), *length, *width)
            .area(at_offset(offset)),
    }
}

/// Adds a swing for each zone that is active this tick.
fn zone_swings(s: &State, swings: &mut Vec<Swing>) {
    for (i, z) in s.zones.iter().enumerate() {
        if z.wind_left > 0 || z.warn_only {
            continue;
        }
        let area = match &z.shape {
            ZoneShape::Circle { radius } => circle(z.at, z.dir, radius.0, None),
            ZoneShape::Arc { radius, half_angle } => {
                circle(z.at, z.dir, radius.0, Sector::new(z.dir, *half_angle))
            }
            ZoneShape::Line { length, width } => Line::new(z.dir, *length, *width).area(z.at),
        };
        swings.push(Swing {
            source: Source::Zone(i),
            owner: z.owner,
            slot: 0,
            friendly: z.friendly,
            area,
            damage: z.damage,
            damage_type: z.damage_type,
            rehit_ticks: None,
            breaks_charge: z.breaks_charge,
            smash: None,
            combo: false,
            poise_damage: None,
            stagger: None,
            rune: None,
            once: false,
        });
    }
}

#[cfg(test)]
mod party_tests;
