//! The snapshot view of this version's state: what the frontend draws after the last tick
//! (ADR-005), and where the drawn things are (`places`). It reads this version's `World` and
//! `State` only, so a copy of the rules brings its own view (ADR-REPL-7 point 4).

use crate::sim::state::PlayerIdx;
use std::sync::LazyLock;

use crate::attack::{Anchor, DamageType, PhaseKind};
use crate::behaviour::Movement;
use crate::engine::{Place, Places};
use crate::fixed::{Fx, Point};
use crate::id::{ContentId, EntityId};
use crate::rewards::MAX_BURST_DEPTH;
use crate::run_content::RunContent;
use crate::snapshot::{
    AttackView, ChargeView, ComboView, Countdown, DamageOverTimeView, EntityKind, EntityView,
    EvadeView, ExpertiseView, Facing, HealPickupView, HoldView, MapView, OrbView, OrbitView,
    PlayerView, ProgressView, PullView, RuneView, ShieldView, SkillView, Snapshot, StarView,
    StatusView, TypeBonusView, ZoneView,
};

use super::state::{AgentState, AttackRun, ShrineState, State, StatusState};
use super::{
    World, announced_patterns, aura_percent, aura_sources, auto_charge_view, beam_to, charge_view,
    combo_view, constellation_views, expertise_offer_view, expertise_styles_view, hold_view,
    offer_view, phase, stat_offer_view, xp_next,
};

/// Clears `out` and fills it for the state after the last tick: what `snapshot()` shows for
/// these fields (and the player's position), without building a snapshot. Allocates only
/// while `out` grows to the largest crowd it has held. Keeps the visibility, position and kind rules of `snapshot()`
/// (which names this function); `tests/places.rs` holds the two equal.
pub(crate) fn places(s: &State, out: &mut Places) {
    // shim: the snapshot is of the first player until it is made per player.
    let me = PlayerIdx::SOLO;
    out.entities.clear();
    out.stars.clear();
    out.boss = None;
    out.player = s.players[me].at;
    // Boss, companions, boss parts, then enemies: the order of their entity IDs.
    let alive_boss = s.boss.as_ref().filter(|b| b.hp > 0);
    if let Some(b) = alive_boss {
        out.boss = Some(b.id);
        out.entities.push(Place {
            id: b.id,
            kind: EntityKind::Boss,
            at: b.at,
        });
    }
    let agents = |kind, agents: &[AgentState], out: &mut Vec<Place>| {
        out.extend(agents.iter().map(|a| Place {
            id: a.id,
            kind: a.entity_kind(kind),
            at: a.at,
        }));
    };
    agents(EntityKind::Companion, &s.companions, &mut out.entities);
    if let Some(b) = alive_boss {
        for part in b.parts.iter().filter(|p| p.hp > 0) {
            out.entities.push(Place {
                id: part.id,
                kind: EntityKind::BossPart,
                at: Point {
                    x: b.at.x.saturating_add(part.offset.x),
                    y: b.at.y.saturating_add(part.offset.y),
                },
            });
        }
    }
    agents(EntityKind::Enemy, &s.enemies, &mut out.entities);
    if let Some(f) = s.stars.as_ref() {
        out.stars.extend(f.on_map.iter().map(|st| (st.id, st.at)));
    }
}

