//! `stage-verify`: re-simulates a replay against its stage and checks that the stage is cleared
//! with the replay's final hash (ADR-009, ADR-015). Stage and replay are untrusted, so this
//! uses the same size limits and validation as the game. Attack presets a stage names are read
//! from the content pack built into this binary (`content/pack.json`).
//!
//! ```text
//! stage-verify STAGE REPLAY [--hashes FILE]
//! stage-verify --run REPLAY [--hashes FILE]
//! stage-verify --share-code FILE [--hashes FILE]      (FILE may be - for standard input)
//! ```
//!
//! `--hashes` names a file of expected state hashes, the state hash after every tick as 8 bytes
//! little-endian, the corpus's `hashes.bin` format; on a mismatch the first differing tick and
//! both hashes are reported.
//!
//! Exit status: 0 verified; 1 not verified (the stage or the replay is at fault); 2 the
//! command line or a file could not be read. On success, standard output holds `result`,
//! `ticks`, `final_hash`, `content_hash` (ADR-034) and `parsed_stage_hash` (ADR-046) as
//! `name: value` lines. Messages never repeat text from the files.

#![forbid(unsafe_code)]

use std::fs::File;
use std::io::Read;
use std::process::ExitCode;

use stella_rain_core::content_hash::to_hex;
use stella_rain_core::engine::Outcome;
use stella_rain_core::replay::{self, MAX_HASHES_BYTES, MAX_STAGE_REPLAY_BYTES, ReplayError};
use stella_rain_core::run_replay::{self, MAX_RUN_REPLAY_BYTES};
use stella_rain_core::share::{self, MAX_SHARE_CODE_CHARS};
use stella_rain_core::validate::limits::MAX_STAGE_BYTES;

const USAGE: &str = "usage: stage-verify STAGE REPLAY [--hashes FILE]\n       \
                     stage-verify --run REPLAY [--hashes FILE]\n       \
                     stage-verify --share-code FILE [--hashes FILE]\n\
                     FILE may be - for standard input. Exit status: 0 verified, \
                     1 not verified, 2 could not run.\n\
                     Attack presets a stage names are read from the content pack built into \
                     this binary (content/pack.json).";

enum Failure {
    /// Exit 2.
    Usage(String),
    /// Exit 1.
    NotVerified(String),
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
        Err(Failure::NotVerified(message)) => {
            eprintln!("not verified: {message}");
            ExitCode::from(1)
        }
    }
}

/// Reads at most `limit` bytes, plus one so that the library can report a file that is too big.
fn read_up_to(path: &str, limit: usize) -> Result<Vec<u8>, Failure> {
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    let read = if path == "-" {
        std::io::stdin().lock().take(cap).read_to_end(&mut bytes)
    } else {
        File::open(path).and_then(|f| f.take(cap).read_to_end(&mut bytes))
    };
    read.map(|_| bytes)
        .map_err(|e| Failure::Usage(format!("cannot read {path}: {}", e.kind())))
}

fn parse_hashes(bytes: &[u8]) -> Result<Vec<u64>, Failure> {
    replay::hashes_from_bytes(bytes).map_err(|e| Failure::Usage(e.to_string()))
}

fn run(args: Vec<String>) -> Result<(), Failure> {
    let mut positional = Vec::new();
    let mut share_code = None;
    let mut run_replay = None;
    let mut hashes_file = None;
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut value_of = |flag: &str| {
            it.next()
                .ok_or_else(|| Failure::Usage(format!("{flag} needs a file\n{USAGE}")))
        };
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--share-code" => share_code = Some(value_of("--share-code")?),
            "--run" => run_replay = Some(value_of("--run")?),
            "--hashes" => hashes_file = Some(value_of("--hashes")?),
            flag if flag.starts_with("--") => {
                return Err(Failure::Usage(format!("unknown option {flag}\n{USAGE}")));
            }
            _ => positional.push(arg),
        }
    }

    let expected = match hashes_file {
        Some(path) => Some(parse_hashes(&read_up_to(&path, MAX_HASHES_BYTES)?)?),
        None => None,
    };

    if let (None, Some(path), []) = (&share_code, &run_replay, positional.as_slice()) {
        let replay_bytes = read_up_to(path, MAX_RUN_REPLAY_BYTES)?;
        let verified = match &expected {
            Some(hashes) => run_replay::verify_run_against(&replay_bytes, hashes),
            None => run_replay::verify_run(&replay_bytes),
        }
        .map_err(|e| Failure::NotVerified(e.to_string()))?;

        let outcome = match verified.outcome {
            Outcome::Cleared => "cleared",
            Outcome::Failed => "failed",
            Outcome::Running => "running",
        };
        println!("result: {outcome}");
        println!("ticks: {}", verified.ticks);
        println!("final_hash: {}", replay::hash_to_hex(verified.final_hash));
        println!("commands: {}", verified.commands);
        println!("stardust: {}", verified.stardust);
        let ledger_json =
            serde_json::to_string(&verified.ledger).map_err(|e| Failure::Usage(e.to_string()))?;
        println!("ledger: {ledger_json}");
        return Ok(());
    }

    let (stage, replay_bytes) = match (share_code, positional.as_slice()) {
        (Some(path), []) => {
            let raw = read_up_to(&path, MAX_SHARE_CODE_CHARS)?;
            let code = std::str::from_utf8(&raw)
                .map_err(|_| Failure::NotVerified("the share code is not text".into()))?;
            share::decode(code.trim()).map_err(|e| Failure::NotVerified(e.to_string()))?
        }
        (None, [stage, replay]) => (
            read_up_to(stage, MAX_STAGE_BYTES)?,
            read_up_to(replay, MAX_STAGE_REPLAY_BYTES)?,
        ),
        _ => return Err(Failure::Usage(USAGE.into())),
    };

    let verified = match &expected {
        Some(hashes) => replay::verify_against(&stage, &replay_bytes, hashes),
        None => replay::verify(&stage, &replay_bytes),
    }
    .map_err(|e: ReplayError| Failure::NotVerified(e.to_string()))?;

    println!("result: cleared");
    println!("ticks: {}", verified.ticks);
    println!("final_hash: {}", replay::hash_to_hex(verified.final_hash));
    println!("content_hash: {}", to_hex(&verified.content_hash));
    println!("parsed_stage_hash: {}", to_hex(&verified.parsed_stage_hash));
    Ok(())
}
