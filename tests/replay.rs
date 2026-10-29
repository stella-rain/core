//! Replays (ADR-009, ADR-016): the packed input encoding, recording, verification and the
//! checks on untrusted replay files (ADR-020).

use crate::common;

use common::{CLEAR_TICK, clearing_inputs, idle, stage_bytes};
use serde_json::{Value, json};
use stella_rain_core::base64;
use stella_rain_core::content_hash::{content_hash, sha256, to_hex};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::replay::{
    HashesError, MAX_HASHES_BYTES, MAX_STAGE_REPLAY_BYTES, Replay, ReplayError, hash_from_hex,
    hash_to_hex, hashes_from_bytes, hashes_to_bytes, pack_inputs, record, verify, verify_against,
};
use stella_rain_core::rng::SplitMix64;
use stella_rain_core::validate;

fn recorded() -> (Vec<u8>, Replay, Vec<u8>) {
    let stage = stage_bytes();
    let replay = record(&stage, &clearing_inputs()).unwrap();
    let bytes = replay.to_bytes();
    (stage, replay, bytes)
}

fn with(replay: &Replay, change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut v = serde_json::to_value(replay).unwrap();
    change(&mut v);
    v.to_string().into_bytes()
}

// --- Packed inputs ---------------------------------------------------------------------------

fn replay_of(inputs: &[Input], ticks: u32) -> Replay {
    Replay {
        schema_version: 0,
        sim_version: 0,
        seed: 7,
        stage_sha256: String::new(),
        ticks,
        final_hash: hash_to_hex(0),
        inputs: pack_inputs(inputs),
    }
}

fn stick(move_x: i8, move_y: i8, evade: bool) -> Input {
    Input {
        move_x,
        move_y,
        evade,
        ..Input::default()
    }
}

/// `stick` with the buttons and switches of bits 1 to 5 of `bits` (ADR-071): `main`, `smash`,
/// `skill`, `assist`, `auto`.
fn buttons(input: Input, bits: u32) -> Input {
    Input {
        main: bits & 0b1 != 0,
        smash: bits & 0b10 != 0,
        skill: bits & 0b100 != 0,
        assist: bits & 0b1000 != 0,
        auto: bits & 0b1_0000 != 0,
        ..input
    }
}

#[test]
fn packed_inputs_round_trip_including_the_extremes() {
    let mut rng = SplitMix64::new(19);
    let mut inputs = vec![
        stick(-127, 127, true),
        stick(127, -127, false),
        stick(0, 0, false),
        stick(-1, 1, true),
    ];
    for _ in 0..500 {
        // Any byte but 0x80, which is −128.
        let mut axis =
            || i8::from_ne_bytes([u8::try_from(rng.below(255)).unwrap().wrapping_add(0x81)]);
        let (move_x, move_y) = (axis(), axis());
        let input = stick(move_x, move_y, rng.below(2) == 0);
        inputs.push(buttons(input, rng.below(32)));
    }
    // Long runs of one input, and of nothing.
    inputs.extend(std::iter::repeat_n(Input::default(), 300));
    inputs.extend(std::iter::repeat_n(stick(0, -127, false), 17));
    let replay = replay_of(&inputs, u32::try_from(inputs.len()).unwrap());
    assert_eq!(replay.unpack_inputs(u32::MAX).unwrap(), inputs);
}

#[test]
fn steady_input_packs_into_a_few_bytes() {
    let held_key = vec![stick(127, 0, false); 18_000];
    assert!(pack_inputs(&held_key).len() <= 16);
    assert_eq!(pack_inputs(&[]), "");
}

#[test]
fn a_replay_of_never_repeating_input_fits_the_size_limit() {
    // The worst case: 18,000 ticks (five minutes), every input different from the last.
    let inputs: Vec<Input> = (0..18_000u32)
        .map(|t| stick(if t % 2 == 0 { -127 } else { 127 }, 0, t % 3 == 0))
        .collect();
    let mut replay = replay_of(&inputs, 18_000);
    replay.stage_sha256 = "0".repeat(64);
    assert!(
        replay.to_bytes().len() < MAX_STAGE_REPLAY_BYTES,
        "{} bytes",
        replay.to_bytes().len()
    );
}

