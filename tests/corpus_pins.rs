//! The pack each corpus case pins (`case.json`), the packs it names (`packs/<sha256>.json`) and
//! the frozen trees (`tests/corpus/v<N>/`, N >= 1), which are never written and carry a
//! `manifest.sha256` (ADR-SIM-3). `corpus.rs` plays the cases; this file loads what they pin,
//! writes it, and checks a frozen tree.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use stella_rain_core::content_hash::{sha256, to_hex};
use stella_rain_core::pack::{ContentFiles, Pack};
use stella_rain_core::run_replay::RunReplay;

use crate::common;
use crate::corpus::{check_case, check_run_case, diff_dir, write_atomic};

/// `case.json`: the pack a case plays on, by the SHA-256 of the pack file's bytes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseJson {
    pub pack_sha256: String,
    #[serde(default)]
    pub note: Option<String>,
}

fn case_name(case: &Path) -> String {
    case.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn is_hex_sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn read_case_json(case: &Path) -> Result<CaseJson, String> {
    let name = case_name(case);
    let bytes = fs::read(case.join("case.json"))
        .map_err(|e| format!("corpus case `{name}`: case.json: {e}"))?;
    let json: CaseJson = serde_json::from_slice(&bytes)
        .map_err(|e| format!("corpus case `{name}`: case.json: {e}"))?;
    if !is_hex_sha(&json.pack_sha256) {
        return Err(format!(
            "corpus case `{name}`: case.json: pack_sha256 is not 64 lowercase hex digits"
        ));
    }
    Ok(json)
}

fn is_frozen_tree(tree: &Path) -> bool {
    tree.file_name()
        .is_some_and(|n| is_frozen_name(&n.to_string_lossy()))
}

/// The pack `pin` names in `tree` (`<tree>/packs/<pin>.json`, its name checked against its
/// bytes). Only a tree that is not frozen resolves the bundled pack's hash without a file.
pub(crate) fn load_pack(tree: &Path, pin: &str) -> Result<Pack, String> {
    let file = tree.join("packs").join(format!("{pin}.json"));
    let bytes = match fs::read(&file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !is_frozen_tree(tree) && pin == to_hex(&Pack::bundled().sha256()) {
                return Ok(Pack::bundled().clone());
            }
            return Err(format!("pack sha256 {pin}: no file {}", file.display()));
        }
        Err(e) => return Err(format!("pack sha256 {pin}: {}: {e}", file.display())),
    };
    let actual = to_hex(&sha256(&bytes));
    if actual != pin {
        return Err(format!(
            "pack sha256 {pin}: {} hashes to {actual}",
            file.display()
        ));
    }
    let pack = Pack::from_bytes(&bytes)
        .map_err(|e| format!("pack sha256 {pin}: {}: {e:?}", file.display()))?;
    if to_hex(&pack.sha256()) != pin {
        return Err(format!(
            "pack sha256 {pin}: the loaded pack hashes differently"
        ));
    }
    Ok(pack)
}

/// The pack of the case folder `case` (`<tree>/stages/<name>` or `<tree>/runs/<name>`).
pub(crate) fn case_pack(case: &Path) -> Result<Pack, String> {
    let name = case_name(case);
    let tree = case
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| format!("corpus case `{name}`: no tree above it"))?;
    let pin = read_case_json(case)?.pack_sha256;
    load_pack(tree, &pin).map_err(|e| format!("corpus case `{name}`: {e}"))
}

/// The pack of a run case, which must also be the one its `run.replay` was recorded with.
pub(crate) fn run_case_pack(case: &Path, replay_bytes: &[u8]) -> Result<Pack, String> {
    let name = case_name(case);
    let pack = case_pack(case)?;
    let recorded = RunReplay::from_bytes(replay_bytes)
        .map_err(|e| format!("corpus case `{name}`: run.replay: {e}"))?;
    let pinned = to_hex(&pack.sha256());
    if recorded.content_sha256 != pinned {
        return Err(format!(
            "corpus case `{name}`: case.json pins pack {pinned}, run.replay was recorded with {}",
            recorded.content_sha256
        ));
    }
    Ok(pack)
}

