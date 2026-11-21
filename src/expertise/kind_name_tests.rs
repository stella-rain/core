//! The per-kind shipped marker (ADR-SIM-9 points 3 and 4): `StyleKindName::ALL` stays complete and
//! in order, and the unshipped kinds are the ones whose rule has not landed.

use super::{Expertise, StyleKindName};
use crate::pack::Pack;
use crate::version::Versions;

/// M2: `ALL` equals, in order, the `type` tags of the schema's kinds.
#[test]
fn the_list_of_kinds_is_the_schemas() {
    let schema = serde_json::to_value(schemars::schema_for!(super::StyleKind)).unwrap();
    let tags: Vec<&str> = schema["oneOf"]
        .as_array()
        .expect("a tagged enum is a oneOf")
        .iter()
        .map(|k| {
            k["properties"]["type"]["const"]
                .as_str()
                .expect("a type tag")
        })
        .collect();
    let all: Vec<&str> = StyleKindName::ALL.iter().map(|n| n.tag()).collect();
    assert_eq!(all, tags);
}

/// The test a rule task edits when it flips an arm of `shipped`.
#[test]
fn the_unshipped_kinds_are_the_three_riders() {
    use StyleKindName::{RepeatProcPct, RepeatRangePct, RepeatSizeStack};
    let unshipped: Vec<StyleKindName> = StyleKindName::ALL
        .into_iter()
        .filter(|n| !n.shipped())
        .collect();
    assert_eq!(unshipped, [RepeatRangePct, RepeatProcPct, RepeatSizeStack]);
}

#[test]
fn the_bundled_pack_names_no_unshipped_kind() {
    let file: serde_json::Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let text = file["expertise"].to_string();
    let expertise = Expertise::parse(text.as_bytes(), Versions::CURRENT).expect("the expertise");
    for w in expertise.weapons() {
        for s in &w.styles {
            let levels = std::iter::once(&s.kinds).chain(
                [&s.upgrade_1, &s.upgrade_2, &s.capstone]
                    .into_iter()
                    .flat_map(|l| l.iter().map(|o| &o.kinds)),
            );
            for kinds in levels {
                for kind in kinds {
                    assert!(kind.name().shipped(), "{:?}", kind.name());
                }
            }
        }
    }
}
