//! `content-pack`: checks a content pack as the game does (ADR-CONT-5, core#148), for the content
//! repository's release workflow, and writes the bundled content as a pack. The pack is
//! untrusted, so this uses the same size limit and validation as the game.
//!
//! ```text
//! content-pack verify FILE [--text TEXT_FILE | --text-dir DIR]  (FILE, TEXT_FILE may be - for standard input)
//! content-pack export [DIR]
//! content-pack unpack PACK DIR
//! content-pack compare --numbers-only BASE_DIR HEAD_DIR
//! ```
//!
//! Exit status: 0 valid (or exported); 1 not valid (the pack is at fault); 2 the command line
//! or the file could not be read. On success, `verify` prints `sha256` (the pack's hash, the
//! SHA-256 of its bytes), `sim_version` and `starting_unlocks` (the counts of its lists, or
//! `none` for a pack released without them), `constellations`, `stardust` (its mode count) and
//! `star_chart` (the counts of its lists, or `none`)
//! as `name: value` lines, and `export` writes the pack
//! to standard output. Without DIR, `export` writes the pack built into this binary; with DIR
//! (core's `content/`, as `unpack` takes it; the mode files in `DIR/../modes`), the pack of the
//! files there. `--text` checks one text table; `--text-dir` reads `DIR/text.<code>.json` for every
//! locale code (two or three lowercase letters) and checks that each shipped locale has a file,
//! English among them, and text for every shown ID (ADR-CONT-8), the two options excluding each
//! other. `unpack` (ADR-CONT-4) checks PACK as `verify` does, then writes the files it
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
use stella_rain_core::text::{TextSet, TextTable, is_locale_code};

const USAGE: &str = "usage: content-pack verify FILE [--text TEXT_FILE | --text-dir DIR]\n       \
                     content-pack export [DIR]\n       \
                     content-pack unpack PACK DIR\n       \
                     content-pack compare --numbers-only BASE_DIR HEAD_DIR\n       \
                     content-pack catalogue\n\
                     FILE, PACK and TEXT_FILE may be - for standard input. --text-dir checks every \
                     text.<code>.json in DIR against every shown ID. unpack writes the pack's \
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

fn read_text(path: &str) -> Result<Vec<u8>, Failure> {
    let cap = u64::try_from(TextTable::MAX_BYTES)
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

/// Reads `DIR/text.<code>.json` for every valid locale code into a set, then checks that every
/// shipped locale is there and that each has text for every shown ID of `pack`.
fn check_text_dir(pack: &Pack, dir: &Path) -> Result<(), Failure> {
    let unreadable =
        |e: std::io::Error| Failure::Usage(format!("cannot read the text directory: {}", e.kind()));
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(unreadable)? {
        let path = entry.map_err(unreadable)?.path();
        let code = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("text.")?.strip_suffix(".json"))
            .filter(|c| is_locale_code(c))
            .map(str::to_string);
        if let Some(code) = code
            && path.is_file()
        {
            let bytes = read_text(&path.to_string_lossy())?;
            files.push((code, bytes));
        }
    }
    files.sort();
    let pairs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(code, bytes)| (code.as_str(), bytes.as_slice()))
        .collect();
    let set = TextSet::from_files(&pairs).map_err(|e| Failure::NotValid(e.to_string()))?;
    if let Err(lacking) = set.check_shipped() {
        return Err(Failure::NotValid(format!(
            "missing locale files: {}",
            lacking.join(", ")
        )));
    }
    if let Err(missing) = pack.check_texts(&set) {
        return Err(Failure::NotValid(format!(
            "missing text for {} IDs: {}",
            missing.len(),
            missing.join(", ")
        )));
    }
    Ok(())
}

fn run(args: Vec<String>) -> Result<(), Failure> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if args.is_empty() {
        return Err(Failure::Usage(USAGE.into()));
    }
    match args.as_slice() {
        ["--help" | "-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["export"] => write_pack(Pack::BUNDLED),
        ["export", dir] => export_dir(Path::new(dir)),
        ["unpack", path, dir] => unpack(path, Path::new(dir)),
        ["compare", "--numbers-only", base_dir, head_dir] => {
            compare_dirs(Path::new(base_dir), Path::new(head_dir))
        }
        ["catalogue"] => write_catalogue(),
        _ if args.first().is_some_and(|a| *a == "verify") => verify_cmd(&args[1..]),
        _ => Err(Failure::Usage(USAGE.into())),
    }
}

fn verify_cmd(args: &[&str]) -> Result<(), Failure> {
    let mut pack_path = None;
    let mut text_path = None;
    let mut text_dir = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--text" {
            i += 1;
            if i >= args.len() {
                return Err(Failure::Usage("missing argument for --text".into()));
            }
            text_path = Some(args[i]);
        } else if args[i] == "--text-dir" {
            i += 1;
            if i >= args.len() {
                return Err(Failure::Usage("missing argument for --text-dir".into()));
            }
            text_dir = Some(args[i]);
        } else if pack_path.is_none() {
            pack_path = Some(args[i]);
        } else {
            return Err(Failure::Usage(USAGE.into()));
        }
        i += 1;
    }
    if text_path.is_some() && text_dir.is_some() {
        return Err(Failure::Usage(USAGE.into()));
    }
    let Some(path) = pack_path else {
        return Err(Failure::Usage(USAGE.into()));
    };
    let bytes = read_pack(path)?;
    let pack = Pack::from_bytes(&bytes).map_err(|e| Failure::NotValid(e.to_string()))?;
    if let Some(text_file) = text_path {
        let text_bytes = read_text(text_file)?;
        let text_table =
            TextTable::from_bytes(&text_bytes).map_err(|e| Failure::NotValid(e.to_string()))?;
        if let Err(missing) = pack.check_text(&text_table) {
            return Err(Failure::NotValid(format!(
                "missing text for {} IDs: {}",
                missing.len(),
                missing.join(", ")
            )));
        }
    }
    if let Some(dir) = text_dir {
        check_text_dir(&pack, Path::new(dir))?;
    }
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
    match pack.star_chart() {
        Some(c) => println!(
            "star_chart: {} weapons, {} perk_sets, {} stage_skills, {} stats",
            c.weapons().len(),
            c.perk_sets().len(),
            c.stage_skills().len(),
            c.stats().len()
        ),
        None => println!("star_chart: none"),
    }
    if !pack.threat_colours().is_empty() {
        println!("threat_colours: {}", pack.threat_colours().len());
    }
    println!(
        "rules: crowd_target {}, cap_headroom_pct {}",
        pack.content().rules().crowd_target(),
        pack.content().rules().cap_headroom_pct()
    );
    Ok(())
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

fn write_catalogue() -> Result<(), Failure> {
    let catalogue = stella_rain_core::limits::tuning_catalogue_markdown();
    std::io::stdout()
        .lock()
        .write_all(catalogue.as_bytes())
        .map_err(|e| Failure::Usage(format!("cannot write catalogue: {}", e.kind())))
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
