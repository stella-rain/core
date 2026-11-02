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
//! one flags byte (bit 0 `evade`, then `main`, `smash`, `skill`, `assist` and `auto`; bits 6 and 7
//! zero), then `move_x` and `move_y` as one two's-complement byte each, −127 to 127 (ADR-071). Five minutes of changing inputs take about
//! 96 KB, inside `MAX_STAGE_REPLAY_BYTES`; steady input takes a few bytes.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::base64;
use crate::content_hash::{self, to_hex};
use crate::engine::{Engine, Outcome};
use crate::input::Input;
use crate::parsed_stage_hash;
use crate::stage::Stage;
use crate::validate;
use crate::version::{
    self, FIRST_SCHEMA_VERSION, FIRST_SIM_VERSION, ReplayLoadError, SCHEMA_VERSION, Versions,
};

/// A stage's clear replay file is at most this big (ADR-020). Even a maximum-length replay
/// whose inputs never repeat fits. It is also what a share code may carry (`share`). A run's
/// replay has its own, larger cap: `run_replay::MAX_RUN_REPLAY_BYTES`.
pub const MAX_STAGE_REPLAY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    /// The `schema_version` the file was written at. This game reads a file at its own
    /// `SCHEMA_VERSION`; an older one is read only through a migration to it, else it is
    /// `Older`; a newer one is `Newer` (ADR-REPL-7).
    pub schema_version: u32,
    /// The `sim_version` the file was written at. This game reads a file at any version
    /// from `FIRST_SIM_VERSION` to its own `SIM_VERSION`; an older one is read only through a
    /// migration, else it is `Older`; a newer one is `Newer` (ADR-REPL-7).
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
    /// Recorded by an older version of the game that cannot read it.
    Older {
        schema_version: u32,
        sim_version: u32,
    },
    /// Written for a newer version of the game (either version is above this one's).
    Newer {
        schema_version: u32,
        sim_version: u32,
    },
    /// Not JSON, no versions, or not the replay format at a version this game reads.
    Damaged {
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
        expected: u64,
        actual: u64,
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
            ReplayError::Older {
                schema_version,
                sim_version,
            } => write!(
                f,
                "the replay was recorded by an older version (schema {schema_version}, sim {sim_version}); this version cannot read it"
            ),
            ReplayError::Newer {
                schema_version,
                sim_version,
            } => write!(
                f,
                "the replay needs a newer version (schema {schema_version}, sim {sim_version})"
            ),
            ReplayError::Damaged { line, column } => {
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
            ReplayError::Diverged {
                tick,
                expected,
                actual,
            } => write!(
                f,
                "the run differs from the expected hashes on tick {tick}: expected {expected:016x}, got {actual:016x}"
            ),
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
    /// The parsed-stage hash of ADR-046, which the blocklist blocks a stage by.
    pub parsed_stage_hash: [u8; 32],
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

/// A per-tick hashes file is at most this big: 131,072 ticks, over 7 times the longest stage.
pub const MAX_HASHES_BYTES: usize = 1024 * 1024;

/// A per-tick hashes file cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashesError {
    /// The file is over `MAX_HASHES_BYTES`.
    TooLarge { size: usize, limit: usize },
    /// The file is not a whole number of 8-byte hashes.
    Length { size: usize },
}

impl std::fmt::Display for HashesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashesError::TooLarge { size, limit } => {
                write!(f, "the hashes file is {size} bytes; the limit is {limit}")
            }
            HashesError::Length { size } => write!(
                f,
                "the hashes file is {size} bytes, not a whole number of 8-byte hashes"
            ),
        }
    }
}

impl std::error::Error for HashesError {}

/// The per-tick hashes file: the state hash after every tick, each as 8 bytes little-endian
/// (ADR-SIM-3), no header. The corpus's `hashes.bin` and `stage-verify --hashes` (CH1).
pub fn hashes_to_bytes(hashes: &[u64]) -> Vec<u8> {
    hashes.iter().flat_map(|h| h.to_le_bytes()).collect()
}

