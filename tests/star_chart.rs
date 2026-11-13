//! The Star Chart price catalogue (ADR-SAVE-17, ADR-CONT-3 points 1 and 8): the Stardust prices
//! of what the chart sells, an optional member of the pack, read as untrusted bytes (ADR-SVC-1).
//! No run reads it.

use serde_json::{Value, json};
use stella_rain_core::pack::{ContentFiles, Pack, PackError};
use stella_rain_core::rewards::Rarity;
use stella_rain_core::run_setup::MAX_LUMINE;
use stella_rain_core::star_chart::{
    MAX_ITEMS, MAX_LEVELS, MAX_PRICE, MAX_STAR_CHART_BYTES, StarChart,
};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

fn file() -> Value {
    crate::before::pack_value()["star_chart"].clone()
}

fn parse_bytes(bytes: &[u8]) -> Result<StarChart, Error> {
    let content = crate::before::content();
    StarChart::parse_with(
        bytes,
        Versions::CURRENT,
        content.run_stats(),
        content.perk_sets(),
        content.stage_skills(),
        content.weapons(),
        content.expertise(),
        content.starting_unlocks(),
    )
}

fn parse(v: &Value) -> Result<StarChart, Error> {
    parse_bytes(v.to_string().as_bytes())
}

/// The paths of the issues a file is refused with.
fn paths(v: &Value) -> Vec<String> {
    match parse(v) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

fn edited(edit: impl FnOnce(&mut Value)) -> Value {
    let mut v = file();
    edit(&mut v);
    v
}

fn pack_with(star_chart: Value) -> Result<Pack, PackError> {
    let mut v: Value = serde_json::from_slice(crate::before::pack_bytes()).unwrap();
    v["star_chart"] = star_chart;
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap())
}

// P1: the bundled catalogue.

#[test]
fn the_bundled_catalogue_is_valid_and_prices_known_ids_only() {
    let pack = Pack::bundled();
    let chart = pack.star_chart().expect("the bundled pack holds it");
    let content = pack.content();
    let f = file();
    for name in ["weapons", "perk_sets", "stage_skills"] {
        for entry in f[name].as_array().unwrap() {
            assert!(
                (1..=MAX_PRICE)
                    .contains(&(u32::try_from(entry["price"].as_u64().unwrap()).unwrap()))
            );
        }
    }
    for entry in f["weapons"].as_array().unwrap() {
        let id = entry["id"].as_str().unwrap();
        assert!(content.weapons().ids().any(|w| w.as_str() == id), "{id}");
        let free = content.starting_unlocks().unwrap();
        assert!(!free.weapons().iter().any(|w| w.as_str() == id), "{id}");
    }
    for entry in f["stats"].as_array().unwrap() {
        let id = entry["id"].as_str().unwrap();
        assert!(
            content
                .run_stats()
                .stats()
                .iter()
                .any(|s| s.id.as_str() == id)
        );
        let prices = entry["prices"].as_array().unwrap();
        assert!(prices.windows(2).all(|w| w[0].as_u64() <= w[1].as_u64()));
    }
    let expertise = f["expertise_multiplier"].as_array().unwrap();
    assert!(expertise.windows(2).all(|w| w[0].as_u64() <= w[1].as_u64()));
    let lumine = f["lumine_levels"].as_array().unwrap();
    assert_eq!(lumine.len(), usize::from(MAX_LUMINE));
    assert!(lumine.windows(2).all(|w| w[0].as_u64() <= w[1].as_u64()));
    assert_eq!(
        chart.sha256().as_slice(),
        sha2::Sha256::digest(ContentFiles::bundled_file("star_chart")).as_slice()
    );
}

use sha2::Digest;

// P2: refusals, one assertion each.

