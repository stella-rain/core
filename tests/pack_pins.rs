//! Test pins that follow the bundled pack (ADR-SAVE-4, ADR-SIM-3). They move with every pack
//! bless, so `cargo xtask bless` writes `tests/pack_pins.toml` (`UPDATE_PACK_PINS=1`) and
//! `--check` reads it; a pin that does not move with the pack stays in its own test.

use std::path::PathBuf;
use stella_rain_core::content_hash::to_hex;
use stella_rain_core::pack::Pack;

const HEADER: &str = "\
# Test pins that follow the bundled pack: they move with every pack bless, so `cargo xtask bless`
# writes them (`UPDATE_PACK_PINS=1 cargo test --test integration pack_pins::`) and `--check`
# reads them. Never edit by hand; a pin that does not move with the pack stays in its test.
";

fn path() -> PathBuf {
    crate::data_root::data_root().join("tests/pack_pins.toml")
}

/// The pins as the code computes them now, in file order.
pub(crate) fn computed() -> Vec<(&'static str, String)> {
    vec![
        ("pack_sha256", to_hex(&Pack::bundled().sha256())),
        (
            "savepoint_v2_trial_300",
            crate::savepoint_proof::live_v2_digest(),
        ),
    ]
}

/// The file text for these pins: the header, then one `key = "value"` line each.
pub(crate) fn render(pins: &[(&str, String)]) -> String {
    let mut out = String::from(HEADER);
    for (key, value) in pins {
        out.push_str(&format!("{key} = \"{value}\"\n"));
    }
    out
}

/// Whether `file` holds exactly `computed`: every difference is named, with both values.
pub(crate) fn check(file: &str, computed: &[(&str, String)]) -> Result<(), String> {
    let table: toml::Table = file
        .parse()
        .map_err(|e| format!("tests/pack_pins.toml is not TOML: {e}"))?;
    let mut wrong = String::new();
    for (key, value) in computed {
        match table.get(*key).and_then(toml::Value::as_str) {
            None => wrong.push_str(&format!("{key}: missing from the file, computed {value}\n")),
            Some(have) if have != value => {
                wrong.push_str(&format!("{key}: file {have}, computed {value}\n"));
            }
            Some(_) => {}
        }
    }
    for key in table.keys() {
        if !computed.iter().any(|(k, _)| k == key) {
            wrong.push_str(&format!("{key}: in the file, not computed\n"));
        }
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "tests/pack_pins.toml is not as computed:\n{wrong}run `cargo xtask bless`"
        ))
    }
}

/// The blessed value of `key`, read from the file at run time.
pub(crate) fn blessed(key: &str) -> String {
    let text = std::fs::read_to_string(path()).unwrap_or_else(|e| {
        panic!("tests/pack_pins.toml is unreadable ({e}): run `cargo xtask bless`")
    });
    let table: toml::Table = text.parse().expect("tests/pack_pins.toml is TOML");
    match table.get(key).and_then(toml::Value::as_str) {
        Some(v) => v.to_string(),
        None => panic!("tests/pack_pins.toml has no `{key}`: run `cargo xtask bless`"),
    }
}

#[expect(clippy::disallowed_methods, reason = "test knob")]
fn updating() -> bool {
    std::env::var("UPDATE_PACK_PINS").as_deref() == Ok("1")
}

#[test]
fn a_file_that_disagrees_with_the_computed_pins_is_stale() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let computed = [
        ("pack_sha256", a.clone()),
        ("savepoint_v2_trial_300", b.clone()),
    ];
    let file = render(&computed);
    assert_eq!(check(&file, &computed), Ok(()));
    let bad = file.replace(&b, &format!("{}c", &b[..63]));
    let err = check(&bad, &computed).unwrap_err();
    assert!(
        err.contains("savepoint_v2_trial_300") && err.contains(&b),
        "{err}"
    );
    assert!(err.contains("cargo xtask bless"), "{err}");
    let missing: String = file
        .lines()
        .filter(|l| !l.starts_with("savepoint"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert!(check(&missing, &computed).unwrap_err().contains("missing"));
    let extra = format!("{file}x = \"1\"\n");
    assert!(check(&extra, &computed).unwrap_err().contains('x'));
}

#[test]
fn the_rendered_file_parses_back() {
    let computed = [
        ("pack_sha256", "a".repeat(64)),
        ("savepoint_v2_trial_300", "b".repeat(64)),
    ];
    let text = render(&computed);
    let table: toml::Table = text.parse().expect("TOML");
    assert_eq!(table.len(), 2);
    for (k, v) in &computed {
        assert_eq!(table[*k].as_str(), Some(v.as_str()));
    }
    assert!(text.ends_with("\"\n") && !text.ends_with("\n\n") && !text.contains('\r'));
}

/// `UPDATE_PACK_PINS=1` writes the file; otherwise the file is compared with the computed pins.
#[test]
fn the_pack_pins_are_as_blessed() {
    let computed = computed();
    if updating() {
        std::fs::write(path(), render(&computed)).expect("tests/pack_pins.toml is writable");
    }
    let file = std::fs::read_to_string(path()).unwrap_or_default();
    if let Err(e) = check(&file, &computed) {
        panic!("{e}");
    }
}
