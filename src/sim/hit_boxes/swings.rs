//! The swings of a tick, in their order.

use crate::attack::{Anchor, DamageType, HitBox, Offset, PhaseKind, ZoneShape};
use crate::fixed::Point;
use crate::id::EntityId;
use crate::rune::Rune;
use crate::sim::World;
use crate::sim::entity::Who;
use crate::sim::state::{Dir, Smash, State};

use super::{Area, Line, Sector, centre, circle};

/// What a swing is made by, and so where the targets it has hit are kept.
#[derive(Clone, Copy)]
pub(super) enum Source {
    /// The attack run at this place in `State::runs`.
    Run(usize),
    /// The zone at this place in `State::zones` (core Z1).
    Zone(usize),
}

/// One hit box or zone that is active this tick.
pub(in crate::sim) struct Swing {
    pub(super) source: Source,
    pub(super) owner: EntityId,
    pub(super) slot: u8,
    pub(super) friendly: bool,
    pub(super) area: Area,
    pub(super) damage: u32,
    pub(super) damage_type: Option<DamageType>,
    pub(super) rehit_ticks: Option<u32>,
    pub(super) breaks_charge: bool,
    pub(super) smash: Option<Smash>,
    pub(super) combo: bool,
    /// The poise damage the hit phase names, if any (`AttackPhase::poise_damage`).
    pub(super) poise_damage: Option<u32>,
    /// The stagger the hit phase names, if any (`AttackPhase::stagger`).
    pub(super) stagger: Option<u32>,
    pub(super) rune: Option<Rune>,
    /// A homing missile's: one hit, then nothing.
    pub(super) once: bool,
}

impl Swing {
    /// The place of its source in `State::runs` or `State::zones`.
    pub(super) fn place(&self) -> usize {
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
pub(super) fn sort_swings(swings: &mut [Swing]) {
    debug_assert!(swings.iter().all(|w| matches!(w.source, Source::Run(_))));
    swings.sort_unstable_by_key(|w| (w.owner.0, w.slot, w.place()));
}

pub(super) fn active_swings(stage: &World, s: &State, swings: &mut Vec<Swing>) {
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
pub(in crate::sim) fn origin_of(
    s: &State,
    r: &super::state::AttackRun,
    h: &HitBox,
    owner: Who,
) -> Point {
    match (h.anchor(), r.spot) {
        (Anchor::Target, Some(spot)) => spot,
        _ => r.at.unwrap_or_else(|| s.at(owner)),
    }
}

/// The area of hit box `h` centred from `from` and facing `facing`.
#[inline(always)]
pub(in crate::sim) fn area_of(h: &HitBox, from: Point, facing: Dir) -> Area {
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
pub(super) fn zone_swings(s: &State, swings: &mut Vec<Swing>) {
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
