//! The live (`sim_version` 2) counterpart of the hand-built half of `savepoint_tests`: a decode
//! checks its lengths and indices against a fresh engine and refuses damaged bytes. The players
//! are per slot (`Players`, `Career`), the live `Streams` has four words and the live offers
//! are each career's, so only the offers that stay (`set_counts`) are edited, on a clone of the
//! engine's own `Offers`. The
//! frozen file guards version 1 and is not edited.

// The indices a test sets are the length of a table of a few hundred entries at most.
#![allow(clippy::cast_possible_truncation)]

use super::hash_coverage_live_tests::{engine, live_full_state};
use super::run::live::*;
use super::{Engine, Outcome, Run};
use crate::id::{ContentId, PlayerIdx};
use crate::ledger::{Counted, RunLedger};
use crate::savepoint::{DecodeError, encode_state};

type Decoded = Result<(State, RunLedger, Counted), DecodeError>;

/// `decode_state` of an engine that runs version 2, the rules these tests edit.
fn decode_state(bytes: &[u8], fresh: &Engine) -> Decoded {
    let (saved, ledger, counted, _) = crate::savepoint::decode_state(bytes, fresh)?;
    Ok((saved.v2(), ledger, counted))
}

/// An engine holding the full state, a ledger and tallies with something in each field.
fn saved() -> Engine {
    let mut e = engine();
    let mut full = live_full_state(&e);
    // Two things the fixture sets for the hash that a load refuses: the hit-point entry of the
    // stat set (the set's hit points live in the totals), and a rune the main character fires
    // with no rune slot filled (`runes_match_slots`)
    full.players[PlayerIdx::SOLO].runes.clear();
    // ... the fixture's expertise gauge has taken more of the tree than its level earned (the
    // engine's own is a legal one) ...
    full.players.career_mut(PlayerIdx::SOLO).expertise = e
        .run
        .live()
        .players
        .career(PlayerIdx::SOLO)
        .expertise
        .clone();
    // ... and the same star in the slot and in the Lumine slot (two equipped stars share an option).
    full.players
        .career_mut(PlayerIdx::SOLO)
        .equipment
        .lumine
        .as_mut()
        .unwrap()
        .held = None;
    full.players
        .career_mut(PlayerIdx::SOLO)
        .progress
        .as_mut()
        .unwrap()
        .set_tenths[crate::run_stats::StatEffect::MaxHp as usize] = 0;
    e.run.set_live_state(full);
    // The index is derived from the runs; the fixture's was lowered by hand.
    let Run::V2 { world, state } = &mut e.run else {
        unreachable!("a test engine runs version 2")
    };
    state.move_index.rebuild(world, &state.runs);
    e.ledger.add_kill("kills.giant_sword.enemy".into());
    e.ledger.add_kill("rune_taken.burn".into());
    e.ledger.add_bestiary_bit(&ContentId::from("slime"), 3);
    e.ledger.picks.push(crate::ledger::StarPick {
        rarity: crate::rewards::Rarity::Rare,
        name: Some("Vega".into()),
    });
    e.ledger.picks.push(crate::ledger::StarPick {
        rarity: crate::rewards::Rarity::Common,
        name: None,
    });
    e.ledger.add_skill_use(ContentId::from("dash"));
    e.counted.kills = 5;
    e.counted.named.push("Vega".into());
    e
}

/// `edit` applied to a copy of the full state, encoded, and decoded against the unedited one.
fn decode_edited(edit: impl FnOnce(&mut State)) -> Decoded {
    let fresh = saved();
    let mut edited = fresh.clone();
    edit(edited.run.live_mut());
    decode_state(&encode_state(&edited), &fresh)
}

/// What a decode gives, as the original engine's `Debug` text of the same parts.
fn debug_of(s: &State) -> String {
    format!(
        "{:?} {:?} {:?} {:?} {:?}",
        s.kinds, s.match_ups, s.runes, s.stun, s.move_index
    )
}

