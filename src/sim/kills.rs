//! Kills and knockback of the live rules: who killed, the death events, pull and knock.

use super::entity::Who;
use super::{World, state};
use crate::attack::Profile;
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::kinds::KindIndex;
use crate::sim::state::AgentState;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{MoveOrder, Pull, State};
use crate::snapshot::EntityKind;

/// Records `me` as the killer of enemy `i`: the first hit of the tick, in its fixed order, that
/// takes the enemy to 0 hit points keeps the credit. Called only for a hit that defeats it.
pub(super) fn record_kill(s: &mut State, i: usize, me: PlayerIdx) {
    let n = s.enemies.len();
    let killers = &mut s.scratch.killers;
    if killers.len() < n {
        killers.resize(n, None);
    }
    if let Some(k) = killers.get_mut(i).filter(|k| k.is_none()) {
        *k = Some(me);
    }
}

/// The player that defeated enemy `i` this tick. An enemy at 0 hit points no hit defeated (a
/// shrine's win) is credited to the lowest present slot.
pub(super) fn killer_of(s: &State, i: usize) -> PlayerIdx {
    s.scratch
        .killers
        .get(i)
        .copied()
        .flatten()
        .or_else(|| s.players.indices().next())
        .unwrap_or(PlayerIdx::SOLO)
}

/// The player that made a hit by `owner`: a companion's, or any other id, is the run's first
/// player (the stage's companions belong to no player).
pub(super) fn hitter(s: &State, owner: EntityId) -> PlayerIdx {
    s.players.of_id(owner).unwrap_or(PlayerIdx::SOLO)
}

/// The other player whose attack-up is on `me` and still running, if any: the one that applied
/// it last (`StatusState::source`). Read when a hit lands, from the hashed state alone, so a
/// replay gives the same answer (ADR-SIM-7).
pub(super) fn buffer_of(s: &State, me: PlayerIdx) -> Option<EntityId> {
    let up = s
        .statuses(Who::Player(me))
        .iter()
        .find(|st| st.kind == state::StatusKind::AtkUp && st.remaining > 0)?;
    s.players
        .of_id(up.source)
        .filter(|b| *b != me)
        .map(|b| s.players.id(b))
}

/// Whether anything can damage `target`: a shrine and a mimic in disguise are objects.
pub(super) fn damageable(s: &State, target: Who) -> bool {
    !matches!(target, Who::Enemy(i) if s.enemies[i].shrine.is_some() || s.enemies[i].hidden())
}

/// What a death is reported as: a part breaks, everything else dies.
pub(super) fn death_event(
    s: &State,
    target: Who,
    id: EntityId,
    killer: Option<PlayerIdx>,
) -> DomainEvent {
    match (target, s.boss.as_ref()) {
        (Who::BossPart(i), Some(boss)) => DomainEvent::PartBroken {
            part: boss.parts[i].name.clone(),
        },
        _ => died_event(s, target, id, killer),
    }
}

/// A death, with what died: the class `EntityView::kind` gives it (`AgentState::entity_kind`), and
/// whether it is an elite. The one place a `Died` is built.
pub(super) fn died_event(
    s: &State,
    target: Who,
    id: EntityId,
    killer: Option<PlayerIdx>,
) -> DomainEvent {
    let (class, agent) = match target {
        Who::Boss => (EntityKind::Boss, None),
        Who::BossPart(_) => (EntityKind::BossPart, None),
        Who::Companion(i) => (EntityKind::Companion, Some(&s.companions[i])),
        Who::Enemy(i) => (
            s.enemies[i].entity_kind(EntityKind::Enemy),
            Some(&s.enemies[i]),
        ),
        // The main character is not reported by a `Died`; it counts with its allies.
        Who::Player(_) => (EntityKind::Companion, None),
    };
    DomainEvent::Died {
        entity: id,
        kind: kind_of(s, target),
        class,
        elite: agent.is_some_and(AgentState::shows_elite),
        killer: killer.map(|k| s.players.id(k)),
    }
}

