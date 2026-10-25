//! The bytes the state hash writes for a task frame, an orb and an orb's flags: exactly the
//! live fields, in order. A field removed from the state takes its byte out of the hash too.

use super::world::ProgramIndex;
use super::{hash_frame, hash_orb, hash_orb_flags};
use crate::engine::{Aim, Dir, Frame, Hook, OrbState, PathStep};
use crate::fixed::{Fx, Point};
use crate::hash::{StateHasher, fnv1a64};
use crate::id::EntityId;

fn bytes(write: impl FnOnce(&mut StateHasher)) -> u64 {
    let mut h = StateHasher::new();
    write(&mut h);
    h.finish()
}

fn orb(breakable: bool) -> OrbState {
    OrbState {
        at: Point {
            x: Fx(1),
            y: Fx(-2),
        },
        vx: Fx(3),
        vy: Fx(-4),
        dir: Dir { x: 5, y: -6 },
        style: 7,
        owner: EntityId(8),
        damage: 9,
        damage_type: None,
        breaks_charge: true,
        smash: None,
        combo: false,
        hook: None,
        breakable,
        life: 10,
    }
}

#[test]
fn a_task_frame_is_its_five_live_fields() {
    let f = Frame {
        idx: 1,
        loop_i: 2,
        loop_n: 3,
        param: -4,
        offset: 5,
    };
    let mut live = Vec::new();
    live.extend(1u32.to_le_bytes());
    live.extend(2u32.to_le_bytes());
    live.extend(3u32.to_le_bytes());
    live.extend((-4i32).to_le_bytes());
    live.extend(5i32.to_le_bytes());
    assert_eq!(bytes(|h| hash_frame(h, &f)), fnv1a64(&live));
}

#[test]
fn an_orb_is_its_live_fields_and_no_friendly_byte() {
    let mut live = Vec::new();
    for v in [1i32, -2, 3, -4, 5, -6] {
        live.extend(v.to_le_bytes());
    }
    live.extend(7u32.to_le_bytes()); // style
    live.extend(8u32.to_le_bytes()); // owner
    live.extend(9u32.to_le_bytes()); // damage
    live.push(1); // breaks_charge
    live.push(0); // smash: none
    live.push(0); // hook: none
    assert_eq!(bytes(|h| hash_orb(h, &orb(false))), fnv1a64(&live));
}

#[test]
fn an_orbs_hook_is_its_live_fields() {
    let mut o = orb(false);
    o.hook = Some(Box::new(Hook {
        program: ProgramIndex(0),
        path: vec![PathStep {
            node: 2,
            then: true,
        }],
        left: 11,
        owner: EntityId(12),
        aim: Aim::Fixed(Dir { x: 13, y: -14 }),
        damage: 15,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
    }));
    let mut live = Vec::new();
    for v in [1i32, -2, 3, -4, 5, -6] {
        live.extend(v.to_le_bytes());
    }
    live.extend(7u32.to_le_bytes()); // style
    live.extend(8u32.to_le_bytes()); // owner
    live.extend(9u32.to_le_bytes()); // damage
    live.push(1); // breaks_charge
    live.push(0); // smash: none
    live.push(1); // hook: some
    live.extend(1u32.to_le_bytes()); // path length
    live.extend(2u32.to_le_bytes()); // node
    live.push(1); // then
    live.extend(11u32.to_le_bytes()); // left
    live.extend(12u32.to_le_bytes()); // owner
    live.extend(1u32.to_le_bytes()); // aim: fixed
    live.extend(13i32.to_le_bytes());
    live.extend((-14i32).to_le_bytes());
    live.extend(15u32.to_le_bytes()); // damage
    live.push(0); // breaks_charge
    live.push(0); // smash: none
    assert_eq!(bytes(|h| hash_orb(h, &o)), fnv1a64(&live));
}

#[test]
fn an_orbs_flags_are_breakable_alone() {
    assert_eq!(bytes(|h| hash_orb_flags(h, &orb(true))), fnv1a64(&[1]));
    assert_eq!(bytes(|h| hash_orb_flags(h, &orb(false))), fnv1a64(&[0]));
}
