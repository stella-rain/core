//! Starting of attacks in the live rules: the start paths, strikes and launches.

use super::super::entity::{Who, status_pct};
use super::super::{AttackIndex, World};
use super::{aim, begin_phase, busy, capped_type_bonus, phase, uses_spot};
use crate::attack::AttackRef;
use crate::event::DomainEvent;
use crate::fixed::Point;
use crate::sim::state::PlayerIdx;
use crate::sim::state::{Aim, AttackRun, Dir, Smash, State, StatusKind};

/// Starts `attack` for `owner` in `slot`, aimed at `target`. Orbs do `base_damage`, raised by
/// the owner's atk-up as it is now. False, starting nothing, if the slot is busy.
#[allow(clippy::too_many_arguments)]
pub(in crate::sim) fn start(
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
/// since its phase began, so that `phase_time` is the phase's time (ADR-SIM-4).
#[allow(clippy::too_many_arguments)]
pub(in crate::sim) fn start_aged(
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
pub(in crate::sim) fn start_with(
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
    // The player whose type bonuses apply: the owner, when a player owns it.
    let me = match owner {
        Who::Player(me) => me,
        _ => PlayerIdx::SOLO,
    };
    launch(
        world,
        s,
        me,
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

/// How the main character's combo step attacks: `bonus_pct` more damage (a perfect
/// release's), `speed_pct` faster (attack speed), `range_pct` wider (the range rewards and
/// options), and the smash's effect for a combo
/// part.
#[derive(Clone, Copy)]
pub(in crate::sim) struct Strike {
    pub bonus_pct: u32,
    pub speed_pct: u32,
    pub range_pct: u32,
    pub smash: Option<Smash>,
}

/// Starts `attack` for the main character in `slot`, along `facing`, with its `strike`. False,
/// starting nothing, if the slot is busy or the attack has no phases.
#[allow(clippy::too_many_arguments)]
pub(in crate::sim) fn start_player(
    world: &World,
    s: &mut State,
    me: PlayerIdx,
    slot: u8,
    facing: Dir,
    attack: &AttackRef,
    base_damage: u32,
    strike: Strike,
    events: &mut Vec<DomainEvent>,
) -> bool {
    if busy(s, s.id_of(Who::Player(me)), slot) {
        return false;
    }
    let attack = world.attack_index(attack);
    if world.attacks[usize::from(attack.0)].phases.is_empty() {
        return false;
    }
    launch(
        world,
        s,
        me,
        Launch {
            owner: Who::Player(me),
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
/// centred on `at` and facing `facing`, at `base_damage`: raised by its atk-up and type
/// bonuses as they are now, as any of its attacks, but no combo step's. False, starting
/// nothing, if the slot is busy.
#[allow(clippy::too_many_arguments)]
pub(in crate::sim) fn start_rune(
    world: &World,
    s: &mut State,
    me: PlayerIdx,
    slot: u8,
    at: Point,
    facing: Dir,
    attack: AttackIndex,
    base_damage: u32,
    events: &mut Vec<DomainEvent>,
) -> bool {
    if busy(s, s.id_of(Who::Player(me)), slot)
        || world.attacks[usize::from(attack.0)].phases.is_empty()
    {
        return false;
    }
    launch(
        world,
        s,
        me,
        Launch {
            owner: Who::Player(me),
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

/// Pushes a run of `launch.attack` and begins its first phase.
fn launch(
    world: &World,
    s: &mut State,
    me: PlayerIdx,
    launch: Launch,
    events: &mut Vec<DomainEvent>,
) {
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
    // The main character's type bonuses, held to the content's limit.
    let held = &s.players[me].type_bonus_pct;
    let type_bonus_pct = (owner == Who::Player(me) && held.iter().any(|pct| *pct > 0))
        .then(|| capped_type_bonus(&s.match_ups, held));
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
