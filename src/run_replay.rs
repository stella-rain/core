//! The run replay (ADR-051 §3): a run of the launch game recorded as its setup, the SHA-256 of
//! the game content it ran, the tick count, the final state hash, one input per tick and one
//! choice per offer. Anyone can re-simulate it from the file alone: the setup names a mode whose
//! content is bundled in `core`, so no stage file and no save data is needed.
//!
//! A `.replay` of a run is one JSON object. Its inputs are packed as a stage replay's are (see
//! `replay`). Its choices are packed in order as `(tick, index)` pairs, each two LEB128 numbers:
//! the tick as a delta from the previous choice's (from 0 for the first), then the index into
//! the offer. A choice carries the tick whose end opened its offer, so several choices on one
//! tick (a level-up opening the next waiting offer) have deltas of 0.
//!
//! Replays are untrusted (ADR-020): a size limit before parsing, strict parsing, no more ticks
//! than the mode is long and no more than `MAX_CHOICES` choices. Re-simulation steps tick by
//! tick; before each tick it takes the choices carrying the current tick, and fails closed on a
//! choice the engine refuses (no offer open, an index outside the offer), on an input while an
//! offer is still open (a missing choice), on an input after the run ended, and on a final hash
//! that differs. A run that ends at an open offer, with no choice after it, is complete: the
//! player stopped there.
//!
//! The content hash covers the mode's bundled file and every attack and weapon preset's
//! definition, so any content update invalidates the replay rather than letting it drift. What
//! the rules do with the content is the `sim_version`'s, which the setup carries.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::base64;
use crate::content_hash::to_hex;
use crate::engine::{ChoiceError, Engine, Outcome};
use crate::input::Input;
use crate::presets;
use crate::replay::{
    MAX_REPLAY_BYTES, hash_from_hex, hash_to_hex, pack_inputs, read_varint, unpack, write_varint,
};
use crate::run_setup::{self, RunSetup};
use crate::validate;

/// The most choices a run replay holds (a tuning value; ADR-051 decides that a cap exists).
/// Version 0 offers at most 25 times in a run, one for each stack of each upgrade; the rest is
/// room for rerolls.
pub const MAX_CHOICES: u32 = 1000;

/// What the content hash starts with, so that it cannot be mistaken for another SHA-256.
const CONTENT_TAG: &[u8] = b"stella-rain run content v0\0";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunReplay {
    /// What the run started from (ADR-051 §1).
    pub setup: RunSetup,
    /// The SHA-256 of the game content the run used, lowercase hex: the setup's mode and every
    /// preset (`content_sha256`). Another game version's content invalidates the replay.
    pub content_sha256: String,
    /// Ticks recorded, at most the mode's length. A run that ended ends on the tick it ended.
    pub ticks: u32,
    /// The FNV-1a 64 state hash after the last tick and the choices made at its end (ADR-019),
    /// 16 lowercase hex digits.
    pub final_hash: String,
    /// The per-tick inputs, packed as a stage replay's are, then base64.
    pub inputs: String,
    /// The choices, packed as described in the module documentation, then base64.
    pub choices: String,
}

/// A choice between ticks: option `index` of the offer that the end of `tick` opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub tick: u32,
    pub index: u32,
}

/// A command of the run's stream (ADR-016, ADR-051): one input for the next tick, or a choice
/// in the open offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Step(Input),
    Choose(u32),
}

