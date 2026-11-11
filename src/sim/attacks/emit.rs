//! Emitting of the attack programs of the live rules: the executor, rounds, zones and orbs.

use super::super::{MAX_ORBS_ALIVE, MAX_ORBS_PER_OWNER, World};
use super::{
    Above, above, aim, body_of, list_at, root_list, rotate, rounds, saturate, ticks, value,
};
use crate::attack::{Anchor, AttackPhase, Direction, Emitter};
use crate::difficulties::scale_pct;
use crate::fixed::{Fx, Point};
use crate::registry;
use crate::sim::state::{
    Aim, AttackTask, Dir, Frame, Hook, OrbLink, OrbState, Origin, PathStep, State, ZoneState,
};
use crate::trig;
use crate::validate::limits::{MAX_LEN, MAX_ORB_TICKS, MAX_ZONE_TICKS};

enum Advance {
    Continue,
    Wait,
    Broken,
}

/// Carries out nodes until the attack waits (true) or is over (false).
pub(super) fn exec(world: &World, s: &mut State, t: &mut AttackTask, budget: &mut u32) -> bool {
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

/// Where the aimed entity is now; `origin` if it is gone or the aim is a fixed direction.
fn target_now(s: &State, aim: Aim, origin: Point) -> Point {
    match aim {
        Aim::Entity(target) => s.find(target).map_or(origin, |w| s.at(w)),
        Aim::Fixed(_) => origin,
    }
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
        warn_only,
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
    // A warning has no `active_ticks` (no draw for it), and a wind-up of at least 1.
    let wind_left = zone_ticks(value(s, t, ab, wind_up_ticks), u32::from(*warn_only));
    let active_left = match active_ticks {
        _ if *warn_only => 0,
        Some(a) => zone_ticks(value(s, t, ab, a), 1),
        None => 1,
    };
    let dir = heading(s, t, ab, origin, direction);
    let from = match anchor {
        Anchor::Attacker => origin,
        Anchor::Target => t.spot.unwrap_or_else(|| target_now(s, t.aim, origin)),
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
            wind_ticks: wind_left,
            active_ticks: active_left,
            damage: if *warn_only { 0 } else { t.damage },
            damage_type: if *warn_only { None } else { t.damage_type },
            breaks_charge: t.breaks_charge && !warn_only,
            warn_only: *warn_only,
            hits: Vec::new(),
            program: t.program,
        },
    );
    t.last_dir = dir;
    true
}

/// Whether any hit box of `phases`, or any zone their nodes place, is anchored at the target:
/// only then does an attack keep the spot where its target stood.
pub(super) fn uses_spot(phases: &[AttackPhase]) -> bool {
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
        anchor,
        offset,
        style,
        ticks,
        breakable,
        burst,
        stop_after,
        link,
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
    // Evaluated last, and only when set, so a stage without it draws as it did.
    let stop = stop_after
        .as_ref()
        .map(|n| self::ticks(value(s, t, ab, n)).min(life));
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
    let hook = if burst.is_empty() {
        None
    } else {
        let mut path = t.base;
        for j in 0..=top {
            let step = PathStep {
                node: t.frames[j].idx,
                then: j == top,
            };
            // The validator holds a path to `MAX_PATTERN_DEPTH`; an unvalidated file that
            // goes deeper loses the orb, as a cap does (fail closed).
            if !path.try_push(step) {
                return true;
            }
        }
        Some(Hook {
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
    };
    let scale = i64::from(trig::SCALE);
    let velocity = |component: i32| Fx(saturate(i64::from(component) * i64::from(speed) / scale));
    let halted = stop == Some(0);
    let link = link.as_ref().map(|l| {
        let slot = u8::try_from(ab.loop_index.clamp(0, i32::from(u8::MAX))).unwrap_or(0);
        // The orb made just before in this ring, if there is one, names the group.
        let before = slot.checked_sub(1).and_then(|k| {
            let mine = s.orbs.iter().rev().filter(|b| b.owner == t.owner);
            mine.filter_map(|b| b.link).find(|b| b.slot == k)
        });
        let group = before.map(|b| b.group).unwrap_or_else(|| {
            s.next_orb_group += 1;
            s.next_orb_group
        });
        OrbLink {
            group,
            slot,
            of: slot + 1,
            width: l.width,
            damage: l.damage.map_or(t.damage, |d| scale_pct(d, s.damage_pct)),
            rehit_ticks: l.rehit_ticks,
            last_hit: None,
        }
    });
    // Sampled once, now, when the node fires; the orb then flies on its own (Z3a-4).
    let from = match anchor {
        Anchor::Attacker => origin,
        Anchor::Target => target_now(s, t.aim, origin),
    };
    s.orbs.push(OrbState {
        at: super::hit_boxes::centre(from, dir, offset.forward.0, offset.right.0),
        vx: if halted { Fx(0) } else { velocity(dir.x) },
        vy: if halted { Fx(0) } else { velocity(dir.y) },
        dir,
        style: style
            .as_ref()
            .map_or(0, |b| registry::orb_style(b.as_str())),
        owner: t.owner,
        damage: t.damage,
        damage_type: t.damage_type,
        breaks_charge: t.breaks_charge,
        smash: t.smash,
        combo: t.combo,
        hook,
        breakable: *breakable,
        life,
        stop,
        link,
        program: t.program,
    });
    if let Some(l) = link {
        for b in s.orbs.iter_mut().filter_map(|b| b.link.as_mut()) {
            if b.group == l.group {
                b.of = l.of;
            }
        }
    }
    true
}
