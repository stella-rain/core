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
use crate::behaviour::{BossCall, Movement};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::input::Input;
use crate::match_ups::MatchUps;
use crate::registry;
use crate::rewards::{Rarity, Rewards};
use crate::rng::SplitMix64;
use crate::rules_v0;
use crate::run_content::RunContent;
use crate::run_setup::{self, RunSetup};
use crate::run_stats::{RunStats, StatEffect};
use crate::rune::Rune;
use crate::runes::Runes;
use crate::snapshot::{
    AttackView, BulletView, ChargeView, ComboView, Countdown, DamageOverTimeView, EntityKind,
    EntityView, EvadeView, ExpertiseOfferKind, ExpertiseView, Facing, HealPickupView, HoldView,
    MapView, OfferKind, OrbitView, PlayerView, ProgressView, PullView, RuneView, ShieldView,
    SkillView, Snapshot, StarView, StatusView, TypeBonusView,
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
        Engine::new_with_upgrades(stage, content, &[])
    }

    /// As `new_with`, for a player who holds the permanent `upgrades` (ADR-079): the combo
    /// steps that name one of them are open, the others absent (ADR-084 point 7). IDs the
    /// registry does not hold open nothing; a run setup validates its own.
    pub fn new_with_upgrades(
        stage: &Stage,
        content: &RunContent,
        upgrades: &[ContentId],
    ) -> Result<Engine, Error> {
        validate::validate(stage)?;
        let pool = rules_v0::RunPool {
            rewards: pool(None, content.rewards())?,
            extra_rerolls: 0,
            upgrades: registry::upgrade_mask(upgrades.iter().map(|id| id.0.as_str())),
            difficulty: rules_v0::RunDifficulty::NONE,
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
        run_setup::validate_in(setup, content)?;
        if setup.mode.0 == "time_limit" {
            return Engine::start_time_limit(setup, content, content.time_limit());
        }
        let stage = run_setup::stage_in(setup, content)?;
        let pool = rules_v0::RunPool {
            rewards: pool(setup.star_pool.as_deref(), content.rewards())?,
            extra_rerolls: setup.extra_rerolls,
            upgrades: registry::upgrade_mask(setup.upgrades.iter().map(|id| id.0.as_str())),
            difficulty: run_difficulty(setup, content)?,
        };
        let mut state = rules_v0::initial_state(&stage, content, pool, setup.seed);
        let loadout = rules_v0::Loadout::of(setup, content);
        rules_v0::apply_loadout(&stage, content.rewards(), &mut state, &loadout);
        Ok(Engine {
            state,
            stage,
            content: content.clone(),
            events: Vec::new(),
            load: Load::default(),
        })
    }

    /// Starts a run of the `time_limit` mode from `mode` in place of the bundled file: its
    /// waves, and its bosses as they come (`rules_v0::spawn_boss`).
    pub fn start_time_limit(
        setup: &RunSetup,
        content: &RunContent,
        mode: &crate::time_limit::TimeLimit,
    ) -> Result<Engine, Error> {
        run_setup::validate_in(setup, content)?;
        let difficulty = run_setup::difficulty_of(setup, content.difficulties())?;
        let (stage, horde) = run_setup::time_limit_run(setup, mode, difficulty, content.trial())?;
        let pool = rules_v0::RunPool {
            rewards: pool(setup.star_pool.as_deref(), content.rewards())?,
            extra_rerolls: setup.extra_rerolls,
            upgrades: registry::upgrade_mask(setup.upgrades.iter().map(|id| id.0.as_str())),
            difficulty: run_difficulty(setup, content)?,
        };
        let mut state = rules_v0::initial_state(&stage, content, pool, setup.seed);
        state.horde = Some(horde);
        let loadout = rules_v0::Loadout::of(setup, content);
        rules_v0::apply_loadout(&stage, content.rewards(), &mut state, &loadout);
        Ok(Engine {
            state,
            stage,
            content: content.clone(),
            events: Vec::new(),
            load: Load::default(),
        })
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
            0 => {
                rules_v0::spawn_boss(&mut self.stage, &mut self.state);
                rules_v0::step(
                    &self.stage,
                    self.content.rewards(),
                    self.content.expertise(),
                    &mut self.state,
                    input,
                    &mut self.events,
                    &mut self.load,
                );
            }
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

    /// The Stardust the run has earned, times 10 (ADR-109); 0 in a stage without a factor.
    pub fn stardust(&self) -> u32 {
        self.state.stardust.unwrap_or(0)
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
        // The auras of the elites, read once: an enemy within reach of one is strengthened.
        let auras: Vec<_> = rules_v0::aura_sources(&self.stage, s).collect();
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
                    disguised: false,
                    elite: false,
                    aura_radius: None,
                    damage_over_time: dot_views(&b.statuses),
                    statuses: status_views(&b.statuses, false),
                    fight: None,
                });
            }
        }
        // Boss, companions, boss parts, then enemies: the order of their entity IDs.
        let agent_views =
            |kind, agents: &[crate::engine::AgentState], out: &mut Vec<EntityView>| {
                for a in agents {
                    let def = rules_v0::agent_def(&self.stage, a.key);
                    let hidden = a.hidden();
                    // Only a non-elite enemy is strengthened by an aura (ADR-090).
                    let reached = kind == EntityKind::Enemy
                        && a.elite.is_none()
                        && rules_v0::aura_percent(auras.iter().copied(), a.at) > 0;
                    out.push(EntityView {
                        id: a.id,
                        kind,
                        asset: match def.elite.as_ref().and_then(|e| e.mimic.as_ref()) {
                            Some(m) if hidden => m.disguise.clone(),
                            _ => ContentId(def.base.0.clone()),
                        },
                        palette: def.palette.clone(),
                        at: a.at,
                        radius: def.radius,
                        hp: a.hp,
                        attack: attack_of(a.id),
                        disguised: hidden,
                        elite: a.elite.is_some() && !hidden,
                        aura_radius: def
                            .elite
                            .as_ref()
                            .and_then(|e| e.aura)
                            .map(|aura| aura.radius),
                        damage_over_time: dot_views(&a.statuses),
                        statuses: status_views(&a.statuses, reached),
                        fight: match (&def.movement, a.shrine.as_deref()) {
                            (
                                Movement::Shrine { fight_ticks, .. },
                                Some(ShrineState {
                                    fight: Some(since), ..
                                }),
                            ) => Some(Countdown {
                                remaining: fight_ticks.saturating_sub(*since),
                                total: *fight_ticks,
                            }),
                            _ => None,
                        },
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
                        disguised: false,
                        elite: false,
                        aura_radius: None,
                        damage_over_time: dot_views(&part.statuses),
                        statuses: status_views(&part.statuses, false),
                        fight: None,
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
                auto_charge: rules_v0::auto_charge_view(s),
                hold: rules_v0::hold_view(&self.stage, s)
                    .map(|(held, max_ticks)| HoldView { held, max_ticks }),
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
                pickup_radius_pct: s.player.pickup_radius_pct,
                skill_cooldown_pct: s.player.skill_cooldown_pct,
                kill_heal: s.player.kill_heal,
                kill_heal_every: s.player.kill_heal_every,
                crit_chance_pct: s.player.crit_chance_pct,
                crit_damage_pct: s.player.crit_damage_pct,
                echo_hit_pct: s.player.echo_hit_pct,
                burst_pct: s.player.burst_pct,
                burst_radius: s.player.burst_radius,
                burst_depth: s.player.burst_depth,
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
                statuses: status_views(&s.player.statuses, false),
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
            constellations: rules_v0::constellation_views(self.content.rewards(), s),
            heal_pickups: s.heal_pickups.as_ref().map_or_else(Vec::new, |h| {
                h.iter()
                    .map(|p| HealPickupView { id: p.id, at: p.at })
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
                let (style, path) =
                    rules_v0::expertise_styles_view(&self.stage, self.content.expertise(), g);
                ExpertiseView {
                    level: g.level,
                    points: g.points,
                    next: g.next,
                    taken: g.taken.clone(),
                    style,
                    path,
                    offer: rules_v0::expertise_offer_view(&self.stage, self.content.expertise(), s),
                }
            }),
            patterns: rules_v0::announced_patterns(&self.stage, s),
            stardust: s.stardust,
        }
    }
}

/// What the setup's difficulty brings to the run (ADR-110): its percents, and its rarity range
/// clamped to the rarities the run's rewards have.
fn run_difficulty(
    setup: &RunSetup,
    content: &RunContent,
) -> Result<rules_v0::RunDifficulty, Error> {
    let d = run_setup::difficulty_of(setup, content.difficulties())?;
    Ok(rules_v0::RunDifficulty {
        hp_pct: d.hp_pct,
        damage_pct: d.damage_pct,
        rarity: d.rarity_range(content.rewards()),
    })
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
    /// What `runs` leave each attacker's speed; updated wherever `runs` changes.
    pub move_index: MoveIndex,
    /// In spawn order.
    pub bullets: Vec<BulletState>,
    /// Enemies spawned so far, per wave.
    pub spawned: Vec<u16>,
    pub next_id: u32,
    /// Experience and levels; `None` in a stage without levels, which hashes nothing for it.
    pub progress: Option<Progress>,
    /// Falling stars and offers; `None` in a stage without stars, which hashes nothing for it.
    pub stars: Option<StarField>,
    /// Heal pickups on the map, in the order they dropped; `None` in a stage without healing,
    /// which hashes nothing for it.
    pub heal_pickups: Option<Vec<HealPickup>>,
    /// The expertise gauge; `None` in a stage whose enemies and boss give no expertise points,
    /// which hashes nothing for it.
    pub expertise: Option<ExpertiseGauge>,
    /// What each match-up does to a hit (ADR-100): content, fixed for the run and covered by
    /// the content hash, so the state hash writes nothing for it.
    pub match_ups: MatchUps,
    /// What each on-hit rune does when it fires (ADR-099): content, as the match-ups are.
    pub runes: Runes,
    /// The stun rules (ADR-085 points 4 and 6): content, as the match-ups are.
    pub stun: crate::run_stats::StunRules,
    /// The cap of the attack speed total (app#298): content, as the stun rules are.
    pub max_attack_speed_pct: u32,
    /// The cap of Rune Haste (app#298): the rewards file's, held to the sum of the picks where
    /// it applies.
    pub max_rune_haste_pct: u32,
    /// The bosses of a `time_limit` run, as they come; `None` in any other run, which hashes
    /// nothing for it.
    pub horde: Option<Horde>,
    /// The Stardust the run has earned, times 10 (ADR-109): `Some` in a stage with a Stardust
    /// factor, hashed only then, so the hashes of stages without stay as they were.
    pub stardust: Option<u32>,
    /// Hit points the main character regenerates each second (a perk, ADR-105): fixed for the
    /// run, part of the setup, so the state hash writes nothing for it.
    pub regen: u32,
    /// The percents the run's difficulty scales enemies' hit points and attacks' damage by
    /// (ADR-110): fixed for the run, part of the setup, so the state hash writes nothing for them.
    pub hp_pct: u32,
    pub damage_pct: u32,
    /// The run's seed, which the critical-hit rolls are hashed from: they draw nothing from
    /// `rng`, so a run without crit chance spends the same random numbers as before.
    pub crit_seed: u64,
    /// The echoes of last tick's hits, waiting for this tick's `collide`, in the order the hits
    /// landed; at most `MAX_ECHOES_PER_TICK`.
    pub echoes: Vec<Echo>,
}

/// One hit's echo, queued when the hit lands and dealt a tick later (core#NS2b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Echo {
    pub source: EntityId,
    pub target: EntityId,
    /// Hit points taken, already the hit's percent of its damage.
    pub damage: u32,
}

