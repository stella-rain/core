//! `cargo xtask bless` regenerates every generated set of core; `--check` reports any set that
//! is not as committed.

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Bless,
    Check,
}

fn parse(_args: &[String]) -> Result<Mode, String> {
    todo!()
}

fn stale_sets(_paths: &[String]) -> Vec<&'static str> {
    todo!()
}

fn stale_message(_sets: &[&str]) -> String {
    todo!()
}

fn main() {}

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
        for bad in [&[][..], &["nope"], &["bless", "--force"], &["bless", "--check", "x"], &["--check"]] {
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
