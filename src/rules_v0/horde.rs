//! The bosses of a `time_limit` run (core#212): at each boss time but the last a mid-boss comes,
//! drawn from the mode's pool by the run's generator without using a pool entry twice, and at
//! the last the final boss. The engine has one boss at a time: one that is still alive when the
//! next comes is replaced by it.
//!
//! Every boss is the mode file's definition of the drawn sprite (core#336), or, for a sprite with
//! none, the horde's template with its sprite set to the one drawn; the template is a placeholder
//! (the trial's golem). Either stands 120 pixels above the main character, its `move_to` steps
//! moved with it, and takes the boss's place in the stage, so the rules that read the stage's
//! boss read it, and the state's boss starts fresh at its time. A boss at a `no_call_ticks` time
//! calls no elites, and the k-th mid-boss's hit points are scaled by the file's k-th
//! `mid_boss_hp_pct`. A boss's death opens the boss offer as any boss's does; the run clears only
//! on the final boss's.
//!
//! Elite minutes (ADR-090): on each whole minute of the run before its length that is not a boss
//! time, one elite is drawn from the tier whose start is the latest one not after the tick, by
//! weight, and spawned a ring's width from the main character. They come in the tick's step
//! right after the waves, so a tick draws the waves' placements, then the elite's type and
//! angle (a boss time, whose draw comes before the tick, has no elite minute). The type and the
//! angle are drawn whether or not the elite can spawn: the cap on elites alive (all of them,
//! whoever spawned them) and the cap on enemies hold it back after the draws, so the generator
//! never depends on either. An elite is built from the definition of the type drawn (`elite_types` of the mode file); its sprite is the type's.

use super::Run;
use crate::behaviour::{Action, BossCall};
use crate::engine::{AgentKey, AgentSource, Outcome, State};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::stage::Step;

use super::scale_hp;

/// 60 ticks a second, 60 seconds.
const TICKS_PER_MINUTE: u32 = 3_600;

/// How far from the main character an elite comes: 400 pixels, as the mode's waves do.
const ELITE_RING: Fx = Fx::px(400);

/// How far above the main character a boss comes: 120 pixels.
const ABOVE: i32 = Fx::px(120).0;

/// Brings the boss whose time the next tick reaches, if any. Called before the tick runs.
pub(crate) fn spawn_boss(stage: &Run, s: &mut State) {
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
    let slot = horde.spawned as usize;
    let is_final = slot + 1 == horde.boss_ticks.len();
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
    // The elites that leave when a boss comes go (ADR-090).
    for e in &mut s.enemies {
        if let (Some(el), Some(l)) = (e.elite, e.leave.as_mut()) {
            l.go |= el.boss_call == BossCall::Leave;
        }
    }

    let mut def = horde
        .bosses
        .iter()
        .find(|b| b.base == base)
        .unwrap_or(&horde.template)
        .clone();
    def.base = base;
    // A boss time the mode file lists calls no elites (core#336).
    if horde.no_call_ticks.contains(&due) {
        def.call_elites = None;
    }
    // A mid-boss's hit points rise by its slot (core#336); the difficulty scales them after.
    if !is_final {
        let pct = horde.mid_boss_hp_pct.get(slot).copied().unwrap_or(100);
        def.hp = scale_hp(def.hp, pct);
        for part in &mut def.parts {
            part.hp = scale_hp(part.hp, pct);
        }
    }
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
    let boss_index = u32::try_from(
        horde
            .bosses
            .iter()
            .position(|b| b.base == def.base)
            .unwrap_or(horde.bosses.len()),
    )
    .unwrap_or(u32::MAX);
    s.boss = Some(super::boss_state(
        def,
        boss_index,
        id,
        &mut s.next_id,
        s.hp_pct,
        &s.kinds,
    ));
}

/// The final boss has come and is dead.
pub(super) fn final_boss_dead(s: &State) -> bool {
    s.horde
        .as_ref()
        .is_some_and(|h| h.spawned as usize == h.boss_ticks.len())
        && s.boss.as_ref().is_some_and(|b| b.hp == 0)
}

/// Brings the elite of this tick's minute, if it is one (see the module).
pub(super) fn spawn_elite(stage: &Run, s: &mut State, events: &mut Vec<DomainEvent>) {
    let tick = s.tick;
    let Some(horde) = s.horde.as_ref() else {
        return;
    };
    if tick == 0
        || tick % TICKS_PER_MINUTE != 0
        || tick >= horde.length_ticks
        || horde.boss_ticks.contains(&tick)
    {
        return;
    }
    let Some(tier) = horde
        .elite_table
        .iter()
        .rev()
        .find(|t| t.start_tick <= tick)
    else {
        return;
    };
    let total: u32 = tier.weights.iter().map(|w| w.weight).sum();
    let mut pick = s.rng.below(total.max(1));
    let Some(chosen) = tier.weights.iter().find(|w| {
        let hit = pick < w.weight;
        pick = pick.saturating_sub(w.weight);
        hit
    }) else {
        return;
    };
    let kind = chosen.elite.clone();
    let Some(&(_, index)) = horde.elite_waves.iter().find(|(id, _)| *id == kind) else {
        return;
    };
    let at = super::around_player(stage, s, ELITE_RING);
    let id = EntityId(s.next_id);
    let key = AgentKey {
        source: AgentSource::Wave,
        index,
        rule: 0,
    };
    let def = &stage.waves[index as usize].enemy;
    if super::spawn_enemy(stage, s, key, at) {
        events.push(DomainEvent::EliteSpawned {
            elite: id,
            // A mimic is announced as what it is disguised as.
            asset: def
                .elite
                .as_ref()
                .and_then(|e| e.mimic.as_ref())
                .map_or_else(|| def.base.clone(), |m| m.disguise.clone()),
            kind: def
                .elite
                .as_ref()
                .is_none_or(|e| e.mimic.is_none())
                .then(|| s.kinds.of(&def.base)),
        });
    }
}
