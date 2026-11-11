//! The foes: bosses and their parts, agents, shrines, leave and elite states (ADR-SIM-3).

use crate::behaviour::BossCall;
use crate::fixed::Point;
use crate::id::{ContentId, EntityId};
use crate::kinds::KindIndex;
use crate::sim::{BossIndex, DefIndex};
use crate::snapshot::EntityKind;

use super::{Rules, Statuses};

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct BossState {
    pub id: EntityId,
    /// Which of the run's boss definitions (`World::bosses`) it was made from. The definition
    /// is never changed; what differs by arrival is below.
    pub boss: BossIndex,
    /// How far the boss's points (its `move_to` targets) are from the definition's: a
    /// `time_limit` boss arrives 120 pixels above the main character; zero for the stage's own.
    pub origin: Point,
    /// A `time_limit` boss at a no-call time calls no elites (core#336).
    pub no_call: bool,
    /// The boss's kind (its sprite), an index into the run's `KindTable` made at spawn.
    pub kind: KindIndex,
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears.
    pub defence: u32,
    pub statuses: Statuses,
    /// The phase being carried out, 0-based (ADR-SIM-4 tier 2).
    pub phase: u32,
    /// Ticks since the phase began.
    pub phase_age: u32,
    /// Ticks left in which shots do the boss and its parts no harm.
    pub invulnerable: u32,
    /// The timeline step to carry out next, and the ticks to wait before it.
    pub step: u32,
    pub wait_left: u32,
    /// A `move_to` in progress.
    pub order: Option<MoveOrder>,
    /// One per rule of the phase, in order.
    pub rules: Rules,
    /// One per part of the stage's boss, in order.
    pub parts: Vec<PartState>,
    /// Whether it has called its elites; `None` for a boss that calls none.
    pub elites_called: Option<bool>,
    /// Ticks since it arrived: 1 on its first tick. The call of its elites counts them
    /// and so does the announcement of that call.
    pub age: u32,
}

