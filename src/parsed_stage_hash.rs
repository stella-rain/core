//! The parsed-stage hash (ADR-046): what the moderation blocklist blocks a stage by.
//!
//! `SHA-256( TAG ‖ compact JSON of the validated stage, fields in declared order, `id` left
//! out )`. It is taken over what `core` parsed, not over the file's bytes, so a reformatted
//! copy, a copy under another stage ID or one with another replay keeps the hash; the content
//! hash of ADR-034 does not. `blocklist.json` stores the result, so from the freeze of ADR-035
//! the definition is frozen: a change needs a new ADR and a new field name. The bytes are
//! pinned in `tests/parsed_stage_hash.rs`.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::behaviour::Agent;
use crate::stage::{Boss, Player, Stage, Wave};

/// Goes in front of the JSON, so the hash cannot be mistaken for one over a stage file.
const TAG: &[u8] = b"stella-rain/parsed-stage/v1\n";

/// `Stage` without its `id`, in `Stage`'s declared order and with its `serde` attributes.
#[derive(Serialize)]
struct Hashed<'a> {
    schema_version: u32,
    sim_version: u32,
    title: &'a str,
    description: &'a str,
    seed: u32,
    length_ticks: u32,
    player: &'a Player,
    companions: &'a [Agent],
    waves: &'a [Wave],
    #[serde(skip_serializing_if = "Option::is_none")]
    boss: &'a Option<Boss>,
}

/// The bytes the parsed-stage hash is taken over. `stage` must have passed the validator.
pub fn parsed_stage_hash_input(stage: &Stage) -> Vec<u8> {
    // Every field is named, so a new one does not compile until it is hashed or left out here.
    let Stage {
        schema_version,
        sim_version,
        id: _,
        title,
        description,
        seed,
        length_ticks,
        player,
        companions,
        waves,
        boss,
    } = stage;
    let hashed = Hashed {
        schema_version: *schema_version,
        sim_version: *sim_version,
        title,
        description,
        seed: *seed,
        length_ticks: *length_ticks,
        player,
        companions,
        waves,
        boss,
    };
    let mut bytes = TAG.to_vec();
    // Writing these types to a `Vec` cannot fail: no maps with non-string keys, no I/O.
    serde_json::to_writer(&mut bytes, &hashed).expect("a stage serialises to JSON");
    bytes
}

/// The parsed-stage hash of a validated stage; `content_hash::to_hex` gives the stored form.
pub fn parsed_stage_hash(stage: &Stage) -> [u8; 32] {
    Sha256::digest(parsed_stage_hash_input(stage)).into()
}