/// What a `time_limit` run brings to its bosses (core#212): the times, the pool they are drawn
/// from and the boss they are drawn as, all content fixed for the run (covered by the content
/// hash); what changes is how many have come and which pool entries are used up.
#[derive(Clone, Debug)]
pub(crate) struct Horde {
    pub boss_ticks: Vec<u32>,
    pub pool: Vec<ContentId>,
    pub final_boss: ContentId,
    /// The boss every one is made from, with its sprite replaced, when the mode file gives no
    /// definition for its sprite: a placeholder.
    pub template: crate::stage::Boss,
    /// The mode file's boss definitions (core#336), by sprite.
    pub bosses: Vec<crate::stage::Boss>,
    /// The boss times at which the boss calls no elites (core#336).
    pub no_call_ticks: Vec<u32>,
    /// The mid-bosses' hit points by slot, in percent (core#336).
    pub mid_boss_hp_pct: Vec<u32>,
    /// Bosses that have come, mid-bosses and the final one.
    pub spawned: u32,
    /// One per pool entry: drawn already.
    pub used: Vec<bool>,
    /// The run's length: an elite minute comes only before it.
    pub length_ticks: u32,
    /// The mode's elite table (ADR-090), by start.
    pub elite_table: Vec<crate::time_limit::EliteTier>,
    /// For each elite type of the mode file, the stage wave that holds its definition: the
    /// type's enemy, experience and expertise, never spawned as a wave (count 0). A kill's
    /// experience and expertise are the wave's.
    pub elite_waves: Vec<(ContentId, u32)>,
}