impl Clone for BossState {
    fn clone(&self) -> BossState {
        let BossState {
            id,
            boss,
            origin,
            no_call,
            kind,
            at,
            hp,
            max_hp,
            defence,
            statuses,
            phase,
            phase_age,
            invulnerable,
            step,
            wait_left,
            order,
            rules,
            parts,
            elites_called,
            age,
        } = self;
        BossState {
            id: *id,
            boss: *boss,
            origin: *origin,
            no_call: *no_call,
            kind: *kind,
            at: *at,
            hp: *hp,
            max_hp: *max_hp,
            defence: *defence,
            statuses: *statuses,
            phase: *phase,
            phase_age: *phase_age,
            invulnerable: *invulnerable,
            step: *step,
            wait_left: *wait_left,
            order: *order,
            rules: *rules,
            parts: parts.clone(),
            elites_called: *elites_called,
            age: *age,
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &BossState) {
        let BossState {
            id,
            boss,
            origin,
            no_call,
            kind,
            at,
            hp,
            max_hp,
            defence,
            statuses,
            phase,
            phase_age,
            invulnerable,
            step,
            wait_left,
            order,
            rules,
            parts,
            elites_called,
            age,
        } = self;
        let BossState {
            id: s_id,
            boss: s_boss,
            origin: s_origin,
            no_call: s_no_call,
            kind: s_kind,
            at: s_at,
            hp: s_hp,
            max_hp: s_max_hp,
            defence: s_defence,
            statuses: s_statuses,
            phase: s_phase,
            phase_age: s_phase_age,
            invulnerable: s_invulnerable,
            step: s_step,
            wait_left: s_wait_left,
            order: s_order,
            rules: s_rules,
            parts: s_parts,
            elites_called: s_elites_called,
            age: s_age,
        } = source;
        *id = *s_id;
        *boss = *s_boss;
        *origin = *s_origin;
        *no_call = *s_no_call;
        *kind = *s_kind;
        *at = *s_at;
        *hp = *s_hp;
        *max_hp = *s_max_hp;
        *defence = *s_defence;
        *statuses = *s_statuses;
        *phase = *s_phase;
        *phase_age = *s_phase_age;
        *invulnerable = *s_invulnerable;
        *step = *s_step;
        *wait_left = *s_wait_left;
        *order = *s_order;
        *rules = *s_rules;
        parts.clone_from(s_parts);
        *elites_called = *s_elites_called;
        *age = *s_age;
    }
}

/// A breakable part of the boss, with its own hit points (ADR-CONT-7).
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct PartState {
    pub id: EntityId,
    /// The ID the stage gave it, which `part_broken` and the `part` selector name.
    pub name: ContentId,
    /// `name` as a kind, an index into the run's `KindTable` made at spawn.
    pub kind: KindIndex,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears.
    pub defence: u32,
    /// From the boss's centre, and the hurt circle's radius: both from the stage.
    pub offset: Point,
    pub radius: i32,
    pub statuses: Statuses,
}

/// Which definition an agent runs: its `Agent` in the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AgentKey {
    pub source: AgentSource,
    /// The companion index, or the wave index.
    pub index: u32,
    /// For summoned agents, the rule of the wave's enemy that summoned them.
    pub rule: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AgentSource {
    Companion,
    Wave,
    Summoned,
    /// Summoned by a rule of the boss: `index` is its phase, `rule` the rule in it.
    BossRule,
    /// Rung around a shrine: `index` is the shrine's wave.
    Guardian,
    /// Brought by a pack leader: `index` is the leader's wave.
    Escort,
}

/// A companion or an enemy: a movement and a rule list (ADR-SIM-4 tier 1).
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct AgentState {
    pub id: EntityId,
    /// Its definition in the run's `AgentDefs`.
    pub def: DefIndex,
    /// Its kind (its sprite), an index into the run's `KindTable` made at spawn; a hit copies it.
    pub kind: KindIndex,
    pub key: AgentKey,
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears.
    pub defence: u32,
    /// Ticks since it spawned.
    pub age: u32,
    /// One per rule of its agent, in order.
    pub rules: Rules,
    pub statuses: Statuses,
    /// Companions only; enemies stay at 0.
    pub aggro: u32,
    /// A `move_to` in progress, which replaces its movement.
    pub order: Option<MoveOrder>,
    /// For `orbit`, in 1/256 degree.
    pub orbit_angle: i32,
    /// Enemies only: the last push-apart found it overlapping its neighbours by more than a
    /// pixel, so it does not take its own step to its place this tick (ADR-SIM-5).
    pub crowded: bool,
    /// Enemies only: it removed itself with a `self_destruct` rule. It is at 0 hit points for
    /// the removal, but it was not defeated, so nothing credits it. It only lives until the end
    /// of the tick it is set in.
    pub burst: bool,
    /// Ticks left in which a new stun cannot land on this enemy, from the end of its last stun
    ///; 0 for none.
    pub stun_immune: u32,
    /// An elite's own state; `None` for every other agent.
    pub elite: Option<EliteState>,
    /// An agent that leaves the map unkilled (a goblin, or any elite whose `boss_call` is
    /// `leave`); `None` for every other agent.
    pub leave: Option<LeaveState>,
    /// A shrine's own state; `None` for every other agent. Boxed: almost no agent has one.
    pub shrine: Option<Box<ShrineState>>,
}

impl Clone for AgentState {
    fn clone(&self) -> AgentState {
        let AgentState {
            id,
            def,
            kind,
            key,
            at,
            hp,
            max_hp,
            defence,
            age,
            rules,
            statuses,
            aggro,
            order,
            orbit_angle,
            crowded,
            burst,
            stun_immune,
            elite,
            leave,
            shrine,
        } = self;
        AgentState {
            id: *id,
            def: *def,
            kind: *kind,
            key: *key,
            at: *at,
            hp: *hp,
            max_hp: *max_hp,
            defence: *defence,
            age: *age,
            rules: *rules,
            statuses: *statuses,
            aggro: *aggro,
            order: *order,
            orbit_angle: *orbit_angle,
            crowded: *crowded,
            burst: *burst,
            stun_immune: *stun_immune,
            elite: *elite,
            leave: *leave,
            shrine: shrine.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &AgentState) {
        let AgentState {
            id,
            def,
            kind,
            key,
            at,
            hp,
            max_hp,
            defence,
            age,
            rules,
            statuses,
            aggro,
            order,
            orbit_angle,
            crowded,
            burst,
            stun_immune,
            elite,
            leave,
            shrine,
        } = self;
        let AgentState {
            id: s_id,
            def: s_def,
            kind: s_kind,
            key: s_key,
            at: s_at,
            hp: s_hp,
            max_hp: s_max_hp,
            defence: s_defence,
            age: s_age,
            rules: s_rules,
            statuses: s_statuses,
            aggro: s_aggro,
            order: s_order,
            orbit_angle: s_orbit_angle,
            crowded: s_crowded,
            burst: s_burst,
            stun_immune: s_stun_immune,
            elite: s_elite,
            leave: s_leave,
            shrine: s_shrine,
        } = source;
        *id = *s_id;
        *def = *s_def;
        *kind = *s_kind;
        *key = *s_key;
        *at = *s_at;
        *hp = *s_hp;
        *max_hp = *s_max_hp;
        *defence = *s_defence;
        *age = *s_age;
        *rules = *s_rules;
        *statuses = *s_statuses;
        *aggro = *s_aggro;
        *order = *s_order;
        *orbit_angle = *s_orbit_angle;
        *crowded = *s_crowded;
        *burst = *s_burst;
        *stun_immune = *s_stun_immune;
        *elite = *s_elite;
        *leave = *s_leave;
        shrine.clone_from(s_shrine);
    }
}

impl AgentState {
    /// A mimic that has not revealed yet: an object nothing touches.
    pub(crate) fn hidden(&self) -> bool {
        self.elite.is_some_and(|el| el.disguised)
    }

