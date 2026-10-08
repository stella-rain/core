//! The simulation engine: runs a validated stage tick by tick to a clear or a fail
//! (ADR-004, ADR-015). The only external influence is one `Input` per tick; everything else
//! comes from the stage and its seed, so the state hash after each tick is the same on every
//! platform (ADR-019).
//!
//! Rules are dispatched by the stage's `sim_version` (ADR-017). Only version 0 exists, and it
//! changes in place until the freeze (ADR-035): see `rules_v0` for what it does and does not
//! do yet.

use std::sync::Arc;

use crate::attack::Emitter;
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::input::Input;
use crate::rng::SplitMix64;
use crate::rules_v0;
use crate::snapshot::{BulletView, EntityKind, EntityView, PlayerView, SkillView, Snapshot};
use crate::stage::Stage;
use crate::validate::{self, Error};

/// How a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Running,
    /// The stage was cleared: this is what a replay has to prove (ADR-009).
    Cleared,
    /// The main character died, or the stage ran out of ticks.
    Failed,
}

/// A run of one stage.
#[derive(Clone, Debug)]
pub struct Engine {
    stage: Stage,
    state: State,
    /// Events of the last tick; cleared when the next one starts.
    events: Vec<DomainEvent>,
}

impl Engine {
    /// Starts a run. The stage is validated first (ADR-020), so the rules below only ever see
    /// stages inside the budgets.
    pub fn new(stage: &Stage) -> Result<Engine, Error> {
        validate::validate(stage)?;
        Ok(Engine {
            state: rules_v0::initial_state(stage),
            stage: stage.clone(),
            events: Vec::new(),
        })
    }

    /// Advances one tick with `input`. Once the run is over this does nothing, so a replay that
    /// is longer than the run cannot change the final hash.
    pub fn step(&mut self, input: Input) {
        self.events.clear();
        match self.stage.sim_version {
            0 => rules_v0::step(&self.stage, &mut self.state, input, &mut self.events),
            // `new` validated the version; fail closed if that ever stops being true.
            _ => self.state.outcome = Outcome::Failed,
        }
    }

    pub fn tick(&self) -> u32 {
        self.state.tick
    }

    pub fn outcome(&self) -> Outcome {
        self.state.outcome
    }

    /// FNV-1a 64 over the whole state after the last tick (ADR-019).
    pub fn state_hash(&self) -> u64 {
        rules_v0::state_hash(&self.state)
    }

    /// The domain events of the last tick, in the order they happened.
    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    /// What the frontend draws after the last tick (ADR-005).
    pub fn snapshot(&self) -> Snapshot {
        let s = &self.state;
        let mut entities = Vec::new();
        if let (Some(b), Some(boss)) = (&s.boss, &self.stage.boss) {
            if b.hp > 0 {
                entities.push(EntityView {
                    id: b.id,
                    kind: EntityKind::Boss,
                    asset: boss.base.clone(),
                    palette: boss.palette.clone(),
                    at: b.at,
                    hp: b.hp,
                });
            }
        }
        // Boss, companions, then enemies: the order of their entity IDs.
        for (kind, agents) in [
            (EntityKind::Companion, &s.companions),
            (EntityKind::Enemy, &s.enemies),
        ] {
            for a in agents {
                let def = rules_v0::agent_def(&self.stage, a.key);
                entities.push(EntityView {
                    id: a.id,
                    kind,
                    asset: ContentId(def.base.0.clone()),
                    palette: def.palette.clone(),
                    at: a.at,
                    hp: a.hp,
                });
            }
        }
        Snapshot {
            tick: s.tick,
            player: PlayerView {
                at: s.player.at,
                hp: s.player.hp,
                focus: s.input.focus,
                firing: s.input.fire,
                held: s.input.held,
                skills: self
                    .stage
                    .player
                    .skills
                    .iter()
                    .zip(&s.player.skills)
                    .map(|(def, state)| SkillView {
                        slot: def.slot,
                        ready: state.is_ready(),
                        cooldown_left: state.cooldown_left,
                        charges_left: (state.charges_left != UNLIMITED)
                            .then(|| u16::try_from(state.charges_left).unwrap_or(u16::MAX)),
                    })
                    .collect(),
            },
            entities,
            bullets: s
                .bullets
                .iter()
                .map(|b| BulletView {
                    at: b.at,
                    radius: Fx(rules_v0::bullet_radius(b.friendly)),
                    style: b.style,
                    friendly: b.friendly,
                })
                .collect(),
        }
    }
}

/// Firings or casts left when there is no limit.
pub(crate) const UNLIMITED: u32 = u32::MAX;

