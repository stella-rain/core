//! The simulation engine: runs a validated stage tick by tick to a clear or a fail
//! (ADR-004, ADR-015). The only external influence is one `Input` per tick and the commands
//! between ticks (ADR-066: `choose` and `spend`); everything else comes from the stage and its
//! seed, or from a run setup, and from the run stats (ADR-067), so the state hash after each
//! tick is the same on every platform (ADR-019).
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
use crate::rewards::{Rarity, Rewards};
use crate::rng::SplitMix64;
use crate::rules_v0;
use crate::run_content::RunContent;
use crate::run_setup::{self, RunSetup};
use crate::run_stats::{RunStats, StatEffect};
use crate::snapshot::{
    AttackView, BulletView, ChargeView, ComboView, EntityKind, EntityView, EvadeView, Facing,
    MapView, OfferKind, PlayerView, ProgressView, Snapshot, StarView,
};
use crate::stage::Stage;
use crate::validate::{self, Error, Issue};

/// Why `Engine::choose` refused a choice; the run is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceError {
    /// No offer is open.
    NoOffer,
    /// `index` is not below the number of the open offer's `options`.
    OutOfRange { index: u32, options: u32 },
}

/// Why `Engine::spend` refused to spend a stat point; the run is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendError {
    /// The run has ended: its unspent points are lost.
    RunOver,
    /// An offer is open: it is answered first (ADR-066 §4).
    OfferOpen,
    /// No point is left (or the stage has no levels).
    NoPoint,
    /// `index` is not below the number of run `stats`.
    OutOfRange { index: u32, stats: u32 },
    /// The stat at `index` already has its `limit` of points.
    AtLimit { index: u32, limit: u8 },
}

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
    /// The content files the run reads (ADR-067), fixed for the run.
    content: RunContent,
    state: State,
    /// Events of the last tick; cleared when the next one starts.
    events: Vec<DomainEvent>,
    /// What the last tick used of the dynamic budgets; zero when the tick did not run.
    load: Load,
}

impl Engine {
    /// Starts a run. The stage is validated first (ADR-020), so the rules below only ever see
    /// stages inside the budgets.
    /// A stage with levels spends its points on the bundled run stats.
    pub fn new(stage: &Stage) -> Result<Engine, Error> {
        Engine::new_with(stage, RunContent::bundled())
    }

    /// As `new`, with `content` in place of the bundled files (ADR-067).
    pub fn new_with(stage: &Stage, content: &RunContent) -> Result<Engine, Error> {
        validate::validate(stage)?;
        let pool = rules_v0::RunPool {
            rewards: pool(None, content.rewards())?,
            extra_rerolls: 0,
        };
        Ok(Engine {
            state: rules_v0::initial_state(stage, content, pool, u64::from(stage.seed)),
            stage: stage.clone(),
            content: content.clone(),
            events: Vec::new(),
            load: Load::default(),
        })
    }

    /// Starts a run from a setup alone (ADR-066 §1): the setup is validated, then its mode's
    /// stage runs with its weapons and seed, and the bundled content files.
    pub fn start(setup: &RunSetup) -> Result<Engine, Error> {
        Engine::start_with(setup, RunContent::bundled())
    }

    /// As `start`, with `content` in place of the bundled files (ADR-067).
    pub fn start_with(setup: &RunSetup, content: &RunContent) -> Result<Engine, Error> {
        let stage = run_setup::stage(setup)?;
        let pool = rules_v0::RunPool {
            rewards: pool(setup.star_pool.as_deref(), content.rewards())?,
            extra_rerolls: setup.extra_rerolls,
        };
        Ok(Engine {
            state: rules_v0::initial_state(&stage, content, pool, setup.seed),
            stage,
            content: content.clone(),
            events: Vec::new(),
            load: Load::default(),
        })
    }

    /// Advances one tick with `input`. Once the run is over this does nothing, so a replay that
    /// is longer than the run cannot change the final hash; nor while an offer is open, when
    /// the world is stopped (ADR-066 §3).
    pub fn step(&mut self, input: Input) {
        self.events.clear();
        self.load = Load::default();
        if self.offer_open() {
            return;
        }
        match self.stage.sim_version {
            0 => rules_v0::step(
                &self.stage,
                self.content.rewards(),
                &mut self.state,
                input,
                &mut self.events,
                &mut self.load,
            ),
            // `new` validated the version; fail closed if that ever stops being true.
            _ => self.state.outcome = Outcome::Failed,
        }
    }

