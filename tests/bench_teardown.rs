//! Every bench forgets its measured result (BG7a, DS66-3, ADR-SIM-8): a `#[bench::…]` without
//! `teardown = support::forget_measured` would count the drop of its result again (core#375).
//! The bench files are also exactly the keys of `benches/budgets.toml`.

use std::fs;
use std::path::PathBuf;

const TEARDOWN: &str = "teardown = support::forget_measured";

/// The errors of one bench file's source: no bench at all, no `mod support;`, and each
/// `#[bench::<id>…]` attribute (possibly over several lines) without the teardown.
fn check(file: &str, src: &str) -> Vec<String> {
    let code: String = src
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("//") {
                ""
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut errors = Vec::new();
    let mut found = 0;
    let mut rest = code.as_str();
    while let Some(at) = rest.find("#[bench::") {
        found += 1;
        let attr_start = &rest[at..];
        let id: String = attr_start["#[bench::".len()..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let mut depth = 0usize;
        let mut end = attr_start.len();
        for (i, c) in attr_start.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !attr_start[..end].contains(TEARDOWN) {
            errors.push(format!("{file}: #[bench::{id}] has no {TEARDOWN}"));
        }
        rest = &attr_start[end..];
    }
    if found == 0 {
        errors.push(format!("{file}: has no #[bench::…] attribute"));
    }
    if !code.contains("mod support;") {
        errors.push(format!("{file}: does not declare `mod support;`"));
    }
    errors
}

/// The `[name]` table headers of a TOML file, in order.
fn table_keys(toml: &str) -> Vec<String> {
    toml.lines()
        .filter_map(|l| l.strip_prefix('[')?.strip_suffix(']'))
        .map(str::to_owned)
        .collect()
}

fn benches_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches")
}

/// The bench files directly in `benches/` (not `benches/support/`), sorted by name.
fn bench_files() -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = fs::read_dir(benches_dir())
        .expect("benches/ is readable")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).expect("bench file is UTF-8"))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn checker_accepts_a_bench_with_the_teardown() {
    let src = "mod support;\n#[bench::one(args = (f()), teardown = support::forget_measured)]\nfn a() {}\n";
    assert_eq!(check("a.rs", src), Vec::<String>::new());
}

#[test]
fn checker_names_the_bench_without_the_teardown() {
    let src = "mod support;\n#[bench::one(args = (f()))]\nfn a() {}\n";
    assert_eq!(
        check("a.rs", src),
        vec!["a.rs: #[bench::one] has no teardown = support::forget_measured".to_owned()]
    );
}

#[test]
fn checker_reads_an_attribute_over_several_lines() {
    let src = "mod support;\n#[bench::one(\n    args = (f()),\n    teardown = support::forget_measured\n)]\nfn a() {}\n";
    assert_eq!(check("a.rs", src), Vec::<String>::new());
}

#[test]
fn checker_does_not_borrow_the_teardown_of_the_next_bench() {
    let src = "mod support;\n#[bench::one(args = (f()))]\n#[bench::two(args = (g()), teardown = support::forget_measured)]\nfn a() {}\n";
    let errors = check("a.rs", src);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("#[bench::one]"), "{errors:?}");
}

#[test]
fn checker_requires_mod_support() {
    let src = "#[bench::one(args = (f()), teardown = support::forget_measured)]\nfn a() {}\n";
    let errors = check("a.rs", src);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("mod support;"), "{errors:?}");
}

#[test]
fn checker_requires_a_bench() {
    let errors = check("a.rs", "mod support;\nfn a() {}\n");
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("no #[bench::"), "{errors:?}");
}

#[test]
fn checker_ignores_commented_out_benches() {
    let src = "mod support;\n// #[bench::old(args = (f()))]\n#[bench::one(args = (f()), teardown = support::forget_measured)]\n";
    assert_eq!(check("a.rs", src), Vec::<String>::new());
}

#[test]
fn table_keys_reads_table_headers_only() {
    let toml = "# [comment]\n[a]\nx = 1\n[b_c]\ny = 2\n";
    assert_eq!(table_keys(toml), vec!["a", "b_c"]);
}

#[test]
fn every_bench_names_the_teardown() {
    let errors: Vec<String> = bench_files()
        .iter()
        .flat_map(|(name, src)| check(name, src))
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn bench_files_are_the_budget_keys() {
    let budgets = fs::read_to_string(benches_dir().join("budgets.toml")).expect("budgets.toml");
    let mut keys = table_keys(&budgets);
    keys.sort();
    let files: Vec<String> = bench_files()
        .into_iter()
        .map(|(name, _)| name.trim_end_matches(".rs").to_owned())
        .collect();
    assert_eq!(files, keys, "benches/*.rs and the budgets.toml keys differ");
}
