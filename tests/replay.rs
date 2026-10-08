//! Replays (ADR-009, ADR-016): the packed input encoding, recording, verification and the
//! checks on untrusted replay files (ADR-020).

mod common;

use common::{CLEAR_TICK, clearing_inputs, firing, stage_bytes};
use serde_json::{Value, json};
use stella_rain_core::base64;
use stella_rain_core::content_hash::{content_hash, sha256, to_hex};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::replay::{
    MAX_REPLAY_BYTES, Replay, ReplayError, hash_from_hex, hash_to_hex, pack_inputs, record, verify,
    verify_against,
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
        inputs: pack_inputs(inputs).unwrap(),
    }
}

#[test]
fn packed_inputs_round_trip_including_the_extremes() {
    let mut rng = SplitMix64::new(19);
    let mut inputs = vec![
        Input {
            dx: i16::MIN,
            dy: i16::MAX,
            focus: true,
            fire: true,
            held: 3,
        },
        Input {
            dx: i16::MAX,
            dy: i16::MIN,
            focus: false,
            fire: false,
            held: 0,
        },
        Input {
            dx: 0,
            dy: 0,
            focus: false,
            fire: false,
            held: 0,
        },
        Input {
            dx: -1,
            dy: 1,
            focus: true,
            fire: false,
            held: 1,
        },
    ];
    for _ in 0..500 {
        let held = rng.below(4) as u8;
        inputs.push(Input {
            dx: (rng.next_u64() & 0xffff) as u16 as i16,
            dy: (rng.next_u64() & 0xffff) as u16 as i16,
            focus: rng.below(2) == 0,
            fire: rng.below(2) == 0,
            held,
        });
    }
    // Long runs of one input, and of nothing.
    inputs.extend(std::iter::repeat_n(Input::default(), 300));
    inputs.extend(std::iter::repeat_n(
        Input {
            fire: true,
            ..Input::default()
        },
        17,
    ));
    let replay = replay_of(&inputs, inputs.len() as u32);
    assert_eq!(replay.unpack_inputs(u32::MAX).unwrap(), inputs);
}

#[test]
fn steady_input_packs_into_a_few_bytes() {
    let held_key = vec![
        Input {
            dx: 768,
            fire: true,
            ..Input::default()
        };
        18_000
    ];
    assert!(pack_inputs(&held_key).unwrap().len() <= 16);
    assert_eq!(pack_inputs(&[]).unwrap(), "");
}

#[test]
fn a_replay_of_never_repeating_input_fits_the_size_limit() {
    // The worst case: 18,000 ticks (five minutes), every input different and large.
    let inputs: Vec<Input> = (0..18_000u32)
        .map(|t| Input {
            dx: if t % 2 == 0 { i16::MIN } else { i16::MAX },
            dy: t as i16,
            focus: t % 3 == 0,
            fire: t % 5 == 0,
            held: (t % 4) as u8,
        })
        .collect();
    let mut replay = replay_of(&inputs, 18_000);
    replay.stage_sha256 = "0".repeat(64);
    assert!(
        replay.to_bytes().len() < MAX_REPLAY_BYTES,
        "{} bytes",
        replay.to_bytes().len()
    );
}

#[test]
fn a_held_slot_above_three_cannot_be_packed() {
    let input = Input {
        held: 4,
        ..Input::default()
    };
    assert!(matches!(
        pack_inputs(&[input]),
        Err(ReplayError::BadInputs(_))
    ));
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
    // One run of two ticks, flags `fire`, dx and dy zero: [2, 0b10, 0, 0].
    assert_eq!(unpack_raw(&[2, 2, 0, 0], 2).unwrap().len(), 2);
    let reason = |bytes: &[u8], ticks: u32| match unpack_raw(bytes, ticks) {
        Err(ReplayError::BadInputs(why)) => why,
        other => panic!("{bytes:?}: {other:?}"),
    };
    assert!(reason(&[0, 0, 0, 0], 1).contains("length 0"));
    assert!(reason(&[3, 2, 0, 0], 2).contains("more inputs than ticks"));
    assert!(reason(&[1, 2, 0, 0], 2).contains("fewer inputs"));
    assert!(reason(&[2, 0xf0, 0, 0], 2).contains("unknown flags"));
    assert!(reason(&[2, 2, 0], 2).contains("middle"));
    assert!(reason(&[2], 2).contains("middle"));
    assert!(
        reason(&[2, 2, 0, 0, 1], 3).contains("middle"),
        "trailing partial run"
    );
    // A movement of zigzag 65536 is 32768, which is not an i16.
    assert!(reason(&[1, 0, 0x80, 0x80, 0x04, 0], 1).contains("out of range"));
    // Numbers not in their shortest form, too long, or too large.
    assert!(reason(&[0x81, 0x00, 2, 0, 0], 1).contains("shortest"));
    assert!(reason(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01, 2, 0, 0], 1).contains("too"));
    assert!(reason(&[0xff, 0xff, 0xff, 0xff, 0x1f, 2, 0, 0], 1).contains("too large"));
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
    let huge_run = [0xff, 0xff, 0xff, 0xff, 0x0f, 2, 0, 0];
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
        Err(ReplayError::Parse { .. })
    ));
    let missing = with(&replay, |v| {
        v.as_object_mut().unwrap().remove("inputs");
    });
    assert!(matches!(
        Replay::from_bytes(&missing),
        Err(ReplayError::Parse { .. })
    ));
    assert!(matches!(
        Replay::from_bytes(b""),
        Err(ReplayError::Parse { .. })
    ));
    assert!(matches!(
        Replay::from_bytes(b"[]"),
        Err(ReplayError::Parse { .. })
    ));
}

