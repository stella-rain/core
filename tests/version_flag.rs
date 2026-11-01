//! The development flag (ADR-REPL-9): `version::DEVELOPMENT` is one plain line the app can match,
//! and it is set exactly while the live version has no frozen corpus.

use stella_rain_core::version;

const SOURCE: &str = include_str!("../src/version.rs");

#[test]
fn the_flag_is_one_plain_line_the_app_can_match() {
    let lines: Vec<&str> = SOURCE.lines().collect();
    let declared = lines
        .iter()
        .filter(|l| l.starts_with("pub const DEVELOPMENT"))
        .count();
    assert_eq!(declared, 1, "exactly one declaration of the flag");
    assert_eq!(
        lines
            .iter()
            .filter(|l| **l == "pub const DEVELOPMENT: bool = true;")
            .count(),
        1,
        "the flag is set, on its own line"
    );
    let sim: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("pub const SIM_VERSION"))
        .collect();
    assert_eq!(sim.len(), 1);
    let digits = sim[0]
        .strip_prefix("pub const SIM_VERSION: u32 = ")
        .and_then(|rest| rest.strip_suffix(';'))
        .expect("the SIM_VERSION line keeps the shape the release script reads");
    assert!(!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()));
}

fn corpus_dir(n: u32) -> std::path::PathBuf {
    crate::data_root::data_root().join(format!("tests/corpus/v{n}"))
}

#[test]
fn the_flag_is_set_exactly_when_the_live_version_is_not_frozen() {
    let frozen = corpus_dir(version::SIM_VERSION).is_dir();
    assert_eq!(version::DEVELOPMENT, !frozen);
}

#[test]
fn the_older_versions_are_frozen_and_the_live_one_is_not() {
    for n in version::FIRST_SIM_VERSION..version::SIM_VERSION {
        assert!(corpus_dir(n).is_dir(), "v{n} is frozen");
    }
    if version::DEVELOPMENT {
        assert!(!corpus_dir(version::SIM_VERSION).exists());
    }
}
