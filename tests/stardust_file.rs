//! The Stardust file (ADR-101 point 4, ADR-080, DS6 Q10): per mode, whether a failed run's bank
//! is on, the share a fail pays, the share it banks and the bank's cap, read from JSON as
//! untrusted bytes (ADR-020). The simulation never reads it; the app pays and banks.

use crate::common;
use serde_json::{Value, json};
use stella_rain_core::pack::Pack;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::stardust::{MAX_BANK_CAP, MAX_STARDUST_BYTES, Stardust};
use stella_rain_core::validate::Error;

fn file() -> Value {
    json!({
        "schema_version": 0,
        "sim_version": 0,
        "modes": [
            { "id": "time_limit", "bank": true, "pay_pct": 50, "bank_pct": 50, "bank_cap": 1000 },
            { "id": "trial", "bank": false, "pay_pct": 100, "bank_pct": 0, "bank_cap": 0 },
        ],
    })
}

fn parse(v: &Value) -> Result<Stardust, Error> {
    Stardust::parse(v.to_string().as_bytes())
}

fn paths(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

#[test]
fn a_valid_file_loads_and_keeps_its_hash() {
    let s = parse(&file()).unwrap();
    let m = s.mode("time_limit").expect("an entry");
    assert!(m.bank);
    assert_eq!((m.pay_pct, m.bank_pct, m.bank_cap), (50, 50, 1000));
    assert!(!s.mode("trial").unwrap().bank);
    assert_eq!(s.mode("tutorial"), None);
    let mut other = file();
    other["modes"][0]["bank_cap"] = json!(999);
    assert_ne!(s.sha256(), parse(&other).unwrap().sha256());
}

#[test]
fn a_bad_id_is_refused_with_its_path() {
    let mut v = file();
    v["modes"][1]["id"] = json!("not_a_mode");
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.modes[1].id"]);
    let mut v = file();
    v["modes"][1]["id"] = json!("Bad Id");
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.modes[1].id"]);
    let mut v = file();
    v["modes"][1]["id"] = json!("time_limit");
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.modes[1].id"]);
}

#[test]
fn an_amount_over_its_ceiling_is_refused_with_its_path() {
    let mut v = file();
    v["modes"][0]["bank_cap"] = json!(MAX_BANK_CAP + 1);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.modes[0].bank_cap"]);
    let mut v = file();
    v["modes"][0]["bank_cap"] = json!(MAX_BANK_CAP);
    assert!(parse(&v).is_ok());
}

#[test]
fn a_share_over_100_or_shares_summing_over_100_are_refused() {
    let mut v = file();
    v["modes"][0]["pay_pct"] = json!(101);
    v["modes"][0]["bank_pct"] = json!(0);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.modes[0].pay_pct"]);
    let mut v = file();
    v["modes"][0]["bank_pct"] = json!(101);
    v["modes"][0]["pay_pct"] = json!(0);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.modes[0].bank_pct"]);
    let mut v = file();
    v["modes"][0]["pay_pct"] = json!(60);
    v["modes"][0]["bank_pct"] = json!(41);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.modes[0]"]);
}

#[test]
fn size_versions_unknown_fields_and_wrong_types_fail_closed() {
    let big = vec![b' '; MAX_STARDUST_BYTES + 1];
    assert!(matches!(Stardust::parse(&big), Err(Error::TooLarge { .. })));
    let mut v = file();
    v["schema_version"] = json!(9);
    assert!(matches!(parse(&v), Err(Error::UnsupportedVersion { .. })));
    let mut v = file();
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = file();
    v["modes"][0]["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = file();
    v["modes"][0]["pay_pct"] = json!(-1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
}

#[test]
fn the_bundled_file_parses_and_is_the_bundled_packs_file() {
    let on_its_own = Stardust::parse(common::bundled_file("stardust")).unwrap();
    assert_eq!(&on_its_own, Stardust::bundled());
    assert_eq!(Pack::bundled().stardust(), &on_its_own);
    assert_eq!(RunContent::bundled().stardust(), &on_its_own);
    let m = on_its_own
        .mode("time_limit")
        .expect("time_limit has an entry");
    assert!(m.bank);
    assert_eq!((m.pay_pct, m.bank_pct, m.bank_cap), (50, 50, 1000));
}

#[test]
fn a_pack_without_the_file_still_loads_and_yields_the_default() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v.as_object_mut()
        .unwrap()
        .remove("stardust")
        .expect("exported with the file");
    let pack = Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("a released pack");
    assert_eq!(pack.stardust(), Stardust::bundled());
    assert_eq!(pack.content().stardust(), Stardust::bundled());
}

#[test]
fn a_pack_refuses_what_the_file_refuses_and_a_pack_file_can_differ() {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["stardust"]["modes"][0]["pay_pct"] = json!(101);
    match Pack::from_bytes(&serde_json::to_vec(&v).unwrap()) {
        Err(e) => assert!(e.to_string().contains("$.stardust.modes[0].pay_pct"), "{e}"),
        Ok(_) => panic!("a share over 100 was accepted"),
    }
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["stardust"]["modes"][0]["bank_cap"] = json!(7);
    let pack = Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(pack.stardust().mode("time_limit").unwrap().bank_cap, 7);
}
