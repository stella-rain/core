//! The savepoint file (ADR-SAVE-4): a run saved at a tick and restored from its bytes and the
//! replay prefix, without re-simulating the prefix, and refused with one typed error per cause.

use crate::common::idle;
use crate::common::run::{TRIAL_LENGTH, trial_setup};
use crate::run_play::{play_run, play_run_with};
use serde_json::Value;
use stella_rain_core::content_hash::sha256;
use stella_rain_core::engine::Outcome;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::pack::Pack;
use stella_rain_core::replay::pack_inputs;
use stella_rain_core::run_replay::{Command, RunRecorder, RunReplay, record_run_with_pack};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::savepoint::{
    HEADER_LEN, MAX_SAVEPOINT_BYTES, SAVEPOINT_FORMAT, Savepoint, SavepointError,
};
use stella_rain_core::version::{NEWEST_READ_SCHEMA_VERSION, SIM_VERSION};

// The offsets of the header's fields (the layout table of the format).
const SIM_VERSION_AT: usize = 8;
const SCHEMA_VERSION_AT: usize = 12;
const TICK_AT: usize = 48;
const STATE_HASH_AT: usize = 52;
const PAYLOAD_LEN_AT: usize = 92;
const CHECKSUM_AT: usize = 96;

fn apply(rec: &mut RunRecorder, c: &Command) {
    match *c {
        Command::Step(input) => rec
            .step(&[input, Default::default(), Default::default()])
            .unwrap(),
        #[allow(unreachable_patterns)]
        _ => unreachable!("a run at the live rules holds no choice"),
    }
}

fn recorder_with(setup: &RunSetup, pack: &Pack, commands: &[Command]) -> RunRecorder {
    let mut rec = RunRecorder::start_with_pack(setup, pack).unwrap();
    commands.iter().for_each(|c| apply(&mut rec, c));
    rec
}

/// A recorder driven by `play_run`'s commands for `ticks` ticks.
fn run(ticks: u32) -> RunRecorder {
    recorder_with(&trial_setup(), Pack::bundled(), &play_run(ticks))
}

/// The savepoint's checksum made right again after a change to its header or payload.
fn reseal(bytes: &mut [u8]) {
    let mut sealed = bytes[..CHECKSUM_AT].to_vec();
    sealed.extend_from_slice(&bytes[HEADER_LEN..]);
    bytes[CHECKSUM_AT..CHECKSUM_AT + 32].copy_from_slice(&sha256(&sealed));
}

