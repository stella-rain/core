//! Attacks: an attacker runs one phased attack at a time per slot (`AttackRun`). A
//! run starts with its first phase on the tick it starts; at the start of each later tick it
//! goes on to its next phase once the current one has lasted its ticks, and ends after the last.
//! Leaving its attacker (death, or a boss leaving the phase that started it) ends it too. A
//! wind-up announces itself with a `Telegraph` event. Each phase spawns its nodes on its first
//! tick as a task of their own, which runs to its end whatever the run does next.
//!
//! The nodes (ADR-SIM-4 tier 3) are a tree carried out over time. A task keeps a stack
//! of frames, one for each list of nodes it is inside, so that it can stop at a `wait` or between
//! the rounds of a `repeat` and go on in a later tick. Presets are expanded to inline nodes
//! before a task starts (see `presets`), so a preset and its inline copy are the same task.
//!
//! What the nodes mean:
//! - `zone` places an area that does not move (`zones`): at the attacker's spot or where the
//!   target stood when the attack began, moved by its `offset` along its direction. A direction
//!   is a unit vector: `aimed` points at the target (along the facing for a combo step's) and
//!   turns by `offset`, `absolute` is an angle from straight down, `relative` turns the direction
//!   the attack's parent was going (straight down for an attack a rule started; the facing for a
//!   combo step's; the orb's own direction for its burst), and `sequential` turns the last
//!   direction by `step`. The first three also turn by every `rotate`, `ring` and `spread` round
//!   them. Positive angles turn from straight down toward the right.
//! - `orb` fires a slow projectile that lives `ticks` ticks (core Z2), the one thing that flies.
//!   When they are up its `burst` is carried out from where it ended, as an attack of its own;
//!   the main character's hit boxes break a `breakable` one with no burst. At most
//!   `MAX_ORBS_PER_OWNER` per attacker and `MAX_ORBS_ALIVE` in all; one over a cap is dropped, in
//!   node order.
//! - `repeat` runs its body `count` times; the next round starts `interval_ticks` ticks after
//!   the body of the last one ends. `ring` runs its body `count` times at once, round after
//!   round turned a full turn / `count` further; `spread` the same, fanned evenly across
//!   `angle` and centred on the direction. A `wait` in their bodies delays the next round.
//! - `rotate` turns everything in its body by `angle`; `wait` pauses the attack.
//!
//! Expressions are evaluated left to right when the node they belong to is carried out, with
//! `rand` drawing from the run's SplitMix64 in that order. Arithmetic saturates, and dividing by
//! zero gives 0. Counts, ticks and speeds that an expression computes are clamped.
//!
//! Dynamic budgets (ADR-SVC-1): at most `MAX_ATTACK_TASKS` attacks at once, and every tick at most
//! `MAX_EMITTER_STEPS_PER_TICK` node steps across all of them, after which the rest wait for
//! the next tick, in order. Orbs and zones are capped as everywhere else.

use super::{Area, hit_boxes, player, select, stun, zones};

use crate::id::EntityId;
use crate::sim::state::State;

/// Whether `owner` is running an attack in `slot`.
pub(super) fn busy(s: &State, owner: EntityId, slot: u8) -> bool {
    s.runs.iter().any(|r| r.owner == owner && r.slot == slot)
}

// --- Carrying out ----------------------------------------------------------------------------

#[cfg(test)]
mod attack_table_tests;

mod emit;
use emit::{exec, uses_spot};

mod program;
use program::{
    Above, above, aim, body_of, list_at, root_list, rounds, saturate, step_task, ticks, value,
};
pub(super) use program::{normalize, rotate};

mod runs;
use runs::begin_phase;
pub(super) use runs::{
    advance_runs, cancel_runs_of, cancelled_token, capped_type_bonus, counterable_run, end_run,
    end_runs_of, fire_hooks, hold_ready, in_end_recovery, is_combo, phase_damage, retain_runs, run,
    run_phase_kind, swing_damage,
};

mod start;
pub(super) use start::{Strike, start, start_aged, start_player, start_rune};

mod phases;
use phases::compiled;
pub(crate) use phases::phase;
pub(super) use phases::{advance_of, dash_of, move_pct, phase_count, phase_ticks, widened_box};

mod ranged;
pub(in crate::sim) use ranged::cool_into;
#[cfg(test)]
pub(super) use ranged::in_view;
use ranged::{cool_token, holds_token};
pub(super) use ranged::{grant_ranged, mark_ranged, ranged_may_start};