#[test]
fn a_stick_at_minus_128_packs_as_minus_127() {
    // The engine runs both the same (`tests/controls.rs`), and a file holds one of them only.
    let inputs = [stick(-128, 0, false), stick(5, -128, true)];
    let replay = replay_of(&inputs, 2);
    assert_eq!(
        replay.unpack_inputs(2).unwrap(),
        [stick(-127, 0, false), stick(5, -127, true)]
    );
}

fn unpack_raw(bytes: &[u8], ticks: u32) -> Result<Vec<Input>, ReplayError> {
    Replay {
        inputs: base64::encode(bytes),
        ..replay_of(&[], ticks)
    }
    .unpack_inputs(u32::MAX)
}

#[test]
fn damaged_packed_inputs_are_refused_with_a_reason() {
    // One run of two ticks, flags `evade`, the stick at (−1, 127): [2, 0b1, 0xff, 0x7f].
    assert_eq!(
        unpack_raw(&[2, 1, 0xff, 0x7f], 2).unwrap(),
        [stick(-1, 127, true); 2]
    );
    let reason = |bytes: &[u8], ticks: u32| match unpack_raw(bytes, ticks) {
        Err(ReplayError::BadInputs(why)) => why,
        other => panic!("{bytes:?}: {other:?}"),
    };
    assert!(reason(&[0, 0, 0, 0], 1).contains("length 0"));
    // Inputs have one encoding: a run of the same input as the run before is two runs that
    // packing would have written as one.
    assert!(reason(&[1, 1, 0xff, 0x7f, 1, 1, 0xff, 0x7f], 2).contains("the run before"));
    assert_eq!(
        unpack_raw(&[1, 1, 0xff, 0x7f, 1, 0, 0xff, 0x7f], 2).unwrap(),
        [stick(-1, 127, true), stick(-1, 127, false)]
    );
    assert!(reason(&[3, 1, 0, 0], 2).contains("more inputs than ticks"));
    assert!(reason(&[1, 1, 0, 0], 2).contains("fewer inputs"));
    // Bits 1 to 5 are `main`, `smash`, `skill`, `assist` and `auto`; the rest are unknown.
    assert_eq!(
        unpack_raw(&[1, 0b11_1111, 0, 0], 1).unwrap(),
        [buttons(stick(0, 0, true), 0b1_1111)]
    );
    assert_eq!(
        unpack_raw(&[1, 0b10_1010, 0, 0], 1).unwrap(),
        [buttons(stick(0, 0, false), 0b1_0101)]
    );
    for flags in [0x40, 0x80, 0xc0] {
        assert!(reason(&[2, flags, 0, 0], 2).contains("unknown flags"));
    }
    assert!(reason(&[2, 1, 0], 2).contains("middle"));
    assert!(reason(&[2], 2).contains("middle"));
    assert!(
        reason(&[2, 1, 0, 0, 1], 3).contains("middle"),
        "trailing partial run"
    );
    // −128 is no stick position, on either axis.
    assert!(reason(&[1, 0, 0x80, 0], 1).contains("out of range"));
    assert!(reason(&[1, 0, 0, 0x80], 1).contains("out of range"));
    // Run lengths not in their shortest form, too long, or too large.
    assert!(reason(&[0x81, 0x00, 1, 0, 0], 1).contains("shortest"));
    assert!(reason(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01, 1, 0, 0], 1).contains("too"));
    assert!(reason(&[0xff, 0xff, 0xff, 0xff, 0x1f, 1, 0, 0], 1).contains("too large"));
    assert!(matches!(
        Replay {
            inputs: "not base64!".into(),
            ..replay_of(&[], 1)
        }
        .unpack_inputs(10),
        Err(ReplayError::BadInputs(_))
    ));
}

#[test]
fn a_hostile_tick_count_is_refused_before_anything_is_allocated() {
    for ticks in [601, u32::MAX] {
        let replay = Replay {
            ticks,
            ..replay_of(&[], 0)
        };
        assert_eq!(
            replay.unpack_inputs(600),
            Err(ReplayError::TooManyTicks { ticks, limit: 600 })
        );
    }
    // A run longer than the ticks, or than the room left, is refused without expanding it.
    let huge_run = [0xff, 0xff, 0xff, 0xff, 0x0f, 1, 0, 0];
    assert!(matches!(
        unpack_raw(&huge_run, 10),
        Err(ReplayError::BadInputs(_))
    ));
}