/// Why a run replay was not recorded or not accepted. Messages never repeat text from files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunReplayError {
    TooLarge {
        size: usize,
        limit: usize,
    },
    /// Not JSON, or not the run replay format.
    Parse {
        line: usize,
        column: usize,
    },
    /// The setup is not valid (ADR-020).
    Setup(validate::Error),
    /// The content hash is not this game's content for the setup's mode.
    ContentChanged,
    /// More ticks than the mode is long.
    TooManyTicks {
        ticks: u32,
        limit: u32,
    },
    /// The packed inputs are damaged; the reason is a fixed phrase.
    BadInputs(&'static str),
    /// The packed choices are damaged; the reason is a fixed phrase.
    BadChoices(&'static str),
    /// More choices than `MAX_CHOICES`.
    TooManyChoices {
        limit: u32,
    },
    /// The `final_hash` field is not 16 lowercase hex digits.
    BadHash,
    /// The offer opened at the end of this tick has no choice, and the run goes on.
    MissingChoice {
        tick: u32,
    },
    /// A choice carrying this tick was refused by the engine.
    ChoiceRefused {
        tick: u32,
        error: ChoiceError,
    },
    /// The run ended on this tick, before the inputs did.
    InputsAfterEnd {
        ended_at: u32,
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

impl fmt::Display for RunReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunReplayError::TooLarge { size, limit } => {
                write!(f, "the replay is {size} bytes; the limit is {limit}")
            }
            RunReplayError::Parse { line, column } => {
                write!(f, "not a valid run replay (line {line}, column {column})")
            }
            RunReplayError::Setup(e) => write!(f, "the run setup is not valid:\n{e}"),
            RunReplayError::ContentChanged => {
                f.write_str("the replay was recorded with other game content")
            }
            RunReplayError::TooManyTicks { ticks, limit } => {
                write!(f, "the replay has {ticks} ticks; the mode allows {limit}")
            }
            RunReplayError::BadInputs(why) => write!(f, "the replay's inputs are damaged: {why}"),
            RunReplayError::BadChoices(why) => {
                write!(f, "the replay's choices are damaged: {why}")
            }
            RunReplayError::TooManyChoices { limit } => {
                write!(f, "the replay has more than {limit} choices")
            }
            RunReplayError::BadHash => f.write_str("the replay's final hash is not 16 hex digits"),
            RunReplayError::MissingChoice { tick } => {
                write!(f, "the offer opened on tick {tick} has no choice")
            }
            RunReplayError::ChoiceRefused { tick, error } => match error {
                ChoiceError::NoOffer => write!(f, "a choice on tick {tick} has no offer open"),
                ChoiceError::OutOfRange { index, options } => write!(
                    f,
                    "a choice on tick {tick} is option {index} of an offer of {options}"
                ),
            },
            RunReplayError::InputsAfterEnd { ended_at } => {
                write!(
                    f,
                    "the run ended on tick {ended_at}, before the replay ends"
                )
            }
            RunReplayError::WrongFinalHash { expected, actual } => write!(
                f,
                "the final hash is {actual:016x}, not the replay's {expected:016x}"
            ),
            RunReplayError::Diverged { tick } => {
                write!(f, "the run differs from the expected hashes on tick {tick}")
            }
            RunReplayError::HashCount { expected, given } => {
                write!(f, "expected {expected} per-tick hashes, got {given}")
            }
        }
    }
}

impl std::error::Error for RunReplayError {}

/// What a successful verification proves: the run as recorded reaches this tick and hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunVerified {
    pub ticks: u32,
    pub final_hash: u64,
    /// How the run stood at the end: a bug report may stop while it is still running.
    pub outcome: Outcome,
    pub choices: u32,
}

/// The SHA-256 of the game content a run of `mode` uses: the mode's bundled file and every
/// preset's definition, tagged and length-prefixed. `None` for a mode with no content.
pub fn content_sha256(mode: &str) -> Option<[u8; 32]> {
    let file = run_setup::mode_stage(mode)?;
    Some(hash_content(mode, file, presets::sources()))
}

