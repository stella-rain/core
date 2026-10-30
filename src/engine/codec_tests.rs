//! The savepoint codec of the state types (`sim::state_codec`): each type round-trips, every
//! field reaches the bytes, and no input makes a decode panic. The perturb half mirrors
//! `hash_coverage_tests`: each type has one invocation of `each_field_moves_the_encoding!`,
//! which destructures the type without `..`, so a field in no list does not compile. Unlike the
//! hash, the encoding saves every field, the content indices included.

use super::hash_coverage_tests::{Perturb, engine, full_state};
use super::*;
use crate::fixed::{Fx, Point};
use crate::hash::{StableHash, StateHasher};
use crate::id::EntityId;
use crate::savepoint::testing::{from_bytes, round_trip, to_bytes};
use crate::savepoint::{Decode, Encode};
use std::fmt::Debug;

fn hash_of<T: StableHash>(v: &T) -> u64 {
    let mut h = StateHasher::new();
    v.stable_hash(&mut h);
    h.finish()
}

/// As `round_trip`, and the decoded value hashes as the original.
fn round_trip_hashed<T: Encode + Decode + StableHash + PartialEq + Debug>(v: &T) {
    round_trip(v);
    let back: T = from_bytes(&to_bytes(v)).unwrap();
    assert_eq!(hash_of(&back), hash_of(v));
}

/// A decode of every proper prefix of `v`'s bytes, and of the bytes with each one set to 0xff,
/// is an `Ok` or an `Err`; a panic fails the test.
fn never_panics<T: Encode + Decode>(v: &T) {
    let bytes = to_bytes(v);
    for n in 0..bytes.len() {
        assert!(
            from_bytes::<T>(&bytes[..n]).is_err(),
            "a prefix of {n} bytes decoded"
        );
    }
    for i in 0..bytes.len() {
        let mut flipped = bytes.clone();
        flipped[i] = 0xff;
        let _ = from_bytes::<T>(&flipped);
    }
}

/// One state type: `get` finds a value of it in the base state, and every field of it, listed,
/// must change the bytes when perturbed.
macro_rules! each_field_moves_the_encoding {
    (
        $failures:ident, $base:ident, $ty:ident, $get:expr,
        fields: [$($f:ident),* $(,)?] $(,)?
    ) => {{
        #[allow(dead_code, unused_variables)]
        fn all_listed(t: &$ty) {
            let $ty { $($f: _,)* } = t;
        }
        let get: fn(&mut State) -> &mut $ty = $get;
        let base_bytes = to_bytes(get(&mut $base.clone()));
        $(
            let mut s = $base.clone();
            Perturb::perturb(&mut get(&mut s).$f);
            if to_bytes(get(&mut s)) == base_bytes {
                $failures.push(format!(
                    "{}.{} does not move the encoding",
                    stringify!($ty),
                    stringify!($f)
                ));
            }
        )*
    }};
}

fn base() -> State {
    full_state(&engine())
}

#[test]
fn the_attack_side_types_of_the_full_state_round_trip() {
    let s = base();
    round_trip_hashed(&s.tasks[0]);
    round_trip_hashed(&s.tasks[0].frames[0]);
    round_trip_hashed(&s.tasks[0].base[0]);
    round_trip_hashed(&s.runs[0]);
    round_trip_hashed(&s.orbs[0]);
    round_trip_hashed(&s.orbs[0].link.unwrap());
    round_trip_hashed(s.orbs[0].hook.as_deref().unwrap());
    round_trip_hashed(&s.zones[0]);
    round_trip_hashed(&s.orbs[0].dir);
    for aim in [Aim::Entity(EntityId(4)), Aim::Fixed(Dir { x: 3, y: -4 })] {
        round_trip_hashed(&aim);
    }
    for origin in [
        Origin::Owner,
        Origin::At(Point {
            x: Fx(1),
            y: Fx(-2),
        }),
    ] {
        round_trip_hashed(&origin);
    }
    let pull = Pull {
        distance: -3,
        ticks: 4,
    };
    for smash in [
        Smash::Pull(pull),
        Smash::Knockback(pull),
        Smash::Bleed,
        Smash::Stun { ticks: 5 },
    ] {
        round_trip_hashed(&smash);
    }
}

#[test]
fn an_unknown_tag_of_an_attack_side_enum_is_refused() {
    for bytes in [&[2u8][..], &[3, 0, 0, 0, 0]] {
        assert!(from_bytes::<Aim>(bytes).is_err());
    }
    assert!(from_bytes::<Origin>(&[2]).is_err());
    assert!(from_bytes::<Smash>(&[4]).is_err());
    assert_eq!(to_bytes(&Smash::Bleed), [2]);
    assert_eq!(to_bytes(&Aim::Entity(EntityId(1))), [0, 1, 0, 0, 0]);
    assert_eq!(to_bytes(&Origin::Owner), [0]);
}

#[test]
fn a_cut_or_damaged_attack_side_value_never_panics_a_decode() {
    let s = base();
    never_panics(&s.tasks[0]);
    never_panics(&s.runs[0]);
    never_panics(&s.orbs[0]);
    never_panics(&s.zones[0]);
}

#[test]
fn every_attack_side_field_moves_the_encoding() {
    let base = base();
    let mut failures: Vec<String> = Vec::new();

    each_field_moves_the_encoding!(failures, base, AttackTask, |s| &mut s.tasks[0],
        fields: [owner, program, base, origin, aim, friendly, damage, damage_type, breaks_charge, smash, combo, age, wait_left, frames, parent_dir, last_dir, spot]);
    each_field_moves_the_encoding!(failures, base, AttackRun, |s| &mut s.runs[0],
        fields: [owner, slot, attack, speed_pct, width_pct, phase, tick, facing, aim, friendly, ranged, base, atk_up_pct, damage_pct, type_bonus_pct, age, hits, smash, at, throw, homing, spot]);
    each_field_moves_the_encoding!(failures, base, Frame, |s| &mut s.tasks[0].frames[0],
        fields: [idx, loop_i, loop_n, param, offset]);
    each_field_moves_the_encoding!(failures, base, PathStep, |s| &mut s.tasks[0].base[0],
        fields: [node, then]);
    each_field_moves_the_encoding!(failures, base, Pull, |s| match s.orbs[0].smash.as_mut().unwrap() { Smash::Pull(p) => p, _ => unreachable!() },
        fields: [distance, ticks]);
    each_field_moves_the_encoding!(failures, base, OrbState, |s| &mut s.orbs[0],
        fields: [at, vx, vy, dir, style, owner, damage, damage_type, breaks_charge, smash, combo, hook, breakable, life, stop, link, program]);
    each_field_moves_the_encoding!(failures, base, OrbLink, |s| s.orbs[0].link.as_mut().unwrap(),
        fields: [group, slot, of, width, damage, rehit_ticks, last_hit]);
    each_field_moves_the_encoding!(failures, base, Hook, |s| s.orbs[0].hook.as_deref_mut().unwrap(),
        fields: [program, path, left, owner, aim, damage, damage_type, breaks_charge, smash, combo]);
    each_field_moves_the_encoding!(failures, base, ZoneState, |s| &mut s.zones[0],
        fields: [owner, friendly, shape, at, dir, wind_left, active_left, wind_ticks, active_ticks, damage, damage_type, breaks_charge, warn_only, hits, program]);
    each_field_moves_the_encoding!(failures, base, Dir, |s| &mut s.player.facing,
        fields: [x, y]);

    assert!(
        failures.is_empty(),
        "{} fields are not saved:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
