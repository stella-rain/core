//! The JSON Lines recording of a run (ADR-016), behind the `record` feature: for every tick the
//! input the frontend sent, the domain events the backend answered with, and the state hash.
//! Golden-file tests and the debug overlay read it; a replay stores only the inputs.
//!
//! ```text
//! {"tick":1203,"dir":"fe>be","type":"Input","data":{"dx":-3,"dy":1,"focus":true,"fire":false,"held":0}}
//! {"tick":1203,"dir":"be>fe","type":"PhaseChanged","data":{"phase":1}}
//! {"tick":1203,"dir":"be>fe","type":"StateHash","data":"9f3a0c12aa55bb66"}
//! ```

use std::fmt;

use serde_json::{Value, json};

use crate::engine::{Engine, Outcome};
use crate::input::Input;
use crate::replay::{hash_from_hex, hash_to_hex};
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
    let mut line = |tick: u32, dir: &str, mut body: Value| {
        let object = body.as_object_mut().expect("a line body is an object");
        object.insert("tick".into(), json!(tick));
        object.insert("dir".into(), json!(dir));
        out.push_str(&body.to_string());
        out.push('\n');
    };
    for input in inputs {
        if engine.outcome() != Outcome::Running {
            break;
        }
        engine.step(*input);
        let tick = engine.tick();
        line(tick, "fe>be", json!({ "type": "Input", "data": input }));
        for event in engine.events() {
            let body = serde_json::to_value(event).expect("an event serialises");
            line(tick, "be>fe", body);
        }
        let hash = hash_to_hex(engine.state_hash());
        line(tick, "be>fe", json!({ "type": "StateHash", "data": hash }));
    }
    Ok(out)
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