#[test]
fn a_price_of_zero_or_over_the_ceiling_is_refused() {
    let zero = edited(|v| v["weapons"][0]["price"] = json!(0));
    assert_eq!(paths(&zero), ["$.weapons[0].price"]);
    let over = edited(|v| v["weapons"][0]["price"] = json!(MAX_PRICE + 1));
    assert_eq!(paths(&over), ["$.weapons[0].price"]);
    assert!(parse(&edited(|v| v["weapons"][0]["price"] = json!(MAX_PRICE))).is_ok());
    let rune = edited(|v| v["starting_rune"] = json!(0));
    assert_eq!(paths(&rune), ["$.starting_rune"]);
}

#[test]
fn an_unknown_or_repeated_or_free_id_is_refused() {
    let unknown = edited(|v| v["weapons"][0]["id"] = json!("no_such_weapon"));
    assert_eq!(paths(&unknown), ["$.weapons[0].id"]);
    let twice = edited(|v| {
        let first = v["perk_sets"][0].clone();
        v["perk_sets"] = json!([first.clone(), first]);
    });
    assert_eq!(paths(&twice), ["$.perk_sets[1].id"]);
    let free = edited(|v| v["weapons"][0]["id"] = json!("giant_sword"));
    assert_eq!(paths(&free), ["$.weapons[0].id"]);
    let stat = edited(|v| v["stats"][0]["id"] = json!("no_such_stat"));
    assert_eq!(paths(&stat), ["$.stats[0].id"]);
}

#[test]
fn the_level_lists_have_their_lengths() {
    let none = edited(|v| v["stats"][0]["prices"] = json!([]));
    assert_eq!(paths(&none), ["$.stats[0].prices"]);
    let many = edited(|v| v["stats"][0]["prices"] = json!(vec![100; MAX_LEVELS + 1]));
    assert_eq!(paths(&many), ["$.stats[0].prices"]);
    let short = edited(|v| v["lumine_levels"] = json!(vec![100; usize::from(MAX_LUMINE) - 1]));
    assert_eq!(paths(&short), ["$.lumine_levels"]);
    let long = edited(|v| v["lumine_levels"] = json!(vec![100; usize::from(MAX_LUMINE) + 1]));
    assert_eq!(paths(&long), ["$.lumine_levels"]);
    let empty = edited(|v| v["expertise_multiplier"] = json!([]));
    assert_eq!(paths(&empty), ["$.expertise_multiplier"]);
    let many = edited(|v| {
        v["expertise_levels"] = json!(MAX_LEVELS);
        v["expertise_multiplier"] = json!(vec![100; MAX_LEVELS + 1]);
    });
    assert_eq!(paths(&many), ["$.expertise_multiplier"]);
}

#[test]
fn a_level_list_that_falls_is_refused() {
    let expertise = edited(|v| {
        v["expertise_levels"] = json!(2);
        v["expertise_multiplier"] = json!([100, 50]);
    });
    assert_eq!(paths(&expertise), ["$.expertise_multiplier"]);
    let stat = edited(|v| v["stats"][1]["prices"] = json!([100, 50]));
    assert_eq!(paths(&stat), ["$.stats[1].prices"]);
    let mut lumine = vec![100; usize::from(MAX_LUMINE)];
    lumine[4] = 99;
    assert_eq!(
        paths(&edited(|v| v["lumine_levels"] = json!(lumine))),
        ["$.lumine_levels"]
    );
    // Equal prices do not fall.
    let equal = edited(|v| {
        v["expertise_levels"] = json!(2);
        v["expertise_multiplier"] = json!([100, 100]);
    });
    assert!(parse(&equal).is_ok());
}

