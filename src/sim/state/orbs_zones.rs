//! Directions and aim, and the orbs, links, zones and hooks an attack spawns (ADR-SIM-3).

use crate::attack::{DamageType, ZoneShape};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::sim::ProgramIndex;

use super::{Path, Smash};

/// A direction as a vector of length about `trig::SCALE`. Angle 0 is straight down the screen
/// and positive angles turn toward the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dir {
    pub x: i32,
    pub y: i32,
}

impl Dir {
    pub const DOWN: Dir = Dir {
        x: 0,
        y: crate::trig::SCALE,
    };
    pub const UP: Dir = Dir {
        x: 0,
        y: -crate::trig::SCALE,
    };
}

/// What an attack's `aimed` directions point at, and which way it faces. This is where the
/// ways of aiming meet: enemies, companions and the boss aim at an entity; the main
/// character's combo steps aim along its facing. A new way of aiming (an effect's
/// target) is a new variant, read in `attacks::aim`, hashed in its `StableHash` impl and given its
/// heading here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aim {
    /// At the entity with this ID; straight down if it is gone.
    Entity(EntityId),
    /// Along a fixed direction.
    Fixed(Dir),
}

impl Aim {
    /// The way the attack faces when it starts: where `relative` and `sequential` directions
    /// begin. An attack at an entity faces down the screen, as the enemies do.
    pub fn heading(self) -> Dir {
        match self {
            Aim::Entity(_) => Dir::DOWN,
            Aim::Fixed(d) => d,
        }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct OrbState {
    pub at: Point,
    pub vx: Fx,
    pub vy: Fx,
    /// The direction it was fired in, which `relative` directions of its children start from.
    pub dir: Dir,
    /// For the renderer; 0 is the default orb.
    pub style: u16,
    /// Who fired it, for the hit event and for aggro.
    pub owner: EntityId,
    pub damage: u32,
    /// The type of the phase that fired it; `None` is neutral.
    pub damage_type: Option<DamageType>,
    /// Its hit breaks the main character's charge (core#83).
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits.
    pub smash: Option<Smash>,
    /// Fired by a combo step, A or B: its hit adds expertise points.
    pub combo: bool,
    /// Starts an attack of its own when it ends: an orb's burst.
    pub hook: Option<Hook>,
    /// An orb the main character's hit boxes break, with no burst.
    pub breakable: bool,
    /// Ticks left before it leaves the map (core#147). It leaves when this runs out or when it is
    /// past the map's margin, whichever comes first.
    pub life: u32,
    /// Moves left before it halts (`stop_after`, Z3a-3); `Some(0)` is halted, `None` never halts.
    pub stop: Option<u32>,
    /// The beams this orb is part of (`link`, Z3a-3).
    pub link: Option<OrbLink>,
    /// The program that fired it, which names its attack in the hit event. Not in the state hash:
    /// nothing reads it back.
    pub program: ProgramIndex,
}

/// An orb's place in a cage (Z3a-3): the orbs of one `ring`, `spread` or `repeat` round of a
/// `link` node share a `group`; slot `j` is joined to slot `(j + 1) % of` by a beam while both are
/// alive and neither is still flying toward its halt.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct OrbLink {
    pub group: u32,
    /// The round of the loop that fired it.
    pub slot: u8,
    /// The slots made so far in the group, which the caps may leave short of the loop's count.
    pub of: u8,
    pub width: Fx,
    /// What a beam hit deals, already scaled by the difficulty.
    pub damage: u32,
    pub rehit_ticks: u32,
    /// The tick the beam from this orb to its neighbour last hit the main character.
    pub last_hit: Option<u32>,
}

/// A zone (core Z1): an area that does not move. It warns for `wind_left` ticks, then hurts for
/// `active_left`, then is gone. It has no ID: nothing refers to one.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ZoneState {
    /// Who placed it, for the hit event and for aggro.
    pub owner: EntityId,
    pub friendly: bool,
    pub shape: ZoneShape,
    /// The centre, fixed when it was placed.
    pub at: Point,
    /// The direction an arc or a line points, a unit vector.
    pub dir: Dir,
    /// Ticks left of the warning; 0 once it is active.
    pub wind_left: u32,
    /// Ticks left of the hurting, counted down once the warning is over.
    pub active_left: u32,
    /// The warning's length when it was placed. Snapshot only: not in the state hash.
    pub wind_ticks: u32,
    /// The hurting's length when it was placed. Snapshot only: not in the state hash.
    pub active_ticks: u32,
    pub damage: u32,
    pub damage_type: Option<DamageType>,
    pub breaks_charge: bool,
    /// A warning only (Z3a-4): it never becomes active and never hurts.
    pub warn_only: bool,
    /// Who it has hit, and on which tick, sorted by entity ID: each once.
    pub hits: Vec<(EntityId, u32)>,
    /// The program that placed it, which names its attack in the hit event. Not in the state
    /// hash: nothing reads it back.
    pub program: ProgramIndex,
}

impl Clone for ZoneState {
    fn clone(&self) -> ZoneState {
        let ZoneState {
            owner,
            friendly,
            shape,
            at,
            dir,
            wind_left,
            active_left,
            wind_ticks,
            active_ticks,
            damage,
            damage_type,
            breaks_charge,
            warn_only,
            hits,
            program,
        } = self;
        ZoneState {
            owner: *owner,
            friendly: *friendly,
            shape: shape.clone(),
            at: *at,
            dir: *dir,
            wind_left: *wind_left,
            active_left: *active_left,
            wind_ticks: *wind_ticks,
            active_ticks: *active_ticks,
            damage: *damage,
            damage_type: *damage_type,
            breaks_charge: *breaks_charge,
            warn_only: *warn_only,
            hits: hits.clone(),
            program: *program,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &ZoneState) {
        let ZoneState {
            owner,
            friendly,
            shape,
            at,
            dir,
            wind_left,
            active_left,
            wind_ticks,
            active_ticks,
            damage,
            damage_type,
            breaks_charge,
            warn_only,
            hits,
            program,
        } = self;
        let ZoneState {
            owner: s_owner,
            friendly: s_friendly,
            shape: s_shape,
            at: s_at,
            dir: s_dir,
            wind_left: s_wind_left,
            active_left: s_active_left,
            wind_ticks: s_wind_ticks,
            active_ticks: s_active_ticks,
            damage: s_damage,
            damage_type: s_damage_type,
            breaks_charge: s_breaks_charge,
            warn_only: s_warn_only,
            hits: s_hits,
            program: s_program,
        } = source;
        *owner = *s_owner;
        *friendly = *s_friendly;
        shape.clone_from(s_shape);
        *at = *s_at;
        *dir = *s_dir;
        *wind_left = *s_wind_left;
        *active_left = *s_active_left;
        *wind_ticks = *s_wind_ticks;
        *active_ticks = *s_active_ticks;
        *damage = *s_damage;
        *damage_type = *s_damage_type;
        *breaks_charge = *s_breaks_charge;
        *warn_only = *s_warn_only;
        hits.clone_from(s_hits);
        *program = *s_program;
    }
}

/// An orb's burst, waiting for the orb to end.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Hook {
    pub program: ProgramIndex,
    /// The `orb` node in the program: its `burst` list is what the orb starts.
    pub path: Path,
    /// Ticks left before it starts.
    pub left: u32,
    pub owner: EntityId,
    pub aim: Aim,
    pub damage: u32,
    pub damage_type: Option<DamageType>,
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits.
    pub smash: Option<Smash>,
    /// Of a combo step, A or B.
    pub combo: bool,
}
