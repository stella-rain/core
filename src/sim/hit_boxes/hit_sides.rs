//! What a landed swing does to each target, in the hit-box pass.

use crate::event::{DomainEvent, HitKind};
use crate::id::EntityId;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{Smash, State};
use crate::sim::world::AttackIndex;

use super::{Source, Swing, covers};
use crate::sim::World;
use crate::sim::entity::Who;
use crate::sim::grid::Grid;
use crate::sim::{Blame, Culprit, Shot, damage, hurt_player};

/// The targets the swing has hit, by ID.
pub(super) fn hits_of<'a>(s: &'a State, w: &Swing) -> &'a [(EntityId, u32)] {
    match w.source {
        Source::Run(i) => &s.runs[i].hits,
        Source::Zone(i) => &s.zones[i].hits,
    }
}

/// Whether the swing may hit `target` now, and if so, counts the hit.
fn take_hit(s: &mut State, w: &Swing, target: EntityId) -> bool {
    let tick = s.tick;
    let hits = match w.source {
        Source::Run(i) => &mut s.runs[i].hits,
        Source::Zone(i) => &mut s.zones[i].hits,
    };
    if w.once && !hits.is_empty() {
        return false;
    }
    match hits.binary_search_by_key(&target.0, |h| h.0.0) {
        Ok(i) => {
            let again = w
                .rehit_ticks
                .is_some_and(|k| tick >= hits[i].1.saturating_add(k));
            if again {
                hits[i].1 = tick;
            }
            again
        }
        Err(i) => {
            // The first hit of a run or a zone takes a list that has a room already, if one is
            // waiting: only the runs and zones that hit hold one, so the pool settles at the
            // most that hit at once and a steady tick allocates nothing (T1b).
            if hits.capacity() == 0 {
                *hits = s.scratch.take_hit_list();
            }
            hits.insert(i, (target, tick));
            true
        }
    }
}

fn shot(w: &Swing) -> Shot {
    Shot {
        at: w.area.centre,
        owner: w.owner,
        friendly: w.friendly,
        damage: w.damage,
        damage_type: w.damage_type,
        breaks_charge: w.breaks_charge,
        smash: w.smash,
        combo: w.combo,
        rune: w.rune,
        chain: false,
        dot: false,
    }
}

