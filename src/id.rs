//! Identifiers shared by the stage, event and snapshot types.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A content ID from the registry (ADR-026): asset, palette, part, preset, status or pattern.
/// Lowercase `snake_case`, at most 32 characters; the validator checks the format and that the
/// ID exists.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ContentId(pub String);

impl From<&str> for ContentId {
    fn from(s: &str) -> Self {
        ContentId(s.to_owned())
    }
}

/// The longest content ID (ADR-026): the validator refuses a longer one.
const MAX_ID_BYTES: usize = 32;

/// A kind id (`EntityView::kind_id`): a content ID held inline, so that naming the kind of the
/// target in every `Hit` is a copy of 33 bytes: no allocation, no reference count, nothing to
/// drop (ADR-058). In JSON and in the schema it is a `ContentId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KindId {
    len: u8,
    bytes: [u8; MAX_ID_BYTES],
}

impl KindId {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

impl From<&str> for KindId {
    /// A validated ID fits; a longer string (which no validated stage holds) is cut at the last
    /// whole character that does.
    fn from(s: &str) -> Self {
        let mut len = s.len().min(MAX_ID_BYTES);
        while !s.is_char_boundary(len) {
            len -= 1;
        }
        let mut bytes = [0; MAX_ID_BYTES];
        bytes[..len].copy_from_slice(&s.as_bytes()[..len]);
        KindId {
            len: u8::try_from(len).unwrap_or(0),
            bytes,
        }
    }
}

impl From<&ContentId> for KindId {
    fn from(id: &ContentId) -> Self {
        KindId::from(id.0.as_str())
    }
}

impl From<KindId> for ContentId {
    fn from(id: KindId) -> Self {
        ContentId(id.as_str().to_owned())
    }
}

impl PartialEq<ContentId> for KindId {
    fn eq(&self, other: &ContentId) -> bool {
        self.as_str() == other.0
    }
}

impl Serialize for KindId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for KindId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(KindId::from(String::deserialize(deserializer)?.as_str()))
    }
}

impl JsonSchema for KindId {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        ContentId::schema_name()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        generator.subschema_for::<ContentId>()
    }
}

/// An entity in a run, numbered in spawn order. Agents are evaluated in this order (ADR-036).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct EntityId(pub u32);
