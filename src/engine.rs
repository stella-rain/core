//! The simulation engine: runs a validated stage tick by tick to a clear or a fail
//! (ADR-004, ADR-015). The only external influence is one `Input` per tick and the commands
//! between ticks (ADR-079: `choose` and `spend`); everything else comes from the stage and its
//! seed, or from a run setup, and from the run stats (ADR-067), so the state hash after each
//! tick is the same on every platform (ADR-019).
//!
//! Rules are dispatched by the stage's `sim_version` (ADR-017). Only version 0 exists, and it
//! changes in place until the freeze (ADR-035): see `rules_v0` for what it does and does not
//! do yet.

use std::sync::Arc;

use crate::attack::{AttackPhase, DamageType, Emitter};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::input::Input;
use crate::match_ups::MatchUps;
use crate::rewards::{Rarity, Rewards};
use crate::rng::SplitMix64;
use crate::rules_v0;
use crate::run_content::RunContent;
use crate::run_setup::{self, RunSetup};
use crate::run_stats::{RunStats, StatEffect};
use crate::rune::Rune;
use crate::runes::Runes;
use crate::snapshot::{
    AttackView, BulletView, ChargeView, ComboView, DamageOverTimeView, EntityKind, EntityView,
    EvadeView, ExpertiseOfferKind, ExpertiseView, Facing, MapView, OfferKind, OrbitView,
    PlayerView, ProgressView, PullView, RuneView, ShieldView, SkillView, Snapshot, StarView,
    TypeBonusView, WeaponRole,
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
    /// An offer is open: it is answered first (ADR-079 §4).
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

    /// Starts a run from a setup alone (ADR-079 §1): the setup is validated, then its mode's
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

    /// Starts a run of the `time_limit` mode from `mode` in place of the bundled file.
    pub fn start_time_limit(
        setup: &RunSetup,
        content: &RunContent,
        _mode: &crate::time_limit::TimeLimit,
    ) -> Result<Engine, Error> {
        Engine::start_with(setup, content)
    }

    /// Advances one tick with `input`. Once the run is over this does nothing, so a replay that
    /// is longer than the run cannot change the final hash; nor while an offer is open, when
    /// the world is stopped (ADR-079 §3).
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
                self.content.expertise(),
                &mut self.state,
                input,
                &mut self.events,
                &mut self.load,
            ),
            // `new` validated the version; fail closed if that ever stops being true.
            _ => self.state.outcome = Outcome::Failed,
        }
    }

    /// Chooses option `index` of the open offer, between ticks (ADR-079 §3): the snapshot's
    /// `offer.options` lists them. It applies at once, and the events are then this choice's.
    /// Refused, changing nothing, with no offer open or an index outside its options.
    pub fn choose(&mut self, index: u32) -> Result<(), ChoiceError> {
        if !self.offer_open() {
            return Err(ChoiceError::NoOffer);
        }
        let (rewards, expertise) = (self.content.rewards(), self.content.expertise());
        let options = rules_v0::offer_options(&self.stage, rewards, expertise, &self.state);
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
            0 => rules_v0::choose(
                &self.stage,
                rewards,
                expertise,
                &mut self.state,
                option,
                &mut self.events,
            ),
            _ => self.state.outcome = Outcome::Failed,
        }
        Ok(())
    }

    /// Spends a stat point on the run stat at `index`, between ticks (ADR-079 §4): the stat
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

    /// An offer is open: `step` does nothing until `choose` closes it (ADR-079 §3).
    pub fn offer_open(&self) -> bool {
        rules_v0::offer_open(&self.state)
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
                at: r.at,
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
                    damage_over_time: dot_views(&b.statuses),
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
                        damage_over_time: dot_views(&a.statuses),
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
                        damage_over_time: dot_views(&part.statuses),
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
                    charges: s.player.evade_charges,
                    ready: s.player.evade_ready(),
                    recovering: s.player.evade_recovering.clone(),
                    dash_left: s.player.dash.map_or(0, |d| d.left),
                },
                combo: {
                    let (presses, dash, buffered, window_left, auto) = rules_v0::combo_view(s);
                    ComboView {
                        presses,
                        dash,
                        buffered,
                        window_left,
                        auto,
                    }
                },
                charge: rules_v0::charge_view(&self.stage, s).map(|(level, held, perfect)| {
                    ChargeView {
                        level,
                        held,
                        perfect,
                    }
                }),
                target: s.player.target,
                skill: s.player.skill.and_then(|slot| {
                    let def = self.content.stage_skills().get(usize::from(slot.index))?;
                    Some(SkillView {
                        skill: def.id.clone(),
                        icon: def.icon.clone(),
                        level: slot.level,
                        ready: slot.cooldown_left == 0,
                        cooldown_left: slot.cooldown_left,
                    })
                }),
                pull: s.player.pull.map(|b| PullView {
                    at: b.at,
                    radius: Fx(b.radius),
                    ticks_left: b.ticks_left,
                }),
                shield: s.player.shield.map(|sh| ShieldView {
                    amount: sh.amount,
                    ticks_left: sh.ticks_left,
                }),
                orbits: s
                    .player
                    .orbits
                    .iter()
                    .filter_map(|o| {
                        Some(OrbitView {
                            reward: self
                                .content
                                .rewards()
                                .rewards()
                                .get(usize::from(o.reward))?
                                .id
                                .clone(),
                            orbs: o.orbs,
                            cooldown_left: o.cooldown_left,
                        })
                    })
                    .collect(),
                range_pct: s.player.range_pct,
                rune_haste_pct: s.player.rune_haste_pct,
                type_bonuses: DamageType::ALL
                    .iter()
                    .zip(s.player.type_bonus_pct)
                    .filter(|(_, pct)| *pct > 0)
                    .map(|(ty, pct)| TypeBonusView {
                        damage_type: *ty,
                        pct,
                    })
                    .collect(),
                runes: s
                    .player
                    .runes
                    .iter()
                    .map(|r| RuneView {
                        rune: r.rune,
                        cooldown_left: r.cooldown_left,
                    })
                    .collect(),
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
            expertise: s.expertise.as_ref().map(|g| {
                let (main_style, sub_style, main_path) =
                    rules_v0::expertise_styles_view(&self.stage, self.content.expertise(), g);
                ExpertiseView {
                    level: g.level,
                    points: g.points,
                    next: g.next,
                    main_taken: g.main_taken.clone(),
                    sub_taken: g.sub_taken.clone(),
                    main_style,
                    sub_style,
                    main_path,
                    offer: rules_v0::expertise_offer_view(&self.stage, self.content.expertise(), s),
                }
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
    /// The expertise gauge; `None` in a stage whose enemies and boss give no expertise points,
    /// which hashes nothing for it.
    pub expertise: Option<ExpertiseGauge>,
    /// What each match-up does to a hit (ADR-100): content, fixed for the run and covered by
    /// the content hash, so the state hash writes nothing for it.
    pub match_ups: MatchUps,
    /// What each on-hit rune does when it fires (ADR-099): content, as the match-ups are.
    pub runes: Runes,
}

/// The one expertise gauge of both weapons (ADR-078 §1 to §3), and its offers (ADR-079 §2, §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpertiseGauge {
    /// From 0.
    pub level: u32,
    /// Points toward the next level; at most `next - 1` between ticks.
    pub points: u32,
    /// The points the next level needs.
    pub next: u32,
    /// The levels whose offers have opened or were passed over: those above it, up to `level`,
    /// are due, in level order.
    pub offered: u32,
    pub offer: Option<ExpertiseOffer>,
    /// For each option of the Main and of the Sub weapon's common pool, in the pool's order:
    /// times taken. Empty for a weapon with no expertise content.
    pub main_taken: Vec<u8>,
    pub sub_taken: Vec<u8>,
    /// What the range options taken add up to: every combo step's hit box that much wider,
    /// with the range rewards' (ADR-070, ADR-099).
    pub range_pct: u32,
    /// The Main and the Sub weapon's style, a place among its three, once chosen at level 3
    /// or 5; held for the run (ADR-078 §5).
    pub main_style: Option<u8>,
    pub sub_style: Option<u8>,
    /// The places of the Main style's choices taken at 6, 9 and 10, in level order.
    pub main_path: Vec<u8>,
}

