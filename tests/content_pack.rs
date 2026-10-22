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
    assert_eq!(out.stdout, pack::export_bundled());
}

#[test]
fn verify_prints_the_hash_and_sim_version_of_a_valid_pack() {
    let bytes = pack::export_bundled();
    let sha = to_hex(&Pack::from_bytes(&bytes).unwrap().sha256());
    let expected = format!(
        "sha256: {sha}\nsim_version: 0\n\
         starting_unlocks: 1 weapons, 1 perk_sets, 1 stage_skills, 3 runes\n\
         constellations: 4 sets\n"
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
    let mut v: Value = serde_json::from_slice(&pack::export_bundled()).unwrap();
    v["introduced"]["rewards"][0]["sim_version"] = json!(1);
    v["zq_secret_marker"] = json!("zq_secret_marker");
    let out = run(&["verify", "-"], &serde_json::to_vec(&v).unwrap());
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err = text(&out.stderr);
    assert!(err.starts_with("not valid: not a valid pack"), "{err}");
    assert!(!err.contains("zq_"), "{err}");

    let mut v: Value = serde_json::from_slice(&pack::export_bundled()).unwrap();
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
        &["export", "x"][..],
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
    let mut v: Value = serde_json::from_slice(&pack::export_bundled()).unwrap();
    v.as_object_mut().unwrap().remove("starting_unlocks");
    let out = run(&["verify", "-"], &serde_json::to_vec(&v).unwrap());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).ends_with("starting_unlocks: none\nconstellations: 4 sets\n"),
        "{}",
        text(&out.stdout)
    );
}
