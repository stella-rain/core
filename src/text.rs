//! UI text table for titles and descriptions outside the hashed content pack (TX1).
//!
//! Untrusted: size limit, strict JSON parsing, no control characters, bounded character lengths.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::pack::Pack;
use crate::run_stats::{MAX_DESCRIPTION_CHARS, MAX_TITLE_CHARS, text_problem};

/// The largest text table read (64 KiB).
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// One text entry holding a title and an optional description.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextEntry {
    /// Name shown in choice panels, 1 to `MAX_TITLE_CHARS` characters.
    pub title: String,
    /// Description in words, 0 to `MAX_DESCRIPTION_CHARS` characters.
    #[serde(default)]
    pub description: String,
}

/// Why a text table was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextError {
    /// Over `MAX_TEXT_BYTES`.
    TooLarge { size: usize, limit: usize },
    /// Not valid JSON, duplicate keys, or unknown fields.
    Parse { line: usize, column: usize },
    /// An entry failed text validation (length or control characters).
    Invalid { key: String, issue: String },
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size, limit } => {
                write!(f, "text table is {size} bytes, over the limit of {limit}")
            }
            Self::Parse { line, column } => {
                write!(f, "text table is not valid at line {line}, column {column}")
            }
            Self::Invalid { key, issue } => write!(f, "entry `{key}` is invalid: {issue}"),
        }
    }
}

impl std::error::Error for TextError {}

/// A table mapping `"<file>/<id>"` to `TextEntry`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TextTable {
    #[serde(deserialize_with = "entries_once")]
    entries: BTreeMap<String, TextEntry>,
}

fn entries_once<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, TextEntry>, D::Error> {
    struct Entries;
    impl<'de> Visitor<'de> for Entries {
        type Value = BTreeMap<String, TextEntry>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a map of text entries")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((key, entry)) = map.next_entry::<String, TextEntry>()? {
                if out.contains_key(&key) {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate key in text table: {key}"
                    )));
                }
                out.insert(key, entry);
            }
            Ok(out)
        }
    }
    d.deserialize_map(Entries)
}

impl TextTable {
    /// The size limit for text files.
    pub const MAX_BYTES: usize = MAX_TEXT_BYTES;

    /// The bundled English text table's bytes.
    pub const BUNDLED: &'static [u8] = include_bytes!("../content/text.en.json");

    /// The bundled English text table parsed once.
    pub fn bundled() -> &'static TextTable {
        static BUNDLED: OnceLock<TextTable> = OnceLock::new();
        BUNDLED.get_or_init(|| {
            TextTable::from_bytes(Self::BUNDLED).expect("the bundled text table is valid")
        })
    }

    /// Parse a text table from untrusted UTF-8 JSON bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TextError> {
        if bytes.len() > Self::MAX_BYTES {
            return Err(TextError::TooLarge {
                size: bytes.len(),
                limit: Self::MAX_BYTES,
            });
        }
        let table: TextTable = serde_json::from_slice(bytes).map_err(|e| TextError::Parse {
            line: e.line(),
            column: e.column(),
        })?;

        for (key, entry) in &table.entries {
            if let Some(problem) = text_problem(&entry.title, 1, MAX_TITLE_CHARS) {
                return Err(TextError::Invalid {
                    key: key.clone(),
                    issue: format!("title: {problem}"),
                });
            }
            if let Some(problem) = text_problem(&entry.description, 0, MAX_DESCRIPTION_CHARS) {
                return Err(TextError::Invalid {
                    key: key.clone(),
                    issue: format!("description: {problem}"),
                });
            }
        }

        Ok(table)
    }

    /// Look up an entry by full key `"<file>/<id>"`.
    pub fn get(&self, key: &str) -> Option<&TextEntry> {
        self.entries.get(key)
    }

    /// Look up an entry by file prefix and ID.
    pub fn get_text(&self, file: &str, id: &str) -> Option<&TextEntry> {
        let key = format!("{file}/{id}");
        self.entries.get(&key)
    }

    /// Title for the given file prefix and ID, if present.
    pub fn title(&self, file: &str, id: &str) -> Option<&str> {
        self.get_text(file, id).map(|e| e.title.as_str())
    }

    /// Description for the given file prefix and ID, if present.
    pub fn description(&self, file: &str, id: &str) -> Option<&str> {
        self.get_text(file, id).map(|e| e.description.as_str())
    }

    /// The number of entries in the table.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The underlying entries map.
    pub fn entries(&self) -> &BTreeMap<String, TextEntry> {
        &self.entries
    }

    /// Check that every shown ID in `pack` has a corresponding entry in this table.
    pub fn check_pack(&self, pack: &Pack) -> Result<(), Vec<String>> {
        let mut missing = Vec::new();

        // 1. difficulties
        for d in pack.content().difficulties().list() {
            let key = format!("difficulties/{}", d.id.0);
            if !self.entries.contains_key(&key) {
                missing.push(key);
            }
        }

        // 2. run_stats
        for s in pack.content().run_stats().stats() {
            let key = format!("run_stats/{}", s.id.0);
            if !self.entries.contains_key(&key) {
                missing.push(key);
            }
        }

        // 3. rewards
        for r in pack.content().rewards().rewards() {
            let key = format!("rewards/{}", r.id.0);
            if !self.entries.contains_key(&key) {
                missing.push(key);
            }
        }

        // 4. stage_skills
        for s in pack.content().stage_skills().skills() {
            let key = format!("stage_skills/{}", s.id.0);
            if !self.entries.contains_key(&key) {
                missing.push(key);
            }
        }

        // 5. perk_sets
        for p in pack.content().perk_sets().sets() {
            let key = format!("perk_sets/{}", p.id.0);
            if !self.entries.contains_key(&key) {
                missing.push(key);
            }
        }

        // 6. expertise
        for w in pack.content().expertise().weapons() {
            for style in &w.styles {
                let key = format!("expertise/{}", style.id.0);
                if !self.entries.contains_key(&key) {
                    missing.push(key);
                }
                for u in style
                    .upgrade_1
                    .iter()
                    .chain(&style.upgrade_2)
                    .chain(&style.capstone)
                {
                    let ukey = format!("expertise/{}", u.id.0);
                    if !self.entries.contains_key(&ukey) {
                        missing.push(ukey);
                    }
                }
            }
            for c in &w.common {
                let key = format!("expertise/{}", c.id.0);
                if !self.entries.contains_key(&key) {
                    missing.push(key);
                }
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            missing.sort();
            missing.dedup();
            Err(missing)
        }
    }
}
