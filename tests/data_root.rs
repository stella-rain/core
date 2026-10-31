// Where a test finds the repository's files and where it may write scratch files.
//
// Plain items only (no inner attributes): `src/limits.rs` and `src/engine/savepoint_tests.rs`
// `include!` this file. A test never reads `CARGO_MANIFEST_DIR` itself (`tests/cwd_free_paths.rs`).
//
// The root is, in order: `STELLA_TEST_DATA_ROOT`; the folder of the test executable when it
// holds a `cargo-dinghy` app bundle (a copy of the crate directory, on an iOS device); the
// manifest directory the test was built from.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Overrides the repository root of a test run.
pub(crate) const ROOT_VAR: &str = "STELLA_TEST_DATA_ROOT";
/// Overrides the folder for scratch files of a test run.
pub(crate) const TMP_VAR: &str = "STELLA_TEST_TMP_DIR";

/// Where the repository's files were found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Root {
    Override(PathBuf),
    Bundle(PathBuf),
    Manifest(PathBuf),
}

impl Root {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Root::Override(p) | Root::Bundle(p) | Root::Manifest(p) => p,
        }
    }
}

pub(crate) fn resolve_root(
    _over: Option<OsString>,
    _exe_dir: Option<&Path>,
    manifest: &Path,
) -> Root {
    Root::Manifest(manifest.to_path_buf())
}

pub(crate) fn resolve_scratch(
    _tmp_var: Option<OsString>,
    _root: &Root,
    target_tmp: Option<&str>,
    temp: &Path,
) -> PathBuf {
    target_tmp.map_or_else(|| temp.to_path_buf(), PathBuf::from)
}

pub(crate) fn resolve_target(root: &Root, _scratch: &Path) -> PathBuf {
    root.path().join("target")
}

fn current_root() -> Root {
    #[cfg(target_os = "ios")]
    let exe = std::env::current_exe().ok();
    #[cfg(target_os = "ios")]
    let exe_dir = exe.as_deref().and_then(Path::parent);
    #[cfg(not(target_os = "ios"))]
    let exe_dir: Option<&Path> = None;
    #[expect(clippy::disallowed_methods, reason = "test knob")]
    let over = std::env::var_os(ROOT_VAR);
    resolve_root(over, exe_dir, Path::new(env!("CARGO_MANIFEST_DIR")))
}

/// The repository root: `content/`, `schema/`, `tests/corpus/` are below it.
pub(crate) fn data_root() -> PathBuf {
    current_root().path().to_path_buf()
}

/// A folder tests may write scratch files to.
pub(crate) fn scratch_dir() -> PathBuf {
    #[expect(clippy::disallowed_methods, reason = "test knob")]
    let tmp = std::env::var_os(TMP_VAR);
    resolve_scratch(
        tmp,
        &current_root(),
        option_env!("CARGO_TARGET_TMPDIR"),
        &std::env::temp_dir(),
    )
}

/// The `target` folder tests write build-like output to (the corpus diff).
pub(crate) fn target_dir() -> PathBuf {
    resolve_target(&current_root(), &scratch_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp tree, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str, bundle: bool, corpus: bool) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("stella-data-root-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            if bundle {
                std::fs::write(dir.join("Cargo.toml"), "").unwrap();
            }
            if corpus {
                std::fs::create_dir_all(dir.join("tests/corpus")).unwrap();
            }
            Tree(dir)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const MANIFEST: &str = "/manifest";

    #[test]
    fn an_override_wins() {
        let bundle = Tree::new("override", true, true);
        let root = resolve_root(Some("/x".into()), Some(&bundle.0), Path::new(MANIFEST));
        assert_eq!(root, Root::Override(PathBuf::from("/x")));
        let root = resolve_root(Some("".into()), None, Path::new(MANIFEST));
        assert_eq!(root, Root::Manifest(PathBuf::from(MANIFEST)));
    }

    #[test]
    fn a_dinghy_bundle_beats_the_manifest_dir() {
        let bundle = Tree::new("bundle", true, true);
        let root = resolve_root(None, Some(&bundle.0), Path::new(MANIFEST));
        assert_eq!(root, Root::Bundle(bundle.0.clone()));
    }

    #[test]
    fn a_folder_that_is_not_a_bundle_is_ignored() {
        let manifest = Path::new(MANIFEST);
        let expected = Root::Manifest(PathBuf::from(MANIFEST));
        let no_corpus = Tree::new("no-corpus", true, false);
        assert_eq!(resolve_root(None, Some(&no_corpus.0), manifest), expected);
        let no_manifest = Tree::new("no-manifest", false, true);
        assert_eq!(resolve_root(None, Some(&no_manifest.0), manifest), expected);
        let neither = Tree::new("neither", false, false);
        assert_eq!(resolve_root(None, Some(&neither.0), manifest), expected);
        assert_eq!(resolve_root(None, None, manifest), expected);
    }

    #[test]
    fn scratch_goes_to_the_temp_dir_on_a_bundle() {
        let root = Root::Bundle(PathBuf::from("/app"));
        let scratch = resolve_scratch(None, &root, Some("/target/tmp"), Path::new("/tmp/app"));
        assert_eq!(scratch, Path::new("/tmp/app/stella-core-tests"));
    }

    #[test]
    fn the_tmp_variable_wins_for_scratch() {
        let root = Root::Bundle(PathBuf::from("/app"));
        let scratch = resolve_scratch(Some("/y".into()), &root, None, Path::new("/tmp/app"));
        assert_eq!(scratch, Path::new("/y"));
    }

    #[test]
    fn scratch_on_the_host_is_the_cargo_target_tmpdir() {
        let root = Root::Manifest(PathBuf::from(MANIFEST));
        let scratch = resolve_scratch(None, &root, Some("/target/tmp"), Path::new("/tmp"));
        assert_eq!(scratch, Path::new("/target/tmp"));
        let scratch = resolve_scratch(None, &root, None, Path::new("/tmp"));
        assert_eq!(scratch, Path::new("/tmp"));
    }

    #[test]
    fn target_is_below_the_root_except_on_a_bundle() {
        let scratch = Path::new("/tmp/stella-core-tests");
        for root in [
            Root::Manifest(PathBuf::from("/m")),
            Root::Override(PathBuf::from("/m")),
        ] {
            assert_eq!(resolve_target(&root, scratch), Path::new("/m/target"));
        }
        let bundle = Root::Bundle(PathBuf::from("/m"));
        assert_eq!(resolve_target(&bundle, scratch), scratch);
    }
}
