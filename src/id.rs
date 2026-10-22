//! Identifiers shared by the stage, event and snapshot types.

use std::borrow::Cow;
use std::sync::Arc;

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

/// A kind id (`EntityView::kind_id`): a content ID shared by reference, so that naming the kind
/// of the target in every `Hit` costs a count, not an allocation (ADR-058). In JSON and in the
/// schema it is a `ContentId`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KindId(Arc<str>);

impl KindId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for KindId {
    fn from(s: &str) -> Self {
        KindId(Arc::from(s))
    }
}

impl From<&ContentId> for KindId {
    fn from(id: &ContentId) -> Self {
        KindId::from(id.0.as_str())
    }
}

impl From<KindId> for ContentId {
    fn from(id: KindId) -> Self {
        ContentId(id.0.to_string())
    }
}

impl PartialEq<ContentId> for KindId {
    fn eq(&self, other: &ContentId) -> bool {
        *self.0 == *other.0
    }
}

impl Serialize for KindId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
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