// --- The file --------------------------------------------------------------------------------

#[test]
fn the_replay_file_round_trips_and_is_strict() {
    let (_, replay, bytes) = recorded();
    assert_eq!(Replay::from_bytes(&bytes).unwrap(), replay);
    assert_eq!(bytes.last(), Some(&b'\n'));
    let extra = with(&replay, |v| v["unexpected"] = json!(1));
    assert!(matches!(
        Replay::from_bytes(&extra),
        Err(ReplayError::Damaged { .. })
    ));
    let missing = with(&replay, |v| {
        v.as_object_mut().unwrap().remove("inputs");
    });
    assert!(matches!(
        Replay::from_bytes(&missing),
        Err(ReplayError::Damaged { .. })
    ));
    assert!(matches!(
        Replay::from_bytes(b""),
        Err(ReplayError::Damaged { .. })
    ));
    assert!(matches!(
        Replay::from_bytes(b"[]"),
        Err(ReplayError::Damaged { .. })
    ));
}

#[test]
fn the_size_limit_applies_before_parsing() {
    assert_eq!(MAX_STAGE_REPLAY_BYTES, 256 * 1024);
    let (_, _, mut bytes) = recorded();
    bytes.resize(MAX_STAGE_REPLAY_BYTES, b' ');
    assert!(Replay::from_bytes(&bytes).is_ok());
    bytes.push(b' ');
    assert_eq!(
        Replay::from_bytes(&bytes),
        Err(ReplayError::TooLarge {
            size: MAX_STAGE_REPLAY_BYTES + 1,
            limit: MAX_STAGE_REPLAY_BYTES
        })
    );
    assert!(matches!(
        Replay::from_bytes(&vec![b'x'; MAX_STAGE_REPLAY_BYTES + 1]),
        Err(ReplayError::TooLarge { .. })
    ));
}

#[test]
fn hashes_are_sixteen_lowercase_hex_digits() {
    assert_eq!(hash_to_hex(0x1f), "000000000000001f");
    assert_eq!(hash_from_hex("000000000000001f"), Some(0x1f));
    assert_eq!(hash_from_hex(&hash_to_hex(u64::MAX)), Some(u64::MAX));
    for bad in [
        "",
        "1f",
        "000000000000001F",
        "+00000000000001f",
        "0x0000000000001f",
        "00000000000000000",
        "000000000000001g",
    ] {
        assert_eq!(hash_from_hex(bad), None, "{bad:?}");
    }
}

// --- Recording and verifying -----------------------------------------------------------------

#[test]
fn a_recorded_clear_verifies() {
    let (stage, replay, bytes) = recorded();
    assert_eq!(replay.ticks, CLEAR_TICK);
    assert_eq!(replay.stage_sha256, to_hex(&sha256(&stage)));
    let verified = verify(&stage, &bytes).unwrap();
    assert_eq!(verified.ticks, CLEAR_TICK);
    assert_eq!(hash_to_hex(verified.final_hash), replay.final_hash);
    assert_eq!(verified.content_hash, content_hash(&stage, &bytes));
    // The hash is the engine's own.
    let parsed = validate::parse_and_validate(&stage).unwrap();
    let mut engine = Engine::new(&parsed).unwrap();
    clearing_inputs().iter().for_each(|i| engine.step(*i));
    assert_eq!(engine.state_hash(), verified.final_hash);
}

#[test]
fn recording_stops_on_the_tick_the_run_ends() {
    let stage = stage_bytes();
    let mut inputs = clearing_inputs();
    inputs.resize(400, idle());
    let replay = record(&stage, &inputs).unwrap();
    assert_eq!(replay.ticks, CLEAR_TICK);
    assert_eq!(
        replay.unpack_inputs(600).unwrap().len(),
        CLEAR_TICK as usize
    );
    assert!(verify(&stage, &replay.to_bytes()).is_ok());
}

