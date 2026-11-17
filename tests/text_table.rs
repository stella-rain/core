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
        "expertise/spirit_blade",
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

// The shown text of the upgrade cells that became kinds (their words, and the content's
// numbers in them). A fixed list of the kind cells whose lines are pinned: no placeholder
// stat cell is left, so a generic "no kind cell shows stat text" rule is an open question.

const CELLS: [&str; 14] = [
    "titan_trigger_cap",
    "momentum_up_1",
    "momentum_up_2",
    "momentum_cap",
    "foresight_counter_up_1",
    "foresight_counter_up_2",
    "foresight_counter_cap",
    "judgement_cut_up_1",
    "judgement_cut_up_2",
    "spirit_blade_up_1",
    "spirit_blade_up_2",
    "breaker_up_1",
    "titan_trigger_up_1",
    "titan_trigger_up_2",
];

/// The wording Kade approves: (cell id, English, Korean).
const WORDING: [(&str, &str, &str); 14] = [
    (
        "titan_trigger_cap",
        "When the giant hammer runs out, the full-gauge smash's first ring strikes once more, free",
        "거대한 망치가 끝나면 꽉 찬 일격의 첫 번째 고리가 공짜로 한 번 더 터져요",
    ),
    (
        "momentum_up_1",
        "At full stacks, every attack the combo starts reaches 25% wider",
        "중첩이 가득 차면 콤보의 모든 공격 범위가 25% 넓어져요",
    ),
    (
        "momentum_up_2",
        "Each stack speeds attacks up 6% instead of 4%, 30% at full stacks",
        "각 중첩이 공격 속도를 4%에서 6%로 올려, 가득 차면 30% 빨라져요",
    ),
    (
        "momentum_cap",
        "A smash at full stacks carries them into the next combo, until the combo window runs out",
        "중첩이 가득 찬 채 강타하면 다음 콤보까지 이어지고, 콤보 시간이 끝나면 사라져요",
    ),
    (
        "foresight_counter_up_1",
        "The counter window after a perfect evade lasts 0.63 seconds, up from 0.5",
        "완벽한 회피 후 반격 가능 시간이 0.5초에서 0.63초로 늘어요",
    ),
    (
        "foresight_counter_up_2",
        "The counter step slides 48 pixels toward the target",
        "반격 동작이 대상 쪽으로 48픽셀 전진해요",
    ),
    (
        "foresight_counter_cap",
        "A counter's first hit returns a spent evade and fills 25% of a gauge; at 4 counters, your next smash is the full-gauge smash",
        "반격의 첫 타격이 사용한 회피 횟수를 하나 돌려주고 게이지를 25% 채워요; 반격 4번이면 다음 강타가 꽉 찬 일격이 돼요",
    ),
    (
        "judgement_cut_up_1",
        "Closing cuts break enemy orbs within 96 pixels of what they strike, 60% of the time",
        "닫히는 베기가 맞힌 대상 주변 96픽셀 안의 적 구슬을 60% 확률로 부숴요",
    ),
    (
        "judgement_cut_up_2",
        "Each closing cut leaves a bleed on every enemy it hits: 1 stack for 3 seconds, up to 6 stacks",
        "닫히는 베기가 맞힌 모든 적에게 3초 동안 출혈을 1중첩씩 남기고, 최대 6중첩까지 쌓여요",
    ),
    (
        "spirit_blade_up_1",
        "A spirit level lasts 30 seconds without a smash, up from 20",
        "강타 없이 정령 단계가 유지되는 시간이 20초에서 30초로 늘어요",
    ),
    (
        "spirit_blade_up_2",
        "The third strike's charge reaches each level 25% sooner",
        "세 번째 공격을 모으면 각 단계에 25% 더 빨리 도달해요",
    ),
    (
        "breaker_up_1",
        "A stagger lasts 50% longer",
        "경직이 50% 더 오래 지속돼요",
    ),
    (
        "titan_trigger_up_1",
        "The giant hammer lasts 12 seconds, up from 8",
        "망치가 거대해지는 시간이 8초에서 12초로 늘어요",
    ),
    (
        "titan_trigger_up_2",
        "Hold the third spin, moving at 65% speed while the hammer is giant and 50% otherwise",
        "세 번째 회전을 누르고 있을 수 있고, 이동 속도는 망치가 거대한 동안 65%, 아니면 50%예요",
    ),
];