    /// An elite enemy that shows as one: `EntityView::elite` and `DomainEvent::Died::elite`. A
    /// mimic in disguise does not, until it reveals; a shrine never does, its kind (`Shrine`)
    /// already says what it is (DS44 item 1). The elite machinery (cap, aura, guardians,
    /// `ShrineActivated`) still reads `elite` itself.
    pub(crate) fn shows_elite(&self) -> bool {
        self.elite.is_some() && !self.hidden() && self.shrine.is_none()
    }

    /// The class `EntityView::kind` and `DomainEvent::Died::class` give it: `listed`, the list it
    /// is in, except that a shrine is `Shrine` (SK1). The one place that decides it.
    pub(crate) fn entity_kind(&self, listed: EntityKind) -> EntityKind {
        if self.shrine.is_some() {
            EntityKind::Shrine
        } else {
            listed
        }
    }
}

/// A shrine's fight.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct ShrineState {
    /// Ticks since the main character touched it, 0 on the tick it did; `None` while untouched.
    pub fight: Option<u32>,
    /// The guardians that spawned, in ring order.
    pub guardians: Vec<EntityId>,
}

impl Clone for ShrineState {
    fn clone(&self) -> ShrineState {
        let ShrineState { fight, guardians } = self;
        ShrineState {
            fight: *fight,
            guardians: guardians.clone(),
        }
    }

    /// Copies `source` into `self`, keeping the buffers `self` already holds.
    fn clone_from(&mut self, source: &ShrineState) {
        let ShrineState { fight, guardians } = self;
        let ShrineState {
            fight: s_fight,
            guardians: s_guardians,
        } = source;
        *fight = *s_fight;
        guardians.clone_from(s_guardians);
    }
}

/// When an elite that leaves is to go.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct LeaveState {
    /// Ticks since it was disturbed, 0 on the tick it was; `None` while it is not.
    pub disturbed: Option<u32>,
    /// A boss came or called: it goes on this tick.
    pub go: bool,
}

/// An elite holds `home`, where it spawned, until a boss calls it.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct EliteState {
    pub home: Point,
    pub called: bool,
    /// What the boss's call does to it; its definition's, so not part of the hash.
    pub boss_call: BossCall,
    /// A mimic that has not revealed yet; false for every other elite.
    pub disguised: bool,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct MoveOrder {
    pub to: Point,
    pub ticks_left: u32,
}