fn invalid(what: &'static str) -> Result<(), DecodeError> {
    Err(DecodeError::Invalid(what))
}

#[test]
fn round_trip_hand_built() {
    let e = saved();
    let bytes = encode_state(&e);
    let (state, ledger, counted) = decode_state(&bytes, &e).unwrap();
    assert_eq!(ledger, e.ledger);
    assert_eq!(counted, e.counted);
    assert_eq!(debug_of(&state), debug_of(e.run.live()));
    let mut back = e.clone();
    back.run.set_live_state(state);
    assert_eq!(back.state_hash(), e.state_hash());
    assert_eq!(encode_state(&back), bytes);
}

#[test]
fn shape_is_checked() {
    let career =
        |f: &dyn Fn(&mut Career)| decode_edited(|s| f(s.players.career_mut(PlayerIdx::SOLO)));
    let progress = |f: fn(&mut Progress)| career(&|c: &mut Career| f(c.progress.as_mut().unwrap()));
    let offers = |f: fn(&mut Offers)| career(&|c: &mut Career| f(c.offers.as_mut().unwrap()));
    let stars = |f: fn(&mut StarField)| decode_edited(|s| f(s.stars.as_mut().unwrap()));
    assert_eq!(
        decode_edited(|s| s.spawned.push(0)).map(drop),
        invalid("spawned length differs from the content")
    );
    assert_eq!(
        progress(|p| p.spent.push(0)).map(drop),
        invalid("spent length differs from the content")
    );
    assert_eq!(
        progress(|p| p.spent.clear()).map(drop),
        invalid("spent length differs from the content")
    );
    assert_eq!(
        stars(|f| f.pool.push(0)).map(drop),
        invalid("star pool length differs from the content")
    );
    // The star-offer list is each career's.
    assert_eq!(
        offers(|o| o.set_counts.push(0)).map(drop),
        invalid("star set_counts length differs from the content")
    );
    assert_eq!(
        decode_edited(|s| s.heal_pickups = None).map(drop),
        invalid("heal_pickups presence differs from the content")
    );
    assert_eq!(
        decode_edited(|s| s.horde = None).map(drop),
        invalid("horde presence differs from the content")
    );
    assert_eq!(
        decode_edited(|s| s.max_attack_speed_pct += 1).map(drop),
        invalid("max_attack_speed_pct differs from the content")
    );
    // The tallies' lengths come from the rewards file too.
    let fresh = saved();
    let mut edited = fresh.clone();
    edited.counted.taken.push(0);
    assert_eq!(
        decode_state(&encode_state(&edited), &fresh).map(drop),
        invalid("counted taken length differs from the content")
    );
}

/// One saved index: how to set it to a given value, and what it indexes.
struct Index {
    message: &'static str,
    set: fn(&mut State, usize),
    /// The length of what it indexes, in an engine.
    len: fn(&Engine) -> usize,
}

