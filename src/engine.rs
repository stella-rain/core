//! The simulation engine: runs a validated stage tick by tick to a clear or a fail
//! (ADR-004, ADR-015). The only external influence is one `Input` per tick; everything else
//! comes from the stage and its seed, so the state hash after each tick is the same on every
//! platform (ADR-019).
//!
//! Rules are dispatched by the stage's `sim_version` (ADR-017). Only version 0 exists, and it
//! changes in place until the freeze (ADR-035): see `rules_v0` for what it does and does not
//! do yet.

use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::input::Input;
use crate::rng::SplitMix64;
use crate::rules_v0;
use crate::snapshot::{BulletView, EntityKind, EntityView, PlayerView, Snapshot};
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
        for e in &s.enemies {
            let agent = &self.stage.waves[e.wave as usize].enemy;
            entities.push(EntityView {
                id: e.id,
                kind: EntityKind::Enemy,
                asset: ContentId(agent.base.0.clone()),
                palette: agent.palette.clone(),
                at: e.at,
                hp: e.hp,
            });
        }
        Snapshot {
            tick: s.tick,
            player: PlayerView {
                at: s.player.at,
                hp: s.player.hp,
                focus: s.input.focus,
                firing: s.input.fire,
                held: s.input.held,
                // Skills are cast once agents and selectors exist (core#16).
                skills: Vec::new(),
            },
            entities,
            bullets: s
                .bullets
                .iter()
                .map(|b| BulletView {
                    at: b.at,
                    radius: Fx(rules_v0::bullet_radius(b.friendly)),
                    style: 0,
                    friendly: b.friendly,
                })
                .collect(),
        }
    }
}

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
    /// In spawn order, which is entity-ID order.
    pub enemies: Vec<EnemyState>,
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
    /// Ticks of invulnerability left after a hit.
    pub invulnerable: u32,
    /// Ticks until the main shot may fire again.
    pub shot_cooldown: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct BossState {
    pub id: EntityId,
    pub at: Point,
    pub hp: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct EnemyState {
    pub id: EntityId,
    /// Index of the wave that spawned it; its agent is `stage.waves[wave].enemy`.
    pub wave: u32,
    pub at: Point,
    pub hp: u32,
    /// Ticks since it spawned.
    pub age: u32,
    /// One per rule of its agent, in order.
    pub rules: Vec<RuleState>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RuleState {
    pub cooldown_left: u32,
    /// Firings left; `u32::MAX` for unlimited.
    pub charges_left: u32,
    pub fired: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BulletState {
    pub at: Point,
    pub vx: Fx,
    pub vy: Fx,
    /// Fired by the main character rather than an enemy.
    pub friendly: bool,
    pub damage: u32,
}
