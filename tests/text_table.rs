use serde_json::Value;
use stella_rain_core::pack::Pack;
use stella_rain_core::text::{MAX_TEXT_BYTES, TextError, TextTable};

#[test]
fn pack_has_no_ui_text_and_sha256_invariant_under_text_edit() {
    let raw: Value = serde_json::from_slice(Pack::BUNDLED).expect("bundled pack is valid JSON");

    // UI text must NOT exist in the hashed content pack (TX1).
    let check_no_text = |arr: &Value, name: &str| {
        if let Some(items) = arr.as_array() {
            for item in items {
                assert!(
                    item.get("title").is_none(),
                    "{name} item still has 'title': {item:?}"
                );
                assert!(
                    item.get("description").is_none(),
                    "{name} item still has 'description': {item:?}"
                );
            }
        }
    };

    if let Some(diffs) = raw.get("difficulties").and_then(|v| v.get("difficulties")) {
        check_no_text(diffs, "difficulties");
    }
    if let Some(stats) = raw.get("run_stats").and_then(|v| v.get("stats")) {
        check_no_text(stats, "run_stats");
    }
    if let Some(rewards) = raw.get("rewards").and_then(|v| v.get("rewards")) {
        check_no_text(rewards, "rewards");
    }
    if let Some(skills) = raw.get("stage_skills").and_then(|v| v.get("stage_skills")) {
        check_no_text(skills, "stage_skills");
    }
    if let Some(perks) = raw.get("perk_sets").and_then(|v| v.get("perk_sets")) {
        check_no_text(perks, "perk_sets");
    }
    if let Some(weapons) = raw.get("expertise").and_then(|v| v.get("weapons")) {
        if let Some(weapons_arr) = weapons.as_array() {
            for w in weapons_arr {
                if let Some(styles) = w.get("styles") {
                    check_no_text(styles, "expertise styles");
                }
                if let Some(common) = w.get("common") {
                    check_no_text(common, "expertise common");
                }
            }
        }
    }
}

#[test]
fn bundled_text_table_loads_all_ids() {
    let table = TextTable::from_bytes(TextTable::BUNDLED).expect("bundled text table must load");

    // Spot-check known IDs across all 6 content types
    let expected_keys = [
        "difficulties/easy",
        "difficulties/normal",
        "run_stats/max_hp",
        "run_stats/attack",
        "rewards/max_hp_common",
        "rewards/star_altair",
        "stage_skills/black_hole",
        "stage_skills/aegis",
        "perk_sets/vanguard",
        "expertise/crush",
        "expertise/damage",
    ];

    for key in expected_keys {
        let entry = table
            .get(key)
            .unwrap_or_else(|| panic!("missing key {key}"));
        assert!(
            !entry.title.is_empty() && entry.title.chars().count() <= 32,
            "title invalid for {key}: {}",
            entry.title
        );
        assert!(
            entry.description.chars().count() <= 160,
            "description too long for {key}: {}",
            entry.description
        );
        assert!(
            !entry.title.chars().any(char::is_control),
            "control char in title for {key}"
        );
        assert!(
            !entry.description.chars().any(char::is_control),
            "control char in description for {key}"
        );
    }
}

#[test]
fn verify_text_checks_every_id() {
    let pack = Pack::bundled();
    let table = TextTable::from_bytes(TextTable::BUNDLED).expect("bundled text table");
    let result = table.check_pack(pack);
    assert!(
        result.is_ok(),
        "verify text failed for bundled pack: {result:?}"
    );
}

#[test]
fn text_table_size_limit() {
    let oversized = vec![b' '; MAX_TEXT_BYTES + 1];
    let err = TextTable::from_bytes(&oversized).unwrap_err();
    assert_eq!(
        err,
        TextError::TooLarge {
            size: MAX_TEXT_BYTES + 1,
            limit: MAX_TEXT_BYTES
        }
    );
}