fn indices() -> Vec<Index> {
    use crate::kinds::KindIndex;
    use crate::sim::{AttackIndex, BossIndex, DefIndex, ProgramIndex};
    // The offer's candidates are not listed: they are each career's offer, which this file
    // does not name (the frozen `indices` lists them for version 1).
    vec![
        Index {
            message: "agent def out of range",
            set: |s, n| s.enemies[0].def = DefIndex(n as u16),
            len: |e| e.run.live_world().defs.len(),
        },
        Index {
            message: "agent kind out of range",
            set: |s, n| s.enemies[0].kind = KindIndex(n as u16),
            len: |e| e.run.live().kinds.len(),
        },
        Index {
            message: "boss index out of range",
            set: |s, n| s.boss.as_mut().unwrap().boss = BossIndex(n as u16),
            len: |e| e.run.live_world().bosses.len(),
        },
        Index {
            message: "boss kind out of range",
            set: |s, n| s.boss.as_mut().unwrap().kind = KindIndex(n as u16),
            len: |e| e.run.live().kinds.len(),
        },
        Index {
            message: "part kind out of range",
            set: |s, n| s.boss.as_mut().unwrap().parts[0].kind = KindIndex(n as u16),
            len: |e| e.run.live().kinds.len(),
        },
        Index {
            message: "run attack out of range",
            set: |s, n| s.runs[0].attack = AttackIndex(n as u16),
            len: |e| e.run.live_world().attacks.len(),
        },
        Index {
            message: "task program out of range",
            set: |s, n| s.tasks[0].program = ProgramIndex(n as u32),
            len: |e| e.run.live_world().emitters.len(),
        },
        Index {
            message: "orb program out of range",
            set: |s, n| s.orbs[0].program = ProgramIndex(n as u32),
            len: |e| e.run.live_world().emitters.len(),
        },
        Index {
            message: "zone program out of range",
            set: |s, n| s.zones[0].program = ProgramIndex(n as u32),
            len: |e| e.run.live_world().emitters.len(),
        },
        Index {
            message: "hook program out of range",
            set: |s, n| s.orbs[0].hook.as_mut().unwrap().program = ProgramIndex(n as u32),
            len: |e| e.run.live_world().emitters.len(),
        },
        Index {
            message: "orbit reward out of range",
            set: |s, n| s.players[PlayerIdx::SOLO].orbits[0].reward = n as u16,
            len: |e| e.content.rewards().rewards().len(),
        },
        Index {
            message: "skill index out of range",
            set: |s, n| s.players[PlayerIdx::SOLO].skill.as_mut().unwrap().index = n as u16,
            len: |e| e.content.rewards().stage_skills().skills().len(),
        },
        Index {
            message: "star pool entry out of range",
            set: |s, n| s.stars.as_mut().unwrap().pool[0] = n as u16,
            len: |e| e.content.rewards().rewards().len(),
        },
    ]
}

#[test]
fn indices_are_checked() {
    let fresh = saved();
    for index in indices() {
        let len = (index.len)(&fresh);
        assert!(
            len > 0,
            "{}: the full state's engine indexes something",
            index.message
        );
        assert_eq!(
            decode_edited(|s| (index.set)(s, len)).map(drop),
            invalid(index.message),
            "{} at {len}",
            index.message
        );
        assert!(
            decode_edited(|s| (index.set)(s, len - 1)).is_ok(),
            "{} at {}",
            index.message,
            len - 1
        );
    }
}

#[test]
fn a_run_phase_past_its_attack_is_refused_before_the_move_index_is_built() {
    let fresh = saved();
    let phases = fresh.run.live_world().attacks[usize::from(fresh.run.live().runs[0].attack.0)]
        .phases
        .len();
    assert_eq!(
        decode_edited(|s| s.runs[0].phase = phases as u32).map(drop),
        invalid("run phase out of range")
    );
    assert!(decode_edited(|s| s.runs[0].phase = phases as u32 - 1).is_ok());
}

/// A queued repeat names the attack and the phase it deals from: an attack at the table's length
/// or a phase at its attack's phase count is refused; the last of each loads. The queue is empty
/// in every shipped run, so no pinned byte holds one.
#[test]
fn a_repeat_past_its_attack_or_phase_is_refused() {
    use crate::sim::AttackIndex;
    use crate::sim::state::{Repeat, RepeatAnchor};
    let fresh = saved();
    let attacks = fresh.run.live_world().attacks.len();
    let phases = |attack: usize| fresh.run.live_world().attacks[attack].phases.len();
    let repeat = |attack: usize, phase: usize| Repeat {
        owner: 0,
        due: 9,
        percent: 50,
        proc_pct: None,
        anchor: RepeatAnchor::HitList(Default::default()),
        attack: AttackIndex(attack as u16),
        phase: phase as u8,
        damage: 10,
        width_pct: 0,
    };
    let last = attacks - 1;
    let edit = |r: Repeat| decode_edited(move |s| s.repeats = vec![r]).map(drop);
    assert!(edit(repeat(last, phases(last) - 1)).is_ok());
    assert_eq!(
        edit(repeat(attacks, 0)),
        invalid("repeat attack out of range")
    );
    assert_eq!(
        edit(repeat(last, phases(last))),
        invalid("repeat phase out of range")
    );
}

