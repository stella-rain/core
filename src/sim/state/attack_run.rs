//! Attack runs, their frames and paths, and the index of moves by owner (ADR-SIM-3).

use crate::attack::DamageType;
use crate::fixed::Point;
use crate::id::EntityId;
use crate::inline::InlineVec;
use crate::sim::{AttackIndex, ProgramIndex, World};
use crate::validate::{self};

use super::{Aim, Dir, Smash};

/// The frames of an attack task: one for the pattern and one for each level of nesting, at most
/// `MAX_PATTERN_DEPTH` of them, and some to spare.
pub(crate) type Frames = InlineVec<Frame, MAX_FRAMES>;
const MAX_FRAMES: usize = 8;
/// A path into an attack program: one step for each level of nesting, and a hooked orb sits at
/// depth `MAX_PATTERN_DEPTH - 1` at the deepest (its burst is one level below), so the path is
/// never longer than `MAX_PATTERN_DEPTH`. Inline, so that an orb's hook allocates nothing (T1b).
pub(crate) type Path = InlineVec<PathStep, { validate::limits::MAX_PATTERN_DEPTH as usize }>;

/// One step down a program: the node at `node` in the current list, then its `body` list, or
/// its `burst` list if `then` is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PathStep {
    pub node: u16,
    pub then: bool,
}

/// Where an attack's orbs and zones start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// Wherever its owner is, for as long as the owner lives.
    Owner,
    /// A fixed point: where an orb was when it started this attack.
    At(Point),
}

/// A place in the middle of a list of nodes, and the state of the loop or turn whose body the
/// next frame up is running.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Frame {
    /// The node of this frame's list that is being carried out.
    pub idx: u16,
    pub loop_i: u32,
    pub loop_n: u32,
    /// A spread's angle.
    pub param: i32,
    /// The turn this node adds to the directions of its body.
    pub offset: i32,
}

/// A phased attack an attacker is carrying out: one at a time per attacker and slot.
/// Its phases spawn their nodes as `AttackTask`s, which run to their own end.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct AttackRun {
    pub owner: EntityId,
    /// 0 for an agent's or the boss's attacks and the main character's combo.
    pub slot: u8,
    /// The attack it carries out, compiled when the run started (`World::attacks`).
    pub attack: AttackIndex,
    /// The attack speed that shortens its phases, in percent, and the range that widens its hit
    /// boxes, fixed when it starts; read through `attacks::phase`.
    pub speed_pct: u32,
    pub width_pct: u32,
    pub phase: u32,
    /// Ticks done in the phase: 0 on its first tick.
    pub tick: u32,
    /// Locked when the attack starts.
    pub facing: Dir,
    pub aim: Aim,
    pub friendly: bool,
    /// Started by a `ranged` rule: while in wind-up or active it holds a ranged token (core Z2).
    pub ranged: bool,
    /// The damage its starter gives a phase that sets none: the main character's
    /// attack, a shot skill's own damage, 1 for an enemy or the boss.
    pub base: u32,
    /// The attacker's atk-up, in percent, fixed when the attack starts.
    pub atk_up_pct: u32,
    /// The percent of the damage a difficulty scales an enemy's or the boss's attack by
    /// fixed when the attack starts; 100 for the main character's and its allies'.
    pub damage_pct: u32,
    /// The attacker's type bonus for each damage type, by its place in `DamageType::ALL`, in
    /// percent, each at most the match-up file's `max_type_bonus_pct`, fixed when the attack
    /// starts: the main character's, once it holds any; none for every other
    /// attacker. Held inline: boxing it allocated on every launch (T1b).
    pub type_bonus_pct: Option<[u32; DamageType::ALL.len()]>,
    /// `phase_time` of the attack's first tick, for the nodes its phases spawn.
    pub age: u32,
    /// Who the current active phase has hit, and on which tick last, sorted by entity ID;
    /// emptied when an active phase begins.
    pub hits: Vec<(EntityId, u32)>,
    /// The Sub weapon's pull, when the attack is a combo part.
    pub smash: Option<Smash>,
    /// Where its hit boxes are centred instead of on its attacker: an on-hit rune's attack, at
    /// the point the combo step hit, or where a thrown one has flown to.
    pub at: Option<Point>,
    /// A thrown rune's turn point, the point the combo step hit: its first phase flies `at`
    /// there from the main character, its second back to the main character (boomerang). A
    /// homing missile's goal: where its target is, or was last.
    pub throw: Option<Point>,
    /// A homing missile's target: `throw` follows it while it is alive, and the
    /// missile is gone once it hits anything.
    pub homing: Option<EntityId>,
    /// Where its target stood when it began, for the hit boxes and zones anchored at the target
    /// (core Z1); only an attack that has one holds it.
    pub spot: Option<Point>,
}