    /// Chooses option `index` of the open offer, between ticks (ADR-066 §3): the snapshot's
    /// `offer.options` lists them. It applies at once, and the events are then this choice's.
    /// Refused, changing nothing, with no offer open or an index outside its options.
    pub fn choose(&mut self, index: u32) -> Result<(), ChoiceError> {
        if !self.offer_open() {
            return Err(ChoiceError::NoOffer);
        }
        let rewards = self.content.rewards();
        let options = rules_v0::offer_options(rewards, &self.state);
        let Some(option) = usize::try_from(index)
            .ok()
            .and_then(|i| options.get(i))
            .copied()
        else {
            return Err(ChoiceError::OutOfRange {
                index,
                options: u32::try_from(options.len()).unwrap_or(u32::MAX),
            });
        };
        self.events.clear();
        match self.stage.sim_version {
            0 => rules_v0::choose(rewards, &mut self.state, option, &mut self.events),
            _ => self.state.outcome = Outcome::Failed,
        }
        Ok(())
    }

    /// Spends a stat point on the run stat at `index`, between ticks (ADR-066 §4): the stat
    /// rises by its step at once, and the events are then this spend's. Refused, changing
    /// nothing, once the run has ended, while an offer is open, with no point left, an index
    /// outside the run stats, or the stat at its limit.
    pub fn spend(&mut self, index: u32) -> Result<(), SpendError> {
        if self.state.outcome != Outcome::Running {
            return Err(SpendError::RunOver);
        }
        if self.offer_open() {
            return Err(SpendError::OfferOpen);
        }
        let p = self
            .state
            .progress
            .as_ref()
            .filter(|p| p.points > 0)
            .ok_or(SpendError::NoPoint)?;
        let stats = self.content.run_stats().stats();
        let Some(at) = usize::try_from(index).ok().filter(|i| *i < stats.len()) else {
            return Err(SpendError::OutOfRange {
                index,
                stats: u32::try_from(stats.len()).unwrap_or(u32::MAX),
            });
        };
        let stat = &stats[at];
        if p.spent[at] >= stat.limit {
            return Err(SpendError::AtLimit {
                index,
                limit: stat.limit,
            });
        }
        self.events.clear();
        match self.stage.sim_version {
            0 => rules_v0::spend(&mut self.state, at, stat, &mut self.events),
            _ => self.state.outcome = Outcome::Failed,
        }
        Ok(())
    }

    /// The run stats this run spends its points on.
    pub fn run_stats(&self) -> &RunStats {
        self.content.run_stats()
    }

    /// The content files this run reads.
    pub fn content(&self) -> &RunContent {
        &self.content
    }

    pub fn tick(&self) -> u32 {
        self.state.tick
    }

    pub fn outcome(&self) -> Outcome {
        self.state.outcome
    }

    /// An offer is open: `step` does nothing until `choose` closes it (ADR-066 §3).
    pub fn offer_open(&self) -> bool {
        self.state.stars.as_ref().is_some_and(|f| f.offer.is_some())
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
                    elite: false,
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
                        elite: a.elite.is_some(),
                    });
                }
            };
        agent_views(EntityKind::Companion, &s.companions, &mut entities);
        // A part is drawn while it and its boss are alive: a dead boss takes its unbroken
        // parts with it (core#60).
        if let (Some(b), Some(boss)) = (&s.boss, &self.stage.boss) {
            for (part, def) in b.parts.iter().zip(&boss.parts) {
                if b.hp > 0 && part.hp > 0 {
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
                        elite: false,
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
                combo: {
                    let (presses, dash, buffered, window_left) = rules_v0::combo_view(s);
                    ComboView {
                        presses,
                        dash,
                        buffered,
                        window_left,
                    }
                },
                charge: rules_v0::charge_view(&self.stage, s).map(|(level, held, perfect)| {
                    ChargeView {
                        level,
                        held,
                        perfect,
                    }
                }),
                target: None,
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
            stars: s.stars.as_ref().map_or_else(Vec::new, |f| {
                f.on_map
                    .iter()
                    .map(|st| StarView {
                        id: st.id,
                        at: st.at,
                        rarity: st.rarity,
                    })
                    .collect()
            }),
            progress: s.progress.as_ref().map(|p| ProgressView {
                level: p.level,
                xp: p.xp,
                xp_next: rules_v0::xp_next(&self.stage, p.level),
                points: p.points,
                spent: p.spent.clone(),
                offer: rules_v0::offer_view(self.content.rewards(), s),
            }),
        }
    }
}

/// The run's star pool: the places in `rewards` of the rewards `ids` names, or of every reward
/// when it names none. A name the file does not hold fails closed, one issue per name.
fn pool(ids: Option<&[ContentId]>, rewards: &Rewards) -> Result<Vec<u16>, Error> {
    let place = |i: usize| u16::try_from(i).unwrap_or(u16::MAX);
    let Some(ids) = ids else {
        return Ok((0..rewards.rewards().len()).map(place).collect());
    };
    let mut issues = Vec::new();
    let mut out = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        match rewards.index_of(id) {
            Some(at) => out.push(place(at)),
            None => issues.push(Issue {
                path: format!("$.star_pool[{i}]"),
                message: "no such reward in the run's rewards file".into(),
            }),
        }
    }
    if issues.is_empty() {
        Ok(out)
    } else {
        Err(Error::Invalid(issues))
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
    /// Experience and levels; `None` in a stage without levels, which hashes nothing for it.
    pub progress: Option<Progress>,
    /// Falling stars and offers; `None` in a stage without stars, which hashes nothing for it.
    pub stars: Option<StarField>,
}

