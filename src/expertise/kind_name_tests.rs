//! The per-kind shipped marker (ADR-SIM-9 points 3 and 4): `StyleKindName::ALL` stays complete and
//! in order, and the unshipped kinds are the ones whose rule has not landed.

use super::{Expertise, StyleEffectName, StyleKindName};
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

/// The tags of a tagged enum's schema, in order.
fn tags_of<T: schemars::JsonSchema>() -> Vec<String> {
    let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
    schema["oneOf"]
        .as_array()
        .expect("a tagged enum is a oneOf")
        .iter()
        .map(|k| {
            k["properties"]["type"]["const"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

/// The properties of a struct's schema that are not in `old`: the fields a decision is owed for.
fn new_fields<T: schemars::JsonSchema>(old: &[&str]) -> Vec<String> {
    let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
    schema["properties"]
        .as_object()
        .expect("a struct has properties")
        .keys()
        .filter(|k| !old.contains(&k.as_str()))
        .cloned()
        .collect()
}

/// M: `StyleEffectName::ALL` equals, in order, the gauge fills, the spirit's field and effects and
/// the counter bonus's effects as the schema lists them.
#[test]
fn the_list_of_effects_is_the_schemas() {
    let mut want = new_fields::<super::Gauge>(&[
        "max",
        "per_step",
        "per_hit",
        "from_damage_pct",
        "step_cap",
        "spend",
        "smash_cap",
    ]);
    want.sort_by_key(|k| {
        ["per_rune", "outer_half_pct", "per_smash_hit"]
            .iter()
            .position(|t| t == k)
    });
    want.extend(new_fields::<super::Spirit>(&["decay_ticks", "levels"]));
    want.extend(tags_of::<super::SpiritEffect>());
    want.extend(tags_of::<super::CounterEffect>());
    let all: Vec<&str> = StyleEffectName::ALL.iter().map(|n| n.tag()).collect();
    assert_eq!(all, want);
}

/// The test a rule task edits when it flips an arm of `StyleEffectName::shipped`.
#[test]
fn the_unshipped_effects_are_the_new_ones() {
    use StyleEffectName::*;
    let unshipped: Vec<StyleEffectName> = StyleEffectName::ALL
        .into_iter()
        .filter(|n| !n.shipped())
        .collect();
    assert_eq!(
        unshipped,
        [
            GaugePerRune,
            GaugeOuterHalfPct,
            GaugePerSmashHit,
            SpiritRaisePerSmash,
            SpiritSizeStackCap,
            SpiritSizeStackTicksPct,
            SpiritSizeStacksPerHit,
            SpiritNormalPull,
            SpiritRuneCooldownPct,
            SpiritRuneSurge,
            SpiritExtraPresses,
            SpiritSmashRepeat,
            CounterPullRound,
            CounterFillSizeStacks,
            CounterClearBullets,
            CounterFireRunes,
        ]
    );
}

#[test]
fn the_bundled_pack_names_no_unshipped_effect() {
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
                    for (effect, at) in kind.effects() {
                        assert!(effect.shipped(), "{effect:?} at {at}");
                    }
                }
            }
        }
    }
}

/// The once-per-level array of `check_spirit` has a slot for every effect variant.
#[test]
fn the_spirit_slots_cover_every_effect() {
    assert_eq!(
        tags_of::<super::SpiritEffect>().len(),
        super::kind_checks::SPIRIT_EFFECT_KINDS
    );
}
