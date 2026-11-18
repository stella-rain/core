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
mod swings;

pub(super) use area::{Area, centre};
#[cfg(test)]
pub(super) use land::apply_ends;
pub(super) use land::resolve;
pub(super) use swings::{Swing, area_of, origin_of};

use super::{aided, attacks, beam_to, hitter, knock_enemy, player, pull_enemy, runes, state, stun};
use area::{Line, Sector, circle, covers};
use hit_sides::{hit_companions_side, hit_enemy_side, hit_players_side, hits_of};
use swings::{Source, active_swings, sort_swings, zone_swings};

// What only the tests of this module name.
#[cfg(test)]
use super::{Blame, Shot, World, scratch::RunEnd};
#[cfg(test)]
use {
    crate::event::HitKind,
    crate::fixed::{Fx, Point},
    crate::id::EntityId,
    crate::sim::state::{Dir, PlayerIdx, Smash, State},
    land::{break_orbs, cage_beams, land},
};

#[cfg(test)]
mod party_tests;
