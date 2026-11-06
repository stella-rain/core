//! Every bench returns its measured result and ends with `teardown = support::forget_measured`
//! (ADR-SIM-6, core#375): the count covers the bench function alone, so the function must hand its
//! fixture back instead of dropping it, and the teardown, which runs outside the count, forgets it.
//! This file checks the text of every `#[bench::…]`; `.github/scripts/bench_drop_guard.py` checks
//! the measured profile. The bench files are also exactly the keys of `benches/budgets.toml`.

use std::fs;
use std::path::PathBuf;

const TEARDOWN: &str = "teardown = support::forget_measured";

/// The errors of one bench file's source: no bench at all, no `mod support;`, and each
/// `#[bench::<id>…]` attribute (possibly over several lines) without the teardown.
fn check(file: &str, src: &str) -> Vec<String> {
    let code: String = src
        .lines()
        .map(|l| l.find("//").map_or(l, |at| &l[..at]))
        .collect::<Vec<_>>()
        .join("\n");
    let mut errors = Vec::new();
    let mut found = 0;
    let mut rest = code.as_str();
    while let Some(at) = ["#[bench::", "#[benches::"]
        .iter()
        .filter_map(|p| rest.find(p))
        .min()
    {
        found += 1;
        let attr_start = &rest[at..];
        let name_at = attr_start.find("::").unwrap() + 2;
        let id: String = attr_start[name_at..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let kind = &attr_start[2..name_at - 2];
        // The attribute ends at the `)` that closes its first `(`; no `(` before the `]`, or
        // parentheses that never balance, leave it without a teardown.
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in attr_start.char_indices() {
            match c {
                ']' if depth == 0 => {
                    end = Some(i);
                    break;
                }
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let attr = &attr_start[..end.unwrap_or(name_at)];
        if !has_teardown(attr) {
            errors.push(format!("{file}: #[{kind}::{id}] has no {TEARDOWN}"));
        }
        rest = &attr_start[end.unwrap_or(name_at)..];
    }
    if found == 0 {
        errors.push(format!("{file}: has no #[bench::…] attribute"));
    }
    if !code.contains("mod support;") {
        errors.push(format!("{file}: does not declare `mod support;`"));
    }
    errors
}

/// The teardown, not a longer name that starts with it.
fn has_teardown(attr: &str) -> bool {
    attr.match_indices(TEARDOWN).any(|(at, _)| {
        !attr[at + TEARDOWN.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

/// The `[name]` table headers of a TOML file, in order.
fn table_keys(toml: &str) -> Vec<String> {
    toml.lines()
        .filter_map(|l| l.strip_prefix('[')?.strip_suffix(']'))
        .map(str::to_owned)
        .collect()
}

fn benches_dir() -> PathBuf {
    crate::data_root::data_root().join("benches")
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
fn checker_covers_the_plural_benches_attribute() {
    let src = "mod support;\n#[benches::many(args = [f(), g()])]\nfn a() {}\n";
    let errors = check("a.rs", src);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("#[benches::many]"), "{errors:?}");
}

#[test]
fn checker_fails_an_attribute_without_parentheses() {
    let src = "mod support;\n#[bench::x]\nfn x() {}\n#[bench::y(args = (f()), teardown = support::forget_measured)]\n";
    let errors = check("a.rs", src);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("#[bench::x]"), "{errors:?}");
}

#[test]
fn checker_fails_an_unbalanced_attribute() {
    let src = "mod support;\n#[bench::x(args = (f())\nfn x() {}\n// teardown = support::forget_measured\n";
    let errors = check("a.rs", src);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("#[bench::x]"), "{errors:?}");
}

#[test]
fn checker_ignores_a_trailing_comment_and_a_longer_name() {
    let trailing =
        "mod support;\n#[bench::x(args = (f()))] // teardown = support::forget_measured\n";
    assert_eq!(check("a.rs", trailing).len(), 1);
    let longer =
        "mod support;\n#[bench::x(args = (f()), teardown = support::forget_measured_v2)]\n";
    assert_eq!(check("a.rs", longer).len(), 1);
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

/// The bench targets that have a file in `benches/` but no key in `benches/budgets.toml` yet
/// (ADR-SIM-6): CI builds and counts only the budgeted ones, so these run locally until a
/// budget is added. Adding the budget removes the name here; the guard below fails until it does.
pub(crate) const UNBUDGETED: [&str; 1] = ["rollback_equip"];

/// The names of `unbudgeted` that are also budget `keys`.
fn listed_and_budgeted(unbudgeted: &[&str], keys: &[&str]) -> Vec<String> {
    unbudgeted
        .iter()
        .filter(|name| keys.contains(name))
        .map(|name| (*name).to_owned())
        .collect()
}

fn budget_keys() -> Vec<String> {
    let budgets = fs::read_to_string(benches_dir().join("budgets.toml")).expect("budgets.toml");
    let mut keys = table_keys(&budgets);
    keys.sort();
    keys
}

#[test]
fn bench_files_are_the_budget_keys() {
    let mut expected = budget_keys();
    expected.extend(UNBUDGETED.iter().map(|name| (*name).to_owned()));
    expected.sort();
    let files: Vec<String> = bench_files()
        .into_iter()
        .map(|(name, _)| name.trim_end_matches(".rs").to_owned())
        .collect();
    assert_eq!(
        files, expected,
        "benches/*.rs differ from the budgets.toml keys and UNBUDGETED"
    );
}

#[test]
fn unbudgeted_names_are_not_budget_keys() {
    let keys = budget_keys();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let both = listed_and_budgeted(&UNBUDGETED, &keys);
    assert!(
        both.is_empty(),
        "{}",
        both.iter()
            .map(|name| format!("remove `{name}` from UNBUDGETED: it has a budget"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn every_unbudgeted_name_has_a_bench_file() {
    let files: Vec<String> = bench_files()
        .into_iter()
        .map(|(name, _)| name.trim_end_matches(".rs").to_owned())
        .collect();
    for name in UNBUDGETED {
        assert!(
            files.iter().any(|f| f == name),
            "UNBUDGETED names `{name}` but benches/{name}.rs does not exist"
        );
    }
}

#[test]
fn listed_and_budgeted_names_the_overlap() {
    assert_eq!(listed_and_budgeted(&["a"], &["a", "b"]), vec!["a"]);
}

#[test]
fn listed_and_budgeted_is_empty_without_overlap() {
    assert_eq!(listed_and_budgeted(&["a"], &["b"]), Vec::<String>::new());
}
