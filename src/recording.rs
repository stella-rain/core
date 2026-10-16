//! The JSON Lines recording of a run (ADR-016), behind the `record` feature: for every tick the
//! input the frontend sent, the domain events the backend answered with, and the state hash.
//! Golden-file tests and the debug overlay read it; a replay stores only the commands.
//!
//! ```text
//! {"tick":1203,"dir":"fe>be","type":"Input","data":{"move_x":-3,"move_y":127,"evade":true}}
//! {"tick":1203,"dir":"be>fe","type":"PhaseChanged","data":{"phase":1}}
//! {"tick":1203,"dir":"be>fe","type":"StateHash","data":"9f3a0c12aa55bb66"}
//! ```
//!
//! A run (ADR-066 §5) also has commands between ticks: a `Choose` or a `Spend` line, `fe>be`,
//! after the hash of the tick they follow, then the events the command caused, all on that
//! tick. The next `StateHash` is the next tick's, so there is still one hash a tick.
//!
//! ```text
//! {"tick":996,"dir":"be>fe","type":"LeveledUp","data":{"level":2}}
//! {"tick":996,"dir":"be>fe","type":"StateHash","data":"41109eebb5db56d4"}
//! {"tick":996,"dir":"fe>be","type":"Spend","data":{"index":1}}
//! {"tick":996,"dir":"be>fe","type":"StatRaised","data":{"stat":"attack","spent":1}}
//! ```

use std::fmt;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::engine::{Engine, Outcome};
use crate::input::Input;
use crate::replay::{hash_from_hex, hash_to_hex};
use crate::run_replay::{Command, RunRecorder, RunReplayError};
use crate::run_setup::RunSetup;
use crate::stage::Stage;
use crate::validate;

/// A recording that cannot be read, and the 1-based line it fails on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordingError {
    pub line: usize,
    pub reason: &'static str,
}

impl fmt::Display for RecordingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "recording line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for RecordingError {}

/// Runs `inputs` through the stage and returns the recording. Like a replay it stops on the
/// tick the run ends.
pub fn record(stage: &Stage, inputs: &[Input]) -> Result<String, validate::Error> {
    let mut engine = Engine::new(stage)?;
    let mut out = String::new();
    for input in inputs {
        if engine.outcome() != Outcome::Running {
            break;
        }
        engine.step(*input);
        let tick = engine.tick();
        line(
            &mut out,
            tick,
            "fe>be",
            json!({ "type": "Input", "data": input }),
        );
        for event in engine.events() {
            let body = serde_json::to_value(event).expect("an event serialises");
            line(&mut out, tick, "be>fe", body);
        }
        let hash = hash_to_hex(engine.state_hash());
        line(
            &mut out,
            tick,
            "be>fe",
            json!({ "type": "StateHash", "data": hash }),
        );
    }
    Ok(out)
}

/// Runs `commands` from a setup (ADR-066), with the bundled run stats, and returns the
/// recording: each tick as `record` writes it, and each command between ticks as a `Choose` or
/// `Spend` line followed by the events it caused. Like the run replay it stops on the tick the
/// run ends, and fails closed on a command the engine refuses (`run_replay::record_run`).
pub fn record_run(setup: &RunSetup, commands: &[Command]) -> Result<String, RunReplayError> {
    let mut recorder = RunRecorder::start(setup)?;
    let mut out = String::new();
    for command in commands {
        if recorder.engine().outcome() != Outcome::Running {
            break;
        }
        let body = match *command {
            Command::Step(input) => {
                recorder.step(input)?;
                json!({ "type": "Input", "data": input })
            }
            Command::Choose(index) => {
                recorder.choose(index)?;
                json!({ "type": "Choose", "data": { "index": index } })
            }
            Command::Spend(index) => {
                recorder.spend(index)?;
                json!({ "type": "Spend", "data": { "index": index } })
            }
        };
        let engine = recorder.engine();
        let tick = engine.tick();
        line(&mut out, tick, "fe>be", body);
        for event in engine.events() {
            let body = serde_json::to_value(event).expect("an event serialises");
            line(&mut out, tick, "be>fe", body);
        }
        if let Command::Step(_) = command {
            let hash = hash_to_hex(engine.state_hash());
            line(
                &mut out,
                tick,
                "be>fe",
                json!({ "type": "StateHash", "data": hash }),
            );
        }
    }
    Ok(out)
}

