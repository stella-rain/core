//! The run replay (ADR-REPL-5): a run of the launch game recorded as its setup, the SHA-256 of
//! the game content it ran, the tick count, the final state hash, one input per tick and one
//! stream of commands between ticks. Anyone can re-simulate it from the file and the content
//! files it used (the run stats, the rewards, the stage skills, the expertise and the
//! match-ups): the setup names a mode
//! whose content is bundled in `core`, so no stage file and no save data is needed.
//!
//! A `.replay` of a run is one JSON object, written at schema 2 (ADR-REPL-2 point 4,
//! ADR-REPL-5): `inputs` is the step log (see `step_log`; a PICKS record is reserved
//! and refused) and `commands` is empty. A schema 1 file is still read and verified, and its two streams are
//! described here. Its inputs are packed as a stage replay's are (see `replay`). Its commands
//! are packed in the order they applied, each as the tick (a LEB128
//! delta from the previous command's tick, from 0 for the first), the kind (one byte: 0
//! `choose`; 1 was `spend`, removed with DS18 and refused) and the index (LEB128). A command carries the number of ticks completed
//! when it applied, so several commands between the same two ticks have deltas of 0.
//!
//! Replays are untrusted (ADR-SVC-1): a size limit before parsing, strict parsing, no more ticks
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
//! A replay also holds `checkpoints`: the state hash after every 60th tick (ADR-REPL-6), each as
//! 16 lowercase hex digits, concatenated, taken right after the tick and before the commands
//! that follow it. The recorder writes them; re-simulation checks each as it passes it, so a run
//! that no longer reproduces fails naming the first second that differs (`CheckpointDiffers`),
//! and a field that is not `ticks / 60` hashes is `BadCheckpoints`. A run case of the corpus
//! keeps its expected hashes in them, in place of a hash per tick.
//!
//! The content hash is the SHA-256 of the bytes of the content pack the run used (ADR-REPL-5,
//! ADR-CONT-5), so any content update invalidates the replay rather than letting it drift, and a
//! kept pack is found by it. The attack presets and the weapons are files of the pack
//! (ADR-CONT-3, CS1b-1, WP1), so a change to a preset is a content release and a new hash.
//! What the rules do with the content is the `sim_version`'s, which the setup carries.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::base64;
use crate::content_hash::to_hex;
use crate::engine::{ActionError, ChoiceError, Engine, Outcome, PresenceError};
use crate::id::{MAX_PLAYERS, PlayerIdx};
use crate::input::{EquipAction, Input};
use crate::ledger::RunLedger;
use crate::pack::Pack;
use crate::replay::{
    block_of, hash_from_hex, hash_to_hex, input_of_block, pack_inputs, read_varint, unpack,
    write_varint,
};
use crate::run_content::RunContent;
use crate::run_setup::{self, RunSetup};
use crate::savepoint::{self, SavepointError};
use crate::step_log::{MAX_PRESENCE_CHANGES, Record, StepLog, StepRange, read_step_range};
use crate::validate;
use crate::version::{
    self, FIRST_SCHEMA_VERSION, FIRST_SIM_VERSION, ReplayLoadError, SCHEMA_VERSION, Versions,
};

/// The most commands a run replay holds (a tuning value; ADR-REPL-5 decides that a cap exists).
/// Version 1 gives at most 99 points in a run; the rest is room for the offers' choices, a few
/// per falling star taken. Measured (`tests/command_cap.rs`): the longest run, a `time_limit` run
/// on normal, hard or rainy played to its end (at most 90,000 ticks, through the whole overtime)
/// by a bot that cannot die, takes every offer or rerolls and banishes at each, answers a stat
/// offer at every level-up, with 20 extra rerolls, takes 425 commands at most (seeds 1 to 5).
/// That is under 500, so the cap stays at 1000, 2.35 times the measure; it is the guard against
/// a crafted file, which `tests/command_cap.rs` pins at twice the measure or more.
pub const MAX_COMMANDS: u32 = 1000;

/// A step log holds at most this many ACTIONS records (equipment actions, ADR-REPL-5 point 8),
/// counted apart from `MAX_COMMANDS`: a run with equipment holds no offers. Placeholder: a
/// tuning value, far above any hand-played count and well inside `MAX_RUN_REPLAY_BYTES`; it is
/// to be measured from a bot's counts.
pub const MAX_ACTION_STEPS: u32 = 4096;

/// A run replay file is at most this big (ADR-SVC-1). Placeholder (R2): 1 MiB, because a
/// 20-minute `time_limit` run with a bot is about 300 KB and the longest (43 minutes with
/// overtime and the Star-Hunter's time) about 510 KB. A run replay is shared as a file, never as
/// a share code (`share`).
pub const MAX_RUN_REPLAY_BYTES: usize = 1024 * 1024;

/// A run replay holds the state hash after every this many ticks (ADR-REPL-6): one second.
pub const CHECKPOINT_TICKS: u32 = 60;

/// What the hash derived from files replaced one by one starts with, so that it cannot be
/// mistaken for another SHA-256.
const CONTENT_TAG: &[u8] = b"stella-rain run content v0\0";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunReplay {
    /// What the run started from (ADR-REPL-5).
    pub setup: RunSetup,
    /// The SHA-256 of the game content the run used, lowercase hex: of the bytes of its content
    /// pack (`content_sha256_with`, ADR-CONT-5). Any other pack invalidates the replay.
    pub content_sha256: String,
    /// Ticks recorded, at most the mode's length (a `time_limit` run's: its length, overtime and the Star-Hunter's extra time). A run that ended ends on the tick it ended.
    pub ticks: u32,
    /// The FNV-1a 64 state hash after the last tick and the commands after it (ADR-SIM-3), 16
    /// lowercase hex digits.
    pub final_hash: String,
    /// The FNV-1a 64 state hash after every 60th tick (60, 120, 180, ...), each taken right
    /// after that tick and before the commands that follow it, each 16 lowercase hex digits,
    /// concatenated: `16 * (ticks / 60)` digits, empty for fewer than 60 ticks.
    pub checkpoints: String,
    /// The inputs: at schema 1 the per-tick input stream packed as a stage replay's is; at
    /// schema 2 the step log (`step_log`), then base64.
    pub inputs: String,
    /// Schema 1: the commands between ticks, packed as described in the module documentation.
    /// Schema 2: the empty string; a pick is a field of the step it was made on.
    pub commands: String,
}