#[test]
fn the_size_limit_applies_before_parsing() {
    let (_, _, mut bytes) = recorded();
    bytes.resize(MAX_REPLAY_BYTES, b' ');
    assert!(Replay::from_bytes(&bytes).is_ok());
    bytes.push(b' ');
    assert_eq!(
        Replay::from_bytes(&bytes),
        Err(ReplayError::TooLarge {
            size: MAX_REPLAY_BYTES + 1,
            limit: MAX_REPLAY_BYTES
        })
    );
    assert!(matches!(
        Replay::from_bytes(&vec![b'x'; MAX_REPLAY_BYTES + 1]),
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
    let replay = record(&stage, &vec![firing(); 400]).unwrap();
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
    // Failed: idle until the stage runs out of ticks on tick 600.
    let failed = record(&stage, &vec![Input::default(); 700]).unwrap();
    assert_eq!(failed.ticks, 600);
    assert_eq!(
        verify(&stage, &failed.to_bytes()),
        Err(ReplayError::NotCleared { tick: 600 })
    );
    // Still running when the inputs end: a bug report (ADR-016).
    let running = record(&stage, &[firing(); 10]).unwrap();
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
        inputs: pack_inputs(&[clearing_inputs(), vec![firing()]].concat()).unwrap(),
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
    // Different inputs reach a different final hash.
    let mut inputs = clearing_inputs();
    inputs[3].dx = 50;
    let swapped = Replay {
        inputs: pack_inputs(&inputs).unwrap(),
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
    for bad_index in [0usize, 30, 61] {
        let mut wrong = hashes.clone();
        wrong[bad_index] ^= 1;
        assert_eq!(
            verify_against(&stage, &bytes, &wrong),
            Err(ReplayError::Diverged {
                tick: bad_index as u32 + 1
            })
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
            r#""stage_sha256":"b66fe06722776b68bb952d2afe01491c456fa7cc6dc17934afd1f793f9fe619d","#,
            r#""ticks":62,"final_hash":"5c062e5141468341","#,
            r#""inputs":"AQpQAAkCUAAKAgAAAQoAABMCAAABCgAAEwIAAAEKAAABAgAA"}"#,
            "\n"
        )
    );
    // The packed inputs, decoded by hand: one tick of fire with slot 2 held and dx 40
    // (01 0a 50 00), then nine ticks of fire and dx 40 (09 02 50 00), and so on.
    assert!(
        base64::decode(&replay.inputs)
            .unwrap()
            .starts_with(&[1, 0x0a, 0x50, 0, 9, 2, 0x50, 0])
    );
    assert_eq!(
        to_hex(&content_hash(&stage, &bytes)),
        "dd57d4abbce72fafd8ffef5cc01e7db9782a26f033258e6b301150695375c515"
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
            let at = rng.below(target.len() as u32) as usize;
            match rng.below(4) {
                0 => target[at] = rng.below(256) as u8,
                1 => target.truncate(at.max(1)),
                2 => {
                    let end = (at + rng.below(32) as usize).min(target.len());
                    target.drain(at..end);
                }
                _ => target.insert(at, rng.below(256) as u8),
            }
            if target.len() < 2 {
                break;
            }
        }
        let _ = verify(&s, &r);
    }
}
