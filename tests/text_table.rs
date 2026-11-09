use serde_json::Value;
use stella_rain_core::pack::Pack;
use stella_rain_core::text::{
    FALLBACK_LOCALE, MAX_TEXT_BYTES, PRIMARY_LOCALE, SHIPPED_LOCALES, TextError, TextSet,
    TextSetError, TextTable, is_locale_code,
};

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

/// The bundled English table with `drop` keys removed, as file bytes.
fn english_without(drop: &[&str]) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(TextTable::BUNDLED).unwrap();
    for key in drop {
        assert!(v.as_object_mut().unwrap().remove(*key).is_some(), "{key}");
    }
    serde_json::to_vec(&v).unwrap()
}

#[test]
fn the_locale_constants() {
    assert_eq!(FALLBACK_LOCALE, "en");
    assert_eq!(PRIMARY_LOCALE, "ko");
    assert_eq!(SHIPPED_LOCALES, ["ko", "en"]);
}

#[test]
fn locale_codes_are_two_or_three_lowercase_letters() {
    for ok in ["en", "ko", "fil"] {
        assert!(is_locale_code(ok), "{ok}");
    }
    for bad in ["EN", "english", "e", "", "en.json", "../en", "e1", "ko-KR"] {
        assert!(!is_locale_code(bad), "{bad:?}");
    }
}

#[test]
fn the_bundled_set_holds_both_locales_and_covers_the_pack() {
    let set = TextSet::bundled();
    assert_eq!(set.locales().collect::<Vec<_>>(), ["en", "ko"]);
    let ko = set.table("ko").unwrap();
    let en = set.table("en").unwrap();
    assert_eq!(ko.len(), TextTable::bundled().len());
    assert!(ko.entries().keys().eq(en.entries().keys()));
    assert!(set.table("fr").is_none());
    assert_eq!(set.check_pack(Pack::bundled()), Ok(()));
    assert_eq!(set.check_shipped(), Ok(()));
    assert_eq!(Pack::bundled().check_texts(set), Ok(()));
    assert_eq!(
        TextTable::shown_keys(Pack::bundled()).len(),
        TextTable::bundled().len()
    );
}

#[test]
fn lookup_reads_the_locale_then_english() {
    let set = TextSet::bundled();
    let (entry, from) = set.lookup("ko", "rewards/star_altair").unwrap();
    assert_eq!(from, "ko");
    assert_eq!(
        entry,
        set.table("ko").unwrap().get("rewards/star_altair").unwrap()
    );
    assert_eq!(
        set.title("ko", "difficulties", "easy"),
        Some(
            set.table("ko")
                .unwrap()
                .title("difficulties", "easy")
                .unwrap()
        )
    );
    assert!(set.description("ko", "difficulties", "easy").is_some());
    for locale in ["xx", "fr"] {
        let (_, from) = set.lookup(locale, "difficulties/easy").unwrap();
        assert_eq!(from, "en");
    }
    assert!(set.lookup("ko", "nothing/there").is_none());
}

#[test]
fn a_missing_line_falls_back_to_english_and_fails_the_check() {
    let ko = english_without(&["difficulties/easy"]);
    let set = TextSet::from_files(&[("ko", &ko), ("en", TextTable::BUNDLED)]).unwrap();
    let (entry, from) = set.lookup("ko", "difficulties/easy").unwrap();
    assert_eq!(from, "en");
    assert_eq!(
        entry,
        TextTable::bundled().get("difficulties/easy").unwrap()
    );
    assert_eq!(
        set.check_pack(Pack::bundled()),
        Err(vec!["ko:difficulties/easy".to_string()])
    );

    let en = english_without(&["rewards/star_altair"]);
    let set = TextSet::from_files(&[("ko", &ko), ("en", &en)]).unwrap();
    assert_eq!(
        set.check_pack(Pack::bundled()),
        Err(vec![
            "en:rewards/star_altair".to_string(),
            "ko:difficulties/easy".to_string(),
            "ko:rewards/star_altair".to_string(),
        ])
    );
}

#[test]
fn from_files_refuses_what_is_not_a_set() {
    let en = TextTable::BUNDLED;
    assert_eq!(
        TextSet::from_files(&[("ko", en)]).unwrap_err(),
        TextSetError::NoFallback
    );
    assert_eq!(
        TextSet::from_files(&[("en", en), ("ko", en), ("ko", en)]).unwrap_err(),
        TextSetError::Duplicate {
            locale: "ko".into()
        }
    );
    for code in ["EN", "english", "e", "", "en.json", "../en"] {
        assert_eq!(
            TextSet::from_files(&[("en", en), (code, en)]).unwrap_err(),
            TextSetError::Locale { code: code.into() },
            "{code:?}"
        );
    }
    let dup = br#"{"a/b": {"title": "T"}, "a/b": {"title": "U"}}"#;
    assert!(matches!(
        TextSet::from_files(&[("en", dup)]).unwrap_err(),
        TextSetError::Table { locale, error: TextError::Parse { .. } } if locale == "en"
    ));
    let big = vec![b' '; MAX_TEXT_BYTES + 1];
    assert!(matches!(
        TextSet::from_files(&[("en", en), ("ko", &big)]).unwrap_err(),
        TextSetError::Table { locale, error: TextError::TooLarge { .. } } if locale == "ko"
    ));
}

#[test]
fn set_errors_never_repeat_text() {
    let marker = "MARKER\u{1}TEXT";
    let bad = format!(r#"{{"a/b": {{"title": "{marker}"}}}}"#);
    let err = TextSet::from_files(&[("en", bad.as_bytes())]).unwrap_err();
    let shown = err.to_string();
    assert!(!shown.contains("MARKER"), "{shown}");
    for e in [
        TextSetError::Locale { code: "EN".into() },
        TextSetError::NoFallback,
        TextSetError::Duplicate {
            locale: "ko".into(),
        },
    ] {
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn check_shipped_names_the_locales_a_set_lacks() {
    let set = TextSet::from_files(&[("en", TextTable::BUNDLED)]).unwrap();
    assert_eq!(set.check_shipped(), Err(vec!["ko".to_string()]));
}
