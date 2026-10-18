//! The expertise gauge (ADR-078 §1 to §3), in a stage whose enemies or boss give expertise
//! points: one gauge for both weapons, from level 0 with no highest level.
//!
//! A hit by an attack of an A or a B step of the combo adds its target's points per hit, and a
//! kill its points per kill too: an enemy's from its wave, the boss's and its parts' from the
//! boss (a part's break is not a kill). A summon gives none. Other damage (an orbit, the stage
//! skill, a companion) gives none. The setup's gain multiplier is not applied yet: the setup
//! holds no upgrades.
//!
//! Levels are reached once a tick, after the collisions: while the points reach the next
//! level's need, they pay it and the level rises. Levels 1 to 10 need the content's thresholds;
//! each after needs the one before's points times the growth factor, rounded down and held to
//! the content maximum. Once the need stops growing, the levels left are one division, so a
//! tick's cost stays bounded however many levels its points cross. A level opens nothing yet.

use crate::engine::{AgentSource, ExpertiseGauge, State};
use crate::expertise::{Expertise, TABLE_LEVELS};
use crate::stage::{ExpertisePoints, Stage};

use super::entity::Who;

/// Whether a run of `stage` has the gauge: some wave or the boss gives points.
fn gives_points(stage: &Stage) -> bool {
    stage.waves.iter().any(|w| w.expertise.is_some())
        || stage.boss.as_ref().is_some_and(|b| b.expertise.is_some())
}

/// The gauge a run of `stage` starts with, at level 0.
pub(super) fn initial(stage: &Stage, expertise: &Expertise) -> Option<ExpertiseGauge> {
    gives_points(stage).then(|| ExpertiseGauge {
        level: 0,
        points: 0,
        next: need(expertise, 1, 0),
    })
}

/// The points level `level` needs from the one before, whose need was `before`.
fn need(expertise: &Expertise, level: u32, before: u32) -> u32 {
    let table = expertise.thresholds();
    match usize::try_from(level) {
        Ok(l @ 1..=TABLE_LEVELS) => table.get(l - 1).copied().unwrap_or(expertise.max_points()),
        _ => {
            let grown = u64::from(before) * u64::from(expertise.growth_pct()) / 100;
            u32::try_from(grown)
                .unwrap_or(u32::MAX)
                .min(expertise.max_points())
        }
    }
}

/// A combo step's hit on `target`, which `killed`, adds its points to the gauge.
pub(super) fn credit(stage: &Stage, s: &mut State, target: Who, killed: bool) {
    let points = match target {
        Who::Enemy(i) => s.enemies.get(i).and_then(|e| match e.key.source {
            AgentSource::Wave => usize::try_from(e.key.index)
                .ok()
                .and_then(|w| stage.waves.get(w))
                .and_then(|w| w.expertise),
            _ => None,
        }),
        Who::Boss => stage.boss.as_ref().and_then(|b| b.expertise),
        Who::BossPart(_) => stage
            .boss
            .as_ref()
            .and_then(|b| b.expertise)
            .map(|p| ExpertisePoints { kill: 0, ..p }),
        Who::Player | Who::Companion(_) => None,
    };
    let (Some(g), Some(p)) = (s.expertise.as_mut(), points) else {
        return;
    };
    let gained = if killed {
        p.hit.saturating_add(p.kill)
    } else {
        p.hit
    };
    g.points = g.points.saturating_add(gained);
}

/// The levels the gauge's points reach this tick.
pub(super) fn level_up(expertise: &Expertise, s: &mut State) {
    let Some(g) = s.expertise.as_mut() else {
        return;
    };
    while g.next > 0 && g.points >= g.next {
        let reached = g.level.saturating_add(1);
        let after = need(expertise, reached.saturating_add(1), g.next);
        if reached > TABLE_LEVELS as u32 && after == g.next {
            // Every level from here needs as much: the rest is one division.
            g.level = g.level.saturating_add(g.points / g.next);
            g.points %= g.next;
            return;
        }
        g.points -= g.next;
        g.level = reached;
        g.next = after;
    }
}
