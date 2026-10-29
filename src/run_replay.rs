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
//! `choose`; 1 was `spend`, removed with DS18 and refused) and the index (LEB128). A command carries the number of ticks completed
//! when it applied, so several commands between the same two ticks have deltas of 0.
//!
//! Replays are untrusted (ADR-020): a size limit before parsing, strict parsing, no more ticks
//! than the mode is long and no more than `MAX_COMMANDS` commands. Re-simulation steps tick by
//! tick; before each tick it applies the commands carrying the current tick, in order, and fails
//! closed on an unknown kind, a command the engine refuses (a choice with no offer open or
//! outside its options), an input while an offer is open,
//! a command left over at the end, an input after the run ended, and a final hash that differs.
//! A replay may end on an open offer: a bug report may stop while the player looks at one.
//! `RunRecorder::resume_with_pack` takes such a prefix of a run still running, re-simulates it
//! with the same checks (a final hash that differs is `PrefixHashDiffers`) and returns a live
//! recorder at the cut, an open offer included, so a later `replay()` is one run from tick 0.
//!
//! A replay also holds `checkpoints`: the state hash after every 60th tick (ADR-REPL-4), each as
//! 16 lowercase hex digits, concatenated, taken right after the tick and before the commands
//! that follow it. The recorder writes them; re-simulation checks each as it passes it, so a run
//! that no longer reproduces fails naming the first second that differs (`CheckpointDiffers`),
//! and a field that is not `ticks / 60` hashes is `BadCheckpoints`. A run case of the corpus
//! keeps its expected hashes in them, in place of a hash per tick.
//!
//! The content hash is the SHA-256 of the bytes of the content pack the run used (ADR-079 §5,
//! ADR-080), so any content update invalidates the replay rather than letting it drift, and a
//! kept pack is found by it. The attack presets and the weapons are files of the pack
//! (ADR-CONT-8, CS1b-1, WP1), so a change to a preset is a content release and a new hash.
//! What the rules do with the content is the `sim_version`'s, which the setup carries.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::base64;
use crate::content_hash::to_hex;
use crate::engine::{ChoiceError, Engine, Outcome};
use crate::input::Input;
use crate::ledger::RunLedger;
use crate::pack::Pack;
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

/// A run replay holds the state hash after every this many ticks (ADR-REPL-4): one second.
pub const CHECKPOINT_TICKS: u32 = 60;

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
    /// The FNV-1a 64 state hash after every 60th tick (60, 120, 180, ...), each taken right
    /// after that tick and before the commands that follow it, each 16 lowercase hex digits,
    /// concatenated: `16 * (ticks / 60)` digits, empty for fewer than 60 ticks.
    pub checkpoints: String,
    /// The per-tick inputs, packed as a stage replay's are, then base64.
    pub inputs: String,
    /// The commands between ticks, packed as described in the module documentation, then
    /// base64.
    pub commands: String,
}

/// The byte `spend` had in the packed stream (DS18 removed it): refused, never reused.
const RETIRED_SPEND: u8 = 1;

/// The kind of a command between ticks, and its byte in the packed stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// Option `index` of the open offer (ADR-079 §3).
    Choose,
}

impl CommandKind {
    fn byte(self) -> u8 {
        match self {
            CommandKind::Choose => 0,
        }
    }

    fn from_byte(byte: u8) -> Option<CommandKind> {
        match byte {
            0 => Some(CommandKind::Choose),
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
}

/// Why a run replay was not recorded or not accepted. Messages never repeat text from files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunReplayError {
    TooLarge {
        size: usize,
        limit: usize,
    },
    /// Not JSON, or not the run replay format.
    Damaged {
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
        expected: u64,
        actual: u64,
    },
    /// The expected per-tick hashes are not one per tick.
    HashCount {
        expected: u32,
        given: usize,
    },
    /// The `checkpoints` field is not one 16-digit hash per 60 ticks.
    BadCheckpoints,
    /// Re-simulation differs from the replay's checkpoint of this second (counted from 1).
    CheckpointDiffers {
        second: u32,
    },
    /// A replay prefix re-simulates to another state at its cut than its hash says.
    PrefixHashDiffers {
        tick: u32,
    },
    /// A replay prefix is of a run that already ended: there is nothing to resume.
    PrefixEnded {
        tick: u32,
    },
}

impl fmt::Display for RunReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunReplayError::TooLarge { size, limit } => {
                write!(f, "the replay is {size} bytes; the limit is {limit}")
            }
            RunReplayError::Damaged { line, column } => {
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
            RunReplayError::Diverged {
                tick,
                expected,
                actual,
            } => write!(
                f,
                "the run differs from the expected hashes on tick {tick}: expected {expected:016x}, got {actual:016x}"
            ),
            RunReplayError::HashCount { expected, given } => {
                write!(f, "expected {expected} per-tick hashes, got {given}")
            }
            RunReplayError::BadCheckpoints => {
                f.write_str("the replay's checkpoints are not one 16-digit hash per 60 ticks")
            }
            RunReplayError::CheckpointDiffers { second } => write!(
                f,
                "the run differs from the replay's checkpoint of second {second} (between tick {} and tick {})",
                (second - 1) * CHECKPOINT_TICKS,
                second * CHECKPOINT_TICKS
            ),
            RunReplayError::PrefixHashDiffers { tick } => write!(
                f,
                "the run re-simulated to tick {tick} reaches another state than the replay prefix says"
            ),
            RunReplayError::PrefixEnded { tick } => {
                write!(f, "the replay prefix is of a run that ended on tick {tick}")
            }
        }
    }
}