/// The one expertise gauge of the one weapon (ADR-106 §1 to §3), and its offers (ADR-079 §2, §3).
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
    /// For each option of the weapon's common pool, in the pool's order: times taken. Empty
    /// for a weapon with no expertise content.
    pub taken: Vec<u8>,
    /// What the range options taken add up to: every combo step's hit box that much wider,
    /// with the range rewards' (ADR-070, ADR-099).
    pub range_pct: u32,
    /// The weapon's style, a place among its three, once chosen at the level table's style
    /// level; held for the run (ADR-106 §5).
    pub style: Option<u8>,
    /// The places of the style's choices taken at the upgrade and capstone levels, in level
    /// order.
    pub path: Vec<u8>,
}

/// An open expertise offer: common options, or a style's fixed choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExpertiseOffer {
    /// The level that opened it.
    pub level: u32,
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
    /// Candidates the offer-candidates rewards taken add to every offer, held to
    /// `MAX_OFFER_CANDIDATES` less the base `OFFER_SIZE`.
    pub extra_candidates: u8,
    /// The rarities its stars and offers may have, lowest and highest (ADR-110): fixed for the
    /// run, so the state hash writes nothing for it.
    pub rarity: (Rarity, Rarity),
    /// The runes the random rune may draw, by `Rune::index` (ADR-107): fixed for the run, part
    /// of the setup, so the state hash writes nothing for it.
    pub rune_pool: u8,
    /// The run's pool: places in the rewards file, in its order, fixed for the run.
    pub pool: Vec<u16>,
    /// For each reward of the file: times taken, and whether it is banished.
    pub taken: Vec<u8>,
    pub banished: Vec<bool>,
    /// For each set of the run's constellations: the members held, counted after every take
    /// (core#NS3). The state hash writes the sets above 0 only.
    pub set_counts: Vec<u8>,
}

/// A heal pickup on the map.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HealPickup {
    pub id: EntityId,
    pub at: Point,
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
    /// What the pickup radius rewards taken add up to, in percent, saturating: the pickup
    /// circle of stars and heal pickups is that much wider, held to
    /// `MAX_PICKUP_RADIUS_TOTAL_PCT` where it is used.
    pub pickup_radius_pct: u32,
    /// What the skill cooldown rewards taken add up to, in percent, saturating: the stage
    /// skill's cooldown is that much shorter, held to `MAX_SKILL_COOLDOWN_TOTAL_PCT`.
    pub skill_cooldown_pct: u32,
    /// Hit points (the stage's scale) the heal-per-kills rewards taken heal together each time
    /// `kill_heal_every` kills have been counted; their amounts add up, held to
    /// `MAX_HEAL_AMOUNT` where it is used.
    pub kill_heal: u32,
    /// The kills between two such heals: the shortest interval among the picks, 0 while none
    /// is held.
    pub kill_heal_every: u32,
    /// Kills counted since the last such heal, below `kill_heal_every` between ticks.
    pub kills_counted: u32,
    /// What the crit chance rewards taken add up to, in percent, saturating; held to
    /// `MAX_CRIT_CHANCE_TOTAL_PCT` where it is rolled.
    pub crit_chance_pct: u32,
    /// What the crit damage rewards add up to: a crit does twice the damage and this many
    /// percent more, held to `MAX_CRIT_DAMAGE_TOTAL_PCT`.
    pub crit_damage_pct: u32,
    /// What the echo rewards add up to: each hit echoes this percent of its damage, held to
    /// `MAX_ECHO_HIT_PCT`; 0 while none is held.
    pub echo_hit_pct: u32,
    /// The kill burst rewards taken, added up: the percent of the dead enemy's maximum hit
    /// points, the radius in pixels, and the longest chain; all 0 while none is held.
    pub burst_pct: u32,
    pub burst_radius: u32,
    pub burst_depth: u32,
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
    /// The presses from the root to the current step, the first in bit 0: 1 for `smash`.
    pub path: u32,
    /// How many presses `path` holds: 0 at the root.
    pub depth: u8,
    /// The path began in the dash branch.
    pub dash: bool,
    /// The press waiting for the next step: `Some(true)` for `smash`.
    pub buffered: Option<bool>,
    /// The current step's attack is running.
    pub running: bool,
    /// Ticks left in which a press continues the combo, once a step has run out.
    pub window_left: u32,
    /// Ticks left in which a press from the root takes the dash branch.
    pub dash_strike_left: u32,
    /// The current step charging: ticks held, 0 on the tick it began.
    pub charge: Option<u32>,
    /// The current step held (ADR-084 point 6): ticks held, 0 on the tick it began.
    pub hold: Option<u32>,
    /// The current step's hold has released, so its release attack, and its recovery, never
    /// begins a hold again; a step taken next clears it.
    pub released: bool,
    /// A press by hand took a step in this combo (ADR-073): auto mode waits, and the combo
    /// becomes the auto combo when it resets, unless it began in the dash branch.
    pub hand: bool,
    /// The auto combo auto mode plays: its presses from the root, the first in bit 0 (1 for
    /// `smash`), and how many; at most the tree's depth.
    pub auto_path: u32,
    pub auto_depth: u8,
    /// The charge level each step of the combo was released at (ADR-114), 4 bits a step from
    /// the root, the first in the low bits (a charge has at most `MAX_CHARGE_LEVELS` = 8
    /// levels): by hand, the level the player let go at; for a step auto mode played, the level
    /// it played. 0 for a plain step.
    pub levels: u32,
    /// The same for the auto combo: the level auto mode charges each of its steps to.
    pub auto_levels: u32,
    /// The current charge is auto mode's: it needs no button and is never perfect.
    pub charge_auto: bool,
    /// The permanent upgrades the run holds, as a mask (`registry::upgrade_mask`): fixed for the
    /// run, so it is not part of the state hash. A step needing one that is not here is absent.
    pub upgrades: u32,
}

