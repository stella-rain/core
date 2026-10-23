//! The snapshot builder: what the frontend draws after the last tick (ADR-005).

use crate::attack::{Anchor, DamageType, PhaseKind};
use crate::behaviour::Movement;
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::rewards::MAX_BURST_DEPTH;
use crate::rules_v0;
use crate::snapshot::{
    AttackView, BulletView, ChargeView, ComboView, Countdown, DamageOverTimeView, EntityKind,
    EntityView, EvadeView, ExpertiseView, Facing, HealPickupView, HoldView, MapView, OrbitView,
    PlayerView, ProgressView, PullView, RuneView, ShieldView, SkillView, Snapshot, StarView,
    StatusView, TypeBonusView, ZoneView,
};

use super::{AttackRun, Engine, ShrineState, StatusState};

impl Engine {
    /// What the frontend draws after the last tick (ADR-005).
    pub fn snapshot(&self) -> Snapshot {
        let s = &self.state;
        let view = |r: &AttackRun| {
            let p = rules_v0::phase(&self.stage, r, r.phase as usize);
            AttackView {
                slot: r.slot,
                preset: self.stage.attacks[usize::from(r.attack.0)].preset.clone(),
                phase: u8::try_from(r.phase).unwrap_or(u8::MAX),
                kind: p.kind,
                tick: r.tick,
                ticks: p.ticks,
                hit_box: p.hit_box().cloned(),
                facing: Facing {
                    x: r.facing.x,
                    y: r.facing.y,
                },
                at: r.at.or_else(|| {
                    p.hit_box()
                        .filter(|h| h.anchor() == Anchor::Target)
                        .and(r.spot)
                }),
            }
        };
        let attack_of = |id: EntityId| s.runs.iter().find(|r| r.owner == id).map(view);
        // The auras of the elites, read once: an enemy within reach of one is strengthened.
        let auras: Vec<_> = rules_v0::aura_sources(&self.stage, s).collect();
        let mut entities = Vec::new();
        if let Some((boss, b)) = s.boss.as_ref().map(|b| (&self.stage.boss(b.boss).boss, b)) {
            if b.hp > 0 {
                entities.push(EntityView {
                    id: b.id,
                    kind: EntityKind::Boss,
                    asset: boss.base.clone(),
                    kind_index: Some(b.kind),
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
                    let def = self.stage.defs.agent(a.def);
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
                        kind_index: (!hidden).then_some(a.kind),
                        palette: def.palette.clone(),
                        at: a.at,
                        radius: def.radius,
                        hp: a.hp,
                        attack: attack_of(a.id),
                        disguised: hidden,
                        elite: a.shows_elite(),
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
        if let Some((boss, b)) = s.boss.as_ref().map(|b| (&self.stage.boss(b.boss).boss, b)) {
            for (part, def) in b.parts.iter().zip(&boss.parts) {
                if b.hp > 0 && part.hp > 0 {
                    entities.push(EntityView {
                        id: part.id,
                        kind: EntityKind::BossPart,
                        asset: ContentId(def.asset.0.clone()),
                        kind_index: Some(part.kind),
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
                            reward: self.content.rewards().orbit_id(o.reward)?,
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
                burst_depth: s
                    .player
                    .burst_depth
                    .saturating_add(s.player.burst_depth_extra)
                    .min(MAX_BURST_DEPTH),
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
            zones: s
                .zones
                .iter()
                .map(|z| ZoneView {
                    shape: z.shape.clone(),
                    at: z.at,
                    direction: Facing {
                        x: z.dir.x,
                        y: z.dir.y,
                    },
                    phase: if z.wind_left > 0 {
                        PhaseKind::WindUp
                    } else {
                        PhaseKind::Active
                    },
                    ticks_left: if z.wind_left > 0 {
                        z.wind_left
                    } else {
                        z.active_left
                    },
                    friendly: z.friendly,
                })
                .collect(),
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
