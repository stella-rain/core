//! The bytes the state hash writes for a task frame, an orb, a zone and an agent: every field of
//! the type in declaration order, a tag byte for every `Option` and enum, a length for every
//! list. A field added to one of these types changes its bytes here.

use super::world::{DefIndex, ProgramIndex};
use crate::attack::ZoneShape;
use crate::engine::{
    AgentKey, AgentSource, AgentState, Aim, Dir, Frame, Hook, OrbState, PathStep, ZoneState,
};
use crate::fixed::{Fx, Point};
use crate::hash::{StableHash, StateHasher, fnv1a64};
use crate::id::EntityId;
use crate::kinds::KindIndex;

fn hash_of(v: &impl StableHash) -> u64 {
    let mut h = StateHasher::new();
    v.stable_hash(&mut h);
    h.finish()
}

fn orb() -> OrbState {
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
        breakable: true,
        life: 10,
        stop: None,
        link: None,
        program: ProgramIndex(0),
    }
}

/// The bytes of `orb()` up to its hook.
fn orb_bytes_before_the_hook() -> Vec<u8> {
    let mut live = Vec::new();
    for v in [1i32, -2, 3, -4, 5, -6] {
        live.extend(v.to_le_bytes());
    }
    live.extend(7u16.to_le_bytes()); // style
    live.extend(8u32.to_le_bytes()); // owner
    live.extend(9u32.to_le_bytes()); // damage
    live.push(0); // damage_type: none
    live.push(1); // breaks_charge
    live.push(0); // smash: none
    live.push(0); // combo
    live
}

#[test]
fn a_task_frame_is_its_five_fields() {
    let f = Frame {
        idx: 1,
        loop_i: 2,
        loop_n: 3,
        param: -4,
        offset: 5,
    };
    let mut live = Vec::new();
    live.extend(1u16.to_le_bytes());
    live.extend(2u32.to_le_bytes());
    live.extend(3u32.to_le_bytes());
    live.extend((-4i32).to_le_bytes());
    live.extend(5i32.to_le_bytes());
    assert_eq!(hash_of(&f), fnv1a64(&live));
}

#[test]
fn an_orb_is_all_its_fields() {
    let mut live = orb_bytes_before_the_hook();
    live.push(0); // hook: none
    live.push(1); // breakable
    live.extend(10u32.to_le_bytes()); // life
    live.push(0); // stop: none
    live.push(0); // link: none
    assert_eq!(hash_of(&orb()), fnv1a64(&live));
}

#[test]
fn an_orbs_hook_is_its_fields_after_a_tag() {
    let mut o = orb();
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
        combo: true,
    }));
    let mut live = orb_bytes_before_the_hook();
    live.push(1); // hook: some
    live.extend(1u32.to_le_bytes()); // path length
    live.extend(2u16.to_le_bytes()); // node
    live.push(1); // then
    live.extend(11u32.to_le_bytes()); // left
    live.extend(12u32.to_le_bytes()); // owner
    live.push(1); // aim: fixed
    live.extend(13i32.to_le_bytes());
    live.extend((-14i32).to_le_bytes());
    live.extend(15u32.to_le_bytes()); // damage
    live.push(0); // damage_type: none
    live.push(0); // breaks_charge
    live.push(0); // smash: none
    live.push(1); // combo
    live.push(1); // breakable
    live.extend(10u32.to_le_bytes()); // life
    live.push(0); // stop: none
    live.push(0); // link: none
    assert_eq!(hash_of(&o), fnv1a64(&live));
}

fn zone(warn_only: bool) -> ZoneState {
    ZoneState {
        owner: EntityId(3),
        friendly: false,
        shape: ZoneShape::Circle { radius: Fx(256) },
        at: Point { x: Fx(1), y: Fx(2) },
        dir: Dir { x: 0, y: 256 },
        wind_left: 4,
        active_left: 0,
        wind_ticks: 4,
        active_ticks: 0,
        damage: 0,
        damage_type: None,
        breaks_charge: false,
        warn_only,
        hits: vec![(EntityId(9), 2)],
        program: ProgramIndex(0),
    }
}

#[test]
fn a_zone_is_its_fields_and_not_its_snapshot_lengths() {
    let mut live = Vec::new();
    live.extend(3u32.to_le_bytes()); // owner
    live.push(0); // friendly
    live.push(0); // shape: circle
    live.extend(256i32.to_le_bytes()); // radius
    live.extend(1i32.to_le_bytes()); // at
    live.extend(2i32.to_le_bytes());
    live.extend(0i32.to_le_bytes()); // dir
    live.extend(256i32.to_le_bytes());
    live.extend(4u32.to_le_bytes()); // wind_left
    live.extend(0u32.to_le_bytes()); // active_left
    live.extend(0u32.to_le_bytes()); // damage
    live.push(0); // damage_type: none
    live.push(0); // breaks_charge
    live.push(1); // warn_only
    live.extend(1u32.to_le_bytes()); // hits length
    live.extend(9u32.to_le_bytes());
    live.extend(2u32.to_le_bytes());
    assert_eq!(hash_of(&zone(true)), fnv1a64(&live));

    // Only the lengths of the warning and the hurting that the snapshot shows are left out.
    let mut z = zone(true);
    z.wind_ticks = 99;
    z.active_ticks = 98;
    assert_eq!(hash_of(&z), fnv1a64(&live));
}

#[test]
fn a_zones_warn_only_flag_moves_its_hash() {
    assert_ne!(hash_of(&zone(true)), hash_of(&zone(false)));
}

#[test]
fn an_agent_is_all_its_fields_with_a_tag_for_each_option() {
    let a = AgentState {
        id: EntityId(1),
        def: DefIndex(2),
        kind: KindIndex(3),
        key: AgentKey {
            source: AgentSource::Summoned,
            index: 4,
            rule: 5,
        },
        at: Point { x: Fx(6), y: Fx(7) },
        hp: 8,
        max_hp: 9,
        defence: 10,
        age: 11,
        rules: Default::default(),
        statuses: Default::default(),
        aggro: 12,
        order: None,
        orbit_angle: -13,
        crowded: true,
        burst: false,
        stun_immune: 14,
        elite: None,
        leave: None,
        shrine: None,
    };
    let mut live = Vec::new();
    live.extend(1u32.to_le_bytes()); // id
    live.push(2); // key.source: summoned
    live.extend(4u32.to_le_bytes());
    live.extend(5u32.to_le_bytes());
    live.extend(6i32.to_le_bytes()); // at
    live.extend(7i32.to_le_bytes());
    for v in [8u32, 9, 10, 11] {
        live.extend(v.to_le_bytes()); // hp, max_hp, defence, age
    }
    live.extend(0u32.to_le_bytes()); // rules: none
    live.extend(0u32.to_le_bytes()); // statuses: none
    live.extend(12u32.to_le_bytes()); // aggro
    live.push(0); // order: none
    live.extend((-13i32).to_le_bytes()); // orbit_angle
    live.push(1); // crowded
    live.push(0); // burst
    live.extend(14u32.to_le_bytes()); // stun_immune
    live.push(0); // elite: none
    live.push(0); // leave: none
    live.push(0); // shrine: none
    assert_eq!(hash_of(&a), fnv1a64(&live));
}