fn put_u32(bytes: &mut [u8], at: usize, v: u32) {
    bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn restore(bytes: &[u8], prefix: &RunReplay) -> Result<RunRecorder, SavepointError> {
    RunRecorder::resume_from_savepoint(bytes, prefix, Pack::bundled())
}

/// `bytes` are refused with `want`, and the caller's fallback, re-simulating the run's own
/// prefix, still works.
fn refused(bytes: &[u8], prefix: &RunReplay, want: SavepointError, good: &RunReplay, pack: &Pack) {
    assert_eq!(
        RunRecorder::resume_from_savepoint(bytes, prefix, pack).err(),
        Some(want)
    );
    assert!(RunRecorder::resume_with_pack(good, pack).is_ok());
}

fn refused_bundled(bytes: &[u8], prefix: &RunReplay, want: SavepointError, good: &RunReplay) {
    refused(bytes, prefix, want, good, Pack::bundled());
}

/// The commands of `all` after the first `play_run(ticks)` of them.
fn split(all: &[Command], ticks: u32) -> (&[Command], &[Command]) {
    let head = play_run(ticks).len();
    assert_eq!(
        &all[..head],
        &play_run(ticks)[..],
        "a run's head is the shorter run"
    );
    all.split_at(head)
}

#[test]
fn save_then_resume_continues_with_identical_hashes() {
    let all = play_run(600);
    let (head, tail) = split(&all, 300);
    let mut rec = recorder_with(&trial_setup(), Pack::bundled(), head);
    let bytes = rec.save();
    let mut r = restore(&bytes, &rec.replay()).unwrap();
    assert_eq!(r.engine().tick(), 300);
    assert_eq!(r.engine().state_hash(), rec.engine().state_hash());
    assert_eq!(r.replay(), rec.replay());

    for c in tail {
        apply(&mut rec, c);
        apply(&mut r, c);
        if matches!(c, Command::Step(_)) {
            assert_eq!(r.engine().state_hash(), rec.engine().state_hash());
        }
    }
    assert_eq!(r.replay().to_bytes(), rec.replay().to_bytes());
    let unbroken = record_run_with_pack(&trial_setup(), Pack::bundled(), &all).unwrap();
    assert_eq!(r.replay().to_bytes(), unbroken.to_bytes());
}

#[test]
fn saving_is_deterministic_and_a_resumed_run_saves_the_same_bytes() {
    let rec = run(300);
    let bytes = rec.save();
    assert_eq!(bytes, rec.save());
    let r = restore(&bytes, &rec.replay()).unwrap();
    let again = r.save();
    assert_eq!(again, bytes);
}

#[test]
fn a_savepoint_resumes_without_replaying_the_prefix() {
    let rec = run(300);
    let prefix = rec.replay();
    let r = restore(&rec.save(), &prefix).unwrap();
    assert_eq!(r.engine().tick(), 300);
}

// --- Refusals ----------------------------------------------------------------------------------

#[test]
fn format_a_file_that_is_not_a_savepoint_is_refused() {
    let rec = run(120);
    let good = rec.replay();
    let bytes = rec.save();
    refused_bundled(&[], &good, SavepointError::Format, &good);
    refused_bundled(&bytes[..7], &good, SavepointError::Format, &good);
    let mut magic = bytes.clone();
    magic[0] ^= 0xff;
    refused_bundled(&magic, &good, SavepointError::Format, &good);
    let mut format = vec![0xAB; 400];
    format[..4].copy_from_slice(&bytes[..4]);
    put_u32(&mut format, 4, SAVEPOINT_FORMAT + 1);
    refused_bundled(&format, &good, SavepointError::Format, &good);
}

#[test]
fn sim_version_another_sim_version_is_refused() {
    let rec = run(120);
    let good = rec.replay();
    let mut bytes = rec.save();
    // The checksum is wrong too: the version is read before it.
    put_u32(&mut bytes, SIM_VERSION_AT, SIM_VERSION + 1);
    refused_bundled(&bytes, &good, SavepointError::SimVersion, &good);
}

/// The bundled files written compactly: other bytes, so another pack hash.
fn kept_pack() -> Pack {
    let v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("a compact pack loads")
}

#[test]
fn pack_a_savepoint_of_another_pack_is_refused() {
    let kept = kept_pack();
    assert_ne!(kept.sha256(), Pack::bundled().sha256());
    let rec = recorder_with_pack(&kept, 120);
    let prefix = rec.replay();
    assert_eq!(
        restore(&rec.save(), &prefix).err(),
        Some(SavepointError::Pack)
    );
    // Its own pack restores it, and is the fallback's.
    assert!(RunRecorder::resume_from_savepoint(&rec.save(), &prefix, &kept).is_ok());
    assert!(RunRecorder::resume_with_pack(&prefix, &kept).is_ok());
}

fn recorder_with_pack(pack: &Pack, ticks: u32) -> RunRecorder {
    recorder_with(&trial_setup(), pack, &play_run(ticks))
}

#[test]
fn damaged_every_truncation_is_refused() {
    let rec = run(120);
    let good = rec.replay();
    let bytes = rec.save();
    let lengths = (8..HEADER_LEN + 40)
        .chain((HEADER_LEN + 40..bytes.len()).step_by(((bytes.len() - 200) / 50).max(1)))
        .chain([bytes.len() - 1]);
    for len in lengths {
        refused_bundled(&bytes[..len], &good, SavepointError::Damaged, &good);
    }

    let mut flipped = bytes.clone();
    flipped[HEADER_LEN + 20] ^= 1;
    refused_bundled(&flipped, &good, SavepointError::Damaged, &good);
    let mut tick = bytes.clone();
    tick[TICK_AT] ^= 1;
    refused_bundled(&tick, &good, SavepointError::Damaged, &good);
    let mut appended = bytes.clone();
    appended.push(0);
    refused_bundled(&appended, &good, SavepointError::Damaged, &good);

    let mut huge = bytes.clone();
    put_u32(&mut huge, PAYLOAD_LEN_AT, u32::MAX);
    reseal(&mut huge);
    refused_bundled(&huge, &good, SavepointError::Damaged, &good);

    // The length and the checksum agree with a payload the codec cannot read.
    let mut short = bytes[..bytes.len() - 5].to_vec();
    let n = u32::try_from(short.len() - HEADER_LEN).unwrap();
    put_u32(&mut short, PAYLOAD_LEN_AT, n);
    reseal(&mut short);
    refused_bundled(&short, &good, SavepointError::Damaged, &good);
    let mut long = bytes.clone();
    long.extend_from_slice(&[0, 0, 0]);
    let n = u32::try_from(long.len() - HEADER_LEN).unwrap();
    put_u32(&mut long, PAYLOAD_LEN_AT, n);
    reseal(&mut long);
    refused_bundled(&long, &good, SavepointError::Damaged, &good);

    // Over the size limit: refused without reading the rest.
    let mut big = vec![0u8; MAX_SAVEPOINT_BYTES + 1];
    big[..HEADER_LEN].copy_from_slice(&bytes[..HEADER_LEN]);
    refused_bundled(&big, &good, SavepointError::Damaged, &good);
}

#[test]
fn hash_mismatch_a_state_that_hashes_otherwise_is_refused() {
    let rec = run(300);
    let good = rec.replay();
    let bytes = rec.save();
    let mut other = bytes.clone();
    other[STATE_HASH_AT] ^= 1;
    reseal(&mut other);
    refused_bundled(&other, &good, SavepointError::HashMismatch, &good);

    // One byte of the payload changed and the file resealed is never a different state taken
    // as the saved one: it is refused, or it hashes as saved.
    let mut mismatches = 0;
    for i in HEADER_LEN..HEADER_LEN + 256 {
        let mut flipped = bytes.clone();
        flipped[i] ^= 1;
        reseal(&mut flipped);
        match restore(&flipped, &good) {
            Err(SavepointError::HashMismatch) => mismatches += 1,
            Err(SavepointError::Damaged) => {}
            Ok(r) => assert_eq!(r.engine().state_hash(), rec.engine().state_hash()),
            Err(e) => panic!("byte {i}: {e:?}"),
        }
    }
    assert!(
        mismatches > 0,
        "some byte of the window is part of the hash"
    );
}

#[test]
fn prefix_mismatch_a_prefix_that_is_not_this_savepoints_run_is_refused() {
    let rec = run(300);
    let good = rec.replay();
    let bytes = rec.save();
    let refuse =
        |prefix: &RunReplay| refused_bundled(&bytes, prefix, SavepointError::PrefixMismatch, &good);

    refuse(&run(240).replay());

    let mut hash = good.clone();
    hash.final_hash = "0000000000000000".into();
    refuse(&hash);

    let mut inputs = good.unpack_inputs(good.ticks).unwrap();
    inputs[100].move_x = inputs[100].move_x.wrapping_neg().max(-i8::MAX);
    let mut altered = good.clone();
    altered.inputs = pack_inputs(&inputs);
    assert_ne!(altered.inputs, good.inputs);
    refuse(&altered);

    let other_seed = RunSetup {
        seed: trial_setup().seed + 1,
        ..trial_setup()
    };
    // The script's picks answer the offers of its own seed (the 64 px circle takes a star by tick
    // 297); another seed's run answers whatever offer is open with the first option instead.
    let mut other = RunRecorder::start_with_pack(&other_seed, Pack::bundled()).unwrap();
    for c in play_run(300) {
        if let Command::Step(input) = c {
            other
                .step(&[input, Default::default(), Default::default()])
                .unwrap();
        }
    }
    refuse(&other.replay());

    let mut content = good.clone();
    // Another first digit than the hash has (a pack whose hash starts with 0 stays itself).
    let first = if good.content_sha256.starts_with('0') {
        '1'
    } else {
        '0'
    };
    content.content_sha256 = format!("{first}{}", &good.content_sha256[1..]);
    refuse(&content);

    let idle_run = play_run_with(
        &trial_setup(),
        stella_rain_core::run_content::RunContent::bundled(),
        300,
        |_, _| idle(),
    );
    refuse(&recorder_with(&trial_setup(), Pack::bundled(), &idle_run).replay());
}

#[test]
fn ended_a_savepoint_of_an_ended_run_is_refused() {
    let rec = run(TRIAL_LENGTH);
    assert_ne!(
        rec.engine().outcome(),
        Outcome::Running,
        "the trial run ends"
    );
    let prefix = rec.replay();
    let bytes = rec.save();
    assert_eq!(restore(&bytes, &prefix).err(), Some(SavepointError::Ended));
}

#[test]
fn every_byte_of_the_header_region_is_checked() {
    let rec = run(120);
    let good = rec.replay();
    let bytes = rec.save();
    for at in 0..bytes.len().min(200) {
        let mut changed = bytes.clone();
        changed[at] ^= 1;
        assert!(
            restore(&changed, &good).is_err(),
            "a changed byte at {at} still loads"
        );
    }
    assert_eq!(
        Savepoint::from_bytes(&bytes)
            .unwrap()
            .header()
            .schema_version,
        // The schema the recorder writes (the header and the prefix agree).
        2
    );
}

#[test]
fn schema_version_another_value_is_refused() {
    let rec = run(120);
    let good = rec.replay();
    let bytes = rec.save();
    for v in [0, NEWEST_READ_SCHEMA_VERSION + 1, 7, u32::MAX] {
        let mut changed = bytes.clone();
        put_u32(&mut changed, SCHEMA_VERSION_AT, v);
        assert_eq!(
            restore(&changed, &good).err(),
            Some(SavepointError::Damaged),
            "schema_version {v} without a reseal"
        );
        reseal(&mut changed);
        refused_bundled(&changed, &good, SavepointError::SimVersion, &good);
    }
}

#[test]
fn the_header_is_read_before_the_payload() {
    let rec = run(120);
    let bytes = rec.save();
    let mut header = bytes[..HEADER_LEN].to_vec();
    put_u32(&mut header, PAYLOAD_LEN_AT, 1 << 30);
    assert_eq!(
        Savepoint::from_bytes(&header).err(),
        Some(SavepointError::Damaged)
    );
    assert_eq!(
        Savepoint::from_bytes(&bytes[..HEADER_LEN + 3]).err(),
        Some(SavepointError::Damaged)
    );
}

// --- The format guard ----------------------------------------------------------------------------

/// The pin files hold the history of the format: `v1.sha256` to `v<SAVEPOINT_FORMAT>.sha256`,
/// one per format, never re-pinned. The first was re-pinned once, when the replay schema moved
/// from 1 to 2 and the layout did not: the header carries `SCHEMA_VERSION` and `history_sha256`
/// hashes the prefix replay's bytes (now the step log), so those two fields changed;
/// `SAVEPOINT_FORMAT` stayed 1 and a savepoint an older build saved (header 1, schema 1 prefix)
/// still restores (`a_savepoint_header_must_agree_with_its_prefix`). Old line:
/// `aa70e16f9c1c228ce7ee88d15d384f017cfcb90dc863122cfe9196c9909b7619  2123`. From there a pinned
/// format is never re-pinned. Format 2 has its own line in `v2.sha256` (the frozen `trial` case's
/// version 1 state under the format 2 header and the 60-byte stats record); `v1.sha256` stays as
/// history, format 1 files being refused. Format 4 adds each player's equipment (`v4.sha256`);
/// format 5 holds the longer stat totals, 11 instead of 6 (`v5.sha256`); format 7 drops the stale
/// offer fields (`v7.sha256`); format 8 drops the star-offer fields; formats 1 to 7 are refused.
/// The golden test of the frozen `trial` run reproduces the newest pin; the byte pin of the live
/// format is `sim::players_pin_tests` in `src/`.
#[test]
fn every_older_format_has_a_pin_and_is_refused() {
    let dir = crate::data_root::data_root().join("tests/fixtures/savepoint");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let n: u32 = name
            .strip_prefix('v')
            .and_then(|n| n.strip_suffix(".sha256"))
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("{name}: not a pin file"));
        found.push(n);
    }
    found.sort_unstable();
    assert_eq!(found, (1..=SAVEPOINT_FORMAT).collect::<Vec<_>>());
}

