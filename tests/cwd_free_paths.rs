//! A test reaches a repository file through `env!("CARGO_MANIFEST_DIR")`, never through the working
//! directory: the iOS Simulator spawns a test binary without the repository as its working
//! directory, so a relative path misses there while every host run passes.

// The scan reads the crate's tree, which the iOS Simulator run does not have.
#![cfg(not(target_os = "ios"))]

use std::fs;
use std::path::{Path, PathBuf};

/// The repository's top folders a test might reach by a relative path.
const ROOTS: &[&str] = &[
    "tests", "benches", "fuzz", "src", "content", "modes", "schema", "docs", "target", "xtask",
];

/// Folders under `tests/` that hold data, not code.
const SKIPPED_DIRS: &[&str] = &["tests/corpus", "tests/fixtures", "tests/golden"];

/// (file, literal, reason): relative literals that open nothing.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "tests/corpus_pins.rs",
        "tests/corpus/v1/stages/x",
        "component test, opens nothing",
    ),
    (
        "tests/corpus_pins.rs",
        "tests/corpus/v12",
        "component test, opens nothing",
    ),
    (
        "tests/corpus_pins.rs",
        "tests/corpus/current/stages/x",
        "component test, opens nothing",
    ),
];

/// The literals on `text` (from byte `from` on) that name a repository file relative to the
/// working directory: the first argument of a path or `fs` call, or the value of a binding.
fn literals(text: &str, from: usize) -> Vec<String> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(open) = text[at..].find('"').map(|i| at + i) {
        let Some(len) = text[open + 1..].find('"') else {
            break;
        };
        let content = &text[open + 1..open + 1 + len];
        at = open + len + 2;
        if open >= from && is_root_path(content) && is_opened(&text[..open]) {
            found.push(content.to_owned());
        }
    }
    found
}

fn is_root_path(literal: &str) -> bool {
    let rest = literal.strip_prefix("./").unwrap_or(literal);
    ROOTS
        .iter()
        .any(|root| rest.strip_prefix(root).is_some_and(|r| r.starts_with('/')))
}

/// Whether a literal after `before` is handed to a file call or bound to a name.
fn is_opened(before: &str) -> bool {
    let before = before.trim_end();
    let before = before.strip_suffix('&').unwrap_or(before).trim_end();
    const HEADS: &[&str] = &[
        "Path::new(",
        "PathBuf::from(",
        "File::open(",
        "File::create(",
        ".open(",
    ];
    if HEADS.iter().any(|head| before.ends_with(head)) {
        return true;
    }
    if let Some(call) = before.strip_suffix('(')
        && let Some(at) = call.rfind("fs::")
        && call[at + 4..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return true;
    }
    let start = before.trim_start();
    before.ends_with('=')
        && !before.ends_with("==")
        && ["let ", "const ", "static ", "pub const ", "pub static "]
            .iter()
            .any(|kw| start.starts_with(kw))
}

/// Whether one line (comments dropped like `tests/one_source.rs` does) is flagged.
fn flagged(line: &str) -> bool {
    !flagged_in(line).is_empty()
}

/// `(line number, literal)` for every flagged literal in `source`. A literal that starts its line
/// (rustfmt splits a long call) is read against the end of the line before.
fn flagged_in(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut previous = "";
    for (i, line) in source.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let joined = format!("{} {code}", previous.trim_end());
        let from = previous.trim_end().len() + 1;
        let hits = if code.trim_start().starts_with('"') {
            literals(&joined, from)
        } else {
            literals(code, 0)
        };
        out.extend(hits.into_iter().map(|literal| (i + 1, literal)));
        if !code.trim().is_empty() {
            previous = code;
        }
    }
    out
}

fn rust_files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            if !SKIPPED_DIRS.contains(&rel.as_str()) {
                rust_files(&path, root, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") && rel != "tests/cwd_free_paths.rs" {
            out.push(path);
        }
    }
}

fn scanned_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in ["tests", "benches", "fuzz/tests", "fuzz/src", "src"] {
        rust_files(&root.join(dir), root, &mut files);
    }
    files.sort();
    files
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn tests_open_files_through_the_manifest_dir() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sites = Vec::new();
    for path in scanned_files(root) {
        let file = relative(root, &path);
        let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (line, literal) in flagged_in(&source) {
            let allowed = ALLOWED.iter().any(|(f, l, _)| *f == file && *l == literal);
            if !allowed {
                sites.push(format!(
                    "{file}:{line}: a test opens \"{literal}\" relative to the working directory; \
                     build it from env!(\"CARGO_MANIFEST_DIR\") (tests/corpus.rs has runs_dir(), \
                     current_dir(), corpus_dir())"
                ));
            }
        }
    }
    assert!(sites.is_empty(), "{}", sites.join("\n"));
}

#[test]
fn only_the_three_component_literals_are_allowed_and_each_still_occurs() {
    assert_eq!(ALLOWED.len(), 3);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, literal, _) in ALLOWED {
        let source = fs::read_to_string(root.join(file)).unwrap();
        assert!(
            flagged_in(&source).iter().any(|(_, l)| l == literal),
            "{file}: the allowance for \"{literal}\" is stale"
        );
    }
}

#[test]
fn the_scan_flags_a_relative_path_handed_to_a_file_call_or_binding() {
    for line in [
        r#"let p = Path::new("tests/corpus/x");"#,
        r#"std::fs::read("tests/corpus/current/runs/trial/run.replay").unwrap()"#,
        r#"let replay_path = "tests/corpus/a";"#,
        r#"fs::read_to_string("content/rules.json")"#,
        r#"File::open("./docs/tuning.md")"#,
        r#"PathBuf::from("schema/x.json")"#,
        r#"assert!(ensure_writable(Path::new("tests/corpus/v12")).is_err())"#,
        r#"const P: &str = &"src/x.rs";"#,
    ] {
        assert!(flagged(line), "not flagged: {line}");
    }
}

#[test]
fn the_scan_leaves_a_joined_macro_message_or_comment_alone() {
    for line in [
        r#"Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")"#,
        r#"std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/content/weapons.json"))"#,
        r#"include_str!("fixtures/stage_v0.json")"#,
        r#".expect("tests/ is readable")"#,
        r#"// Path::new("tests/x")"#,
    ] {
        assert!(!flagged(line), "flagged: {line}");
    }
}

#[test]
fn a_literal_split_onto_its_own_line_is_read_against_the_call_before() {
    let source = "let bytes = std::fs::read(\n    \"tests/corpus/x\",\n);\nlet ok = 1;\n";
    assert_eq!(flagged_in(source), vec![(2, "tests/corpus/x".to_owned())]);
}
