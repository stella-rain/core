//! The `packs.json` manifest (FT1a, D-FETCH): one channel per core minor, so an app finds the
//! content pack built for its own core minor. The compatibility key lives here, not in the pack's
//! bytes, so a core release never moves the pack hash.
//!
//! The manifest is untrusted (ADR-020), read from the network or from the cache alike: a size
//! limit before parsing, `schema` 1 only, strict parsing (unknown fields refused, a channel key
//! twice refused), budgets on the channels, then each channel's fields checked. Anything else
//! fails closed, with a number or an index and never text from the manifest.
//!
//! The file is
//! `{"schema":1,"seq":N,"channels":{"<major>.<minor>":{"tag","pack_sha256","text_sha256","url"}}}`,
//! as content's `release.cjs` writes it. A signature over the file is a detached `.minisig`
//! beside it, so the manifest has no signature field.

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

/// The most channels (placeholder; one per core minor, two or three live at a time).
pub const MAX_CHANNELS: usize = 16;
/// The longest channel key, in bytes (placeholder; `65535.65535` is 11).
pub const MAX_KEY_BYTES: usize = 16;
/// The longest release tag, in bytes (placeholder).
pub const MAX_TAG_BYTES: usize = 64;
/// The longest pack URL, in bytes (placeholder).
pub const MAX_URL_BYTES: usize = 512;

/// The manifest as it sits in the file; `Packs::from_bytes` checks it.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PacksFile {
    /// Always 1.
    pub schema: u64,
    /// Rises with every release; a client never accepts a lower one than it has seen.
    pub seq: u64,
    /// One channel per core minor, keyed `"<major>.<minor>"`, at most 16.
    #[serde(deserialize_with = "channels_once")]
    #[schemars(with = "BTreeMap<String, ChannelFile>")]
    pub channels: Vec<(String, ChannelFile)>,
}

/// One channel as it sits in the file.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelFile {
    /// The release tag of the pack, such as `v0.2.0`.
    pub tag: String,
    /// SHA-256 of the pack's bytes: 64 lowercase hex digits.
    pub pack_sha256: String,
    /// SHA-256 of the pack's text content: 64 lowercase hex digits, or null until it is made.
    #[serde(default)]
    pub text_sha256: Option<String>,
    /// Where the pack downloads from: `https://`, at most 512 bytes.
    pub url: String,
}

/// The channels in file order, refusing a key that appears twice (a map would keep the last).
fn channels_once<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, ChannelFile)>, D::Error> {
    struct Channels;
    impl<'de> Visitor<'de> for Channels {
        type Value = Vec<(String, ChannelFile)>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an object of channels")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut out: Vec<(String, ChannelFile)> = Vec::new();
            while let Some((key, channel)) = map.next_entry::<String, ChannelFile>()? {
                if out.iter().any(|(k, _)| *k == key) {
                    return Err(serde::de::Error::custom("a channel key twice"));
                }
                out.push((key, channel));
                // Over the budget: `from_bytes` needs only to know that, so stop collecting.
                if out.len() > MAX_CHANNELS {
                    while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                    break;
                }
            }
            Ok(out)
        }
    }
    d.deserialize_map(Channels)
}

/// Why a manifest was refused. Numbers and positions only, never text from the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PacksError {
    /// Over `Packs::MAX_BYTES`; the manifest is not parsed.
    TooLarge { size: usize, limit: usize },
    /// Not JSON, or not the manifest format (unknown or missing fields, wrong types, a channel
    /// key twice).
    Parse { line: usize, column: usize },
    /// A `schema` other than 1, such as a manifest for a newer client.
    UnsupportedSchema { schema: u64 },
    /// More channels than the limit (`found` is at least `limit + 1`).
    TooManyChannels { found: usize, limit: usize },
    /// The key of the channel at `index` (in key order) is not `<digits>.<digits>`.
    ChannelKey { index: usize },
    /// The tag of the channel at `index` is empty, too long or has a character outside
    /// `A-Z a-z 0-9 . _ + -`.
    Tag { index: usize },
    /// A hash of the channel at `index` is not 64 lowercase hex digits.
    Hash { index: usize, field: &'static str },
    /// The URL of the channel at `index` is not `https://` with a host, is over the limit, or
    /// has a character outside printable ASCII.
    Url { index: usize },
}

impl fmt::Display for PacksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacksError::TooLarge { size, limit } => {
                write!(f, "packs.json is {size} bytes, over the limit of {limit}")
            }
            PacksError::Parse { line, column } => {
                write!(f, "packs.json is not valid at line {line}, column {column}")
            }
            PacksError::UnsupportedSchema { schema } => {
                write!(f, "packs.json schema {schema} is not supported")
            }
            PacksError::TooManyChannels { found, limit } => {
                write!(
                    f,
                    "packs.json has {found} channels, over the limit of {limit}"
                )
            }
            PacksError::ChannelKey { index } => {
                write!(
                    f,
                    "packs.json channel {index} has a key that is not major.minor"
                )
            }
            PacksError::Tag { index } => write!(f, "packs.json channel {index} has a bad tag"),
            PacksError::Hash { index, field } => {
                write!(f, "packs.json channel {index} has a bad {field}")
            }
            PacksError::Url { index } => write!(f, "packs.json channel {index} has a bad url"),
        }
    }
}