/// The Sub weapon's pull on what a combo part hits (ADR-070), carried by the attack and the
/// bullets it spawns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pull {
    pub distance: i32,
    pub ticks: u32,
}

/// What a smash (a step reached by B) does to the enemy it hits (ADR-084 point 8), carried by
/// the attack and the bullets it spawns, as a combo step's hits carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Smash {
    Pull(Pull),
    /// The enemy is pushed away from the main character, as far and as long as the pull's numbers.
    Knockback(Pull),
    /// The `bleed` status, with the bleed rune's numbers from the runes file (ADR-099 point 3).
    Bleed,
    /// The `stun` status of a smash, for its ticks (ADR-085).
    Stun {
        ticks: u32,
    },
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
    /// Ticks since it arrived: 1 on its first tick. The call of its elites counts them
    /// (ADR-103), and so does the announcement of that call.
    pub age: u32,
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
    /// Rung around a shrine: `index` is the shrine's wave.
    Guardian,
    /// Brought by a pack leader: `index` is the leader's wave.
    Escort,
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
    /// Enemies only: it removed itself with a `self_destruct` rule. It is at 0 hit points for
    /// the removal, but it was not defeated, so nothing credits it. Not hashed: it only lives
    /// until the end of the tick it is set in.
    pub burst: bool,
    /// Ticks left in which a new stun cannot land on this enemy, from the end of its last stun
    /// (ADR-085 point 4); 0 for none.
    pub stun_immune: u32,
    /// An elite's own state; `None` for every other agent, which hashes nothing for it.
    pub elite: Option<EliteState>,
    /// An agent that leaves the map unkilled (a goblin, or any elite whose `boss_call` is
    /// `leave`); `None` for every other agent, which hashes nothing for it.
    pub leave: Option<LeaveState>,
    /// A shrine's own state; `None` for every other agent, which hashes nothing for it. Boxed:
    /// almost no agent has one.
    pub shrine: Option<Box<ShrineState>>,
}

impl AgentState {
    /// A mimic that has not revealed yet: an object nothing touches (ADR-090).
    pub(crate) fn hidden(&self) -> bool {
        self.elite.is_some_and(|el| el.disguised)
    }
}

/// A shrine's fight (ADR-090).
#[derive(Clone, Debug)]
pub(crate) struct ShrineState {
    /// Ticks since the main character touched it, 0 on the tick it did; `None` while untouched.
    pub fight: Option<u32>,
    /// The guardians that spawned, in ring order.
    pub guardians: Vec<EntityId>,
}

/// When an elite that leaves is to go (ADR-090).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LeaveState {
    /// Ticks since it was disturbed, 0 on the tick it was; `None` while it is not.
    pub disturbed: Option<u32>,
    /// A boss came or called: it goes on this tick.
    pub go: bool,
}

