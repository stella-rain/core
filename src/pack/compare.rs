//! Comparison of two content directories, for the numbers-only checks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// An error comparing two content directories with numbers-only comparison (RP2-1, ADR-CONT-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareError {
    /// Command line usage / invalid directory error (exit status 2).
    Usage(String),
    /// Content difference or validation failure (exit status 1).
    NotValid(String),
}

impl std::fmt::Display for CompareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(msg) => write!(f, "{msg}"),
            Self::NotValid(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CompareError {}

fn walk_json(
    dir: &Path,
    root: &Path,
    out: &mut BTreeMap<String, PathBuf>,
) -> Result<(), CompareError> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| {
        CompareError::NotValid(format!("cannot read {}: {}", dir.display(), e.kind()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            CompareError::NotValid(format!(
                "cannot read entry in {}: {}",
                dir.display(),
                e.kind()
            ))
        })?;
        let path = entry.path();
        if path.is_dir() {
            walk_json(&path, root, out)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| CompareError::NotValid(format!("path strip prefix failed: {e}")))?
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, path);
        }
    }
    Ok(())
}

fn collect_json_files(dir: &Path) -> Result<BTreeMap<String, PathBuf>, CompareError> {
    let mut files = BTreeMap::new();
    walk_json(dir, dir, &mut files)?;
    let has_modes = files.keys().any(|k| k.starts_with("modes/"));
    if !has_modes {
        let sibling_modes = dir.join("..").join("modes");
        if sibling_modes.is_dir() {
            let mut modes_files = BTreeMap::new();
            walk_json(&sibling_modes, &sibling_modes, &mut modes_files)?;
            for (rel, path) in modes_files {
                files.insert(format!("modes/{rel}"), path);
            }
        }
    }
    Ok(files)
}

/// Compares two JSON values structurally, returning `Ok(())` if they are identical or differ
/// only in numeric values. Returns `Err(CompareError::NotValid)` on type mismatches, differing
/// strings, booleans, nulls, array lengths, or object keys.
pub fn compare_json_values(
    base: &serde_json::Value,
    head: &serde_json::Value,
    path: &str,
) -> Result<(), CompareError> {
    match (base, head) {
        (serde_json::Value::Number(_), serde_json::Value::Number(_)) => Ok(()),
        (serde_json::Value::String(s1), serde_json::Value::String(s2)) => {
            if s1 == s2 {
                Ok(())
            } else {
                Err(CompareError::NotValid(format!("{path}: string differs")))
            }
        }
        (serde_json::Value::Bool(b1), serde_json::Value::Bool(b2)) => {
            if b1 == b2 {
                Ok(())
            } else {
                Err(CompareError::NotValid(format!("{path}: boolean differs")))
            }
        }
        (serde_json::Value::Null, serde_json::Value::Null) => Ok(()),
        (serde_json::Value::Array(a1), serde_json::Value::Array(a2)) => {
            if a1.len() != a2.len() {
                return Err(CompareError::NotValid(format!(
                    "{path}: array length differs ({} vs {})",
                    a1.len(),
                    a2.len()
                )));
            }
            for (i, (item1, item2)) in a1.iter().zip(a2.iter()).enumerate() {
                compare_json_values(item1, item2, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (serde_json::Value::Object(m1), serde_json::Value::Object(m2)) => {
            if m1.len() != m2.len() {
                return Err(CompareError::NotValid(format!(
                    "{path}: key count differs ({} vs {})",
                    m1.len(),
                    m2.len()
                )));
            }
            for (k, v1) in m1 {
                match m2.get(k) {
                    Some(v2) => compare_json_values(v1, v2, &format!("{path}.{k}"))?,
                    None => {
                        return Err(CompareError::NotValid(format!("{path}: missing key {k:?}")));
                    }
                }
            }
            Ok(())
        }
        _ => Err(CompareError::NotValid(format!("{path}: type differs"))),
    }
}

/// Compares two content directories, exiting 0 if only numeric field values differ.
/// Fails with `CompareError::NotValid` if any key, ID, string, boolean, or array length differs,
/// or if any file cannot be read or parsed.
pub fn compare_numbers_only(base_dir: &Path, head_dir: &Path) -> Result<(), CompareError> {
    if !base_dir.is_dir() {
        return Err(CompareError::Usage("BASE_DIR is not a directory".into()));
    }
    if !head_dir.is_dir() {
        return Err(CompareError::Usage("HEAD_DIR is not a directory".into()));
    }

    let base_files = collect_json_files(base_dir)?;
    let head_files = collect_json_files(head_dir)?;

    if !base_files.keys().eq(head_files.keys()) {
        let missing_in_head: Vec<_> = base_files
            .keys()
            .filter(|k| !head_files.contains_key(*k))
            .collect();
        let extra_in_head: Vec<_> = head_files
            .keys()
            .filter(|k| !base_files.contains_key(*k))
            .collect();
        return Err(CompareError::NotValid(format!(
            "content files differ: missing {missing_in_head:?}, extra {extra_in_head:?}"
        )));
    }

    for (rel_path, base_path) in &base_files {
        let head_path = &head_files[rel_path];
        let base_bytes = std::fs::read(base_path).map_err(|e| {
            CompareError::NotValid(format!("cannot read base {rel_path}: {}", e.kind()))
        })?;
        let head_bytes = std::fs::read(head_path).map_err(|e| {
            CompareError::NotValid(format!("cannot read head {rel_path}: {}", e.kind()))
        })?;

        let base_val: serde_json::Value = serde_json::from_slice(&base_bytes)
            .map_err(|e| CompareError::NotValid(format!("cannot parse base {rel_path}: {e}")))?;
        let head_val: serde_json::Value = serde_json::from_slice(&head_bytes)
            .map_err(|e| CompareError::NotValid(format!("cannot parse head {rel_path}: {e}")))?;

        compare_json_values(&base_val, &head_val, rel_path)?;
    }

    Ok(())
}
