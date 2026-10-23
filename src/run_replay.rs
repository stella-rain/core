//! The run replay (ADR-079 §5): a run of the launch game recorded as its setup, the SHA-256 of
//! the game content it ran, the tick count, the final state hash, one input per tick and one
//! stream of commands between ticks. Anyone can re-simulate it from the file and the content
//! files it used (the run stats, the rewards, the stage skills, the expertise and the
//! match-ups): the setup names a mode
//! whose content is bundled in `core`, so no stage file and no save data is needed.
//!
//! A `.replay` of a run is one JSON object. Its inputs are packed as a stage replay's are (see
//! `replay`). Its commands are packed in the order they applied, each as the tick (a LEB128
//! delta from the previous command's tick, from 0 for the first), the kind (one byte: 0
//! `choose`, 1 `spend`) and the index (LEB128). A command carries the number of ticks completed
//! when it applied, so several commands between the same two ticks have deltas of 0.
//!
//! Replays are untrusted (ADR-020): a size limit before parsing, strict parsing, no more ticks
//! than the mode is long and no more than `MAX_COMMANDS` commands. Re-simulation steps tick by
//! tick; before each tick it applies the commands carrying the current tick, in order, and fails
//! closed on an unknown kind, a command the engine refuses (a choice with no offer open or
//! outside its options, a spend after the run ended, while an offer is open or with no point
//! left, an index outside the run stats, a stat at its limit), an input while an offer is open,
//! a command left over at the end, an input after the run ended, and a final hash that differs.
//! A replay may end on an open offer: a bug report may stop while the player looks at one.
//!
//! The content hash is the SHA-256 of the bytes of the content pack the run used (ADR-079 §5,
//! ADR-080), so any content update invalidates the replay rather than letting it drift, and a
//! kept pack is found by it. The attack and weapon presets are in the stage's own JSON and in
//! the code, not in the pack, so the hash does not cover them: a change to a preset is a
//! change to what the `sim_version` does. What the rules do with the content is the
//! `sim_version`'s, which the setup carries.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::base64;
use crate::content_hash::to_hex;
use crate::engine::{ChoiceError, Engine, Outcome, SpendError};
use crate::input::Input;
use crate::pack::Pack;
use crate::presets;
use crate::replay::{hash_from_hex, hash_to_hex, pack_inputs, read_varint, unpack, write_varint};
use crate::run_content::RunContent;
use crate::run_setup::{self, RunSetup};
use crate::validate;

/// The most commands a run replay holds (a tuning value; ADR-079 decides that a cap exists).
/// Version 0 gives at most 99 points in a run; the rest is room for the offers' choices, a few
/// per falling star taken. Measured (`tests/command_cap.rs`): the longest run, a
/// `time_limit` run on normal, hard or rainy through its full overtime (90,000 ticks), played by
/// a bot that cannot die, takes every offer or rerolls and banishes at each, with 20 extra
/// rerolls, and spends every point at once, takes 188 commands at most (seeds 1 to 5). That is
/// under 500, so the cap stays at 1000, over five times the measure; it is the guard against a
/// crafted file, which `tests/command_cap.rs` pins at twice the measure or more.
pub const MAX_COMMANDS: u32 = 1000;

/// A run replay file is at most this big (ADR-020). Placeholder (R2): 1 MiB, because a
/// 20-minute `time_limit` run with a bot is about 300 KB and the longest (43 minutes with
/// overtime and the Star-Hunter's time) about 510 KB. A run replay is shared as a file, never as
/// a share code (`share`).
pub const MAX_RUN_REPLAY_BYTES: usize = 1024 * 1024;

/// What the hash derived from files replaced one by one starts with, so that it cannot be
/// mistaken for another SHA-256.
const CONTENT_TAG: &[u8] = b"stella-rain run content v0\0";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunReplay {
    /// What the run started from (ADR-079 §1).
    pub setup: RunSetup,
    /// The SHA-256 of the game content the run used, lowercase hex: of the bytes of its content
    /// pack (`content_sha256_with`, ADR-080). Any other pack invalidates the replay.
    pub content_sha256: String,
    /// Ticks recorded, at most the mode's length (a `time_limit` run's: its length, overtime and the Star-Hunter's extra time). A run that ended ends on the tick it ended.
    pub ticks: u32,
    /// The FNV-1a 64 state hash after the last tick and the commands after it (ADR-019), 16
    /// lowercase hex digits.
    pub final_hash: String,
    /// The per-tick inputs, packed as a stage replay's are, then base64.
    pub inputs: String,
    /// The commands between ticks, packed as described in the module documentation, then
    /// base64.
    pub commands: String,
}

