//! The source of `sim_v1` is the frozen copy of the first rules (ADR-REPL-2 point 3): the corpus
//! `tests/corpus/v1/` guards its outcomes, this guards its text.

use crate::hash::StateHasher;

/// FNV-1a 64 over the files of `src/sim_v1`, sorted by path, each as its path, a zero byte, its
/// length and its content.
const SIM_V1_SOURCE: u64 = 265_515_498_147_215_253;

/// Every source file of `sim_v1`, by path from its folder, sorted. The text is compiled in, so
/// that the test needs no file on a device; `sim_v1` takes no new file but `event.rs`, its own event types, so the list is closed.
const SIM_V1_FILES: &[(&str, &str)] = &[
    ("act.rs", include_str!("../sim_v1/act.rs")),
    ("agents.rs", include_str!("../sim_v1/agents.rs")),
    ("attack_table.rs", include_str!("../sim_v1/attack_table.rs")),
    ("attacks.rs", include_str!("../sim_v1/attacks.rs")),
    ("boss.rs", include_str!("../sim_v1/boss.rs")),
    ("combat.rs", include_str!("../sim_v1/combat.rs")),
    ("dash.rs", include_str!("../sim_v1/dash.rs")),
    ("entity.rs", include_str!("../sim_v1/entity.rs")),
    ("event.rs", include_str!("../sim_v1/event.rs")),
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
        "src/sim_v1 changed: a change to its behaviour needs Kade's call (ADR-REPL-2 point 3); \
         an edit only to keep compiling updates this constant in the same commit and says so in \
         its body"
    );
}

/// The 1-based lines of `text` that reach the live event module: `crate::event` followed by
/// anything that is not part of an identifier, or a `crate::{ .. }` list, over any number of
/// lines, that names `event` at its top level. Line comments are blanked first: a comment may
/// name the live type, only code imports it.
fn live_event_lines(text: &str) -> Vec<usize> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let code: String = text
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = code[from..].find("crate::").map(|i| i + from) {
        let after = at + "crate::".len();
        let rest = &code[after..];
        let hit = if let Some(tail) = rest.strip_prefix("event") {
            !tail.starts_with(is_ident)
        } else if rest.starts_with('{') {
            let (mut depth, mut top) = (0, String::new());
            for c in rest.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ if depth == 1 => top.push(c),
                    _ => top.push(' '),
                }
            }
            top.split(|c: char| !is_ident(c)).any(|w| w == "event")
        } else {
            false
        };
        if hit {
            found.push(code[..at].matches('\n').count() + 1);
        }
        from = after;
    }
    found
}

#[test]
fn the_event_scan_finds_a_live_import() {
    let finds = |text: &str| !live_event_lines(text).is_empty();
    assert!(finds("use crate::event::DomainEvent;"));
    assert!(finds("use crate::{event, fixed};"));
    assert!(finds("use crate::{fixed, event::Press};"));
    assert!(finds(
        "use crate::{\n    fixed::Fx,\n    event::DomainEvent,\n};"
    ));
    assert!(finds("let x = 1;\nuse crate::event;"));
    assert!(!finds("use super::event::DomainEvent;"));
    assert!(!finds("use crate::snapshot::Press;"));
    assert!(!finds("use crate::event_log::Log;"));
    assert!(!finds("use crate::{snapshot::{event_view, Press}, fixed};"));
    assert!(!finds("//! into the live `crate::event::DomainEvent`."));
    assert!(!finds("let x = 1; // was crate::event::Press"));
    assert_eq!(live_event_lines("a\nuse crate::{\n  event,\n};"), [2]);
}

/// The frozen rules own the events they emit (ADR-REPL-2 points 1 to 3): a change
/// to the live event edits `engine/v1_events.rs`, never `sim_v1`.
#[test]
fn sim_v1_imports_nothing_from_the_live_event() {
    let mut found = Vec::new();
    for (path, text) in SIM_V1_FILES {
        for line in live_event_lines(text) {
            found.push(format!("sim_v1/{path}:{line}"));
        }
    }
    assert!(
        found.is_empty(),
        "sim_v1 imports the live event type: {found:?}"
    );
}

/// The offer types `sim_v1` owns copies of, in `sim_v1/views.rs`.
const OFFER_TYPES: [&str; 10] = [
    "OfferOption",
    "OfferView",
    "CandidateView",
    "ConstellationView",
    "ConstellationProgress",
    "ExpertiseOfferView",
    "ExpertiseCandidateView",
    "StatOfferView",
    "OfferKind",
    "ExpertiseOfferKind",
];

/// The 1-based lines of `text` where a `crate::snapshot::` path names one of `OFFER_TYPES`, as the
/// item after it or inside its `{ .. }` list (over any number of lines). Line comments are
/// blanked first.
fn live_offer_lines(text: &str) -> Vec<usize> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let code: String = text
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let prefix = "crate::snapshot::";
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = code[from..].find(prefix).map(|i| i + from) {
        let after = at + prefix.len();
        let rest = &code[after..];
        let named = if rest.starts_with('{') {
            let mut depth = 0;
            let mut group = String::new();
            for c in rest.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => group.push(c),
                }
            }
            group
        } else {
            rest.chars().take_while(|&c| is_ident(c)).collect()
        };
        if named
            .split(|c: char| !is_ident(c))
            .any(|w| OFFER_TYPES.contains(&w))
        {
            found.push(code[..at].matches('\n').count() + 1);
        }
        from = after;
    }
    found
}

#[test]
fn the_offer_scan_finds_a_live_import() {
    let finds = |text: &str| !live_offer_lines(text).is_empty();
    assert!(finds("use crate::snapshot::OfferOption;"));
    assert!(finds("use crate::snapshot::{EntityKind, OfferKind};"));
    assert!(finds(
        "use crate::snapshot::{\n    EntityKind,\n    ExpertiseOfferKind,\n};"
    ));
    assert!(finds("fn f() -> crate::snapshot::OfferView { todo!() }"));
    assert!(!finds("use crate::snapshot::Press;"));
    assert!(!finds("use crate::snapshot::{EntityKind, PatternView};"));
    assert!(!finds("use crate::snapshot::OfferViewer;"));
    assert!(!finds("use crate::snapshot::{ExpertiseOfferKinds};"));
    assert!(!finds("use super::views::OfferKind;"));
    assert!(!finds("// was crate::snapshot::OfferKind"));
    assert_eq!(
        live_offer_lines("a\nuse crate::snapshot::{\n  EntityKind,\n  OfferView,\n};"),
        [2]
    );
}

/// The frozen rules own the offer views they build and read (ADR-REPL-2 points 1 to 3): a change
/// to the live ones edits `engine/v1_events.rs`, never `sim_v1`.
#[test]
fn sim_v1_imports_no_live_offer_type() {
    let mut found = Vec::new();
    for (path, text) in SIM_V1_FILES {
        for line in live_offer_lines(text) {
            found.push(format!("sim_v1/{path}:{line}"));
        }
    }
    assert!(
        found.is_empty(),
        "sim_v1 imports a live offer type: {found:?}"
    );
}