/// The boss, its parts and the enemies, in entity-ID order. An invulnerable boss shields its
/// parts, as it does from orbs.
pub(super) fn hit_enemy_side(
    stage: &World,
    s: &mut State,
    grid: &Grid,
    w: &Swing,
    (bits, found): (&mut Vec<u64>, &mut Vec<usize>),
    events: &mut Vec<DomainEvent>,
) {
    // The player who made the swing; a companion's swing is the run's first player's (the stage's
    // companions belong to no player).
    let me = super::hitter(s, w.owner);
    if let Some((def, boss)) = s.boss_pair(stage) {
        if boss.invulnerable == 0 {
            let (id, at, radius) = (boss.id, boss.at, def.radius.0);
            if boss.hp > 0 && covers(w, at, radius) && take_hit(s, w, id) {
                let c = counterable(stage, s, me, w, id);
                damage(stage, s, me, shot(w), Who::Boss, events);
                super::stun::counter_bonus_stun(
                    stage,
                    s,
                    me,
                    (Who::Boss, w.combo),
                    c,
                    w.damage,
                    events,
                );
                if stage.has_stagger {
                    stagger_by(stage, s, me, w, Who::Boss, events);
                    resisted_move_fills(stage, s, me, w, Who::Boss, events);
                }
            }
            let parts = s.boss.as_ref().map_or(0, |b| b.parts.len());
            for i in 0..parts {
                let Some(part) = s.boss.as_ref().map(|b| &b.parts[i]) else {
                    break;
                };
                let (id, hp, radius) = (part.id, part.hp, part.radius);
                if hp > 0 && covers(w, s.at(Who::BossPart(i)), radius) && take_hit(s, w, id) {
                    let owner = s.boss_state().id;
                    let c = counterable(stage, s, me, w, owner);
                    // The breaker's factor on a smash step's hit on a part.
                    let mut part_shot = shot(w);
                    part_shot.damage =
                        super::player::breaker_part(stage, s, me, w.combo, part_shot.damage);
                    damage(stage, s, me, part_shot, Who::BossPart(i), events);
                    super::stun::counter_bonus_stun(
                        stage,
                        s,
                        me,
                        (Who::BossPart(i), w.combo),
                        c,
                        w.damage,
                        events,
                    );
                    if stage.has_stagger {
                        stagger_by(stage, s, me, w, Who::BossPart(i), events);
                        resisted_move_fills(stage, s, me, w, Who::BossPart(i), events);
                    }
                }
            }
        }
    }
    grid.within(w.area.centre, w.area.radius, bits, found);
    // A swing that never hits a target again (a zone, or a hit box with no `rehit_ticks`) passes
    // over the ones it has hit with nothing else to find out, so those are told from its hit
    // list, sorted by ID like `found`, with a cursor that only moves forward. A cursor that has
    // gone past a target only sends it down the full path, which decides alone.
    let hit_once = w.rehit_ticks.is_none() && !w.once;
    let mut seen = 0;
    for &i in found.iter() {
        let e = &s.enemies[i];
        let (id, at, hp) = (e.id, e.at, e.hp);
        if hit_once {
            let hits = hits_of(s, w);
            while seen < hits.len() && hits[seen].0.0 < id.0 {
                seen += 1;
            }
            if hits.get(seen).is_some_and(|h| h.0 == id) {
                continue;
            }
        }
        let radius = stage.defs.agent(e.def).radius.0;
        // A shrine is not hurt, nor a mimic in disguise: the hit passes through them.
        if hp > 0
            && e.shrine.is_none()
            && !e.hidden()
            && covers(w, at, radius)
            && take_hit(s, w, id)
        {
            let c = counterable(stage, s, me, w, id);
            damage(stage, s, me, shot(w), Who::Enemy(i), events);
            match w.smash {
                Some(Smash::Pull(pull)) => super::pull_enemy(stage, s, me, i, pull),
                Some(Smash::Knockback(push)) => {
                    super::knock_enemy(stage, s, me, i, push);
                    // C09: the knockback also knocks a regular enemy down, when the rules name it.
                    if stage.has_down {
                        super::stun::knock_down(stage, s, me, i, events);
                    }
                }
                _ => {}
            }
            // C05: the smash that hit it in a counterable wind-up counters the attack, after the
            // smash's own stun, pull and knockback have landed.
            super::stun::counter_bonus_stun(
                stage,
                s,
                me,
                (Who::Enemy(i), w.combo),
                c,
                w.damage,
                events,
            );
            // C03, C04: a combo hit fills an elite's gauge, and a pull or a knockback it resists
            // fills it more.
            if stage.has_stagger {
                stagger_by(stage, s, me, w, Who::Enemy(i), events);
                resisted_move_fills(stage, s, me, w, Who::Enemy(i), events);
            }
            // C02: a combo hit that deals the enemy's poise makes it flinch; a smash's own stun
            // (applied inside `damage`) has already won.
            if w.combo
                && stage.tuning.flinch_ticks > 0
                && stage.defs.agent(s.enemies[i].def).poise.is_some()
            {
                flinch_by(stage, s, me, i, w, events);
            }
        }
    }
}

/// C05: the attack of `owner` that a combo hit of `me`'s counters, if it does: the rules flag some
/// phase, the swing is a combo hit of a smash step of an up player (not a rune's, an echo's or a
/// zone's), and `owner` is in a counterable wind-up. Read before the hit, so that a target the
/// hit kills is not countered. One flag read when no phase is flagged.
#[inline(always)]
fn counterable(
    stage: &World,
    s: &State,
    me: PlayerIdx,
    w: &Swing,
    owner: EntityId,
) -> Option<AttackIndex> {
    if !stage.has_counter {
        return None;
    }
    counterable_slow(stage, s, me, w, owner)
}

#[inline(never)]
fn counterable_slow(
    stage: &World,
    s: &State,
    me: PlayerIdx,
    w: &Swing,
    owner: EntityId,
) -> Option<AttackIndex> {
    if w.combo
        && w.rune.is_none()
        && s.players[me].is_up()
        && super::player::last_press_smash(&s.players[me].combo)
    {
        super::attacks::counterable_run(stage, s, owner)
    } else {
        None
    }
}

