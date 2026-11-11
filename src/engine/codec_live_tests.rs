//! The live (`sim_version` 2) counterpart of the state half of `codec_tests`: the savepoint
//! codec of `sim::state_codec` for the live state. Each value round-trips, every field reaches
//! the bytes, and no input makes a decode panic. The live `Streams`, `StarField` and `Players`
//! have no codec of their own (the state's encoder writes them in place), so the checks of
//! those, and every field check, run on the bytes of the whole state. The frozen file guards
//! version 1 and is not edited. Of the star offers `Offers` keeps `set_counts` alone.

use super::hash_coverage_live_tests::{Perturb, engine, live_full_state};
use super::run::live::*;
use super::{Engine, Input, Outcome};
use crate::fixed::Fx;
use crate::hash::{StableHash, StateHasher};
use crate::id::{EntityId, PlayerIdx};
use crate::kinds::KindTable;
use crate::rewards::Rarity;
use crate::run_stats::{StatEffect, StunRules};
use crate::savepoint::testing::{from_bytes, round_trip, to_bytes};
use crate::savepoint::{Decode, DecodeError, Encode, Reader};
use crate::sim::equipment::{EquipSlot, Equipment};
use std::fmt::Debug;
use std::sync::Arc;

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

/// The savepoint bytes of the whole state.
fn bytes(s: &State) -> Vec<u8> {
    to_bytes(s)
}

/// The state in `bytes`, over `fresh`, all bytes read.
fn decoded(bytes: &[u8], fresh: &State) -> Result<State, DecodeError> {
    let mut r = Reader::new(bytes);
    let s = State::decode_over(&mut r, fresh)?;
    r.finish()?;
    Ok(s)
}

/// One state type: `get` finds a value of it in the base state, and every field of it, listed,
/// must change the bytes of the whole state when perturbed. A field in `rebuilt` is not saved
/// (it comes from the fresh engine or is derived): perturbing it must leave the bytes as they
/// are. A field in `skip` is perturbed by the test itself, as it cannot change alone. Every
/// field is in one of the lists, or the invocation does not compile.
macro_rules! each_field_moves_the_encoding {
    (
        $failures:ident, $base:ident, $ty:ident, $get:expr,
        fields: [$($f:ident),* $(,)?]
        $(, rebuilt: [$($r:ident),* $(,)?])?
        $(, skip: [$($k:ident),* $(,)?])? $(,)?
    ) => {{
        #[allow(dead_code, unused_variables)]
        fn all_listed(t: &$ty) {
            let $ty { $($f: _,)* $($($r: _,)*)? $($($k: _,)*)? } = t;
        }
        let get: fn(&mut State) -> &mut $ty = $get;
        let base_bytes = bytes(&$base);
        $(
            let mut s = $base.clone();
            Perturb::perturb(&mut get(&mut s).$f);
            if bytes(&s) == base_bytes {
                $failures.push(format!(
                    "{}.{} does not move the encoding",
                    stringify!($ty),
                    stringify!($f)
                ));
            }
        )*
        $($(
            let mut s = $base.clone();
            Perturb::perturb(&mut get(&mut s).$r);
            if bytes(&s) != base_bytes {
                $failures.push(format!(
                    "{}.{} is listed as rebuilt but moves the encoding",
                    stringify!($ty),
                    stringify!($r)
                ));
            }
        )*)?
    }};
}

/// The full state of the hash test's probe, with the one value the decode refuses put right:
/// a stat set value of `MaxHp` (its hit points live in the totals).
fn base() -> State {
    let mut s = live_full_state(&engine());
    s.players
        .career_mut(PlayerIdx::SOLO)
        .progress
        .as_mut()
        .unwrap()
        .set_tenths[StatEffect::MaxHp as usize] = 0;
    s
}

fn career(s: &State) -> &Career {
    s.players.career(PlayerIdx::SOLO)
}