#[test]
fn a_replay_that_does_not_clear_is_recorded_but_does_not_verify() {
    let stage = stage_bytes();
    // Failed: facing down the field, away from the boss, until the stage runs out of ticks on
    // tick 600.
    let away = Input {
        move_y: 1,
        ..Input::default()
    };
    let failed = record(&stage, &vec![away; 700]).unwrap();
    assert_eq!(failed.ticks, 600);
    assert_eq!(
        verify(&stage, &failed.to_bytes()),
        Err(ReplayError::NotCleared { tick: 600 })
    );
    // Still running when the inputs end: a bug report (ADR-016).
    let running = record(&stage, &[idle(); 10]).unwrap();
    assert_eq!(running.ticks, 10);
    assert_eq!(
        verify(&stage, &running.to_bytes()),
        Err(ReplayError::NotCleared { tick: 10 })
    );
    assert_eq!(
        verify(&stage, &record(&stage, &[]).unwrap().to_bytes()),
        Err(ReplayError::NotCleared { tick: 0 })
    );
}

#[test]
fn inputs_after_the_clear_are_refused() {
    let (stage, replay, _) = recorded();
    let longer = Replay {
        ticks: CLEAR_TICK + 1,
        inputs: pack_inputs(&[clearing_inputs(), vec![idle()]].concat()),
        ..replay
    };
    assert_eq!(
        verify(&stage, &longer.to_bytes()),
        Err(ReplayError::InputsAfterClear {
            cleared_at: CLEAR_TICK
        })
    );
}

#[test]
fn a_replay_only_verifies_against_its_own_stage() {
    let (stage, replay, bytes) = recorded();
    // One more byte in the stage file: a different file, even though it parses the same.
    let mut changed = stage.clone();
    changed.push(b'\n');
    assert_eq!(
        verify(&changed, &bytes),
        Err(ReplayError::NotThisStage("the stage file has changed"))
    );
    for (field, value, why) in [
        ("seed", json!(8), "another seed"),
        ("sim_version", json!(1), "another version"),
        ("schema_version", json!(1), "another version"),
    ] {
        let tampered = with(&replay, |v| v[field] = value.clone());
        assert_eq!(
            verify(&stage, &tampered),
            Err(ReplayError::NotThisStage(why)),
            "{field}"
        );
    }
    let tampered = with(&replay, |v| v["final_hash"] = json!(hash_to_hex(0xdead)));
    assert!(matches!(
        verify(&stage, &tampered),
        Err(ReplayError::WrongFinalHash { actual, .. }) if actual != 0xdead
    ));
    for bad in ["", "xyz", "9F3A0C12AA55BB66"] {
        let tampered = with(&replay, |v| v["final_hash"] = json!(bad));
        assert_eq!(
            verify(&stage, &tampered),
            Err(ReplayError::BadHash),
            "{bad:?}"
        );
    }
    // Different inputs reach a different final hash: a step up the field still clears.
    let mut inputs = clearing_inputs();
    inputs[20].move_y = -50;
    let swapped = Replay {
        inputs: pack_inputs(&inputs),
        ..replay
    };
    assert!(matches!(
        verify(&stage, &swapped.to_bytes()),
        Err(ReplayError::WrongFinalHash { .. })
    ));
}

#[test]
fn an_invalid_stage_is_reported_as_such() {
    let (_, _, bytes) = recorded();
    assert!(matches!(
        verify(b"{ not json", &bytes),
        Err(ReplayError::Stage(validate::Error::Parse { .. }))
    ));
    let mut stage: Value = serde_json::from_slice(&stage_bytes()).unwrap();
    stage["player"]["hp"] = json!(0);
    assert!(matches!(
        record(stage.to_string().as_bytes(), &[]),
        Err(ReplayError::Stage(validate::Error::Invalid(_)))
    ));
}

#[test]
fn per_tick_hashes_name_the_first_tick_that_differs() {
    let (stage, _, bytes) = recorded();
    let parsed = validate::parse_and_validate(&stage).unwrap();
    let mut engine = Engine::new(&parsed).unwrap();
    let hashes: Vec<u64> = clearing_inputs()
        .iter()
        .map(|i| {
            engine.step(*i);
            engine.state_hash()
        })
        .collect();
    assert!(verify_against(&stage, &bytes, &hashes).is_ok());
    for bad_index in [0usize, 17, CLEAR_TICK as usize - 1] {
        let mut wrong = hashes.clone();
        wrong[bad_index] ^= 1;
        let err = verify_against(&stage, &bytes, &wrong).unwrap_err();
        assert_eq!(
            err,
            ReplayError::Diverged {
                tick: u32::try_from(bad_index).unwrap() + 1,
                expected: wrong[bad_index],
                actual: hashes[bad_index],
            }
        );
        assert_eq!(
            err.to_string(),
            format!(
                "the run differs from the expected hashes on tick {}: expected {:016x}, got {:016x}",
                bad_index + 1,
                wrong[bad_index],
                hashes[bad_index]
            )
        );
    }
    assert_eq!(
        verify_against(&stage, &bytes, &hashes[..10]),
        Err(ReplayError::HashCount {
            expected: CLEAR_TICK,
            given: 10
        })
    );
}