/// C03: a combo hit of `me`'s adds the stagger its phase names, else the rules' for a smash step,
/// to the gauge of `target` (the boss, one of its parts or an elite), when it is still alive.
/// Rune hits, echoes, bursts, damage-over-time ticks, orbs and zones are no combo hit.
fn stagger_by(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    w: &Swing,
    target: Who,
    events: &mut Vec<DomainEvent>,
) {
    // A part that this hit broke still fills its boss's gauge: the fill tests the boss alive.
    let alive = matches!(target, Who::BossPart(_)) || s.is_alive(target);
    if !stage.has_stagger || !w.combo || !alive {
        return;
    }
    let value = w
        .stagger
        .unwrap_or(if super::player::last_press_smash(&s.players[me].combo) {
            stage.tuning.stagger_smash
        } else {
            0
        });
    super::stun::fill(stage, s, target, me, value, w.damage, events);
}

/// C04: the pull or the knockback of a smash that hit `target`, an elite, the boss or a part,
/// which resists it whole, fills its gauge by the rules' value of that kind.
fn resisted_move_fills(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    w: &Swing,
    target: Who,
    events: &mut Vec<DomainEvent>,
) {
    let value = match w.smash {
        Some(Smash::Pull(_)) => stage.tuning.stagger_values[1],
        // A knockback that would have knocked a regular enemy down adds the knockdown's value.
        Some(Smash::Knockback(_)) => {
            stage.tuning.stagger_values[2]
                + if stage.has_down {
                    stage.tuning.stagger_down
                } else {
                    0
                }
        }
        _ => return,
    };
    let resists = match target {
        Who::Enemy(i) => s.enemies[i].elite.is_some() && s.enemies[i].hp > 0,
        _ => true,
    };
    if w.combo && resists {
        super::stun::fill(stage, s, target, me, value, w.damage, events);
    }
}

/// The poise damage of a combo hit of `me`'s: the phase's own, else the rules' for the press kind
/// (an A step or a smash step, read from the combo the running step still names). A downed or
/// out player's hit deals none.
fn flinch_by(
    stage: &World,
    s: &mut State,
    me: PlayerIdx,
    i: usize,
    w: &Swing,
    events: &mut Vec<DomainEvent>,
) {
    if !s.players[me].is_up() {
        return;
    }
    let poise_damage =
        w.poise_damage
            .unwrap_or(if super::player::last_press_smash(&s.players[me].combo) {
                stage.tuning.poise_damage_b
            } else {
                stage.tuning.poise_damage_a
            });
    let poise_damage = super::player::breaker_poise(stage, s, me, poise_damage);
    let source = s.players.id(me);
    super::stun::flinch(stage, s, i, poise_damage, source, events);
}

/// Every player in the party that the swing covers, in slot order, unless it is invulnerable:
/// each is hurt once.
pub(super) fn hit_players_side(
    stage: &World,
    s: &mut State,
    w: &Swing,
    events: &mut Vec<DomainEvent>,
) {
    for me in s.players.indices() {
        let id = s.players.id(me);
        let p = &s.players[me];
        if p.invulnerable > 0 {
            if p.combat.perfect_window > 0 && covers(w, p.at, stage.tuning.player_radius) {
                super::player::near_miss(stage, s, me, events);
            }
            continue;
        }
        if p.hp > 0 && covers(w, p.at, stage.tuning.player_radius) && take_hit(s, w, id) {
            let by = match w.source {
                Source::Run(i) => Blame {
                    kind: HitKind::Attack,
                    by: Culprit::Run(s.runs[i].attack),
                },
                Source::Zone(i) => Blame::of_program(HitKind::Zone, s.zones[i].program),
            };
            hurt_player(stage, s, me, super::aided(stage, s, shot(w)), by, events);
        }
    }
}

/// The companions in ID order, each hurt once however many players there are.
pub(super) fn hit_companions_side(
    stage: &World,
    s: &mut State,
    w: &Swing,
    events: &mut Vec<DomainEvent>,
) {
    for i in 0..s.companions.len() {
        let c = &s.companions[i];
        let (id, at, hp) = (c.id, c.at, c.hp);
        let radius = stage.defs.agent(c.def).radius.0;
        if hp > 0 && covers(w, at, radius) && take_hit(s, w, id) {
            // An enemy's hit on a companion reads no player (it neither crits nor echoes).
            damage(
                stage,
                s,
                PlayerIdx::SOLO,
                super::aided(stage, s, shot(w)),
                Who::Companion(i),
                events,
            );
        }
    }
}
