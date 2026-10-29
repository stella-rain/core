//! `cargo xtask bless` regenerates every generated set of core (schema, corpus, goldens, the
//! content pack and the bench pins); `--check` reports any set that
//! is not as committed. Std only: it shells out to `cargo` and `git`.

use std::process::{Command, ExitCode};

/// Where each generated set lives, in the order `bless` regenerates them.
const SETS: [(&str, &str); 5] = [
    ("schema", "schema/"),
    ("corpus", "tests/corpus/"),
    ("golden", "tests/golden/"),
    ("pack", "content/pack.json"),
    ("bench pins", "tests/bench_pins.toml"),
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

/// One step of `bless`, run in order.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    ExportPack,
    Test {
        env: Option<(&'static str, &'static str)>,
        /// Run without the `CI` variable (a knob under test refuses to write when it is set).
        drop_ci: bool,
        filter: &'static str,
    },
}

/// The steps of `bless`. First: the tests below read the pack compiled into core, so they must
/// be built on the pack of the files as they are now.
fn steps() -> Vec<Step> {
    vec![
        Step::ExportPack,
        Step::Test {
            env: Some(("UPDATE_SCHEMA", "1")),
            drop_ci: false,
            filter: "schema::",
        },
        Step::Test {
            env: Some(("UPDATE_CORPUS", "1")),
            drop_ci: false,
            filter: "corpus::",
        },
        Step::Test {
            env: Some(("INSTA_UPDATE", "always")),
            drop_ci: false,
            filter: "golden::",
        },
        // Last: the pins depend on the code and the compiled-in pack only, not on the three sets
        // above. `CI` is removed because the knob refuses to write when it is set, and
        // `bless --check` runs under CI.
        Step::Test {
            env: Some(("UPDATE_BENCH_PINS", "1")),
            drop_ci: true,
            filter: "bench_fixture_pins::",
        },
    ]
}

impl Step {
    fn command(&self) -> Command {
        match self {
            Step::ExportPack => {
                let mut cmd = Command::new("cargo");
                cmd.args([
                    "run",
                    "--quiet",
                    "--locked",
                    "--bin",
                    "content-pack",
                    "export",
                    "content",
                ]);
                cmd
            }
            Step::Test {
                env,
                drop_ci,
                filter,
            } => {
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
                if *drop_ci {
                    cmd.env_remove("CI");
                }
                cmd
            }
        }
    }

    fn run(&self) -> Result<(), String> {
        let mut cmd = self.command();
        match self {
            Step::ExportPack => {
                let out = cmd.output().map_err(|e| format!("content-pack: {e}"))?;
                if !out.status.success() {
                    return Err(format!("content-pack export failed: {}", out.status));
                }
                std::fs::write("content/pack.json", out.stdout)
                    .map_err(|e| format!("content/pack.json: {e}"))
            }
            Step::Test { .. } => run(&mut cmd),
        }
    }
}

fn regenerate() -> Result<(), String> {
    steps().iter().try_for_each(Step::run)
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
            let names: Vec<_> = SETS.iter().map(|(name, _)| *name).collect();
            println!(
                "blessed: {}
Commit body: Corpus regenerated: <why>",
                names.join(", ")
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
            "tests/corpus/current/stages/a/hashes.bin",
            "tests/corpus/current/stages/b/hashes.bin",
            "schema/stage.json",
            "tests/golden/x.snap",
            "content/pack.json",
            "src/lib.rs",
        ]);
        assert_eq!(stale_sets(&paths), ["schema", "corpus", "golden", "pack"]);
        let pins = args(&["tests/bench_pins.toml", "tests/golden/x.snap"]);
        assert_eq!(stale_sets(&pins), ["golden", "bench pins"]);
        assert!(stale_sets(&args(&["README.md"])).is_empty());
    }

    fn test_step(env: (&'static str, &'static str), filter: &'static str) -> Step {
        Step::Test {
            env: Some(env),
            drop_ci: false,
            filter,
        }
    }

    #[test]
    fn bless_runs_pack_schema_corpus_golden_in_that_order() {
        assert_eq!(
            steps(),
            [
                Step::ExportPack,
                test_step(("UPDATE_SCHEMA", "1"), "schema::"),
                test_step(("UPDATE_CORPUS", "1"), "corpus::"),
                test_step(("INSTA_UPDATE", "always"), "golden::"),
                Step::Test {
                    env: Some(("UPDATE_BENCH_PINS", "1")),
                    drop_ci: true,
                    filter: "bench_fixture_pins::",
                },
            ]
        );
    }

    fn pins_step() -> Option<usize> {
        steps().iter().position(|s| {
            matches!(
                s,
                Step::Test {
                    filter: "bench_fixture_pins::",
                    ..
                }
            )
        })
    }

    /// The set a step writes.
    fn writes(step: &Step) -> &'static str {
        match step {
            Step::ExportPack => "pack",
            Step::Test { filter, .. } => match *filter {
                "schema::" => "schema",
                "corpus::" => "corpus",
                "golden::" => "golden",
                "bench_fixture_pins::" => "bench pins",
                other => panic!("step {other} writes no known set"),
            },
        }
    }

    #[test]
    fn bless_runs_the_bench_pins_step_with_its_knob() {
        let i = pins_step().expect("no bench_fixture_pins:: step");
        assert!(matches!(
            &steps()[i],
            Step::Test {
                env: Some(("UPDATE_BENCH_PINS", "1")),
                ..
            }
        ));
    }

    #[test]
    fn the_bench_pins_step_runs_after_the_pack_export() {
        let pack = steps().iter().position(|s| *s == Step::ExportPack);
        assert!(pins_step().expect("no pins step") > pack.expect("no pack step"));
    }

    #[test]
    fn every_set_has_exactly_one_step_that_writes_it() {
        let steps = steps();
        for (name, _) in SETS {
            let n = steps.iter().filter(|s| writes(s) == name).count();
            assert_eq!(n, 1, "set {name} is written by {n} steps");
        }
        assert_eq!(
            SETS.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            ["schema", "corpus", "golden", "pack", "bench pins"]
        );
    }

    #[test]
    fn the_message_names_each_stale_set_and_the_fix() {
        let m = stale_message(&["corpus", "pack"]);
        assert!(m.contains("corpus") && m.contains("pack"), "{m}");
        assert!(m.contains("cargo xtask bless"), "{m}");
        let m = stale_message(&["bench pins"]);
        assert!(
            m.contains("bench pins") && m.contains("cargo xtask bless"),
            "{m}"
        );
    }

    #[test]
    fn only_the_pins_step_runs_without_ci() {
        let ci_removed = |s: &Step| {
            s.command()
                .get_envs()
                .any(|(k, v)| k == "CI" && v.is_none())
        };
        for step in steps() {
            assert_eq!(ci_removed(&step), writes(&step) == "bench pins", "{step:?}");
        }
        let pins = steps().remove(pins_step().expect("pins step"));
        let cmd = pins.command();
        let knob = cmd.get_envs().find(|(k, _)| *k == "UPDATE_BENCH_PINS");
        assert_eq!(knob.and_then(|(_, v)| v), Some(std::ffi::OsStr::new("1")));
    }
}