/// Falling stars, the offers due and the open one, and what the run's offers have used
/// (ADR-066 §2, §3).
#[derive(Clone, Debug)]
pub(crate) struct StarField {
    /// Kills in a row that dropped no star, for the pity.
    pub misses: u32,
    /// The stars waiting on the map, in the order they dropped (entity-ID order).
    pub on_map: Vec<StarState>,
    /// The offers due that have not opened yet: a boss kill's first, then the stars taken, in
    /// the order taken.
    pub due: Vec<Due>,
    pub offer: Option<OfferState>,
    pub rerolls_left: u32,
    pub banishes_left: u32,
    /// The run's pool: places in the rewards file, in its order, fixed for the run.
    pub pool: Vec<u16>,
    /// For each reward of the file: times taken, and whether it is banished.
    pub taken: Vec<u8>,
    pub banished: Vec<bool>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StarState {
    pub id: EntityId,
    pub at: Point,
    pub rarity: Rarity,
}

/// An offer due: what opens it, and the highest rarity a candidate may have.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Due {
    pub kind: OfferKind,
    pub rarity: Rarity,
}

#[derive(Clone, Debug)]
pub(crate) struct OfferState {
    pub kind: OfferKind,
    /// The highest rarity a candidate may have: the star's, or the top of the boss offer's tier.
    pub rarity: Rarity,
    /// Places in the rewards file, in the offer's order.
    pub candidates: Vec<u16>,
}

/// Experience, levels and stat points (ADR-066 §2, §4).
#[derive(Clone, Debug)]
pub(crate) struct Progress {
    /// From 1.
    pub level: u32,
    /// Experience toward the next level.
    pub xp: u32,
    /// Stat points not spent yet: one per level-up.
    pub points: u32,
    /// Points spent on each run stat, in the run stats' order.
    pub spent: Vec<u8>,
    /// What the points spent add up to for each effect, in `StatEffect::ALL` order.
    pub totals: [u32; StatEffect::ALL.len()],
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
    /// The last non-zero movement direction (ADR-069): where a combo step attacks and where an
    /// evade without movement dashes.
    pub facing: Dir,
    /// Ticks until the evade is ready again, counted from its press.
    pub evade_cooldown: u32,
    /// The buttons held on the last tick (`input::held_buttons`), so that only a button held now
    /// and not then is a press (ADR-071).
    pub held: u8,
    /// Where it is in its Main weapon's combos (ADR-070).
    pub combo: ComboState,
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

/// Where the main character is in its Main weapon's combo tree (ADR-070, ADR-076).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ComboState {
    /// The presses from the root to the current step, the first in bit 0: 1 for `sub`.
    pub path: u32,
    /// How many presses `path` holds: 0 at the root.
    pub depth: u8,
    /// The path began in the dash branch.
    pub dash: bool,
    /// The press waiting for the next step: `Some(true)` for `sub`.
    pub buffered: Option<bool>,
    /// The current step's attack is running.
    pub running: bool,
    /// Ticks left in which a press continues the combo, once a step has run out.
    pub window_left: u32,
    /// Ticks left in which a press from the root takes the dash branch.
    pub dash_strike_left: u32,
    /// The current step charging: ticks held, 0 on the tick it began.
    pub charge: Option<u32>,
}

/// The Sub weapon's pull on what a combo part hits (ADR-070), carried by the attack and the
/// bullets it spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pull {
    pub distance: i32,
    pub ticks: u32,
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
    /// Whether it has called its elites; `None` for a boss that calls none, which hashes
    /// nothing for it.
    pub elites_called: Option<bool>,
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
    /// An elite's own state; `None` for every other agent, which hashes nothing for it.
    pub elite: Option<EliteState>,
}

/// An elite holds `home`, where it spawned, until a boss calls it (ADR-068).
#[derive(Clone, Copy, Debug)]
pub(crate) struct EliteState {
    pub home: Point,
    pub called: bool,
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
/// character's combo steps aim along its facing (ADR-069). A new way of aiming (an effect's
/// target, ADR-069) is a new variant, read in `attacks::aim`, hashed in `hash_aim` and given its
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
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub pull: Option<Pull>,
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
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub pull: Option<Pull>,
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
    /// 0 for an agent's or the boss's attacks and the main character's combo (ADR-069).
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
    /// The Sub weapon's pull, when the attack is a combo part (ADR-070).
    pub pull: Option<Pull>,
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
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub pull: Option<Pull>,
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