/// An elite holds `home`, where it spawned, until a boss calls it (ADR-068).
#[derive(Clone, Copy, Debug)]
pub(crate) struct EliteState {
    pub home: Point,
    pub called: bool,
    /// What the boss's call does to it; its definition's, so not part of the hash.
    pub boss_call: BossCall,
    /// A mimic that has not revealed yet (ADR-090); false for every other elite.
    pub disguised: bool,
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
    /// The enemy stops what it was doing for its ticks (ADR-085). Not a skill's status.
    Stun,
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
            StatusKind::Stun => "stun",
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
    /// The ticks of the application that set `remaining`, for the snapshot (ADR-103): a
    /// refresh with a longer duration sets it, a shorter one leaves both alone.
    pub total: u32,
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

/// The statuses of `statuses` for the snapshot (ADR-103 decision 3), sorted by ID: each with
/// what is left of it and the ticks it was applied for. `aura` adds a pack leader's aura,
/// which has no timer: it lasts while its entity is in reach.
fn status_views(statuses: &[StatusState], aura: bool) -> Vec<StatusView> {
    let mut views: Vec<StatusView> = statuses
        .iter()
        .map(|st| StatusView {
            id: ContentId(st.kind.name().to_owned()),
            remaining: st.remaining,
            total: st.total,
        })
        .collect();
    if aura {
        views.push(StatusView {
            id: ContentId("aura".to_owned()),
            remaining: 0,
            total: 0,
        });
    }
    views.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    views
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
    pub smash: Option<Smash>,
    /// Fired by a combo step, A or B: its hit adds expertise points (ADR-078 §2).
    pub combo: bool,
    /// Starts an attack of its own after a while (`on_bullet`).
    pub hook: Option<Box<Hook>>,
    /// Ticks left before it leaves the map, `UNLIMITED` for a bullet with no lifetime (core#147).
    /// It leaves when this runs out or when it is past the map's margin, whichever comes first.
    pub life: u32,
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
    pub smash: Option<Smash>,
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
    /// The percent of the damage a difficulty scales an enemy's or the boss's attack by
    /// (ADR-110), fixed when the attack starts; 100 for the main character's and its allies'.
    pub damage_pct: u32,
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
    pub smash: Option<Smash>,
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

/// The movement speed, in percent, that the running attacks leave each attacker: the lowest
/// `move_pct` of the current phases of its runs. Kept beside `State::runs` so that asking costs
/// a search over the attackers, not a scan of every run; a derived value that no hash reads.
/// Sorted by owner, one entry for each owner with a run.
#[derive(Clone, Debug, Default)]
pub(crate) struct MoveIndex {
    by_owner: Vec<(EntityId, i64)>,
}

impl MoveIndex {
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
    pub fn rebuild(&mut self, runs: &[AttackRun]) {
        self.by_owner.clear();
        self.by_owner.extend(
            runs.iter()
                .map(|r| (r.owner, i64::from(r.phases[r.phase as usize].move_pct))),
        );
        // Lowest first within an owner, so that the entry `dedup_by_key` keeps is the lowest.
        self.by_owner.sort_unstable();
        self.by_owner.dedup_by_key(|e| e.0);
    }
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
    pub smash: Option<Smash>,
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

#[cfg(test)]
mod horde_tests {
    use super::*;
    use crate::run_content::RunContent;
    use crate::time_limit::TimeLimit;

    fn engine(boss_ticks: &[u32]) -> Engine {
        engine_with(boss_ticks, &[])
    }

    fn engine_with(boss_ticks: &[u32], no_call_ticks: &[u32]) -> Engine {
        let mut v: serde_json::Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
        v["no_call_ticks"] = serde_json::json!(no_call_ticks);
        v["length_ticks"] = serde_json::json!(boss_ticks.last().unwrap());
        v["boss_ticks"] = serde_json::json!(boss_ticks);
        v["waves"] = serde_json::json!([]);
        v["elite_table"] = serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
        let mode = TimeLimit::parse(v.to_string().as_bytes()).unwrap();
        let setup = RunSetup {
            schema_version: 0,
            sim_version: 0,
            mode: "time_limit".into(),
            weapon: "giant_sword".into(),
            seed: 1,
            star_pool: None,
            extra_rerolls: 0,
            upgrades: Vec::new(),
            perk_set: "duelist".into(),
            rune_pool: Vec::new(),
            carried: Vec::new(),
            lumine: 0,
            starting_rune: None,
            difficulty: "normal".into(),
        };
        Engine::start_time_limit(&setup, RunContent::bundled(), &mode).unwrap()
    }

    fn run_to(e: &mut Engine, tick: u32) {
        while e.tick() < tick {
            e.step(Input::default());
        }
    }

    /// The tick each boss came on and the tick it called its elites on, the main character
    /// kept alive.
    fn arrivals_and_calls(e: &mut Engine, until: u32) -> (Vec<u32>, Vec<u32>) {
        let (mut arrived, mut called) = (Vec::new(), Vec::new());
        let mut last = None;
        while e.tick() < until {
            e.state.player.hp = e.state.player.max_hp;
            e.step(Input::default());
            let id = e.state.boss.as_ref().map(|b| b.id);
            if id.is_some() && id != last {
                arrived.push(e.tick());
            }
            last = id;
            if e.events()
                .iter()
                .any(|ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
            {
                called.push(e.tick());
            }
        }
        (arrived, called)
    }

    #[test]
    fn a_boss_calls_its_elites_after_the_same_time_whenever_it_arrives() {
        // Mid-bosses at 60 and 3000, the final boss at 6000; the mode's call is 1800 ticks.
        let mut e = engine(&[60, 3000, 6000]);
        let (arrived, called) = arrivals_and_calls(&mut e, 6000);
        assert_eq!(arrived, [60, 3000, 6000]);
        let delays: Vec<u32> = arrived.iter().zip(&called).map(|(a, c)| c - a).collect();
        assert_eq!(delays, [1800, 1800], "{arrived:?} {called:?}");
    }

    /// Drops the boss on the field below 70% of its hit points and steps once: whether it
    /// called its elites on that tick.
    fn calls_below_70_pct(e: &mut Engine) -> bool {
        let boss = e.state.boss.as_mut().unwrap();
        boss.hp = boss.max_hp * 69 / 100;
        e.step(Input::default());
        e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
    }

    #[test]
    fn a_boss_calls_below_70_pct_unless_its_time_is_a_no_call_time() {
        // The first time stands for the 5-minute boss, the second for the 10-minute one.
        let mut e = engine_with(&[50, 200, 400], &[50]);
        run_to(&mut e, 60);
        assert!(!calls_below_70_pct(&mut e), "a no-call boss never calls");
        run_to(&mut e, 210);
        assert!(calls_below_70_pct(&mut e), "well before its 30 seconds");
    }

    #[test]
    fn the_final_boss_is_the_core_pair_and_its_death_clears_the_run() {
        let mut e = engine(&[50, 100]);
        run_to(&mut e, 100);
        assert_eq!(e.stage.boss.as_ref().unwrap().base.0, "core_pair");
        assert_eq!(e.state.boss.as_ref().unwrap().parts.len(), 2);
        e.state.boss.as_mut().unwrap().hp = 0;
        e.step(Input::default());
        assert_eq!(e.outcome(), Outcome::Cleared);
    }

    #[test]
    fn a_mid_boss_s_death_does_not_clear_the_run_and_the_final_boss_s_does() {
        let mut e = engine(&[50, 100]);
        run_to(&mut e, 50);
        assert!(e.state.boss.as_ref().is_some_and(|b| b.hp > 0));
        // The mid-boss falls; the stage's own rule would clear the run here.
        e.state.boss.as_mut().unwrap().hp = 0;
        run_to(&mut e, 99);
        assert_eq!(e.outcome(), Outcome::Running);
        run_to(&mut e, 100);
        let final_boss = e.state.boss.as_ref().unwrap();
        assert!(final_boss.hp > 0, "a new boss at the last time");
        e.state.boss.as_mut().unwrap().hp = 0;
        e.step(Input::default());
        assert_eq!(e.outcome(), Outcome::Cleared);
        assert!(e.events().contains(&DomainEvent::StageCleared));
    }

    #[test]
    fn the_main_character_s_death_fails_the_run_and_wins_over_the_final_boss_s() {
        let mut e = engine(&[50, 100]);
        run_to(&mut e, 100);
        e.state.boss.as_mut().unwrap().hp = 0;
        e.state.player.hp = 0;
        e.step(Input::default());
        assert_eq!(e.outcome(), Outcome::Failed);
        assert!(e.events().contains(&DomainEvent::StageFailed));
        assert!(!e.events().contains(&DomainEvent::StageCleared));
    }

    #[test]
    fn the_run_lasts_its_length_and_its_overtime_and_an_empty_field_does_not_clear_it() {
        let mut e = engine(&[50, 100]);
        run_to(&mut e, 99);
        assert_eq!(e.outcome(), Outcome::Running);
        assert_eq!(e.stage.length_ticks, 100 + 18_000, "the run's overtime");
    }

    #[test]
    fn the_run_fails_when_the_overtime_runs_out_with_the_final_boss_alive() {
        let mut e = engine(&[50, 100]);
        e.state.player.hp = u32::MAX;
        e.state.player.max_hp = u32::MAX;
        run_to(&mut e, 100 + 18_000 - 1);
        assert_eq!(e.outcome(), Outcome::Running);
        e.step(Input::default());
        assert_eq!(e.outcome(), Outcome::Failed);
        assert!(e.events().contains(&DomainEvent::StageFailed));
    }

    /// A `time_limit` run of `edit`ed bundled mode, no waves, that the main character cannot die in.
    fn run_of(seed: u64, edit: impl FnOnce(&mut serde_json::Value)) -> Engine {
        let mut v: serde_json::Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
        v["waves"] = serde_json::json!([]);
        edit(&mut v);
        let mode = TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"));
        let setup = RunSetup {
            schema_version: 0,
            sim_version: 0,
            mode: "time_limit".into(),
            weapon: "giant_sword".into(),
            seed,
            star_pool: None,
            extra_rerolls: 0,
            upgrades: Vec::new(),
            perk_set: "duelist".into(),
            rune_pool: Vec::new(),
            carried: Vec::new(),
            lumine: 0,
            starting_rune: None,
            difficulty: "normal".into(),
        };
        let mut e = Engine::start_time_limit(&setup, RunContent::bundled(), &mode).unwrap();
        e.state.player.hp = u32::MAX;
        e.state.player.max_hp = u32::MAX;
        e
    }

    /// Steps to `until` and lists each elite spawned: the tick, the sprite.
    fn elites_spawned(e: &mut Engine, until: u32) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        while e.tick() < until {
            e.step(Input::default());
            for ev in e.events() {
                if let DomainEvent::EliteSpawned { asset, .. } = ev {
                    out.push((e.tick(), asset.0.clone()));
                }
            }
        }
        out
    }

    #[test]
    fn elite_minutes_land_on_each_whole_minute_but_boss_ticks_and_overtime() {
        // Room for all sixteen: the cap is not what is tested here.
        let mut e = run_of(1, |v| v["elite_cap"] = serde_json::json!(32));
        let ticks: Vec<u32> = elites_spawned(&mut e, 72_000 + 3_600 * 2)
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        let expected: Vec<u32> = (1..=19)
            .map(|k| k * 3_600)
            .filter(|t| ![18_000, 36_000, 54_000].contains(t))
            .collect();
        assert_eq!(expected.len(), 16);
        assert_eq!(ticks, expected);
    }

    #[test]
    fn a_tier_boundary_on_an_elite_minute_picks_the_later_tier() {
        let mut e = run_of(1, |v| {
            v["length_ticks"] = serde_json::json!(18_000);
            v["boss_ticks"] = serde_json::json!([18_000]);
            v["mid_boss_pool"] = serde_json::json!([]);
            v["elite_cap"] = serde_json::json!(32);
            v["elite_table"] = serde_json::json!([
                { "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] },
                { "start_tick": 10_800, "weights": [{ "elite": "elite_mid", "weight": 1 }] }
            ]);
        });
        let spawned = elites_spawned(&mut e, 14_400);
        let sprites: Vec<(u32, &str)> = spawned.iter().map(|(t, a)| (*t, a.as_str())).collect();
        assert_eq!(
            sprites,
            [
                (3_600, "bat"),
                (7_200, "bat"),
                (10_800, "fairy_a"),
                (14_400, "fairy_a")
            ]
        );
    }

    #[test]
    fn the_draws_are_those_of_a_table_of_the_same_shape_before_types_were_content() {
        // Recorded before elite types became content: same weights, same seed. The positions are
        // those after the boss's call counts from its arrival (ADR-103); the draws are the same.
        let table = serde_json::json!([{ "start_tick": 0, "weights": [
            { "elite": "elite_low", "weight": 1 }, { "elite": "elite_mid", "weight": 2 } ] },
            { "start_tick": 10_800, "weights": [
            { "elite": "elite_low", "weight": 1 }, { "elite": "elite_high", "weight": 1 }] }]);
        let mut e = run_of(7, |v| {
            v["elite_cap"] = serde_json::json!(32);
            v["elite_table"] = table.clone();
        });
        let spawned = elites_spawned(&mut e, 5 * 3_600 + 10);
        let sprites: Vec<(u32, &str)> = spawned.iter().map(|(t, a)| (*t, a.as_str())).collect();
        assert_eq!(
            sprites,
            [
                (3_600, "bat"),
                (7_200, "bat"),
                (10_800, "bat"),
                (14_400, "bat")
            ]
        );
        assert_eq!(e.state.rng.state(), 10_372_713_005_361_028_292);
        let at: Vec<(i32, i32)> = e
            .state
            .enemies
            .iter()
            .map(|a| (a.at.x.0, a.at.y.0))
            .collect();
        assert_eq!(
            at,
            [
                (430_742, 482_638),
                (468_518, 610_167),
                (623_198, 497_785),
                (502_998, 624_450)
            ]
        );
    }

    #[test]
    fn each_grade_is_spawned_from_its_own_definition() {
        let mode = TimeLimit::bundled();
        for (id, sprite) in [
            ("elite_low", "bat"),
            ("elite_mid", "fairy_a"),
            ("elite_high", "horns_2"),
        ] {
            let def = mode.elite_types().iter().find(|t| t.id.0 == id).unwrap();
            let mut e = run_of(1, |v| {
                v["elite_table"] =
                    serde_json::json!([{"start_tick": 0, "weights": [{"elite": id, "weight": 1}]}]);
            });
            let spawned = elites_spawned(&mut e, 3_600);
            assert_eq!(spawned, [(3_600, sprite.to_string())], "{id}");
            let elites: Vec<_> = e
                .state
                .enemies
                .iter()
                .filter(|a| a.elite.is_some())
                .collect();
            assert_eq!(elites.len(), 1);
            assert_eq!(
                elites[0].hp, def.enemy.hp,
                "{id}'s health is its definition's"
            );
        }
        let hp = |id: &str| {
            mode.elite_types()
                .iter()
                .find(|t| t.id.0 == id)
                .unwrap()
                .enemy
                .hp
        };
        assert!(hp("elite_low") < hp("elite_mid") && hp("elite_mid") < hp("elite_high"));
    }

    #[test]
    fn the_cap_holds_an_elite_back_but_the_generator_is_drawn_the_same() {
        let table = serde_json::json!([{ "start_tick": 0, "weights": [
            { "elite": "elite_low", "weight": 1 }, { "elite": "elite_mid", "weight": 1 } ] }]);
        let run = |cap: u32| {
            run_of(7, |v| {
                v["elite_cap"] = serde_json::json!(cap);
                v["elite_table"] = table.clone();
            })
        };
        let (mut tight, mut roomy) = (run(1), run(32));
        let (mut few, mut many) = (0, 0);
        for minute in 1..=4 {
            let at = minute * 3_600;
            few += elites_spawned(&mut tight, at).len();
            many += elites_spawned(&mut roomy, at).len();
            assert_eq!(
                tight.state.rng.state(),
                roomy.state.rng.state(),
                "the generator after minute {minute}"
            );
        }
        assert_eq!((few, many), (1, 4), "the cap held three back");
        let alive = |e: &Engine| e.state.enemies.iter().filter(|a| a.elite.is_some()).count();
        assert_eq!((alive(&tight), alive(&roomy)), (1, 4));
    }

    #[test]
    fn the_boss_calls_the_elites_of_the_minutes_before_and_after_it_comes() {
        let mut e = run_of(3, |v| {
            v["length_ticks"] = serde_json::json!(20_000);
            v["boss_ticks"] = serde_json::json!([4_000, 20_000]);
            v["mid_boss_pool"] = serde_json::json!(["golem"]);
            v["elite_cap"] = serde_json::json!(32);
            v["elite_table"] = serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
        });
        let mut called = Vec::new();
        while e.tick() < 7_200 {
            e.step(Input::default());
            for ev in e.events() {
                if let DomainEvent::ElitesCalled { elites } = ev {
                    called.push((e.tick(), *elites));
                }
            }
        }
        // The elite of minute 1 is alive when the boss comes at 4000, and is called when the
        // boss's 1800 ticks from its arrival are up (ADR-103).
        assert_eq!(called, [(5_800, 1)]);
        // The elite of minute 2 spawned after the call: called at once.
        let elites: Vec<bool> = e
            .state
            .enemies
            .iter()
            .filter_map(|a| a.elite.map(|el| el.called))
            .collect();
        assert_eq!(elites, [true, true]);
    }

    /// A bundled `time_limit` run on `difficulty`, the main character alive (`arrivals_and_calls`
    /// keeps it so; `boss_sprite_at` does too).
    fn bundled_run(difficulty: &str, seed: u64) -> Engine {
        let setup = RunSetup {
            schema_version: 0,
            sim_version: 0,
            mode: "time_limit".into(),
            weapon: "giant_sword".into(),
            seed,
            star_pool: None,
            extra_rerolls: 0,
            upgrades: Vec::new(),
            perk_set: "duelist".into(),
            rune_pool: Vec::new(),
            carried: Vec::new(),
            lumine: 0,
            starting_rune: None,
            difficulty: difficulty.into(),
        };
        let mode = TimeLimit::bundled();
        Engine::start_time_limit(&setup, RunContent::bundled(), mode).unwrap()
    }

    /// The sprite of the boss on the field once the run has reached `tick`.
    fn boss_sprite_at(e: &mut Engine, tick: u32) -> Option<String> {
        while e.tick() < tick {
            e.state.player.hp = e.state.player.max_hp;
            e.step(Input::default());
        }
        e.snapshot()
            .entities
            .iter()
            .find(|v| v.kind == crate::snapshot::EntityKind::Boss)
            .map(|v| v.asset.0.clone())
    }

    fn assert_three_distinct_mid_bosses(difficulty: &str, times: [u32; 3]) {
        const POOL: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];
        for seed in 0..4 {
            let mut e = bundled_run(difficulty, seed);
            let drawn: Vec<String> = times
                .iter()
                .map(|&t| boss_sprite_at(&mut e, t).expect("a boss"))
                .collect();
            let mut unique = drawn.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), 3, "seed {seed}: {drawn:?}");
            assert!(
                drawn.iter().all(|d| POOL.contains(&d.as_str())),
                "{drawn:?}"
            );
        }
    }

    #[test]
    fn the_20_minute_run_draws_three_distinct_mid_bosses_from_the_four() {
        assert_three_distinct_mid_bosses("normal", [18_000, 36_000, 54_000]);
    }

    #[test]
    fn the_easy_run_draws_one_mid_boss_from_the_four_and_then_the_final_boss() {
        const POOL: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];
        for seed in 0..2 {
            let mut e = bundled_run("easy", seed);
            let horde = e.state.horde.as_ref().unwrap();
            assert_eq!(horde.boss_ticks, [18_000, 36_000], "seed {seed}");
            let drawn = boss_sprite_at(&mut e, 18_000).expect("a boss");
            assert!(POOL.contains(&drawn.as_str()), "seed {seed}: {drawn}");
        }
    }

