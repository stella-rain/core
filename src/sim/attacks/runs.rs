//! Attack runs of the live rules: advancing, ending and the damage of a run.

use super::super::Scratch;
use super::super::entity::scale_damage;
use super::super::scratch::RunEnd;
use super::super::{MAX_ATTACK_TASKS, MAX_EMITTER_STEPS_PER_TICK, World, count};
use super::{compiled, cool_token, holds_token, phase, phase_count, step_task};
use crate::attack::{AttackPhase, DamageType, PhaseKind};
use crate::difficulties::scale_pct;
use crate::event::DomainEvent;
use crate::id::EntityId;
use crate::match_ups::MatchUps;
use crate::rewards::MAX_TYPE_BONUS_PCT;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{AttackRun, AttackTask, Frame, Frames, Origin, Path, State};
use crate::sim::world::RangedPool;

/// Every run goes on by a tick: to its next phase once the current one has lasted its ticks,
/// or away after its last phase or when its attacker is gone. Called at the start of a tick,
/// before anyone acts, so that an attacker whose attack has ended can start the next one.
pub(in crate::sim) fn advance_runs(world: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    // One pass that moves each run that stays down to the next free place, in the order they
    // had, instead of shifting every run behind each one that ends.
    let mut changed = false;
    let mut kept = 0;
    for i in 0..s.runs.len() {
        if s.find(s.runs[i].owner).is_none() {
            changed = true;
            if holds_token(world, &s.runs[i]) {
                cool_token(
                    world,
                    s,
                    world.attacks[usize::from(s.runs[i].attack.0)].pool,
                );
            }
            continue;
        }
        s.runs.swap(kept, i);
        // The perfect evade (ADR-SIM-8): a slowed owner's ranged run frees its token for good, and
        // on a holding tick its run waits.
        if let Some(hold) = crate::sim::player::slowed(s, s.runs[kept].owner) {
            if holds_token(world, &s.runs[kept]) {
                s.runs[kept].ranged = false;
            }
            if hold {
                kept += 1;
                continue;
            }
        }
        let r = &mut s.runs[kept];
        r.tick = r.tick.saturating_add(1);
        if r.tick >= phase(world, r, r.phase as usize).ticks {
            let held = holds_token(world, r);
            // The step's last `active` phase ends here: its repeats are queued while the run
            // still stands in that phase (one `bool` at a phase change, nothing per tick).
            if world.has_repeat && last_active_ends(world, r) {
                crate::sim::combat::queue_repeats(world, s, kept);
            }
            let r = &mut s.runs[kept];
            r.phase += 1;
            r.tick = 0;
            changed = true;
            if r.phase as usize >= phase_count(world, r) {
                if held {
                    cool_token(
                        world,
                        s,
                        world.attacks[usize::from(s.runs[kept].attack.0)].pool,
                    );
                }
                continue;
            }
            if held && !holds_token(world, &s.runs[kept]) {
                cool_token(
                    world,
                    s,
                    world.attacks[usize::from(s.runs[kept].attack.0)].pool,
                );
            }
            begin_phase(world, s, kept, events);
        }
        kept += 1;
    }
    // The hit lists of the runs that ended wait in the scratch for the next runs to start with.
    for r in &mut s.runs[kept..] {
        Scratch::recycle_hits(&mut s.scratch.hit_lists, std::mem::take(&mut r.hits));
    }
    s.runs.truncate(kept);
    if changed {
        s.move_index.rebuild(world, &s.runs);
    }
}

/// Whether run `r` is in an `active` phase that no later phase of its attack follows with another
/// `active` one (a gap between two active phases does not end the step's last).
fn last_active_ends(world: &World, r: &AttackRun) -> bool {
    let phases = compiled(world, r);
    phases[r.phase as usize].kind == PhaseKind::Active
        && phases[r.phase as usize + 1..]
            .iter()
            .all(|p| p.kind != PhaseKind::Active)
}

/// Keeps the runs `keep` accepts; the hit lists of the others are kept for later runs.
pub(in crate::sim) fn retain_runs(s: &mut State, keep: impl Fn(&AttackRun) -> bool) {
    debug_assert!(
        s.scratch.ends_pinned.is_none(),
        "a run end inside the hit pass is deferred"
    );
    s.runs.retain_mut(|r| {
        let kept = keep(r);
        if !kept {
            Scratch::recycle_hits(&mut s.scratch.hit_lists, std::mem::take(&mut r.hits));
        }
        kept
    });
}

