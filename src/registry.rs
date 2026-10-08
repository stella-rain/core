//! The content-ID registry (ADR-026): every ID a stage may use, with the `sim_version` that
//! introduced it. The validator rejects an ID that is missing, of the wrong kind, or newer than
//! the stage (ADR-017, ADR-020). Under version 0 everything is introduced in 0 and may change
//! in place (ADR-035); the entries are placeholders until the registry file replaces them.
//!
//! Asset sources (ADR-027) join the entries when real assets exist.

/// What an ID names. An ID is only valid where its kind is expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A sprite: the main character, an agent, a boss or a boss part.
    Asset,
    Palette,
    /// A bullet sprite.
    Bullet,
    /// An attack preset (ADR-036); `args` is how many arguments it takes.
    Preset,
    /// A buff or debuff (ADR-036).
    Status,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Active,
    /// Still loads and plays; the editor stops offering it (ADR-026).
    Deprecated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: &'static str,
    pub kind: Kind,
    /// The `sim_version` that introduced the ID; a stage may use it from that version on.
    pub introduced: u32,
    pub availability: Availability,
    /// Arguments an attack preset takes; 0 for every other kind.
    pub args: u8,
}

const fn entry(id: &'static str, kind: Kind, args: u8) -> Entry {
    Entry {
        id,
        kind,
        introduced: 0,
        availability: Availability::Active,
        args,
    }
}

/// Sorted by kind, then ID, with no duplicates (`tests/registry.rs`).
pub const ENTRIES: &[Entry] = &[
    entry("bat", Kind::Asset, 0),
    entry("fairy_a", Kind::Asset, 0),
    entry("golem", Kind::Asset, 0),
    entry("horns_2", Kind::Asset, 0),
    entry("pilot_a", Kind::Asset, 0),
    entry("ice", Kind::Palette, 0),
    entry("orb_small", Kind::Bullet, 0),
    entry("aimed_single", Kind::Preset, 1),
    entry("aimed_spread", Kind::Preset, 1),
    entry("spiral", Kind::Preset, 1),
    entry("spread_5", Kind::Preset, 0),
    entry("twin_shot", Kind::Preset, 0),
    entry("atk_up", Kind::Status, 0),
    entry("slow", Kind::Status, 0),
    entry("vulnerable", Kind::Status, 0),
];

/// Lowercase `snake_case`, 1 to 32 characters (ADR-026). Stage-defined names, such as boss part
/// IDs, follow the same format.
pub fn is_well_formed(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// The entry for `id` of `kind` in `entries`.
pub fn find<'a>(entries: &'a [Entry], kind: Kind, id: &str) -> Option<&'a Entry> {
    entries.iter().find(|e| e.kind == kind && e.id == id)
}

/// The style of a bullet asset for the renderer: 0 for the default bullet, otherwise 1 plus the
/// position of the asset among the bullet assets of `ENTRIES`.
pub fn bullet_style(id: &str) -> u16 {
    ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Bullet)
        .position(|e| e.id == id)
        .map_or(0, |i| u16::try_from(i + 1).unwrap_or(0))
}
