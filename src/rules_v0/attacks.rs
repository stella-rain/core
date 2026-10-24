//! Attacks (ADR-055): an attacker runs one phased attack at a time per slot (`AttackRun`). A
//! run starts with its first phase on the tick it starts; at the start of each later tick it
//! goes on to its next phase once the current one has lasted its ticks, and ends after the last.
//! Leaving its attacker (death, or a boss leaving the phase that started it) ends it too. A
//! wind-up announces itself with a `Telegraph` event. Each phase spawns its nodes on its first
//! tick as a task of their own, which runs to its end whatever the run does next.
//!
//! The nodes (ADR-036 tier 3) are a tree carried out over time. A task keeps a stack
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
//! Dynamic budgets (ADR-020): at most `MAX_ATTACK_TASKS` attacks at once, and every tick at most
//! `MAX_EMITTER_STEPS_PER_TICK` node steps across all of them, after which the rest wait for
//! the next tick, in order. Orbs and zones are capped as everywhere else.

use std::ops::Deref;

use crate::attack::{Anchor, AttackPhase, AttackRef, Direction, Emitter, Expr, HitBox, PhaseKind};
use crate::difficulties::scale_pct;
use crate::engine::{
    Aim, AttackRun, AttackTask, Dir, Frame, Frames, Hook, OrbState, Origin, PathStep, Smash, State,
    StatusKind, ZoneState,
};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::registry;
use crate::rewards::{MAX_RANGE_PCT, MAX_TYPE_BONUS_PCT};
use crate::rng::SplitMix64;
use crate::trig;
use crate::validate::limits::{MAX_LEN, MAX_ORB_TICKS, MAX_REPEAT, MAX_TICKS, MAX_ZONE_TICKS};

use super::Scratch;
use super::entity::{Who, scale_damage, status_pct};
use super::{
    AttackIndex, MAX_ATTACK_TASKS, MAX_EMITTER_STEPS_PER_TICK, MAX_ORBS_ALIVE, MAX_ORBS_PER_OWNER,
    World, count,
};

/// Whether `owner` is running an attack in `slot`.
pub(super) fn busy(s: &State, owner: EntityId, slot: u8) -> bool {
    s.runs.iter().any(|r| r.owner == owner && r.slot == slot)
}

/// The ranged tokens held now (core Z2): the `ranged` runs in their wind-up or active phase.
pub(super) fn ranged_tokens(world: &World, s: &State) -> u32 {
    let held = s
        .runs
        .iter()
        .filter(|r| r.ranged && phase(world, r, r.phase as usize).kind != PhaseKind::Recovery);
    u32::try_from(held.count()).unwrap_or(u32::MAX)
}

/// Marks the run that just started as a `ranged` one: it is the last in `runs`.
pub(super) fn mark_ranged(s: &mut State) {
    if let Some(r) = s.runs.last_mut() {
        r.ranged = true;
    }
}

/// The movement speed, in percent, that `owner`'s running attacks leave it: the lowest
/// `move_pct` of their current phases, 100 with none.
pub(super) fn move_pct(world: &World, s: &State, owner: EntityId) -> i64 {
    let pct = s.move_index.get(owner);
    debug_assert_eq!(
        pct,
        scan_move_pct(world, s, owner),
        "the move index is stale"
    );
    pct
}

/// What `move_pct` was before `State::move_index`: every run scanned. The index is checked
/// against it in debug builds.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn scan_move_pct(world: &World, s: &State, owner: EntityId) -> i64 {
    s.runs
        .iter()
        .filter(|r| r.owner == owner)
        .map(|r| i64::from(phase(world, r, r.phase as usize).move_pct))
        .min()
        .unwrap_or(100)
}

/// Starts `attack` for `owner` in `slot`, aimed at `target`. Orbs do `base_damage`, raised by
/// the owner's atk-up as it is now. False, starting nothing, if the slot is busy.
#[allow(clippy::too_many_arguments)]
pub(super) fn start(
    world: &World,
    s: &mut State,
    owner: Who,
    slot: u8,
    target: Who,
    attack: &AttackRef,
    base_damage: u32,
    events: &mut Vec<DomainEvent>,
) -> bool {
    start_aged(
        world,
        s,
        owner,
        slot,
        target,
        attack,
        base_damage,
        0,
        events,
    )
}

/// As `start`, for an attack whose clock does not start at 0: the boss's attacks count the time
/// since its phase began, so that `phase_time` is the phase's time (ADR-036).
#[allow(clippy::too_many_arguments)]
pub(super) fn start_aged(
    world: &World,
    s: &mut State,
    owner: Who,
    slot: u8,
    target: Who,
    attack: &AttackRef,
    base_damage: u32,
    age: u32,
    events: &mut Vec<DomainEvent>,
) -> bool {
    let aim = Aim::Entity(s.id_of(target));
    start_with(world, s, owner, slot, aim, attack, base_damage, age, events)
}

/// As `start_aged`, for any way of aiming.
#[allow(clippy::too_many_arguments)]
pub(super) fn start_with(
    world: &World,
    s: &mut State,
    owner: Who,
    slot: u8,
    aim: Aim,
    attack: &AttackRef,
    base_damage: u32,
    age: u32,
    events: &mut Vec<DomainEvent>,
) -> bool {
    let id = s.id_of(owner);
    if busy(s, id, slot) {
        return false;
    }
    let attack = world.attack_index(attack);
    if world.attacks[usize::from(attack.0)].phases.is_empty() {
        return false;
    }
    launch(
        world,
        s,
        Launch {
            owner,
            slot,
            aim,
            attack,
            speed_pct: 0,
            width_pct: 0,
            base_damage,
            age,
            bonus_pct: 0,
            smash: None,
            at: None,
        },
        events,
    );
    true
}

