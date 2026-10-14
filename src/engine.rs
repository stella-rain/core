//! The simulation engine: runs a validated stage tick by tick to a clear or a fail
//! (ADR-004, ADR-015). The only external influence is one `Input` per tick; everything else
//! comes from the stage and its seed, or from a run setup (ADR-051), so the state hash after
//! each tick is the same on every platform (ADR-019).
//!
//! Rules are dispatched by the stage's `sim_version` (ADR-017). Only version 0 exists, and it
//! changes in place until the freeze (ADR-035): see `rules_v0` for what it does and does not
//! do yet.

use std::sync::Arc;

use crate::attack::{AttackPhase, Emitter};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::input::Input;
use crate::rng::SplitMix64;
use crate::rules_v0;
use crate::run_setup::{self, RunSetup};
use crate::snapshot::{
    AttackView, BulletView, ChargeView, EntityKind, EntityView, EvadeView, Facing, MapView,
    PlayerView, SkillView, Snapshot,
};
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

/// How much of each dynamic budget (ADR-020) a tick used: the most it reached during the tick.
/// A read-out for tests and benchmarks, not part of the state: nothing here is hashed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Load {
    /// Enemies alive when the attacks ran, after spawns and summons.
    pub enemies: u32,
    /// Attacks being carried out, counted before any of them finished.
    pub tasks: u32,
    /// Bullets alive after the attacks fired, before the tick moved and removed any.
    pub bullets: u32,
    /// Bullets made in the tick.
    pub bullet_spawns: u32,
    /// Emitter nodes carried out in the tick, across all attacks.
    pub emitter_steps: u32,
}

impl Load {
    /// The caps of the rules (`rules_v0`): a tick that reaches all of them is a worst-case tick.
    pub const CAPS: Load = Load {
        enemies: rules_v0::MAX_ENEMIES_ALIVE,
        tasks: rules_v0::MAX_ATTACK_TASKS,
        bullets: rules_v0::MAX_BULLETS_ALIVE,
        bullet_spawns: rules_v0::MAX_BULLET_SPAWNS_PER_TICK,
        emitter_steps: rules_v0::MAX_EMITTER_STEPS_PER_TICK,
    };
}

/// A run of one stage.
#[derive(Clone, Debug)]
pub struct Engine {
    stage: Stage,
    state: State,
    /// Events of the last tick; cleared when the next one starts.
    events: Vec<DomainEvent>,
    /// What the last tick used of the dynamic budgets; zero when the tick did not run.
    load: Load,
}

impl Engine {
    /// Starts a run. The stage is validated first (ADR-020), so the rules below only ever see
    /// stages inside the budgets.
    pub fn new(stage: &Stage) -> Result<Engine, Error> {
        validate::validate(stage)?;
        Ok(Engine {
            state: rules_v0::initial_state(stage, u64::from(stage.seed)),
            stage: stage.clone(),
            events: Vec::new(),
            load: Load::default(),
        })
    }

    /// Starts a run from a setup alone (ADR-051): the setup is validated, then its mode's stage
    /// runs with its weapons and seed.
    pub fn start(setup: &RunSetup) -> Result<Engine, Error> {
        let stage = run_setup::stage(setup)?;
        Ok(Engine {
            state: rules_v0::initial_state(&stage, setup.seed),
            stage,
            events: Vec::new(),
            load: Load::default(),
        })
    }

