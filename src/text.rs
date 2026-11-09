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

    /// The sorted, deduped keys `"<file>/<id>"` that every shown content ID of `pack` needs.
    pub fn shown_keys(pack: &Pack) -> Vec<String> {
        let mut keys = Vec::new();

        // 1. difficulties
        for d in pack.content().difficulties().list() {
            let key = format!("difficulties/{}", d.id.as_str());
            keys.push(key);
        }

        // 2. run_stats
        for s in pack.content().run_stats().stats() {
            let key = format!("run_stats/{}", s.id.as_str());
            keys.push(key);
        }

        // 3. rewards
        for r in pack.content().rewards().rewards() {
            let key = format!("rewards/{}", r.id.as_str());
            keys.push(key);
        }

        // 4. stage_skills
        for s in pack.content().stage_skills().skills() {
            let key = format!("stage_skills/{}", s.id.as_str());
            keys.push(key);
        }

        // 5. perk_sets
        for p in pack.content().perk_sets().sets() {
            let key = format!("perk_sets/{}", p.id.as_str());
            keys.push(key);
        }

        // 6. expertise
        for w in pack.content().expertise().weapons() {
            for style in &w.styles {
                let key = format!("expertise/{}", style.id.as_str());
                keys.push(key);
                for u in style
                    .upgrade_1
                    .iter()
                    .chain(&style.upgrade_2)
                    .chain(&style.capstone)
                {
                    let ukey = format!("expertise/{}", u.id.as_str());
                    keys.push(ukey);
                }
            }
            for c in &w.common {
                let key = format!("expertise/{}", c.id.as_str());
                keys.push(key);
            }
        }

        keys.sort();
        keys.dedup();
        keys
    }

    /// Check that every shown ID in `pack` has a corresponding entry in this table.
    pub fn check_pack(&self, pack: &Pack) -> Result<(), Vec<String>> {
        let missing: Vec<String> = Self::shown_keys(pack)
            .into_iter()
            .filter(|key| !self.entries.contains_key(key))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

/// The locale every missing string falls back to, and the source text (ADR-CONT-8 points 5, 8).
pub const FALLBACK_LOCALE: &str = "en";

/// The primary locale (ADR-CONT-8 point 1); the app, not core, uses it as the default.
pub const PRIMARY_LOCALE: &str = "ko";

/// The locales core ships text for, primary first. A new language adds a code here and a file
/// `content/text.<code>.json` (ADR-CONT-8 point 1).
pub const SHIPPED_LOCALES: [&str; 2] = ["ko", "en"];

/// Whether `code` is a locale code: two or three lowercase ASCII letters.
pub fn is_locale_code(code: &str) -> bool {
    (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase())
}

/// Why a set of text tables was refused. Messages carry locale codes only, never text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextSetError {
    /// A locale code that is not two or three lowercase ASCII letters.
    Locale { code: String },
    /// There is no table for the fallback locale `en`.
    NoFallback,
    /// Two tables for one locale.
    Duplicate { locale: String },
    /// A locale's table was refused.
    Table { locale: String, error: TextError },
}

impl fmt::Display for TextSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locale { code } => {
                write!(f, "`{}` is not a locale code", code.escape_debug())
            }
            Self::NoFallback => {
                write!(f, "no text table for the fallback locale {FALLBACK_LOCALE}")
            }
            Self::Duplicate { locale } => write!(f, "two text tables for locale {locale}"),
            Self::Table { locale, error } => write!(f, "text table {locale}: {error}"),
        }
    }
}

impl std::error::Error for TextSetError {}

/// The text tables of every locale, with the English table always present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSet {
    tables: BTreeMap<String, TextTable>,
}

impl TextSet {
    /// The bundled tables: (locale code, file bytes), primary first.
    pub const BUNDLED: [(&'static str, &'static [u8]); 2] = [
        ("ko", include_bytes!("../content/text.ko.json")),
        ("en", TextTable::BUNDLED),
    ];

    /// The bundled set parsed once.
    pub fn bundled() -> &'static TextSet {
        static BUNDLED: OnceLock<TextSet> = OnceLock::new();
        BUNDLED.get_or_init(|| {
            TextSet::from_files(&Self::BUNDLED).expect("the bundled text tables are valid")
        })
    }

    /// Parse a set from (locale code, untrusted file bytes) pairs; `en` must be among them.
    pub fn from_files(files: &[(&str, &[u8])]) -> Result<Self, TextSetError> {
        let mut tables = BTreeMap::new();
        for (code, bytes) in files {
            if !is_locale_code(code) {
                return Err(TextSetError::Locale {
                    code: (*code).to_string(),
                });
            }
            if tables.contains_key(*code) {
                return Err(TextSetError::Duplicate {
                    locale: (*code).to_string(),
                });
            }
            let table = TextTable::from_bytes(bytes).map_err(|error| TextSetError::Table {
                locale: (*code).to_string(),
                error,
            })?;
            tables.insert((*code).to_string(), table);
        }
        if !tables.contains_key(FALLBACK_LOCALE) {
            return Err(TextSetError::NoFallback);
        }
        Ok(Self { tables })
    }

    /// The locale codes in the set, sorted.
    pub fn locales(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }

    /// One locale's table, with no fallback.
    pub fn table(&self, locale: &str) -> Option<&TextTable> {
        self.tables.get(locale)
    }

    /// The entry for `key` in `locale`, else in English, with the locale it came from. An
    /// unknown locale reads as English; `None` only when English lacks the key.
    pub fn lookup(&self, locale: &str, key: &str) -> Option<(&TextEntry, &str)> {
        if let Some((code, table)) = self.tables.get_key_value(locale)
            && let Some(entry) = table.get(key)
        {
            return Some((entry, code.as_str()));
        }
        let (code, table) = self.tables.get_key_value(FALLBACK_LOCALE)?;
        table.get(key).map(|entry| (entry, code.as_str()))
    }

    /// Title for `file` and `id` in `locale`, falling back to English.
    pub fn title(&self, locale: &str, file: &str, id: &str) -> Option<&str> {
        self.lookup(locale, &format!("{file}/{id}"))
            .map(|(e, _)| e.title.as_str())
    }

    /// Description for `file` and `id` in `locale`, falling back to English.
    pub fn description(&self, locale: &str, file: &str, id: &str) -> Option<&str> {
        self.lookup(locale, &format!("{file}/{id}"))
            .map(|(e, _)| e.description.as_str())
    }

    /// Check that every locale in the set has text for every shown ID of `pack`. The error
    /// holds sorted `"<locale>:<key>"` entries.
    pub fn check_pack(&self, pack: &Pack) -> Result<(), Vec<String>> {
        let mut missing = Vec::new();
        for (locale, table) in &self.tables {
            if let Err(keys) = table.check_pack(pack) {
                missing.extend(keys.into_iter().map(|key| format!("{locale}:{key}")));
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            missing.sort();
            Err(missing)
        }
    }

    /// Check that the set has a table for every `SHIPPED_LOCALES` code; the error lists the
    /// codes it lacks.
    pub fn check_shipped(&self) -> Result<(), Vec<String>> {
        let missing: Vec<String> = SHIPPED_LOCALES
            .iter()
            .filter(|code| !self.tables.contains_key(**code))
            .map(|code| (*code).to_string())
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}
