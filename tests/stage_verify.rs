//! The `stage-verify` command line (ADR-009, ADR-015): exit status, output, and that nothing
//! from the files is repeated back.

// It starts the `stage-verify` binary as a host process; the iOS Simulator run (ios-sim.yml)
// cannot start one, and the library the binary calls is covered by the other tests.
#![cfg(not(target_os = "ios"))]

use crate::common;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use common::{clearing_inputs, stage_bytes};
use serde_json::{Value, json};
use stella_rain_core::content_hash::{content_hash, to_hex};
use stella_rain_core::engine::Engine;
use stella_rain_core::parsed_stage_hash::parsed_stage_hash;
use stella_rain_core::replay::{MAX_HASHES_BYTES, hash_to_hex, hashes_to_bytes, record};
use stella_rain_core::share;
use stella_rain_core::validate::parse_and_validate;

/// A scratch directory for one test, removed when it ends.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("stage-verify-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stage-verify"))
        .args(args)
        .output()
        .unwrap()
}

fn run_with_stdin(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stage-verify"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

struct Case {
    dir: Dir,
    stage: Vec<u8>,
    replay: Vec<u8>,
}

fn case(name: &str) -> Case {
    let stage = stage_bytes();
    let replay = record(&stage, &clearing_inputs()).unwrap().to_bytes();
    Case {
        dir: Dir::new(name),
        stage,
        replay,
    }
}

#[test]
fn a_clear_is_verified_and_reported() {
    let c = case("ok");
    let (stage, replay) = (
        c.dir.file("stage.json", &c.stage),
        c.dir.file("a.replay", &c.replay),
    );
    let o = run(&[&stage, &replay]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let parsed = parse_and_validate(&c.stage).unwrap();
    let mut engine = Engine::new(&parsed).unwrap();
    clearing_inputs().iter().for_each(|i| engine.step(*i));
    assert_eq!(
        out(&o),
        format!(
            "result: cleared\nticks: 35\nfinal_hash: {}\ncontent_hash: {}\n\
             parsed_stage_hash: {}\n",
            hash_to_hex(engine.state_hash()),
            to_hex(&content_hash(&c.stage, &c.replay)),
            to_hex(&parsed_stage_hash(&parsed))
        )
    );
    assert_eq!(err(&o), "");
}

#[test]
fn a_replay_that_does_not_match_exits_with_one() {
    let c = case("bad");
    let mut v: Value = serde_json::from_slice(&c.replay).unwrap();
    v["final_hash"] = json!("0000000000000001");
    let stage = c.dir.file("stage.json", &c.stage);
    let replay = c.dir.file("a.replay", v.to_string().as_bytes());
    let o = run(&[&stage, &replay]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(out(&o), "");
    assert!(
        err(&o).starts_with("not verified: the final hash is "),
        "{}",
        err(&o)
    );
}

#[test]
fn per_tick_hashes_name_the_first_tick_that_differs() {
    let c = case("hashes");
    let parsed = parse_and_validate(&c.stage).unwrap();
    let mut engine = Engine::new(&parsed).unwrap();
    let mut hashes: Vec<u64> = clearing_inputs()
        .iter()
        .map(|i| {
            engine.step(*i);
            engine.state_hash()
        })
        .collect();
    let stage = c.dir.file("stage.json", &c.stage);
    let replay = c.dir.file("a.replay", &c.replay);
    let good = c.dir.file("good.bin", &hashes_to_bytes(&hashes));
    assert_eq!(
        run(&[&stage, &replay, "--hashes", &good]).status.code(),
        Some(0)
    );

    let actual = hashes[20];
    hashes[20] = 0;
    let wrong = c.dir.file("wrong.bin", &hashes_to_bytes(&hashes));
    let o = run(&[&stage, &replay, "--hashes", &wrong]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("on tick 21"), "{}", err(&o));
    assert!(err(&o).contains("expected 0000000000000000"), "{}", err(&o));
    assert!(
        err(&o).contains(&format!("got {}", hash_to_hex(actual))),
        "{}",
        err(&o)
    );

    // The old text form is 17 bytes a tick: refused as a usage error.
    let text: String = hashes.iter().map(|h| hash_to_hex(*h) + "\n").collect();
    let old = c.dir.file("old.txt", text.as_bytes());
    let o = run(&[&stage, &replay, "--hashes", &old]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        err(&o).starts_with("error: the hashes file is 595 bytes"),
        "{}",
        err(&o)
    );

    let cut = c.dir.file("cut.bin", &vec![0; 279]);
    assert_eq!(
        run(&[&stage, &replay, "--hashes", &cut]).status.code(),
        Some(2)
    );

    let big = c.dir.file("big.bin", &vec![0; MAX_HASHES_BYTES + 8]);
    let o = run(&[&stage, &replay, "--hashes", &big]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("the limit is 1048576"), "{}", err(&o));

    let junk = c.dir.file("junk.txt", b"not a hash\n");
    assert_eq!(
        run(&[&stage, &replay, "--hashes", &junk]).status.code(),
        Some(2)
    );
}

#[test]
fn a_share_code_is_verified_from_a_file_or_standard_input() {
    let c = case("share");
    let code = share::encode(&c.stage, &c.replay).unwrap();
    let file = c.dir.file("code.txt", format!("  {code}\n").as_bytes());
    let o = run(&["--share-code", &file]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).starts_with("result: cleared\nticks: 35\n"));
    let o = run_with_stdin(&["--share-code", "-"], code.as_bytes());
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));

    let bad = c.dir.file("bad.txt", b"definitely not a code");
    let o = run(&["--share-code", &bad]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        err(&o).starts_with("not verified: the share code"),
        "{}",
        err(&o)
    );
}

