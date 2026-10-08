//! Replays (ADR-009, ADR-016): the recorded inputs of a run, which anyone can re-simulate to
//! check that the stage can be cleared and reaches the same final hash.
//!
//! A `.replay` file is one JSON object: the versions, the seed, the SHA-256 of the exact stage
//! file, the number of ticks, the final state hash, and the per-tick inputs packed and
//! base64-encoded. Replays are untrusted (ADR-020): a size limit, strict parsing, and no more
//! inputs than the stage has ticks. The checks are in `Replay::from_bytes`, `unpack_inputs`
//! and `verify`.
//!
//! Packed inputs are runs of identical inputs. Each run is a LEB128 run length (at least 1),
//! one flags byte (bit 0 `focus`, bit 1 `fire`, bits 2 and 3 the held skill slot, the rest
//! zero), then `dx` and `dy` as zigzag LEB128 numbers. Five minutes of changing inputs take
//! about 100 KB, inside `MAX_REPLAY_BYTES`; steady input takes a few bytes.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::base64;
use crate::content_hash::{self, to_hex};
use crate::engine::{Engine, Outcome};
use crate::input::Input;
use crate::stage::Stage;
use crate::validate;

/// A replay file is at most this big (ADR-020). Even a maximum-length replay whose inputs never
/// repeat fits.
pub const MAX_REPLAY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    /// Must equal the stage's (ADR-020).
    pub schema_version: u32,
    /// Must equal the stage's (ADR-020).
    pub sim_version: u32,
    /// Must equal the stage's.
    pub seed: u32,
    /// SHA-256 of the exact stage file bytes, lowercase hex: a changed stage invalidates the
    /// replay.
    pub stage_sha256: String,
    /// Ticks recorded, at most the stage's length. A replay that proves a clear ends on the
    /// tick of the clear.
    pub ticks: u32,
    /// The FNV-1a 64 state hash after the last tick (ADR-019), 16 lowercase hex digits. Hex,
    /// because JSON readers that use doubles cannot hold every 64-bit integer.
    pub final_hash: String,
    /// The per-tick inputs, packed as described in the module documentation, then base64.
    pub inputs: String,
}

/// Why a replay was not accepted. Messages never repeat text from the files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    TooLarge {
        size: usize,
        limit: usize,
    },
    /// Not JSON, or not the replay format.
    Parse {
        line: usize,
        column: usize,
    },
    /// The packed inputs are damaged; the reason is a fixed phrase.
    BadInputs(&'static str),
    /// The stage file is not valid.
    Stage(validate::Error),
    /// The replay is for another version, seed or stage file.
    NotThisStage(&'static str),
    /// More ticks than the stage has.
    TooManyTicks {
        ticks: u32,
        limit: u32,
    },
    /// The `final_hash` field is not 16 lowercase hex digits.
    BadHash,
    /// The run ended without a clear, on this tick (0 if no input was given).
    NotCleared {
        tick: u32,
    },
    /// The stage was cleared before the last input.
    InputsAfterClear {
        cleared_at: u32,
    },
    /// Re-simulation reached a different final hash.
    WrongFinalHash {
        expected: u64,
        actual: u64,
    },
    /// Re-simulation differs from the expected per-tick hashes, first on this tick.
    Diverged {
        tick: u32,
    },
    /// The expected per-tick hashes are not one per tick.
    HashCount {
        expected: u32,
        given: usize,
    },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::TooLarge { size, limit } => {
                write!(f, "the replay is {size} bytes; the limit is {limit}")
            }
            ReplayError::Parse { line, column } => {
                write!(f, "not a valid replay (line {line}, column {column})")
            }
            ReplayError::BadInputs(why) => write!(f, "the replay's inputs are damaged: {why}"),
            ReplayError::Stage(e) => write!(f, "the stage is not valid:\n{e}"),
            ReplayError::NotThisStage(why) => write!(f, "the replay is not for this stage: {why}"),
            ReplayError::TooManyTicks { ticks, limit } => {
                write!(f, "the replay has {ticks} ticks; the stage allows {limit}")
            }
            ReplayError::BadHash => f.write_str("the replay's final hash is not 16 hex digits"),
            ReplayError::NotCleared { tick } => {
                write!(
                    f,
                    "the replay does not clear the stage (it ends on tick {tick})"
                )
            }
            ReplayError::InputsAfterClear { cleared_at } => {
                write!(
                    f,
                    "the stage is cleared on tick {cleared_at}, before the replay ends"
                )
            }
            ReplayError::WrongFinalHash { expected, actual } => write!(
                f,
                "the final hash is {actual:016x}, not the replay's {expected:016x}"
            ),
            ReplayError::Diverged { tick } => {
                write!(f, "the run differs from the expected hashes on tick {tick}")
            }
            ReplayError::HashCount { expected, given } => {
                write!(f, "expected {expected} per-tick hashes, got {given}")
            }
        }
    }
}