/// Ends `owner`'s run in `slot`, if any: an evade cancels the main character's combo step, and
/// a follow-up press its recovery. Orbs it spawned fly on.
pub(in crate::sim) fn end_run(world: &World, s: &mut State, owner: EntityId, slot: u8) {
    // Inside the hit pass the end waits for the last swing (ADR-SIM-7).
    if s.scratch.ends_pinned.is_some() {
        s.scratch.push_end(RunEnd::Slot(owner, slot));
        return;
    }
    retain_runs(s, |r| r.owner != owner || r.slot != slot);
    s.move_index.rebuild(world, &s.runs);
}

/// Whether `owner`'s run in `slot` is in a `recovery` phase after its last `active` one: where
/// a follow-up press may cancel it. False with no run, or no active phase.
pub(in crate::sim) fn in_end_recovery(world: &World, s: &State, owner: EntityId, slot: u8) -> bool {
    s.runs.iter().any(|r| {
        r.owner == owner
            && r.slot == slot
            && compiled(world, r)[r.phase as usize].kind == PhaseKind::Recovery
            && compiled(world, r)
                .iter()
                .rposition(|p| p.kind == PhaseKind::Active)
                .is_some_and(|last| (r.phase as usize) > last)
    })
}

/// Whether `owner`'s run in `slot` has just reached the recovery after its last `active` phase,
/// on its first tick: where a held step's hold begins.
pub(in crate::sim) fn hold_ready(world: &World, s: &State, owner: EntityId, slot: u8) -> bool {
    in_end_recovery(world, s, owner, slot)
        && s.runs
            .iter()
            .any(|r| r.owner == owner && r.slot == slot && r.tick == 0)
}

/// Ends every run of `owner` (a boss leaving the phase that started them).
pub(in crate::sim) fn end_runs_of(world: &World, s: &mut State, owner: EntityId) {
    if s.scratch.ends_pinned.is_some() {
        s.scratch.push_end(RunEnd::All(owner));
        return;
    }
    retain_runs(s, |r| r.owner != owner);
    s.move_index.rebuild(world, &s.runs);
}

/// The pool whose token run `r` holds and a cancel cools: a ranged run of a channelled attack
/// in a wind-up or active phase. A zone-leaving run frees its token at once, with no cooling.
pub(in crate::sim) fn cancelled_token(world: &World, r: &AttackRun) -> Option<RangedPool> {
    let attack = &world.attacks[usize::from(r.attack.0)];
    (attack.channelled && holds_token(world, r)).then_some(attack.pool)
}

/// Ends every run of `owner` because a control cancelled it: as `end_runs_of`, and each
/// channelled ranged run that held a token starts its cooling. Inside the hit pass the end is
/// deferred to `apply_ends`, which cools once for each run it removes.
pub(in crate::sim) fn cancel_runs_of(world: &World, s: &mut State, owner: EntityId) {
    if s.scratch.ends_pinned.is_some() {
        s.scratch.push_end(RunEnd::Cancel(owner));
        return;
    }
    for i in 0..s.runs.len() {
        if s.runs[i].owner == owner {
            if let Some(pool) = cancelled_token(world, &s.runs[i]) {
                cool_token(world, s, pool);
            }
        }
    }
    retain_runs(s, |r| r.owner != owner);
    s.move_index.rebuild(world, &s.runs);
}

/// The damage of run `r`'s current phase, for its hit box and the orbs it spawns: the
/// phase's own, or else the starter's, raised by the atk-up and by the type bonus of the
/// phase's damage type (none for a neutral phase), both fixed at the attack's start.
pub(in crate::sim) fn phase_damage(world: &World, r: &AttackRun) -> u32 {
    swing_damage(
        &compiled(world, r)[r.phase as usize],
        r.base,
        r.damage_pct,
        r.atk_up_pct,
        r.type_bonus_pct.as_ref(),
        world.tuning.defence_k,
    )
}

/// The damage of one hit of `phase` for an attack of `base` damage: the phase's own damage or
/// else `base`, at `damage_pct` percent, raised by `atk_up_pct` and by the bonus held for the
/// phase's damage type (none for a neutral phase, or with no bonus held).
#[inline]
pub(in crate::sim) fn swing_damage(
    phase: &AttackPhase,
    base: u32,
    damage_pct: u32,
    atk_up_pct: u32,
    type_bonus_pct: Option<&[u32; DamageType::ALL.len()]>,
    defence_k: u64,
) -> u32 {
    let type_bonus = match (phase.damage_type, type_bonus_pct) {
        (Some(ty), Some(pct)) => pct[ty.index()],
        _ => 0,
    };
    scale_damage(
        scale_pct(phase.damage.unwrap_or(base), damage_pct),
        atk_up_pct,
        type_bonus,
        0,
        0,
        0,
        defence_k,
    )
}