// --- Format 2: the player stats record --------------------------------------------------------

/// The record is 3 slots of `damage_dealt: u64`, `kills: u32`, `support: u64`, little-endian.
const STATS_RECORD: usize = 60;

fn record_of(stats: &stella_rain_core::ledger::PlayerStats) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..3 {
        let s = stats.slot(stella_rain_core::id::PlayerIdx::new(i).unwrap());
        out.extend(s.damage_dealt.to_le_bytes());
        out.extend(s.kills.to_le_bytes());
        out.extend(s.support.to_le_bytes());
    }
    out
}

#[test]
fn format_eight_is_written_and_an_older_file_is_refused() {
    let rec = run(300);
    let good = rec.replay();
    let bytes = rec.save();
    assert_eq!(bytes[4..8], 8u32.to_le_bytes());
    assert_eq!(SAVEPOINT_FORMAT, 8);
    for older in [1, 2, 3, 4, 5, 6, 7] {
        let mut old = bytes.clone();
        put_u32(&mut old, 4, older);
        reseal(&mut old);
        refused_bundled(&old, &good, SavepointError::Format, &good);
    }
}

#[test]
fn the_payload_starts_with_the_stats_record() {
    let rec = run(600);
    let stats = rec.engine().player_stats();
    assert!(
        stats
            .slot(stella_rain_core::id::PlayerIdx::SOLO)
            .damage_dealt
            > 0
    );
    let bytes = rec.save();
    let payload = &bytes[HEADER_LEN..];
    assert_eq!(payload[..STATS_RECORD], record_of(stats)[..]);
    assert_eq!(
        payload[..8],
        stats
            .slot(stella_rain_core::id::PlayerIdx::SOLO)
            .damage_dealt
            .to_le_bytes()
    );
}

