//! The starting unlocks file (ADR-080, ADR-099 point 4a, ADR-124): the weapons, perk sets,
//! stage skills and runes a new save begins with, read from JSON as untrusted bytes (ADR-020).
//! The simulation never reads it; the app seeds a new save from the pack.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::pack::Pack;
use stella_rain_core::perk_sets::PerkSets;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::runes::Runes;
use stella_rain_core::stage_skill::StageSkills;
use stella_rain_core::starting_unlocks::{
    MAX_STARTING_UNLOCKS, MAX_STARTING_UNLOCKS_BYTES, StartingUnlocks,
};
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;
use stella_rain_core::weapons::Weapons;

fn file() -> Value {
    json!({
        "free_weapon": "giant_sword",
        "free_perk_set": "duelist",
        "weapons": ["giant_sword"],
        "perk_sets": ["duelist"],
        "stage_skills": ["nova"],
        "runes": ["explosion", "bleed", "boomerang"],
    })
}

fn parse_at(v: &Value, versions: Versions) -> Result<StartingUnlocks, Error> {
    StartingUnlocks::parse_with(
        v.to_string().as_bytes(),
        versions,
        PerkSets::bundled(),
        StageSkills::bundled(),
        Runes::bundled(),
        Weapons::bundled(),
    )
}

fn parse(v: &Value) -> Result<StartingUnlocks, Error> {
    parse_at(v, Versions::CURRENT)
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
    // The free picks must be in their lists (DS128): an empty weapons or perk_sets list is
    // refused at the free pick it cannot hold.
    let mut v = file();
    for list in ["stage_skills", "runes"] {
        v[list] = json!([]);
    }
    assert!(parse(&v).unwrap().runes().is_empty());
    let mut v = file();
    v["weapons"] = json!([]);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.free_weapon"]);
    let mut v = file();
    v["perk_sets"] = json!([]);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.free_perk_set"]);
}

#[test]
fn an_id_its_source_does_not_know_is_refused() {
    // Pushed onto each list, so a free pick stays in its list and only the unknown ID is refused.
    for (kind, n) in [
        ("weapons", 1),
        ("perk_sets", 1),
        ("stage_skills", 1),
        ("runes", 3),
    ] {
        let mut v = file();
        v[kind].as_array_mut().unwrap().push(json!("not_a_real_id"));
        assert_eq!(paths(parse(&v).unwrap_err()), [format!("$.{kind}[{n}]")]);
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
            Versions::CURRENT,
            PerkSets::bundled(),
            StageSkills::bundled(),
            Runes::bundled(),
            Weapons::bundled()
        ),
        Err(Error::TooLarge { .. })
    ));
}

#[test]
fn the_versions_and_unknown_fields_fail_closed() {
    let mut v = file();
    assert!(matches!(
        parse_at(
            &v,
            Versions {
                schema_version: 9,
                sim_version: 0
            }
        ),
        Err(Error::UnsupportedVersion { .. })
    ));
    v["schema_version"] = json!(0);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
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
        common::bundled_file("starting_unlocks"),
        Versions::CURRENT,
        PerkSets::bundled(),
        StageSkills::bundled(),
        Runes::bundled(),
        Weapons::bundled(),
    )
    .unwrap();
    assert_eq!(&on_its_own, u);
}

#[test]
fn a_pack_without_the_file_still_loads_and_reports_none() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
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
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["starting_unlocks"]["perk_sets"] = json!(["not_a_real_id"]);
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Err(e) => assert!(
            e.to_string().contains("$.starting_unlocks.perk_sets[0]"),
            "{e}"
        ),
        Ok(_) => panic!("an unknown perk set was accepted"),
    }
}

// --- The free weapon and the free perk set (CS1b-2) ----------------------------------------

#[test]
fn the_free_picks_are_required_ids_the_pack_holds() {
    let u = parse(&file()).unwrap();
    assert_eq!(u.free_weapon().0, "giant_sword");
    assert_eq!(u.free_perk_set().0, "duelist");
    for (field, bad) in [
        ("free_weapon", "no_such"),
        ("free_weapon", "Bad"),
        ("free_perk_set", "no_such"),
        ("free_perk_set", "Bad"),
    ] {
        let mut v = file();
        v[field] = json!(bad);
        assert_eq!(paths(parse(&v).unwrap_err()), [format!("$.{field}")]);
    }
    for field in ["free_weapon", "free_perk_set"] {
        let mut v = file();
        v.as_object_mut().unwrap().remove(field);
        assert!(matches!(parse(&v), Err(Error::Parse { .. })), "{field}");
    }
}

#[test]
fn the_bundled_pack_names_the_free_picks() {
    let u = Pack::bundled()
        .starting_unlocks()
        .expect("the bundled slot");
    assert_eq!(u.free_weapon().0, "giant_sword");
    assert_eq!(u.free_perk_set().0, "duelist");
}

// --- A free pick is one of its starting list's IDs (ECO1, DS128) ---------------------------

fn issues(e: Error) -> Vec<(String, String)> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

#[test]
fn a_free_pick_outside_its_starting_list_is_refused() {
    let mut v = file();
    v["free_weapon"] = json!("giant_hammer");
    assert_eq!(
        issues(parse(&v).unwrap_err()),
        [(
            "$.free_weapon".to_string(),
            "not in this file's `weapons` list".to_string()
        )]
    );
    let mut v = file();
    v["free_perk_set"] = json!("vanguard");
    assert_eq!(
        issues(parse(&v).unwrap_err()),
        [(
            "$.free_perk_set".to_string(),
            "not in this file's `perk_sets` list".to_string()
        )]
    );
    let mut v = file();
    v["free_weapon"] = json!("giant_hammer");
    v["free_perk_set"] = json!("vanguard");
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.free_weapon", "$.free_perk_set"]
    );
}

#[test]
fn a_free_pick_anywhere_in_its_list_is_accepted() {
    let mut v = file();
    v["weapons"] = json!(["giant_hammer", "giant_sword"]);
    v["perk_sets"] = json!(["vanguard", "duelist"]);
    let u = parse(&v).unwrap();
    assert_eq!(u.free_weapon().0, "giant_sword");
    assert_eq!(u.free_perk_set().0, "duelist");
}

#[test]
fn a_pack_whose_free_pick_is_not_a_starting_unlock_is_refused() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["starting_unlocks"]["free_weapon"] = json!("giant_hammer");
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Err(e) => {
            let text = e.to_string();
            assert!(text.contains("$.starting_unlocks.free_weapon"), "{text}");
            assert!(text.contains("not in this file's `weapons` list"), "{text}");
        }
        Ok(_) => panic!("a free weapon outside its list was accepted"),
    }
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["starting_unlocks"]["free_perk_set"] = json!("vanguard");
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Err(e) => {
            let text = e.to_string();
            assert!(text.contains("$.starting_unlocks.free_perk_set"), "{text}");
            assert!(
                text.contains("not in this file's `perk_sets` list"),
                "{text}"
            );
        }
        Ok(_) => panic!("a free perk set outside its list was accepted"),
    }
    assert!(
        Pack::from_bytes(Pack::BUNDLED)
            .unwrap()
            .starting_unlocks()
            .is_some()
    );
}
