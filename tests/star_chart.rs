//! The Star Chart price catalogue (ADR-SAVE-17, ADR-CONT-3 points 1 and 8): the Stardust prices
//! of what the chart sells, an optional member of the pack, read as untrusted bytes (ADR-SVC-1).
//! No run reads it.

use serde_json::{Value, json};
use stella_rain_core::pack::{ContentFiles, Pack, PackError};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::MAX_LUMINE;
use stella_rain_core::star_chart::{
    MAX_ITEMS, MAX_LEVELS, MAX_PRICE, MAX_STAR_CHART_BYTES, StarChart,
};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

fn file() -> Value {
    serde_json::from_slice(ContentFiles::bundled_file("star_chart")).expect("the bundled file")
}

fn parse_bytes(bytes: &[u8]) -> Result<StarChart, Error> {
    let content = RunContent::bundled();
    StarChart::parse_with(
        bytes,
        Versions::CURRENT,
        content.run_stats(),
        content.perk_sets(),
        content.stage_skills(),
        content.weapons(),
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
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
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
}

#[test]
fn a_level_list_that_falls_is_refused() {
    let expertise = edited(|v| v["expertise_multiplier"] = json!([100, 50]));
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
    assert!(parse(&edited(|v| v["expertise_multiplier"] = json!([100, 100]))).is_ok());
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
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
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