impl std::error::Error for ReplayError {}

/// What a successful verification proves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    pub ticks: u32,
    pub final_hash: u64,
    /// The stage content hash of ADR-034, for the catalog and the blocklist.
    pub content_hash: [u8; 32],
}

/// A state hash as written in files: 16 lowercase hex digits.
pub fn hash_to_hex(hash: u64) -> String {
    format!("{hash:016x}")
}

/// The inverse of `hash_to_hex`; nothing else is accepted.
pub fn hash_from_hex(text: &str) -> Option<u64> {
    let ok = text.len() == 16 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if ok {
        u64::from_str_radix(text, 16).ok()
    } else {
        None
    }
}

impl Replay {
    /// The file's bytes: compact JSON and a newline. These exact bytes are what the content
    /// hash covers (ADR-034).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(self).expect("a replay serialises");
        bytes.push(b'\n');
        bytes
    }

    /// Parses a replay file: size limit first, then strict parsing.
    pub fn from_bytes(bytes: &[u8]) -> Result<Replay, ReplayError> {
        if bytes.len() > MAX_REPLAY_BYTES {
            return Err(ReplayError::TooLarge {
                size: bytes.len(),
                limit: MAX_REPLAY_BYTES,
            });
        }
        serde_json::from_slice(bytes).map_err(|e| ReplayError::Parse {
            line: e.line(),
            column: e.column(),
        })
    }

    /// The inputs, one per tick. There are exactly `ticks` of them and `ticks` is at most
    /// `max_ticks`, so a hostile count cannot make this allocate more than the stage allows.
    pub fn unpack_inputs(&self, max_ticks: u32) -> Result<Vec<Input>, ReplayError> {
        if self.ticks > max_ticks {
            return Err(ReplayError::TooManyTicks {
                ticks: self.ticks,
                limit: max_ticks,
            });
        }
        let bytes =
            base64::decode(&self.inputs).map_err(|_| ReplayError::BadInputs("not valid base64"))?;
        unpack(&bytes, self.ticks).map_err(ReplayError::BadInputs)
    }
}

fn write_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u32, &'static str> {
    let mut value = 0u32;
    for i in 0..5u32 {
        let b = *bytes.get(*pos).ok_or("ends in the middle of a number")?;
        *pos += 1;
        let low = u32::from(b & 0x7f);
        // The fifth byte may only carry the top four bits of a u32.
        if i == 4 && low > 0x0f {
            return Err("a number is too large");
        }
        value |= low << (7 * i);
        if b & 0x80 == 0 {
            if i > 0 && low == 0 {
                return Err("a number is not in its shortest form");
            }
            return Ok(value);
        }
    }
    Err("a number is too long")
}

fn zigzag(v: i16) -> u32 {
    let v = i32::from(v);
    ((v << 1) ^ (v >> 31)) as u32
}

fn unzigzag(z: u32) -> Option<i16> {
    let v = ((z >> 1) as i32) ^ -((z & 1) as i32);
    i16::try_from(v).ok()
}

/// Packs `inputs` into runs, then base64. Fails if a `held` slot is above 3.
pub fn pack_inputs(inputs: &[Input]) -> Result<String, ReplayError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < inputs.len() {
        let input = inputs[i];
        if input.held > 3 {
            return Err(ReplayError::BadInputs("a held skill slot is above 3"));
        }
        let run = inputs[i..].iter().take_while(|x| **x == input).count();
        write_varint(&mut out, u32::try_from(run).unwrap_or(u32::MAX));
        out.push(u8::from(input.focus) | (u8::from(input.fire) << 1) | (input.held << 2));
        write_varint(&mut out, zigzag(input.dx));
        write_varint(&mut out, zigzag(input.dy));
        i += run;
    }
    Ok(base64::encode(&out))
}

/// Unpacks exactly `ticks` inputs from the runs in `bytes`.
fn unpack(bytes: &[u8], ticks: u32) -> Result<Vec<Input>, &'static str> {
    let mut inputs = Vec::with_capacity(ticks as usize);
    let mut pos = 0;
    while pos < bytes.len() {
        let run = read_varint(bytes, &mut pos)?;
        if run == 0 {
            return Err("a run has length 0");
        }
        let room = ticks - u32::try_from(inputs.len()).unwrap_or(u32::MAX);
        if run > room {
            return Err("there are more inputs than ticks");
        }
        let flags = *bytes.get(pos).ok_or("ends in the middle of a run")?;
        pos += 1;
        if flags & 0xf0 != 0 {
            return Err("a run has unknown flags");
        }
        let dx = unzigzag(read_varint(bytes, &mut pos)?).ok_or("a movement is out of range")?;
        let dy = unzigzag(read_varint(bytes, &mut pos)?).ok_or("a movement is out of range")?;
        let input = Input {
            dx,
            dy,
            focus: flags & 1 != 0,
            fire: flags & 2 != 0,
            held: (flags >> 2) & 3,
        };
        inputs.extend(std::iter::repeat_n(input, run as usize));
    }
    if inputs.len() != ticks as usize {
        return Err("there are fewer inputs than ticks");
    }
    Ok(inputs)
}