#[test]
fn the_encoding_is_pinned() {
    // The replay file and its hashes for a fixed stage and inputs. Every platform must write
    // and verify exactly this; a change is a change to the format or to the version 0 rules,
    // so a PR that changes it regenerates these values and says why (ADR-035).
    let (stage, replay, bytes) = recorded();
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        concat!(
            r#"{"schema_version":0,"sim_version":0,"seed":7,"#,
            r#""stage_sha256":"45111ffeb51b8951352c5494a5bcfb4295d9cd74ad559ec909e9062a7a0872de","#,
            r#""ticks":35,"final_hash":"f41f6599e39fe57c","#,
            r#""inputs":"AQEAAAIAAAACAgAAGQAAAAICAAADAAAA"}"#,
            "\n"
        )
    );
    // The packed inputs, decoded by hand: one tick of evade (01 01 00 00), two idle
    // (02 00 00 00), two of `main` (02 02 00 00), 25 idle (19 00 00 00), two of `main`, then
    // three idle (03 00 00 00).
    assert_eq!(
        base64::decode(&replay.inputs).unwrap(),
        [
            1, 1, 0, 0, 2, 0, 0, 0, 2, 2, 0, 0, 25, 0, 0, 0, 2, 2, 0, 0, 3, 0, 0, 0
        ]
    );
    assert_eq!(
        to_hex(&content_hash(&stage, &bytes)),
        "a17bc0f364c51628c4ca8171e2232c8ce1f61474aeac43d4ddf7ca1373def12d"
    );
}

// --- Nothing breaks it -----------------------------------------------------------------------

#[test]
fn damaged_files_are_refused_without_panicking() {
    let (stage, _, bytes) = recorded();
    let mut rng = SplitMix64::new(0xBAD);
    for round in 0..3000 {
        let (mut s, mut r) = (stage.clone(), bytes.clone());
        let target = if round % 2 == 0 { &mut r } else { &mut s };
        for _ in 0..=rng.below(3) {
            let at = usize::try_from(rng.below(u32::try_from(target.len()).unwrap())).unwrap();
            match rng.below(4) {
                0 => target[at] = u8::try_from(rng.below(256)).unwrap(),
                1 => target.truncate(at.max(1)),
                2 => {
                    let end = (at + rng.below(32) as usize).min(target.len());
                    target.drain(at..end);
                }
                _ => target.insert(at, u8::try_from(rng.below(256)).unwrap()),
            }
            if target.len() < 2 {
                break;
            }
        }
        let _ = verify(&s, &r);
    }
}

fn clearing_hashes() -> Vec<u64> {
    let parsed = validate::parse_and_validate(&stage_bytes()).unwrap();
    let mut engine = Engine::new(&parsed).unwrap();
    clearing_inputs()
        .iter()
        .map(|i| {
            engine.step(*i);
            engine.state_hash()
        })
        .collect()
}

#[test]
fn the_hashes_file_is_eight_little_endian_bytes_a_tick() {
    let bytes = hashes_to_bytes(&[1, 0x0123_4567_89ab_cdef]);
    assert_eq!(
        bytes,
        [
            1, 0, 0, 0, 0, 0, 0, 0, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01
        ]
    );
    assert_eq!(
        hashes_from_bytes(&bytes),
        Ok(vec![1, 0x0123_4567_89ab_cdef])
    );
    let h = clearing_hashes();
    assert_eq!(h.len(), 35);
    let bytes = hashes_to_bytes(&h);
    assert_eq!(bytes.len(), 280);
    assert_eq!(hashes_from_bytes(&bytes), Ok(h));
    assert_eq!(hashes_from_bytes(&[]), Ok(vec![]));
}