#[test]
fn too_many_entries_are_refused_by_the_count_budget_before_the_file_is_read() {
    for name in ["weapons", "perk_sets", "stage_skills", "stats"] {
        let mut chart = file();
        let list = chart[name].as_array_mut().unwrap();
        let first = list[0].clone();
        list.resize(MAX_ITEMS + 1, first);
        match pack_with(chart) {
            Err(PackError::Budget { path, limit, found }) => {
                assert_eq!(path, format!("$.star_chart.{name}"));
                assert_eq!((limit, found), (MAX_ITEMS, MAX_ITEMS + 1));
            }
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn a_refusal_in_a_pack_names_the_file() {
    let bad = edited(|v| v["weapons"][0]["price"] = json!(0));
    match pack_with(bad) {
        Err(PackError::File {
            file: "star_chart",
            error: Error::Invalid(issues),
        }) => assert_eq!(issues[0].path, "$.weapons[0].price"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_key_and_an_oversized_file_are_refused() {
    let extra = edited(|v| v["stars"] = json!([]));
    assert!(matches!(parse(&extra), Err(Error::Parse { .. })));
    let mut bytes = serde_json::to_vec(&file()).unwrap();
    bytes.resize(MAX_STAR_CHART_BYTES + 1, b' ');
    assert!(matches!(parse_bytes(&bytes), Err(Error::TooLarge { .. })));
}

// P3: a released pack stays valid.

#[test]
fn a_pack_without_the_member_loads_and_has_no_catalogue() {
    let mut v: Value = serde_json::from_slice(crate::before::pack_bytes()).unwrap();
    v.as_object_mut().unwrap().remove("star_chart");
    let pack = Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(pack.star_chart().is_none());
    assert!(pack.content().star_chart().is_none());
}

#[test]
fn every_frozen_pack_loads_without_a_catalogue() {
    let dir = crate::data_root::data_root().join("tests/corpus/v1/packs");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let pack = Pack::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
            assert!(pack.star_chart().is_none(), "{path:?}");
            seen += 1;
        }
    }
    assert!(seen > 0);
}

// P4: a stat's price list is as long as that stat's `limit`.

#[test]
fn a_stat_price_list_is_as_long_as_the_stats_limit() {
    for (found, len) in [(4, "4"), (6, "6")] {
        let v = edited(|v| v["stats"][0]["prices"] = json!(vec![100; found]));
        match parse(&v) {
            Err(Error::Invalid(issues)) => {
                assert_eq!(issues.len(), 1, "{issues:?}");
                assert_eq!(issues[0].path, "$.stats[0].prices");
                assert_eq!(issues[0].message, format!("exactly 5 prices (found {len})"));
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(parse(&edited(|v| v["stats"][0]["prices"] = json!(vec![100; 5]))).is_ok());
}

fn pack_with_attack_limit(limit: u64, attack_prices: usize) -> Result<Pack, PackError> {
    let mut v: Value = serde_json::from_slice(crate::before::pack_bytes()).unwrap();
    v["run_stats"]["stats"][1]["limit"] = json!(limit);
    let mut chart = file();
    chart["stats"][1]["prices"] = json!(vec![100; attack_prices]);
    v["star_chart"] = chart;
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap())
}

#[test]
fn the_price_list_follows_the_stats_limit_not_a_constant() {
    // The bundled 5-price list against a limit of 3.
    match pack_with_attack_limit(3, 5) {
        Err(PackError::File {
            file: "star_chart",
            error: Error::Invalid(issues),
        }) => {
            assert_eq!(issues.len(), 1, "{issues:?}");
            assert_eq!(issues[0].path, "$.stats[1].prices");
            assert_eq!(issues[0].message, "exactly 3 prices (found 5)");
        }
        other => panic!("{other:?}"),
    }
    // A limit of 20 takes a 20-price list; the budget allows it.
    assert!(pack_with_attack_limit(20, 20).is_ok());
    // 21 prices are over the budget, before the file is read.
    match pack_with_attack_limit(20, 21) {
        Err(PackError::Budget { path, limit, found }) => {
            assert_eq!(path, "$.star_chart.stats[1].prices");
            assert_eq!((limit, found), (20, 21));
        }
        other => panic!("{other:?}"),
    }
}

// P5: the expertise multiplier's level cap is pack content.

fn issue_of(v: &Value) -> Vec<(String, String)> {
    match parse(v) {
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

#[test]
fn the_expertise_cap_equals_the_length_of_its_price_list() {
    let f = file();
    let n = f["expertise_levels"]
        .as_u64()
        .expect("the bundled file has the key");
    let prices = f["expertise_multiplier"].as_array().unwrap().len();
    assert_eq!(n, u64::try_from(prices).unwrap());
    let chart = Pack::bundled().star_chart().unwrap().expertise_levels();
    assert_eq!(u64::from(chart), n);
}

#[test]
fn the_expertise_price_list_is_as_long_as_the_cap() {
    let short_cap = edited(|v| v["expertise_levels"] = json!(3));
    assert_eq!(
        issue_of(&short_cap),
        [(
            "$.expertise_multiplier".into(),
            "exactly 3 prices (found 5)".into()
        )]
    );
    let three = edited(|v| {
        v["expertise_levels"] = json!(3);
        v["expertise_multiplier"] = json!([100, 200, 300]);
    });
    assert!(parse(&three).is_ok());
}

#[test]
fn the_expertise_cap_is_bounded_by_the_ceiling() {
    for n in [0, MAX_LEVELS + 1] {
        let v = edited(|v| v["expertise_levels"] = json!(n));
        assert_eq!(
            issue_of(&v),
            [(
                "$.expertise_levels".into(),
                format!("1 to {MAX_LEVELS} (found {n})")
            )]
        );
    }
    let top = edited(|v| {
        v["expertise_levels"] = json!(MAX_LEVELS);
        v["expertise_multiplier"] = json!(vec![100; MAX_LEVELS]);
    });
    assert!(parse(&top).is_ok());
}

#[test]
fn a_missing_negative_or_fractional_expertise_cap_is_refused() {
    let missing = edited(|v| {
        v.as_object_mut().unwrap().remove("expertise_levels");
    });
    assert!(parse(&missing).is_err());
    for bad in [json!(-1), json!(2.5)] {
        assert!(parse(&edited(|v| v["expertise_levels"] = bad)).is_err());
    }
}

// P6: the Lumine levels and the rewards file's Lumine costs.

/// The catalogue's `lumine_levels` are the Stardust price of each Lumine level; the rewards
/// file's `lumine_cost` is the Lumine a star of a rarity costs. Different units: neither
/// decides the other. The only link is the level count, and the dearest star takes them all
/// (ADR-SAVE-7 point 9: level 10 allows a supernova). A pack may price a supernova lower, so
/// this guards the bundled pack and is not a validator rule.
#[test]
fn the_lumine_levels_cover_the_rewards_lumine_costs() {
    let pack = Pack::bundled();
    let levels = pack.star_chart().unwrap().lumine_levels().len();
    assert_eq!(levels, usize::from(MAX_LUMINE));
    let rewards = pack.content().rewards();
    for rarity in Rarity::ALL {
        assert!(
            usize::from(rewards.lumine_cost(rarity)) <= levels,
            "{rarity:?}"
        );
    }
    assert_eq!(rewards.lumine_cost(Rarity::Supernova), MAX_LUMINE);
}

// P5: per-weapon style rows (ADR-SIM-9 point 8, ADR-SAVE-7 point 10): which styles a weapon sells
// and for how much, empty in the bundled pack.

fn hammer_row() -> Value {
    json!({ "weapon": "giant_hammer", "styles": [
        { "id": "crush", "price": 1000 }, { "id": "quake", "price": 2000 }
    ] })
}

fn with_styles(rows: Value) -> Value {
    edited(|v| v["styles"] = rows)
}

/// The issues (path, message) a pack with `chart` is refused with.
fn refused(chart: Value) -> Vec<(String, String)> {
    match pack_with(chart) {
        Err(PackError::File {
            file: "star_chart",
            error: Error::Invalid(issues),
        }) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

#[test]
fn the_bundled_file_has_no_style_rows() {
    assert!(file().get("styles").is_none());
}

#[test]
fn style_rows_are_accepted_and_the_pack_keeps_them() {
    let rows = json!([hammer_row()]);
    let pack = pack_with(with_styles(rows.clone())).expect("a Hammer row");
    assert!(pack.star_chart().is_some());
    let two = json!([
        hammer_row(),
        { "weapon": "giant_sword", "styles": [{ "id": "edge", "price": 500 }] }
    ]);
    assert!(pack_with(with_styles(two)).is_ok());
    let unpriced =
        json!([{ "weapon": "giant_hammer", "styles": [{ "id": "bulwark", "price": 7 }] }]);
    assert!(pack_with(with_styles(unpriced)).is_ok());
}

#[test]
fn bad_style_rows_are_refused_at_their_path() {
    let one = |row: Value| refused(with_styles(json!([row])));
    let row = |weapon: &str, id: &str, price: u32| json!({ "weapon": weapon, "styles": [{ "id": id, "price": price }] });
    assert_eq!(
        one(row("no_such_weapon", "crush", 1))[0],
        (
            "$.styles[0].weapon".into(),
            "not an ID the game holds".into()
        )
    );
    assert_eq!(
        one(row("Not A Weapon", "crush", 1))[0].0,
        "$.styles[0].weapon"
    );
    let twice = refused(with_styles(json!([hammer_row(), hammer_row()])));
    assert_eq!(
        twice,
        [(
            "$.styles[1].weapon".into(),
            "the same ID as an earlier entry".into()
        )]
    );
    assert_eq!(
        one(row("giant_hammer", "edge", 1))[0],
        (
            "$.styles[0].styles[0].id".into(),
            "not a style of the weapon".into()
        )
    );
    let dup = json!({ "weapon": "giant_hammer", "styles": [
        { "id": "crush", "price": 1 }, { "id": "crush", "price": 2 }
    ] });
    assert_eq!(one(dup)[0].0, "$.styles[0].styles[1].id");
    assert_eq!(
        one(row("giant_hammer", "crush", 0))[0].0,
        "$.styles[0].styles[0].price"
    );
    assert_eq!(
        one(row("giant_hammer", "crush", MAX_PRICE + 1))[0].0,
        "$.styles[0].styles[0].price"
    );
}

#[test]
fn the_style_lists_have_count_budgets_before_the_file_is_read() {
    let rows: Vec<Value> = (0..=MAX_ITEMS).map(|_| hammer_row()).collect();
    match pack_with(with_styles(json!(rows))) {
        Err(PackError::Budget { path, limit, found }) => {
            assert_eq!(path, "$.star_chart.styles");
            assert_eq!((limit, found), (MAX_ITEMS, MAX_ITEMS + 1));
        }
        other => panic!("{other:?}"),
    }
    let four = json!([{ "weapon": "giant_hammer", "styles": [
        { "id": "a", "price": 1 }, { "id": "b", "price": 1 },
        { "id": "c", "price": 1 }, { "id": "d", "price": 1 }
    ] }]);
    match pack_with(with_styles(four)) {
        Err(PackError::Budget { path, limit, found }) => {
            assert_eq!(path, "$.star_chart.styles[0].styles");
            assert_eq!((limit, found), (3, 4));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_accessor_returns_the_rows_and_the_bundled_chart_has_none() {
    let chart = parse(&with_styles(json!([hammer_row()]))).expect("a Hammer row");
    assert_eq!(chart.styles().len(), 1);
    assert_eq!(chart.styles()[0].weapon.as_str(), "giant_hammer");
    let ids: Vec<&str> = chart.styles()[0]
        .styles
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(ids, ["crush", "quake"]);
    assert!(Pack::bundled().star_chart().unwrap().styles().is_empty());
}
