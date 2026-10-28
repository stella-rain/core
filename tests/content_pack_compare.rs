//! Tests for `content-pack compare --numbers-only BASE_DIR HEAD_DIR` (RP2-1, ADR-CONT-5).
//!
//! Exit status:
//! - 0: valid numbers-only difference (or identical).
//! - 1: non-numeric difference (changed string, ID, boolean, array length, added/removed key,
//!   or unparseable file).
//! - 2: command line usage error.

#![cfg(not(target_os = "ios"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct CompareTree(PathBuf);

impl CompareTree {
    fn new(name: &str) -> CompareTree {
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("compare-{}-{name}-{count}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        CompareTree(root)
    }

    fn write_file(&self, rel_path: &str, content: &str) {
        let path = self.0.join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for CompareTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_content_pack(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_content-pack"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn compare_numbers_only_succeeds_when_only_numbers_change() {
    let base = CompareTree::new("num-base");
    let head = CompareTree::new("num-head");

    base.write_file(
        "run_stats.json",
        r#"{"stats": [{"id": "hp", "base": 100, "multiplier": 1.5}]}"#,
    );
    head.write_file(
        "run_stats.json",
        r#"{"stats": [{"id": "hp", "base": 120, "multiplier": 2.25}]}"#,
    );

    let out = run_content_pack(&[
        "compare",
        "--numbers-only",
        base.path().to_str().unwrap(),
        head.path().to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn compare_numbers_only_fails_when_string_changes() {
    let base = CompareTree::new("str-base");
    let head = CompareTree::new("str-head");

    base.write_file("rewards.json", r#"{"title": "Speed Up", "value": 10}"#);
    head.write_file("rewards.json", r#"{"title": "Haste", "value": 10}"#);

    let out = run_content_pack(&[
        "compare",
        "--numbers-only",
        base.path().to_str().unwrap(),
        head.path().to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for string change"
    );
}

#[test]
fn compare_numbers_only_fails_when_id_added_or_changed() {
    let base = CompareTree::new("id-base");
    let head = CompareTree::new("id-head");

    base.write_file("run_stats.json", r#"{"stats": [{"id": "hp"}]}"#);
    head.write_file(
        "run_stats.json",
        r#"{"stats": [{"id": "hp"}, {"id": "speed"}]}"#,
    );

    let out = run_content_pack(&[
        "compare",
        "--numbers-only",
        base.path().to_str().unwrap(),
        head.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1), "expected exit 1 for added ID");
}

#[test]
fn compare_numbers_only_fails_when_boolean_changes() {
    let base = CompareTree::new("bool-base");
    let head = CompareTree::new("bool-head");

    base.write_file("rules.json", r#"{"enabled": true, "cost": 5}"#);
    head.write_file("rules.json", r#"{"enabled": false, "cost": 5}"#);

    let out = run_content_pack(&[
        "compare",
        "--numbers-only",
        base.path().to_str().unwrap(),
        head.path().to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for boolean change"
    );
}

#[test]
fn compare_numbers_only_fails_when_array_length_changes() {
    let base = CompareTree::new("arr-base");
    let head = CompareTree::new("arr-head");

    base.write_file("perk_sets.json", r#"{"items": [1, 2]}"#);
    head.write_file("perk_sets.json", r#"{"items": [1, 2, 3]}"#);

    let out = run_content_pack(&[
        "compare",
        "--numbers-only",
        base.path().to_str().unwrap(),
        head.path().to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for array length change"
    );
}

#[test]
fn compare_numbers_only_fails_when_key_added_or_removed() {
    let base = CompareTree::new("key-base");
    let head = CompareTree::new("key-head");

    base.write_file("rules.json", r#"{"a": 1}"#);
    head.write_file("rules.json", r#"{"a": 1, "b": 2}"#);

    let out = run_content_pack(&[
        "compare",
        "--numbers-only",
        base.path().to_str().unwrap(),
        head.path().to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for key addition"
    );
}

#[test]
fn compare_usage_error_when_args_invalid() {
    let base = CompareTree::new("usage-base");
    let head = CompareTree::new("usage-head");

    // Missing --numbers-only
    let out = run_content_pack(&[
        "compare",
        base.path().to_str().unwrap(),
        head.path().to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 for missing --numbers-only"
    );

    // Missing directories
    let out = run_content_pack(&["compare", "--numbers-only"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 for missing directories"
    );
}
