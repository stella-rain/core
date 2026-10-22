//! `content-pack unpack PACK DIR` (BS1, ADR-131): a pack's files written back into `content/`
//! (and `modes/` beside it), so that exporting them gives the same pack bytes.

// It starts the `content-pack` binary as a host process, which the iOS Simulator run cannot.
#![cfg(not(target_os = "ios"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use stella_rain_core::pack::{self, ContentFiles};

/// A scratch tree for one test, `<tmp>/<name>/content` and `<tmp>/<name>/modes`, removed when
/// it ends.
struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("unpack-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("content")).unwrap();
        Tree(root)
    }

    fn content(&self) -> PathBuf {
        self.0.join("content")
    }

    fn pack(&self, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("in.pack.json");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Every file under the tree, with its bytes.
    fn files(&self) -> Vec<(String, Vec<u8>)> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    let name = path.strip_prefix(root).unwrap().to_string_lossy();
                    out.push((name.replace('\\', "/"), std::fs::read(&path).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &self.0, &mut out);
        out.sort();
        out
    }

    /// The files the layout names, read from the tree.
    fn read(&self) -> ContentFiles {
        let mut files = ContentFiles::default();
        for slot in ContentFiles::LAYOUT {
            let dir = if slot.in_modes { "modes" } else { "content" };
            let path = self.0.join(dir).join(format!("{}.json", slot.name));
            if let Ok(bytes) = std::fs::read(path) {
                assert!(files.insert(slot.name, bytes));
            }
        }
        files
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn unpack(pack: &Path, dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_content-pack"))
        .arg("unpack")
        .arg(pack)
        .arg(dir)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The bundled pack with `edit` applied to its text.
fn edited(edit: impl Fn(&str) -> String) -> Vec<u8> {
    edit(&text(&pack::export_bundled())).into_bytes()
}

#[test]
fn unpacking_the_bundled_pack_and_exporting_gives_its_bytes() {
    let tree = Tree::new("round-trip");
    let bytes = pack::export_bundled();
    let out = unpack(&tree.pack(&bytes), &tree.content());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(tree.read().export().unwrap(), bytes);
    // Each file is the bundled file itself, so a core tree that unpacks its own pack changes
    // nothing.
    for (name, written) in tree.files() {
        if name == "in.pack.json" {
            continue;
        }
        let bundled = std::fs::read(format!("{}/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert_eq!(written, bundled, "{name}");
        assert!(!written.contains(&b'\r'), "{name}");
    }
}

#[test]
fn a_pack_with_one_changed_number_unpacks_and_exports_as_it_was() {
    let tree = Tree::new("changed");
    let bytes = edited(|s| s.replacen("\"step\": 8,", "\"step\": 9,", 1));
    assert_ne!(bytes, pack::export_bundled(), "the edit must change the pack");
    let out = unpack(&tree.pack(&bytes), &tree.content());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(tree.read().export().unwrap(), bytes);
    let run_stats = text(&std::fs::read(tree.content().join("run_stats.json")).unwrap());
    assert!(run_stats.contains("\"step\": 9,"));
}

#[test]
fn an_invalid_pack_is_refused_and_nothing_is_written() {
    let tree = Tree::new("invalid");
    let mut v: Value = serde_json::from_slice(&pack::export_bundled()).unwrap();
    v["introduced"]["rewards"][0]["sim_version"] = json!(1);
    let out = unpack(&tree.pack(&serde_json::to_vec(&v).unwrap()), &tree.content());
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(err.contains("$.introduced.rewards[0].sim_version"), "{err}");
    assert!(out.stdout.is_empty());
    let names: Vec<String> = tree.files().into_iter().map(|f| f.0).collect();
    assert_eq!(names, ["in.pack.json"]);
}

#[test]
fn a_valid_pack_that_export_would_not_write_is_refused_with_a_path() {
    let tree = Tree::new("form");
    // Valid JSON and content, but not the form export writes.
    let bytes = edited(|s| s.replacen("\"sim_version\": 0,", "\"sim_version\":0,", 1));
    assert_ne!(bytes, pack::export_bundled());
    let out = unpack(&tree.pack(&bytes), &tree.content());
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("$"), "{}", text(&out.stderr));
    assert_eq!(tree.files().len(), 1);
    // CRLF is refused the same way.
    let bytes = edited(|s| s.replace('\n', "\r\n"));
    let out = unpack(&tree.pack(&bytes), &tree.content());
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert_eq!(tree.files().len(), 1);
}

#[test]
fn an_optional_file_the_pack_lacks_is_not_written_and_a_stale_one_is_removed() {
    let tree = Tree::new("optional");
    let full = text(&pack::export_bundled());
    let start = full.find("  \"stardust\": ").unwrap();
    let end = full.find("  \"modes\"").unwrap();
    let bytes = format!("{}{}", &full[..start], &full[end..]).into_bytes();
    // A stale stardust.json from an earlier unpack, and a file that is not the pack's at all.
    std::fs::write(tree.content().join("stardust.json"), b"stale\n").unwrap();
    std::fs::write(tree.content().join("notes.txt"), b"mine\n").unwrap();
    let out = unpack(&tree.pack(&bytes), &tree.content());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(!tree.content().join("stardust.json").exists());
    assert_eq!(
        std::fs::read(tree.content().join("notes.txt")).unwrap(),
        b"mine\n"
    );
    assert_eq!(tree.read().export().unwrap(), bytes);
}

#[test]
fn a_bad_command_line_or_an_unreadable_pack_exits_2() {
    let tree = Tree::new("usage");
    for args in [
        &["unpack"][..],
        &["unpack", "a"][..],
        &["unpack", "a", "b", "c"][..],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_content-pack"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
    let out = unpack(&tree.0.join("missing.json"), &tree.content());
    assert_eq!(out.status.code(), Some(2));
    let out = unpack(
        &tree.pack(&pack::export_bundled()),
        &tree.0.join("no-such-dir"),
    );
    assert_eq!(out.status.code(), Some(2));
    let help = Command::new(env!("CARGO_BIN_EXE_content-pack"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(text(&help.stdout).contains("content-pack unpack PACK DIR"));
}