/// How the main character's combo step attacks (ADR-070): `bonus_pct` more damage (a perfect
/// release's), `speed_pct` faster (attack speed), `range_pct` wider (the range rewards and
/// options; ADR-078, ADR-099), and the smash's effect for a combo
/// part.
#[derive(Clone, Copy)]
pub(super) struct Strike {
    pub bonus_pct: u32,
    pub speed_pct: u32,
    pub range_pct: u32,
    pub smash: Option<Smash>,
}

/// Starts `attack` for the main character in `slot`, along `facing`, with its `strike`. False,
/// starting nothing, if the slot is busy or the attack has no phases.
#[allow(clippy::too_many_arguments)]
pub(super) fn start_player(
    world: &World,
    s: &mut State,
    slot: u8,
    facing: Dir,
    attack: &AttackRef,
    base_damage: u32,
    strike: Strike,
    events: &mut Vec<DomainEvent>,
) -> bool {
    if busy(s, s.id_of(Who::Player), slot) {
        return false;
    }
    let attack = world.attack_index(attack);
    if world.attacks[usize::from(attack.0)].phases.is_empty() {
        return false;
    }
    launch(
        world,
        s,
        Launch {
            owner: Who::Player,
            slot,
            aim: Aim::Fixed(facing),
            attack,
            speed_pct: strike.speed_pct,
            width_pct: strike.range_pct,
            base_damage,
            age: 0,
            bonus_pct: strike.bonus_pct,
            smash: strike.smash,
            at: None,
        },
        events,
    );
    true
}

/// Starts an on-hit rune's `attack` for the main character in `slot`, its hit boxes
/// centred on `at` and facing `facing`, at `base_damage` (ADR-099): raised by its atk-up and type
/// bonuses as they are now, as any of its attacks, but no combo step's. False, starting
/// nothing, if the slot is busy.
#[allow(clippy::too_many_arguments)]
pub(super) fn start_rune(
    world: &World,
    s: &mut State,
    slot: u8,
    at: Point,
    facing: Dir,
    attack: AttackIndex,
    base_damage: u32,
    events: &mut Vec<DomainEvent>,
) -> bool {
    if busy(s, s.id_of(Who::Player), slot) || world.attacks[usize::from(attack.0)].phases.is_empty()
    {
        return false;
    }
    launch(
        world,
        s,
        Launch {
            owner: Who::Player,
            slot,
            aim: Aim::Fixed(facing),
            attack,
            speed_pct: 0,
            width_pct: 0,
            base_damage,
            age: 0,
            bonus_pct: 0,
            smash: None,
            at: Some(at),
        },
        events,
    );
    true
}

/// What starts a run (`launch`).
struct Launch {
    owner: Who,
    slot: u8,
    aim: Aim,
    attack: AttackIndex,
    /// The attack speed and range that change the phases (`phase`), in percent.
    speed_pct: u32,
    width_pct: u32,
    base_damage: u32,
    age: u32,
    bonus_pct: u32,
    smash: Option<Smash>,
    at: Option<Point>,
}

/// A phase of a run as it is carried out: the compiled phase with attack speed and range
/// applied, which is read when needed instead of kept. It derefs to the compiled
/// phase for everything they leave alone.
pub(crate) struct PhaseView<'a> {
    base: &'a AttackPhase,
    /// The phase's length in ticks, shortened by attack speed.
    pub ticks: u32,
    widened: Option<HitBox>,
}

impl PhaseView<'_> {
    /// The phase's hit box, widened by the range.
    pub fn hit_box(&self) -> Option<&HitBox> {
        self.widened.as_ref().or(self.base.hit_box.as_ref())
    }
}

impl Deref for PhaseView<'_> {
    type Target = AttackPhase;

    fn deref(&self) -> &AttackPhase {
        self.base
    }
}

/// The compiled phases of run `r`'s attack, before attack speed and range.
fn compiled<'a>(world: &'a World, r: &AttackRun) -> &'a [AttackPhase] {
    &world.attacks[usize::from(r.attack.0)].phases
}

/// How many phases run `r` has.
pub(super) fn phase_count(world: &World, r: &AttackRun) -> usize {
    compiled(world, r).len()
}

/// Phase `i` of run `r`, `r.speed_pct` percent faster and `r.width_pct` percent wider.
pub(crate) fn phase<'a>(world: &'a World, r: &AttackRun, i: usize) -> PhaseView<'a> {
    let phases = compiled(world, r);
    let base = &phases[i];
    PhaseView {
        base,
        ticks: if r.speed_pct == 0 {
            base.ticks
        } else {
            faster_ticks(phases, r.speed_pct, i)
        },
        widened: if r.width_pct == 0 {
            None
        } else {
            base.hit_box.as_ref().map(|h| wider_box(h, r.width_pct))
        },
    }
}

/// The length of phase `i` of `phases` `pct` percent faster: the attack's length divided by
/// `1 + pct / 100`, rounded to the nearest tick, and shared among the phases in proportion,
/// each at least 1 tick. The ticks left over after rounding each phase down go to the phases
/// with the largest remainders, the first of equal ones first.
fn faster_ticks(phases: &[AttackPhase], pct: u32, i: usize) -> u32 {
    let per = 100 + u64::from(pct);
    let total: u64 = phases.iter().map(|p| u64::from(p.ticks)).sum();
    let target = ((total * 100 + per / 2) / per).max(phases.len() as u64);
    let rounded = |p: &AttackPhase| {
        let exact = u64::from(p.ticks) * 100;
        let ticks = (exact / per).max(1);
        (ticks, exact.saturating_sub(ticks * per))
    };
    let (own, own_rest) = rounded(&phases[i]);
    let mut sum = 0u64;
    // Phases that come before this one in the order of the largest remainders.
    let mut before = 0u64;
    for (j, p) in phases.iter().enumerate() {
        let (ticks, rest) = rounded(p);
        sum += ticks;
        if rest > own_rest || (rest == own_rest && j < i) {
            before += 1;
        }
    }
    let ticks = u32::try_from(own).unwrap_or(u32::MAX);
    if before < target.saturating_sub(sum) {
        ticks.saturating_add(1)
    } else {
        ticks
    }
}