    #[test]
    fn each_mid_boss_s_hit_points_rise_by_its_slot_and_the_final_boss_s_do_not() {
        // Normal difficulty scales nothing; the slots are 100%, 130% and 160%.
        let mut e = engine(&[50, 100, 150, 200]);
        let mode = TimeLimit::bundled();
        let mut seen = Vec::new();
        for (t, pct) in [(50, 100), (100, 130), (150, 160), (200, 100)] {
            run_to(&mut e, t);
            let boss = e.state.boss.as_ref().unwrap();
            let def = mode
                .bosses()
                .iter()
                .find(|b| b.base == e.stage.boss.as_ref().unwrap().base)
                .unwrap();
            assert_eq!(boss.max_hp, def.hp * pct / 100, "at {t}");
            let parts: Vec<u32> = boss.parts.iter().map(|p| p.hp).collect();
            let want: Vec<u32> = def.parts.iter().map(|p| p.hp * pct / 100).collect();
            assert_eq!(parts, want, "at {t}");
            seen.push(def.base.0.clone());
        }
        assert_eq!(seen[3], "core_pair");
    }

    #[test]
    fn the_five_minute_boss_calls_no_elites_and_the_later_ones_call_after_30_seconds() {
        // The 5-minute boss comes at 18000 and calls nothing; the 10-minute one at 36000 calls
        // 1800 ticks after it came, and so does the 15-minute one at 54000.
        for seed in 0..2 {
            let mut e = bundled_run("normal", seed);
            let (arrived, called) = arrivals_and_calls(&mut e, 55_800);
            assert_eq!(arrived, [18_000, 36_000, 54_000], "seed {seed}");
            assert_eq!(called, [37_800, 55_800], "seed {seed}");
        }
    }
}