impl std::error::Error for RunReplayError {}

/// What a successful verification proves: the run as recorded reaches this tick and hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// The ledger of counts, bestiary encounters and build picks (ADR-BRDG-6).
    pub ledger: RunLedger,
}

/// The SHA-256 of the game content a run of `mode` uses with the bundled pack: the pack's hash.
/// `None` for a mode with no content.
pub fn content_sha256(mode: &str) -> Option<[u8; 32]> {
    content_sha256_with(mode, RunContent::bundled())
}

/// The SHA-256 of the game content a run of `mode` uses with `content`. Content read from a
/// pack has the pack's hash, the SHA-256 of the pack file's bytes (ADR-079 §5, ADR-080), whatever
/// the mode. Content with any file replaced one by one has a hash derived from the mode's file,
/// the attack presets file, and the bytes of the run stats, the rewards, the stage skills, the
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
        &content.rules().sha256(),
        content.constellations().sha256().as_ref(),
        &content.attack_presets().sha256(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn hash_content(
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
    rules_sha256: &[u8; 32],
    constellations_sha256: Option<&[u8; 32]>,
    attack_presets_sha256: &[u8; 32],
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
    // The simulation reads the rules file, so a replay's hash covers it (DIR1).
    part(&mut sha, b'l', b"rules", rules_sha256);
    // Only with the file, so that content without sets hashes as it did before it.
    if let Some(constellations) = constellations_sha256 {
        part(&mut sha, b'c', b"constellations", constellations);
    }
    part(&mut sha, b'a', b"attack_presets", attack_presets_sha256);
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
        serde_json::from_slice(bytes).map_err(|e| RunReplayError::Damaged {
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
            if kind == RETIRED_SPEND {
                return Err(bad("a spend command, which DS18 removed"));
            }
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
    checkpoints: Vec<u64>,
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
        let content = content_sha256_with(setup.mode.as_str(), content)
            .expect("a mode that starts has content");
        Ok(RunRecorder {
            setup: setup.clone(),
            content_sha256: to_hex(&content),
            engine,
            inputs: Vec::new(),
            commands: Vec::new(),
            checkpoints: Vec::new(),
        })
    }

    /// As `start`, with the content of `pack` in place of the bundled pack: the replay records
    /// the SHA-256 of the pack's bytes (ADR-079 §5, ADR-080).
    pub fn start_with_pack(setup: &RunSetup, pack: &Pack) -> Result<RunRecorder, RunReplayError> {
        RunRecorder::start_with(setup, pack.content())
    }

    /// Resumes the run a replay prefix records (ADR-SAVE-3): re-simulates it from tick 0 with
    /// `verify_run_with_pack`'s checks, and returns a live recorder at the cut, whose `replay()`
    /// after more steps is one replay from tick 0. The prefix may end on an open offer.
    pub fn resume_with_pack(
        prefix: &RunReplay,
        pack: &Pack,
    ) -> Result<RunRecorder, RunReplayError> {
        let size = prefix.to_bytes().len();
        if size > MAX_RUN_REPLAY_BYTES {
            return Err(RunReplayError::TooLarge {
                size,
                limit: MAX_RUN_REPLAY_BYTES,
            });
        }
        let run = resimulate(prefix, pack.content(), None)?;
        let engine = run.engine;
        if engine.outcome() != Outcome::Running {
            return Err(RunReplayError::PrefixEnded {
                tick: engine.tick(),
            });
        }
        if engine.state_hash() != run.final_hash {
            return Err(RunReplayError::PrefixHashDiffers {
                tick: engine.tick(),
            });
        }
        Ok(RunRecorder {
            setup: prefix.setup.clone(),
            content_sha256: prefix.content_sha256.clone(),
            engine,
            inputs: run.inputs,
            commands: run.commands,
            checkpoints: run.checkpoints,
        })
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
        if self.engine.tick() % CHECKPOINT_TICKS == 0 {
            self.checkpoints.push(self.engine.state_hash());
        }
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
            checkpoints: self.checkpoints.iter().map(|h| hash_to_hex(*h)).collect(),
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
    let run = resimulate(&replay, run_content, expected)?;
    let engine = run.engine;
    let actual = engine.state_hash();
    if actual != run.final_hash {
        return Err(RunReplayError::WrongFinalHash {
            expected: run.final_hash,
            actual,
        });
    }
    Ok(RunVerified {
        ticks: engine.tick(),
        final_hash: actual,
        outcome: engine.outcome(),
        commands: u32::try_from(run.commands.len()).unwrap_or(u32::MAX),
        stardust: engine.stardust(),
        ledger: engine.ledger().clone(),
    })
}

/// What re-simulating a replay leaves: the engine after the last tick and the commands after it,
/// the hash the replay claims for that point, and the unpacked inputs and commands.
struct Resimulated {
    engine: Engine,
    final_hash: u64,
    inputs: Vec<Input>,
    commands: Vec<TickCommand>,
    /// The state hashes after ticks 60, 120, ..., as this run reached them.
    checkpoints: Vec<u64>,
}

/// The replay's checkpoints as numbers: exactly `ticks / 60` hashes of 16 lowercase hex
/// digits. Works on bytes, so a multi-byte character is a bad chunk, never a panic.
fn parse_checkpoints(replay: &RunReplay) -> Result<Vec<u64>, RunReplayError> {
    let bytes = replay.checkpoints.as_bytes();
    let count = usize::try_from(replay.ticks / CHECKPOINT_TICKS).unwrap_or(usize::MAX);
    if bytes.len() / 16 != count || bytes.len() % 16 != 0 {
        return Err(RunReplayError::BadCheckpoints);
    }
    bytes
        .chunks(16)
        .map(|chunk| {
            std::str::from_utf8(chunk)
                .ok()
                .and_then(hash_from_hex)
                .ok_or(RunReplayError::BadCheckpoints)
        })
        .collect()
}

/// Every check of `verify_inner` but the comparison of the final hash, on a parsed replay.
fn resimulate(
    replay: &RunReplay,
    run_content: &RunContent,
    expected: Option<&[u64]>,
) -> Result<Resimulated, RunReplayError> {
    let stage = run_setup::stage_in(&replay.setup, run_content).map_err(RunReplayError::Setup)?;
    let content = content_sha256_with(replay.setup.mode.as_str(), run_content)
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
    let claimed = parse_checkpoints(replay)?;
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
    let mut reached = Vec::with_capacity(claimed.len());
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
            let actual = engine.state_hash();
            if expected[i] != actual {
                return Err(RunReplayError::Diverged {
                    tick: engine.tick(),
                    expected: expected[i],
                    actual,
                });
            }
        }
        if engine.tick() % CHECKPOINT_TICKS == 0 {
            let actual = engine.state_hash();
            let at = reached.len();
            if claimed[at] != actual {
                return Err(RunReplayError::CheckpointDiffers {
                    second: u32::try_from(at + 1).unwrap_or(u32::MAX),
                });
            }
            reached.push(actual);
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
    Ok(Resimulated {
        engine,
        final_hash,
        inputs,
        commands,
        checkpoints: reached,
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
        }
        *next += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::hash_content;

    const ATTACK_PRESETS_SHA: [u8; 32] = [14; 32];
    const STATS: [u8; 32] = [7; 32];
    const REWARDS: [u8; 32] = [9; 32];
    const SKILLS: [u8; 32] = [5; 32];
    const EXPERTISE: [u8; 32] = [3; 32];
    const MATCH_UPS: [u8; 32] = [1; 32];
    const RUNES: [u8; 32] = [2; 32];
    const PERKS: [u8; 32] = [4; 32];
    const DIFFICULTIES: [u8; 32] = [6; 32];
    const WEAPONS_SHA: [u8; 32] = [8; 32];
    const RULES_SHA: [u8; 32] = [12; 32];

    fn hash(mode: &str, file: &[u8], attack_presets: &[u8; 32]) -> [u8; 32] {
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
            &RULES_SHA,
            None,
            attack_presets,
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
            &RULES_SHA,
            None,
            &ATTACK_PRESETS_SHA,
        )
    }

    #[test]
    fn the_content_hash_covers_the_mode_the_content_files_and_the_attack_presets() {
        let base = hash("trial", b"{}", &ATTACK_PRESETS_SHA);
        assert_eq!(base, hash("trial", b"{}", &ATTACK_PRESETS_SHA));
        assert_ne!(
            base,
            hash("trial", b"{ }", &ATTACK_PRESETS_SHA),
            "the mode's file"
        );
        assert_ne!(
            base,
            hash("other", b"{}", &ATTACK_PRESETS_SHA),
            "the mode's ID"
        );
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
                &RULES_SHA,
                None,
                &ATTACK_PRESETS_SHA,
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
                &RULES_SHA,
                None,
                &ATTACK_PRESETS_SHA,
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
                &RULES_SHA,
                None,
                &ATTACK_PRESETS_SHA,
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
                &RULES_SHA,
                None,
                &ATTACK_PRESETS_SHA,
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
                &RULES_SHA,
                None,
                &ATTACK_PRESETS_SHA,
            ),
            "the weapons"
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
                &WEAPONS_SHA,
                &[13; 32],
                None,
                &ATTACK_PRESETS_SHA,
            ),
            "the rules"
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
                &RULES_SHA,
                sets,
                &ATTACK_PRESETS_SHA,
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
        assert_ne!(base, hash("trial", b"{}", &[15; 32]), "the attack presets");
    }
}
