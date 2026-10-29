//! The pack each corpus case pins (`case.json`), the packs it names (`packs/<sha256>.json`) and
//! the frozen trees (`tests/corpus/v<N>/`, N >= 1), which are never written and carry a
//! `manifest.sha256` (ADR-SIM-3). `corpus.rs` plays the cases; this file loads what they pin,
//! writes it, and checks a frozen tree.

use std::fs;
use std::path::{Path, PathBuf};

use stella_rain_core::pack::Pack;

/// `case.json`: the pack a case plays on, by the SHA-256 of the pack file's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaseJson {
    pub pack_sha256: String,
    pub note: Option<String>,
}

pub(crate) fn read_case_json(_case: &Path) -> Result<CaseJson, String> {
    Err("not implemented".into())
}

/// The pack `pin` names in `tree` (`<tree>/packs/<pin>.json`, its name checked against its
/// bytes). Only a tree that is not frozen resolves the bundled pack's hash without a file.
pub(crate) fn load_pack(_tree: &Path, _pin: &str) -> Result<Pack, String> {
    Err("not implemented".into())
}

/// The pack of the case folder `case` (`<tree>/stages/<name>` or `<tree>/runs/<name>`).
pub(crate) fn case_pack(_case: &Path) -> Result<Pack, String> {
    Err("not implemented".into())
}

/// The pack of a run case, which must also be the one its `run.replay` was recorded with.
pub(crate) fn run_case_pack(_case: &Path, _replay_bytes: &[u8]) -> Result<Pack, String> {
    Err("not implemented".into())
}

/// A frozen folder is `v<N>` with N >= 1, digits only.
pub(crate) fn is_frozen_name(_name: &str) -> bool {
    false
}

/// Refuses a path with a frozen folder in it.
pub(crate) fn ensure_writable(_dir: &Path) -> Result<(), String> {
    Err("not implemented".into())
}

pub(crate) fn write_case_json(_case: &Path, _pack_sha256: &str) -> Result<(), String> {
    Err("not implemented".into())
}

/// Writes `packs/<sha256>.json` in `tree`; returns the hash.
pub(crate) fn write_pack_file(_tree: &Path, _bytes: &[u8]) -> Result<String, String> {
    Err("not implemented".into())
}

/// The four packs the corpus derives from the bundled one: the run case that plays on each, and
/// the pack file's bytes.
pub(crate) fn override_packs() -> Vec<(&'static str, Vec<u8>)> {
    Vec::new()
}

/// The manifest text of a frozen tree.
pub(crate) fn manifest_of(_tree: &Path) -> Result<String, String> {
    Err("not implemented".into())
}

/// A frozen tree is what its manifest says, its pack files are named by their hashes, and every
/// case verifies against the pack it pins, with no bundled fallback.
pub(crate) fn check_frozen_tree(_tree: &Path) -> Result<(), String> {
    Err("not implemented".into())
}

/// Test helper: copies a tree to `to` and writes its manifest, with the bundled pack stored.
pub(crate) fn freeze_copy(_from: &Path, _to: &Path) -> Result<(), String> {
    Err("not implemented".into())
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
