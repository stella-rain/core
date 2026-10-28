//! Checks that core's `docs/tuning.md` matches `content-pack catalogue` output (ADR-126).

#![cfg(not(target_os = "ios"))]

use std::path::Path;
use std::process::Command;

#[test]
fn tuning_stale() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tuning_path = manifest_dir.join("docs/tuning.md");
    let output = Command::new(env!("CARGO_BIN_EXE_content-pack"))
        .arg("catalogue")
        .output()
        .expect("run content-pack catalogue");
    assert!(
        output.status.success(),
        "content-pack catalogue failed with status: {}",
        output.status
    );
    let expected = String::from_utf8(output.stdout).expect("catalogue output is valid utf-8");
    if std::env::var("UPDATE_TUNING").as_deref() == Ok("1") {
        if let Some(parent) = tuning_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&tuning_path, &expected).expect("write docs/tuning.md");
    }
    let actual = std::fs::read_to_string(&tuning_path).expect(
        "read docs/tuning.md; run `cargo run --bin content-pack catalogue > docs/tuning.md` to \
         generate it",
    );
    assert_eq!(
        actual, expected,
        "docs/tuning.md is stale. Run `cargo run --bin content-pack catalogue > docs/tuning.md` to \
         regenerate"
    );
}
