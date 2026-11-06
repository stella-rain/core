//! Equipment mode drops (ADR-SIM-2 point 2, ADR-SIM-5 point 8, ADR-SIM-7): a star taken and a boss
//! kill give each member their own roll into their own inventory; offers mode does not move.

use stella_rain_core::engine::Engine;
use stella_rain_core::run_setup::{BuildMode, RunSetup};
use stella_rain_core::validate::Error;

fn equipment() -> RunSetup {
    RunSetup {
        build_mode: BuildMode::Equipment,
        ..RunSetup::default()
    }
}

fn messages(error: Error) -> Vec<(String, String)> {
    match error {
        Error::Invalid(found) => found.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not an Invalid error: {other:?}"),
    }
}

/// T1: an equipment setup starts at `sim_version` 2; the other refusals stay.
#[test]
fn an_equipment_setup_starts() {
    assert!(Engine::start(&equipment()).is_ok());
    assert!(Engine::start_party(&equipment(), 3).is_ok());

    let old = RunSetup {
        sim_version: 1,
        ..equipment()
    };
    assert_eq!(
        messages(Engine::start(&old).expect_err("refused")),
        [(
            "$.build_mode".to_string(),
            "equipment mode needs sim_version 2".to_string()
        )]
    );

    let offers_with_star = RunSetup {
        lumine: 1,
        lumine_star: Some("max_hp_common".into()),
        ..RunSetup::default()
    };
    assert_eq!(
        messages(Engine::start(&offers_with_star).expect_err("refused")),
        [(
            "$.lumine_star".to_string(),
            "needs equipment mode".to_string()
        )]
    );
}
