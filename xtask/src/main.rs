//! `cargo xtask bless` regenerates every generated set of core (schema, corpus, goldens, the
//! content pack and the bench pins); `--check` reports any set that
//! is not as committed. `cargo xtask bless --freeze v<N>` records `tests/corpus/current/` into a
//! new frozen `tests/corpus/v<N>/` (ADR-SIM-3): it refuses an existing `v<N>/` and a `current/`
//! that is not what bless writes, then runs the record test of core, which writes the folder and
//! its manifest. Std only: it shells out to `cargo` and `git`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, ExitCode};

/// Where each generated set lives, in the order `bless` regenerates them.
const SETS: [(&str, &str); 5] = [
    ("schema", "schema/"),
    ("corpus", "tests/corpus/current/"),
    ("golden", "tests/golden/"),
    ("pack", "content/pack.json"),
    ("bench pins", "tests/bench_pins.toml"),
];

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Bless,
    Check,
    Freeze(String),
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

/// The corpus folder of the frozen versions, `tests/corpus/v<N>/`, N >= 1: immutable, so `bless`
/// never writes it and fails when it is dirty.
const CORPUS: &str = "tests/corpus/";

/// A frozen folder is `v<N>` with N >= 1, digits only, no leading zero.
fn is_frozen_name(name: &str) -> bool {
    name.strip_prefix('v').is_some_and(|n| {
        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && !n.starts_with('0')
    })
}

/// The paths among `paths` that are inside a frozen corpus folder.
fn frozen_dirty(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|p| {
            p.strip_prefix(CORPUS)
                .and_then(|rest| rest.split('/').next())
                .is_some_and(is_frozen_name)
        })
        .cloned()
        .collect()
}

fn frozen_check() -> Result<(), String> {
    let dirty = frozen_dirty(&dirty_in(&[CORPUS])?);
    if dirty.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "bless must never change the frozen corpus tests/corpus/v1/: {}",
            dirty.join(", ")
        ))
    }
}

/// Changed or untracked files under the generated sets, from `git status`.
fn dirty_paths() -> Result<Vec<String>, String> {
    dirty_in(&SETS.map(|(_, path)| path))
}

