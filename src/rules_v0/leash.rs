//! The leash (EC2, DS80): the crowd trails out of reach of a main character who is faster than
//! most enemies, so a regular enemy that falls more than `LEASH_DISTANCE` behind moves to the
//! ring of `LEASH_RING` around the main character, on the side opposite where it was, and
//! chases from there. At most `MAX_LEASH_MOVES_PER_TICK` move in a tick, in entity-ID order.
//!
//! Who moves: an enemy with no `elite` state (so no elite, mimic or shrine), no `leave` state
//! (a goblin that flees keeps fleeing), and no attack running. The boss, its parts and the
//! companions are not enemies of `State::enemies`. It keeps its hit points, statuses, runes, ID
//! and kind; its `move_to` order and crowded flag are cleared. No random number is drawn.

use super::{LEASH_DISTANCE, LEASH_RING, MAX_LEASH_MOVES_PER_TICK, World};
use crate::engine::State;
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};

/// Moves the enemies the leash catches; returns how many moved.
pub(super) fn relocate(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) -> u32 {
    let player = s.player.at;
    let limit = i64::from(LEASH_DISTANCE.0) * i64::from(LEASH_DISTANCE.0);
    let mut moved = 0;
    for i in 0..s.enemies.len() {
        if moved == MAX_LEASH_MOVES_PER_TICK {
            break;
        }
        let e = &s.enemies[i];
        let dx = i64::from(e.at.x.0) - i64::from(player.x.0);
        let dy = i64::from(e.at.y.0) - i64::from(player.y.0);
        let squared = dx * dx + dy * dy;
        if squared <= limit
            || e.hp == 0
            || e.elite.is_some()
            || e.leave.is_some()
            || s.runs.iter().any(|r| r.owner == e.id)
        {
            continue;
        }
        let len = i64::try_from(squared.unsigned_abs().isqrt()).unwrap_or(i64::MAX);
        let ring = i64::from(LEASH_RING.0);
        let to = stage.map().clamp(Point {
            x: player.x.saturating_add(Fx(opposite(dx, ring, len))),
            y: player.y.saturating_add(Fx(opposite(dy, ring, len))),
        });
        let e = &mut s.enemies[i];
        e.at = to;
        e.order = None;
        e.crowded = false;
        events.push(DomainEvent::EnemyRelocated { entity: e.id });
        moved += 1;
    }
    moved
}

/// One axis of the offset from the main character to the landing point: `-d * ring / len`.
fn opposite(d: i64, ring: i64, len: i64) -> i32 {
    i32::try_from(-d * ring / len).unwrap_or(0)
}
