//! The leash (EC2, DS80, DS106): the crowd trails out of reach of a main character who is faster
//! than most enemies, so a regular enemy outside the reach rectangle around the view's centre
//! (`view_centre`, the camera's clamped centre, which is the main character away from a map
//! edge; more than `leash_reach_x` to the side or `leash_reach_y` above or below, from
//! `rules.json`) moves to the rectangle of `leash_land_x` by `leash_land_y` around that centre,
//! on the side opposite where it was, scaled along its own direction and clamped to the map,
//! and chases from there. Spawns are on the same centre's rectangle, so none is beyond the
//! reach on its spawn tick. At most
//! `MAX_LEASH_MOVES_PER_TICK` move in a tick, in entity-ID order.
//!
//! Who moves: an enemy with no `elite` state (so no elite, mimic or shrine), no `leave` state
//! (a goblin that flees keeps fleeing), no attack running, and a definition that roams: not
//! anchored (no `hold` movement, no `move_to` rule), since it would only walk back and be moved
//! again (EC2b), and not placed by an elite or the boss (a guardian, an escort, an elite's or
//! the boss's summon). The boss, its parts and the
//! companions are not enemies of `State::enemies`. It keeps its hit points, statuses, runes, ID
//! and kind; its `move_to` order and crowded flag are cleared. No random number is drawn.

use super::{MAX_LEASH_MOVES_PER_TICK, World};
use crate::engine::State;
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};

/// Moves the enemies the leash catches; returns how many moved.
pub(super) fn relocate(stage: &World, s: &mut State, events: &mut Vec<DomainEvent>) -> u32 {
    let map = stage.map();
    let centre = super::view_centre(&map, s.player.at);
    let reach_x = i64::from(stage.tuning.leash_reach_x.0);
    let reach_y = i64::from(stage.tuning.leash_reach_y.0);
    let mut moved = 0;
    for i in 0..s.enemies.len() {
        if moved == MAX_LEASH_MOVES_PER_TICK {
            break;
        }
        let e = &s.enemies[i];
        let dx = i64::from(e.at.x.0) - i64::from(centre.x.0);
        let dy = i64::from(e.at.y.0) - i64::from(centre.y.0);
        if (dx.abs() <= reach_x && dy.abs() <= reach_y)
            || e.hp == 0
            || e.elite.is_some()
            || e.leave.is_some()
            || !stage.defs.get(e.def).leash
            || s.runs.iter().any(|r| r.owner == e.id)
        {
            continue;
        }
        let (ox, oy) = landing_offset(stage, dx, dy);
        let to = map.clamp(Point {
            x: centre.x.saturating_add(Fx(ox)),
            y: centre.y.saturating_add(Fx(oy)),
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

/// The offset from the view's centre to the landing point of an enemy at (`dx`, `dy`) from it,
/// outside the reach rectangle: mirrored, and scaled along the enemy's own direction until it
/// meets the landing rectangle. Integer division truncates toward zero.
fn landing_offset(stage: &World, dx: i64, dy: i64) -> (i32, i32) {
    let (lx, ly) = (
        i64::from(stage.tuning.leash_land_x.0),
        i64::from(stage.tuning.leash_land_y.0),
    );
    let (ax, ay) = (dx.abs(), dy.abs());
    // The vertical side is reached first when `lx / ax <= ly / ay`; the divisor of the branch
    // taken is never 0, since the trigger excludes dx = dy = 0 and a zero axis picks the other.
    let (ox, oy) = if lx * ay <= ly * ax {
        (-dx.signum() * lx, -dy * lx / ax)
    } else {
        (-dx * ly / ay, -dy.signum() * ly)
    };
    (
        i32::try_from(ox).unwrap_or(0),
        i32::try_from(oy).unwrap_or(0),
    )
}