#[cfg(test)]
mod move_index_tests {
    use super::*;
    use crate::attack::PhaseKind;

    fn phase(move_pct: u8) -> AttackPhase {
        AttackPhase {
            kind: PhaseKind::Active,
            ticks: 5,
            hit_box: None,
            damage: None,
            damage_type: None,
            rehit_ticks: None,
            spawn: Vec::new(),
            move_pct,
            breaks_charge: false,
        }
    }

    /// A run of `owner` whose phases have these `move_pct`s, on its `phase`.
    fn run(owner: u32, phase_at: u32, pcts: &[u8]) -> AttackRun {
        AttackRun {
            owner: EntityId(owner),
            slot: 0,
            preset: None,
            phases: Arc::new(pcts.iter().map(|p| phase(*p)).collect()),
            phase: phase_at,
            tick: 0,
            facing: Dir::DOWN,
            aim: Aim::Fixed(Dir::DOWN),
            friendly: false,
            base: 1,
            atk_up_pct: 0,
            damage_pct: 100,
            type_bonus_pct: None,
            age: 0,
            hits: Vec::new(),
            smash: None,
            at: None,
            throw: None,
            homing: None,
        }
    }

    /// The scan the index replaces, kept as the oracle.
    fn scan(runs: &[AttackRun], owner: EntityId) -> i64 {
        runs.iter()
            .filter(|r| r.owner == owner)
            .map(|r| i64::from(r.phases[r.phase as usize].move_pct))
            .min()
            .unwrap_or(100)
    }