fn solo(s: &State) -> &PlayerState {
    &s.players[PlayerIdx::SOLO]
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
    round_trip_hashed(s.orbs[0].hook.as_ref().unwrap());
    round_trip_hashed(&s.zones[0]);
    round_trip_hashed(&s.orbs[0].dir);
    for aim in [Aim::Entity(EntityId(4)), Aim::Fixed(Dir { x: 3, y: -4 })] {
        round_trip_hashed(&aim);
    }
    for origin in [
        Origin::Owner,
        Origin::At(crate::fixed::Point {
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
    each_field_moves_the_encoding!(failures, base, Hook, |s| s.orbs[0].hook.as_mut().unwrap(),
        fields: [program, path, left, owner, aim, damage, damage_type, breaks_charge, smash, combo]);
    each_field_moves_the_encoding!(failures, base, ZoneState, |s| &mut s.zones[0],
        fields: [owner, friendly, shape, at, dir, wind_left, active_left, wind_ticks, active_ticks, damage, damage_type, breaks_charge, warn_only, hits, program]);
    each_field_moves_the_encoding!(failures, base, Dir, |s| &mut s.players[PlayerIdx::SOLO].facing,
        fields: [x, y]);

    assert!(
        failures.is_empty(),
        "{} fields are not saved:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_agent_boss_and_status_types_of_the_full_state_round_trip() {
    let s = base();
    let agent = &s.enemies[0];
    round_trip_hashed(agent);
    round_trip_hashed(&agent.key);
    round_trip_hashed(&agent.order.unwrap());
    round_trip_hashed(&agent.rules[0]);
    round_trip_hashed(&agent.elite.unwrap());
    round_trip_hashed(&agent.leave.unwrap());
    round_trip_hashed(agent.shrine.as_deref().unwrap());
    round_trip_hashed(&agent.statuses);
    let boss = s.boss.as_ref().unwrap();
    round_trip_hashed(boss);
    round_trip_hashed(&boss.parts[0]);
    let status = solo(&s).statuses[0];
    round_trip_hashed(&status);
    round_trip_hashed(&status.dot.unwrap());
    round_trip_hashed(&s.echoes[0]);
    for kind in StatusKind::ALL {
        round_trip_hashed(&kind);
    }
    for source in [
        AgentSource::Companion,
        AgentSource::Wave,
        AgentSource::Summoned,
        AgentSource::BossRule,
        AgentSource::Guardian,
        AgentSource::Escort,
    ] {
        round_trip_hashed(&source);
    }
}

#[test]
fn a_cut_or_damaged_agent_or_boss_never_panics_a_decode() {
    let s = base();
    never_panics(&s.enemies[0]);
    never_panics(s.boss.as_ref().unwrap());
    never_panics(&s.echoes[0]);
}

#[test]
fn the_longest_legal_lists_round_trip_and_one_more_is_refused() {
    let s = base();
    let id = |n: u32| EntityId(n);

    let mut shrine = s.enemies[0].shrine.clone().unwrap();
    shrine.guardians = (0..16).map(id).collect();
    round_trip(&*shrine);
    shrine.guardians.push(id(16));
    assert_eq!(
        from_bytes::<ShrineState>(&to_bytes(&*shrine)),
        Err(DecodeError::Invalid("list too long"))
    );

    let mut boss = s.boss.clone().unwrap();
    let part = boss.parts[0].clone();
    boss.parts = vec![part.clone(); 8];
    round_trip(&boss);
    boss.parts.push(part);
    assert_eq!(
        from_bytes::<BossState>(&to_bytes(&boss)),
        Err(DecodeError::Invalid("list too long"))
    );

    let mut run = s.runs[0].clone();
    run.hits = (0..500).map(|n| (id(n), n)).collect();
    round_trip(&run);
    run.hits.push((id(500), 500));
    assert_eq!(
        from_bytes::<AttackRun>(&to_bytes(&run)),
        Err(DecodeError::Invalid("list too long"))
    );

    let mut zone = s.zones[0].clone();
    zone.hits = (0..500).map(|n| (id(n), n)).collect();
    round_trip(&zone);
    zone.hits.push((id(500), 500));
    assert!(from_bytes::<ZoneState>(&to_bytes(&zone)).is_err());

    // The path lists are inline, four steps at most: no more, no fewer.
    let mut task = s.tasks[0].clone();
    let step = task.base[0];
    task.base = std::iter::repeat_n(step, 4).collect();
    round_trip(&task);
    let mut hook = s.orbs[0].hook.clone().unwrap();
    hook.path = std::iter::repeat_n(step, 4).collect();
    round_trip(&hook);
    let steps: crate::inline::InlineVec<PathStep, 4> = std::iter::repeat_n(step, 4).collect();
    round_trip(&steps);
    let mut five = to_bytes(&steps);
    five[0] = 5;
    five.extend(to_bytes(&step));
    assert_eq!(
        from_bytes::<crate::inline::InlineVec<PathStep, 4>>(&five),
        Err(DecodeError::Invalid("list too long"))
    );

    // The inline lists hold as many as their type: no more, no fewer.
    let rule = s.enemies[0].rules[0];
    let rules: Rules = std::iter::repeat_n(rule, 8).collect();
    round_trip(&rules);
    let status = solo(&s).statuses[0];
    let statuses: Statuses = std::iter::repeat_n(status, StatusKind::COUNT).collect();
    round_trip(&statuses);
    let frame = s.tasks[0].frames[0];
    let frames: Frames = std::iter::repeat_n(frame, 8).collect();
    round_trip(&frames);
    let mut nine = to_bytes(&frames);
    nine[0] = 9;
    nine.extend(to_bytes(&frame));
    assert_eq!(
        from_bytes::<Frames>(&nine),
        Err(DecodeError::Invalid("list too long"))
    );
}

#[test]
fn every_agent_boss_and_status_field_moves_the_encoding() {
    let base = base();
    let mut failures: Vec<String> = Vec::new();

    each_field_moves_the_encoding!(failures, base, BossState, |s| s.boss.as_mut().unwrap(),
        fields: [id, boss, origin, no_call, kind, at, hp, max_hp, defence, statuses, phase, phase_age, invulnerable, step, wait_left, order, rules, parts, elites_called, age]);
    each_field_moves_the_encoding!(failures, base, PartState, |s| &mut s.boss.as_mut().unwrap().parts[0],
        fields: [id, name, kind, hp, max_hp, defence, offset, radius, statuses]);
    each_field_moves_the_encoding!(failures, base, AgentKey, |s| &mut s.enemies[0].key,
        fields: [source, index, rule]);
    each_field_moves_the_encoding!(failures, base, AgentState, |s| &mut s.enemies[0],
        fields: [id, def, kind, key, at, hp, max_hp, defence, age, rules, statuses, aggro, order, orbit_angle, crowded, burst, stun_immune, elite, leave, shrine]);
    each_field_moves_the_encoding!(failures, base, ShrineState, |s| s.enemies[0].shrine.as_deref_mut().unwrap(),
        fields: [fight, guardians]);
    each_field_moves_the_encoding!(failures, base, LeaveState, |s| s.enemies[0].leave.as_mut().unwrap(),
        fields: [disturbed, go]);
    each_field_moves_the_encoding!(failures, base, EliteState, |s| s.enemies[0].elite.as_mut().unwrap(),
        fields: [home, called, boss_call, disguised]);
    each_field_moves_the_encoding!(failures, base, MoveOrder, |s| s.enemies[0].order.as_mut().unwrap(),
        fields: [to, ticks_left]);
    each_field_moves_the_encoding!(failures, base, RuleState, |s| &mut s.enemies[0].rules[0],
        fields: [cooldown_left, charges_left, fired]);
    each_field_moves_the_encoding!(failures, base, StatusState, |s| &mut s.players[PlayerIdx::SOLO].statuses[0],
        fields: [kind, source, remaining, total, stacks, value_pct, dot]);
    each_field_moves_the_encoding!(failures, base, DotState, |s| s.players[PlayerIdx::SOLO].statuses[0].dot.as_mut().unwrap(),
        fields: [damage, damage_type, atk_up_pct, type_bonus_pct, interval, next_in]);
    each_field_moves_the_encoding!(failures, base, Echo, |s| &mut s.echoes[0],
        fields: [source, target, damage, damage_type]);

    assert!(
        failures.is_empty(),
        "{} fields are not saved:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The live state saves four generator words after the tick, `spawn, attack, loot, horde`; the
/// offer generator is each career's, in its player's block.
#[test]
fn the_streams_are_four_words_in_order() {
    let mut s = base();
    s.rng = Streams::new(7);
    let words = [
        s.rng.spawn.state(),
        s.rng.attack.state(),
        s.rng.loot.state(),
        s.rng.horde.state(),
    ];
    for (i, w) in words.iter().enumerate() {
        assert!(!words[..i].contains(w), "the four streams differ");
    }
    // The tick is a `u32`; the streams follow it; the outcome byte follows them.
    let mut expected = Vec::new();
    for w in words {
        expected.extend_from_slice(&w.to_le_bytes());
    }
    assert_eq!(expected.len(), 32);
    s.outcome = Outcome::Failed;
    let b = bytes(&s);
    assert_eq!(b[..4], s.tick.to_le_bytes());
    assert_eq!(b[4..36], expected[..]);
    assert_eq!(b[36], 2, "the outcome follows the fourth word");

    let base = base();
    let mut failures: Vec<String> = Vec::new();
    each_field_moves_the_encoding!(failures, base, Streams, |s| &mut s.rng,
        fields: [spawn, attack, loot, horde]);
    assert!(failures.is_empty(), "{failures:?}");
}

/// A live run, not a built state: a few hundred ticks of the bundled `time_limit` mode with its
/// waves, and at every tick each entity-side value the engine holds round-trips, and so does the
/// whole state. A short run does not reach every type (`base` covers the rest); the kinds seen
/// are counted below.
#[test]
fn the_values_of_a_live_run_round_trip_at_every_tick() {
    let mut e: Engine = super::live_support::live_bundled_run("normal", 7);
    let fresh = e.run.live().clone();
    // agents, parts and bosses, tasks, runs, orbs, zones, echoes
    let mut seen = [0usize; 7];
    for _ in 0..900 {
        e.step(&[Input::default(), Default::default(), Default::default()]);
        let s = e.run.live();
        let saved = bytes(s);
        let back = decoded(&saved, &fresh).expect("the state decodes");
        assert_eq!(bytes(&back), saved);
        assert_eq!(hash_of(&back), hash_of(s));
        for a in s.companions.iter().chain(&s.enemies) {
            round_trip_hashed(a);
            seen[0] += 1;
        }
        if let Some(b) = &s.boss {
            round_trip_hashed(b);
            seen[1] += 1;
        }
        for t in &s.tasks {
            round_trip_hashed(t);
            seen[2] += 1;
        }
        for r in &s.runs {
            round_trip_hashed(r);
            seen[3] += 1;
        }
        for o in &s.orbs {
            round_trip_hashed(o);
            seen[4] += 1;
        }
        for z in &s.zones {
            round_trip_hashed(z);
            seen[5] += 1;
        }
        for x in &s.echoes {
            round_trip_hashed(x);
            seen[6] += 1;
        }
    }
    println!("seen: agents, bosses, tasks, runs, orbs, zones, echoes = {seen:?}");
    assert!(seen[0] > 0, "the run holds agents");
}

#[test]
fn the_player_side_types_of_the_full_state_round_trip() {
    let s = base();
    let p = solo(&s);
    round_trip_hashed(p);
    round_trip_hashed(&p.combo);
    round_trip_hashed(p.dash.as_ref().unwrap());
    round_trip_hashed(p.skill.as_ref().unwrap());
    round_trip_hashed(p.pull.as_ref().unwrap());
    round_trip_hashed(p.shield.as_ref().unwrap());
    round_trip_hashed(&p.orbits[0]);
    round_trip_hashed(&p.runes[0]);
    never_panics(p);
}

#[test]
fn a_player_list_past_its_cap_is_refused() {
    // The most the evade can recover at once, the orbits and the runes a run can hold.
    let mut p = solo(&base()).clone();
    p.evade_recovering = vec![1; 4];
    assert_eq!(
        from_bytes::<PlayerState>(&to_bytes(&p)),
        Err(DecodeError::Invalid("list too long"))
    );
    let mut p = solo(&base()).clone();
    p.orbits = vec![p.orbits[0]; 5];
    assert_eq!(
        from_bytes::<PlayerState>(&to_bytes(&p)),
        Err(DecodeError::Invalid("list too long"))
    );
    let mut p = solo(&base()).clone();
    p.runes = vec![p.runes[0]; 8];
    assert_eq!(
        from_bytes::<PlayerState>(&to_bytes(&p)),
        Err(DecodeError::Invalid("list too long"))
    );
}

#[test]
fn the_progression_types_of_the_full_state_round_trip() {
    let s = base();
    // `stats` and the horde's times are content, left empty by the decode: the originals are
    // put back before the comparison, as `decode_state` does.
    let p = career(&s).progress.as_ref().unwrap();
    let mut back: Progress = from_bytes(&to_bytes(p)).unwrap();
    assert!(back.stats.is_empty());
    back.stats = p.stats.clone();
    assert_eq!(&back, p);
    assert_eq!(hash_of(&back), hash_of(p));
    let h = s.horde.as_ref().unwrap();
    let mut back: Horde = from_bytes(&to_bytes(h)).unwrap();
    assert!(back.boss_ticks.is_empty() && back.no_call_ticks.is_empty());
    assert!(back.mid_boss_hp_pct.is_empty());
    (back.boss_ticks, back.no_call_ticks, back.mid_boss_hp_pct) = (
        h.boss_ticks.clone(),
        h.no_call_ticks.clone(),
        h.mid_boss_hp_pct.clone(),
    );
    assert_eq!(&back, h);
    assert_eq!(hash_of(&back), hash_of(h));

    // The star field has no codec of its own: the whole state carries it.
    let f = s.stars.as_ref().unwrap();
    let fresh = engine().run.live().clone();
    let saved = bytes(&s);
    let back = decoded(&saved, &fresh).expect("the full state decodes");
    assert_eq!(bytes(&back), saved);
    assert_eq!(hash_of(&back), hash_of(&s));
    let g = back.stars.as_ref().unwrap();
    assert_eq!(
        (g.misses, &g.on_map, g.rarity, g.rune_pool, &g.pool),
        (f.misses, &f.on_map, f.rarity, f.rune_pool, &f.pool)
    );
    round_trip_hashed(&f.on_map[0]);
    round_trip_hashed(&s.heal_pickups.as_ref().unwrap()[0]);
    let e = career(&s).expertise.as_ref().unwrap();
    round_trip(e);
    let o = career(&s).offers.as_ref().unwrap();
    round_trip_hashed(o);
    never_panics(e);
    never_panics(o);
}

/// The star field's bytes, found in the state's: the `Option` tag, `misses`, `on_map`,
/// `rarity`, `rune_pool`, `pool`, in that order, written out by hand below; and the tail of
/// the offers (`set_counts`) the same way.
#[test]
fn the_star_field_has_its_layout_and_its_caps() {
    let mut s = base();
    {
        let f = s.stars.as_mut().unwrap();
        f.on_map.clear();
        f.pool.clear();
        f.misses = 1;
        f.rarity = (Rarity::Uncommon, Rarity::Epic);
        f.rune_pool = 5;
    }
    let with = bytes(&s);
    // Without the field, and without the offers that go with it: the bytes are the same up to
    // the field's `Option` tag.
    let mut none = s.clone();
    none.stars = None;
    none.players.career_mut(PlayerIdx::SOLO).offers = None;
    let without = bytes(&none);
    let at = with
        .iter()
        .zip(&without)
        .position(|(a, b)| a != b)
        .expect("the field is saved");
    let expected = [
        1, // Some
        1, 0, 0, 0, // misses
        0, 0, 0, 0, // on_map
        1, 3, // rarity
        5, // rune_pool
        0, 0, 0, 0, // pool
    ];
    assert_eq!(with[at..at + expected.len()], expected);

    // The caps: 64 stars on the map, 192 places in the pool.
    let fresh = engine().run.live().clone();
    let mut s = base();
    let star = s.stars.as_ref().unwrap().on_map[0];
    s.stars.as_mut().unwrap().on_map = vec![star; 64];
    assert!(decoded(&bytes(&s), &fresh).is_ok());
    s.stars.as_mut().unwrap().on_map = vec![star; 65];
    assert_eq!(
        decoded(&bytes(&s), &fresh).err(),
        Some(DecodeError::Invalid("list too long"))
    );
    let mut s = base();
    s.stars.as_mut().unwrap().pool = vec![0; 192];
    assert!(decoded(&bytes(&s), &fresh).is_ok());
    s.stars.as_mut().unwrap().pool = vec![0; 193];
    assert_eq!(
        decoded(&bytes(&s), &fresh).err(),
        Some(DecodeError::Invalid("list too long"))
    );

    // The offers: the engine's own value with `set_counts` set.
    let mut offers = career(&base()).offers.clone().unwrap();
    offers.set_counts = vec![3];
    let tail = [1, 0, 0, 0, 3]; // set_counts
    let b = to_bytes(&offers);
    assert_eq!(b, tail);
    round_trip(&offers);
    let mut o = offers.clone();
    o.set_counts = vec![0; 192];
    round_trip(&o);
    o.set_counts = vec![0; 193];
    assert_eq!(
        from_bytes::<Offers>(&to_bytes(&o)),
        Err(DecodeError::Invalid("list too long"))
    );
}

// ---- The perturbation of the types the hash test does not cover ----

impl Perturb for StunRules {
    fn perturb(&mut self) {
        self.immune_ticks.perturb();
    }
}

impl Perturb for Arc<KindTable> {
    fn perturb(&mut self) {
        let ids: Vec<crate::id::ContentId> = self
            .ids()
            .iter()
            .cloned()
            .chain([crate::id::ContentId::from("zz_other")])
            .collect();
        *self = Arc::new(KindTable::from_ids(&ids, crate::kinds::MAX_KINDS).unwrap());
    }
}

/// The scratch buffers, the match-up table and the runes have no way to change one number from
/// here; they are content or buffers (`decode_state` takes them from the fresh engine), so
/// their perturbation is no change, and listing them under `rebuilt` is the compile-time half
/// of the check.
impl Perturb for crate::sim::Scratch {
    fn perturb(&mut self) {}
}

impl Perturb for crate::match_ups::MatchUps {
    fn perturb(&mut self) {}
}

impl Perturb for Arc<crate::runes::Runes> {
    fn perturb(&mut self) {}
}

#[test]
fn every_saved_field_of_the_state_moves_the_encoding() {
    let base = base();
    let mut failures: Vec<String> = Vec::new();
    each_field_moves_the_encoding!(failures, base, State, |s| s,
        fields: [tick, rng, outcome, players, boss, companions, enemies, tasks, runs, ranged_cooling, orbs, zones, spawned, next_id, next_orb_group, heal_pickups, max_attack_speed_pct, enemy_attack_damage, max_rune_haste_pct, horde, hp_pct, damage_pct, crit_seed, echoes],
        rebuilt: [scratch, kinds, move_index, match_ups, runes, stun],
        // `stars` and each career's `offers` are saved together: perturbed as a pair below.
        skip: [stars]);
    each_field_moves_the_encoding!(failures, base, PlayerState, |s| &mut s.players[PlayerIdx::SOLO],
        fields: [at, hp, max_hp, defence, invulnerable, facing, target, evade_charges, evade_recovering, held, combo, dash, skill, pull, shield, orbits, range_pct, rune_haste_pct, pickup_radius_pct, skill_cooldown_pct, kill_heal, kill_heal_every, kills_counted, crit_chance_pct, crit_damage_pct, echo_hit_pct, burst_pct, burst_radius, burst_depth, burst_depth_extra, type_bonus_pct, runes, aggro, statuses, down_ticks, revive_hold, out]);
    each_field_moves_the_encoding!(failures, base, Career, |s| s.players.career_mut(PlayerIdx::SOLO),
        fields: [progress, offer, expertise, stardust, regen, equipment],
        skip: [offers]);
    each_field_moves_the_encoding!(failures, base, Equipment, |s| &mut s.players.career_mut(PlayerIdx::SOLO).equipment,
        fields: [inventory, slots, lumine, starlight, tree_points]);
    each_field_moves_the_encoding!(failures, base, EquipSlot, |s| &mut s.players.career_mut(PlayerIdx::SOLO).equipment.slots[0],
        fields: [kind, held, level, expertise]);
    each_field_moves_the_encoding!(failures, base, ComboState, |s| &mut s.players[PlayerIdx::SOLO].combo,
        fields: [path, depth, dash, buffered, running, window_left, dash_strike_left, charge, hold, released, hand, auto_path, auto_depth, levels, auto_levels, charge_auto]);
    each_field_moves_the_encoding!(failures, base, SkillSlot, |s| s.players[PlayerIdx::SOLO].skill.as_mut().unwrap(),
        fields: [index, level, cooldown_left]);
    each_field_moves_the_encoding!(failures, base, OrbitState, |s| &mut s.players[PlayerIdx::SOLO].orbits[0],
        fields: [reward, orbs, round, cooldown_left, angle]);
    each_field_moves_the_encoding!(failures, base, HeldRune, |s| &mut s.players[PlayerIdx::SOLO].runes[0],
        fields: [rune, cooldown_left]);
    each_field_moves_the_encoding!(failures, base, PullState, |s| s.players[PlayerIdx::SOLO].pull.as_mut().unwrap(),
        fields: [at, radius, strength, ticks_left]);
    each_field_moves_the_encoding!(failures, base, ShieldState, |s| s.players[PlayerIdx::SOLO].shield.as_mut().unwrap(),
        fields: [amount, ticks_left]);
    each_field_moves_the_encoding!(failures, base, DashState, |s| s.players[PlayerIdx::SOLO].dash.as_mut().unwrap(),
        fields: [dir, done, left]);
    each_field_moves_the_encoding!(failures, base, Progress, |s| s.players.career_mut(PlayerIdx::SOLO).progress.as_mut().unwrap(),
        fields: [level, xp, priority, spent, totals, set_tenths],
        rebuilt: [stats]);
    each_field_moves_the_encoding!(failures, base, StarField, |s| s.stars.as_mut().unwrap(),
        fields: [misses, on_map, rarity, rune_pool, pool]);
    each_field_moves_the_encoding!(failures, base, StarState, |s| &mut s.stars.as_mut().unwrap().on_map[0],
        fields: [id, at, rarity]);
    each_field_moves_the_encoding!(failures, base, HealPickup, |s| &mut s.heal_pickups.as_mut().unwrap()[0],
        fields: [id, at]);
    each_field_moves_the_encoding!(failures, base, ExpertiseGauge, |s| s.players.career_mut(PlayerIdx::SOLO).expertise.as_mut().unwrap(),
        fields: [level, points, next, range_pct, style, path]);
    each_field_moves_the_encoding!(failures, base, Horde, |s| s.horde.as_mut().unwrap(),
        fields: [spawned, used, held],
        rebuilt: [boss_ticks, no_call_ticks, mid_boss_hp_pct]);

    // The pair: the star field and the careers' offers are saved together; dropping both moves
    // the bytes.
    let mut s = base.clone();
    s.stars = None;
    s.players.career_mut(PlayerIdx::SOLO).offers = None;
    if bytes(&s) == bytes(&base) {
        failures.push("State.stars with Career.offers does not move the encoding".into());
    }
    // The offers: `set_counts`, on the engine's own value.
    for (name, edit) in [(
        "set_counts",
        (|o: &mut Offers| o.set_counts.perturb()) as fn(&mut Offers),
    )] {
        let mut s = base.clone();
        edit(
            s.players
                .career_mut(PlayerIdx::SOLO)
                .offers
                .as_mut()
                .unwrap(),
        );
        if bytes(&s) == bytes(&base) {
            failures.push(format!("Offers.{name} does not move the encoding"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fields:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