/// A frozen folder is `v<N>` with N >= 1, digits only.
pub(crate) fn is_frozen_name(name: &str) -> bool {
    match name.strip_prefix('v') {
        Some(n) => !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && !n.starts_with('0'),
        None => false,
    }
}

/// Refuses a path with a frozen folder in it.
pub(crate) fn ensure_writable(dir: &Path) -> Result<(), String> {
    match dir
        .components()
        .find(|c| is_frozen_name(&c.as_os_str().to_string_lossy()))
    {
        Some(c) => Err(format!(
            "{} is inside the frozen corpus folder {}; a frozen tree is never written",
            dir.display(),
            c.as_os_str().to_string_lossy()
        )),
        None => Ok(()),
    }
}

pub(crate) fn write_case_json(case: &Path, pack_sha256: &str) -> Result<(), String> {
    ensure_writable(case)?;
    fs::create_dir_all(case).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&serde_json::json!({ "pack_sha256": pack_sha256 }))
        .map_err(|e| e.to_string())?;
    write_atomic(&case.join("case.json"), (json + "\n").as_bytes());
    Ok(())
}

/// Writes `packs/<sha256>.json` in `tree`; returns the hash.
pub(crate) fn write_pack_file(tree: &Path, bytes: &[u8]) -> Result<String, String> {
    ensure_writable(tree)?;
    let sha = to_hex(&sha256(bytes));
    let dir = tree.join("packs");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    write_atomic(&dir.join(format!("{sha}.json")), bytes);
    Ok(sha)
}

