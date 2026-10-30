//! The content-ID registry (ADR-026): every ID a stage may use, with the `sim_version` that
//! introduced it. The validator rejects an ID that is missing, of the wrong kind, or newer than
//! the stage (ADR-017, ADR-020). Every ID here is introduced in version 1, the frozen first
//! version (ADR-REPL-7 point 8); the entries are placeholders until the registry file replaces
//! them.
//!
//! An attack preset's arguments are bounded in code by its registry entry (ADR-CONT-8, ADR-CONT-2).
//!
//! Asset sources (ADR-027) join the entries when real assets exist.

/// What an ID names. An ID is only valid where its kind is expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A sprite: the main character, an agent, a boss or a boss part, or a stage skill's icon
    /// (ADR-098).
    Asset,
    Palette,
    /// An orb sprite.
    Orb,
    /// An attack preset (ADR-036); `args` is how many arguments it takes.
    Preset,
    /// A buff or debuff (ADR-036).
    Status,
    /// A weapon preset, with its Main and Sub roles (ADR-070).
    Weapon,
    /// A game mode: the content a run plays (ADR-051), bundled in `core` (`modes/`).
    Mode,
    /// A permanent upgrade bought with Stardust (ADR-079): the run setup lists the ones the
    /// player holds, and a combo step names the one that opens it (ADR-084 point 7).
    Upgrade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Active,
    /// Still loads and plays; the editor stops offering it (ADR-026).
    Deprecated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgBound {
    pub name: &'static str,
    pub min: i32,
    pub max: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: &'static str,
    pub kind: Kind,
    /// The `sim_version` that introduced the ID; a stage may use it from that version on.
    pub introduced: u32,
    pub availability: Availability,
    /// Arguments an attack preset takes; 0 for every other kind. Preset arguments are bounded in
    /// code (ADR-CONT-8, ADR-CONT-2).
    pub args: u8,
    /// The name, minimum, and maximum of each argument an attack preset takes; empty for every other
    /// kind or arity-0 preset.
    pub arg_bounds: &'static [ArgBound],
}

const fn entry(id: &'static str, kind: Kind, args: u8) -> Entry {
    Entry {
        id,
        kind,
        introduced: 1,
        availability: Availability::Active,
        args,
        arg_bounds: &[],
    }
}

const fn preset(id: &'static str, bounds: &'static [ArgBound]) -> Entry {
    assert!(
        bounds.len() <= u8::MAX as usize,
        "too many arguments for u8 arity"
    );
    #[allow(clippy::cast_possible_truncation)]
    let args = bounds.len() as u8;
    Entry {
        id,
        kind: Kind::Preset,
        introduced: 1,
        availability: Availability::Active,
        args,
        arg_bounds: bounds,
    }
}

pub const MAX_WIND_UP: i32 = 600;

const WIND_UP_BOUNDS: &[ArgBound] = &[ArgBound {
    name: "wind_up_ticks",
    min: 0,
    max: MAX_WIND_UP,
}];

/// Sorted by kind, then ID, with no duplicates (`tests/registry.rs`).
pub const ENTRIES: &[Entry] = &[
    entry("bat", Kind::Asset, 0),
    entry("core_pair", Kind::Asset, 0),
    entry("fairy_a", Kind::Asset, 0),
    entry("golem", Kind::Asset, 0),
    entry("horn_charger", Kind::Asset, 0),
    entry("horns_2", Kind::Asset, 0),
    entry("icon_aegis", Kind::Asset, 0),
    entry("icon_black_hole", Kind::Asset, 0),
    entry("icon_hex", Kind::Asset, 0),
    entry("icon_nova", Kind::Asset, 0),
    entry("icon_war_cry", Kind::Asset, 0),
    entry("lantern_choir", Kind::Asset, 0),
    entry("pilot_a", Kind::Asset, 0),
    entry("stone_warden", Kind::Asset, 0),
    entry("weaver", Kind::Asset, 0),
    entry("ice", Kind::Palette, 0),
    entry("orb_small", Kind::Orb, 0),
    preset("aimed_single", WIND_UP_BOUNDS),
    preset("aimed_spread", WIND_UP_BOUNDS),
    entry("elite_slam", Kind::Preset, 0),
    entry("slam", Kind::Preset, 0),
    entry("slash", Kind::Preset, 0),
    preset("spiral", WIND_UP_BOUNDS),
    entry("spread_5", Kind::Preset, 0),
    entry("thrust", Kind::Preset, 0),
    entry("twin_shot", Kind::Preset, 0),
    entry("atk_up", Kind::Status, 0),
    entry("slow", Kind::Status, 0),
    entry("vulnerable", Kind::Status, 0),
    entry("giant_hammer", Kind::Weapon, 0),
    entry("giant_sword", Kind::Weapon, 0),
    entry("time_limit", Kind::Mode, 0),
    entry("trial", Kind::Mode, 0),
    entry("tutorial", Kind::Mode, 0),
    entry("giant_hammer_aaab", Kind::Upgrade, 0),
    entry("giant_hammer_aab", Kind::Upgrade, 0),
    entry("giant_sword_aaab", Kind::Upgrade, 0),
    entry("giant_sword_aab", Kind::Upgrade, 0),
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

/// The style of an orb asset for the renderer: 0 for the default orb, otherwise 1 plus the
/// position of the asset among the orb assets of `ENTRIES`.
pub fn orb_style(id: &str) -> u16 {
    ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Orb)
        .position(|e| e.id == id)
        .map_or(0, |i| u16::try_from(i + 1).unwrap_or(0))
}

/// The most upgrades a setup names (ADR-079): the registry holds at most this many.
pub const MAX_UPGRADES: usize = 32;
