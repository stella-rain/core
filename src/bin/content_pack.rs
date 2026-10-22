//! `content-pack`: checks a content pack as the game does (ADR-080, core#148), for the content
//! repository's release workflow, and writes the bundled content as a pack. The pack is
//! untrusted, so this uses the same size limit and validation as the game.
//!
//! ```text
//! content-pack verify FILE      (FILE may be - for standard input)
//! content-pack export
//! ```
//!
//! Exit status: 0 valid (or exported); 1 not valid (the pack is at fault); 2 the command line
//! or the file could not be read. On success, `verify` prints `sha256` (the pack's hash, the
//! SHA-256 of its bytes), `sim_version` and `starting_unlocks` (the counts of its lists, or
//! `none` for a pack released without them), `constellations` and `stardust` (its mode count)
//! as `name: value` lines, and `export` writes the pack
//! to standard output. Messages never repeat text from the pack.

use std::fs::File;
use std::io::{Read, Write};
use std::process::ExitCode;

use stella_rain_core::content_hash::to_hex;
use stella_rain_core::pack::{self, Pack};

const USAGE: &str = "usage: content-pack verify FILE\n       \
                     content-pack export\n\
                     FILE may be - for standard input. Exit status: 0 valid, \
                     1 not valid, 2 could not run.";

enum Failure {
    /// Exit 2.
    Usage(String),
    /// Exit 1.
    NotValid(String),
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
        Err(Failure::NotValid(message)) => {
            eprintln!("not valid: {message}");
            ExitCode::from(1)
        }
    }
}

/// Reads at most `Pack::MAX_BYTES` bytes, plus one so that the library can report a pack that
/// is too big.
fn read_pack(path: &str) -> Result<Vec<u8>, Failure> {
    let cap = u64::try_from(Pack::MAX_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut bytes = Vec::new();
    let read = if path == "-" {
        std::io::stdin().lock().take(cap).read_to_end(&mut bytes)
    } else {
        File::open(path).and_then(|f| f.take(cap).read_to_end(&mut bytes))
    };
    read.map(|_| bytes)
        .map_err(|e| Failure::Usage(format!("cannot read {path}: {}", e.kind())))
}

fn run(args: Vec<String>) -> Result<(), Failure> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["--help" | "-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["verify", path] => {
            let bytes = read_pack(path)?;
            let pack = Pack::from_bytes(&bytes).map_err(|e| Failure::NotValid(e.to_string()))?;
            println!("sha256: {}", to_hex(&pack.sha256()));
            println!("sim_version: {}", pack.sim_version());
            match pack.starting_unlocks() {
                Some(u) => println!(
                    "starting_unlocks: {} weapons, {} perk_sets, {} stage_skills, {} runes",
                    u.weapons().len(),
                    u.perk_sets().len(),
                    u.stage_skills().len(),
                    u.runes().len()
                ),
                None => println!("starting_unlocks: none"),
            }
            println!(
                "constellations: {} sets",
                pack.content().constellations().sets().len()
            );
            println!("stardust: {} modes", pack.stardust().modes().len());
            Ok(())
        }
        ["export"] => std::io::stdout()
            .lock()
            .write_all(&pack::export_bundled())
            .map_err(|e| Failure::Usage(format!("cannot write the pack: {}", e.kind()))),
        _ => Err(Failure::Usage(USAGE.into())),
    }
}