#[test]
fn a_restored_run_has_the_totals_it_was_saved_with() {
    let all = play_run(900);
    let (head, tail) = split(&all, 450);
    let mut saver = recorder_with(&trial_setup(), Pack::bundled(), head);
    let (bytes, prefix) = (saver.save(), saver.replay());
    let mut restored = restore(&bytes, &prefix).unwrap();
    assert_eq!(
        restored.engine().player_stats(),
        saver.engine().player_stats()
    );
    tail.iter().for_each(|c| {
        apply(&mut saver, c);
        apply(&mut restored, c);
    });
    assert_eq!(
        restored.engine().player_stats(),
        saver.engine().player_stats()
    );
    assert_eq!(restored.engine().state_hash(), saver.engine().state_hash());
}

#[test]
fn an_edited_record_restores_with_the_edited_totals_and_saves_the_same_bytes() {
    let rec = run(450);
    let prefix = rec.replay();
    let mut bytes = rec.save();
    bytes[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&777u64.to_le_bytes());
    reseal(&mut bytes);
    let restored = restore(&bytes, &prefix).expect("the record is not hashed");
    let slot0 = restored
        .engine()
        .player_stats()
        .slot(stella_rain_core::id::PlayerIdx::SOLO);
    assert_eq!(slot0.damage_dealt, 777);
    assert_eq!(restored.save(), bytes);
}

#[test]
fn a_record_for_a_player_who_is_not_in_the_party_is_accepted() {
    // A player who left keeps the totals they had, so the record of an absent slot may hold
    // them; the record is unhashed and trusted like the replay prefix.
    let rec = run(450);
    let prefix = rec.replay();
    let mut bytes = rec.save();
    // Slot 2's damage_dealt, 40 bytes into the record.
    bytes[HEADER_LEN + 40] = 1;
    reseal(&mut bytes);
    let restored = restore(&bytes, &prefix).unwrap();
    assert_eq!(
        restored
            .engine()
            .player_stats()
            .slot(PlayerIdx::new(2).unwrap())
            .damage_dealt,
        1
    );
}