/// The kind of a command between ticks, and its byte in the packed stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// Option `index` of the open offer (ADR-079 §3).
    Choose,
    /// A stat point on the run stat at `index` (ADR-079 §4).
    Spend,
}

impl CommandKind {
    fn byte(self) -> u8 {
        match self {
            CommandKind::Choose => 0,
            CommandKind::Spend => 1,
        }
    }

    fn from_byte(byte: u8) -> Option<CommandKind> {
        match byte {
            0 => Some(CommandKind::Choose),
            1 => Some(CommandKind::Spend),
            _ => None,
        }
    }
}

/// A command between ticks: `kind` with `index`, applied after `tick` ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickCommand {
    pub tick: u32,
    pub kind: CommandKind,
    pub index: u32,
}

/// A command of the run's stream (ADR-016, ADR-079): one input for the next tick, or a command
/// between ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Step(Input),
    Choose(u32),
    Spend(u32),
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
    /// The content hash is not this game's content for the setup's mode and content files.
    ContentChanged,
    /// More ticks than the mode is long.
    TooManyTicks {
        ticks: u32,
        limit: u32,
    },
    /// The packed inputs are damaged; the reason is a fixed phrase.
    BadInputs(&'static str),
    /// The packed commands are damaged; the reason is a fixed phrase.
    BadCommands(&'static str),
    /// More commands than `MAX_COMMANDS`.
    TooManyCommands {
        limit: u32,
    },
    /// The `final_hash` field is not 16 lowercase hex digits.
    BadHash,
    /// A choice carrying this tick was refused by the engine.
    ChoiceRefused {
        tick: u32,
        error: ChoiceError,
    },
    /// A spend carrying this tick was refused by the engine.
    SpendRefused {
        tick: u32,
        error: SpendError,
    },
    /// The run ended on this tick, before the inputs did.
    InputsAfterEnd {
        ended_at: u32,
    },
    /// An input came while an offer was open on this tick: a choice answers it first.
    OfferOpen {
        tick: u32,
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
            RunReplayError::BadCommands(why) => {
                write!(f, "the replay's commands are damaged: {why}")
            }
            RunReplayError::TooManyCommands { limit } => {
                write!(f, "the replay has more than {limit} commands")
            }
            RunReplayError::BadHash => f.write_str("the replay's final hash is not 16 hex digits"),
            RunReplayError::ChoiceRefused { tick, error } => match error {
                ChoiceError::NoOffer => write!(f, "a choice on tick {tick} has no offer open"),
                ChoiceError::OutOfRange { index, options } => write!(
                    f,
                    "a choice on tick {tick} is option {index} of an offer of {options}"
                ),
            },
            RunReplayError::SpendRefused { tick, error } => match error {
                SpendError::RunOver => {
                    write!(f, "a spend on tick {tick} comes after the run ended")
                }
                SpendError::OfferOpen => {
                    write!(f, "a spend on tick {tick} comes while an offer is open")
                }
                SpendError::NoPoint => write!(f, "a spend on tick {tick} has no point to spend"),
                SpendError::OutOfRange { index, stats } => write!(
                    f,
                    "a spend on tick {tick} is on stat {index} of {stats} run stats"
                ),
                SpendError::AtLimit { index, limit } => write!(
                    f,
                    "a spend on tick {tick} is on stat {index}, already at its limit of {limit}"
                ),
            },
            RunReplayError::InputsAfterEnd { ended_at } => {
                write!(
                    f,
                    "the run ended on tick {ended_at}, before the replay ends"
                )
            }
            RunReplayError::OfferOpen { tick } => {
                write!(f, "an input comes while the offer of tick {tick} is open")
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
    /// The commands between ticks that applied.
    pub commands: u32,
    /// The Stardust the run earned, times 10, at most `limits::MAX_RUN_STARDUST` (ADR-109): the
    /// app pays it after the run (ADR-079, ADR-101).
    pub stardust: u32,
}

/// The SHA-256 of the game content a run of `mode` uses with the bundled pack: the pack's hash.
/// `None` for a mode with no content.
pub fn content_sha256(mode: &str) -> Option<[u8; 32]> {
    content_sha256_with(mode, RunContent::bundled())
}

/// The SHA-256 of the game content a run of `mode` uses with `content`. Content read from a
/// pack has the pack's hash, the SHA-256 of the pack file's bytes (ADR-079 §5, ADR-080), whatever
/// the mode. Content with any file replaced one by one has a hash derived from the mode's file,
/// every preset's definition, and the bytes of the run stats, the rewards, the stage skills, the
/// expertise, the match-ups, the runes and the perk sets, tagged and length-prefixed: never a pack's.
/// `None` for a mode with no content.
pub fn content_sha256_with(mode: &str, content: &RunContent) -> Option<[u8; 32]> {
    if !matches!(mode, "trial" | "time_limit" | "tutorial") {
        return None;
    }
    if let Some(pack_sha256) = content.pack_sha256() {
        return Some(pack_sha256);
    }
    let file = match mode {
        "time_limit" => [content.time_limit().bytes(), content.trial_bytes()].concat(),
        "tutorial" => content.tutorial_bytes().to_vec(),
        _ => content.trial_bytes().to_vec(),
    };
    Some(hash_content(
        mode,
        &file,
        &content.run_stats().sha256(),
        &content.rewards().sha256(),
        &content.stage_skills().sha256(),
        &content.expertise().sha256(),
        &content.match_ups().sha256(),
        &content.runes().sha256(),
        &content.perk_sets().sha256(),
        &content.difficulties().sha256(),
        &content.weapons().sha256(),
        content.constellations().sha256().as_ref(),
        presets::sources(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn hash_content<'a>(
    mode: &str,
    file: &[u8],
    run_stats_sha256: &[u8; 32],
    rewards_sha256: &[u8; 32],
    stage_skills_sha256: &[u8; 32],
    expertise_sha256: &[u8; 32],
    match_ups_sha256: &[u8; 32],
    runes_sha256: &[u8; 32],
    perk_sets_sha256: &[u8; 32],
    difficulties_sha256: &[u8; 32],
    weapons_sha256: &[u8; 32],
    constellations_sha256: Option<&[u8; 32]>,
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
    part(&mut sha, b's', b"run_stats", run_stats_sha256);
    part(&mut sha, b'r', b"rewards", rewards_sha256);
    part(&mut sha, b'k', b"stage_skills", stage_skills_sha256);
    part(&mut sha, b'e', b"expertise", expertise_sha256);
    part(&mut sha, b'u', b"match_ups", match_ups_sha256);
    part(&mut sha, b'n', b"runes", runes_sha256);
    part(&mut sha, b'p', b"perk_sets", perk_sets_sha256);
    part(&mut sha, b'd', b"difficulties", difficulties_sha256);
    part(&mut sha, b'w', b"weapons", weapons_sha256);
    // Only with the file, so that content without sets hashes as it did before it.
    if let Some(constellations) = constellations_sha256 {
        part(&mut sha, b'c', b"constellations", constellations);
    }
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
        if bytes.len() > MAX_RUN_REPLAY_BYTES {
            return Err(RunReplayError::TooLarge {
                size: bytes.len(),
                limit: MAX_RUN_REPLAY_BYTES,
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

    /// The commands between ticks, in order: at most `MAX_COMMANDS`, of known kinds, none after
    /// tick `ticks`.
    pub fn unpack_commands(&self) -> Result<Vec<TickCommand>, RunReplayError> {
        let bad = RunReplayError::BadCommands;
        let bytes = base64::decode(&self.commands).map_err(|_| bad("not valid base64"))?;
        let mut commands = Vec::new();
        let mut pos = 0;
        let mut tick = 0u32;
        while pos < bytes.len() {
            if commands.len() >= MAX_COMMANDS as usize {
                return Err(RunReplayError::TooManyCommands {
                    limit: MAX_COMMANDS,
                });
            }
            let delta = read_varint(&bytes, &mut pos).map_err(bad)?;
            let kind = *bytes
                .get(pos)
                .ok_or(bad("ends in the middle of a command"))?;
            pos += 1;
            let kind = CommandKind::from_byte(kind).ok_or(bad("an unknown command kind"))?;
            if pos == bytes.len() {
                return Err(bad("ends in the middle of a command"));
            }
            let index = read_varint(&bytes, &mut pos).map_err(bad)?;
            tick = tick
                .checked_add(delta)
                .filter(|t| *t <= self.ticks)
                .ok_or(bad("a command is after the last tick"))?;
            commands.push(TickCommand { tick, kind, index });
        }
        Ok(commands)
    }
}

/// Packs commands, given in the order they applied, then base64.
pub fn pack_commands(commands: &[TickCommand]) -> String {
    let mut out = Vec::new();
    let mut previous = 0;
    for c in commands {
        let delta = c.tick.checked_sub(previous).expect("commands are in order");
        write_varint(&mut out, delta);
        out.push(c.kind.byte());
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
    commands: Vec<TickCommand>,
}

impl RunRecorder {
    /// Starts a run from `setup` (ADR-079 §1) with the bundled content files.
    pub fn start(setup: &RunSetup) -> Result<RunRecorder, RunReplayError> {
        RunRecorder::start_with(setup, RunContent::bundled())
    }

    /// As `start`, with `content` in place of the bundled files (ADR-067).
    pub fn start_with(
        setup: &RunSetup,
        content: &RunContent,
    ) -> Result<RunRecorder, RunReplayError> {
        let engine = Engine::start_with(setup, content).map_err(RunReplayError::Setup)?;
        let content =
            content_sha256_with(&setup.mode.0, content).expect("a mode that starts has content");
        Ok(RunRecorder {
            setup: setup.clone(),
            content_sha256: to_hex(&content),
            engine,
            inputs: Vec::new(),
            commands: Vec::new(),
        })
    }

    /// As `start`, with the content of `pack` in place of the bundled pack: the replay records
    /// the SHA-256 of the pack's bytes (ADR-079 §5, ADR-080).
    pub fn start_with_pack(setup: &RunSetup, pack: &Pack) -> Result<RunRecorder, RunReplayError> {
        RunRecorder::start_with(setup, pack.content())
    }

    /// The run, for its snapshot and events.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Runs the next tick with `input`. Refused, recording nothing, after the run ended or
    /// while an offer is open (ADR-079 §3: a choice answers it first).
    pub fn step(&mut self, input: Input) -> Result<(), RunReplayError> {
        if self.engine.outcome() != Outcome::Running {
            return Err(RunReplayError::InputsAfterEnd {
                ended_at: self.engine.tick(),
            });
        }
        if self.engine.offer_open() {
            return Err(RunReplayError::OfferOpen {
                tick: self.engine.tick(),
            });
        }
        self.engine.step(input);
        self.inputs.push(input);
        Ok(())
    }

    /// Chooses option `index` of the open offer. Refused, recording nothing, when the engine
    /// refuses it or the replay already holds `MAX_COMMANDS` commands.
    pub fn choose(&mut self, index: u32) -> Result<(), RunReplayError> {
        self.room()?;
        let tick = self.engine.tick();
        self.engine
            .choose(index)
            .map_err(|error| RunReplayError::ChoiceRefused { tick, error })?;
        self.record(tick, CommandKind::Choose, index);
        Ok(())
    }

    /// Spends a stat point on the run stat at `index`. Refused, recording nothing, when the
    /// engine refuses it or the replay already holds `MAX_COMMANDS` commands.
    pub fn spend(&mut self, index: u32) -> Result<(), RunReplayError> {
        self.room()?;
        let tick = self.engine.tick();
        self.engine
            .spend(index)
            .map_err(|error| RunReplayError::SpendRefused { tick, error })?;
        self.record(tick, CommandKind::Spend, index);
        Ok(())
    }

    fn room(&self) -> Result<(), RunReplayError> {
        if self.commands.len() >= MAX_COMMANDS as usize {
            return Err(RunReplayError::TooManyCommands {
                limit: MAX_COMMANDS,
            });
        }
        Ok(())
    }

    fn record(&mut self, tick: u32, kind: CommandKind, index: u32) {
        self.commands.push(TickCommand { tick, kind, index });
    }

    /// The replay of the run so far: complete once the run has ended, a bug report before.
    pub fn replay(&self) -> RunReplay {
        RunReplay {
            setup: self.setup.clone(),
            content_sha256: self.content_sha256.clone(),
            ticks: self.engine.tick(),
            final_hash: hash_to_hex(self.engine.state_hash()),
            inputs: pack_inputs(&self.inputs),
            commands: pack_commands(&self.commands),
        }
    }
}

/// Runs `commands` from `setup`, with the bundled content files, and returns the replay. Recording
/// stops on the tick the run ends, so commands after it are left out; a command the engine
/// refuses fails closed.
pub fn record_run(setup: &RunSetup, commands: &[Command]) -> Result<RunReplay, RunReplayError> {
    record_run_with(setup, RunContent::bundled(), commands)
}

/// As `record_run`, with the content files `content` in place of the bundled ones.
pub fn record_run_with(
    setup: &RunSetup,
    content: &RunContent,
    commands: &[Command],
) -> Result<RunReplay, RunReplayError> {
    let mut recorder = RunRecorder::start_with(setup, content)?;
    for command in commands {
        if recorder.engine.outcome() != Outcome::Running {
            break;
        }
        match *command {
            Command::Step(input) => recorder.step(input)?,
            Command::Choose(index) => recorder.choose(index)?,
            Command::Spend(index) => recorder.spend(index)?,
        }
    }
    Ok(recorder.replay())
}

/// As `record_run`, with the content of `pack` in place of the bundled pack: the replay records
/// the SHA-256 of the pack's bytes.
pub fn record_run_with_pack(
    setup: &RunSetup,
    pack: &Pack,
    commands: &[Command],
) -> Result<RunReplay, RunReplayError> {
    record_run_with(setup, pack.content(), commands)
}

/// Re-simulates a run replay from its bytes and the bundled content files, and checks every
/// command and the final hash (ADR-079 §5). The whole file is untrusted.
pub fn verify_run(replay_bytes: &[u8]) -> Result<RunVerified, RunReplayError> {
    verify_inner(replay_bytes, RunContent::bundled(), None)
}

/// As `verify_run`, with the content files the run used in place of the bundled ones.
pub fn verify_run_with(
    replay_bytes: &[u8],
    content: &RunContent,
) -> Result<RunVerified, RunReplayError> {
    verify_inner(replay_bytes, content, None)
}

/// As `verify_run`, with the content of `pack`, the one the replay's `content_sha256` names:
/// refuses a replay recorded with any other pack (`ContentChanged`).
pub fn verify_run_with_pack(
    replay_bytes: &[u8],
    pack: &Pack,
) -> Result<RunVerified, RunReplayError> {
    verify_inner(replay_bytes, pack.content(), None)
}

/// As `verify_run_against`, with the content of `pack`.
pub fn verify_run_against_with_pack(
    replay_bytes: &[u8],
    pack: &Pack,
    expected: &[u64],
) -> Result<RunVerified, RunReplayError> {
    verify_inner(replay_bytes, pack.content(), Some(expected))
}

/// As `verify_run`, and also compares the hash after every tick (before the commands that
/// follow it) with `expected`, naming the first tick that differs: what a corpus check reports
/// (ADR-019).
pub fn verify_run_against(
    replay_bytes: &[u8],
    expected: &[u64],
) -> Result<RunVerified, RunReplayError> {
    verify_inner(replay_bytes, RunContent::bundled(), Some(expected))
}

/// As `verify_run_against`, with the content files the run used in place of the bundled ones.
pub fn verify_run_against_with(
    replay_bytes: &[u8],
    content: &RunContent,
    expected: &[u64],
) -> Result<RunVerified, RunReplayError> {
    verify_inner(replay_bytes, content, Some(expected))
}

fn verify_inner(
    replay_bytes: &[u8],
    run_content: &RunContent,
    expected: Option<&[u64]>,
) -> Result<RunVerified, RunReplayError> {
    let replay = RunReplay::from_bytes(replay_bytes)?;
    let stage = run_setup::stage_in(&replay.setup, run_content).map_err(RunReplayError::Setup)?;
    let content = content_sha256_with(&replay.setup.mode.0, run_content)
        .ok_or(RunReplayError::ContentChanged)?;
    if replay.content_sha256 != to_hex(&content) {
        return Err(RunReplayError::ContentChanged);
    }
    let final_hash = hash_from_hex(&replay.final_hash).ok_or(RunReplayError::BadHash)?;
    let inputs = replay.unpack_inputs(run_setup::max_replay_ticks(
        &replay.setup,
        &stage,
        run_content.time_limit(),
    ))?;
    let commands = replay.unpack_commands()?;
    if let Some(expected) = expected {
        if expected.len() != inputs.len() {
            return Err(RunReplayError::HashCount {
                expected: replay.ticks,
                given: expected.len(),
            });
        }
    }

    let mut engine =
        Engine::start_with(&replay.setup, run_content).map_err(RunReplayError::Setup)?;
    let mut next = 0;
    for (i, input) in inputs.iter().enumerate() {
        apply_commands(&mut engine, &commands, &mut next)?;
        if engine.outcome() != Outcome::Running {
            return Err(RunReplayError::InputsAfterEnd {
                ended_at: engine.tick(),
            });
        }
        if engine.offer_open() {
            return Err(RunReplayError::OfferOpen {
                tick: engine.tick(),
            });
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
    apply_commands(&mut engine, &commands, &mut next)?;
    // Every tick from 0 to the last took its commands, and none is after the last; this only
    // keeps the check closed if that ever stops being true.
    if next < commands.len() {
        return Err(RunReplayError::BadCommands(
            "a command is after the last tick",
        ));
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
        commands: u32::try_from(commands.len()).unwrap_or(u32::MAX),
        stardust: engine.stardust(),
    })
}

/// Applies the commands that carry the engine's current tick, from `next` on, in order.
fn apply_commands(
    engine: &mut Engine,
    commands: &[TickCommand],
    next: &mut usize,
) -> Result<(), RunReplayError> {
    while let Some(c) = commands.get(*next).filter(|c| c.tick == engine.tick()) {
        let tick = c.tick;
        match c.kind {
            CommandKind::Choose => engine
                .choose(c.index)
                .map_err(|error| RunReplayError::ChoiceRefused { tick, error })?,
            CommandKind::Spend => engine
                .spend(c.index)
                .map_err(|error| RunReplayError::SpendRefused { tick, error })?,
        }
        *next += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::hash_content;

    const SOURCES: [(u8, &str, &str); 2] = [(b'a', "shot", "[1]"), (b'w', "gun", "{}")];
    const STATS: [u8; 32] = [7; 32];
    const REWARDS: [u8; 32] = [9; 32];
    const SKILLS: [u8; 32] = [5; 32];
    const EXPERTISE: [u8; 32] = [3; 32];
    const MATCH_UPS: [u8; 32] = [1; 32];
    const RUNES: [u8; 32] = [2; 32];
    const PERKS: [u8; 32] = [4; 32];
    const DIFFICULTIES: [u8; 32] = [6; 32];
    const WEAPONS_SHA: [u8; 32] = [8; 32];

    fn hash(mode: &str, file: &[u8], sources: &[(u8, &'static str, &'static str)]) -> [u8; 32] {
        hash_content(
            mode,
            file,
            &STATS,
            &REWARDS,
            &SKILLS,
            &EXPERTISE,
            &MATCH_UPS,
            &RUNES,
            &PERKS,
            &DIFFICULTIES,
            &WEAPONS_SHA,
            None,
            sources.iter().copied(),
        )
    }

    fn files(
        stats: &[u8; 32],
        rewards: &[u8; 32],
        skills: &[u8; 32],
        expertise: &[u8; 32],
    ) -> [u8; 32] {
        hash_content(
            "trial",
            b"{}",
            stats,
            rewards,
            skills,
            expertise,
            &MATCH_UPS,
            &RUNES,
            &PERKS,
            &DIFFICULTIES,
            &WEAPONS_SHA,
            None,
            SOURCES.iter().copied(),
        )
    }

    #[test]
    fn the_content_hash_covers_the_mode_the_content_files_and_every_preset() {
        let base = hash("trial", b"{}", &SOURCES);
        assert_eq!(base, hash("trial", b"{}", &SOURCES));
        assert_ne!(base, hash("trial", b"{ }", &SOURCES), "the mode's file");
        assert_ne!(base, hash("other", b"{}", &SOURCES), "the mode's ID");
        assert_eq!(base, files(&STATS, &REWARDS, &SKILLS, &EXPERTISE));
        assert_ne!(
            base,
            files(&[8; 32], &REWARDS, &SKILLS, &EXPERTISE),
            "the run stats"
        );
        assert_ne!(
            base,
            files(&STATS, &[8; 32], &SKILLS, &EXPERTISE),
            "the rewards"
        );
        assert_ne!(
            base,
            files(&STATS, &REWARDS, &[8; 32], &EXPERTISE),
            "the stage skills"
        );
        assert_ne!(
            base,
            files(&STATS, &REWARDS, &SKILLS, &[8; 32]),
            "the expertise"
        );
        assert_ne!(
            base,
            hash_content(
                "trial",
                b"{}",
                &STATS,
                &REWARDS,
                &SKILLS,
                &EXPERTISE,
                &[8; 32],
                &RUNES,
                &PERKS,
                &DIFFICULTIES,
                &WEAPONS_SHA,
                None,
                SOURCES.iter().copied(),
            ),
            "the match-ups"
        );
        assert_ne!(
            base,
            hash_content(
                "trial",
                b"{}",
                &STATS,
                &REWARDS,
                &SKILLS,
                &EXPERTISE,
                &MATCH_UPS,
                &[8; 32],
                &PERKS,
                &DIFFICULTIES,
                &WEAPONS_SHA,
                None,
                SOURCES.iter().copied(),
            ),
            "the runes"
        );
        assert_ne!(
            base,
            hash_content(
                "trial",
                b"{}",
                &STATS,
                &REWARDS,
                &SKILLS,
                &EXPERTISE,
                &MATCH_UPS,
                &RUNES,
                &[8; 32],
                &DIFFICULTIES,
                &WEAPONS_SHA,
                None,
                SOURCES.iter().copied(),
            ),
            "the perk sets"
        );
        assert_ne!(
            base,
            hash_content(
                "trial",
                b"{}",
                &STATS,
                &REWARDS,
                &SKILLS,
                &EXPERTISE,
                &MATCH_UPS,
                &RUNES,
                &PERKS,
                &[8; 32],
                &WEAPONS_SHA,
                None,
                SOURCES.iter().copied(),
            ),
            "the difficulties"
        );
        assert_ne!(
            base,
            hash_content(
                "trial",
                b"{}",
                &STATS,
                &REWARDS,
                &SKILLS,
                &EXPERTISE,
                &MATCH_UPS,
                &RUNES,
                &PERKS,
                &DIFFICULTIES,
                &[11; 32],
                None,
                SOURCES.iter().copied(),
            ),
            "the weapons"
        );
        // Sets of named stars change it, and no file leaves it as it was.
        let with = |sets: Option<&[u8; 32]>| {
            hash_content(
                "trial",
                b"{}",
                &STATS,
                &REWARDS,
                &SKILLS,
                &EXPERTISE,
                &MATCH_UPS,
                &RUNES,
                &PERKS,
                &DIFFICULTIES,
                &WEAPONS_SHA,
                sets,
                SOURCES.iter().copied(),
            )
        };
        assert_eq!(base, with(None));
        assert_ne!(base, with(Some(&[9; 32])), "the constellations");
        assert_ne!(with(Some(&[9; 32])), with(Some(&[10; 32])));
        assert_ne!(
            base,
            files(&REWARDS, &STATS, &SKILLS, &EXPERTISE),
            "which file is which"
        );
        assert_ne!(
            base,
            files(&STATS, &SKILLS, &REWARDS, &EXPERTISE),
            "which file is which"
        );
        assert_ne!(
            base,
            files(&STATS, &REWARDS, &EXPERTISE, &SKILLS),
            "which file is which"
        );
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