    fn assert_same(index: &MoveIndex, runs: &[AttackRun]) {
        for id in 0..8 {
            assert_eq!(
                index.get(EntityId(id)),
                scan(runs, EntityId(id)),
                "owner {id}"
            );
        }
    }

    #[test]
    fn an_owner_with_several_runs_gets_the_lowest_move_pct() {
        let runs = vec![
            run(3, 0, &[80, 10]),
            run(5, 1, &[0, 100]),
            run(3, 1, &[100, 40]),
            run(3, 0, &[90]),
            run(1, 0, &[0]),
        ];
        let mut index = MoveIndex::default();
        index.rebuild(&runs);
        assert_same(&index, &runs);
        assert_eq!(index.get(EntityId(3)), 40);
        assert_eq!(index.get(EntityId(1)), 0);
        assert_eq!(index.get(EntityId(2)), 100);
    }

    #[test]
    fn lowering_for_a_new_run_matches_a_rebuild() {
        let mut runs = vec![run(4, 0, &[70]), run(2, 0, &[100])];
        let mut index = MoveIndex::default();
        index.rebuild(&runs);
        for (owner, pcts) in [(4, [30u8]), (6, [100]), (2, [55]), (6, [0]), (0, [90])] {
            let r = run(owner, 0, &pcts);
            index.lower(r.owner, i64::from(r.phases[0].move_pct));
            runs.push(r);
            assert_same(&index, &runs);
        }
        let mut rebuilt = MoveIndex::default();
        rebuilt.rebuild(&runs);
        assert_same(&rebuilt, &runs);
    }
}