/// `hit_box` `pct` percent larger, at most `MAX_RANGE_PCT`: its radius and its offset, each
/// rounded toward zero and held within `i32`; an arc's angle stays.
fn wider_box(hit_box: &HitBox, pct: u32) -> HitBox {
    // The validator holds the amount to `MAX_RANGE_PCT`; the clamp keeps an unvalidated file there.
    let per = 100 + i64::from(pct.min(MAX_RANGE_PCT));
    let grow = |v: &mut Fx| {
        v.0 = i32::try_from(i64::from(v.0) * per / 100).unwrap_or(if v.0 < 0 {
            i32::MIN
        } else {
            i32::MAX
        });
    };
    let mut out = hit_box.clone();
    match &mut out {
        HitBox::Circle { offset, radius, .. } | HitBox::Arc { offset, radius, .. } => {
            grow(radius);
            grow(&mut offset.forward);
            grow(&mut offset.right);
        }
        HitBox::Line {
            offset,
            length,
            width,
            ..
        } => {
            grow(length);
            grow(width);
            grow(&mut offset.forward);
            grow(&mut offset.right);
        }
    }
    out
}

/// Pushes a run of `launch.attack` and begins its first phase.
fn launch(world: &World, s: &mut State, launch: Launch, events: &mut Vec<DomainEvent>) {
    let Launch {
        owner,
        slot,
        aim,
        attack,
        speed_pct,
        width_pct,
        base_damage,
        age,
        bonus_pct,
        smash,
        at,
    } = launch;
    let id = s.id_of(owner);
    // A bonus (a perfect release's) adds to the atk-up, for every hit of the attack.
    let atk_up_pct = status_pct(s.statuses(owner), StatusKind::AtkUp).saturating_add(bonus_pct);
    // The main character's type bonuses, each held to the content's limit, which the validator
    // holds to the hard cap; the clamp keeps an unvalidated table there (ADR-100).
    let limit = s.match_ups.max_type_bonus_pct().min(MAX_TYPE_BONUS_PCT);
    let held = &s.player.type_bonus_pct;
    let type_bonus_pct = (owner == Who::Player && held.iter().any(|pct| *pct > 0))
        .then(|| Box::new(held.map(|pct| pct.min(limit))));
    let facing = self::aim(s, s.at(owner), aim);
    let spot = uses_spot(&world.attacks[usize::from(attack.0)].phases).then(|| match aim {
        Aim::Entity(target) => s.find(target).map_or_else(|| s.at(owner), |w| s.at(w)),
        Aim::Fixed(_) => s.at(owner),
    });
    s.runs.push(AttackRun {
        owner: id,
        slot,
        attack,
        speed_pct,
        width_pct,
        phase: 0,
        tick: 0,
        facing,
        aim,
        friendly: State::is_ally(owner),
        ranged: false,
        base: base_damage,
        atk_up_pct,
        damage_pct: if State::is_ally(owner) {
            100
        } else {
            s.damage_pct
        },
        type_bonus_pct,
        age,
        hits: Vec::new(),
        smash,
        at,
        throw: None,
        homing: None,
        spot,
    });
    let first = &s.runs[s.runs.len() - 1];
    s.move_index
        .lower(first.owner, i64::from(phase(world, first, 0).move_pct));
    begin_phase(world, s, s.runs.len() - 1, events);
}