fn expertise() -> Value {
    let file = std::fs::read(crate::data_root::data_root().join("content/expertise.json")).unwrap();
    serde_json::from_slice(&file).unwrap()
}

/// The style (null for a common cell) and the cell with this id.
fn cell(root: &Value, id: &str) -> (Value, Value) {
    for w in root["weapons"].as_array().unwrap() {
        for s in w["styles"].as_array().unwrap() {
            for slot in ["upgrade_1", "upgrade_2", "capstone"] {
                for c in s[slot].as_array().unwrap() {
                    if c["id"] == id {
                        return (s.clone(), c.clone());
                    }
                }
            }
        }
        for c in w["common"].as_array().unwrap() {
            if c["id"] == id {
                return (Value::Null, c.clone());
            }
        }
    }
    panic!("no cell {id}");
}

/// The first kind of this type in a style or a cell.
fn kind(holder: &Value, ty: &str) -> Value {
    holder["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["type"] == ty)
        .unwrap_or_else(|| panic!("no kind {ty}"))
        .clone()
}

/// The first effect of this type in a kind.
fn effect(k: &Value, ty: &str) -> Value {
    k["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == ty)
        .unwrap_or_else(|| panic!("no effect {ty}"))
        .clone()
}

/// Ticks as seconds (60 ticks a second), rounded to two decimals, no trailing zero.
fn seconds(ticks: i64) -> String {
    let hundredths = (ticks * 100 + 30) / 60;
    let (whole, frac) = (hundredths / 100, hundredths % 100);
    match frac {
        0 => format!("{whole}"),
        f if f % 10 == 0 => format!("{whole}.{}", f / 10),
        f => format!("{whole}.{f:02}"),
    }
}

fn n(v: &Value) -> i64 {
    v.as_i64().unwrap()
}

/// The numbers a cell's line shows, from the content, joined.
fn expected_numbers(root: &Value, id: &str) -> String {
    let (style, c) = cell(root, id);
    let parts: Vec<String> = match id {
        "foresight_counter_up_1" => vec![
            seconds(n(&kind(&style, "counter_bonus")["window_ticks"])),
            seconds(n(&kind(&c, "counter_bonus")["window_ticks"])),
        ],
        "foresight_counter_up_2" => {
            let e = effect(&kind(&c, "counter_bonus"), "advance");
            vec![(n(&e["distance"]) / 256).to_string()]
        }
        "foresight_counter_cap" => {
            let k = kind(&c, "counter_bonus");
            effect(&k, "refund_evade");
            let fill = n(&effect(&k, "gauge_fill_pct")["pct"]);
            let max = n(&kind(&c, "gauge")["max"]);
            assert_eq!(max % fill, 0, "the line counts whole counters");
            vec![fill.to_string(), (max / fill).to_string()]
        }
        "titan_trigger_cap" => {
            effect(&kind(&c, "giant_trigger"), "end_ring");
            vec![]
        }
        "momentum_cap" => {
            effect(&kind(&c, "combo_tempo_pct"), "carry_at_full");
            vec![]
        }
        "momentum_up_1" => {
            let e = effect(&kind(&c, "combo_tempo_pct"), "range_pct_at_full");
            vec![n(&e["pct"]).to_string()]
        }
        "momentum_up_2" => {
            let k = kind(&c, "combo_tempo_pct");
            let cell_speed = n(&effect(&k, "speed_pct")["pct"]);
            let style_speed = n(&effect(&kind(&style, "combo_tempo_pct"), "speed_pct")["pct"]);
            vec![
                (cell_speed * n(&k["max_stacks"])).to_string(),
                style_speed.to_string(),
                cell_speed.to_string(),
            ]
        }
        "judgement_cut_up_1" => {
            let k = kind(&c, "repeat_breaks_orbs");
            vec![
                n(&k["radius_px"]).to_string(),
                n(&k["chance_pct"]).to_string(),
            ]
        }
        "judgement_cut_up_2" => {
            let k = kind(&c, "repeat_bleed");
            vec![
                n(&k["stacks"]).to_string(),
                seconds(n(&k["duration_ticks"])),
                n(&k["max_stacks"]).to_string(),
            ]
        }
        "spirit_blade_up_1" => vec![
            seconds(n(&kind(&style, "spirit")["decay_ticks"])),
            seconds(n(&kind(&c, "spirit")["decay_ticks"])),
        ],
        "spirit_blade_up_2" => vec![n(&kind(&c, "charge_sooner")["pct"]).to_string()],
        "breaker_up_1" => {
            let e = effect(&kind(&c, "breaker"), "stagger_ticks_pct");
            vec![(n(&e["pct"]) - 100).to_string()]
        }
        "titan_trigger_up_1" => vec![
            seconds(n(&kind(&style, "giant_trigger")["ticks"])),
            seconds(n(&kind(&c, "giant_trigger")["ticks"])),
        ],
        "titan_trigger_up_2" => {
            let e = effect(&kind(&c, "giant_trigger"), "hold_move_pct");
            let step = kind(&c, "replace_step");
            assert_eq!(step["step"], serde_json::json!(["main", "main", "main"]));
            let hold = &step["grant"]["hold"];
            vec![n(&e["pct"]).to_string(), n(&hold["move_pct"]).to_string()]
        }
        other => panic!("no numbers for {other}"),
    };
    parts.join(" ")
}

#[test]
fn a_kind_cell_does_not_show_placeholder_stat_text() {
    let root = expertise();
    let set = TextSet::bundled();
    let (en, ko) = (set.table("en").unwrap(), set.table("ko").unwrap());
    for id in CELLS {
        assert_eq!(cell(&root, id).1["effect"], "none", "{id} is not a kind");
        let key = format!("expertise/{id}");
        assert!(
            !en.get(&key).unwrap().description.starts_with('+'),
            "{key} still shows stat text in English"
        );
        let k = &ko.get(&key).unwrap().description;
        for amount in [10, 20] {
            assert_ne!(k, &format!("공격력이 {amount}% 늘어요"), "{key}");
            assert_ne!(k, &format!("공격 속도가 {amount}% 늘어요"), "{key}");
        }
    }
}

#[test]
fn the_kind_cells_show_the_approved_wording() {
    let set = TextSet::bundled();
    let (en, ko) = (set.table("en").unwrap(), set.table("ko").unwrap());
    for (id, english, korean) in WORDING {
        let key = format!("expertise/{id}");
        assert_eq!(en.get(&key).unwrap().description, english, "{key} (en)");
        assert_eq!(ko.get(&key).unwrap().description, korean, "{key} (ko)");
    }
}

#[test]
fn the_kind_cells_show_the_numbers_of_the_content() {
    let root = expertise();
    let set = TextSet::bundled();
    let (en, ko) = (set.table("en").unwrap(), set.table("ko").unwrap());
    for id in CELLS {
        let key = format!("expertise/{id}");
        let want = digit_runs(&expected_numbers(&root, id));
        assert_eq!(
            digit_runs(&en.get(&key).unwrap().description),
            want,
            "{key} (en)"
        );
        assert_eq!(
            digit_runs(&ko.get(&key).unwrap().description),
            want,
            "{key} (ko)"
        );
    }
}
