//! Identifiers shared by the stage, event and snapshot types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

/// A content ID from the registry (ADR-026): asset, palette, part, preset, status or pattern.
/// Lowercase `snake_case`, at most 32 characters; the validator checks the format and that the
/// ID exists.
//
// It holds a shared string: cloning one is a refcount bump, so an event or a snapshot that names
// a content ID allocates nothing. Read it with `as_str`, `Deref` or `Display`; build it with
// `ContentId::from`. Its order, equality and JSON are the string's. (A plain comment: a doc
// comment would reach the generated schema.)
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ContentId(Arc<str>);

impl ContentId {
    /// The ID as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ContentId {
    fn from(s: &str) -> Self {
        ContentId(Arc::from(s))
    }
}

impl From<String> for ContentId {
    fn from(s: String) -> Self {
        ContentId(Arc::from(s))
    }
}

impl Deref for ContentId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ContentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Sound because the derived `Eq` and `Ord` are `Arc<str>`'s, which are `str`'s.
impl Borrow<str> for ContentId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::ContentId;
    use std::collections::BTreeMap;

    #[test]
    fn a_map_of_ids_is_searched_by_str() {
        let mut map = BTreeMap::new();
        map.insert(ContentId::from("stun"), 1u8);
        assert_eq!(map.get("stun"), Some(&1));
        assert_eq!(map.get("slow"), None);
    }

    #[test]
    fn ids_sort_as_the_strings_do() {
        let texts = ["b", "a_1", "a", "ab", "é", "10", "9"];
        let mut ids: Vec<ContentId> = texts.iter().map(|t| ContentId::from(*t)).collect();
        ids.sort();
        let mut strings: Vec<String> = texts.iter().map(|t| (*t).to_owned()).collect();
        strings.sort();
        let sorted: Vec<&str> = ids.iter().map(ContentId::as_str).collect();
        assert_eq!(sorted, strings);
    }

    #[test]
    fn an_id_reads_as_its_string() {
        assert_eq!(format!("{}", ContentId::from("x")), "x");
        assert_eq!(ContentId::from("x"), ContentId::from(String::from("x")));
        assert_eq!(ContentId::from("abc").len(), 3);
        assert_eq!(
            serde_json::to_string(&ContentId::from("x")).unwrap(),
            "\"x\""
        );
    }
}

/// An entity in a run, numbered in spawn order. Agents are evaluated in this order (ADR-036).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct EntityId(pub u32);