impl Clone for AttackRun {
    fn clone(&self) -> AttackRun {
        let AttackRun {
            owner,
            slot,
            attack,
            speed_pct,
            width_pct,
            phase,
            tick,
            facing,
            aim,
            friendly,
            ranged,
            base,
            atk_up_pct,
            damage_pct,
            type_bonus_pct,
            age,
            hits,
            smash,
            at,
            throw,
            homing,
            spot,
        } = self;
        AttackRun {
            owner: *owner,
            slot: *slot,
            attack: *attack,
            speed_pct: *speed_pct,
            width_pct: *width_pct,
            phase: *phase,
            tick: *tick,
            facing: *facing,
            aim: *aim,
            friendly: *friendly,
            ranged: *ranged,
            base: *base,
            atk_up_pct: *atk_up_pct,
            damage_pct: *damage_pct,
            type_bonus_pct: *type_bonus_pct,
            age: *age,
            hits: hits.clone(),
            smash: *smash,
            at: *at,
            throw: *throw,
            homing: *homing,
            spot: *spot,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &AttackRun) {
        let AttackRun {
            owner,
            slot,
            attack,
            speed_pct,
            width_pct,
            phase,
            tick,
            facing,
            aim,
            friendly,
            ranged,
            base,
            atk_up_pct,
            damage_pct,
            type_bonus_pct,
            age,
            hits,
            smash,
            at,
            throw,
            homing,
            spot,
        } = self;
        let AttackRun {
            owner: s_owner,
            slot: s_slot,
            attack: s_attack,
            speed_pct: s_speed_pct,
            width_pct: s_width_pct,
            phase: s_phase,
            tick: s_tick,
            facing: s_facing,
            aim: s_aim,
            friendly: s_friendly,
            ranged: s_ranged,
            base: s_base,
            atk_up_pct: s_atk_up_pct,
            damage_pct: s_damage_pct,
            type_bonus_pct: s_type_bonus_pct,
            age: s_age,
            hits: s_hits,
            smash: s_smash,
            at: s_at,
            throw: s_throw,
            homing: s_homing,
            spot: s_spot,
        } = source;
        *owner = *s_owner;
        *slot = *s_slot;
        *attack = *s_attack;
        *speed_pct = *s_speed_pct;
        *width_pct = *s_width_pct;
        *phase = *s_phase;
        *tick = *s_tick;
        *facing = *s_facing;
        *aim = *s_aim;
        *friendly = *s_friendly;
        *ranged = *s_ranged;
        *base = *s_base;
        *atk_up_pct = *s_atk_up_pct;
        *damage_pct = *s_damage_pct;
        *type_bonus_pct = *s_type_bonus_pct;
        *age = *s_age;
        hits.clone_from(s_hits);
        *smash = *s_smash;
        *at = *s_at;
        *throw = *s_throw;
        *homing = *s_homing;
        *spot = *s_spot;
    }
}

/// The movement speed, in percent, that the running attacks leave each attacker: the lowest
/// `move_pct` of the current phases of its runs. Kept beside `State::runs` so that asking costs
/// a search over the attackers, not a scan of every run; a derived value that no hash reads.
/// Sorted by owner, one entry for each owner with a run.
#[derive(Debug, Default)]
pub(crate) struct MoveIndex {
    by_owner: Vec<(EntityId, i64)>,
}

impl Clone for MoveIndex {
    fn clone(&self) -> MoveIndex {
        let MoveIndex { by_owner } = self;
        MoveIndex {
            by_owner: by_owner.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &MoveIndex) {
        let MoveIndex { by_owner } = self;
        let MoveIndex {
            by_owner: s_by_owner,
        } = source;
        by_owner.clone_from(s_by_owner);
    }
}

impl MoveIndex {
    /// Room for `owners` entries (DIR1): one per attacker that can have a run.
    pub fn reserve(&mut self, owners: usize) {
        crate::sim::scratch::reserve_to(&mut self.by_owner, owners);
    }

    /// The speed `owner`'s runs leave it, 100 with none.
    #[inline]
    pub fn get(&self, owner: EntityId) -> i64 {
        let at = self.by_owner.partition_point(|e| e.0 < owner);
        match self.by_owner.get(at) {
            Some(e) if e.0 == owner => e.1,
            _ => 100,
        }
    }

    /// Takes in a run that starts, or whose phase changes to one, with this `move_pct`.
    pub fn lower(&mut self, owner: EntityId, pct: i64) {
        let at = self.by_owner.partition_point(|e| e.0 < owner);
        match self.by_owner.get_mut(at) {
            Some(e) if e.0 == owner => e.1 = e.1.min(pct),
            _ => self.by_owner.insert(at, (owner, pct)),
        }
    }

    /// Starts again from `runs`: after one ended or went on to another phase.
    pub fn rebuild(&mut self, world: &World, runs: &[AttackRun]) {
        self.by_owner.clear();
        self.by_owner.extend(runs.iter().map(|r| {
            let phase = &world.attacks[usize::from(r.attack.0)].phases[r.phase as usize];
            (r.owner, i64::from(phase.move_pct))
        }));
        // Lowest first within an owner, so that the entry `dedup_by_key` keeps is the lowest.
        self.by_owner.sort_unstable();
        self.by_owner.dedup_by_key(|e| e.0);
    }
}

/// An attack being carried out (ADR-SIM-4 tier 3): a program, where it has got to, and what it
/// needs to know to aim and to hurt. Presets are expanded before they get here, so a preset and
/// its inline copy are the same task.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct AttackTask {
    pub owner: EntityId,
    pub program: ProgramIndex,
    /// Where in the program this task starts; empty for the whole program.
    pub base: Path,
    pub origin: Origin,
    /// What `aimed` directions point at.
    pub aim: Aim,
    pub friendly: bool,
    pub damage: u32,
    /// The type of the phase that started it, which its orbs carry.
    pub damage_type: Option<DamageType>,
    /// The orbs it fires break the main character's charge (core#83).
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits.
    pub smash: Option<Smash>,
    /// Started by a combo step, A or B: its hits add expertise points.
    pub combo: bool,
    /// Ticks since it started.
    pub age: u32,
    /// Ticks to wait before going on.
    pub wait_left: u32,
    /// The stack of lists being carried out, outermost first.
    pub frames: Frames,
    /// The direction `relative` directions start from.
    pub parent_dir: Dir,
    /// The direction of the last orb, which `sequential` directions turn from.
    pub last_dir: Dir,
    /// The spot of the attack that started it (`AttackRun::spot`), for its zones anchored at the
    /// target.
    pub spot: Option<Point>,
}