    /// Advances one tick with `input`. Once the run is over this does nothing, so a replay that
    /// is longer than the run cannot change the final hash.
    pub fn step(&mut self, input: Input) {
        self.events.clear();
        self.load = Load::default();
        match self.stage.sim_version {
            0 => rules_v0::step(
                &self.stage,
                &mut self.state,
                input,
                &mut self.events,
                &mut self.load,
            ),
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

    /// How much of each dynamic budget the last tick used (ADR-020).
    pub fn load(&self) -> Load {
        self.load
    }

    /// What the frontend draws after the last tick (ADR-005).
    pub fn snapshot(&self) -> Snapshot {
        let s = &self.state;
        let view = |r: &AttackRun| {
            let p = &r.phases[r.phase as usize];
            AttackView {
                slot: r.slot,
                preset: r.preset.clone(),
                phase: u8::try_from(r.phase).unwrap_or(u8::MAX),
                kind: p.kind,
                tick: r.tick,
                ticks: p.ticks,
                hit_box: p.hit_box.clone(),
                facing: Facing {
                    x: r.facing.x,
                    y: r.facing.y,
                },
            }
        };
        let attack_of = |id: EntityId| s.runs.iter().find(|r| r.owner == id).map(view);
        let mut entities = Vec::new();
        if let (Some(b), Some(boss)) = (&s.boss, &self.stage.boss) {
            if b.hp > 0 {
                entities.push(EntityView {
                    id: b.id,
                    kind: EntityKind::Boss,
                    asset: boss.base.clone(),
                    palette: boss.palette.clone(),
                    at: b.at,
                    radius: boss.radius,
                    hp: b.hp,
                    attack: attack_of(b.id),
                });
            }
        }
        // Boss, companions, boss parts, then enemies: the order of their entity IDs.
        let agent_views =
            |kind, agents: &[crate::engine::AgentState], out: &mut Vec<EntityView>| {
                for a in agents {
                    let def = rules_v0::agent_def(&self.stage, a.key);
                    out.push(EntityView {
                        id: a.id,
                        kind,
                        asset: ContentId(def.base.0.clone()),
                        palette: def.palette.clone(),
                        at: a.at,
                        radius: def.radius,
                        hp: a.hp,
                        attack: attack_of(a.id),
                    });
                }
            };
        agent_views(EntityKind::Companion, &s.companions, &mut entities);
        if let (Some(b), Some(boss)) = (&s.boss, &self.stage.boss) {
            for (part, def) in b.parts.iter().zip(&boss.parts) {
                if part.hp > 0 {
                    entities.push(EntityView {
                        id: part.id,
                        kind: EntityKind::BossPart,
                        asset: ContentId(def.asset.0.clone()),
                        palette: None,
                        at: Point {
                            x: b.at.x.saturating_add(part.offset.x),
                            y: b.at.y.saturating_add(part.offset.y),
                        },
                        radius: Fx(part.radius),
                        hp: part.hp,
                        attack: None,
                    });
                }
            }
        }
        agent_views(EntityKind::Enemy, &s.enemies, &mut entities);
        let map = self.stage.map();
        Snapshot {
            tick: s.tick,
            map: MapView {
                width: map.width,
                height: map.height,
            },
            player: PlayerView {
                at: s.player.at,
                hp: s.player.hp,
                radius: Fx(rules_v0::PLAYER_RADIUS),
                facing: Facing {
                    x: s.player.facing.x,
                    y: s.player.facing.y,
                },
                invulnerable: s.player.invulnerable,
                evade: EvadeView {
                    ready: s.player.evade_ready(),
                    cooldown_left: s.player.evade_cooldown,
                    dash_left: s.player.dash.map_or(0, |d| d.left),
                },
                charge: s.player.charge.and_then(|held| {
                    let (level, perfect) = rules_v0::charge_view(&self.stage, held)?;
                    Some(ChargeView {
                        level,
                        held,
                        perfect,
                    })
                }),
                main_skill: SkillView {
                    ready: rules_v0::charge_ready(s),
                    cooldown_left: s.player.charge_cooldown,
                },
                sub_skill: SkillView {
                    ready: rules_v0::skill_ready(s),
                    cooldown_left: s.player.skill_cooldown,
                },
                attacks: {
                    let mut runs: Vec<&AttackRun> = s
                        .runs
                        .iter()
                        .filter(|r| r.owner == rules_v0::PLAYER_ID)
                        .collect();
                    runs.sort_by_key(|r| r.slot);
                    runs.into_iter().map(view).collect()
                },
            },
            entities,
            bullets: s
                .bullets
                .iter()
                .map(|b| BulletView {
                    at: b.at,
                    vx: b.vx,
                    vy: b.vy,
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
    pub player: PlayerState,
    pub boss: Option<BossState>,
    /// The allies that act on their own, in entity-ID order.
    pub companions: Vec<AgentState>,
    /// In spawn order, which is entity-ID order.
    pub enemies: Vec<AgentState>,
    /// Attacks that are being carried out, oldest first (ADR-036 tier 3).
    pub tasks: Vec<AttackTask>,
    /// Phased attacks in progress, in the order they started (ADR-055).
    pub runs: Vec<AttackRun>,
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
    /// Copied from the stage when it appears (ADR-056).
    pub defence: u32,
    /// Ticks of invulnerability left, after a hit or from an evade.
    pub invulnerable: u32,
    /// The last non-zero movement direction (ADR-055): where a weapon aimed along the facing
    /// attacks and where an evade without movement dashes.
    pub facing: Dir,
    /// Ticks until the Main weapon may attack again, counted from its last attack's start.
    pub main_cooldown: u32,
    /// The same for the Sub weapon.
    pub sub_cooldown: u32,
    /// Ticks until the evade is ready again, counted from its press.
    pub evade_cooldown: u32,
    /// The charge in progress (skill 2): ticks held since the press, 0 on the press's tick.
    pub charge: Option<u32>,
    /// Skill 2 was held on the last tick, so that only a press starts a charge.
    pub charge_held: bool,
    /// Ticks until the charged skill is ready again, counted from its release.
    pub charge_cooldown: u32,
    /// Ticks until the Sub skill is ready again, counted from its start.
    pub skill_cooldown: u32,
    /// The dash in progress, if any.
    pub dash: Option<DashState>,
    pub aggro: u32,
    pub statuses: Vec<StatusState>,
}

impl PlayerState {
    pub fn evade_ready(&self) -> bool {
        self.evade_cooldown == 0 && self.dash.is_none()
    }
}

/// An evade's dash: locked to its direction, `done` of its ticks moved, `left` to go.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DashState {
    pub dir: Dir,
    pub done: u32,
    pub left: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct BossState {
    pub id: EntityId,
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears (ADR-056).
    pub defence: u32,
    pub statuses: Vec<StatusState>,
    /// The phase being carried out, 0-based (ADR-036 tier 2).
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
    pub rules: Vec<RuleState>,
    /// One per part of the stage's boss, in order.
    pub parts: Vec<PartState>,
}

/// A breakable part of the boss, with its own hit points (ADR-007).
#[derive(Clone, Debug)]
pub(crate) struct PartState {
    pub id: EntityId,
    /// The ID the stage gave it, which `part_broken` and the `part` selector name.
    pub name: ContentId,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears (ADR-056).
    pub defence: u32,
    /// From the boss's centre, and the hurt circle's radius: both from the stage.
    pub offset: Point,
    pub radius: i32,
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
    /// Summoned by a rule of the boss: `index` is its phase, `rule` the rule in it.
    BossRule,
}

/// A companion or an enemy: a movement and a rule list (ADR-036 tier 1).
#[derive(Clone, Debug)]
pub(crate) struct AgentState {
    pub id: EntityId,
    pub key: AgentKey,
    pub at: Point,
    pub hp: u32,
    pub max_hp: u32,
    /// Copied from the stage when it appears (ADR-056).
    pub defence: u32,
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
    /// Enemies only: the last push-apart found it overlapping its neighbours by more than a
    /// pixel, so it does not take its own step to its place this tick (ADR-058).
    pub crowded: bool,
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
    pub const UP: Dir = Dir {
        x: 0,
        y: -crate::trig::SCALE,
    };
}

/// What an attack's `aimed` directions point at, and which way it faces. This is where the
/// ways of aiming meet: enemies, companions and the boss aim at an entity; the main
/// character's weapons aim along its facing or at the target their targeting picked. A new way
/// of aiming (a homing shot the main character acquires) is a new variant, read in
/// `attacks::aim`, hashed in `hash_aim` and given its heading here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aim {
    /// At the entity with this ID; straight down if it is gone.
    Entity(EntityId),
    /// Along a fixed direction.
    Fixed(Dir),
    /// At a weapon's target (core#83); when it is gone, along `last`, the last direction taken
    /// to it. `heading` is the direction to it when the attack started.
    Target {
        id: EntityId,
        heading: Dir,
        last: Dir,
    },
}

impl Aim {
    /// The way the attack faces when it starts: where `relative` and `sequential` directions
    /// begin. An attack at an entity faces down the screen, as the enemies do.
    pub fn heading(self) -> Dir {
        match self {
            Aim::Entity(_) => Dir::DOWN,
            Aim::Fixed(d) | Aim::Target { heading: d, .. } => d,
        }
    }
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
    /// Its hit breaks the main character's charge (core#83).
    pub breaks_charge: bool,
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
    pub aim: Aim,
    pub friendly: bool,
    pub damage: u32,
    pub breaks_charge: bool,
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

/// A phased attack an attacker is carrying out (ADR-055): one at a time per attacker and slot.
/// Its phases spawn their nodes as `AttackTask`s, which run to their own end.
#[derive(Clone, Debug)]
pub(crate) struct AttackRun {
    pub owner: EntityId,
    /// 0 for an agent's or the boss's attacks and the main character's Main weapon; 1 for its
    /// Sub weapon, 2 for its charged skill and 3 for its Sub skill (core#83).
    pub slot: u8,
    /// The preset it came from, for the frontend to pick its animation.
    pub preset: Option<ContentId>,
    pub phases: Arc<Vec<AttackPhase>>,
    pub phase: u32,
    /// Ticks done in the phase: 0 on its first tick.
    pub tick: u32,
    /// Locked when the attack starts.
    pub facing: Dir,
    pub aim: Aim,
    pub friendly: bool,
    /// The damage its starter gives a phase that sets none (ADR-055): the main character's
    /// attack, a shot skill's own damage, 1 for an enemy or the boss.
    pub base: u32,
    /// The attacker's atk-up, in percent, fixed when the attack starts (ADR-056).
    pub atk_up_pct: u32,
    /// `phase_time` of the attack's first tick, for the nodes its phases spawn.
    pub age: u32,
    /// Who the current active phase has hit, and on which tick last, sorted by entity ID;
    /// emptied when an active phase begins.
    pub hits: Vec<(EntityId, u32)>,
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
    /// What `aimed` directions point at.
    pub aim: Aim,
    pub friendly: bool,
    pub damage: u32,
    /// The bullets it fires break the main character's charge (core#83).
    pub breaks_charge: bool,
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
