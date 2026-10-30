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
use crate::rewards::Rarity;
use crate::savepoint::testing::{from_bytes, round_trip, to_bytes};
use crate::savepoint::{Decode, DecodeError, Encode};
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
    let status = s.player.statuses[0];
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
fn the_tags_of_the_status_and_source_enums_are_their_hash_numbers() {
    assert_eq!(to_bytes(&StatusKind::Slow), [2]);
    assert_eq!(to_bytes(&StatusKind::Stun), [6]);
    assert_eq!(to_bytes(&AgentSource::Escort), [5]);
    assert!(from_bytes::<StatusKind>(&[7]).is_err());
    assert!(from_bytes::<AgentSource>(&[6]).is_err());
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

    let mut task = s.tasks[0].clone();
    let step = task.base[0];
    task.base = vec![step; 4];
    round_trip(&task);
    task.base.push(step);
    assert_eq!(
        from_bytes::<AttackTask>(&to_bytes(&task)),
        Err(DecodeError::Invalid("list too long"))
    );
    let mut hook = s.orbs[0].hook.clone().unwrap();
    hook.path = vec![step; 4];
    round_trip(&*hook);
    hook.path.push(step);
    assert!(from_bytes::<Hook>(&to_bytes(&*hook)).is_err());

    // The inline lists hold as many as their type: no more, no fewer.
    let rule = s.enemies[0].rules[0];
    let rules: Rules = std::iter::repeat_n(rule, 8).collect();
    round_trip(&rules);
    let status = s.player.statuses[0];
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
    each_field_moves_the_encoding!(failures, base, StatusState, |s| &mut s.player.statuses[0],
        fields: [kind, source, remaining, total, stacks, value_pct, dot]);
    each_field_moves_the_encoding!(failures, base, DotState, |s| s.player.statuses[0].dot.as_mut().unwrap(),
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

#[test]
fn the_streams_are_five_words_in_order() {
    let s = base();
    round_trip_hashed(&s.rng);
    let bytes = to_bytes(&s.rng);
    assert_eq!(bytes.len(), 40);
    assert_eq!(bytes[..8], s.rng.spawn.state().to_le_bytes());
    assert_eq!(bytes[32..], s.rng.horde.state().to_le_bytes());
    never_panics(&s.rng);
    let mut failures: Vec<String> = Vec::new();
    each_field_moves_the_encoding!(failures, s, Streams, |s| &mut s.rng,
        fields: [spawn, attack, loot, offer, horde]);
    assert!(failures.is_empty(), "{failures:?}");
}

/// A live run, not a built state: a few hundred ticks of the bundled `time_limit` mode with its
/// waves, and at every tick each entity-side value the engine holds round-trips. A short run
/// does not reach every type (`full_state` covers the rest); the kinds seen are counted below.
#[test]
fn the_values_of_a_live_run_round_trip_at_every_tick() {
    use crate::pack::ContentFiles;
    use crate::run_content::RunContent;
    use crate::time_limit::TimeLimit;
    use crate::version::Versions;

    let mode = TimeLimit::parse(ContentFiles::bundled_file("time_limit"), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let setup = RunSetup {
        schema_version: 1,
        sim_version: 1,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 7,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        stat_priority: Vec::new(),
        pick_stats: false,
        difficulty: "normal".into(),
    };
    let mut e = Engine::start_time_limit(&setup, RunContent::bundled(), &mode).unwrap();
    // agents, parts and bosses, tasks, runs, orbs, zones, echoes
    let mut seen = [0usize; 7];
    for _ in 0..900 {
        e.step(Input::default());
        let s = &e.state;
        round_trip_hashed(&s.rng);
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
    let p = &s.player;
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
fn the_combo_and_the_dash_have_their_layout() {
    let combo = ComboState {
        path: 5,
        depth: 2,
        dash: true,
        buffered: Some(true),
        running: false,
        window_left: 7,
        dash_strike_left: 8,
        charge: Some(3),
        hold: None,
        released: true,
        hand: false,
        auto_path: 9,
        auto_depth: 1,
        levels: 10,
        auto_levels: 11,
        charge_auto: true,
    };
    assert_eq!(
        to_bytes(&combo),
        [
            5, 0, 0, 0, 2, 1, 1, 1, 0, 7, 0, 0, 0, 8, 0, 0, 0, 1, 3, 0, 0, 0, 0, 1, 0, 9, 0, 0, 0,
            1, 10, 0, 0, 0, 11, 0, 0, 0, 1
        ]
    );
    let dash = DashState {
        dir: Dir { x: 3, y: -4 },
        done: 2,
        left: 1,
    };
    assert_eq!(
        to_bytes(&dash),
        [3, 0, 0, 0, 0xfc, 0xff, 0xff, 0xff, 2, 0, 0, 0, 1, 0, 0, 0]
    );
    round_trip(&combo);
    round_trip(&dash);
}

#[test]
fn a_player_list_past_its_cap_is_refused() {
    // The most the evade can recover at once, the orbits and the runes a run can hold.
    let mut p = base().player;
    p.evade_recovering = vec![1; 4];
    assert_eq!(
        from_bytes::<PlayerState>(&to_bytes(&p)),
        Err(DecodeError::Invalid("list too long"))
    );
    let mut p = base().player;
    p.orbits = vec![p.orbits[0]; 5];
    assert_eq!(
        from_bytes::<PlayerState>(&to_bytes(&p)),
        Err(DecodeError::Invalid("list too long"))
    );
    let mut p = base().player;
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
    let p = s.progress.as_ref().unwrap();
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

    let f = s.stars.as_ref().unwrap();
    round_trip_hashed(f);
    round_trip_hashed(&f.on_map[0]);
    round_trip_hashed(&f.due[0]);
    round_trip_hashed(f.offer.as_ref().unwrap());
    round_trip_hashed(&s.heal_pickups.as_ref().unwrap()[0]);
    let g = s.expertise.as_ref().unwrap();
    round_trip_hashed(g);
    round_trip_hashed(g.offer.as_ref().unwrap());
    never_panics(f);
    never_panics(g);
}

#[test]
fn the_star_field_has_its_layout_and_its_caps() {
    let mut f = base().stars.unwrap();
    f.on_map.clear();
    f.due.clear();
    f.offer = None;
    f.pool.clear();
    f.taken.clear();
    f.banished.clear();
    f.set_counts.clear();
    f.misses = 1;
    f.rerolls_left = 2;
    f.banishes_left = 3;
    f.extra_candidates = 4;
    f.rarity = (Rarity::Uncommon, Rarity::Epic);
    f.rune_pool = 5;
    assert_eq!(
        to_bytes(&f),
        [
            1, 0, 0, 0, // misses
            0, 0, 0, 0, // on_map
            0, 0, 0, 0, // due
            0, // offer
            2, 0, 0, 0, 3, 0, 0, 0, // rerolls_left, banishes_left
            4, // extra_candidates
            1, 3, // rarity
            5, // rune_pool
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, // pool, taken, banished, set_counts
        ]
    );
    round_trip(&f);
    let mut f = base().stars.unwrap();
    f.on_map = vec![f.on_map[0]; 65];
    assert_eq!(
        from_bytes::<StarField>(&to_bytes(&f)),
        Err(DecodeError::Invalid("list too long"))
    );
    let mut f = base().stars.unwrap();
    f.pool = vec![0; 193];
    assert_eq!(
        from_bytes::<StarField>(&to_bytes(&f)),
        Err(DecodeError::Invalid("list too long"))
    );
}
