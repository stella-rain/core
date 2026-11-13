//! Attack phases of the live rules: the compiled table, its view and the timing of a phase.

use super::super::World;
use crate::attack::{AttackPhase, Dash, HitBox, PhaseKind};
use crate::fixed::Fx;
use crate::id::EntityId;
use crate::rewards::MAX_RANGE_PCT;
use crate::sim::state::{AttackRun, State};
use std::ops::Deref;

/// The movement speed, in percent, that `owner`'s running attacks leave it: the lowest
/// `move_pct` of their current phases, 100 with none.
pub(in crate::sim) fn move_pct(world: &World, s: &State, owner: EntityId) -> i64 {
    let pct = s.move_index.get(owner);
    debug_assert_eq!(
        pct,
        scan_move_pct(world, s, owner),
        "the move index is stale"
    );
    pct
}

/// What `move_pct` was before `State::move_index`: every run scanned. The index is checked
/// against it in debug builds.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn scan_move_pct(world: &World, s: &State, owner: EntityId) -> i64 {
    s.runs
        .iter()
        .filter(|r| r.owner == owner)
        .map(|r| i64::from(phase(world, r, r.phase as usize).move_pct))
        .min()
        .unwrap_or(100)
}

/// A phase of a run as it is carried out: the compiled phase with attack speed and range
/// applied, which is read when needed instead of kept. It derefs to the compiled
/// phase for everything they leave alone.
pub(crate) struct PhaseView<'a> {
    base: &'a AttackPhase,
    /// The phase's length in ticks, shortened by attack speed.
    pub ticks: u32,
    widened: Option<HitBox>,
}

impl PhaseView<'_> {
    /// The phase's hit box, widened by the range.
    pub fn hit_box(&self) -> Option<&HitBox> {
        self.widened.as_ref().or(self.base.hit_box.as_ref())
    }
}

impl Deref for PhaseView<'_> {
    type Target = AttackPhase;

    fn deref(&self) -> &AttackPhase {
        self.base
    }
}

/// The compiled phases of run `r`'s attack, before attack speed and range.
pub(super) fn compiled<'a>(world: &'a World, r: &AttackRun) -> &'a [AttackPhase] {
    &world.attacks[usize::from(r.attack.0)].phases
}

/// The dash of run `r`'s current phase, if it is an active phase with one. Read from the compiled
/// phase, so that a tick that has no dash to move pays for a lookup, not for a `PhaseView`.
pub(in crate::sim) fn dash_of(world: &World, r: &AttackRun) -> Option<Dash> {
    let p = &compiled(world, r)[r.phase as usize];
    p.dash.filter(|_| p.kind == PhaseKind::Active)
}

/// The `advance` of run `r`'s current phase (attack magnetism): the slide of a combo step, any
/// phase kind. Read from the compiled phase, like `dash_of`.
pub(in crate::sim) fn advance_of(world: &World, r: &AttackRun) -> Option<crate::fixed::Fx> {
    compiled(world, r).get(r.phase as usize)?.advance
}

/// How many phases run `r` has.
pub(in crate::sim) fn phase_count(world: &World, r: &AttackRun) -> usize {
    compiled(world, r).len()
}

/// Phase `i` of run `r`, `r.speed_pct` percent faster and `r.width_pct` percent wider.
pub(crate) fn phase<'a>(world: &'a World, r: &AttackRun, i: usize) -> PhaseView<'a> {
    let phases = compiled(world, r);
    let base = &phases[i];
    PhaseView {
        base,
        ticks: phase_ticks(phases, r.speed_pct, i),
        widened: if r.width_pct == 0 {
            None
        } else {
            base.hit_box.as_ref().map(|h| wider_box(h, r.width_pct))
        },
    }
}

/// The length of phase `i` of `phases` at `speed_pct` percent attack speed: its own length at
/// 0, else `faster_ticks`.
#[inline]
pub(in crate::sim) fn phase_ticks(phases: &[AttackPhase], speed_pct: u32, i: usize) -> u32 {
    if speed_pct == 0 {
        phases[i].ticks
    } else {
        faster_ticks(phases, speed_pct, i)
    }
}

/// The length of phase `i` of `phases` `pct` percent faster: the attack's length divided by
/// `1 + pct / 100`, rounded to the nearest tick, and shared among the phases in proportion,
/// each at least 1 tick. The ticks left over after rounding each phase down go to the phases
/// with the largest remainders, the first of equal ones first.
fn faster_ticks(phases: &[AttackPhase], pct: u32, i: usize) -> u32 {
    let per = 100 + u64::from(pct);
    let total: u64 = phases.iter().map(|p| u64::from(p.ticks)).sum();
    let target = ((total * 100 + per / 2) / per).max(phases.len() as u64);
    let rounded = |p: &AttackPhase| {
        let exact = u64::from(p.ticks) * 100;
        let ticks = (exact / per).max(1);
        (ticks, exact.saturating_sub(ticks * per))
    };
    let (own, own_rest) = rounded(&phases[i]);
    let mut sum = 0u64;
    // Phases that come before this one in the order of the largest remainders.
    let mut before = 0u64;
    for (j, p) in phases.iter().enumerate() {
        let (ticks, rest) = rounded(p);
        sum += ticks;
        if rest > own_rest || (rest == own_rest && j < i) {
            before += 1;
        }
    }
    let ticks = u32::try_from(own).unwrap_or(u32::MAX);
    if before < target.saturating_sub(sum) {
        ticks.saturating_add(1)
    } else {
        ticks
    }
}

/// Phase `i` of `attack`'s hit box `pct` percent wider (a delayed repeat's place); none when the
/// phase has no hit box.
pub(in crate::sim) fn widened_box(
    world: &World,
    attack: crate::sim::AttackIndex,
    i: usize,
    pct: u32,
) -> Option<HitBox> {
    let h = world.attacks[usize::from(attack.0)]
        .phases
        .get(i)?
        .hit_box
        .as_ref()?;
    Some(if pct == 0 {
        h.clone()
    } else {
        wider_box(h, pct)
    })
}

/// `hit_box` `pct` percent larger, at most `MAX_RANGE_PCT`: its radius and its offset, each
/// rounded toward zero and held within `i32`; an arc's angle stays.
fn wider_box(hit_box: &HitBox, pct: u32) -> HitBox {
    // The validator holds the amount to `MAX_RANGE_PCT`; the clamp keeps an unvalidated file there.
    let per = 100 + i64::from(pct.min(MAX_RANGE_PCT));
    let grow = |v: &mut Fx| {
        v.0 = i32::try_from(i64::from(v.0) * per / 100).unwrap_or(if v.0 < 0 {
            i32::MIN
        } else {
            i32::MAX
        });
    };
    let mut out = hit_box.clone();
    match &mut out {
        HitBox::Circle { offset, radius, .. } | HitBox::Arc { offset, radius, .. } => {
            grow(radius);
            grow(&mut offset.forward);
            grow(&mut offset.right);
        }
        HitBox::Line {
            offset,
            length,
            width,
            ..
        } => {
            grow(length);
            grow(width);
            grow(&mut offset.forward);
            grow(&mut offset.right);
        }
    }
    out
}