/// The byte `spend` had in the packed stream (DS18 removed it): refused, never reused.
const RETIRED_SPEND: u8 = 1;

/// The kind of a command between ticks, and its byte in the packed stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// Option `index` of the open offer (ADR-REPL-5). Read from schema 1 files; no writer
    /// writes it into a file (schema 2 reserves the PICKS record).
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

/// A command of the run's stream (ADR-BRDG-3, ADR-REPL-5): one input for the next tick, or a command
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
    /// Not JSON, no versions, or not the run replay format at a version this game reads.
    Damaged {
        line: usize,
        column: usize,
    },
    /// The setup is not valid (ADR-SVC-1).
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
    /// More equipment action steps than `MAX_ACTION_STEPS`.
    TooManyActions {
        limit: u32,
    },
    /// The `final_hash` field is not 16 lowercase hex digits.
    BadHash,
    /// A choice carrying this tick was refused by the engine.
    ChoiceRefused {
        tick: u32,
        error: ChoiceError,
    },
    /// An equipment action on this tick was refused by the engine.
    ActionRefused {
        tick: u32,
        error: ActionError,
    },
    /// A join or leave on this tick was refused by the engine.
    PresenceRefused {
        tick: u32,
        error: PresenceError,
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
            RunReplayError::Older {
                schema_version,
                sim_version,
            } => write!(
                f,
                "the replay was recorded by an older version (schema {schema_version}, sim {sim_version}); this version cannot read it"
            ),
            RunReplayError::Newer {
                schema_version,
                sim_version,
            } => write!(
                f,
                "the replay needs a newer version (schema {schema_version}, sim {sim_version})"
            ),
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
            RunReplayError::TooManyActions { limit } => {
                write!(f, "the replay has more than {limit} equipment action steps")
            }
            RunReplayError::BadHash => f.write_str("the replay's final hash is not 16 hex digits"),
            RunReplayError::ChoiceRefused { tick, error } => match error {
                ChoiceError::NoPlayer { player } => write!(
                    f,
                    "a choice on tick {tick} names player {} outside the party",
                    player.get()
                ),
                ChoiceError::NoOffer => write!(f, "a choice on tick {tick} has no offer open"),
                ChoiceError::OutOfRange { index, options } => write!(
                    f,
                    "a choice on tick {tick} is option {index} of an offer of {options}"
                ),
                ChoiceError::Unsupported { sim_version } => write!(
                    f,
                    "a choice on tick {tick} is in a run of sim_version {sim_version}, \
                     which this game does not run"
                ),
            },
            RunReplayError::ActionRefused { tick, error } => match error {
                ActionError::NoPlayer { player } => write!(
                    f,
                    "an equipment action on tick {tick} names player {} outside the party",
                    player.get()
                ),
                ActionError::NotBuilt => {
                    write!(f, "an equipment action on tick {tick} is not built yet")
                }
                ActionError::NoItem { item } => write!(
                    f,
                    "an equipment action on tick {tick} names inventory item {item}, \
                     which is not in the inventory"
                ),
                ActionError::NoSlot { slot } => write!(
                    f,
                    "an equipment action on tick {tick} names slot {slot}, \
                     which is not an equipment slot"
                ),
                ActionError::SlotFilled { slot } => write!(
                    f,
                    "an equipment action on tick {tick} equips into slot {slot}, which is filled"
                ),
                ActionError::SlotEmpty { slot } => write!(
                    f,
                    "an equipment action on tick {tick} replaces in slot {slot}, which is empty"
                ),
                ActionError::WrongSlot { slot } => write!(
                    f,
                    "an equipment action on tick {tick} puts an item into slot {slot}, \
                     which does not take it"
                ),
                ActionError::NotEquippable { item } => write!(
                    f,
                    "an equipment action on tick {tick} equips inventory item {item}, \
                     which cannot be equipped"
                ),
                ActionError::OptionHeld { slot } => write!(
                    f,
                    "an equipment action on tick {tick} equips a star whose option slot {slot} \
                     already holds"
                ),
                ActionError::RuneHeld { slot } => write!(
                    f,
                    "an equipment action on tick {tick} equips a rune that slot {slot} \
                     already holds"
                ),
                ActionError::NoStar { star } => write!(
                    f,
                    "an equipment action on tick {tick} crafts star {star}, \
                     which is not in the star pool"
                ),
                ActionError::NotCraftable { star } => write!(
                    f,
                    "an equipment action on tick {tick} crafts star {star}, \
                     which cannot be crafted"
                ),
                ActionError::AlreadyHeld { star } => write!(
                    f,
                    "an equipment action on tick {tick} crafts star {star}, \
                     which the player already holds"
                ),
                ActionError::NotEnoughStarlight { need, have } => write!(
                    f,
                    "an equipment action on tick {tick} crafts a star for {need} Starlight, \
                     but the player has {have}"
                ),
                ActionError::InventoryFull => write!(
                    f,
                    "an equipment action on tick {tick} crafts a star into a full inventory"
                ),
                ActionError::Downed => write!(
                    f,
                    "an equipment action on tick {tick} is made by a downed player, whose \
                     equipment window is locked until a teammate revives it"
                ),
                ActionError::Out => write!(
                    f,
                    "an equipment action on tick {tick} is made by a player who is out of the run"
                ),
                ActionError::NoTreePoint => write!(
                    f,
                    "an equipment action on tick {tick} takes a tree pick, \
                     but no tree point is held"
                ),
                ActionError::NoTreeOption { option, options } => write!(
                    f,
                    "an equipment action on tick {tick} takes tree option {option}, \
                     but the pick has {options} options"
                ),
            },
            RunReplayError::PresenceRefused { tick, error } => match error {
                PresenceError::Occupied { slot } => write!(
                    f,
                    "tick {tick}: slot {} joins but is already in the party",
                    slot.get()
                ),
                PresenceError::Absent { slot } => write!(
                    f,
                    "tick {tick}: slot {} leaves but is not in the party",
                    slot.get()
                ),
                PresenceError::FirstSlot => write!(f, "tick {tick}: slot 0 never leaves"),
                PresenceError::Ended => write!(f, "tick {tick}: the run is over"),
                PresenceError::SoloRules => write!(
                    f,
                    "tick {tick}: a run of sim_version 1 has one player for good"
                ),
                PresenceError::Exhausted { slot } => {
                    write!(f, "tick {tick}: slot {} has left 255 times", slot.get())
                }
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
    /// The Stardust the run earned, times 10, at most `limits::MAX_RUN_STARDUST`: the
    /// app pays it after the run (ADR-REPL-5).
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
/// pack has the pack's hash, the SHA-256 of the pack file's bytes (ADR-REPL-5, ADR-CONT-5), whatever
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

    /// Parses a run replay file: the size limit first, then the versions alone (the setup's),
    /// then the strict parse (ADR-REPL-1, ADR-SVC-1). A file for a newer version is `Newer` even if
    /// it has fields this version rejects; a file that is not JSON, has no versions, or fails the
    /// strict parse at a version this game reads is `Damaged`, never `Older`.
    pub fn from_bytes(bytes: &[u8]) -> Result<RunReplay, RunReplayError> {
        /// The versions alone; the rest of the file is ignored.
        #[derive(serde::Deserialize)]
        struct Header {
            setup: Versions,
        }
        version::read_replay(
            bytes,
            MAX_RUN_REPLAY_BYTES,
            FIRST_SCHEMA_VERSION,
            FIRST_SIM_VERSION,
            Versions::NEWEST_READ,
            &[],
            |bytes| serde_json::from_slice::<Header>(bytes).map(|h| h.setup),
        )
        .map_err(|e| match e {
            ReplayLoadError::TooLarge { size, limit } => RunReplayError::TooLarge { size, limit },
            ReplayLoadError::Older(v) => RunReplayError::Older {
                schema_version: v.schema_version,
                sim_version: v.sim_version,
            },
            ReplayLoadError::Newer(v) => RunReplayError::Newer {
                schema_version: v.schema_version,
                sim_version: v.sim_version,
            },
            ReplayLoadError::Damaged { line, column } => RunReplayError::Damaged { line, column },
        })
    }

    /// The solo inputs of the replay, one per tick: exactly `ticks` of them, and `ticks` at
    /// most `max_ticks`. A schema 1 replay's `inputs` are this stream; a schema 2 replay's are a
    /// step log, whose solo view this is (a party other than one is refused).
    pub fn unpack_inputs(&self, max_ticks: u32) -> Result<Vec<Input>, RunReplayError> {
        if self.setup.schema_version >= 2 {
            return Ok(self.step_log(max_ticks)?.to_solo()?.0);
        }
        self.unpack_inputs_v1(max_ticks)
    }

    fn unpack_inputs_v1(&self, max_ticks: u32) -> Result<Vec<Input>, RunReplayError> {
        if self.ticks > max_ticks {
            return Err(RunReplayError::TooManyTicks {
                ticks: self.ticks,
                limit: max_ticks,
            });
        }
        let bytes = base64::decode(&self.inputs)
            .map_err(|_| RunReplayError::BadInputs("not valid base64"))?;
        unpack(&bytes, self.ticks, false).map_err(RunReplayError::BadInputs)
    }

    /// The commands between ticks, in order: at most `MAX_COMMANDS`, of known kinds, none after
    /// tick `ticks`. A schema 1 replay's `commands` are this list; a schema 2 log holds no picks, so
    /// it is empty or refused (a PICKS record is reserved, an ACTIONS step has no solo view).
    pub fn unpack_commands(&self) -> Result<Vec<TickCommand>, RunReplayError> {
        if self.setup.schema_version >= 2 {
            return self.step_log(u32::MAX)?.solo_commands();
        }
        self.unpack_commands_v1()
    }

    fn unpack_commands_v1(&self) -> Result<Vec<TickCommand>, RunReplayError> {
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

    /// The steps of world ticks `a..b` of the run (ADR-REPL-5), by the rule of
    /// `read_step_range`, whichever schema the replay is. At schema 2 the log is validated and
    /// cut without expanding its runs; a schema 1 replay is migrated first, as every schema 1
    /// read is (the cost of its ticks).
    pub fn step_range(&self, a: u32, b: u32, max_ticks: u32) -> Result<StepRange, RunReplayError> {
        if self.setup.schema_version >= 2 {
            if !self.commands.is_empty() {
                return Err(RunReplayError::BadCommands("schema 2 has no command list"));
            }
            let bytes = base64::decode(&self.inputs)
                .map_err(|_| RunReplayError::BadInputs("not valid base64"))?;
            return read_step_range(&bytes, self.ticks, max_ticks, a, b);
        }
        self.step_log(max_ticks)?.range(a, b)
    }

    /// The party the replay starts with, read from the first byte of its step log alone (the
    /// `present` byte, ADR-REPL-5): 1 for a schema 1 replay. `None` for a byte that is not a
    /// party from slot 0 (a hole, a bit past slot 2, no byte, bad base64).
    #[cfg(test)]
    pub(crate) fn party_size(&self) -> Option<u8> {
        if self.setup.schema_version < 2 {
            return Some(1);
        }
        let head: String = self.inputs.chars().take(4).collect();
        let present = *base64::decode(&head).ok()?.first()?;
        StepLog::party_of(present)
    }

    /// The party after the last join or leave of the replay's step log: `0b001` for a schema 1
    /// replay, the first byte alone when the log holds no change; `None` for bytes the step log
    /// scanner refuses. Reads the bytes once, with the full checks, and builds no log.
    pub(crate) fn presence_at_end(&self) -> Option<u8> {
        if self.setup.schema_version < 2 {
            return Some(0b001);
        }
        let raw = base64::decode(&self.inputs).ok()?;
        StepLog::presence_at_end(&raw, self.ticks)
    }

    /// The run's steps as one log, whichever schema the replay is: a schema 1 replay's inputs
    /// and commands are migrated in memory, a schema 2 replay's `inputs` are decoded and its
    /// `commands` must be empty (schema 2 has no picks).
    pub(crate) fn step_log(&self, max_ticks: u32) -> Result<StepLog, RunReplayError> {
        if self.setup.schema_version >= 2 {
            if !self.commands.is_empty() {
                return Err(RunReplayError::BadCommands("schema 2 has no command list"));
            }
            let bytes = base64::decode(&self.inputs)
                .map_err(|_| RunReplayError::BadInputs("not valid base64"))?;
            return StepLog::decode(&bytes, self.ticks, max_ticks);
        }
        let inputs = self.unpack_inputs_v1(max_ticks)?;
        let commands = self.unpack_commands_v1()?;
        Ok(StepLog::from_schema1(&inputs, &commands))
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

/// The setup as the recorder writes it, a function of its `sim_version` (ADR-REPL-5): a run of
/// the frozen rules is written in the schema 1 form it was frozen in (`inputs` and `commands`),
/// every other at `SCHEMA_VERSION`, whatever schema the setup or the prefix it comes from was
/// read at.
fn written(setup: &RunSetup) -> RunSetup {
    RunSetup {
        schema_version: if setup.sim_version == 1 {
            1
        } else {
            SCHEMA_VERSION
        },
        ..setup.clone()
    }
}

/// A run being played and recorded: every command goes through it to the engine, and only the
/// commands the engine takes are recorded, so the replay always verifies.
#[derive(Clone, Debug)]
pub struct RunRecorder {
    setup: RunSetup,
    content_sha256: String,
    engine: Engine,
    /// Bit `i` set for player slot `i`: the party, from slot 0 (the log's `present` byte).
    present: u8,
    /// The run as a step log (ADR-REPL-5): runs of ticks with one input per player, and the
    /// steps of picks between them.
    records: Vec<Record>,
    /// The number of `Record::Actions` in `records`: what `MAX_ACTION_STEPS` counts.
    actions_made: u32,
    checkpoints: Vec<u64>,
}

impl RunRecorder {
    /// Starts a run from `setup` (ADR-REPL-5) with the bundled content files.
    pub fn start(setup: &RunSetup) -> Result<RunRecorder, RunReplayError> {
        RunRecorder::start_with(setup, RunContent::bundled())
    }

    /// As `start`, with `content` in place of the bundled files (ADR-CONT-1).
    pub fn start_with(
        setup: &RunSetup,
        content: &RunContent,
    ) -> Result<RunRecorder, RunReplayError> {
        RunRecorder::start_party_with(setup, content, 1)
    }

    /// As `start`, for a party of `party` players (1 to 3): `step` takes one input per slot
    /// and `choose` names the player. Slots 1 and 2 take the run's loadout until the per-player
    /// setup.
    pub fn start_party(setup: &RunSetup, party: u8) -> Result<RunRecorder, RunReplayError> {
        RunRecorder::start_party_with(setup, RunContent::bundled(), party)
    }

    /// As `start_party`, with `content` in place of the bundled files. A party outside 1 to 3
    /// is `Setup`.
    pub fn start_party_with(
        setup: &RunSetup,
        content: &RunContent,
        party: u8,
    ) -> Result<RunRecorder, RunReplayError> {
        let engine =
            Engine::start_party_with(setup, content, party).map_err(RunReplayError::Setup)?;
        let content = content_sha256_with(setup.mode.as_str(), content)
            .expect("a mode that starts has content");
        Ok(RunRecorder {
            setup: written(setup),
            content_sha256: to_hex(&content),
            engine,
            present: (1u8 << party) - 1,
            records: Vec::new(),
            actions_made: 0,
            checkpoints: Vec::new(),
        })
    }

    /// As `start_party`, with the content of `pack` in place of the bundled pack.
    pub fn start_party_with_pack(
        setup: &RunSetup,
        pack: &Pack,
        party: u8,
    ) -> Result<RunRecorder, RunReplayError> {
        RunRecorder::start_party_with(setup, pack.content(), party)
    }

    /// As `start`, with the content of `pack` in place of the bundled pack: the replay records
    /// the SHA-256 of the pack's bytes (ADR-REPL-5, ADR-CONT-5).
    pub fn start_with_pack(setup: &RunSetup, pack: &Pack) -> Result<RunRecorder, RunReplayError> {
        RunRecorder::start_with(setup, pack.content())
    }

    /// Resumes the run a replay prefix records (ADR-SAVE-4): re-simulates it from tick 0 with
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
            setup: written(&prefix.setup),
            content_sha256: prefix.content_sha256.clone(),
            engine,
            present: run.log.present,
            actions_made: run.log.actions_made(),
            records: run.log.records,
            checkpoints: run.checkpoints,
        })
    }

    /// The run as a savepoint file (ADR-SAVE-4): the engine's state with a header that binds it
    /// to this run's replay so far. Infallible; a run that has ended saves too, and is refused
    /// on restore.
    pub fn save(&self) -> Vec<u8> {
        savepoint::save(&self.engine, &self.replay())
    }

    /// Resumes the run a savepoint holds, without re-simulating `prefix`, the replay up to the
    /// savepoint (ADR-SAVE-4). The savepoint must match `SAVEPOINT_FORMAT`, `SIM_VERSION` and
    /// `pack`, and `prefix` must be the very run it was taken from; a later `replay()` is one
    /// replay from tick 0. Any refusal leaves nothing behind: the caller runs
    /// `resume_with_pack(prefix, pack)`.
    pub fn resume_from_savepoint(
        bytes: &[u8],
        prefix: &RunReplay,
        pack: &Pack,
    ) -> Result<RunRecorder, SavepointError> {
        let (engine, history) = savepoint::restore(bytes, prefix, pack)?;
        Ok(RunRecorder {
            setup: written(&prefix.setup),
            content_sha256: prefix.content_sha256.clone(),
            engine,
            present: history.log.present,
            actions_made: history.log.actions_made(),
            records: history.log.records,
            checkpoints: history.claimed,
        })
    }

    /// The run, for its snapshot and events.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Runs the next tick with `inputs`, one entry per slot, present or not, in slot order; an
    /// absent slot's entry is ignored and the step log keeps one block per present slot. Refused,
    /// recording nothing, after the run ended or while an offer is open (ADR-REPL-5: a choice
    /// answers it first).
    pub fn step(&mut self, inputs: &[Input; MAX_PLAYERS as usize]) -> Result<(), RunReplayError> {
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
        if self.setup.schema_version == 1 && inputs.iter().any(|input| input.interact) {
            // Schema 1 packs no `interact`: the file could not reproduce the run.
            return Err(RunReplayError::BadInputs("a version 1 run has no interact"));
        }
        let mut cleared = *inputs;
        // The actions the engine accepts are recorded as one ACTIONS step, the rest are ignored
        // as a refused pick is; the tick runs on inputs without actions, so the log holds none
        // in a TICKS block.
        if cleared.iter().any(|input| input.action.is_some()) {
            let mask = self.engine.present();
            let mut actions = [None; MAX_PLAYERS as usize];
            for (slot, input) in cleared.iter_mut().enumerate() {
                actions[slot] = PlayerIdx::new(u8::try_from(slot).unwrap_or(u8::MAX))
                    .zip(input.action.take())
                    .filter(|(player, action)| {
                        mask >> player.get() & 1 == 1
                            && self.engine.check_action(*player, *action).is_ok()
                    })
                    .map(|(_, action)| action);
            }
            if actions.iter().any(Option::is_some) {
                self.actions(&actions)?;
            }
        }
        let inputs = &cleared;
        // The inputs of the present slots, in slot order, in a fixed array: no allocation per tick.
        let mask = self.engine.present();
        let mut kept = [Input::default(); MAX_PLAYERS as usize];
        let mut count = 0;
        for (slot, input) in inputs.iter().enumerate() {
            if mask >> slot & 1 == 1 {
                kept[count] = *input;
                count += 1;
            }
        }
        self.engine.step(inputs);
        let present = &kept[..count];
        // The log holds each input as its block bytes read it back (a stick axis of -128 is
        // -127, a pick is never in the stream), so equal blocks are one run.
        let held = |input: &Input| {
            input_of_block(block_of(input), true).expect("a block of an input reads back")
        };
        match self.records.last_mut() {
            Some(Record::Ticks { run, inputs: last })
                if last.iter().zip(present).all(|(a, b)| *a == held(b)) =>
            {
                *run += 1;
            }
            _ => self.records.push(Record::Ticks {
                run: 1,
                inputs: present.iter().map(held).collect(),
            }),
        }
        if self.engine.tick() % CHECKPOINT_TICKS == 0 {
            self.checkpoints.push(self.engine.state_hash());
        }
        Ok(())
    }

    /// Test seam (ADR-SIM-7's `bench` gate): answers a frozen run's offer with one recorded PICKS
    /// step, never ticking.
    ///
    /// Every pick is checked first (`ChoiceRefused { tick, error }`), so a refusal changes and
    /// records nothing; a step with no pick among the present slots is `BadInputs`.
    #[cfg(feature = "bench")]
    pub fn v1_pick_step(
        &mut self,
        picks: &[Option<u32>; MAX_PLAYERS as usize],
    ) -> Result<(), RunReplayError> {
        let dense = dense(self.engine.present(), picks);
        if dense.iter().all(Option::is_none) {
            return Err(RunReplayError::BadInputs("a step of picks holds a pick"));
        }
        apply_picks(&mut self.engine, &dense)?;
        self.records.push(Record::Picks { picks: dense });
        Ok(())
    }

    /// One step that does not advance the world: the equipment action of each slot (`None` for
    /// none), at least one among the present slots (ADR-SIM-7, ADR-REPL-5 point 8); an action at
    /// an absent slot is ignored. Refused, recording nothing, after the run ended or when the replay already holds
    /// `MAX_ACTION_STEPS` steps of actions or the engine refuses an action (`check_action`).
    pub fn actions(
        &mut self,
        actions: &[Option<EquipAction>; MAX_PLAYERS as usize],
    ) -> Result<(), RunReplayError> {
        let dense = dense(self.engine.present(), actions);
        if dense.iter().all(Option::is_none) {
            return Err(RunReplayError::BadInputs(
                "a step of actions holds an action",
            ));
        }
        if self.engine.outcome() != Outcome::Running {
            return Err(RunReplayError::InputsAfterEnd {
                ended_at: self.engine.tick(),
            });
        }
        if self.actions_made >= MAX_ACTION_STEPS {
            return Err(RunReplayError::TooManyActions {
                limit: MAX_ACTION_STEPS,
            });
        }
        check_actions(&self.engine, &dense)?;
        self.engine.action_step(actions);
        self.records.push(Record::Actions { actions: dense });
        self.actions_made += 1;
        Ok(())
    }

    /// Slot `slot` joins the party before the next step (`Engine::join`): a JOIN record at the
    /// tick the run is on. Refused, recording nothing, after the run ended, past
    /// `MAX_PRESENCE_CHANGES`, or when the engine refuses.
    pub fn join(&mut self, slot: PlayerIdx) -> Result<(), RunReplayError> {
        self.change(slot, true)
    }

    /// Slot `slot` leaves the party before the next step (`Engine::leave`): a LEAVE record.
    /// Refused as `join` is.
    pub fn leave(&mut self, slot: PlayerIdx) -> Result<(), RunReplayError> {
        self.change(slot, false)
    }

    fn change(&mut self, slot: PlayerIdx, join: bool) -> Result<(), RunReplayError> {
        let tick = self.engine.tick();
        if self.engine.outcome() != Outcome::Running {
            return Err(RunReplayError::InputsAfterEnd { ended_at: tick });
        }
        let made = self
            .records
            .iter()
            .filter(|r| matches!(r, Record::Join { .. } | Record::Leave { .. }))
            .count();
        if made >= MAX_PRESENCE_CHANGES as usize {
            return Err(RunReplayError::BadInputs(
                "more joins and leaves than the cap",
            ));
        }
        let done = if join {
            self.engine.join(slot)
        } else {
            self.engine.leave(slot)
        };
        done.map_err(|error| RunReplayError::PresenceRefused { tick, error })?;
        let slot = u8::try_from(slot.get()).unwrap_or(u8::MAX);
        self.records.push(if join {
            Record::Join { slot }
        } else {
            Record::Leave { slot }
        });
        Ok(())
    }

    /// The `inputs` and `commands` fields for `log`, in the schema the recorder writes: at
    /// schema 2 the step log, then base64 (`commands` empty); at schema 1 the solo streams.
    fn streams(&self, log: &StepLog) -> (String, String) {
        if self.setup.schema_version == 1 {
            let (inputs, commands) = log.to_solo().expect("a version 1 run is solo");
            (pack_inputs(&inputs), pack_commands(&commands))
        } else {
            (
                base64::encode(&log.encode().expect("a schema 2 log holds no picks")),
                String::new(),
            )
        }
    }

    /// The replay of the run so far: complete once the run has ended, a bug report before.
    pub fn replay(&self) -> RunReplay {
        let log = StepLog {
            present: self.present,
            records: self.records.clone(),
        };
        let (inputs, commands) = self.streams(&log);
        RunReplay {
            setup: self.setup.clone(),
            content_sha256: self.content_sha256.clone(),
            ticks: self.engine.tick(),
            final_hash: hash_to_hex(self.engine.state_hash()),
            checkpoints: self.checkpoints.iter().map(|h| hash_to_hex(*h)).collect(),
            inputs,
            commands,
        }
    }

    /// The replay of the first `ticks` ticks of the run so far, built from what the recorder has
    /// recorded alone: it reads nothing from the engine, so it is safe to call after the engine
    /// panicked. `final_hash` is the state hash after tick `ticks` and the picks that carry it
    /// (what `Engine::state_hash` gave at that point; the caller keeps it, because this method
    /// cannot ask the engine). The replay holds the first `ticks` ticks of inputs, the first
    /// `ticks / CHECKPOINT_TICKS` checkpoints and the steps of picks up to and including those
    /// that apply after tick `ticks`, so it verifies when `ticks` and `final_hash` are the run's
    /// at one point the run reached. What was recorded past `ticks` is left out. A `ticks` above
    /// the number of ticks recorded does not panic; the replay then holds everything it has and
    /// does not verify. After the run ended and with the engine's own tick and hash, this is
    /// exactly `replay()`.
    pub fn replay_at(&self, ticks: u32, final_hash: u64) -> RunReplay {
        let checkpoints = usize::try_from(ticks / CHECKPOINT_TICKS)
            .map_or(self.checkpoints.len(), |c| c.min(self.checkpoints.len()));
        let log = StepLog {
            present: self.present,
            records: self.records.clone(),
        }
        .prefix(ticks);
        let (inputs, commands) = self.streams(&log);
        RunReplay {
            setup: self.setup.clone(),
            content_sha256: self.content_sha256.clone(),
            ticks,
            final_hash: hash_to_hex(final_hash),
            checkpoints: self.checkpoints[..checkpoints]
                .iter()
                .map(|h| hash_to_hex(*h))
                .collect(),
            inputs,
            commands,
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
            Command::Step(input) => {
                recorder.step(&[input, Default::default(), Default::default()])?
            }
            Command::Choose(_) => {
                return Err(RunReplayError::BadCommands("a choice cannot be recorded"));
            }
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
/// command and the final hash (ADR-REPL-5). The whole file is untrusted.
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
/// (ADR-SIM-3).
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
        commands: run.picks,
        stardust: engine.stardust(),
        ledger: engine.ledger().clone(),
    })
}

/// What re-simulating a replay leaves: the engine after the last tick and the picks after it,
/// the hash the replay claims for that point, the step log and the number of picks in it.
struct Resimulated {
    engine: Engine,
    final_hash: u64,
    log: StepLog,
    picks: u32,
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

/// What a replay holds once parsed and checked against its content, before any tick runs.
pub(crate) struct History {
    pub(crate) final_hash: u64,
    pub(crate) log: StepLog,
    /// The checkpoints the replay claims, as numbers.
    pub(crate) claimed: Vec<u64>,
}

/// The parsing half of re-simulation: the setup, the content hash, the final hash, the inputs,
/// the commands and the checkpoints, each checked, in that order. No tick runs.
pub(crate) fn unpack_history(
    replay: &RunReplay,
    run_content: &RunContent,
) -> Result<History, RunReplayError> {
    let stage = run_setup::stage_in(&replay.setup, run_content).map_err(RunReplayError::Setup)?;
    let content = content_sha256_with(replay.setup.mode.as_str(), run_content)
        .ok_or(RunReplayError::ContentChanged)?;
    if replay.content_sha256 != to_hex(&content) {
        return Err(RunReplayError::ContentChanged);
    }
    let final_hash = hash_from_hex(&replay.final_hash).ok_or(RunReplayError::BadHash)?;
    let log = replay.step_log(run_setup::max_replay_ticks(
        &replay.setup,
        &stage,
        run_content.time_limit(),
    ))?;
    let claimed = parse_checkpoints(replay)?;
    Ok(History {
        final_hash,
        log,
        claimed,
    })
}

/// Every check of `verify_inner` but the comparison of the final hash, on a parsed replay.
fn resimulate(
    replay: &RunReplay,
    run_content: &RunContent,
    expected: Option<&[u64]>,
) -> Result<Resimulated, RunReplayError> {
    let History {
        final_hash,
        log,
        claimed,
    } = unpack_history(replay, run_content)?;
    if let Some(expected) = expected {
        if expected.len() != replay.ticks as usize {
            return Err(RunReplayError::HashCount {
                expected: replay.ticks,
                given: expected.len(),
            });
        }
    }

    // The slots present must be a party from slot 0 (a hole is for joining and leaving).
    let party = StepLog::party_of(log.present).ok_or(RunReplayError::BadInputs(
        "the party does not match the run",
    ))?;
    let mut engine = Engine::start_party_with(&replay.setup, run_content, party)
        .map_err(RunReplayError::Setup)?;
    let mut reached = Vec::with_capacity(claimed.len());
    let mut picks = 0u32;
    for record in &log.records {
        match record {
            Record::Ticks { run, inputs } => {
                let inputs = slots(engine.present(), inputs);
                for _ in 0..*run {
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
                    engine.step(&inputs);
                    if let Some(expected) = expected {
                        let actual = engine.state_hash();
                        let at = engine.tick() as usize - 1;
                        if expected[at] != actual {
                            return Err(RunReplayError::Diverged {
                                tick: engine.tick(),
                                expected: expected[at],
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
            }
            Record::Join { slot } | Record::Leave { slot } => {
                if engine.outcome() != Outcome::Running {
                    return Err(RunReplayError::InputsAfterEnd {
                        ended_at: engine.tick(),
                    });
                }
                let tick = engine.tick();
                let slot = PlayerIdx::new(*slot).ok_or(RunReplayError::BadInputs(
                    "the party does not match the run",
                ))?;
                let done = if matches!(record, Record::Join { .. }) {
                    engine.join(slot)
                } else {
                    engine.leave(slot)
                };
                done.map_err(|error| RunReplayError::PresenceRefused { tick, error })?;
            }
            Record::Actions { actions } => {
                // A step that holds no action is a damaged log; every action is checked against
                // the engine first, then they are made together, in slot order.
                if actions.iter().all(Option::is_none) {
                    return Err(RunReplayError::BadInputs("a record has no action"));
                }
                check_actions(&engine, actions)?;
                engine.action_step(&slots(engine.present(), actions));
            }
            Record::Picks { picks: chosen } => {
                apply_picks(&mut engine, chosen)?;
                picks += u32::try_from(chosen.iter().flatten().count()).unwrap_or(u32::MAX);
            }
        }
    }
    Ok(Resimulated {
        engine,
        final_hash,
        log,
        picks,
        checkpoints: reached,
    })
}

/// One step that does not advance the world: every pick is checked against the engine first,
/// then taken together (`Engine::v1_pick_step`).
fn apply_picks(engine: &mut Engine, chosen: &[Option<u32>]) -> Result<(), RunReplayError> {
    let tick = engine.tick();
    engine
        .v1_pick_step(&slots(engine.present(), chosen))
        .map_err(|error| RunReplayError::ChoiceRefused { tick, error })
}

/// The dense entries of a record, one per slot of `mask` in slot order, placed at their slots
/// and padded with the default: what `Engine::step` takes.
fn slots<T: Copy + Default>(mask: u8, dense: &[T]) -> [T; MAX_PLAYERS as usize] {
    let mut all = [T::default(); MAX_PLAYERS as usize];
    let mut from = dense.iter();
    for (slot, to) in all.iter_mut().enumerate() {
        if mask >> slot & 1 == 1 {
            if let Some(entry) = from.next() {
                *to = *entry;
            }
        }
    }
    all
}

/// Asks `engine` about each action of one dense ACTIONS step, in slot order: the first refusal
/// is `ActionRefused` at the engine's tick. Nothing is applied.
fn check_actions(engine: &Engine, dense: &[Option<EquipAction>]) -> Result<(), RunReplayError> {
    let slots = (0..MAX_PLAYERS)
        .filter_map(PlayerIdx::new)
        .filter(|player| engine.present() >> player.get() & 1 == 1);
    for (player, action) in slots.zip(dense) {
        if let Some(action) = action {
            engine.check_action(player, *action).map_err(|error| {
                RunReplayError::ActionRefused {
                    tick: engine.tick(),
                    error,
                }
            })?;
        }
    }
    Ok(())
}

/// The entries of the slots of `mask` out of one entry per slot: the dense record.
fn dense<T: Copy>(mask: u8, all: &[T; MAX_PLAYERS as usize]) -> Vec<T> {
    all.iter()
        .enumerate()
        .filter(|(slot, _)| mask >> slot & 1 == 1)
        .map(|(_, entry)| *entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ActionError, RunReplayError};
    use super::{RunRecorder, hash_content, verify_run};
    use crate::engine::Engine;
    use crate::input::EquipAction;
    use crate::input::Input;
    use crate::run_setup::RunSetup;

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

    #[test]
    fn replay_at_reads_nothing_from_the_engine() {
        let setup = RunSetup {
            seed: 20_261_010,
            ..RunSetup::default()
        };
        let mut r = RunRecorder::start(&setup).unwrap();
        for _ in 0..130 {
            r.step(&[Input::default(), Default::default(), Default::default()])
                .unwrap();
        }
        let hash = r.engine().state_hash();
        let bytes_before = r.replay().to_bytes();

        // A recorder whose engine is unrelated to the run: a fresh one at tick 0.
        let mut bad = r.clone();
        bad.engine = Engine::start(&setup).unwrap();
        assert_eq!(bad.replay().ticks, 0, "replay() reads the engine");

        let replay = bad.replay_at(130, hash);
        assert!(
            replay.to_bytes() == bytes_before,
            "the replay is the one the good recorder gave"
        );
        let verified = verify_run(&replay.to_bytes()).unwrap();
        assert_eq!(verified.ticks, 130);
        assert_eq!(verified.final_hash, hash);
    }

    /// T23: a replay whose ACTIONS step the engine refuses is `ActionRefused` at that tick, with
    /// the refusal's own message; a step with no action is still a damaged log.
    #[test]
    fn a_replay_with_a_refused_equip_names_the_refusal() {
        use crate::run_replay::Record;
        let setup = RunSetup {
            ..RunSetup::default()
        };
        let mut rec = RunRecorder::start(&setup).unwrap();
        rec.step(&[Input::default(); 3]).unwrap();
        rec.records.push(Record::Actions {
            actions: vec![Some(EquipAction::Equip { item: 0, slot: 0 })],
        });
        let refused = RunReplayError::ActionRefused {
            tick: 1,
            error: ActionError::NoItem { item: 0 },
        };
        assert_eq!(
            refused.to_string(),
            "an equipment action on tick 1 names inventory item 0, which is not in the inventory"
        );
        assert_eq!(verify_run(&rec.replay().to_bytes()).unwrap_err(), refused);
        rec.records.pop();
        rec.records.push(Record::Actions {
            actions: vec![None],
        });
        assert_eq!(
            verify_run(&rec.replay().to_bytes()).unwrap_err(),
            RunReplayError::BadInputs("a record has no action")
        );
    }

    /// L6: the two lock refusals read as the plan names them.
    #[test]
    fn the_lock_refusals_read_well() {
        let say = |error| RunReplayError::ActionRefused { tick: 7, error }.to_string();
        assert_eq!(
            say(ActionError::Downed),
            "an equipment action on tick 7 is made by a downed player, whose equipment window \
             is locked until a teammate revives it"
        );
        assert_eq!(
            say(ActionError::Out),
            "an equipment action on tick 7 is made by a player who is out of the run"
        );
    }

    /// L7: the recorder's `step` drops a downed member's action and goes on; `actions` refuses it.
    #[test]
    fn the_recorder_drops_a_downed_action_and_refuses_it_direct() {
        use crate::engine::Run;
        let setup = RunSetup {
            ..RunSetup::default()
        };
        let mut rec = RunRecorder::start_party(&setup, 2).unwrap();
        if let Run::V2 { state, .. } = rec.engine.run_mut() {
            let one = crate::id::PlayerIdx::new(1).unwrap();
            state.players[one].hp = 0;
        }
        let act = EquipAction::Equip { item: 0, slot: 5 };
        let acting = Input {
            action: Some(act),
            ..Input::default()
        };
        rec.step(&[Input::default(), acting, Input::default()])
            .unwrap();
        assert_eq!(rec.engine().tick(), 1, "the run went on");
        assert!(
            rec.records
                .iter()
                .all(|r| !matches!(r, super::Record::Actions { .. })),
            "no ACTIONS step was recorded"
        );
        assert_eq!(
            rec.actions(&[None, Some(act), None]).unwrap_err(),
            RunReplayError::ActionRefused {
                tick: 1,
                error: ActionError::Downed
            }
        );
    }

    /// L8: a replay forged with an ACTIONS step of a downed member does not verify.
    #[test]
    fn a_replay_with_an_action_of_a_downed_player_does_not_verify() {
        use crate::run_replay::Record;
        let setup = RunSetup {
            seed: 20_261_010,
            ..RunSetup::default()
        };
        let mut rec = RunRecorder::start_party(&setup, 2).unwrap();
        let walk = Input {
            move_x: 100,
            ..Input::default()
        };
        let one = crate::id::PlayerIdx::new(1).unwrap();
        while rec.engine().snapshot().players[1].down.is_none() {
            assert!(rec.engine().tick() < 4_000, "slot 1 goes down");
            rec.step(&[walk, Input::default(), Input::default()])
                .unwrap();
        }
        assert!(
            rec.engine()
                .check_action(one, EquipAction::Extract { item: 0 })
                .is_err()
        );
        rec.records.push(Record::Actions {
            actions: vec![None, Some(EquipAction::Extract { item: 0 })],
        });
        assert_eq!(
            verify_run(&rec.replay().to_bytes()).unwrap_err(),
            RunReplayError::ActionRefused {
                tick: rec.engine().tick(),
                error: ActionError::Downed
            }
        );
    }

    fn v1_pack() -> crate::pack::Pack {
        crate::pack::Pack::from_bytes(include_bytes!(
            "../tests/corpus/v1/packs/32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5.json"
        ))
        .expect("the version 1 pack")
    }

    #[test]
    fn a_version_1_recorder_refuses_interact() {
        let frozen = RunSetup {
            schema_version: 1,
            sim_version: 1,
            ..RunSetup::default()
        };
        let mut rec = RunRecorder::start_with_pack(&frozen, &v1_pack()).unwrap();
        rec.step(&[Input::default(); 3]).unwrap();
        let (replay, tick) = (rec.replay(), rec.engine().tick());
        let mut held = [Input::default(); 3];
        held[0].interact = true;
        assert_eq!(
            rec.step(&held),
            Err(RunReplayError::BadInputs("a version 1 run has no interact"))
        );
        assert_eq!((rec.replay(), rec.engine().tick()), (replay, tick));
        // The live rules take the same input.
        let mut live = RunRecorder::start(&RunSetup::default()).unwrap();
        assert_eq!(live.step(&held), Ok(()));
    }

    #[test]
    fn a_replay_with_an_action_step_loads_and_ranges_but_does_not_verify() {
        use crate::pack::Pack;
        use crate::run_replay::Record;
        use crate::step_log::RangeStep;
        // The frozen rules have no equipment: every action is refused.
        let frozen = RunSetup {
            schema_version: 1,
            sim_version: 1,
            ..RunSetup::default()
        };
        // The frozen rules play the pack they were released with, not the bundled files.
        let v1 = Pack::from_bytes(include_bytes!(
            "../tests/corpus/v1/packs/32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5.json"
        ))
        .expect("the version 1 pack");
        let mut rec = RunRecorder::start_with_pack(&frozen, &v1).unwrap();
        rec.step(&[Input::default(); 3]).unwrap();
        let mut replay = rec.replay();
        rec.records.push(Record::Actions {
            actions: vec![Some(EquipAction::Extract { item: 0 })],
        });
        // A schema 1 file cannot hold an action: the replay is forged at schema 2, with the log
        // written by hand.
        replay.setup.schema_version = 2;
        replay.inputs = crate::base64::encode(
            &crate::step_log::StepLog {
                present: 0b1,
                records: rec.records.clone(),
            }
            .encode()
            .unwrap(),
        );
        replay.commands = String::new();
        let refused = RunReplayError::ActionRefused {
            tick: 1,
            error: ActionError::NotBuilt,
        };
        assert_eq!(
            refused.to_string(),
            "an equipment action on tick 1 is not built yet"
        );
        assert_eq!(
            crate::run_replay::verify_run_with(&replay.to_bytes(), v1.content()).unwrap_err(),
            refused
        );
        let range = replay.step_range(0, 1, 1_000_000).unwrap();
        assert!(
            range
                .steps
                .iter()
                .any(|s| matches!(s, RangeStep::Actions { .. }))
        );
        assert_eq!(
            RunRecorder::resume_with_pack(&replay, &v1).err(),
            Some(refused)
        );
    }
}

#[cfg(test)]
mod party_size_tests {
    use super::RunRecorder;
    use crate::base64;
    use crate::input::Input;
    use crate::run_setup::RunSetup;

    fn setup() -> RunSetup {
        RunSetup {
            seed: 20_261_010,
            ..RunSetup::default()
        }
    }

    #[test]
    fn the_size_is_the_present_byte() {
        for n in 1..=3u8 {
            let mut rec = RunRecorder::start_party(&setup(), n).unwrap();
            rec.step(&[Input::default(); 3]).unwrap();
            assert_eq!(rec.replay().party_size(), Some(n));
        }
    }

    #[test]
    fn a_byte_that_is_not_a_party_has_no_size() {
        let mut rec = RunRecorder::start_party(&setup(), 2).unwrap();
        rec.step(&[Input::default(); 3]).unwrap();
        let mut replay = rec.replay();
        let bytes = base64::decode(&replay.inputs).unwrap();
        for bad in [0u8, 0b010, 0b101, 0b1000, 0b1111, 0xFF] {
            let mut edited = bytes.clone();
            edited[0] = bad;
            replay.inputs = crate::base64::encode(&edited);
            assert_eq!(replay.party_size(), None, "{bad:#b}");
        }
        replay.inputs = String::new();
        assert_eq!(replay.party_size(), None);
        replay.inputs = "!!!!".into();
        assert_eq!(replay.party_size(), None);
    }

    #[test]
    fn a_schema_1_replay_is_one_player() {
        let mut rec = RunRecorder::start(&setup()).unwrap();
        rec.step(&[Input::default(), Default::default(), Default::default()])
            .unwrap();
        let mut replay = rec.replay();
        replay.setup.schema_version = 1;
        assert_eq!(replay.party_size(), Some(1));
    }
}
