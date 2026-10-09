//! `stage-verify`: re-simulates a replay against its stage and checks that the stage is cleared
//! with the replay's final hash (ADR-009, ADR-015). Stage and replay are untrusted, so this
//! uses the same size limits and validation as the game.
//!
//! ```text
//! stage-verify STAGE REPLAY [--hashes FILE]
//! stage-verify --share-code FILE [--hashes FILE]      (FILE may be - for standard input)
//! ```
//!
//! `--hashes` names a file of expected state hashes, one per tick, 16 lowercase hex digits per
//! line; on a mismatch the first differing tick is reported.
//!
//! Exit status: 0 verified; 1 not verified (the stage or the replay is at fault); 2 the
//! command line or a file could not be read. On success, standard output holds `result`,
//! `ticks`, `final_hash`, `content_hash` (ADR-034) and `parsed_stage_hash` (ADR-046) as
//! `name: value` lines. Messages never repeat text from the files.

use std::fs::File;
use std::io::Read;
use std::process::ExitCode;

use stella_rain_core::content_hash::to_hex;
use stella_rain_core::replay::{self, MAX_REPLAY_BYTES, ReplayError};
use stella_rain_core::share::{self, MAX_SHARE_CODE_CHARS};
use stella_rain_core::validate::limits::MAX_STAGE_BYTES;

const USAGE: &str = "usage: stage-verify STAGE REPLAY [--hashes FILE]\n       \
                     stage-verify --share-code FILE [--hashes FILE]\n\
                     FILE may be - for standard input. Exit status: 0 verified, \
                     1 not verified, 2 could not run.";

/// A hashes file is at most this big: one line per tick of the longest stage, with room to spare.
const MAX_HASHES_BYTES: usize = 1024 * 1024;

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
    let usage = |what: &str| Failure::Usage(format!("the hashes file {what}"));
    if bytes.len() > MAX_HASHES_BYTES {
        return Err(usage("is too large"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| usage("is not text"))?;
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            replay::hash_from_hex(l).ok_or_else(|| usage("has a line that is not 16 hex digits"))
        })
        .collect()
}

fn run(args: Vec<String>) -> Result<(), Failure> {
    let mut positional = Vec::new();
    let mut share_code = None;
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
            "--hashes" => hashes_file = Some(value_of("--hashes")?),
            flag if flag.starts_with("--") => {
                return Err(Failure::Usage(format!("unknown option {flag}\n{USAGE}")));
            }
            _ => positional.push(arg),
        }
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
            read_up_to(replay, MAX_REPLAY_BYTES)?,
        ),
        _ => return Err(Failure::Usage(USAGE.into())),
    };
    let expected = match hashes_file {
        Some(path) => Some(parse_hashes(&read_up_to(&path, MAX_HASHES_BYTES)?)?),
        None => None,
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