fn hash_content<'a>(
    mode: &str,
    file: &[u8],
    sources: impl Iterator<Item = (u8, &'a str, &'a str)>,
) -> [u8; 32] {
    fn part(sha: &mut Sha256, kind: u8, id: &[u8], body: &[u8]) {
        let len = |b: &[u8]| u32::try_from(b.len()).unwrap_or(u32::MAX).to_le_bytes();
        sha.update([kind]);
        sha.update(len(id));
        sha.update(id);
        sha.update(len(body));
        sha.update(body);
    }
    let mut sha = Sha256::new();
    sha.update(CONTENT_TAG);
    part(&mut sha, b'm', mode.as_bytes(), file);
    for (kind, id, definition) in sources {
        part(&mut sha, kind, id.as_bytes(), definition.as_bytes());
    }
    sha.finalize().into()
}

impl RunReplay {
    /// The file's bytes: compact JSON and a newline.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(self).expect("a run replay serialises");
        bytes.push(b'\n');
        bytes
    }

    /// Parses a run replay file: size limit first, then strict parsing.
    pub fn from_bytes(bytes: &[u8]) -> Result<RunReplay, RunReplayError> {
        if bytes.len() > MAX_REPLAY_BYTES {
            return Err(RunReplayError::TooLarge {
                size: bytes.len(),
                limit: MAX_REPLAY_BYTES,
            });
        }
        serde_json::from_slice(bytes).map_err(|e| RunReplayError::Parse {
            line: e.line(),
            column: e.column(),
        })
    }

    /// The inputs, one per tick: exactly `ticks` of them, and `ticks` at most `max_ticks`.
    pub fn unpack_inputs(&self, max_ticks: u32) -> Result<Vec<Input>, RunReplayError> {
        if self.ticks > max_ticks {
            return Err(RunReplayError::TooManyTicks {
                ticks: self.ticks,
                limit: max_ticks,
            });
        }
        let bytes = base64::decode(&self.inputs)
            .map_err(|_| RunReplayError::BadInputs("not valid base64"))?;
        unpack(&bytes, self.ticks).map_err(RunReplayError::BadInputs)
    }

    /// The choices in order: at most `MAX_CHOICES`, none after tick `ticks`.
    pub fn unpack_choices(&self) -> Result<Vec<Choice>, RunReplayError> {
        let bad = RunReplayError::BadChoices;
        let bytes = base64::decode(&self.choices).map_err(|_| bad("not valid base64"))?;
        let mut choices = Vec::new();
        let mut pos = 0;
        let mut tick = 0u32;
        while pos < bytes.len() {
            if choices.len() >= MAX_CHOICES as usize {
                return Err(RunReplayError::TooManyChoices { limit: MAX_CHOICES });
            }
            let delta = read_varint(&bytes, &mut pos).map_err(bad)?;
            if pos == bytes.len() {
                return Err(bad("ends in the middle of a choice"));
            }
            let index = read_varint(&bytes, &mut pos).map_err(bad)?;
            tick = tick
                .checked_add(delta)
                .filter(|t| *t <= self.ticks)
                .ok_or(bad("a choice is after the last tick"))?;
            choices.push(Choice { tick, index });
        }
        Ok(choices)
    }
}

/// Packs choices, given in the order they were made, then base64.
pub fn pack_choices(choices: &[Choice]) -> String {
    let mut out = Vec::new();
    let mut previous = 0;
    for c in choices {
        let delta = c.tick.checked_sub(previous).expect("choices are in order");
        write_varint(&mut out, delta);
        write_varint(&mut out, c.index);
        previous = c.tick;
    }
    base64::encode(&out)
}

/// A run being played and recorded: every command goes through it to the engine, and only the
/// commands the engine takes are recorded, so the replay always verifies.
#[derive(Clone, Debug)]
pub struct RunRecorder {
    setup: RunSetup,
    content_sha256: String,
    engine: Engine,
    inputs: Vec<Input>,
    choices: Vec<Choice>,
}

impl RunRecorder {
    /// Starts a run from `setup` (ADR-051 §1).
    pub fn start(setup: &RunSetup) -> Result<RunRecorder, RunReplayError> {
        let engine = Engine::start(setup).map_err(RunReplayError::Setup)?;
        let content = content_sha256(&setup.mode.0).expect("a mode that starts has content");
        Ok(RunRecorder {
            setup: setup.clone(),
            content_sha256: to_hex(&content),
            engine,
            inputs: Vec::new(),
            choices: Vec::new(),
        })
    }