/// Runs `inputs` through the stage and writes the replay. Recording stops on the tick the run
/// ends, so inputs after a clear or a fail are left out; a run that is still going when the
/// inputs end is recorded as it stands (a bug report, ADR-016). `stage_bytes` are the exact
/// bytes of the stage file.
pub fn record(stage_bytes: &[u8], inputs: &[Input]) -> Result<Replay, ReplayError> {
    let stage = validate::parse_and_validate(stage_bytes).map_err(ReplayError::Stage)?;
    let mut engine = Engine::new(&stage).map_err(ReplayError::Stage)?;
    let mut used = 0;
    for input in inputs {
        if engine.outcome() != Outcome::Running {
            break;
        }
        engine.step(*input);
        used += 1;
    }
    Ok(Replay {
        schema_version: stage.schema_version,
        sim_version: stage.sim_version,
        seed: stage.seed,
        stage_sha256: to_hex(&content_hash::sha256(stage_bytes)),
        ticks: engine.tick(),
        final_hash: hash_to_hex(engine.state_hash()),
        inputs: pack_inputs(&inputs[..used])?,
    })
}

/// Checks that the replay is for this stage and clears it with the replay's final hash
/// (ADR-009, ADR-015). The whole of both files is untrusted.
pub fn verify(stage_bytes: &[u8], replay_bytes: &[u8]) -> Result<Verified, ReplayError> {
    verify_inner(stage_bytes, replay_bytes, None)
}

/// As `verify`, and also compares the hash after every tick with `expected` (one per tick),
/// naming the first tick that differs: what a corpus check reports (ADR-019).
pub fn verify_against(
    stage_bytes: &[u8],
    replay_bytes: &[u8],
    expected: &[u64],
) -> Result<Verified, ReplayError> {
    verify_inner(stage_bytes, replay_bytes, Some(expected))
}

fn verify_inner(
    stage_bytes: &[u8],
    replay_bytes: &[u8],
    expected: Option<&[u64]>,
) -> Result<Verified, ReplayError> {
    let replay = Replay::from_bytes(replay_bytes)?;
    let stage: Stage = validate::parse_and_validate(stage_bytes).map_err(ReplayError::Stage)?;
    if replay.schema_version != stage.schema_version || replay.sim_version != stage.sim_version {
        return Err(ReplayError::NotThisStage("another version"));
    }
    if replay.seed != stage.seed {
        return Err(ReplayError::NotThisStage("another seed"));
    }
    if replay.stage_sha256 != to_hex(&content_hash::sha256(stage_bytes)) {
        return Err(ReplayError::NotThisStage("the stage file has changed"));
    }
    let final_hash = hash_from_hex(&replay.final_hash).ok_or(ReplayError::BadHash)?;
    let inputs = replay.unpack_inputs(stage.length_ticks)?;
    if let Some(expected) = expected {
        if expected.len() != inputs.len() {
            return Err(ReplayError::HashCount {
                expected: replay.ticks,
                given: expected.len(),
            });
        }
    }

    let mut engine = Engine::new(&stage).map_err(ReplayError::Stage)?;
    for (i, input) in inputs.iter().enumerate() {
        match engine.outcome() {
            Outcome::Running => {}
            Outcome::Failed => {
                return Err(ReplayError::NotCleared {
                    tick: engine.tick(),
                });
            }
            Outcome::Cleared => {
                return Err(ReplayError::InputsAfterClear {
                    cleared_at: engine.tick(),
                });
            }
        }
        engine.step(*input);
        if let Some(expected) = expected {
            if expected[i] != engine.state_hash() {
                return Err(ReplayError::Diverged {
                    tick: engine.tick(),
                });
            }
        }
    }
    if engine.outcome() != Outcome::Cleared {
        return Err(ReplayError::NotCleared {
            tick: engine.tick(),
        });
    }
    let actual = engine.state_hash();
    if actual != final_hash {
        return Err(ReplayError::WrongFinalHash {
            expected: final_hash,
            actual,
        });
    }
    Ok(Verified {
        ticks: engine.tick(),
        final_hash: actual,
        content_hash: content_hash::content_hash(stage_bytes, replay_bytes),
    })
}