fn dirty_in(pathspecs: &[&str]) -> Result<Vec<String>, String> {
    let mut git = Command::new("git");
    git.args(["status", "--porcelain", "--untracked-files=all", "--"]);
    git.args(pathspecs);
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
    /// Writes `tests/corpus/<name>/` from `current/`: the record test, under its knob.
    Record(String),
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

/// The steps of `bless --freeze <name>`: the pack and the corpus as bless regenerates them, then
/// the record. The schema, goldens and bench pins are not touched.
fn freeze_steps(_name: &str) -> Vec<Step> {
    Vec::new()
}

/// Why `name` cannot be frozen in `corpus` (the `tests/corpus/` folder), if it cannot.
fn freeze_refusal(_corpus: &Path, _name: &str) -> Result<(), String> {
    Ok(())
}

/// Every file under `root` with its bytes, keyed by path relative to `root` with `/`.
fn snapshot(_root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    Ok(BTreeMap::new())
}

/// The paths that differ between two snapshots (changed, added or removed), sorted, once each.
fn stale_paths(
    _before: &BTreeMap<String, Vec<u8>>,
    _after: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    Vec::new()
}

impl Step {
    fn command(&self) -> Command {
        match self {
            Step::Record(_) => {
                let mut cmd = Command::new("cargo");
                cmd.args([
                    "test",
                    "--locked",
                    "--all-features",
                    "--test",
                    "integration",
                ]);
                cmd
            }
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
            Step::Test { .. } | Step::Record(_) => run(&mut cmd),
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
        frozen_check()?;
        let before = if mode == Mode::Check {
            dirty_paths()?
        } else {
            Vec::new()
        };
        regenerate()?;
        frozen_check()?;
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
        (Ok(_), Mode::Freeze(_)) => ExitCode::SUCCESS,
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

    #[test]
    fn bless_writes_only_current() {
        let corpus = SETS.iter().find(|(name, _)| *name == "corpus").unwrap();
        assert_eq!(corpus.1, "tests/corpus/current/");
        assert!(
            SETS.iter()
                .all(|(_, path)| frozen_dirty(&[path.to_string()]).is_empty())
        );
        assert!(!format!("{:?}", steps()).contains("v1"));
    }

    #[test]
    fn a_dirty_frozen_path_fails_bless() {
        let dirty = args(&[
            "tests/corpus/v1/x/case.json",
            "tests/corpus/v12/packs/a.json",
            "tests/corpus/current/stages/x/case.json",
            "tests/corpus/v0/x",
            "tests/corpus/v1a/x",
            "src/lib.rs",
        ]);
        assert_eq!(
            frozen_dirty(&dirty),
            [
                "tests/corpus/v1/x/case.json",
                "tests/corpus/v12/packs/a.json"
            ]
        );
        assert!(frozen_dirty(&args(&["tests/corpus/current/packs/a.json"])).is_empty());
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
            Step::Record(_) => "frozen version",
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

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn freeze_parses_one_name_and_nothing_else() {
        assert_eq!(
            parse(&args(&["bless", "--freeze", "v1"])),
            Ok(Mode::Freeze("v1".into()))
        );
        assert_eq!(
            parse(&args(&["bless", "--freeze", "v12"])),
            Ok(Mode::Freeze("v12".into()))
        );
        for bad in [
            &["bless", "--freeze"][..],
            &["bless", "--freeze", "v1", "v2"],
            &["bless", "--check", "--freeze", "v1"],
            &["bless", "--freeze", "v1", "--check"],
            &["--freeze", "v1"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn freeze_refuses_a_name_that_is_not_a_frozen_version() {
        let dir = tmp("names");
        for bad in ["v0", "v01", "v1a", "1", "V1", "v", "v-1", "current", ""] {
            let e = freeze_refusal(&dir, bad).unwrap_err();
            assert!(e.contains(">= 1"), "{bad}: {e}");
        }
        for ok in ["v1", "v9", "v12"] {
            assert_eq!(freeze_refusal(&dir, ok), Ok(()), "{ok}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn freeze_refuses_an_existing_folder_of_any_kind() {
        let dir = tmp("exists");
        std::fs::create_dir(dir.join("v1")).unwrap();
        std::fs::write(dir.join("v3"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("v4/stages")).unwrap();
        for taken in ["v1", "v3", "v4"] {
            let e = freeze_refusal(&dir, taken).unwrap_err();
            assert!(e.contains(&format!("tests/corpus/{taken}/")), "{e}");
        }
        std::fs::create_dir(dir.join("v2")).unwrap();
        std::fs::remove_dir(dir.join("v2")).unwrap();
        std::fs::create_dir(dir.join("v5")).unwrap();
        assert_eq!(freeze_refusal(&dir, "v6"), Ok(()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn freeze_regenerates_the_pack_and_the_corpus_then_records() {
        assert_eq!(
            freeze_steps("v1"),
            [
                Step::ExportPack,
                test_step(("UPDATE_CORPUS", "1"), "corpus::"),
                Step::Record("v1".into()),
            ]
        );
        let text = format!("{:?}", freeze_steps("v1"));
        for other in ["UPDATE_SCHEMA", "INSTA_UPDATE", "UPDATE_BENCH_PINS"] {
            assert!(!text.contains(other), "{other}");
        }
        let cmd = Step::Record("v1".into()).command();
        assert!(
            cmd.get_envs()
                .any(|(k, v)| k == "FREEZE_CORPUS" && v == Some(std::ffi::OsStr::new("v1")))
        );
        assert!(cmd.get_envs().all(|(k, v)| k != "CI" || v.is_some()));
        let line = format!("{:?}", cmd.get_args().collect::<Vec<_>>());
        assert!(
            line.contains("corpus_pins::the_record_is_written_on_request"),
            "{line}"
        );
        assert!(!format!("{:?}", steps()).contains("Record"));
    }

    #[test]
    fn stale_paths_name_what_changed_appeared_or_vanished() {
        let dir = tmp("snap");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/x"), b"1").unwrap();
        std::fs::write(dir.join("a/y"), b"2").unwrap();
        std::fs::write(dir.join("z"), b"3").unwrap();
        let before = snapshot(&dir).unwrap();
        assert_eq!(before.len(), 3);
        assert_eq!(before.get("a/b/x"), Some(&b"1".to_vec()));
        assert_eq!(before, snapshot(&dir).unwrap());
        assert!(stale_paths(&before, &before).is_empty());
        std::fs::write(dir.join("a/y"), b"changed").unwrap();
        std::fs::remove_file(dir.join("z")).unwrap();
        std::fs::write(dir.join("new"), b"4").unwrap();
        let after = snapshot(&dir).unwrap();
        assert_eq!(stale_paths(&before, &after), ["a/y", "new", "z"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