/// The first byte at which two encodings differ.
fn first_diff(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .expect("they differ")
}

fn encoded(edit: impl FnOnce(&mut State)) -> Vec<u8> {
    let mut e = saved();
    edit(e.run.live_mut());
    encode_state(&e)
}

#[test]
fn bytes_are_checked() {
    let fresh = saved();
    let empty = engine();
    let bare = {
        let mut e = saved();
        let s = e.run.live_mut();
        let career = s.players.career_mut(PlayerIdx::SOLO);
        career.progress = None;
        // The star field and a player's offers go together.
        career.offers = None;
        s.stars = None;
        e
    };
    for (name, e) in [("full", &fresh), ("fresh", &empty), ("bare", &bare)] {
        let bytes = encode_state(e);
        assert!(decode_state(&bytes, e).is_ok() || name == "bare", "{name}");
        for n in 0..bytes.len() {
            let r = decode_state(&bytes[..n], e);
            assert!(
                matches!(r, Err(DecodeError::Truncated | DecodeError::Invalid(_))),
                "{name}: a prefix of {n} bytes decoded"
            );
        }
    }
    let mut bytes = encode_state(&fresh);
    bytes.push(0);
    assert_eq!(
        decode_state(&bytes, &fresh).map(drop),
        Err(DecodeError::Invalid("trailing bytes"))
    );

    // A byte patched in an otherwise valid encoding: found as the first byte two encodings of
    // the same field with different values differ at.
    let patch = |a: Vec<u8>, b: Vec<u8>, bytes: &[u8]| {
        let mut out = a.clone();
        let at = first_diff(&a, &b);
        out[at..at + bytes.len()].copy_from_slice(bytes);
        out
    };
    let long = patch(
        encoded(|s| s.ranged_cooling.clear()),
        encoded(|s| s.ranged_cooling.truncate(1)),
        &[0xff; 4],
    );
    assert_eq!(
        decode_state(&long, &fresh).map(drop),
        invalid("list too long")
    );
    use crate::rewards::Rarity;
    let outcome = patch(
        encoded(|s| s.outcome = Outcome::Running),
        encoded(|s| s.outcome = Outcome::Failed),
        &[3],
    );
    assert!(matches!(
        decode_state(&outcome, &fresh),
        Err(DecodeError::Invalid(_))
    ));
    let rarity = patch(
        encoded(|s| s.stars.as_mut().unwrap().rarity.0 = Rarity::Common),
        encoded(|s| s.stars.as_mut().unwrap().rarity.0 = Rarity::Uncommon),
        &[6],
    );
    assert!(matches!(
        decode_state(&rarity, &fresh),
        Err(DecodeError::Invalid(_))
    ));
    // The live state has no `pick_stats`; the horde's `used` flags are the saved bools.
    let boolean = patch(
        encoded(|s| s.horde.as_mut().unwrap().used[0] = false),
        encoded(|s| s.horde.as_mut().unwrap().used[0] = true),
        &[2],
    );
    assert_eq!(decode_state(&boolean, &fresh).map(drop), invalid("bool"));
}

#[test]
fn a_decoded_ledger_goes_on_counting_where_the_original_does() {
    let mut e = engine();
    // A kill key that sorts before the weapon's own shifts the kill indices (`add_kill`).
    e.ledger.add_kill("rune_taken.burn".into());
    e.ledger.add_kill("a_first_key".into());
    let (_, ledger, _) = decode_state(&encode_state(&e), &e).unwrap();
    let mut original = e.ledger.clone();
    let mut resumed = ledger;
    assert_eq!(resumed, original);
    for key in ["a_first_key", "kills.giant_sword.enemy", "b_new_key"] {
        original.add_kill(key.into());
        resumed.add_kill(key.into());
    }
    assert_eq!(resumed, original);
    assert_eq!(resumed.stat("kills.giant_sword.enemy"), 1);
}