/// The kind of a target (`EntityView::kind_index`): the index of the content ID the Bestiary keys
/// on. An
/// enemy's, an elite's or a companion's is its sprite ID, the boss's is its own, a boss part's
/// is the part's ID. The main character has none, and a mimic still in disguise shows none.
pub(super) fn kind_of(s: &State, who: Who) -> Option<KindIndex> {
    match who {
        Who::Player(_) => None,
        Who::Boss => Some(s.boss_state().kind),
        Who::BossPart(i) => Some(s.boss_state().parts[i].kind),
        Who::Companion(i) => Some(s.companions[i].kind),
        Who::Enemy(i) => {
            let e = &s.enemies[i];
            (!e.hidden()).then_some(e.kind)
        }
    }
}

/// The profile of a target: an enemy's or an elite's own, the boss's, or a part's own.
/// The main character and the companions have none.
pub(super) fn profile<'a>(stage: &'a World, s: &'a State, target: Who) -> Option<&'a Profile> {
    match target {
        Who::Enemy(i) => stage.defs.agent(s.enemies[i].def).profile.as_ref(),
        Who::Boss => s.boss_def(stage)?.profile.as_ref(),
        Who::BossPart(i) => s.boss_def(stage)?.parts.get(i)?.profile.as_ref(),
        Who::Player(_) | Who::Companion(_) => None,
    }
}

/// The Sub weapon's pull on enemy `i`, hit by a combo part: it is drawn toward the
/// main character by at most `distance`, over `ticks`, as a move order that replaces its
/// movement, and stops where their circles touch. A dead enemy and an elite are not pulled.
pub(super) fn pull_enemy(stage: &World, s: &mut State, me: PlayerIdx, i: usize, pull: Pull) {
    let e = &s.enemies[i];
    if e.hp == 0 || e.elite.is_some() {
        return;
    }
    let to = s.players[me].at;
    let (dx, dy) = (
        i64::from(e.at.x.0) - i64::from(to.x.0),
        i64::from(e.at.y.0) - i64::from(to.y.0),
    );
    let len = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    let contact =
        i64::from(stage.tuning.player_radius) + i64::from(stage.defs.agent(e.def).radius.0);
    let left = (len - i64::from(pull.distance)).max(contact);
    if left >= len {
        return;
    }
    let at =
        |from: Fx, d: i64| Fx(i32::try_from(i64::from(from.0) + d * left / len).unwrap_or(from.0));
    let goal = stage.map().clamp(Point {
        x: at(to.x, dx),
        y: at(to.y, dy),
    });
    s.enemies[i].order = Some(MoveOrder {
        to: goal,
        ticks_left: pull.ticks.max(1),
    });
}

/// A smash's knockback on enemy `i`: the pull's mirror. It is pushed away from
/// the main character by at most `distance`, over `ticks`, as a move order that replaces its
/// movement, and ends inside the map. A dead enemy and an elite are not pushed, nor one on the
/// main character's very centre (no direction); a boss and its parts are never enemies here.
pub(super) fn knock_enemy(stage: &World, s: &mut State, me: PlayerIdx, i: usize, push: Pull) {
    let e = &s.enemies[i];
    if e.hp == 0 || e.elite.is_some() {
        return;
    }
    let from = s.players[me].at;
    let (dx, dy) = (
        i64::from(e.at.x.0) - i64::from(from.x.0),
        i64::from(e.at.y.0) - i64::from(from.y.0),
    );
    let len = i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    if len == 0 {
        return;
    }
    let step = i64::from(push.distance);
    let at =
        |from: Fx, d: i64| Fx(i32::try_from(i64::from(from.0) + d * step / len).unwrap_or(from.0));
    let goal = stage.map().clamp(Point {
        x: at(e.at.x, dx),
        y: at(e.at.y, dy),
    });
    s.enemies[i].order = Some(MoveOrder {
        to: goal,
        ticks_left: push.ticks.max(1),
    });
}
