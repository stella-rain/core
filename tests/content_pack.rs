//! The `content-pack` command line (core#148): `verify` checks a pack as the game does and
//! prints its hash, `export` writes the bundled content as a pack; exit status, output, and
//! that nothing from the pack is repeated back.

// It starts the `content-pack` binary as a host process, which the iOS Simulator run cannot;
// the library it calls is covered by `tests/pack.rs`.
#![cfg(not(target_os = "ios"))]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use stella_rain_core::content_hash::to_hex;
use stella_rain_core::pack::{self, Pack};

/// A scratch file for one test, removed when it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str, bytes: &[u8]) -> Scratch {
        let path = std::env::temp_dir().join(format!("content-pack-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        Scratch(path)
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_content-pack"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn export_writes_the_bundled_pack() {
    let out = run(&["export"], b"");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(out.stdout, Pack::BUNDLED.to_vec());
}

#[test]
fn verify_prints_the_hash_and_sim_version_of_a_valid_pack() {
    let bytes = Pack::BUNDLED.to_vec();
    let sha = to_hex(&Pack::from_bytes(&bytes).unwrap().sha256());
    let expected = format!(
        "sha256: {sha}\nsim_version: 0\n\
         starting_unlocks: 1 weapons, 1 perk_sets, 1 stage_skills, 3 runes\n\
         constellations: 4 sets\n\
         stardust: 2 modes\n\
         rules: crowd_target 200, cap_headroom_pct 30\n"
    );
    let file = Scratch::new("valid.json", &bytes);
    let out = run(&["verify", file.path()], b"");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), expected);
    // From standard input, the same.
    let out = run(&["verify", "-"], &bytes);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), expected);
}

#[test]
fn verify_refuses_an_invalid_pack_with_the_reason_and_without_its_text() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["introduced"]["rewards"][0]["sim_version"] = json!(1);
    v["zq_secret_marker"] = json!("zq_secret_marker");
    let out = run(&["verify", "-"], &serde_json::to_vec(&v).unwrap());
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err = text(&out.stderr);
    assert!(err.starts_with("not valid: not a valid pack"), "{err}");
    assert!(!err.contains("zq_"), "{err}");

    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["introduced"]["rewards"][0]["sim_version"] = json!(1);
    let out = run(&["verify", "-"], &serde_json::to_vec(&v).unwrap());
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(
        err.contains("$.introduced.rewards[0].sim_version: introduced in sim_version 1"),
        "{err}"
    );
}

#[test]
fn verify_refuses_a_pack_past_the_byte_limit() {
    let file = Scratch::new("large.json", &vec![b' '; Pack::MAX_BYTES + 1]);
    let out = run(&["verify", file.path()], b"");
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("the limit is 1048576"));
}

#[test]
fn a_bad_command_line_or_a_missing_file_exits_2() {
    for args in [
        &[][..],
        &["verify"][..],
        &["verify", "a", "b"][..],
        &["export", "a", "b"][..],
        &["sign", "x"][..],
    ] {
        let out = run(args, b"");
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(
            text(&out.stderr).contains("usage: content-pack"),
            "{args:?}"
        );
    }
    let out = run(&["verify", "/no/such/pack.json"], b"");
    assert_eq!(out.status.code(), Some(2));
    assert!(!text(&out.stderr).contains("usage"));
    let out = run(&["--help"], b"");
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).contains("usage: content-pack"));
}

#[test]
fn verify_says_when_a_pack_lists_no_starting_unlocks() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v.as_object_mut().unwrap().remove("starting_unlocks");
    let out = run(&["verify", "-"], &serde_json::to_vec(&v).unwrap());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).ends_with(
            "starting_unlocks: none\nconstellations: 4 sets\nstardust: 2 modes\n\
                 rules: crowd_target 200, cap_headroom_pct 30\n"
        ),
        "{}",
        text(&out.stdout)
    );
}

/// A copy of core's `content/` and `modes/` as `<tmp>/<name>/content` and `<tmp>/<name>/modes`,
/// removed when the test ends.
struct Tree(PathBuf);

impl Tree {
    fn copy(name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("export-dir-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["content", "modes"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(dir);
            for entry in std::fs::read_dir(from).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|e| e == "json") {
                    std::fs::copy(&path, root.join(dir).join(path.file_name().unwrap())).unwrap();
                }
            }
        }
        Tree(root)
    }

    fn content(&self) -> PathBuf {
        self.0.join("content")
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn export_dir(dir: &std::path::Path) -> Output {
    run(&["export", dir.to_str().unwrap()], b"")
}

#[test]
fn export_with_no_dir_writes_the_pack_built_in() {
    let out = run(&["export"], b"");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(out.stdout, Pack::BUNDLED);
}

#[test]
fn export_dir_of_the_core_files_is_the_bundled_pack() {
    let tree = Tree::copy("same");
    let out = export_dir(&tree.content());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(out.stdout, Pack::BUNDLED);
}

#[test]
fn export_dir_builds_the_pack_of_the_files_on_disk() {
    let tree = Tree::copy("edit");
    let path = tree.content().join("run_stats.json");
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.contains("\"step\": 10000"));
    std::fs::write(&path, file.replacen("\"step\": 10000", "\"step\": 9999", 1)).unwrap();
    let out = export_dir(&tree.content());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_ne!(out.stdout, Pack::BUNDLED);
    let before = pack::ContentFiles::from_pack(Pack::BUNDLED).unwrap();
    let after = pack::ContentFiles::from_pack(&out.stdout).unwrap();
    for slot in pack::ContentFiles::LAYOUT {
        assert_eq!(
            before.get(slot.name) == after.get(slot.name),
            slot.name != "run_stats",
            "{}",
            slot.name
        );
    }
    let verified = run(&["verify", "-"], &out.stdout);
    assert_eq!(
        verified.status.code(),
        Some(0),
        "{}",
        text(&verified.stderr)
    );
}

#[test]
fn export_dir_without_a_required_file_is_not_valid() {
    let tree = Tree::copy("missing");
    std::fs::remove_file(tree.content().join("weapons.json")).unwrap();
    let out = export_dir(&tree.content());
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("$.weapons: the file is missing"));
}

#[test]
fn export_dir_without_an_optional_file_leaves_it_out() {
    let tree = Tree::copy("optional");
    std::fs::remove_file(tree.content().join("stardust.json")).unwrap();
    let out = export_dir(&tree.content());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.get("stardust").is_none());
}

#[test]
fn export_dir_that_is_not_a_directory_is_refused() {
    let out = run(&["export", "/no/such/dir"], b"");
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("DIR is not a directory"));
}

#[test]
fn unpack_then_export_of_the_same_dir_round_trips() {
    let tree = Tree::copy("round");
    let pack_file = Scratch::new("round.pack.json", Pack::BUNDLED);
    let unpacked = run(
        &["unpack", pack_file.path(), tree.content().to_str().unwrap()],
        b"",
    );
    assert_eq!(
        unpacked.status.code(),
        Some(0),
        "{}",
        text(&unpacked.stderr)
    );
    let out = export_dir(&tree.content());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(out.stdout, Pack::BUNDLED);
}

#[test]
fn help_names_export_with_a_dir() {
    let out = run(&["--help"], b"");
    assert!(text(&out.stdout).contains("content-pack export [DIR]"));
}