/// What the frontend draws after the last tick (ADR-005). `places` repeats its rules for
/// the ids, kinds and positions, and the player's position: change them together.
pub(crate) fn build(world: &World, content: &RunContent, s: &State) -> Snapshot {
    // shim: the snapshot is of the first player until it is made per player.
    let me = PlayerIdx::SOLO;
    let view = |r: &AttackRun| {
        let p = phase(world, r, r.phase as usize);
        AttackView {
            slot: r.slot,
            preset: world.attacks[usize::from(r.attack.0)].preset.clone(),
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
    let auras: Vec<_> = aura_sources(world, s).collect();
    let mut entities = Vec::new();
    if let Some((boss, b)) = s.boss.as_ref().map(|b| (&world.boss(b.boss).boss, b)) {
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
    let agent_views = |kind, agents: &[AgentState], out: &mut Vec<EntityView>| {
        for a in agents {
            let def = world.defs.agent(a.def);
            let hidden = a.hidden();
            // Only a non-elite enemy is strengthened by an aura (ADR-090).
            let reached = kind == EntityKind::Enemy
                && a.elite.is_none()
                && aura_percent(auras.iter().copied(), a.at) > 0;
            out.push(EntityView {
                id: a.id,
                kind: a.entity_kind(kind),
                asset: match def.elite.as_ref().and_then(|e| e.mimic.as_ref()) {
                    Some(m) if hidden => m.disguise.clone(),
                    _ => def.base.clone(),
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
    if let Some((boss, b)) = s.boss.as_ref().map(|b| (&world.boss(b.boss).boss, b)) {
        for (part, def) in b.parts.iter().zip(&boss.parts) {
            if b.hp > 0 && part.hp > 0 {
                entities.push(EntityView {
                    id: part.id,
                    kind: EntityKind::BossPart,
                    asset: def.asset.clone(),
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
    let map = world.map();
    Snapshot {
        tick: s.tick,
        map: MapView {
            width: map.width,
            height: map.height,
        },
        player: PlayerView {
            at: s.players[me].at,
            hp: s.players[me].hp,
            radius: Fx(world.tuning.player_radius),
            facing: Facing {
                x: s.players[me].facing.x,
                y: s.players[me].facing.y,
            },
            invulnerable: s.players[me].invulnerable,
            evade: EvadeView {
                charges: s.players[me].evade_charges,
                ready: s.players[me].evade_ready(),
                recovering: s.players[me].evade_recovering.clone(),
                dash_left: s.players[me].dash.map_or(0, |d| d.left),
            },
            combo: {
                let (presses, dash, buffered, window_left, auto) = combo_view(s, me);
                ComboView {
                    presses,
                    dash,
                    buffered,
                    window_left,
                    auto,
                }
            },
            charge: charge_view(world, s, me).map(|(level, held, perfect)| ChargeView {
                level,
                held,
                perfect,
            }),
            auto_charge: auto_charge_view(s, me),
            hold: hold_view(world, s, me).map(|(held, max_ticks)| HoldView { held, max_ticks }),
            target: s.players[me].target,
            skill: s.players[me].skill.and_then(|slot| {
                let def = content.stage_skills().get(usize::from(slot.index))?;
                Some(SkillView {
                    skill: def.id.clone(),
                    icon: def.icon.clone(),
                    level: slot.level,
                    ready: slot.cooldown_left == 0,
                    cooldown_left: slot.cooldown_left,
                })
            }),
            pull: s.players[me].pull.map(|b| PullView {
                at: b.at,
                radius: Fx(b.radius),
                ticks_left: b.ticks_left,
            }),
            shield: s.players[me].shield.map(|sh| ShieldView {
                amount: sh.amount,
                ticks_left: sh.ticks_left,
            }),
            orbits: s.players[me]
                .orbits
                .iter()
                .filter_map(|o| {
                    Some(OrbitView {
                        reward: content.rewards().orbit_id(o.reward)?,
                        orbs: o.orbs,
                        cooldown_left: o.cooldown_left,
                    })
                })
                .collect(),
            range_pct: s.players[me].range_pct,
            rune_haste_pct: s.players[me].rune_haste_pct,
            pickup_radius_pct: s.players[me].pickup_radius_pct,
            skill_cooldown_pct: s.players[me].skill_cooldown_pct,
            kill_heal: s.players[me].kill_heal,
            kill_heal_every: s.players[me].kill_heal_every,
            crit_chance_pct: s.players[me].crit_chance_pct,
            crit_damage_pct: s.players[me].crit_damage_pct,
            echo_hit_pct: s.players[me].echo_hit_pct,
            burst_pct: s.players[me].burst_pct,
            burst_radius: s.players[me].burst_radius,
            burst_depth: s.players[me]
                .burst_depth
                .saturating_add(s.players[me].burst_depth_extra)
                .min(MAX_BURST_DEPTH),
            type_bonuses: DamageType::ALL
                .iter()
                .zip(s.players[me].type_bonus_pct)
                .filter(|(_, pct)| *pct > 0)
                .map(|(ty, pct)| TypeBonusView {
                    damage_type: *ty,
                    pct,
                })
                .collect(),
            runes: s.players[me]
                .runes
                .iter()
                .map(|r| RuneView {
                    rune: r.rune,
                    cooldown_left: r.cooldown_left,
                })
                .collect(),
            attacks: {
                let mut runs: Vec<&AttackRun> =
                    s.runs.iter().filter(|r| r.owner == me.id()).collect();
                runs.sort_by_key(|r| r.slot);
                runs.into_iter().map(view).collect()
            },
            statuses: status_views(&s.players[me].statuses, false),
        },
        entities,
        zones: s
            .zones
            .iter()
            .map(|z| {
                let (phase, ticks_left, ticks) = if z.wind_left > 0 {
                    (PhaseKind::WindUp, z.wind_left, z.wind_ticks)
                } else {
                    (PhaseKind::Active, z.active_left, z.active_ticks)
                };
                ZoneView {
                    shape: z.shape.clone(),
                    at: z.at,
                    direction: Facing {
                        x: z.dir.x,
                        y: z.dir.y,
                    },
                    phase,
                    ticks_left,
                    ticks,
                    friendly: z.friendly,
                    warn_only: z.warn_only,
                }
            })
            .collect(),
        orbs: s
            .orbs
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let beam = beam_to(&s.orbs, i);
                OrbView {
                    at: b.at,
                    vx: b.vx,
                    vy: b.vy,
                    radius: Fx(world.tuning.orb_radius),
                    style: b.style,
                    breakable: b.breakable,
                    link_to: beam.and_then(|j| u32::try_from(j).ok()),
                    link_width: beam.and_then(|_| s.orbs[i].link.map(|l| l.width)),
                }
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
        constellations: constellation_views(content.rewards(), s),
        heal_pickups: s.heal_pickups.as_ref().map_or_else(Vec::new, |h| {
            h.iter()
                .map(|p| HealPickupView { id: p.id, at: p.at })
                .collect()
        }),
        progress: s.progress.as_ref().map(|p| ProgressView {
            level: p.level,
            xp: p.xp,
            xp_next: xp_next(world, p.level),
            spent: p.spent.clone(),
            stat_offer: stat_offer_view(s),
            offer: offer_view(content.rewards(), s),
        }),
        expertise: s.expertise.as_ref().map(|g| {
            let (style, path) = expertise_styles_view(world, content.expertise(), g);
            ExpertiseView {
                level: g.level,
                points: g.points,
                next: g.next,
                taken: g.taken.clone(),
                style,
                path,
                offer: expertise_offer_view(world, content.expertise(), s),
            }
        }),
        patterns: announced_patterns(world, s),
        stardust: s.stardust,
    }
}

/// The damage-over-time statuses of `statuses`, for the snapshot.
fn dot_views(statuses: &[StatusState]) -> Vec<DamageOverTimeView> {
    statuses
        .iter()
        .filter_map(|st| {
            st.dot.map(|d| DamageOverTimeView {
                status: st.kind.id(),
                stacks: st.stacks,
                ticks_left: st.remaining,
                next_tick_in: d.next_in,
            })
        })
        .collect()
}

/// The ID of a pack leader's aura, shared like the statuses' (`StatusKind::id`).
static AURA_ID: LazyLock<ContentId> = LazyLock::new(|| ContentId::from("aura"));

/// The statuses of `statuses` for the snapshot (ADR-103 decision 3), sorted by ID: each with
/// what is left of it and the ticks it was applied for. `aura` adds a pack leader's aura,
/// which has no timer: it lasts while its entity is in reach.
fn status_views(statuses: &[StatusState], aura: bool) -> Vec<StatusView> {
    let mut views: Vec<StatusView> = statuses
        .iter()
        .map(|st| StatusView {
            id: st.kind.id(),
            remaining: st.remaining,
            total: st.total,
        })
        .collect();
    if aura {
        views.push(StatusView {
            id: AURA_ID.clone(),
            remaining: 0,
            total: 0,
        });
    }
    views.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    views
}