/// Every run goes on by a tick: to its next phase once the current one has lasted its ticks,
/// or away after its last phase or when its attacker is gone. Called at the start of a tick,
/// before anyone acts, so that an attacker whose attack has ended can start the next one.
pub(super) fn advance_runs(world: &World, s: &mut State, events: &mut Vec<DomainEvent>) {
    // One pass that moves each run that stays down to the next free place, in the order they
    // had, instead of shifting every run behind each one that ends.
    let mut changed = false;
    let mut kept = 0;
    for i in 0..s.runs.len() {
        if s.find(s.runs[i].owner).is_none() {
            changed = true;
            continue;
        }
        s.runs.swap(kept, i);
        let r = &mut s.runs[kept];
        r.tick = r.tick.saturating_add(1);
        if r.tick >= phase(world, r, r.phase as usize).ticks {
            r.phase += 1;
            r.tick = 0;
            changed = true;
            if r.phase as usize >= phase_count(world, r) {
                continue;
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

/// Keeps the runs `keep` accepts; the hit lists of the others are kept for later runs.
pub(super) fn retain_runs(s: &mut State, keep: impl Fn(&AttackRun) -> bool) {
    s.runs.retain_mut(|r| {
        let kept = keep(r);
        if !kept {
            Scratch::recycle_hits(&mut s.scratch.hit_lists, std::mem::take(&mut r.hits));
        }
        kept
    });
}

/// Ends `owner`'s run in `slot`, if any: an evade cancels the main character's combo step, and
/// a follow-up press its recovery (ADR-070, ADR-076). Orbs it spawned fly on.
pub(super) fn end_run(world: &World, s: &mut State, owner: EntityId, slot: u8) {
    retain_runs(s, |r| r.owner != owner || r.slot != slot);
    s.move_index.rebuild(world, &s.runs);
}

/// Whether `owner`'s run in `slot` is in a `recovery` phase after its last `active` one: where
/// a follow-up press may cancel it (ADR-076). False with no run, or no active phase.
pub(super) fn in_end_recovery(world: &World, s: &State, owner: EntityId, slot: u8) -> bool {
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
/// on its first tick: where a held step's hold begins (ADR-084 point 6).
pub(super) fn hold_ready(world: &World, s: &State, owner: EntityId, slot: u8) -> bool {
    in_end_recovery(world, s, owner, slot)
        && s.runs
            .iter()
            .any(|r| r.owner == owner && r.slot == slot && r.tick == 0)
}

/// Ends every run of `owner` (a boss leaving the phase that started them).
pub(super) fn end_runs_of(world: &World, s: &mut State, owner: EntityId) {
    retain_runs(s, |r| r.owner != owner);
    s.move_index.rebuild(world, &s.runs);
}

/// The damage of run `r`'s current phase, for its hit box and the orbs it spawns: the
/// phase's own, or else the starter's, raised by the atk-up and by the type bonus of the
/// phase's damage type (none for a neutral phase), both fixed at the attack's start.
pub(super) fn phase_damage(world: &World, r: &AttackRun) -> u32 {
    let p = &compiled(world, r)[r.phase as usize];
    let type_bonus = match (p.damage_type, &r.type_bonus_pct) {
        (Some(ty), Some(pct)) => pct[ty.index()],
        _ => 0,
    };
    scale_damage(
        scale_pct(p.damage.unwrap_or(r.base), r.damage_pct),
        r.atk_up_pct,
        type_bonus,
        0,
        0,
        0,
    )
}

/// Whether run `r` is the main character's combo step, A or B (ADR-078 §2).
pub(super) fn is_combo(r: &AttackRun) -> bool {
    r.owner == super::PLAYER_ID && r.slot == super::player::COMBO_SLOT
}

/// The first tick of run `i`'s current phase: a wind-up announces itself, an active phase
/// starts a new hit list, and the phase's nodes start as a task (unless too many attacks are
/// going already).
fn begin_phase(world: &World, s: &mut State, i: usize, events: &mut Vec<DomainEvent>) {
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
        base: Vec::new(),
        origin: r.at.map_or(Origin::Owner, Origin::At),
        aim: r.aim,
        friendly: r.friendly,
        damage: phase_damage(world, r),
        damage_type: p.damage_type,
        breaks_charge: p.breaks_charge,
        smash: r.smash,
        combo: is_combo(r),
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
// Kept out of `rules_v0::step`, so the tick-cost gate measures the rules, not the inliner
// (ADR-058).
#[inline(never)]
pub(super) fn fire_hooks(s: &mut State) {
    let mut starts = Vec::new();
    for b in &mut s.orbs {
        let Some(hook) = b.hook.as_mut() else {
            continue;
        };
        hook.left = hook.left.saturating_sub(1);
        if hook.left == 0 {
            if let Some(hook) = b.hook.take() {
                starts.push((hook, b.at, b.dir));
            }
        }
    }
    for (hook, at, dir) in starts {
        if count(&s.tasks) >= MAX_ATTACK_TASKS {
            break;
        }
        s.tasks.push(AttackTask {
            owner: hook.owner,
            program: hook.program,
            base: hook.path,
            origin: Origin::At(at),
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
pub(super) fn run(world: &World, s: &mut State) -> (u32, u32) {
    let mut tasks = std::mem::take(&mut s.tasks);
    let running = count(&tasks);
    let mut budget = MAX_EMITTER_STEPS_PER_TICK;
    tasks.retain_mut(|t| step_task(world, s, t, &mut budget));
    s.tasks = tasks;
    (running, MAX_EMITTER_STEPS_PER_TICK - budget)
}

/// One tick of one attack. False once it has finished.
fn step_task(world: &World, s: &mut State, t: &mut AttackTask, budget: &mut u32) -> bool {
    // An attack from an agent ends when the agent is gone.
    if t.origin == Origin::Owner && s.find(t.owner).is_none() {
        return false;
    }
    if t.wait_left > 0 {
        t.wait_left -= 1;
        if t.wait_left > 0 {
            t.age = t.age.saturating_add(1);
            return true;
        }
    }
    let going = exec(world, s, t, budget);
    t.age = t.age.saturating_add(1);
    going
}

// --- Walking the program ---------------------------------------------------------------------

fn body_of(node: &Emitter) -> Option<&[Emitter]> {
    match node {
        Emitter::Repeat { body, .. }
        | Emitter::Ring { body, .. }
        | Emitter::Spread { body, .. }
        | Emitter::Rotate { body, .. } => Some(body),
        Emitter::Wait { .. } | Emitter::Zone { .. } | Emitter::Orb { .. } => None,
    }
}

/// The list a task starts in.
fn root_list<'a>(program: &'a [Emitter], base: &[PathStep]) -> Option<&'a [Emitter]> {
    let mut list = program;
    for step in base {
        let node = list.get(usize::from(step.node))?;
        list = match (step.then, node) {
            // The burst of an orb is carried out as an attack of its own.
            (true, Emitter::Orb { burst, .. }) => burst,
            (true, _) => return None,
            (false, node) => body_of(node)?,
        };
    }
    Some(list)
}

/// The list that the frame at `depth` is in.
fn list_at<'a>(root: &'a [Emitter], frames: &[Frame], depth: usize) -> Option<&'a [Emitter]> {
    let mut list = root;
    for f in frames.get(..depth)? {
        list = body_of(list.get(usize::from(f.idx))?)?;
    }
    Some(list)
}

/// What the nodes above a frame add up to.
struct Above {
    /// The round of the innermost loop (`repeat`, `ring`, `spread`) round the frame.
    loop_index: i32,
    /// The turn of every `rotate`, `ring` and `spread` round it.
    turn: i32,
}

fn above(root: &[Emitter], frames: &[Frame], depth: usize) -> Option<Above> {
    let mut list = root;
    let mut out = Above {
        loop_index: 0,
        turn: 0,
    };
    for f in frames.get(..depth)? {
        let node = list.get(usize::from(f.idx))?;
        match node {
            Emitter::Repeat { .. } => out.loop_index = i32::try_from(f.loop_i).unwrap_or(i32::MAX),
            Emitter::Ring { .. } | Emitter::Spread { .. } => {
                out.loop_index = i32::try_from(f.loop_i).unwrap_or(i32::MAX);
                out.turn = out.turn.saturating_add(f.offset);
            }
            Emitter::Rotate { .. } => out.turn = out.turn.saturating_add(f.offset),
            Emitter::Wait { .. } | Emitter::Zone { .. } | Emitter::Orb { .. } => {}
        }
        list = body_of(node)?;
    }
    Some(out)
}

// --- Expressions -----------------------------------------------------------------------------

fn saturate(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// A random number from `lo` to `hi`, both included; `lo` if the range is empty.
fn rand_between(rng: &mut SplitMix64, lo: i32, hi: i32) -> i32 {
    if hi <= lo {
        return lo;
    }
    let span = u32::try_from(i64::from(hi) - i64::from(lo) + 1).unwrap_or(u32::MAX);
    saturate(i64::from(lo) + i64::from(rng.below(span)))
}

struct Eval<'a> {
    rng: &'a mut SplitMix64,
    loop_index: i32,
    phase_time: i32,
}

fn eval(e: &Expr, c: &mut Eval) -> i32 {
    let two = |b: &(Expr, Expr), c: &mut Eval| {
        let x = i64::from(eval(&b.0, c));
        (x, i64::from(eval(&b.1, c)))
    };
    match e {
        Expr::Const(v) => *v,
        // Presets are taken apart before they run; an `arg` that is left reads as 0.
        Expr::Arg(_) => 0,
        Expr::LoopIndex => c.loop_index,
        Expr::PhaseTime => c.phase_time,
        Expr::Rand(b) => {
            let lo = eval(&b.0, c);
            let hi = eval(&b.1, c);
            rand_between(c.rng, lo, hi)
        }
        Expr::Add(b) => {
            let (x, y) = two(b, c);
            saturate(x + y)
        }
        Expr::Sub(b) => {
            let (x, y) = two(b, c);
            saturate(x - y)
        }
        Expr::Mul(b) => {
            let (x, y) = two(b, c);
            saturate(x * y)
        }
        Expr::Div(b) => {
            let (x, y) = two(b, c);
            if y == 0 { 0 } else { saturate(x / y) }
        }
        Expr::Min(b) => {
            let (x, y) = two(b, c);
            saturate(x.min(y))
        }
        Expr::Max(b) => {
            let (x, y) = two(b, c);
            saturate(x.max(y))
        }
        Expr::Clamp(b) => {
            let v = eval(&b.0, c);
            let lo = eval(&b.1, c);
            let hi = eval(&b.2, c);
            v.max(lo).min(hi)
        }
    }
}

/// Evaluates `e` for a node whose frame is `top`.
fn value(s: &mut State, t: &AttackTask, ab: &Above, e: &Expr) -> i32 {
    let mut c = Eval {
        rng: &mut s.rng.attack,
        loop_index: ab.loop_index,
        phase_time: i32::try_from(t.age).unwrap_or(i32::MAX),
    };
    eval(e, &mut c)
}

fn ticks(v: i32) -> u32 {
    u32::try_from(v.clamp(0, i32::try_from(MAX_TICKS).unwrap_or(i32::MAX))).unwrap_or(0)
}

fn rounds(v: i32) -> u32 {
    u32::try_from(v.clamp(0, MAX_REPEAT)).unwrap_or(0)
}

// --- Directions ------------------------------------------------------------------------------

/// The unit direction of the vector `(x, y)`; straight down if it has no length.
pub(super) fn normalize(x: i64, y: i64) -> Dir {
    let len = i64::try_from((x * x + y * y).unsigned_abs().isqrt()).unwrap_or(i64::MAX);
    if len == 0 {
        return Dir::DOWN;
    }
    let scale = i64::from(trig::SCALE);
    Dir {
        x: saturate(x * scale / len),
        y: saturate(y * scale / len),
    }
}

/// `d` turned by `angle` (1/256 degree), toward the right for positive angles.
pub(super) fn rotate(d: Dir, angle: i32) -> Dir {
    let (sin, cos) = trig::sin_cos(angle);
    let (x, y) = (i64::from(d.x), i64::from(d.y));
    let (sin, cos) = (i64::from(sin), i64::from(cos));
    let scale = i64::from(trig::SCALE);
    normalize((x * cos + y * sin) / scale, (-x * sin + y * cos) / scale)
}

/// The direction from `from` to the entity `target`, if it is there.
fn toward(s: &State, from: Point, target: EntityId) -> Option<Dir> {
    let to = s.at(s.find(target)?);
    Some(normalize(
        i64::from(to.x.0) - i64::from(from.x.0),
        i64::from(to.y.0) - i64::from(from.y.0),
    ))
}

/// The direction an `aimed` orb takes from `from`: at the entity (straight down if it is
/// gone), or along the fixed direction.
fn aim(s: &State, from: Point, aim: Aim) -> Dir {
    match aim {
        Aim::Entity(target) => toward(s, from, target).unwrap_or(Dir::DOWN),
        Aim::Fixed(d) => d,
    }
}

// --- Carrying out ----------------------------------------------------------------------------

enum Advance {
    Continue,
    Wait,
    Broken,
}

/// Carries out nodes until the attack waits (true) or is over (false).
fn exec(world: &World, s: &mut State, t: &mut AttackTask, budget: &mut u32) -> bool {
    let program = &world.emitters[t.program.0 as usize];
    let Some(root) = root_list(program, &t.base) else {
        return false;
    };
    loop {
        if *budget == 0 {
            return true;
        }
        *budget -= 1;
        let Some(top) = t.frames.len().checked_sub(1) else {
            return false;
        };
        let (Some(list), Some(ab)) = (list_at(root, &t.frames, top), above(root, &t.frames, top))
        else {
            return false;
        };
        let Some(node) = list.get(usize::from(t.frames[top].idx)) else {
            // The end of this list: the node round it goes on to its next round, or is done.
            t.frames.pop();
            let Some(parent) = t.frames.len().checked_sub(1) else {
                return false;
            };
            match next_round(s, t, root, parent) {
                Advance::Continue => continue,
                Advance::Wait => return true,
                Advance::Broken => return false,
            }
        };
        match node {
            Emitter::Zone { .. } => {
                if !emit_zone(s, t, &ab, node) {
                    return false;
                }
                t.frames[top].idx += 1;
            }
            Emitter::Orb { .. } => {
                if !emit_orb(s, t, &ab, node, top) {
                    return false;
                }
                t.frames[top].idx += 1;
            }
            Emitter::Wait { ticks: n } => {
                let wait = ticks(value(s, t, &ab, n));
                t.frames[top].idx += 1;
                if wait > 0 {
                    t.wait_left = wait;
                    return true;
                }
            }
            Emitter::Repeat { count: n, body, .. } | Emitter::Ring { count: n, body } => {
                let n = rounds(value(s, t, &ab, n));
                enter_loop(t, top, node, n, 0, body.is_empty());
            }
            Emitter::Spread {
                count: n,
                angle,
                body,
            } => {
                let n = rounds(value(s, t, &ab, n));
                let angle = value(s, t, &ab, angle);
                enter_loop(t, top, node, n, angle, body.is_empty());
            }
            Emitter::Rotate { angle, body } => {
                t.frames[top].offset = value(s, t, &ab, angle);
                enter_body(t, top, body.is_empty());
            }
        }
    }
}

/// Goes into the body of the node the frame `top` is at, or past the node if it has none.
fn enter_body(t: &mut AttackTask, top: usize, empty: bool) {
    if empty {
        t.frames[top].idx += 1;
    } else {
        t.frames.push(Frame::default());
    }
}

/// Starts a `repeat`, `ring` or `spread` of `n` rounds.
fn enter_loop(t: &mut AttackTask, top: usize, node: &Emitter, n: u32, param: i32, empty: bool) {
    if n == 0 {
        t.frames[top].idx += 1;
        return;
    }
    let f = &mut t.frames[top];
    f.loop_i = 0;
    f.loop_n = n;
    f.param = param;
    f.offset = round_turn(node, f);
    enter_body(t, top, empty);
}

/// How far round `f.loop_i` of a `ring` or `spread` turns its directions.
fn round_turn(node: &Emitter, f: &Frame) -> i32 {
    let (i, n) = (i64::from(f.loop_i), i64::from(f.loop_n));
    match node {
        Emitter::Ring { .. } => saturate(i * i64::from(trig::PER_TURN) / n),
        Emitter::Spread { .. } if n > 1 => {
            let angle = i64::from(f.param);
            saturate(i * angle / (n - 1) - angle / 2)
        }
        _ => 0,
    }
}

/// The body of the node at frame `parent` has ended: start its next round, or go on past it.
fn next_round(s: &mut State, t: &mut AttackTask, root: &[Emitter], parent: usize) -> Advance {
    let Some(list) = list_at(root, &t.frames, parent) else {
        return Advance::Broken;
    };
    let Some(node) = list.get(usize::from(t.frames[parent].idx)) else {
        return Advance::Broken;
    };
    match node {
        Emitter::Repeat { .. } | Emitter::Ring { .. } | Emitter::Spread { .. } => {
            let f = &mut t.frames[parent];
            f.loop_i += 1;
            if f.loop_i >= f.loop_n {
                f.idx += 1;
                return Advance::Continue;
            }
            f.offset = round_turn(node, f);
            t.frames.push(Frame::default());
            if let Emitter::Repeat { interval_ticks, .. } = node {
                let Some(ab) = above(root, &t.frames, parent) else {
                    return Advance::Broken;
                };
                let wait = ticks(value(s, t, &ab, interval_ticks));
                if wait > 0 {
                    t.wait_left = wait;
                    return Advance::Wait;
                }
            }
            Advance::Continue
        }
        Emitter::Rotate { .. } => {
            t.frames[parent].idx += 1;
            Advance::Continue
        }
        Emitter::Wait { .. } | Emitter::Zone { .. } | Emitter::Orb { .. } => Advance::Broken,
    }
}

/// The direction a `zone` or `orb` node takes from `origin`, turned by the `rotate`, `ring` and
/// `spread` nodes round it.
fn heading(s: &mut State, t: &AttackTask, ab: &Above, origin: Point, direction: &Direction) -> Dir {
    match direction {
        Direction::Aimed { offset } => {
            let turn = value(s, t, ab, offset).saturating_add(ab.turn);
            rotate(aim(s, origin, t.aim), turn)
        }
        Direction::Absolute { angle } => {
            rotate(Dir::DOWN, value(s, t, ab, angle).saturating_add(ab.turn))
        }
        Direction::Relative { angle } => {
            rotate(t.parent_dir, value(s, t, ab, angle).saturating_add(ab.turn))
        }
        Direction::Sequential { step } => rotate(t.last_dir, value(s, t, ab, step)),
    }
}

/// A zone's tick count, from `lo` to `MAX_ZONE_TICKS`.
fn zone_ticks(v: i32, lo: u32) -> u32 {
    ticks(v).min(MAX_ZONE_TICKS).max(lo)
}

/// Places the zone of a `zone` node, unless there are too many. False if the attack's owner is
/// gone. Its tick counts are evaluated first, then its direction, so the order of `rand` draws is
/// fixed; the zone is centred at its anchor moved by `offset` in the frame of its direction.
fn emit_zone(s: &mut State, t: &mut AttackTask, ab: &Above, node: &Emitter) -> bool {
    let Emitter::Zone {
        shape,
        anchor,
        offset,
        direction,
        wind_up_ticks,
        active_ticks,
    } = node
    else {
        return true;
    };
    let origin = match t.origin {
        Origin::At(p) => p,
        Origin::Owner => match s.find(t.owner) {
            Some(w) => s.at(w),
            None => return false,
        },
    };
    let wind_left = zone_ticks(value(s, t, ab, wind_up_ticks), 0);
    let active_left = zone_ticks(value(s, t, ab, active_ticks), 1);
    let dir = heading(s, t, ab, origin, direction);
    let from = match anchor {
        Anchor::Attacker => origin,
        Anchor::Target => t.spot.unwrap_or_else(|| match t.aim {
            Aim::Entity(target) => s.find(target).map_or(origin, |w| s.at(w)),
            Aim::Fixed(_) => origin,
        }),
    };
    super::zones::place(
        s,
        ZoneState {
            owner: t.owner,
            friendly: t.friendly,
            shape: shape.clone(),
            at: super::hit_boxes::centre(from, dir, offset.forward.0, offset.right.0),
            dir,
            wind_left,
            active_left,
            damage: t.damage,
            damage_type: t.damage_type,
            breaks_charge: t.breaks_charge,
            hits: Vec::new(),
        },
    );
    t.last_dir = dir;
    true
}

/// Whether any hit box of `phases`, or any zone their nodes place, is anchored at the target:
/// only then does an attack keep the spot where its target stood.
fn uses_spot(phases: &[AttackPhase]) -> bool {
    fn nodes(list: &[Emitter]) -> bool {
        list.iter().any(|n| match n {
            Emitter::Zone { anchor, .. } => *anchor == Anchor::Target,
            Emitter::Orb { burst, .. } => nodes(burst),
            other => body_of(other).is_some_and(nodes),
        })
    }
    phases.iter().any(|p| {
        p.hit_box
            .as_ref()
            .is_some_and(|h| h.anchor() == Anchor::Target)
            || nodes(&p.spawn)
    })
}

/// An orb's lifetime in ticks, from 1 to `MAX_ORB_TICKS` (core Z2).
fn orb_ticks(v: i32) -> u32 {
    u32::try_from(v.clamp(1, i32::try_from(MAX_ORB_TICKS).unwrap_or(i32::MAX))).unwrap_or(1)
}

/// Makes the orb of an `orb` node, a slow projectile with a lifetime whose burst is its hook: the
/// `burst` list is carried out where the orb ended, `ticks` ticks on, as an attack of its own.
/// False if the attack's owner is gone. Every expression is evaluated, in the order
/// speed, ticks, direction, whether or not a cap then drops the orb, so the draws of `rand` do
/// not depend on the caps. An orb over `MAX_ORBS_PER_OWNER` or `MAX_ORBS_ALIVE` is dropped; so is
/// any orb of an ally's attack (the validator refuses them, this keeps an unvalidated one out).
fn emit_orb(s: &mut State, t: &mut AttackTask, ab: &Above, node: &Emitter, top: usize) -> bool {
    let Emitter::Orb {
        speed,
        direction,
        style,
        ticks,
        breakable,
        burst,
    } = node
    else {
        return true;
    };
    let origin = match t.origin {
        Origin::At(p) => p,
        Origin::Owner => match s.find(t.owner) {
            Some(w) => s.at(w),
            None => return false,
        },
    };
    let speed = value(s, t, ab, speed).clamp(-MAX_LEN, MAX_LEN);
    let life = orb_ticks(value(s, t, ab, ticks));
    let dir = heading(s, t, ab, origin, direction);
    t.last_dir = dir;
    if t.friendly {
        return true;
    }
    if s.orbs.len() >= MAX_ORBS_ALIVE as usize {
        return true;
    }
    if s.orbs.len() >= MAX_ORBS_PER_OWNER as usize {
        let mine = s.orbs.iter().filter(|b| b.owner == t.owner);
        if mine.count() >= MAX_ORBS_PER_OWNER as usize {
            return true;
        }
    }
    let hook = (!burst.is_empty()).then(|| {
        let path = t
            .base
            .iter()
            .copied()
            .chain((0..=top).map(|j| PathStep {
                node: t.frames[j].idx,
                then: j == top,
            }))
            .collect();
        Box::new(Hook {
            program: t.program,
            path,
            left: life,
            owner: t.owner,
            aim: t.aim,
            damage: t.damage,
            damage_type: t.damage_type,
            breaks_charge: t.breaks_charge,
            smash: t.smash,
            combo: t.combo,
        })
    });
    let scale = i64::from(trig::SCALE);
    let velocity = |component: i32| Fx(saturate(i64::from(component) * i64::from(speed) / scale));
    s.orbs.push(OrbState {
        at: origin,
        vx: velocity(dir.x),
        vy: velocity(dir.y),
        dir,
        style: style.as_ref().map_or(0, |b| registry::orb_style(&b.0)),
        owner: t.owner,
        damage: t.damage,
        damage_type: t.damage_type,
        breaks_charge: t.breaks_charge,
        smash: t.smash,
        combo: t.combo,
        hook,
        breakable: *breakable,
        life,
    });
    true
}

#[cfg(test)]
mod attack_table_tests {
    use super::*;
    use crate::attack::Offset;
    use crate::engine::{Aim, Dir};
    use crate::rules_v0::world::World;
    use crate::run_content::RunContent;

    // The two functions below are `faster` and `wider` as they were before attacks were compiled
    // into the world's table, kept as the oracle for `phase`.
    fn reference_faster(phases: &[AttackPhase], pct: u32) -> Vec<AttackPhase> {
        let per = 100 + u64::from(pct);
        let total: u64 = phases.iter().map(|p| u64::from(p.ticks)).sum();
        let target = ((total * 100 + per / 2) / per).max(phases.len() as u64);
        let mut out: Vec<AttackPhase> = phases.to_vec();
        let mut rests: Vec<(u64, usize)> = Vec::with_capacity(phases.len());
        let mut sum = 0u64;
        for (i, p) in phases.iter().enumerate() {
            let exact = u64::from(p.ticks) * 100;
            let ticks = (exact / per).max(1);
            rests.push((exact.saturating_sub(ticks * per), i));
            out[i].ticks = u32::try_from(ticks).unwrap_or(u32::MAX);
            sum += ticks;
        }
        rests.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for &(_, i) in rests.iter().take(target.saturating_sub(sum) as usize) {
            out[i].ticks = out[i].ticks.saturating_add(1);
        }
        out
    }

    fn reference_wider(phases: &[AttackPhase], pct: u32) -> Vec<AttackPhase> {
        let per = 100 + i64::from(pct.min(MAX_RANGE_PCT));
        let grow = |v: &mut Fx| {
            v.0 = i32::try_from(i64::from(v.0) * per / 100).unwrap_or(if v.0 < 0 {
                i32::MIN
            } else {
                i32::MAX
            });
        };
        let mut out = phases.to_vec();
        for p in &mut out {
            match &mut p.hit_box {
                Some(
                    HitBox::Circle { offset, radius, .. } | HitBox::Arc { offset, radius, .. },
                ) => {
                    grow(radius);
                    grow(&mut offset.forward);
                    grow(&mut offset.right);
                }
                Some(HitBox::Line {
                    offset,
                    length,
                    width,
                    ..
                }) => {
                    grow(length);
                    grow(width);
                    grow(&mut offset.forward);
                    grow(&mut offset.right);
                }
                None => {}
            }
        }
        out
    }

    fn phase_of(kind: PhaseKind, ticks: u32, hit_box: Option<HitBox>) -> AttackPhase {
        AttackPhase {
            kind,
            ticks,
            hit_box,
            damage: None,
            damage_type: None,
            rehit_ticks: None,
            spawn: Vec::new(),
            move_pct: 100,
            breaks_charge: false,
        }
    }

    fn offset(forward: i32, right: i32) -> Offset {
        Offset {
            forward: Fx(forward),
            right: Fx(right),
        }
    }

    /// A combo step of every hit box shape, with lengths that do not divide evenly.
    fn combo_step() -> Vec<AttackPhase> {
        vec![
            phase_of(PhaseKind::WindUp, 7, None),
            phase_of(
                PhaseKind::Active,
                5,
                Some(HitBox::Circle {
                    anchor: Default::default(),
                    offset: offset(3_000, -1_000),
                    radius: Fx(9_001),
                }),
            ),
            phase_of(
                PhaseKind::Active,
                4,
                Some(HitBox::Arc {
                    anchor: Default::default(),
                    offset: offset(0, 777),
                    radius: Fx(20_000),
                    half_angle: 5_000,
                }),
            ),
            phase_of(
                PhaseKind::Active,
                3,
                Some(HitBox::Line {
                    anchor: Default::default(),
                    offset: offset(-501, 2),
                    angle: 100,
                    length: Fx(30_001),
                    width: Fx(7_777),
                }),
            ),
            phase_of(PhaseKind::Recovery, 11, None),
        ]
    }

    fn run_of(attack: AttackIndex, speed_pct: u32, width_pct: u32) -> AttackRun {
        AttackRun {
            owner: EntityId(1),
            slot: 0,
            attack,
            speed_pct,
            width_pct,
            phase: 0,
            tick: 0,
            facing: Dir::DOWN,
            aim: Aim::Fixed(Dir::DOWN),
            friendly: true,
            ranged: false,
            base: 1,
            atk_up_pct: 0,
            damage_pct: 100,
            type_bonus_pct: None,
            age: 0,
            hits: Vec::new(),
            smash: None,
            at: None,
            throw: None,
            homing: None,
            spot: None,
        }
    }

    #[test]
    fn a_phase_read_through_the_table_is_what_faster_and_wider_made() {
        let attack = AttackRef::Inline(combo_step());
        let mut world = World::for_test(RunContent::bundled().trial().clone());
        let index = world.compile_attack(&attack).unwrap();
        // (attack speed, range): none, the case of the task, each alone, and the cap.
        let cases = [
            (0, 0),
            (30, 0),
            (0, 25),
            (30, 25),
            (30, 33),
            (100, 50),
            (1, 1),
            (250, MAX_RANGE_PCT),
        ];
        for (speed, range) in cases {
            let mut oracle = combo_step();
            if speed != 0 {
                oracle = reference_faster(&oracle, speed);
            }
            if range != 0 {
                oracle = reference_wider(&oracle, range);
            }
            let run = run_of(index, speed, range);
            for (i, expected) in oracle.iter().enumerate() {
                let got = phase(&world, &run, i);
                assert_eq!(got.ticks, expected.ticks, "ticks of {i} at {speed}/{range}");
                assert_eq!(
                    got.hit_box(),
                    expected.hit_box.as_ref(),
                    "box of {i} at {speed}/{range}"
                );
                assert_eq!(got.kind, expected.kind);
                assert_eq!(got.move_pct, expected.move_pct);
            }
            assert_eq!(phase_count(&world, &run), oracle.len());
        }
    }

    #[test]
    fn an_attack_compiles_once_and_a_preset_keeps_its_name() {
        let mut world = World::for_test(RunContent::bundled().trial().clone());
        let inline = AttackRef::Inline(combo_step());
        let a = world.compile_attack(&inline).unwrap();
        assert_eq!(world.compile_attack(&inline).unwrap(), a);
        assert_eq!(world.attacks[usize::from(a.0)].preset, None);
        let other = AttackRef::Inline(vec![phase_of(PhaseKind::Active, 2, None)]);
        assert_ne!(world.compile_attack(&other).unwrap(), a);
    }
}
