//! The savepoint file (ADR-SAVE-3): a run saved at a tick and restored from its bytes and the
//! replay prefix, without re-simulating the prefix, and refused with one typed error per cause.

use crate::common::idle;
use crate::common::run::{TRIAL_LENGTH, trial_setup};
use crate::run_play::{play_run, play_run_with};
use serde_json::Value;
use stella_rain_core::content_hash::sha256;
use stella_rain_core::engine::Outcome;
use stella_rain_core::pack::Pack;
use stella_rain_core::replay::pack_inputs;
use stella_rain_core::run_replay::{
    Command, RunRecorder, RunReplay, RunReplayError, record_run_with_pack,
};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::savepoint::{
    HEADER_LEN, MAX_SAVEPOINT_BYTES, SAVEPOINT_FORMAT, Savepoint, SavepointError,
};

// The offsets of the header's fields (the layout table of the format).
const SIM_VERSION_AT: usize = 8;
const SCHEMA_VERSION_AT: usize = 12;
const TICK_AT: usize = 48;
const STATE_HASH_AT: usize = 52;
const PAYLOAD_LEN_AT: usize = 92;
const CHECKSUM_AT: usize = 96;
const CRATE_VERSION_AT: usize = 128;

fn apply(rec: &mut RunRecorder, c: &Command) {
    match *c {
        Command::Step(input) => rec.step(input).unwrap(),
        Command::Choose(index) => rec.choose(index).unwrap(),
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
fn a_savepoint_taken_with_an_offer_open_resumes_with_it_open() {
    let all = play_run(600);
    let at = all
        .iter()
        .position(|c| matches!(c, Command::Choose(_)))
        .unwrap();
    let mut rec = recorder_with(&trial_setup(), Pack::bundled(), &all[..at]);
    assert!(rec.engine().offer_open());
    let mut r = restore(&rec.save(), &rec.replay()).unwrap();
    assert!(r.engine().offer_open());
    assert!(matches!(
        r.step(idle()),
        Err(RunReplayError::OfferOpen { .. })
    ));
    for c in &all[at..] {
        apply(&mut rec, c);
        apply(&mut r, c);
        assert_eq!(r.engine().state_hash(), rec.engine().state_hash());
    }
    assert_eq!(r.replay().to_bytes(), rec.replay().to_bytes());
}

#[test]
fn saving_is_deterministic_and_a_resumed_run_saves_the_same_bytes() {
    let rec = run(300);
    let bytes = rec.save();
    assert_eq!(bytes, rec.save());
    let r = restore(&bytes, &rec.replay()).unwrap();
    let again = r.save();
    assert_eq!(again.len(), bytes.len());
    // The crate version is the only field a build may change.
    assert_eq!(
        again[..CRATE_VERSION_AT],
        bytes[..CRATE_VERSION_AT],
        "the header before the crate version"
    );
    assert_eq!(again[HEADER_LEN..], bytes[HEADER_LEN..]);
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
    put_u32(&mut bytes, SIM_VERSION_AT, 2);
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
    refuse(&recorder_with(&other_seed, Pack::bundled(), &play_run(300)).replay());

    let mut content = good.clone();
    content.content_sha256 = format!("0{}", &good.content_sha256[1..]);
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
fn the_crate_version_is_never_compared() {
    let rec = run(120);
    let good = rec.replay();
    let bytes = rec.save();
    for text in [&b"9.9.9"[..], &[0u8; 5][..]] {
        let mut changed = bytes.clone();
        changed[CRATE_VERSION_AT..CRATE_VERSION_AT + 32].fill(0);
        changed[CRATE_VERSION_AT..CRATE_VERSION_AT + text.len()].copy_from_slice(text);
        assert!(restore(&changed, &good).is_ok());
    }
    assert_eq!(
        Savepoint::from_bytes(&bytes)
            .unwrap()
            .header()
            .schema_version,
        1
    );
    let mut schema = bytes.clone();
    put_u32(&mut schema, SCHEMA_VERSION_AT, 7);
    reseal(&mut schema);
    assert!(restore(&schema, &good).is_ok());
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

mod golden {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use stella_rain_core::content_hash::to_hex;
    use stella_rain_core::replay::hash_from_hex;

    fn corpus() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/v1/runs/trial")
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/savepoint")
    }

    /// The frozen `trial` case, its replay and its pack, read and never written.
    fn case() -> (RunReplay, Pack) {
        let dir = corpus();
        let replay = RunReplay::from_bytes(&fs::read(dir.join("run.replay")).unwrap()).unwrap();
        let case: Value =
            serde_json::from_slice(&fs::read(dir.join("case.json")).unwrap()).unwrap();
        let sha = case["pack_sha256"].as_str().unwrap();
        let pack = fs::read(corpus().join(format!("../../packs/{sha}.json"))).unwrap();
        (replay, Pack::from_bytes(&pack).unwrap())
    }

    /// The case's run driven to `ticks` ticks, its commands applied as the replay holds them.
    fn drive(
        replay: &RunReplay,
        pack: &Pack,
        ticks: u32,
        from: Option<RunRecorder>,
    ) -> RunRecorder {
        let inputs = replay.unpack_inputs(replay.ticks).unwrap();
        let commands = replay.unpack_commands().unwrap();
        let mut rec =
            from.unwrap_or_else(|| RunRecorder::start_with_pack(&replay.setup, pack).unwrap());
        let mut next = commands
            .iter()
            .take_while(|c| c.tick < rec.engine().tick())
            .count();
        let take = |rec: &mut RunRecorder, next: &mut usize| {
            while let Some(c) = commands
                .get(*next)
                .filter(|c| c.tick == rec.engine().tick())
            {
                rec.choose(c.index).unwrap();
                *next += 1;
            }
        };
        while rec.engine().tick() < ticks {
            take(&mut rec, &mut next);
            rec.step(inputs[usize::try_from(rec.engine().tick()).unwrap()])
                .unwrap();
        }
        take(&mut rec, &mut next);
        rec
    }

    /// The case's savepoint at tick 300, its crate version zeroed.
    fn bytes() -> (Vec<u8>, RunReplay, Pack, RunReplay) {
        let (replay, pack) = case();
        let rec = drive(&replay, &pack, 300, None);
        let mut bytes = rec.save();
        bytes[CRATE_VERSION_AT..HEADER_LEN].fill(0);
        let prefix = rec.replay();
        (bytes, prefix, pack, replay)
    }

    fn pin_path() -> PathBuf {
        fixtures().join(format!("v{SAVEPOINT_FORMAT}.sha256"))
    }

    fn line(bytes: &[u8]) -> String {
        format!("{}  {}\n", to_hex(&sha256(bytes)), bytes.len())
    }

    #[expect(clippy::disallowed_methods, reason = "test knob")]
    fn updating() -> bool {
        std::env::var("UPDATE_SAVEPOINT_PIN").as_deref() == Ok("1")
    }

    #[test]
    fn the_encoding_matches_its_pinned_format() {
        let (bytes, ..) = bytes();
        let path = pin_path();
        if updating() {
            assert!(
                !path.exists(),
                "{path:?} exists: a pinned format is never re-pinned"
            );
            fs::create_dir_all(fixtures()).unwrap();
            fs::write(&path, line(&bytes)).unwrap();
        }
        let pinned = fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            line(&bytes),
            pinned,
            "the savepoint encoding changed without a bump: bump SAVEPOINT_FORMAT in \
             src/savepoint/file.rs, then `UPDATE_SAVEPOINT_PIN=1 cargo test --test integration \
             savepoint::golden` writes tests/fixtures/savepoint/v<N>.sha256"
        );
    }

    #[test]
    fn the_pinned_file_loads_and_resumes() {
        let (bytes, prefix, pack, replay) = bytes();
        let sp = Savepoint::from_bytes(&bytes).unwrap();
        assert_eq!(sp.header().format, SAVEPOINT_FORMAT);
        let r = RunRecorder::resume_from_savepoint(&bytes, &prefix, &pack).unwrap();
        // To the end of tick 360: the checkpoint is taken before the commands that follow it.
        let mut r = drive(&replay, &pack, 359, Some(r));
        let input = replay.unpack_inputs(replay.ticks).unwrap()[359];
        r.step(input).unwrap();
        let checkpoints = replay.checkpoints.as_bytes();
        let want = hash_from_hex(std::str::from_utf8(&checkpoints[5 * 16..6 * 16]).unwrap());
        assert_eq!(Some(r.engine().state_hash()), want);
    }

    #[test]
    fn every_older_format_has_a_pin_and_is_refused() {
        let mut found = Vec::new();
        for entry in fs::read_dir(fixtures()).unwrap() {
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
}
