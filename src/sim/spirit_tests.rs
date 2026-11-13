//! The spirit level a style grants (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SIM-7): the kind
//! changes no step of the tree, the level rises with the full-gauge smash, falls with a
//! countdown and carries its own effects. Every number is a test value, not content.

use crate::expertise::{Expertise, weapon_with_style};
use crate::pack::Pack;
use crate::version::Versions;
use serde_json::json;

/// S5: a style that names only `spirit` leaves the weapon's tree equal to the base's: the kind
/// changes no step, the rules read it beside the weapon.
#[test]
fn the_spirit_kind_changes_no_step() {
    let content = Pack::bundled().content();
    let base = content
        .weapons()
        .tree("giant_sword", None)
        .expect("the Sword's base tree");
    let bundled: serde_json::Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let mut file = bundled["expertise"].clone();
    let w = file["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_sword")
        .unwrap();
    w["styles"][0]["kinds"] = json!([{
        "type": "spirit", "decay_ticks": 100,
        "levels": [[{ "type": "damage_pct", "pct": 8 }]]
    }]);
    let expertise =
        Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the expertise");
    let style = &expertise
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_sword")
        .unwrap()
        .styles[0];
    for taken in 0..=3 {
        assert_eq!(
            &weapon_with_style(base, style, taken),
            base,
            "taken {taken}"
        );
    }
}