/// The main character's type bonuses, each held to the content's limit, which the validator
/// holds to the hard cap; the clamp keeps an unvalidated table there.
#[inline]
pub(in crate::sim) fn capped_type_bonus(
    match_ups: &MatchUps,
    held: &[u32; DamageType::ALL.len()],
) -> [u32; DamageType::ALL.len()] {
    let limit = match_ups.max_type_bonus_pct().min(MAX_TYPE_BONUS_PCT);
    held.map(|pct| pct.min(limit))
}

/// Whether run `r` is the main character's combo step, A or B.
pub(in crate::sim) fn is_combo(s: &State, r: &AttackRun) -> bool {
    r.slot == super::player::COMBO_SLOT
        && (r.owner == s.players.id(PlayerIdx::SOLO) || s.players.of_id(r.owner).is_some())
}

/// The first tick of run `i`'s current phase: a wind-up announces itself, an active phase
/// starts a new hit list, and the phase's nodes start as a task (unless too many attacks are
/// going already).
pub(super) fn begin_phase(world: &World, s: &mut State, i: usize, events: &mut Vec<DomainEvent>) {
    let r = &s.runs[i];
    if compiled(world, r)[r.phase as usize].kind == PhaseKind::Active {
        s.runs[i].hits.clear();
    }
    let r = &s.runs[i];
    let p = phase(world, r, r.phase as usize);
    if p.kind == PhaseKind::WindUp {
        events.push(DomainEvent::Telegraph {
            source: r.owner,
            ticks: p.ticks,
        });
    }
    if p.spawn.is_empty() || count(&s.tasks) >= MAX_ATTACK_TASKS {
        return;
    }
    // `phase_time` counts from the attack's first tick.
    let elapsed: u32 = (0..r.phase as usize).fold(0, |sum, k| {
        sum.saturating_add(phase(world, r, k).ticks.max(1))
    });
    let def = &world.attacks[usize::from(r.attack.0)];
    let task = AttackTask {
        owner: r.owner,
        program: def.programs[r.phase as usize],
        base: Path::new(),
        origin: r.at.map_or(Origin::Owner, Origin::At),
        aim: r.aim,
        friendly: r.friendly,
        damage: phase_damage(world, r),
        damage_type: p.damage_type,
        breaks_charge: p.breaks_charge,
        smash: r.smash,
        combo: is_combo(s, r),
        age: r.age.saturating_add(elapsed),
        wait_left: 0,
        frames: Frames::from_iter([Frame::default()]),
        parent_dir: r.aim.heading(),
        last_dir: r.aim.heading(),
        spot: r.spot,
    };
    s.tasks.push(task);
}

/// Orbs whose lifetime is over start their bursts, to be carried out this tick.
// Kept out of `sim::step`, so the tick-cost gate measures the rules, not the inliner
// (ADR-SIM-5).
#[inline(never)]
pub(in crate::sim) fn fire_hooks(s: &mut State) {
    let State { orbs, tasks, .. } = s;
    for b in orbs.iter_mut() {
        let Some(hook) = b.hook.as_mut() else {
            continue;
        };
        hook.left = hook.left.saturating_sub(1);
        if hook.left != 0 {
            continue;
        }
        let Some(hook) = b.hook.take() else {
            continue;
        };
        // Past the cap the hook is taken and dropped all the same: the orb loses it either way.
        if count(tasks) >= MAX_ATTACK_TASKS {
            continue;
        }
        let dir = b.dir;
        tasks.push(AttackTask {
            owner: hook.owner,
            program: hook.program,
            base: hook.path,
            origin: Origin::At(b.at),
            aim: hook.aim,
            friendly: false,
            damage: hook.damage,
            damage_type: hook.damage_type,
            breaks_charge: hook.breaks_charge,
            smash: hook.smash,
            combo: hook.combo,
            age: 0,
            wait_left: 0,
            frames: Frames::from_iter([Frame::default()]),
            parent_dir: dir,
            last_dir: dir,
            spot: None,
        });
    }
}

/// Carries out every attack up to its next wait, in the order they started. Returns how many
/// attacks there were and how many nodes were carried out (`Load`).
pub(in crate::sim) fn run(world: &World, s: &mut State) -> (u32, u32) {
    let mut tasks = std::mem::take(&mut s.tasks);
    let running = count(&tasks);
    let mut budget = MAX_EMITTER_STEPS_PER_TICK;
    tasks.retain_mut(|t| step_task(world, s, t, &mut budget));
    s.tasks = tasks;
    (running, MAX_EMITTER_STEPS_PER_TICK - budget)
}
