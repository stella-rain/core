//! The source of `sim_v1` is the frozen copy of the first rules (ADR-REPL-9 point 1): the corpus
//! `tests/corpus/v1/` guards its outcomes, this guards its text.

use crate::hash::StateHasher;

/// FNV-1a 64 over the files of `src/sim_v1`, sorted by path, each as its path, a zero byte, its
/// length and its content.
const SIM_V1_SOURCE: u64 = 18_142_368_067_911_637_289;

/// Every source file of `sim_v1`, by path from its folder, sorted. The text is compiled in, so
/// that the test needs no file on a device; `sim_v1` takes no new file, so the list is closed.
const SIM_V1_FILES: &[(&str, &str)] = &[
    ("act.rs", include_str!("../sim_v1/act.rs")),
    ("agents.rs", include_str!("../sim_v1/agents.rs")),
    ("attack_table.rs", include_str!("../sim_v1/attack_table.rs")),
    ("attacks.rs", include_str!("../sim_v1/attacks.rs")),
    ("boss.rs", include_str!("../sim_v1/boss.rs")),
    ("combat.rs", include_str!("../sim_v1/combat.rs")),
    ("dash.rs", include_str!("../sim_v1/dash.rs")),
    ("entity.rs", include_str!("../sim_v1/entity.rs")),
    ("expertise.rs", include_str!("../sim_v1/expertise.rs")),
    ("grid.rs", include_str!("../sim_v1/grid.rs")),
    (
        "hash_layout_tests.rs",
        include_str!("../sim_v1/hash_layout_tests.rs"),
    ),
    ("heal.rs", include_str!("../sim_v1/heal.rs")),
    ("hit_boxes.rs", include_str!("../sim_v1/hit_boxes.rs")),
    ("horde.rs", include_str!("../sim_v1/horde.rs")),
    ("leash.rs", include_str!("../sim_v1/leash.rs")),
    ("leash_tests.rs", include_str!("../sim_v1/leash_tests.rs")),
    ("levels.rs", include_str!("../sim_v1/levels.rs")),
    ("loadout.rs", include_str!("../sim_v1/loadout.rs")),
    ("mod.rs", include_str!("../sim_v1/mod.rs")),
    ("orbit.rs", include_str!("../sim_v1/orbit.rs")),
    ("player.rs", include_str!("../sim_v1/player.rs")),
    ("push.rs", include_str!("../sim_v1/push.rs")),
    ("runes.rs", include_str!("../sim_v1/runes.rs")),
    ("schedule.rs", include_str!("../sim_v1/schedule.rs")),
    ("scratch.rs", include_str!("../sim_v1/scratch.rs")),
    ("select.rs", include_str!("../sim_v1/select.rs")),
    ("skill.rs", include_str!("../sim_v1/skill.rs")),
    ("snapshot.rs", include_str!("../sim_v1/snapshot.rs")),
    (
        "spawn_edge_tests.rs",
        include_str!("../sim_v1/spawn_edge_tests.rs"),
    ),
    ("stardust.rs", include_str!("../sim_v1/stardust.rs")),
    ("stars.rs", include_str!("../sim_v1/stars.rs")),
    ("state.rs", include_str!("../sim_v1/state.rs")),
    ("state_codec.rs", include_str!("../sim_v1/state_codec.rs")),
    ("state_hash.rs", include_str!("../sim_v1/state_hash.rs")),
    ("stun.rs", include_str!("../sim_v1/stun.rs")),
    ("world.rs", include_str!("../sim_v1/world.rs")),
    ("zones.rs", include_str!("../sim_v1/zones.rs")),
];

#[test]
fn sim_v1_is_the_frozen_copy() {
    let mut h = StateHasher::new();
    for (path, text) in SIM_V1_FILES {
        h.write_bytes(path.as_bytes());
        h.write_u8(0);
        h.write_u64(text.len() as u64);
        h.write_bytes(text.as_bytes());
    }
    assert_eq!(
        h.finish(),
        SIM_V1_SOURCE,
        "src/sim_v1 changed: a change to its behaviour needs Kade's call (ADR-REPL-9 point 1); \
         an edit only to keep compiling updates this constant in the same commit and says so in \
         its body"
    );
}

/// Whether `line` reaches the live event module: `crate::event` followed by anything that is
/// not part of an identifier, or a `crate::{ .. }` list that names `event`.
fn imports_live_event(line: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut rest = line;
    while let Some(at) = rest.find("crate::") {
        let after = &rest[at + "crate::".len()..];
        if let Some(tail) = after.strip_prefix("event") {
            if !tail.starts_with(is_ident) {
                return true;
            }
        }
        if let Some(list) = after.strip_prefix('{') {
            let list = list.split('}').next().unwrap_or("");
            let names_event = list
                .split(|c: char| !is_ident(c))
                .any(|word| word == "event");
            if names_event {
                return true;
            }
        }
        rest = after;
    }
    false
}

#[test]
fn the_event_scan_finds_a_live_import() {
    assert!(imports_live_event("use crate::event::DomainEvent;"));
    assert!(imports_live_event("use crate::{event, fixed};"));
    assert!(imports_live_event("use crate::{fixed, event::Press};"));
    assert!(!imports_live_event("use super::event::DomainEvent;"));
    assert!(!imports_live_event("use crate::snapshot::Press;"));
    assert!(!imports_live_event("use crate::event_log::Log;"));
}

/// The frozen rules own the events they emit (ADR-REPL-9 point 1, ADR-REPL-7 point 3): a change
/// to the live event edits `engine/v1_events.rs`, never `sim_v1`.
#[test]
fn sim_v1_imports_nothing_from_the_live_event() {
    let mut found = Vec::new();
    for (path, text) in SIM_V1_FILES {
        for (n, line) in text.lines().enumerate() {
            if imports_live_event(line) {
                found.push(format!("sim_v1/{path}:{}", n + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "sim_v1 imports the live event type: {found:?}"
    );
}
