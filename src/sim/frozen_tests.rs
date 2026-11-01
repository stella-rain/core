//! The source of `sim_v1` is the frozen copy of the first rules (ADR-REPL-9 point 1): the corpus
//! `tests/corpus/v1/` guards its outcomes, this guards its text.

use crate::hash::StateHasher;
use std::path::{Path, PathBuf};

/// FNV-1a 64 over the files of `src/sim_v1`, sorted by path, each as its path, a zero byte, its
/// length and its content.
const SIM_V1_SOURCE: u64 = 1_179_136_501_895_703_855;

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("src/sim_v1 is readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn sim_v1_is_the_frozen_copy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sim_v1");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    let mut named: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(&root).expect("under src/sim_v1");
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            (rel, p)
        })
        .collect();
    named.sort();
    let mut h = StateHasher::new();
    for (rel, path) in &named {
        let bytes = std::fs::read(path).expect("a source file is readable");
        h.write_bytes(rel.as_bytes());
        h.write_u8(0);
        h.write_u64(bytes.len() as u64);
        h.write_bytes(&bytes);
    }
    assert_eq!(
        h.finish(),
        SIM_V1_SOURCE,
        "src/sim_v1 changed: a change to its behaviour needs Kade's call (ADR-REPL-9 point 1); \
         an edit only to keep compiling updates this constant in the same commit and says so in \
         its body"
    );
}
