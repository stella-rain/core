//! The bundled content as it stood before the weapons' content moved: the placeholder styles
//! `crush quake bulwark` / `edge tempo reach`, the base trees' AAA charge and hold, the level
//! table with the style at level 3, no `try_styles`, no Star Chart style rows. A hand test that
//! patches or reads the bundled styles or base trees reads this copy instead, so it holds on
//! today's content and on the content that replaces it. Test code only: nothing here is content
//! and nothing reads a generated set. `pack_value` applies five patches to `Pack::BUNDLED`, each a
//! no-op on a pack that already has the old content (ADR-CONT-3, ADR-SIM-9, ADR-SIM-3).

use serde_json::{Value, json};
use std::sync::OnceLock;
use stella_rain_core::engine::Engine;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::validate::Error;

/// `content/expertise.json` before the move: the bytes of its placeholder styles.
pub const EXPERTISE: &[u8] = include_bytes!("fixtures/expertise_before.json");

/// The pack's `introduced.expertise` table for those IDs, each at the pack's own `sim_version`
/// (a new ID must carry it, whether or not the bundled table lists the ID).
const INTRODUCED_EXPERTISE: &[u8] = include_bytes!("fixtures/introduced_expertise_before.json");

/// Each weapon's AAA step's `charge` (Sword) or `hold` (Hammer) as `content/weapons.json` has it
/// before the move.
const WEAPONS_AAA: &[u8] = include_bytes!("fixtures/weapons_aaa_before.json");

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("a fixture is JSON")
}

/// The bundled pack as JSON with the old content patched in.
#[allow(dead_code)]
pub fn pack_value() -> Value {
    let mut v: Value = json_of(Pack::BUNDLED);
    v["expertise"] = json_of(EXPERTISE);
    let sim_version = v["sim_version"].clone();
    let mut introduced = json_of(INTRODUCED_EXPERTISE);
    for weapon in introduced.as_array_mut().expect("a list").iter_mut() {
        for id in weapon["ids"].as_array_mut().expect("a list").iter_mut() {
            id["sim_version"] = sim_version.clone();
        }
    }
    v["introduced"]["expertise"] = introduced;
    let aaa = json_of(WEAPONS_AAA);
    for weapon in v["weapons"]["weapons"].as_array_mut().expect("a list") {
        let id = weapon["id"].as_str().expect("an ID").to_owned();
        let step =
            &mut weapon["trees"][0]["weapon"]["combo"]["main"]["next"]["main"]["next"]["main"];
        let step = step.as_object_mut().expect("the AAA step");
        step.remove("charge");
        step.remove("hold");
        if let Some(own) = aaa[&id].as_object() {
            step.extend(own.clone());
        }
    }
    for mode in ["trial", "time_limit", "tutorial"] {
        let m = &mut v["modes"][mode];
        m["level_table"] = json!({ "style": 3, "upgrades": [6, 9], "capstone": 10 });
        m.as_object_mut().expect("a mode").remove("try_styles");
    }
    v["star_chart"]
        .as_object_mut()
        .expect("the star chart")
        .remove("styles");
    v
}

/// `pack_value` as the bytes of a pack.
#[allow(dead_code)]
pub fn pack_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| serde_json::to_vec(&pack_value()).expect("the pack serialises"))
}

/// `pack_value` as a loaded pack.
#[allow(dead_code)]
pub fn pack() -> &'static Pack {
    static PACK: OnceLock<Pack> = OnceLock::new();
    PACK.get_or_init(|| Pack::from_bytes(pack_bytes()).expect("the old-content pack loads"))
}

/// The pack's run content.
#[allow(dead_code)]
pub fn content() -> &'static RunContent {
    pack().content()
}

/// The old styles, parsed.
#[allow(dead_code)]
pub fn expertise() -> &'static Expertise {
    content().expertise()
}

/// `Engine::start` on the old content.
#[allow(dead_code)]
pub fn start(setup: &RunSetup) -> Result<Engine, Error> {
    Engine::start_with(setup, content())
}

/// A stage's weapon holding the old base tree of `id` inline, as the weapons file stores it.
#[allow(dead_code)]
pub fn inline_weapon(id: &str) -> Value {
    let v = pack_value();
    let weapon = v["weapons"]["weapons"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|w| w["id"] == id)
        .unwrap_or_else(|| panic!("no weapon {id}"));
    json!({ "inline": weapon["trees"][0]["weapon"] })
}
