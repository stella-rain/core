//! `cargo xtask bless` regenerates every generated set of core; `--check` reports any set that
//! is not as committed. Std only: it shells out to `cargo` and `git`.

use std::process::{Command, ExitCode};

/// Where each generated set lives, in the order `bless` regenerates them.
const SETS: [(&str, &str); 4] = [
    ("schema", "schema/"),
    ("corpus", "tests/corpus/"),
    ("golden", "tests/golden/"),
    ("pack", "content/pack.json"),
];

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Bless,
    Check,
}

fn parse(args: &[String]) -> Result<Mode, String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["bless"] => Ok(Mode::Bless),
        ["bless", "--check"] => Ok(Mode::Check),
        _ => Err(format!("unknown arguments {args:?}")),
    }
}

/// The sets that own at least one of `paths`, once each, in `SETS` order.
fn stale_sets(paths: &[String]) -> Vec<&'static str> {
    SETS.iter()
        .filter(|(_, prefix)| paths.iter().any(|p| p.starts_with(prefix)))
        .map(|(name, _)| *name)
        .collect()
}

fn stale_message(sets: &[&str]) -> String {
    format!(
        "stale generated sets: {}\nrun `cargo xtask bless`, review the diff and commit it with \
         `Corpus regenerated: <why>` in the commit body",
        sets.join(", ")
    )
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed: {status}"))
    }
}

/// Changed or untracked files under the generated sets, from `git status`.
fn dirty_paths() -> Result<Vec<String>, String> {
    let mut git = Command::new("git");
    git.args(["status", "--porcelain", "--untracked-files=all", "--"]);
    git.args(SETS.map(|(_, path)| path));
    let out = git.output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git status failed: {}", out.status));
    }
    // "XY path": the path starts at column 3.
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.get(3..).unwrap_or("").to_string())
        .collect())
}

fn cargo_test(env: Option<(&str, &str)>, filter: &str) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.args([
        "test",
        "--locked",
        "--all-features",
        "--test",
        "integration",
        filter,
    ]);
    if let Some((k, v)) = env {
        cmd.env(k, v);
    }
    run(&mut cmd)
}

fn export_pack() -> Result<(), String> {
    let out = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "--locked",
            "--bin",
            "content-pack",
            "export",
            "content",
        ])
        .output()
        .map_err(|e| format!("content-pack: {e}"))?;
    if !out.status.success() {
        return Err(format!("content-pack export failed: {}", out.status));
    }
    std::fs::write("content/pack.json", out.stdout).map_err(|e| format!("content/pack.json: {e}"))
}

fn regenerate() -> Result<(), String> {
    cargo_test(Some(("UPDATE_SCHEMA", "1")), "schema::")?;
    cargo_test(Some(("UPDATE_CORPUS", "1")), "corpus::")?;
    cargo_test(Some(("INSTA_UPDATE", "always")), "golden::")?;
    export_pack()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match parse(&args) {
        Ok(mode) => mode,
        Err(e) => {
            eprintln!(
                "{e}
usage: cargo xtask bless [--check]"
            );
            return ExitCode::from(2);
        }
    };
    // Dirty before regenerating counts as stale too: regenerating would hide a hand edit.
    let result = (|| {
        let before = if mode == Mode::Check {
            dirty_paths()?
        } else {
            Vec::new()
        };
        regenerate()?;
        Ok::<_, String>([before, dirty_paths()?].concat())
    })();
    match (result, mode) {
        (Err(e), _) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
        (Ok(_), Mode::Bless) => {
            println!(
                "blessed: schema, corpus, golden, pack
Commit body: Corpus regenerated: <why>"
            );
            ExitCode::SUCCESS
        }
        (Ok(paths), Mode::Check) => {
            let sets = stale_sets(&paths);
            if sets.is_empty() {
                println!("generated sets are current");
                ExitCode::SUCCESS
            } else {
                eprintln!("{}", stale_message(&sets));
                ExitCode::FAILURE
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bless_alone_blesses_and_with_check_checks() {
        assert_eq!(parse(&args(&["bless"])), Ok(Mode::Bless));
        assert_eq!(parse(&args(&["bless", "--check"])), Ok(Mode::Check));
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            &[][..],
            &["nope"],
            &["bless", "--force"],
            &["bless", "--check", "x"],
            &["--check"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn changed_paths_name_their_sets_once_in_order() {
        let paths = args(&[
            "tests/corpus/v0/a/hashes.txt",
            "tests/corpus/v0/b/hashes.txt",
            "schema/stage.json",
            "tests/golden/x.snap",
            "content/pack.json",
            "src/lib.rs",
        ]);
        assert_eq!(stale_sets(&paths), ["schema", "corpus", "golden", "pack"]);
        assert!(stale_sets(&args(&["README.md"])).is_empty());
    }

    #[test]
    fn the_message_names_each_stale_set_and_the_fix() {
        let m = stale_message(&["corpus", "pack"]);
        assert!(m.contains("corpus") && m.contains("pack"), "{m}");
        assert!(m.contains("cargo xtask bless"), "{m}");
    }
}