/// Appends one line: `body` with its `tick` and `dir`.
fn line(out: &mut String, tick: u32, dir: &str, mut body: Value) {
    let object = body.as_object_mut().expect("a line body is an object");
    object.insert("tick".into(), json!(tick));
    object.insert("dir".into(), json!(dir));
    out.push_str(&body.to_string());
    out.push('\n');
}

/// The `Input` lines, in order. Ticks must count up from 1 without gaps.
pub fn inputs_from_jsonl(text: &str) -> Result<Vec<Input>, RecordingError> {
    let mut inputs = Vec::new();
    for (n, line) in parse(text)? {
        if line.kind == "Input" {
            if line.tick as usize != inputs.len() + 1 {
                return Err(RecordingError {
                    line: n,
                    reason: "ticks are not in order",
                });
            }
            let input = serde_json::from_value(line.data).map_err(|_| RecordingError {
                line: n,
                reason: "not an input",
            })?;
            inputs.push(input);
        }
    }
    Ok(inputs)
}

/// The `Input`, `Choose` and `Spend` lines, in order, as the commands of a run. Ticks must
/// count up from 1 without gaps, and a command between ticks must carry the tick of the input
/// before it (0 before any).
pub fn commands_from_jsonl(text: &str) -> Result<Vec<Command>, RecordingError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Index {
        index: u32,
    }
    let mut commands = Vec::new();
    let mut ticks = 0u32;
    for (n, line) in parse(text)? {
        let error = |reason| RecordingError { line: n, reason };
        match line.kind.as_str() {
            "Input" => {
                if line.tick != ticks.saturating_add(1) {
                    return Err(error("ticks are not in order"));
                }
                let input = serde_json::from_value(line.data).map_err(|_| error("not an input"))?;
                commands.push(Command::Step(input));
                ticks = line.tick;
            }
            "Choose" => {
                if line.tick != ticks {
                    return Err(error("a choice is not after its tick"));
                }
                let choose: Index =
                    serde_json::from_value(line.data).map_err(|_| error("not a choice"))?;
                commands.push(Command::Choose(choose.index));
            }
            "Spend" => {
                if line.tick != ticks {
                    return Err(error("a spend is not after its tick"));
                }
                let spend: Index =
                    serde_json::from_value(line.data).map_err(|_| error("not a spend"))?;
                commands.push(Command::Spend(spend.index));
            }
            _ => {}
        }
    }
    Ok(commands)
}

/// The `StateHash` lines, in order: the per-tick hashes a corpus check compares.
pub fn hashes_from_jsonl(text: &str) -> Result<Vec<u64>, RecordingError> {
    let mut hashes = Vec::new();
    for (n, line) in parse(text)? {
        if line.kind == "StateHash" {
            if line.tick as usize != hashes.len() + 1 {
                return Err(RecordingError {
                    line: n,
                    reason: "ticks are not in order",
                });
            }
            let hash = line
                .data
                .as_str()
                .and_then(hash_from_hex)
                .ok_or(RecordingError {
                    line: n,
                    reason: "not a state hash",
                })?;
            hashes.push(hash);
        }
    }
    Ok(hashes)
}

struct Line {
    tick: u32,
    kind: String,
    data: Value,
}

/// Every non-blank line with its 1-based number. A line that is not JSON with a `tick` and a
/// `type` is an error.
fn parse(text: &str) -> Result<Vec<(usize, Line)>, RecordingError> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let bad = RecordingError {
            line: i + 1,
            reason: "not a recording line",
        };
        let value: Value = serde_json::from_str(raw).map_err(|_| bad)?;
        let line = (|| {
            Some(Line {
                tick: u32::try_from(value.get("tick")?.as_u64()?).ok()?,
                kind: value.get("type")?.as_str()?.to_owned(),
                data: value.get("data").cloned().unwrap_or(Value::Null),
            })
        })()
        .ok_or(bad)?;
        out.push((i + 1, line));
    }
    Ok(out)
}