/// Deletes the files of `<tree>/packs/` that no `case.json` of the tree names.
pub(crate) fn prune_packs(tree: &Path) -> Result<(), String> {
    ensure_writable(tree)?;
    let mut named = Vec::new();
    for sub in ["stages", "runs"] {
        for case in case_dirs(&tree.join(sub)) {
            named.push(read_case_json(&case)?.pack_sha256);
        }
    }
    let Ok(entries) = fs::read_dir(tree.join("packs")) else {
        return Ok(());
    };
    for e in entries {
        let path = e.map_err(|e| e.to_string())?.path();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        if !stem.is_some_and(|s| named.contains(&s)) {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The case folders of `dir`, sorted.
pub(crate) fn case_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    cases.sort();
    cases
}

/// A pack made of the bundled files with `files` replaced.
fn derived_pack(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut all = ContentFiles::bundled().clone();
    for (name, bytes) in files {
        assert!(all.insert(name, bytes.clone()), "no content file `{name}`");
    }
    all.export().expect("a derived corpus pack exports")
}

/// The four packs the corpus derives from the bundled one: the run case that plays on each, and
/// the pack file's bytes. Enemy attack at 100 stored (`elites`, `stage_skill`: the bot cannot
/// get out of the way of the bundled hit of 10,000), and a short `time_limit` mode (`elites`,
/// `time_limit_kill`, `time_limit_overtime`).
pub(crate) fn override_packs() -> Vec<(&'static str, Vec<u8>)> {
    let stats = || ("run_stats", common::design_run_stats_json().into_bytes());
    let mode = |json: String| ("time_limit", json.into_bytes());
    vec![
        (
            "elites",
            derived_pack(&[stats(), mode(crate::corpus::elites_time_limit().0)]),
        ),
        ("stage_skill", derived_pack(&[stats()])),
        (
            "time_limit_kill",
            derived_pack(&[mode(crate::corpus::short_time_limit(12_000).0)]),
        ),
        (
            "time_limit_overtime",
            derived_pack(&[mode(crate::corpus::short_time_limit(540).0)]),
        ),
    ]
}

/// The pack file's bytes of the run case `case` that plays on a derived pack.
pub(crate) fn override_pack(case: &str) -> Vec<u8> {
    override_packs()
        .into_iter()
        .find(|(c, _)| *c == case)
        .unwrap_or_else(|| panic!("no derived pack for `{case}`"))
        .1
}

/// Every file of `tree` but its own manifest, as `path -> sha256`, paths relative with `/`.
fn hashes_of(tree: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![tree.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = e.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(tree)
                .map_err(|e| e.to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if rel == "manifest.sha256" {
                continue;
            }
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.insert(rel, to_hex(&sha256(&bytes)));
        }
    }
    Ok(out)
}

/// The manifest text of a frozen tree: `<hex>  <path>\n`, sorted by path.
pub(crate) fn manifest_of(tree: &Path) -> Result<String, String> {
    Ok(hashes_of(tree)?
        .into_iter()
        .map(|(path, hash)| format!("{hash}  {path}\n"))
        .collect())
}

/// A frozen tree is what its manifest says, its pack files are named by their hashes, and every
/// case verifies against the pack it pins, with no bundled fallback.
pub(crate) fn check_frozen_tree(tree: &Path) -> Result<(), String> {
    let label = tree
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let stored = fs::read_to_string(tree.join("manifest.sha256"))
        .map_err(|e| format!("frozen tree {label}: manifest.sha256: {e}"))?;
    let mut listed = BTreeMap::new();
    for line in stored.lines() {
        let (hash, path) = line
            .split_once("  ")
            .ok_or_else(|| format!("frozen tree {label}: bad manifest line `{line}`"))?;
        listed.insert(path.to_string(), hash.to_string());
    }
    let actual = hashes_of(tree)?;
    let mut problems = Vec::new();
    for (path, hash) in &listed {
        match actual.get(path) {
            None => problems.push(format!("frozen tree {label}: {path} is missing")),
            Some(h) if h != hash => problems.push(format!("frozen tree {label}: {path} changed")),
            Some(_) => {}
        }
    }
    for path in actual.keys().filter(|p| !listed.contains_key(*p)) {
        problems.push(format!("frozen tree {label}: {path} was added"));
    }
    if stored != manifest_of(tree)? && problems.is_empty() {
        problems.push(format!(
            "frozen tree {label}: manifest.sha256 is not sorted or has doubles"
        ));
    }
    for (path, hash) in &actual {
        if let Some(name) = path
            .strip_prefix("packs/")
            .and_then(|p| p.strip_suffix(".json"))
        {
            if name != hash {
                problems.push(format!(
                    "frozen tree {label}: {path} is not named by its hash"
                ));
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let mut failures = Vec::new();
    for case in case_dirs(&tree.join("stages")) {
        failures.extend(check_case(&case, &diff_dir()).err());
    }
    for case in case_dirs(&tree.join("runs")) {
        failures.extend(check_run_case(&case, &diff_dir()).err());
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// Test helper: copies a tree to `to` and writes its manifest, with the bundled pack stored when
/// a case pins it (a frozen tree has no bundled fallback).
pub(crate) fn freeze_copy(from: &Path, to: &Path) -> Result<(), String> {
    fn copy(from: &Path, to: &Path) -> Result<(), String> {
        fs::create_dir_all(to).map_err(|e| e.to_string())?;
        for e in fs::read_dir(from).map_err(|e| e.to_string())? {
            let path = e.map_err(|e| e.to_string())?.path();
            let dest = to.join(path.file_name().unwrap());
            if path.is_dir() {
                copy(&path, &dest)?;
            } else {
                fs::copy(&path, &dest).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    copy(from, to)?;
    let bundled = to_hex(&Pack::bundled().sha256());
    let mut pins = Vec::new();
    for sub in ["stages", "runs"] {
        for case in case_dirs(&to.join(sub)) {
            pins.push(read_case_json(&case)?.pack_sha256);
        }
    }
    let file = to.join("packs").join(format!("{bundled}.json"));
    if pins.contains(&bundled) && !file.exists() {
        fs::create_dir_all(to.join("packs")).map_err(|e| e.to_string())?;
        fs::write(&file, Pack::BUNDLED).map_err(|e| e.to_string())?;
    }
    fs::write(to.join("manifest.sha256"), manifest_of(to)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{corpus_dir, runs_dir};
    use stella_rain_core::content_hash::{sha256, to_hex};
    use stella_rain_core::run_replay::{self, RunReplay, RunReplayError};

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("pins-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn copy_files(from: &Path, to: &Path, files: &[&str]) {
        fs::create_dir_all(to).unwrap();
        for f in files {
            fs::copy(from.join(f), to.join(f)).unwrap();
        }
    }

    fn bundled_sha() -> String {
        to_hex(&Pack::bundled().sha256())
    }

    /// A small tree under `<scratch>/<folder>`: the stage case `clear` and the run case `trial`,
    /// both pinning the bundled pack, whose file is stored when `store_bundled`.
    fn tree(name: &str, folder: &str, store_bundled: bool) -> PathBuf {
        let tree = scratch(name).join(folder);
        let stage = tree.join("stages/clear");
        copy_files(
            &corpus_dir().join("clear"),
            &stage,
            &["stage.json", "run.replay", "hashes.bin"],
        );
        let run = tree.join("runs/trial");
        copy_files(&runs_dir().join("trial"), &run, &["run.replay"]);
        let pin = format!("{{\"pack_sha256\":\"{}\"}}\n", bundled_sha());
        fs::write(stage.join("case.json"), &pin).unwrap();
        fs::write(run.join("case.json"), &pin).unwrap();
        if store_bundled {
            fs::create_dir_all(tree.join("packs")).unwrap();
            fs::write(
                tree.join(format!("packs/{}.json", bundled_sha())),
                Pack::BUNDLED,
            )
            .unwrap();
        }
        tree
    }

    fn err_of<T>(r: Result<T, String>) -> String {
        match r {
            Ok(_) => panic!("expected a refusal"),
            Err(e) => e,
        }
    }

    #[test]
    fn a_case_whose_pack_sha_does_not_match_is_refused() {
        // (a) the pinned file has one flipped byte.
        let t = tree("a", "v9", true);
        let file = t.join(format!("packs/{}.json", bundled_sha()));
        let mut bytes = fs::read(&file).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 1;
        fs::write(&file, bytes).unwrap();
        let e = err_of(case_pack(&t.join("runs/trial")));
        assert!(e.contains("trial") && e.contains("pack sha256"), "{e}");

        // (b) a pack file whose name is not the hash of its bytes.
        let t = tree("b", "v9", true);
        let wrong = "0".repeat(64);
        fs::rename(
            t.join(format!("packs/{}.json", bundled_sha())),
            t.join(format!("packs/{wrong}.json")),
        )
        .unwrap();
        let e = err_of(load_pack(&t, &wrong));
        assert!(e.contains(&wrong) && e.contains("pack sha256"), "{e}");

        // (c) case.json pins pack A, the replay was recorded with pack B.
        let t = tree("c", "current", false);
        let other = "1".repeat(64);
        fs::write(
            t.join("runs/trial/case.json"),
            format!("{{\"pack_sha256\":\"{other}\"}}"),
        )
        .unwrap();
        let replay = fs::read(t.join("runs/trial/run.replay")).unwrap();
        let e = err_of(run_case_pack(&t.join("runs/trial"), &replay));
        assert!(e.contains(&other), "{e}");

        // (d) a pin with no file in a frozen tree: no bundled fallback.
        let t = tree("d", "v9", false);
        let e = err_of(case_pack(&t.join("stages/clear")));
        assert!(e.contains("pack sha256") && e.contains("packs"), "{e}");
        // ... while the live tree resolves the bundled pin without a file.
        let t = tree("d2", "current", false);
        assert_eq!(
            case_pack(&t.join("stages/clear")).unwrap().sha256(),
            Pack::bundled().sha256()
        );

        // (e) no case.json, and a field it does not know.
        let t = tree("e", "current", false);
        fs::remove_file(t.join("stages/clear/case.json")).unwrap();
        assert!(err_of(read_case_json(&t.join("stages/clear"))).contains("case.json"));
        fs::write(
            t.join("runs/trial/case.json"),
            format!("{{\"pack_sha256\":\"{}\",\"extra\":1}}", bundled_sha()),
        )
        .unwrap();
        assert!(err_of(read_case_json(&t.join("runs/trial"))).contains("extra"));
    }

    #[test]
    fn a_run_case_plays_its_pinned_pack() {
        let case = runs_dir().join("elites");
        let replay = fs::read(case.join("run.replay")).unwrap();
        let pack = run_case_pack(&case, &replay).unwrap();
        assert_ne!(pack.sha256(), Pack::bundled().sha256());
        run_replay::verify_run_with_pack(&replay, &pack).unwrap();
        assert!(matches!(
            run_replay::verify_run_with_pack(&replay, Pack::bundled()),
            Err(RunReplayError::ContentChanged)
        ));
        let recorded = RunReplay::from_bytes(&replay).unwrap();
        assert_eq!(recorded.content_sha256, to_hex(&pack.sha256()));
    }

    #[test]
    fn every_override_pack_is_a_valid_pack_and_exports_the_same_bytes_twice() {
        let first = override_packs();
        let cases: Vec<_> = first.iter().map(|(c, _)| *c).collect();
        assert_eq!(
            cases,
            [
                "elites",
                "stage_skill",
                "time_limit_kill",
                "time_limit_overtime"
            ]
        );
        for ((case, bytes), (_, again)) in first.iter().zip(override_packs()) {
            assert_eq!(bytes, &again, "{case}: two exports differ");
            Pack::from_bytes(bytes).unwrap_or_else(|e| panic!("{case}: {e:?}"));
            assert_ne!(sha256(bytes), Pack::bundled().sha256(), "{case}");
        }
    }

    #[test]
    fn a_frozen_case_is_never_rewritten() {
        assert!(is_frozen_name("v1") && is_frozen_name("v12") && is_frozen_name("v9"));
        assert!(!is_frozen_name("v0") && !is_frozen_name("current") && !is_frozen_name("v"));
        assert!(!is_frozen_name("v1a") && !is_frozen_name("v01"));
        assert!(ensure_writable(Path::new("tests/corpus/v1/stages/x")).is_err());
        assert!(ensure_writable(Path::new("tests/corpus/v12")).is_err());
        assert!(ensure_writable(Path::new("tests/corpus/current/stages/x")).is_ok());

        let t = tree("frozen-write", "v9", true);
        let before = manifest_bytes(&t);
        assert!(write_case_json(&t.join("stages/clear"), &bundled_sha()).is_err());
        assert!(write_pack_file(&t, b"{}").is_err());
        assert_eq!(before, manifest_bytes(&t), "the frozen tree changed");
    }

    /// Every file of a tree with its bytes, sorted.
    fn manifest_bytes(tree: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![tree.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push((p.clone(), fs::read(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn the_frozen_check_catches_every_edit() {
        let make = |name: &str| {
            let from = tree(name, "current", false);
            let to = from.parent().unwrap().join("v9");
            freeze_copy(&from, &to).unwrap();
            to
        };
        let ok = make("edit-ok");
        check_frozen_tree(&ok).unwrap();
        assert!(ok.join("manifest.sha256").is_file());

        let flip = |t: &Path, rel: &str| {
            let p = t.join(rel);
            let mut b = fs::read(&p).unwrap();
            let i = b.len() / 2;
            b[i] ^= 1;
            fs::write(p, b).unwrap();
        };
        let pack = format!("packs/{}.json", bundled_sha());
        for rel in [
            "stages/clear/stage.json",
            pack.as_str(),
            "runs/trial/run.replay",
        ] {
            let t = make(&format!("edit-flip-{}", rel.replace('/', "-")));
            flip(&t, rel);
            assert!(err_of(check_frozen_tree(&t)).contains(rel), "{rel}");
        }
        let t = make("edit-delete");
        fs::remove_file(t.join("stages/clear/hashes.bin")).unwrap();
        assert!(err_of(check_frozen_tree(&t)).contains("stages/clear/hashes.bin"));
        let t = make("edit-add");
        fs::write(t.join("stages/clear/extra.txt"), b"x").unwrap();
        assert!(err_of(check_frozen_tree(&t)).contains("stages/clear/extra.txt"));
        let t = make("edit-manifest");
        let m = fs::read_to_string(t.join("manifest.sha256")).unwrap();
        let first = m.lines().next().unwrap().to_string();
        let edited = m.replacen(
            &first[..1],
            if first.starts_with('0') { "1" } else { "0" },
            1,
        );
        fs::write(t.join("manifest.sha256"), edited).unwrap();
        let path = first.split("  ").nth(1).unwrap().to_string();
        assert!(err_of(check_frozen_tree(&t)).contains(&path));
    }
}