/// An open expertise offer: common options, or a style's fixed choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpertiseOffer {
    /// The level that opened it.
    pub level: u32,
    pub weapon: WeaponRole,
    pub kind: ExpertiseOfferKind,
    /// Places in the weapon's common pool, or among the fixed choices, in the offer's order.
    pub candidates: Vec<u8>,
}

/// Falling stars, the offers due and the open one, and what the run's offers have used
/// (ADR-079 §2, §3).
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

/// Experience, levels and stat points (ADR-079 §2, §4).
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
    /// The enemy aim assist and auto mode face, picked each tick while either is on (ADR-072).
    pub target: Option<EntityId>,
    /// The evade's charges held, ready or recovering (ADR-074).
    pub evade_charges: u8,
    /// Ticks left on each spent charge, in the order they were spent; none is 0 and there are
    /// never more than `evade_charges`.
    pub evade_recovering: Vec<u32>,
    /// The buttons held on the last tick (`input::held_buttons`), so that only a button held now
    /// and not then is a press (ADR-071).
    pub held: u8,
    /// Where it is in its Main weapon's combos (ADR-070).
    pub combo: ComboState,
    /// The dash in progress, if any.
    pub dash: Option<DashState>,
    /// The stage skill slot (ADR-075): empty until a reward fills it.
    pub skill: Option<SkillSlot>,
    /// The stage skill's pull while it pulls, and its shield while it holds.
    pub pull: Option<PullState>,
    pub shield: Option<ShieldState>,
    /// The orbits rewards gave (ADR-069), in the order they came, at most `MAX_ORBITS`.
    pub orbits: Vec<OrbitState>,
    /// What the range rewards taken add up to, in percent, saturating (ADR-099).
    pub range_pct: u32,
    /// What the Rune Haste rewards taken add up to, in percent, saturating (ADR-099).
    pub rune_haste_pct: u32,
    /// What the type bonus rewards taken add up to for each damage type, by its place in
    /// `DamageType::ALL`, in percent, saturating (ADR-100): a group's bonus is added to each
    /// type of its group.
    pub type_bonus_pct: [u32; DamageType::ALL.len()],
    /// The runes the Main weapon holds, in the order drawn, each once, at most `MAX_RUNES`
    /// (ADR-099).
    pub runes: Vec<HeldRune>,
    pub aggro: u32,
    pub statuses: Vec<StatusState>,
}