    /// The run, for its snapshot, events and offer.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Runs the next tick with `input`. Refused, recording nothing, while an offer is open or
    /// after the run ended.
    pub fn step(&mut self, input: Input) -> Result<(), RunReplayError> {
        let tick = self.engine.tick();
        if self.engine.offer().is_some() {
            return Err(RunReplayError::MissingChoice { tick });
        }
        if self.engine.outcome() != Outcome::Running {
            return Err(RunReplayError::InputsAfterEnd { ended_at: tick });
        }
        self.engine.step(input);
        self.inputs.push(input);
        Ok(())
    }

    /// Chooses option `index` of the open offer. Refused, recording nothing, when the engine
    /// refuses it or the replay already holds `MAX_CHOICES` choices.
    pub fn choose(&mut self, index: u32) -> Result<(), RunReplayError> {
        if self.choices.len() >= MAX_CHOICES as usize {
            return Err(RunReplayError::TooManyChoices { limit: MAX_CHOICES });
        }
        let tick = self.engine.tick();
        self.engine
            .choose(index)
            .map_err(|error| RunReplayError::ChoiceRefused { tick, error })?;
        self.choices.push(Choice { tick, index });
        Ok(())
    }

    /// The replay of the run so far: complete once the run has ended, a bug report before.
    pub fn replay(&self) -> RunReplay {
        RunReplay {
            setup: self.setup.clone(),
            content_sha256: self.content_sha256.clone(),
            ticks: self.engine.tick(),
            final_hash: hash_to_hex(self.engine.state_hash()),
            inputs: pack_inputs(&self.inputs),
            choices: pack_choices(&self.choices),
        }
    }
}

/// Runs `commands` from `setup` and returns the replay. Recording stops on the tick the run
/// ends, so commands after it are left out; a step while an offer is open, or a choice the
/// engine refuses, fails closed.
pub fn record_run(setup: &RunSetup, commands: &[Command]) -> Result<RunReplay, RunReplayError> {
    let mut recorder = RunRecorder::start(setup)?;
    for command in commands {
        if recorder.engine.outcome() != Outcome::Running {
            break;
        }
        match *command {
            Command::Step(input) => recorder.step(input)?,
            Command::Choose(index) => recorder.choose(index)?,
        }
    }
    Ok(recorder.replay())
}

/// Re-simulates a run replay from its bytes alone and checks every choice and the final hash
/// (ADR-051 §3). The whole file is untrusted.
pub fn verify_run(replay_bytes: &[u8]) -> Result<RunVerified, RunReplayError> {
    verify_inner(replay_bytes, None)
}

/// As `verify_run`, and also compares the hash after every tick (before that tick's choices)
/// with `expected`, naming the first tick that differs: what a corpus check reports (ADR-019).
pub fn verify_run_against(
    replay_bytes: &[u8],
    expected: &[u64],
) -> Result<RunVerified, RunReplayError> {
    verify_inner(replay_bytes, Some(expected))
}

