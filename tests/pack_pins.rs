//! Test pins that follow the bundled pack (ADR-SAVE-4, ADR-SIM-3): stub, filled in a later commit.

pub(crate) fn computed() -> Vec<(&'static str, String)> {
    Vec::new()
}

pub(crate) fn render(_pins: &[(&str, String)]) -> String {
    String::new()
}

pub(crate) fn check(_file: &str, _computed: &[(&str, String)]) -> Result<(), String> {
    Ok(())
}

pub(crate) fn blessed(_key: &str) -> String {
    String::new()
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

#[test]
fn the_pack_pins_are_as_blessed() {
    let c = computed();
    assert_eq!(c.len(), 2);
    let _ = blessed("pack_sha256");
}
