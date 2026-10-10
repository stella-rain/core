//! The bodies of the fuzz targets (ADR-020), as plain functions: the targets in
//! `fuzz_targets/` only call them, and `tests/smoke.rs` runs them on stable.
//!
//! Each function takes any bytes, must not panic, and returns whether the bytes got past the
//! first gate (a stage that validates, a replay that verifies, a share code that decodes), so
//! the tests can tell a target that reaches the code behind the gate from one that does not.
//! A panic is a finding: the assertions below say what must hold, not what happens to.

use miniz_oxide::deflate::compress_to_vec_zlib;
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::replay::{self, Replay};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::limits::MAX_TICKS;
use stella_rain_core::validate::{self, parse_and_validate};
use stella_rain_core::{base64, share};

/// Ticks `stage_run` simulates: long enough for waves, attacks and boss phases to start, short
/// enough that the fuzzer keeps its speed.
const RUN_TICKS: usize = 120;

/// A stage the replay target verifies against (the corpus case that is cleared).
const CLEAR_STAGE: &[u8] = include_bytes!("../../tests/corpus/v0/clear/stage.json");

/// The stage parser and validator (ADR-020 items 1 to 4). A stage that loads can be saved
/// again, which is what the editor does, and loads again as the same stage.
pub fn stage_parse(data: &[u8]) -> bool {
    let Ok(stage) = parse_and_validate(data) else {
        return false;
    };
    let saved = serde_json::to_vec(&stage).expect("a stage serialises");
    let again = parse_and_validate(&saved).expect("a stage that loaded loads again once saved");
    assert_eq!(again, stage, "a saved stage loads as another stage");
    true
}

/// The validator, then the simulation: a stage that validates must start a run and survive
/// `RUN_TICKS` ticks of any input without a panic (ADR-019, ADR-020). The bytes are the stage
/// file and, read over and over, the inputs.
pub fn stage_run(data: &[u8]) -> bool {
    // Deserialised without the validator's own parse, so the validator sees what the strict
    // types accept, not only what the size and version checks let through.
    let Ok(stage) = serde_json::from_slice::<Stage>(data) else {
        return false;
    };
    if validate::validate(&stage).is_err() {
        return false;
    }
    let mut engine = Engine::new(&stage).expect("a stage that validates starts a run");
    let ticks = usize::try_from(stage.length_ticks).map_or(RUN_TICKS, |n| n.min(RUN_TICKS));
    for input in inputs_from(data, ticks) {
        engine.step(input);
        let _ = engine.snapshot();
    }
    engine.tick() > 0
}

/// Inputs read out of `data`, five bytes a tick and round again at its end. Skill slots and
/// speeds go beyond what a player can send, because a replay is untrusted.
fn inputs_from(data: &[u8], ticks: usize) -> Vec<Input> {
    let byte = |t: usize, i: usize| {
        data.get((t * 5 + i) % data.len().max(1))
            .copied()
            .unwrap_or(0)
    };
    (0..ticks)
        .map(|t| Input {
            dx: i16::from_le_bytes([byte(t, 0), byte(t, 1)]),
            dy: i16::from_le_bytes([byte(t, 2), byte(t, 3)]),
            focus: byte(t, 4) & 1 != 0,
            fire: byte(t, 4) & 2 != 0,
            held: byte(t, 4) >> 2,
        })
        .collect()
}

/// The replay loader (ADR-020 item 5): a replay that loads unpacks without a panic and saves
/// to bytes that load as the same replay; and any bytes verified against a stage that is
/// cleared never panic, whatever they claim.
pub fn replay_load(data: &[u8]) -> bool {
    if let Ok(replay) = Replay::from_bytes(data) {
        let _ = replay.unpack_inputs(MAX_TICKS);
        let again = Replay::from_bytes(&replay.to_bytes())
            .expect("a replay that loaded loads again once saved");
        assert_eq!(again, replay, "a saved replay loads as another replay");
    }
    replay::verify(CLEAR_STAGE, data).is_ok()
}

/// The share-code decoder (ADR-020 item 7, ADR-009). The bytes are tried as a code, and wrapped
/// as the zlib stream of a code so that they reach the code behind the decompression. A code
/// that decodes holds files that encode to a code that decodes to them; base64 round-trips
/// any bytes.
pub fn share_decode(data: &[u8]) -> bool {
    let bytes_back = base64::decode(&base64::encode(data)).expect("base64 decodes what it encoded");
    assert_eq!(bytes_back, data, "base64 changed the bytes");

    let mut decoded = false;
    if let Ok(code) = std::str::from_utf8(data) {
        decoded |= check_code(code);
    }
    decoded | check_code(&base64::encode(&compress_to_vec_zlib(data, 1)))
}

fn check_code(code: &str) -> bool {
    let Ok((stage, replay)) = share::decode(code) else {
        return false;
    };
    // The code may be longer than `encode` allows when another compressor made it.
    if let Ok(again) = share::encode(&stage, &replay) {
        let back = share::decode(&again).expect("a code that was encoded decodes");
        assert_eq!(back, (stage, replay), "a share code changed its files");
    }
    true
}