fn verify_inner(
    replay_bytes: &[u8],
    expected: Option<&[u64]>,
) -> Result<RunVerified, RunReplayError> {
    let replay = RunReplay::from_bytes(replay_bytes)?;
    let stage = run_setup::stage(&replay.setup).map_err(RunReplayError::Setup)?;
    let content = content_sha256(&replay.setup.mode.0).ok_or(RunReplayError::ContentChanged)?;
    if replay.content_sha256 != to_hex(&content) {
        return Err(RunReplayError::ContentChanged);
    }
    let final_hash = hash_from_hex(&replay.final_hash).ok_or(RunReplayError::BadHash)?;
    let inputs = replay.unpack_inputs(stage.length_ticks)?;
    let choices = replay.unpack_choices()?;
    if let Some(expected) = expected {
        if expected.len() != inputs.len() {
            return Err(RunReplayError::HashCount {
                expected: replay.ticks,
                given: expected.len(),
            });
        }
    }

    let mut engine = Engine::start(&replay.setup).map_err(RunReplayError::Setup)?;
    let mut next = 0;
    for (i, input) in inputs.iter().enumerate() {
        take_choices(&mut engine, &choices, &mut next)?;
        let tick = engine.tick();
        if engine.offer().is_some() {
            return Err(RunReplayError::MissingChoice { tick });
        }
        if engine.outcome() != Outcome::Running {
            return Err(RunReplayError::InputsAfterEnd { ended_at: tick });
        }
        engine.step(*input);
        if let Some(expected) = expected {
            if expected[i] != engine.state_hash() {
                return Err(RunReplayError::Diverged {
                    tick: engine.tick(),
                });
            }
        }
    }
    take_choices(&mut engine, &choices, &mut next)?;
    // Every tick from 0 to the last took its choices, and none is after the last; this only
    // keeps the check closed if that ever stops being true.
    if let Some(c) = choices.get(next) {
        return Err(RunReplayError::ChoiceRefused {
            tick: c.tick,
            error: ChoiceError::NoOffer,
        });
    }
    let actual = engine.state_hash();
    if actual != final_hash {
        return Err(RunReplayError::WrongFinalHash {
            expected: final_hash,
            actual,
        });
    }
    Ok(RunVerified {
        ticks: engine.tick(),
        final_hash: actual,
        outcome: engine.outcome(),
        choices: u32::try_from(choices.len()).unwrap_or(u32::MAX),
    })
}

/// Applies the choices that carry the engine's current tick, from `next` on.
fn take_choices(
    engine: &mut Engine,
    choices: &[Choice],
    next: &mut usize,
) -> Result<(), RunReplayError> {
    while let Some(c) = choices.get(*next).filter(|c| c.tick == engine.tick()) {
        engine
            .choose(c.index)
            .map_err(|error| RunReplayError::ChoiceRefused {
                tick: c.tick,
                error,
            })?;
        *next += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::hash_content;

    const SOURCES: [(u8, &str, &str); 2] = [(b'a', "shot", "[1]"), (b'w', "gun", "{}")];

    fn hash(mode: &str, file: &[u8], sources: &[(u8, &'static str, &'static str)]) -> [u8; 32] {
        hash_content(mode, file, sources.iter().copied())
    }

    #[test]
    fn the_content_hash_covers_the_mode_and_every_preset() {
        let base = hash("trial", b"{}", &SOURCES);
        assert_eq!(base, hash("trial", b"{}", &SOURCES));
        assert_ne!(base, hash("trial", b"{ }", &SOURCES), "the mode's file");
        assert_ne!(base, hash("other", b"{}", &SOURCES), "the mode's ID");
        let changed = [(b'a', "shot", "[2]"), SOURCES[1]];
        assert_ne!(base, hash("trial", b"{}", &changed), "a definition");
        let renamed = [(b'a', "shot2", "[1]"), SOURCES[1]];
        assert_ne!(base, hash("trial", b"{}", &renamed), "an ID");
        let kind = [(b'w', "shot", "[1]"), SOURCES[1]];
        assert_ne!(base, hash("trial", b"{}", &kind), "a kind");
        let reordered = [SOURCES[1], SOURCES[0]];
        assert_ne!(base, hash("trial", b"{}", &reordered), "the order");
        assert_ne!(
            base,
            hash("trial", b"{}", &SOURCES[..1]),
            "a preset left out"
        );
        // A byte moved across the boundary of an ID and its definition.
        let moved = [(b'a', "shot[", "1]"), SOURCES[1]];
        assert_ne!(base, hash("trial", b"{}", &moved), "the boundaries");
    }
}