impl PlayerState {
    /// The evade's charges ready: held and not recovering.
    pub fn evade_ready(&self) -> u8 {
        // There are never more timers than charges, and charges are at most 3.
        self.evade_charges
            .saturating_sub(self.evade_recovering.len() as u8)
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
    /// A press by hand took a step in this combo (ADR-073): auto mode waits, and the combo
    /// becomes the auto combo when it resets, unless it began in the dash branch.
    pub hand: bool,
    /// The auto combo auto mode plays: its presses from the root, the first in bit 0 (1 for
    /// `sub`), and how many; at most the tree's depth.
    pub auto_path: u32,
    pub auto_depth: u8,
}

/// The Sub weapon's pull on what a combo part hits (ADR-070), carried by the attack and the
/// bullets it spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pull {
    pub distance: i32,
    pub ticks: u32,
}

/// The stage skill held (ADR-075): its place in the stage skills file, its level from 1, and
/// the ticks until it is ready again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SkillSlot {
    pub index: u16,
    pub level: u8,
    pub cooldown_left: u32,
}

/// An orbit held: its reward's place in the rewards file, its orbs, the orbs of the round going
/// on, the ticks until the next round may start, and where orb 0 is round the turn (1/256
/// degree from straight down the screen, in `0..PER_TURN`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OrbitState {
    pub reward: u16,
    pub orbs: u8,
    pub round: u8,
    pub cooldown_left: u32,
    pub angle: i32,
}

/// A rune the Main weapon holds, and for an on-hit rune the ticks until it may fire again;
/// always 0 for a damage-over-time rune.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeldRune {
    pub rune: Rune,
    pub cooldown_left: u32,
}

/// A pull pulling: its point, radius and strength (fixed-point units), and the ticks left, at
/// least 1.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PullState {
    pub at: Point,
    pub radius: i32,
    pub strength: i32,
    pub ticks_left: u32,
}

/// A shield: the damage it still absorbs, at least 1, and the ticks left, at least 1.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShieldState {
    pub amount: u32,
    pub ticks_left: u32,
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

/// The statuses of this version (ADR-036): the three a skill can apply, and bleed, poison and
/// burn, which only their runes apply (ADR-099). The IDs in the registry are the names of the
/// first three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusKind {
    /// More damage dealt.
    AtkUp,
    /// More damage taken.
    Vulnerable,
    /// Slower movement.
    Slow,
    /// Damage over time, in stacks (ADR-099, ADR-084 point 8).
    Bleed,
    /// Damage over time, in stacks, of its own beside bleed's (ADR-099).
    Poison,
    /// Damage over time, in stacks, of its own beside bleed's (ADR-099).
    Burn,
}

