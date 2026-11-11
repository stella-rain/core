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

// The Korean text (ADR-CONT-8): one entry per shown key, the same game numbers as the English.

/// Titles that may keep Latin letters because the name has no Korean form. Empty on purpose:
/// add a key here with a comment saying why.
const LATIN_TITLE_ALLOWED: &[&str] = &[];

fn is_hangul(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

fn has_hangul(s: &str) -> bool {
    s.chars().any(is_hangul)
}

/// The runs of ASCII digits in `s`, sorted.
fn digit_runs(s: &str) -> Vec<String> {
    let mut runs: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs.sort();
    runs
}

/// The word in a trailing ` (word)` of a title, if it has one.
fn parenthesised_suffix(title: &str) -> Option<&str> {
    let (_, rest) = title.rsplit_once(" (")?;
    rest.strip_suffix(')')
}

/// The keys (sorted) for which `bad` holds, for a failure message that names them.
fn keys_where(bad: impl Fn(&str, &str, &str, &str, &str) -> bool) -> Vec<String> {
    let set = TextSet::bundled();
    let ko = set.table("ko").unwrap();
    let en = set.table("en").unwrap();
    en.entries()
        .iter()
        .filter(|(key, e)| {
            let k = ko.get(key).unwrap_or_else(|| panic!("missing ko {key}"));
            bad(key, &e.title, &e.description, &k.title, &k.description)
        })
        .map(|(key, _)| key.to_string())
        .collect()
}

#[test]
fn every_korean_entry_has_hangul() {
    let bad = keys_where(|_, _, en_desc, ko_title, ko_desc| {
        !has_hangul(ko_title) || (!en_desc.is_empty() && !has_hangul(ko_desc))
    });
    assert!(bad.is_empty(), "no Hangul in: {bad:?}");
}

#[test]
fn no_entry_is_still_the_english_copy() {
    let bad = keys_where(|key, en_title, en_desc, ko_title, ko_desc| {
        (ko_title == en_title && !LATIN_TITLE_ALLOWED.contains(&key))
            || (!en_desc.is_empty() && ko_desc == en_desc)
    });
    assert!(bad.is_empty(), "still English: {bad:?}");
}

#[test]
fn no_latin_letters_are_left_in_korean_text() {
    let bad = keys_where(|key, _, _, ko_title, ko_desc| {
        ko_desc.chars().any(|c| c.is_ascii_alphabetic())
            || (ko_title.chars().any(|c| c.is_ascii_alphabetic())
                && !LATIN_TITLE_ALLOWED.contains(&key))
    });
    assert!(bad.is_empty(), "Latin letters in: {bad:?}");
}

#[test]
fn korean_text_keeps_the_english_numbers() {
    let set = TextSet::bundled();
    let ko = set.table("ko").unwrap();
    let en = set.table("en").unwrap();
    for (key, e) in en.entries() {
        let k = ko.get(key).unwrap();
        assert_eq!(
            digit_runs(&k.description),
            digit_runs(&e.description),
            "numbers differ in the description of {key}"
        );
        assert_eq!(
            digit_runs(&k.title),
            digit_runs(&e.title),
            "numbers differ in the title of {key}"
        );
    }
}

#[test]
fn korean_file_has_the_english_keys_the_bounds_and_the_canonical_form() {
    let set = TextSet::bundled();
    let ko = set.table("ko").unwrap();
    let en = set.table("en").unwrap();
    assert_eq!(ko.len(), en.len());
    assert!(ko.entries().keys().eq(en.entries().keys()));
    let file = std::fs::read(crate::data_root::data_root().join("content/text.ko.json")).unwrap();
    assert!(TextTable::from_bytes(&file).is_ok());
    let value: Value = serde_json::from_slice(&file).unwrap();
    let mut canonical = serde_json::to_string_pretty(&value).unwrap();
    canonical.push('\n');
    assert_eq!(
        canonical.as_bytes(),
        file.as_slice(),
        "content/text.ko.json is not sorted two-space pretty JSON with raw UTF-8 and one final newline"
    );
}

#[test]
fn one_korean_word_per_rarity_and_per_damage_type() {
    let set = TextSet::bundled();
    let ko = set.table("ko").unwrap();
    let en = set.table("en").unwrap();

    // Rarity: the parenthesised word of the title.
    let mut by_rarity: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for (key, e) in en.entries() {
        let Some(rarity) = parenthesised_suffix(&e.title) else {
            continue;
        };
        let k = ko.get(key).unwrap();
        let word = parenthesised_suffix(&k.title)
            .unwrap_or_else(|| panic!("{key}: Korean title has no (rarity) suffix"));
        assert!(
            !word.is_empty() && word.chars().all(is_hangul),
            "{key}: rarity word {word:?} is not Hangul"
        );
        by_rarity
            .entry(rarity.to_string())
            .or_default()
            .insert(word.to_string());
    }
    assert_eq!(by_rarity.len(), 6, "{by_rarity:?}");
    let mut words = std::collections::BTreeSet::new();
    for (rarity, ws) in &by_rarity {
        assert_eq!(ws.len(), 1, "{rarity} has several Korean words: {ws:?}");
        words.insert(ws.iter().next().unwrap().clone());
    }
    assert_eq!(words.len(), 6, "two rarities share a Korean word");

    // Damage type: the title before ` (` and the description without digits.
    let mut by_type: std::collections::BTreeMap<
        String,
        std::collections::BTreeSet<(String, String)>,
    > = std::collections::BTreeMap::new();
    for key in en.entries().keys() {
        let Some(rest) = key.strip_prefix("rewards/type_bonus_") else {
            continue;
        };
        let (kind, _rarity) = rest.rsplit_once('_').unwrap();
        let k = ko.get(key).unwrap();
        let name = k
            .title
            .rsplit_once(" (")
            .map_or(k.title.as_str(), |(n, _)| n);
        let desc: String = k
            .description
            .chars()
            .filter(|c| !c.is_ascii_digit())
            .collect();
        by_type
            .entry(kind.to_string())
            .or_default()
            .insert((name.to_string(), desc));
    }
    assert_eq!(by_type.len(), 10, "{:?}", by_type.keys());
    let mut names = std::collections::BTreeSet::new();
    for (kind, forms) in &by_type {
        assert_eq!(forms.len(), 1, "{kind} varies across rarities: {forms:?}");
        names.insert(forms.iter().next().unwrap().0.clone());
    }
    assert_eq!(names.len(), 10, "two damage types share a Korean name");
}

#[test]
fn the_korean_draft_leaves_no_shown_key_out() {
    let set = TextSet::bundled();
    assert_eq!(set.check_pack(Pack::bundled()), Ok(()));
    assert_eq!(set.check_shipped(), Ok(()));
    assert_eq!(Pack::bundled().check_texts(set), Ok(()));
}
