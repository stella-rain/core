//! The version 1 pack the frozen run cases play on: the rules and content of `sim_version` 1 as
//! they were released, never edited. A test that starts version 1 on purpose (an offers run, a
//! carried star, a recording of the frozen rules) builds from it, not from the bundled files,
//! which the live rules (`sim_version` 2) edit.

use std::sync::OnceLock;
use stella_rain_core::pack::Pack;

/// The pack the version 1 `easy`, `rainy`, `time_limit`, `trial` and `tutorial` cases record.
pub const PACK_SHA256: &str = "32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5";

/// The version 1 pack, read from the frozen corpus.
pub fn pack() -> &'static Pack {
    static PACK: OnceLock<Pack> = OnceLock::new();
    PACK.get_or_init(|| {
        let path = crate::data_root::data_root()
            .join("tests/corpus/v1/packs")
            .join(format!("{PACK_SHA256}.json"));
        Pack::from_bytes(&std::fs::read(path).expect("the version 1 pack is kept"))
            .expect("the version 1 pack loads")
    })
}

/// The version 1 pack with `edit` applied to its JSON before it loads: a test that needs another
/// rewards file, run stats or perk set under the frozen rules builds its content here.
pub fn pack_edited(edit: impl FnOnce(&mut serde_json::Value)) -> Pack {
    let mut value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            crate::data_root::data_root()
                .join("tests/corpus/v1/packs")
                .join(format!("{PACK_SHA256}.json")),
        )
        .expect("the version 1 pack is kept"),
    )
    .expect("the version 1 pack is JSON");
    edit(&mut value);
    Pack::from_bytes(&serde_json::to_vec(&value).unwrap()).expect("the edited pack loads")
}