impl StatusKind {
    /// The statuses a skill can apply: the damage-over-time ones are not among them.
    pub const FROM_SKILLS: [StatusKind; 3] =
        [StatusKind::AtkUp, StatusKind::Vulnerable, StatusKind::Slow];

    /// The status a skill can apply whose registry ID is `name`.
    pub fn from_name(name: &str) -> Option<StatusKind> {
        StatusKind::FROM_SKILLS
            .into_iter()
            .find(|k| k.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            StatusKind::AtkUp => "atk_up",
            StatusKind::Vulnerable => "vulnerable",
            StatusKind::Slow => "slow",
            StatusKind::Bleed => "bleed",
            StatusKind::Poison => "poison",
            StatusKind::Burn => "burn",
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
    /// A damage-over-time status's own state; `None` for every other status, which hashes
    /// nothing for it.
    pub dot: Option<DotState>,
}

/// The damage-over-time statuses of `statuses`, for the snapshot.
fn dot_views(statuses: &[StatusState]) -> Vec<DamageOverTimeView> {
    statuses
        .iter()
        .filter_map(|st| {
            st.dot.map(|d| DamageOverTimeView {
                status: ContentId(st.kind.name().to_owned()),
                stacks: st.stacks,
                ticks_left: st.remaining,
                next_tick_in: d.next_in,
            })
        })
        .collect()
}

/// What a damage-over-time status deals and when (ADR-099 points 2 and 3): its damage a stack
/// and its type, from the content that applied it; the atk-up and type bonus of its source,
/// fixed when it was last applied, as an attack's are when it starts (ADR-056); its interval,
/// and the ticks to its next tick, at least 1.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DotState {
    pub damage: u32,
    pub damage_type: Option<DamageType>,
    pub atk_up_pct: u32,
    pub type_bonus_pct: u32,
    pub interval: u32,
    pub next_in: u32,
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
    /// The type of the phase that fired it (ADR-100); `None` is neutral.
    pub damage_type: Option<DamageType>,
    /// Its hit breaks the main character's charge (core#83).
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub pull: Option<Pull>,
    /// Fired by a combo step, A or B: its hit adds expertise points (ADR-078 §2).
    pub combo: bool,
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
    pub damage_type: Option<DamageType>,
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub pull: Option<Pull>,
    /// Of a combo step, A or B (ADR-078 §2).
    pub combo: bool,
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
    /// The attacker's type bonus for each damage type, by its place in `DamageType::ALL`, in
    /// percent, each at most the match-up file's `max_type_bonus_pct`, fixed when the attack
    /// starts (ADR-100): the main character's, once it holds any; none for every other
    /// attacker. Boxed, so that a run without one stays as small as it was.
    pub type_bonus_pct: Option<Box<[u32; DamageType::ALL.len()]>>,
    /// `phase_time` of the attack's first tick, for the nodes its phases spawn.
    pub age: u32,
    /// Who the current active phase has hit, and on which tick last, sorted by entity ID;
    /// emptied when an active phase begins.
    pub hits: Vec<(EntityId, u32)>,
    /// The Sub weapon's pull, when the attack is a combo part (ADR-070).
    pub pull: Option<Pull>,
    /// Where its hit boxes are centred instead of on its attacker: an on-hit rune's attack, at
    /// the point the combo step hit (ADR-099), or where a thrown one has flown to.
    pub at: Option<Point>,
    /// A thrown rune's turn point, the point the combo step hit: its first phase flies `at`
    /// there from the main character, its second back to the main character (boomerang). A
    /// homing missile's goal: where its target is, or was last.
    pub throw: Option<Point>,
    /// A homing missile's target (ADR-099): `throw` follows it while it is alive, and the
    /// missile is gone once it hits anything.
    pub homing: Option<EntityId>,
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
    /// The type of the phase that started it, which its bullets carry (ADR-100).
    pub damage_type: Option<DamageType>,
    /// The bullets it fires break the main character's charge (core#83).
    pub breaks_charge: bool,
    /// The Sub weapon's pull, for a combo part's hits (ADR-070).
    pub pull: Option<Pull>,
    /// Started by a combo step, A or B: its hits add expertise points (ADR-078 §2).
    pub combo: bool,
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