/// The inverse of `hashes_to_bytes`. The size is checked first, then the length, and nothing is
/// allocated before both pass; the file is untrusted (ADR-SVC-1).
pub fn hashes_from_bytes(bytes: &[u8]) -> Result<Vec<u64>, HashesError> {
    let size = bytes.len();
    if size > MAX_HASHES_BYTES {
        return Err(HashesError::TooLarge {
            size,
            limit: MAX_HASHES_BYTES,
        });
    }
    if size % 8 != 0 {
        return Err(HashesError::Length { size });
    }
    Ok(bytes
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("chunks of 8")))
        .collect())
}

impl Replay {
    /// The file's bytes: compact JSON and a newline. These exact bytes are what the content
    /// hash covers (ADR-034).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(self).expect("a replay serialises");
        bytes.push(b'\n');
        bytes
    }

    /// Parses a replay file: the size limit first, then the versions alone, then the strict
    /// parse (ADR-017, ADR-SVC-1). A file for a newer version is `Newer` even if it has fields
    /// this version rejects; a file that is not JSON, has no versions, or fails the strict parse
    /// at a version this game reads is `Damaged`, never `Older`.
    pub fn from_bytes(bytes: &[u8]) -> Result<Replay, ReplayError> {
        version::read_replay(
            bytes,
            MAX_STAGE_REPLAY_BYTES,
            FIRST_SCHEMA_VERSION,
            FIRST_SIM_VERSION,
            Versions::NEWEST_READ,
            &[],
            |bytes| serde_json::from_slice(bytes),
        )
        .map_err(|e| match e {
            ReplayLoadError::TooLarge { size, limit } => ReplayError::TooLarge { size, limit },
            ReplayLoadError::Older(v) => ReplayError::Older {
                schema_version: v.schema_version,
                sim_version: v.sim_version,
            },
            ReplayLoadError::Newer(v) => ReplayError::Newer {
                schema_version: v.schema_version,
                sim_version: v.sim_version,
            },
            ReplayLoadError::Damaged { line, column } => ReplayError::Damaged { line, column },
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
        unpack(&bytes, self.ticks, self.schema_version >= 2).map_err(ReplayError::BadInputs)
    }
}

/// Appends `v` as LEB128: seven bits a byte, low first.
pub(crate) fn write_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push(u8::try_from(v & 0x7f).expect("masked to 7 bits") | 0x80);
        v >>= 7;
    }
    out.push(u8::try_from(v).expect("below 0x80"));
}

/// Reads one LEB128 `u32` at `pos`, in its shortest form only.
pub(crate) fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u32, &'static str> {
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

/// −128, which is not a stick position (ADR-071): packing writes it as −127, which the engine
/// runs the same, and unpacking refuses it, so each input has one encoding.
const NO_STICK: u8 = 0x80;

/// The flags byte of a run (ADR-071): `evade` in bit 0, then `main`, `smash`, `skill`, `assist`
/// and `auto`; bit 6 (interact) is allowed at schema 2 only, bit 7 is never set.
const ALL_FLAGS: u8 = 0b11_1111;

/// Bit 6 of the flags byte at schema 2: the interact button (ADR-SIM-13). Refused at schema 1.
pub(crate) const INTERACT_FLAG: u8 = 0b100_0000;

pub(crate) fn flags_of(input: &Input) -> u8 {
    u8::from(input.evade)
        | u8::from(input.main) << 1
        | u8::from(input.smash) << 2
        | u8::from(input.skill) << 3
        | u8::from(input.assist) << 4
        | u8::from(input.auto) << 5
}

/// Packs `inputs` into runs, then base64: the schema 1 stream, without `interact`.
pub fn pack_inputs(inputs: &[Input]) -> String {
    pack_inputs_with(inputs, false)
}