#[test]
fn a_hashes_file_that_is_not_whole_hashes_fails_closed() {
    for size in [1usize, 7, 9, 15, 279] {
        assert_eq!(
            hashes_from_bytes(&vec![0; size]),
            Err(HashesError::Length { size })
        );
    }
    assert_eq!(
        HashesError::Length { size: 279 }.to_string(),
        "the hashes file is 279 bytes, not a whole number of 8-byte hashes"
    );
}

#[test]
fn the_hashes_file_size_limit_is_checked_first() {
    assert_eq!(MAX_HASHES_BYTES, 1024 * 1024);
    assert_eq!(
        hashes_from_bytes(&vec![0; MAX_HASHES_BYTES]).unwrap().len(),
        131_072
    );
    let err = hashes_from_bytes(&vec![0; MAX_HASHES_BYTES + 8]).unwrap_err();
    assert_eq!(
        err,
        HashesError::TooLarge {
            size: 1_048_584,
            limit: 1_048_576
        }
    );
    assert_eq!(
        err.to_string(),
        "the hashes file is 1048584 bytes; the limit is 1048576"
    );
}

proptest::proptest! {
    #[test]
    fn any_bytes_decode_exactly_when_whole_and_re_encode(
        bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096)
    ) {
        match hashes_from_bytes(&bytes) {
            Ok(h) => {
                proptest::prop_assert_eq!(bytes.len() % 8, 0);
                proptest::prop_assert_eq!(hashes_to_bytes(&h), bytes);
            }
            Err(_) => proptest::prop_assert_ne!(bytes.len() % 8, 0),
        }
    }
}

// --- The version header ----------------------------------------------------------------------

fn newer(schema_version: u32, sim_version: u32) -> Result<Replay, ReplayError> {
    Err(ReplayError::Newer {
        schema_version,
        sim_version,
    })
}

#[test]
fn a_replay_for_a_newer_version_is_newer() {
    let (_, replay, _) = recorded();
    let schema = with(&replay, |v| v["schema_version"] = json!(1));
    assert_eq!(Replay::from_bytes(&schema), newer(1, 0));
    // A newer file may hold fields this version rejects; it is still "newer", not damaged.
    let future = with(&replay, |v| {
        v["sim_version"] = json!(1);
        v["future"] = json!(1);
    });
    assert_eq!(Replay::from_bytes(&future), newer(0, 1));
    let huge = with(&replay, |v| v["schema_version"] = json!(4_000_000_000u32));
    assert_eq!(Replay::from_bytes(&huge), newer(4_000_000_000, 0));
}

#[test]
fn the_size_limit_stays_before_the_version() {
    let (_, replay, _) = recorded();
    let mut big = with(&replay, |v| v["schema_version"] = json!(1));
    big.resize(MAX_STAGE_REPLAY_BYTES + 1, b' ');
    assert_eq!(
        Replay::from_bytes(&big),
        Err(ReplayError::TooLarge {
            size: MAX_STAGE_REPLAY_BYTES + 1,
            limit: MAX_STAGE_REPLAY_BYTES
        })
    );
    assert!(matches!(
        Replay::from_bytes(&vec![b'x'; MAX_STAGE_REPLAY_BYTES + 1]),
        Err(ReplayError::TooLarge { .. })
    ));
}

#[test]
fn a_replay_that_cannot_be_parsed_at_a_known_version_is_damaged() {
    let (_, replay, bytes) = recorded();
    let damaged = |b: &[u8]| matches!(Replay::from_bytes(b), Err(ReplayError::Damaged { .. }));
    assert!(damaged(&with(&replay, |v| {
        v.as_object_mut().unwrap().remove("inputs");
    })));
    assert!(damaged(&with(&replay, |v| v["unexpected"] = json!(1))));
    assert!(damaged(&with(&replay, |v| v["ticks"] = json!("many"))));
    assert!(damaged(&bytes[..bytes.len() / 2]));
    for no_versions in [&b""[..], b"[]", b"{}"] {
        assert!(damaged(no_versions));
    }
    assert!(damaged(&with(&replay, |v| {
        v.as_object_mut().unwrap().remove("schema_version");
    })));
    assert!(damaged(&with(&replay, |v| v["sim_version"] = json!("0"))));
}
