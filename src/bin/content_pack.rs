//! `content-pack`: checks a content pack as the game does (ADR-080, core#148), for the content
//! repository's release workflow, and writes the bundled content as a pack. The pack is
//! untrusted, so this uses the same size limit and validation as the game.
//!
//! ```text
//! content-pack verify FILE      (FILE may be - for standard input)
//! content-pack export [DIR]
//! content-pack unpack PACK DIR
//! ```
//!
//! Exit status: 0 valid (or exported); 1 not valid (the pack is at fault); 2 the command line
//! or the file could not be read. On success, `verify` prints `sha256` (the pack's hash, the
//! SHA-256 of its bytes), `sim_version` and `starting_unlocks` (the counts of its lists, or
//! `none` for a pack released without them), `constellations` and `stardust` (its mode count)
//! as `name: value` lines, and `export` writes the pack
//! to standard output. Without DIR, `export` writes the pack built into this binary; with DIR
//! (core's `content/`, as `unpack` takes it; the mode files in `DIR/../modes`), the pack of the
//! files there. `unpack` (ADR-131) checks PACK as `verify` does, then writes the files it
//! holds into DIR (core's `content/`; the three mode files into `modes/` beside it), each as the
//! file on its own, LF, so that `export` of those files gives PACK byte for byte; a pack that
//! `export` would not write that way is refused. A file an optional slot of the pack lacks is
//! not written, and a stale one in DIR is removed; no other file is touched, and
//! `content/pack.json` is left to `export`. Messages never repeat text from the pack.

#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

use stella_rain_core::content_hash::to_hex;
use stella_rain_core::pack::{CompareError, ContentFiles, Pack};

const USAGE: &str = "usage: content-pack verify FILE\n       \
                     content-pack export [DIR]\n       \
                     content-pack unpack PACK DIR\n       \
                     content-pack compare --numbers-only BASE_DIR HEAD_DIR\n\
                     FILE and PACK may be - for standard input. unpack writes the pack's \
                     files into DIR (core's content/) and its mode files into modes/ beside \
                     it, and leaves content/pack.json to export. Without DIR, export writes the pack built \
                     into this binary; with DIR (core's content/, as unpack), the pack of the \
                     files there. Exit status: 0 valid or \
                     done, 1 not valid, 2 could not run.";

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
            println!(
                "rules: crowd_target {}, cap_headroom_pct {}",
                pack.content().rules().crowd_target(),
                pack.content().rules().cap_headroom_pct()
            );
            Ok(())
        }
        ["export"] => write_pack(Pack::BUNDLED),
        ["export", dir] => export_dir(Path::new(dir)),
        ["unpack", path, dir] => unpack(path, Path::new(dir)),
        ["compare", "--numbers-only", base_dir, head_dir] => {
            compare_dirs(Path::new(base_dir), Path::new(head_dir))
        }
        _ => Err(Failure::Usage(USAGE.into())),
    }
}

fn compare_dirs(base_dir: &Path, head_dir: &Path) -> Result<(), Failure> {
    if !base_dir.is_dir() {
        return Err(Failure::Usage("BASE_DIR is not a directory".into()));
    }
    if !head_dir.is_dir() {
        return Err(Failure::Usage("HEAD_DIR is not a directory".into()));
    }
    stella_rain_core::pack::compare_numbers_only(base_dir, head_dir).map_err(|e| match e {
        CompareError::Usage(msg) => Failure::Usage(msg),
        CompareError::NotValid(msg) => Failure::NotValid(msg),
    })
}

fn write_pack(bytes: &[u8]) -> Result<(), Failure> {
    std::io::stdout()
        .lock()
        .write_all(bytes)
        .map_err(|e| Failure::Usage(format!("cannot write the pack: {}", e.kind())))
}

/// Writes the pack of the files in `dir` (and `dir/../modes`), checked as `verify` checks one.
fn export_dir(dir: &Path) -> Result<(), Failure> {
    if !dir.is_dir() {
        return Err(Failure::Usage("DIR is not a directory".into()));
    }
    let files = ContentFiles::read_dir(dir)
        .map_err(|e| Failure::Usage(format!("cannot read the files: {}", e.kind())))?;
    let bytes = files
        .export()
        .map_err(|e| Failure::NotValid(e.to_string()))?;
    write_pack(&bytes)
}

/// Writes the files of the pack at `path` into `dir` and `dir/../modes`. Everything is checked
/// before the first write, so a refused pack leaves `dir` as it was.
fn unpack(path: &str, dir: &Path) -> Result<(), Failure> {
    let bytes = read_pack(path)?;
    let files = ContentFiles::from_pack(&bytes).map_err(|e| Failure::NotValid(e.to_string()))?;
    if !dir.is_dir() {
        return Err(Failure::Usage("DIR is not a directory".into()));
    }
    let modes = dir.join("..").join("modes");
    std::fs::create_dir_all(&modes)
        .map_err(|e| Failure::Usage(format!("cannot create modes: {}", e.kind())))?;
    for slot in ContentFiles::LAYOUT {
        let target = slot.path(dir);
        let result = match files.get(slot.name) {
            Some(bytes) => std::fs::write(&target, bytes),
            // An optional file the pack lacks: a stale copy would make `export` write it.
            None => match std::fs::remove_file(&target) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        result.map_err(|e| {
            Failure::Usage(format!("cannot write {}.json: {}", slot.name, e.kind()))
        })?;
    }
    Ok(())
}