impl std::error::Error for PacksError {}

/// One channel: the pack built for one core minor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    tag: String,
    pack_sha256: [u8; 32],
    text_sha256: Option<[u8; 32]>,
    url: String,
}

impl Channel {
    /// The release tag of the pack, such as `v0.2.0`.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// SHA-256 of the pack's bytes.
    pub fn pack_sha256(&self) -> &[u8; 32] {
        &self.pack_sha256
    }

    /// SHA-256 of the pack's text content, when the release made it.
    pub fn text_sha256(&self) -> Option<&[u8; 32]> {
        self.text_sha256.as_ref()
    }

    /// Where the pack downloads from; always `https://`.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// A checked `packs.json`: a sequence number and the channels by core minor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packs {
    seq: u64,
    channels: BTreeMap<String, Channel>,
}

impl Packs {
    /// The most bytes of a manifest (64 KiB), checked before parsing.
    pub const MAX_BYTES: usize = 64 * 1024;

    /// Reads a manifest from untrusted bytes, or says why not.
    pub fn from_bytes(bytes: &[u8]) -> Result<Packs, PacksError> {
        if bytes.len() > Packs::MAX_BYTES {
            return Err(PacksError::TooLarge {
                size: bytes.len(),
                limit: Packs::MAX_BYTES,
            });
        }
        let parse_error = |e: serde_json::Error| PacksError::Parse {
            line: e.line(),
            column: e.column(),
        };
        // The schema first: a manifest for a newer client may have fields this one rejects.
        #[derive(Deserialize)]
        struct Schema {
            schema: u64,
        }
        let Schema { schema } = serde_json::from_slice(bytes).map_err(parse_error)?;
        if schema != 1 {
            return Err(PacksError::UnsupportedSchema { schema });
        }
        let file: PacksFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        if file.channels.len() > MAX_CHANNELS {
            return Err(PacksError::TooManyChannels {
                found: file.channels.len(),
                limit: MAX_CHANNELS,
            });
        }
        // Indexes count in key order, which is the order `release.cjs` writes.
        let mut sorted = file.channels;
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut channels = BTreeMap::new();
        for (index, (key, c)) in sorted.into_iter().enumerate() {
            if !is_channel_key(&key) {
                return Err(PacksError::ChannelKey { index });
            }
            if !is_tag(&c.tag) {
                return Err(PacksError::Tag { index });
            }
            let hash = |field, text: &str| hex32(text).ok_or(PacksError::Hash { index, field });
            let pack_sha256 = hash("pack_sha256", &c.pack_sha256)?;
            let text_sha256 = match &c.text_sha256 {
                Some(text) => Some(hash("text_sha256", text)?),
                None => None,
            };
            if !is_url(&c.url) {
                return Err(PacksError::Url { index });
            }
            channels.insert(
                key,
                Channel {
                    tag: c.tag,
                    pack_sha256,
                    text_sha256,
                    url: c.url,
                },
            );
        }
        Ok(Packs {
            seq: file.seq,
            channels,
        })
    }

    /// The sequence number; a client never accepts a lower one than it has seen.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// The channel for a core minor (`CORE_MINOR`), or `None`: an app built for 0.2 has no
    /// use for a 0.3 channel.
    pub fn channel(&self, minor: &str) -> Option<&Channel> {
        self.channels.get(minor)
    }

    /// The channels by key, in key order.
    pub fn channels(&self) -> impl Iterator<Item = (&str, &Channel)> {
        self.channels.iter().map(|(k, c)| (k.as_str(), c))
    }
}

/// `<digits>.<digits>`, ASCII, at most `MAX_KEY_BYTES`.
fn is_channel_key(key: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    key.len() <= MAX_KEY_BYTES
        && key
            .split_once('.')
            .is_some_and(|(a, b)| digits(a) && digits(b))
}

fn is_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= MAX_TAG_BYTES
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

/// `https://` and a host, printable ASCII without spaces, within `MAX_URL_BYTES`.
fn is_url(url: &str) -> bool {
    url.len() <= MAX_URL_BYTES
        && url.bytes().all(|b| b.is_ascii_graphic())
        && url
            .strip_prefix("https://")
            .is_some_and(|rest| !rest.is_empty())
}

/// 64 lowercase hex digits as 32 bytes.
fn hex32(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        *byte = nibble(pair[0])? << 4 | nibble(pair[1])?;
    }
    Some(out)
}