/// The whole state of a run. Everything here is in the state hash (`rules_v0::state_hash`),
/// at fixed widths and in this order.
#[derive(Clone, Debug)]
pub(crate) struct State {
    pub tick: u32,
    pub rng: SplitMix64,
    pub outcome: Outcome,
    /// The input of the last tick, for the snapshot.
    pub input: Input,
    pub player: PlayerState,
    pub boss: Option<BossState>,
    /// The allies that act on their own, in entity-ID order.
    pub companions: Vec<AgentState>,
    /// In spawn order, which is entity-ID order.
    pub enemies: Vec<AgentState>,
    /// Attacks that are being carried out, oldest first (ADR-036 tier 3).
    pub tasks: Vec<AttackTask>,
    /// In spawn order.
    pub bullets: Vec<BulletState>,
    /// Enemies spawned so far, per wave.
    pub spawned: Vec<u16>,
    pub next_id: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct PlayerState {
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Ticks of invulnerability left after a hit.
    pub invulnerable: u32,
    /// Ticks until the main shot may fire again.
    pub shot_cooldown: u32,
    /// Ticks in a row the input has had `fire`.
    pub firing_streak: u32,
    pub aggro: u32,
    pub statuses: Vec<StatusState>,
    /// One per skill slot of the stage, in order.
    pub skills: Vec<SkillState>,
    /// The slot being held with a ready skill, if any (ADR-032).
    pub hold: Option<HoldState>,
    /// The `held` field of the last tick's input, to see presses and releases.
    pub held_before: u8,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SkillState {
    pub cooldown_left: u32,
    /// Casts left; `UNLIMITED` if there is no limit.
    pub charges_left: u32,
}

impl SkillState {
    pub fn is_ready(&self) -> bool {
        self.cooldown_left == 0 && self.charges_left != 0
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HoldState {
    /// Index into the stage's skill slots.
    pub index: u32,
    /// Ticks held so far.
    pub ticks: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct BossState {
    pub id: EntityId,
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    pub statuses: Vec<StatusState>,
}

/// Which definition an agent runs: its `Agent` in the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AgentKey {
    pub source: AgentSource,
    /// The companion index, or the wave index.
    pub index: u32,
    /// For summoned agents, the rule of the wave's enemy that summoned them.
    pub rule: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentSource {
    Companion,
    Wave,
    Summoned,
}

/// A companion or an enemy: a movement and a rule list (ADR-036 tier 1).
#[derive(Clone, Debug)]
pub(crate) struct AgentState {
    pub id: EntityId,
    pub key: AgentKey,
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Ticks since it spawned.
    pub age: u32,
    /// One per rule of its agent, in order.
    pub rules: Vec<RuleState>,
    pub statuses: Vec<StatusState>,
    /// Companions only; enemies stay at 0.
    pub aggro: u32,
    /// A `move_to` in progress, which replaces its movement.
    pub order: Option<MoveOrder>,
    /// For `orbit`, in 1/256 degree.
    pub orbit_angle: i32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MoveOrder {
    pub to: Point,
    pub ticks_left: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RuleState {
    pub cooldown_left: u32,
    /// Firings left; `UNLIMITED` if there is no limit.
    pub charges_left: u32,
    pub fired: bool,
}

/// The statuses a skill can apply in this version (ADR-036). The IDs in the registry are the
/// names below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusKind {
    /// More damage dealt.
    AtkUp,
    /// More damage taken.
    Vulnerable,
    /// Slower movement.
    Slow,
}

impl StatusKind {
    pub fn name(self) -> &'static str {
        match self {
            StatusKind::AtkUp => "atk_up",
            StatusKind::Vulnerable => "vulnerable",
            StatusKind::Slow => "slow",
        }
    }
}

/// One status on one entity. Applying it again refreshes the duration and adds a stack.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StatusState {
    pub kind: StatusKind,
    /// Who applied it last.
    pub source: EntityId,
    pub remaining: u32,
    pub stacks: u16,
    /// Percent per stack.
    pub value_pct: u16,
}

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
}

#[derive(Clone, Debug)]
pub(crate) struct BulletState {
    pub at: Point,
    pub vx: Fx,
    pub vy: Fx,
    /// The direction it was fired in, which `relative` directions of its children start from.
    pub dir: Dir,
    /// For the renderer; 0 is the default bullet.
    pub style: u16,
    /// Who fired it, for the hit event and for aggro.
    pub owner: EntityId,
    /// Fired by an ally rather than an enemy.
    pub friendly: bool,
    pub damage: u32,
    /// Starts an attack of its own after a while (`on_bullet`).
    pub hook: Option<Box<Hook>>,
}

/// An `on_bullet` waiting on a bullet.
#[derive(Clone, Debug)]
pub(crate) struct Hook {
    pub program: Arc<Vec<Emitter>>,
    /// The `on_bullet` node in the program: its `then` list is what the bullet starts.
    pub path: Vec<PathStep>,
    /// Ticks left before it starts.
    pub left: u32,
    pub owner: EntityId,
    pub target: EntityId,
    pub friendly: bool,
    pub damage: u32,
}

/// One step down a program: the node at `node` in the current list, then its `body` list, or
/// its `then` list if `then` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PathStep {
    pub node: u16,
    pub then: bool,
}

/// Where an attack's bullets start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// Wherever its owner is, for as long as the owner lives.
    Owner,
    /// A fixed point: where a bullet was when it started this attack.
    At(Point),
}

/// A place in the middle of a list of nodes, and the state of the loop or turn whose body the
/// next frame up is running.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Frame {
    /// The node of this frame's list that is being carried out.
    pub idx: u16,
    pub loop_i: u32,
    pub loop_n: u32,
    /// A spread's angle.
    pub param: i32,
    /// The turn this node adds to the directions of its body.
    pub offset: i32,
    /// For `on_bullet`: ticks the bullets of its body wait before starting their attack.
    pub hook_after: u32,
}

/// An attack being carried out (ADR-036 tier 3): a program, where it has got to, and what it
/// needs to know to aim and to hurt. Presets are expanded before they get here, so a preset and
/// its inline copy are the same task.
#[derive(Clone, Debug)]
pub(crate) struct AttackTask {
    pub owner: EntityId,
    pub program: Arc<Vec<Emitter>>,
    /// Where in the program this task starts; empty for the whole program.
    pub base: Vec<PathStep>,
    pub origin: Origin,
    /// What `aimed` directions point at, by entity ID.
    pub target: EntityId,
    pub friendly: bool,
    pub damage: u32,
    /// Ticks since it started.
    pub age: u32,
    /// Ticks to wait before going on.
    pub wait_left: u32,
    /// The stack of lists being carried out, outermost first.
    pub frames: Vec<Frame>,
    /// The direction `relative` directions start from.
    pub parent_dir: Dir,
    /// The direction of the last bullet, which `sequential` directions turn from.
    pub last_dir: Dir,
}