#[test]
fn an_invalid_stage_is_not_verified_and_nothing_from_it_is_repeated() {
    let c = case("secret");
    let mut stage: Value = serde_json::from_slice(&c.stage).unwrap();
    stage["player"]["hp"] = json!(0);
    stage["title"] = json!("ZZZ_SECRET_TITLE");
    stage["id"] = json!("zzz_secret_id");
    let s = c.dir.file("stage.json", stage.to_string().as_bytes());
    let r = c.dir.file("a.replay", &c.replay);
    let o = run(&[&s, &r]);
    assert_eq!(o.status.code(), Some(1));
    let text = err(&o);
    assert!(
        text.contains("$.player.hp") && text.contains("$.id"),
        "{text}"
    );
    assert!(!text.to_lowercase().contains("zzz_secret"), "{text}");

    let unknown = c.dir.file("unknown.json", br#"{"ZZZ_FIELD": 1}"#);
    let o = run(&[&unknown, &r]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!err(&o).contains("ZZZ_FIELD"));
}

#[test]
fn files_over_their_limits_are_refused_not_read_to_the_end() {
    let c = case("big");
    let r = c.dir.file("a.replay", &c.replay);
    let big_stage = c.dir.file("big.json", &vec![b' '; 300 * 1024]);
    let o = run(&[&big_stage, &r]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("the limit is"), "{}", err(&o));
    let s = c.dir.file("stage.json", &c.stage);
    let big_replay = c.dir.file("big.replay", &vec![b' '; 300 * 1024]);
    assert_eq!(run(&[&s, &big_replay]).status.code(), Some(1));
}

#[test]
fn bad_command_lines_exit_with_two() {
    let c = case("usage");
    let s = c.dir.file("stage.json", &c.stage);
    let missing = c.dir.0.join("missing").to_str().unwrap().to_owned();
    for args in [
        vec![],
        vec![s.as_str()],
        vec![s.as_str(), missing.as_str()],
        vec![missing.as_str(), s.as_str()],
        vec!["--share-code"],
        vec!["--share-code", missing.as_str()],
        vec!["--frobnicate"],
        vec![s.as_str(), s.as_str(), "--hashes"],
        vec!["--share-code", s.as_str(), s.as_str()],
    ] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", err(&o));
        assert!(err(&o).starts_with("error: "), "{args:?}");
    }
    let o = run(&["--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(out(&o).starts_with("usage: stage-verify"));
    assert!(
        out(&o).contains(
            "Attack presets a stage names are read from the content pack built into this binary"
        ),
        "{}",
        out(&o)
    );
}

#[test]
fn a_run_replay_is_verified_and_prints_ledger() {
    let replay_path = crate::corpus::runs_dir().join("trial/run.replay");
    let o = run(&["--run", replay_path.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let stdout = out(&o);
    assert!(stdout.contains("result: "));
    assert!(stdout.contains("ticks: "));
    assert!(stdout.contains("final_hash: "));
    assert!(stdout.contains("commands: "));
    assert!(stdout.contains("stardust: "));
    assert!(stdout.contains("ledger: {"));
    assert!(stdout.contains("\"kills\":"));
    assert!(stdout.contains("\"bestiary\":"));
    assert!(stdout.contains("\"picks\":"));
    assert!(stdout.contains("\"skill_uses\":"));
    assert_eq!(err(&o), "");
}

#[test]
fn a_run_replay_with_a_changed_checkpoint_is_refused_naming_its_second() {
    let bytes = std::fs::read(crate::corpus::runs_dir().join("trial/run.replay")).unwrap();
    let mut replay: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let checkpoints = replay["checkpoints"].as_str().unwrap().to_string();
    let second = checkpoints.len() / 16;
    let last = checkpoints.len() - 1;
    let digit = if checkpoints.ends_with('0') { "1" } else { "0" };
    replay["checkpoints"] = serde_json::json!(format!("{}{digit}", &checkpoints[..last]));
    let dir = crate::data_root::scratch_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("trial-checkpoint.replay");
    std::fs::write(&path, replay.to_string()).unwrap();
    let o = run(&["--run", path.to_str().unwrap()]);
    assert_ne!(o.status.code(), Some(0));
    let message = err(&o);
    assert!(
        message.contains(&format!("checkpoint of second {second}")),
        "{message}"
    );
}
