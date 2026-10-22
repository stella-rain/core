//! The starting unlocks file (ADR-080, ADR-099 point 4a, ADR-124): the weapons, perk sets,
//! stage skills and runes a new save begins with, read from JSON as untrusted bytes (ADR-020).
//! The simulation never reads it; the app seeds a new save from the pack.

use serde_json::{Value, json};
use stella_rain_core::pack::{self, Pack};
use stella_rain_core::perk_sets::PerkSets;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::runes::Runes;
use stella_rain_core::stage_skill::StageSkills;
use stella_rain_core::starting_unlocks::{
    MAX_STARTING_UNLOCKS, MAX_STARTING_UNLOCKS_BYTES, StartingUnlocks,
};
use stella_rain_core::validate::Error;

fn file() -> Value {
    json!({
        "schema_version": 0,
        "sim_version": 0,
        "weapons": ["giant_sword"],
        "perk_sets": ["duelist"],
        "stage_skills": ["nova"],
        "runes": ["explosion", "bleed", "boomerang"],
    })
}

fn parse(v: &Value) -> Result<StartingUnlocks, Error> {
    StartingUnlocks::parse_with(
        v.to_string().as_bytes(),
        PerkSets::bundled(),
        StageSkills::bundled(),
        Runes::bundled(),
    )
}

fn paths(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

fn ids(list: &[stella_rain_core::id::ContentId]) -> Vec<&str> {
    list.iter().map(|id| id.0.as_str()).collect()
}

#[test]
fn a_valid_file_loads_and_keeps_its_hash() {
    let u = parse(&file()).unwrap();
    assert_eq!(ids(u.weapons()), ["giant_sword"]);
    assert_eq!(ids(u.perk_sets()), ["duelist"]);
    assert_eq!(ids(u.stage_skills()), ["nova"]);
    assert_eq!(ids(u.runes()), ["explosion", "bleed", "boomerang"]);
    let mut other = file();
    other["runes"] = json!([]);
    assert_ne!(u.sha256(), parse(&other).unwrap().sha256());
}

#[test]
fn empty_lists_are_allowed() {
    let v = json!({ "schema_version": 0, "sim_version": 0,
        "weapons": [], "perk_sets": [], "stage_skills": [], "runes": [] });
    assert!(parse(&v).unwrap().weapons().is_empty());
}

#[test]
fn an_id_its_source_does_not_know_is_refused() {
    for kind in ["weapons", "perk_sets", "stage_skills", "runes"] {
        let mut v = file();
        v[kind] = json!(["not_a_real_id"]);
        assert_eq!(paths(parse(&v).unwrap_err()), [format!("$.{kind}[0]")]);
    }
    // A perk set is not a stage skill, and a rune the closed list knows but the file lacks is
    // refused with the file's own runes.
    let mut v = file();
    v["stage_skills"] = json!(["duelist"]);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.stage_skills[0]"]);
}

#[test]
fn a_duplicate_is_refused() {
    let mut v = file();
    v["runes"] = json!(["bleed", "explosion", "bleed"]);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.runes[2]"]);
}

#[test]
fn a_list_over_its_budget_is_refused() {
    let mut v = file();
    let many: Vec<String> = (0..=MAX_STARTING_UNLOCKS)
        .map(|i| format!("w{i}"))
        .collect();
    v["weapons"] = json!(many);
    let issues = paths(parse(&v).unwrap_err());
    assert_eq!(issues, ["$.weapons"]);
}

#[test]
fn a_file_that_is_too_large_is_refused_before_parsing() {
    let big = vec![b' '; MAX_STARTING_UNLOCKS_BYTES + 1];
    assert!(matches!(
        StartingUnlocks::parse_with(
            &big,
            PerkSets::bundled(),
            StageSkills::bundled(),
            Runes::bundled()
        ),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn the_versions_and_unknown_fields_fail_closed() {
    let mut v = file();
    v["schema_version"] = json!(9);
    assert!(matches!(parse(&v), Err(Error::UnsupportedVersion { .. })));
    let mut v = file();
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = file();
    v["weapons"] = json!("giant_sword");
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
}

#[test]
fn the_bundled_pack_lists_todays_seed() {
    let u = Pack::bundled()
        .starting_unlocks()
        .expect("the bundled pack has the file");
    assert_eq!(ids(u.weapons()), ["giant_sword"]);
    assert_eq!(ids(u.perk_sets()), ["duelist"]);
    assert_eq!(ids(u.stage_skills()), ["nova"]);
    assert_eq!(ids(u.runes()), ["explosion", "bleed", "boomerang"]);
    assert_eq!(RunContent::bundled().starting_unlocks(), Some(u));
    // The bundled file on its own is the pack's file.
    let on_its_own = StartingUnlocks::parse_with(
        StartingUnlocks::BUNDLED,
        PerkSets::bundled(),
        StageSkills::bundled(),
        Runes::bundled(),
    )
    .unwrap();
    assert_eq!(&on_its_own, u);
}

#[test]
fn a_pack_without_the_file_still_loads_and_reports_none() {
    let mut v: Value = serde_json::from_slice(&pack::export_bundled()).unwrap();
    v.as_object_mut()
        .unwrap()
        .remove("starting_unlocks")
        .expect("exported with the file");
    let pack = Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("a released pack");
    assert_eq!(pack.starting_unlocks(), None);
    assert_eq!(pack.content().starting_unlocks(), None);
}

#[test]
fn a_pack_refuses_what_the_file_refuses() {
    // The pack's own perk sets decide: an unknown perk set is refused with its place.
    let mut v: Value = serde_json::from_slice(&pack::export_bundled()).unwrap();
    v["starting_unlocks"]["perk_sets"] = json!(["not_a_real_id"]);
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Err(e) => assert!(
            e.to_string().contains("$.starting_unlocks.perk_sets[0]"),
            "{e}"
        ),
        Ok(_) => panic!("an unknown perk set was accepted"),
    }
}