/// As `pack_inputs`; `interact` keeps bit 6 of the flags (the schema 2 stage replay stream).
pub(crate) fn pack_inputs_with(inputs: &[Input], interact: bool) -> String {
    let canonical = |i: &Input| Input {
        move_x: i.move_x.max(-i8::MAX),
        move_y: i.move_y.max(-i8::MAX),
        // The pick is not part of the stream.
        pick: None,
        interact: interact && i.interact,
        ..*i
    };
    let inputs: Vec<Input> = inputs.iter().map(canonical).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < inputs.len() {
        let input = inputs[i];
        let run = inputs[i..].iter().take_while(|x| **x == input).count();
        write_varint(&mut out, u32::try_from(run).unwrap_or(u32::MAX));
        out.extend_from_slice(&block_of(&input));
        i += run;
    }
    base64::encode(&out)
}

/// The input of one 3-byte block (`flags`, `move_x`, `move_y`); `interact` says whether bit 6 is
/// allowed (schema 2). Each input has one encoding, so unknown bits and -128 are refused.
pub(crate) fn input_of_block(
    [flags, x, y]: [u8; 3],
    interact: bool,
) -> Result<Input, &'static str> {
    let allowed = if interact {
        ALL_FLAGS | INTERACT_FLAG
    } else {
        ALL_FLAGS
    };
    if flags & !allowed != 0 {
        return Err("a run has unknown flags");
    }
    if x == NO_STICK || y == NO_STICK {
        return Err("a stick axis is out of range");
    }
    Ok(Input {
        move_x: i8::from_le_bytes([x]),
        move_y: i8::from_le_bytes([y]),
        evade: flags & 1 != 0,
        main: flags & 0b10 != 0,
        smash: flags & 0b100 != 0,
        skill: flags & 0b1000 != 0,
        assist: flags & 0b1_0000 != 0,
        auto: flags & 0b10_0000 != 0,
        pick: None,
        interact: flags & INTERACT_FLAG != 0,
    })
}

/// The 3-byte block of `input` for a schema 2 stream: the schema 1 flags and bit 6 for
/// `interact`; a stick axis of -128 is written as -127, which the engine runs the same.
pub(crate) fn block_of(input: &Input) -> [u8; 3] {
    [
        flags_of(input) | u8::from(input.interact) << 6,
        input.move_x.max(-i8::MAX).to_le_bytes()[0],
        input.move_y.max(-i8::MAX).to_le_bytes()[0],
    ]
}

/// Unpacks exactly `ticks` inputs from the runs in `bytes`; `interact` says whether the
/// interact flag is allowed (schema 2 stage replays).
pub(crate) fn unpack(bytes: &[u8], ticks: u32, interact: bool) -> Result<Vec<Input>, &'static str> {
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
        let block = *bytes
            .get(pos..pos + 3)
            .and_then(|b| <&[u8; 3]>::try_from(b).ok())
            .ok_or("ends in the middle of a run")?;
        pos += 3;
        let input = input_of_block(block, interact)?;
        // Packing writes equal inputs as one run, so each list of inputs has one encoding.
        if inputs.last() == Some(&input) {
            return Err("a run repeats the input of the run before");
        }
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
        engine.step(&[*input]);
        used += 1;
    }
    Ok(Replay {
        schema_version: SCHEMA_VERSION,
        sim_version: stage.sim_version,
        seed: stage.seed,
        stage_sha256: to_hex(&content_hash::sha256(stage_bytes)),
        ticks: engine.tick(),
        final_hash: hash_to_hex(engine.state_hash()),
        inputs: pack_inputs_with(&inputs[..used], true),
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
    // The replay's own schema was range-checked when it was read; a stage file at schema 1 or 2
    // pairs with a replay at 1 or 2.
    if replay.sim_version != stage.sim_version {
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
        engine.step(&[*input]);
        if let Some(expected) = expected {
            let actual = engine.state_hash();
            if expected[i] != actual {
                return Err(ReplayError::Diverged {
                    tick: engine.tick(),
                    expected: expected[i],
                    actual,
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
        parsed_stage_hash: parsed_stage_hash::parsed_stage_hash(&stage),
    })
}
