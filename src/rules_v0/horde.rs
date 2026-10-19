//! The bosses of a `time_limit` run (core#212): at each boss time but the last a mid-boss comes,
//! drawn from the mode's pool by the run's generator without using a pool entry twice, and at
//! the last the final boss. The engine has one boss at a time: one that is still alive when the
//! next comes is replaced by it.
//!
//! Every boss is the horde's template with its sprite set to the one drawn, standing 120 pixels
//! above the main character, its `move_to` steps moved with it. The template is a placeholder
//! (the trial's golem); it takes the boss's place in the stage, so the rules that read the
//! stage's boss read it, and the state's boss starts fresh at its time. A boss's death opens the
//! boss offer as any boss's does; the run clears only on the final boss's.

use crate::behaviour::Action;
use crate::engine::{Outcome, State};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::stage::{Stage, Step};

/// How far above the main character a boss comes: 120 pixels.
const ABOVE: i32 = Fx::px(120).0;

/// Brings the boss whose time the next tick reaches, if any. Called before the tick runs.
pub(crate) fn spawn_boss(stage: &mut Stage, s: &mut State) {
    if s.outcome != Outcome::Running {
        return;
    }
    let tick = s.tick.saturating_add(1);
    let Some(horde) = s.horde.as_mut() else {
        return;
    };
    let Some(&due) = horde.boss_ticks.get(horde.spawned as usize) else {
        return;
    };
    if tick < due {
        return;
    }
    let is_final = horde.spawned as usize + 1 == horde.boss_ticks.len();
    let base = if is_final {
        horde.final_boss.clone()
    } else {
        let unused: Vec<usize> = (0..horde.used.len()).filter(|&i| !horde.used[i]).collect();
        match unused.len() {
            0 => horde.final_boss.clone(),
            n => {
                let pick = unused[s.rng.below(u32::try_from(n).unwrap_or(1)) as usize];
                horde.used[pick] = true;
                horde.pool[pick].clone()
            }
        }
    };
    horde.spawned += 1;

    let mut def = horde.template.clone();
    def.base = base;
    let to = stage.map().clamp(Point {
        x: s.player.at.x,
        y: s.player.at.y.saturating_add(Fx(-ABOVE)),
    });
    let (dx, dy) = (
        i64::from(to.x.0) - i64::from(def.spawn.x.0),
        i64::from(to.y.0) - i64::from(def.spawn.y.0),
    );
    let shift = |p: &mut Point| {
        let at = |v: Fx, d: i64| Fx(i32::try_from(i64::from(v.0) + d).unwrap_or(v.0));
        *p = Point {
            x: at(p.x, dx),
            y: at(p.y, dy),
        };
    };
    def.spawn = to;
    for phase in &mut def.phases {
        for step in &mut phase.timeline {
            if let Step::MoveTo { to, .. } = step {
                shift(to);
            }
        }
        for rule in &mut phase.rules {
            if let Action::MoveTo { to, .. } = &mut rule.action {
                shift(to);
            }
        }
    }
    let id = EntityId(s.next_id);
    s.next_id += 1;
    s.boss = Some(super::boss_state(&def, id, &mut s.next_id));
    stage.boss = Some(def);
}

/// The final boss has come and is dead.
pub(super) fn final_boss_dead(s: &State) -> bool {
    s.horde
        .as_ref()
        .is_some_and(|h| h.spawned as usize == h.boss_ticks.len())
        && s.boss.as_ref().is_some_and(|b| b.hp == 0)
}
