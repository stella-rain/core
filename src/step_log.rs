//! The step log: one in-memory model of a run's inputs for both replay schemas (ADR-REPL-7
//! point 4, ADR-REPL-10). A run holds one input per present player per world tick, and the
//! picks made on the steps in between; a schema 1 replay (inputs and a command list) is
//! migrated into it on read, and a schema 2 replay's `inputs` field is its bytes.
//!
//! The schema 2 bytes: a `present` byte (bit `i` for player slot `i`, at least one set, bits 3
//! to 7 zero), then records until the end. `0x00` TICKS: a varint `run` of at least 1, then one
//! 3-byte block (`flags`, `move_x`, `move_y`) per present slot in slot order, repeated for `run`
//! world ticks; two TICKS in a row never hold equal blocks. `0x01` PICKS: one step that does not
//! advance the world, one byte per present slot (`0x00` no pick, `0x01` and a varint index), at
//! least one pick. `0x02` JOIN and `0x03` LEAVE are reserved for the per-player model and
//! refused. The runs sum to `ticks`; at most `MAX_COMMANDS` PICKS records.

use crate::input::Input;
use crate::replay::{block_of, input_of_block, read_varint, write_varint};
use crate::run_replay::{CommandKind, MAX_COMMANDS, RunReplayError, TickCommand};

const TICKS: u8 = 0;
const PICKS: u8 = 1;
const JOIN: u8 = 2;
const LEAVE: u8 = 3;

/// The run's steps, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StepLog {
    /// Bit `i` set when player slot `i` is present at tick 0.
    pub(crate) present: u8,
    pub(crate) records: Vec<Record>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Record {
    /// `run` world ticks, each with these inputs (one per present slot, `pick` is `None`).
    Ticks { run: u32, inputs: Vec<Input> },
    /// One step that does not advance the world: the pick of each present slot, if any.
    Picks { picks: Vec<Option<u32>> },
}

impl StepLog {
    fn players(present: u8) -> usize {
        present.count_ones() as usize
    }

    /// Decodes the schema 2 bytes of a replay of `ticks` world ticks, at most `max_ticks`. The
    /// bytes are untrusted: a run is checked against the room left before anything is made of
    /// it, so no allocation follows a hostile count.
    pub(crate) fn decode(
        bytes: &[u8],
        ticks: u32,
        max_ticks: u32,
    ) -> Result<StepLog, RunReplayError> {
        let mut sink = Collect {
            records: Vec::new(),
        };
        let present = scan(bytes, ticks, max_ticks, &mut sink)?;
        Ok(StepLog {
            present,
            records: sink.records,
        })
    }

    /// The canonical bytes: `decode(encode(x)) == x` for any log `decode` can return.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.present];
        for record in &self.records {
            match record {
                Record::Ticks { run, inputs } => {
                    out.push(TICKS);
                    write_varint(&mut out, *run);
                    for input in inputs {
                        out.extend(block_of(input));
                    }
                }
                Record::Picks { picks } => {
                    out.push(PICKS);
                    for pick in picks {
                        match pick {
                            None => out.push(0),
                            Some(index) => {
                                out.push(1);
                                write_varint(&mut out, *index);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// A schema 1 replay's solo inputs (one per tick) and commands, as a log: a command at tick
    /// `t` is a PICKS record after the `t`-th tick, several at one tick in order; equal
    /// neighbouring inputs are one run, split where a PICKS falls.
    pub(crate) fn from_schema1(inputs: &[Input], commands: &[TickCommand]) -> StepLog {
        let mut records = Vec::new();
        let mut next = 0;
        // Takes the commands carrying `tick`, in order, as PICKS records.
        let picks_at = |tick: usize, next: &mut usize, records: &mut Vec<Record>| {
            while let Some(c) = commands.get(*next).filter(|c| c.tick as usize == tick) {
                records.push(Record::Picks {
                    picks: vec![Some(c.index)],
                });
                *next += 1;
            }
        };
        picks_at(0, &mut next, &mut records);
        let mut i = 0;
        while i < inputs.len() {
            // A command at a tick not past this one is already taken; the `max` keeps a
            // misordered list from stalling the loop.
            let limit = commands
                .get(next)
                .map_or(inputs.len(), |c| inputs.len().min(c.tick as usize))
                .max(i + 1);
            let input = inputs[i];
            let run = inputs[i..limit].iter().take_while(|x| **x == input).count();
            records.push(Record::Ticks {
                run: u32::try_from(run).unwrap_or(u32::MAX),
                inputs: vec![input],
            });
            i += run;
            picks_at(i, &mut next, &mut records);
        }
        StepLog {
            present: 0b1,
            records,
        }
    }

    /// The party a `present` byte names: its popcount when it is the slots from slot 0 up to
    /// slot 2 (`0b001`, `0b011`, `0b111`), else `None` (empty, a hole, a bit past slot 2).
    pub(crate) fn party_of(present: u8) -> Option<u8> {
        matches!(present, 0b001 | 0b011 | 0b111)
            .then(|| u8::try_from(present.count_ones()).unwrap_or(u8::MAX))
    }

    /// The number of PICKS records, the unit `MAX_COMMANDS` counts.
    pub(crate) fn picks_made(&self) -> u32 {
        let n = self
            .records
            .iter()
            .filter(|r| matches!(r, Record::Picks { .. }))
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    /// The log cut at world tick `ticks`: a TICKS run clipped to the room left, a PICKS step
    /// kept while the world tick it follows is at most `ticks`. A `ticks` past the end keeps
    /// everything.
    pub(crate) fn prefix(self, ticks: u32) -> StepLog {
        let mut world = 0u32;
        let mut records = Vec::new();
        for record in self.records {
            match record {
                Record::Ticks { run, inputs } => {
                    let take = run.min(ticks.saturating_sub(world));
                    world = world.saturating_add(run);
                    if take > 0 {
                        records.push(Record::Ticks { run: take, inputs });
                    }
                }
                Record::Picks { picks } => {
                    if world <= ticks {
                        records.push(Record::Picks { picks });
                    }
                }
            }
        }
        StepLog {
            present: self.present,
            records,
        }
    }

    /// The solo commands of a log without expanding its inputs: a run of `run` ticks only moves
    /// the tick count, so a hostile `run` allocates nothing here.
    pub(crate) fn solo_commands(&self) -> Result<Vec<TickCommand>, RunReplayError> {
        if self.present != 0b1 {
            return Err(RunReplayError::BadInputs("a party has no solo view"));
        }
        let mut tick = 0u32;
        let mut commands = Vec::new();
        for record in &self.records {
            match record {
                Record::Ticks { run, .. } => tick = tick.saturating_add(*run),
                Record::Picks { picks } => {
                    if let Some(index) = picks[0] {
                        commands.push(TickCommand {
                            tick,
                            kind: CommandKind::Choose,
                            index,
                        });
                    }
                }
            }
        }
        Ok(commands)
    }

    /// The solo inputs and commands of a log, the inverse of `from_schema1`: only a party of
    /// one.
    pub(crate) fn to_solo(&self) -> Result<(Vec<Input>, Vec<TickCommand>), RunReplayError> {
        if self.present != 0b1 {
            return Err(RunReplayError::BadInputs("a party has no solo view"));
        }
        let mut inputs = Vec::new();
        let mut commands = Vec::new();
        for record in &self.records {
            match record {
                Record::Ticks { run, inputs: held } => {
                    inputs.extend(std::iter::repeat_n(held[0], *run as usize));
                }
                Record::Picks { picks } => {
                    if let Some(index) = picks[0] {
                        commands.push(TickCommand {
                            tick: u32::try_from(inputs.len()).unwrap_or(u32::MAX),
                            kind: CommandKind::Choose,
                            index,
                        });
                    }
                }
            }
        }
        Ok((inputs, commands))
    }
}

/// One step of a tick range (ADR-REPL-10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeStep {
    /// `run` world ticks, each with these inputs (one per present slot, in slot order).
    Ticks { run: u32, inputs: Vec<Input> },
    /// One step that does not advance the world: the pick of each present slot, if any.
    Picks { picks: Vec<Option<u32>> },
}

/// The steps of world ticks `first..end` of a step log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRange {
    /// Bit `i` set when player slot `i` is present at tick 0 (the log's, not the range's).
    pub present: u8,
    pub first: u32,
    pub end: u32,
    pub steps: Vec<RangeStep>,
}

/// The steps of world ticks `a..b` of a schema 2 step log of `ticks` world ticks, at most
/// `max_ticks` (ADR-REPL-10). Ticks are 0-based and the range is half-open: tick `i` takes the
/// state after `i` ticks to the state after `i + 1`.
///
/// - A TICKS run that overlaps the range is returned once, clipped to its overlap: a run is
///   never split into ticks and never expanded, so the memory is bounded by the records in the
///   range, not by `ticks`, `a`, `b` or a claimed run.
/// - A PICKS step after `p` world ticks goes with the tick after it: it is in the range iff
///   `a <= p < b`; a range with `b == ticks` also holds the picks after the last tick. An empty
///   range (`a == b`) holds no step. The picks at `p == a` are therefore in the range, which is
///   what a re-simulation from the state after `a` ticks needs.
/// - The whole log is validated with the full decoder's checks, so a damaged log is refused
///   with the same error however small the range. `a > b` and `b > ticks` are `BadInputs`; both
///   are checked before the bytes are read.
pub fn read_step_range(
    bytes: &[u8],
    ticks: u32,
    max_ticks: u32,
    a: u32,
    b: u32,
) -> Result<StepRange, RunReplayError> {
    check_range(ticks, a, b)?;
    let mut sink = RangeSink::new(ticks, a, b);
    let present = scan(bytes, ticks, max_ticks, &mut sink)?;
    Ok(sink.finish(present))
}

fn check_range(ticks: u32, a: u32, b: u32) -> Result<(), RunReplayError> {
    if a > b {
        return Err(RunReplayError::BadInputs("a tick range that is backwards"));
    }
    if b > ticks {
        return Err(RunReplayError::BadInputs("a tick range past the last tick"));
    }
    Ok(())
}

/// The sink of the range reader: keeps what falls in `a..b`.
struct RangeSink {
    ticks: u32,
    a: u32,
    b: u32,
    steps: Vec<RangeStep>,
}

impl RangeSink {
    fn new(ticks: u32, a: u32, b: u32) -> RangeSink {
        RangeSink {
            ticks,
            a,
            b,
            steps: Vec::new(),
        }
    }

    fn finish(self, present: u8) -> StepRange {
        StepRange {
            present,
            first: self.a,
            end: self.b,
            steps: self.steps,
        }
    }
}

impl Sink for RangeSink {
    fn ticks(&mut self, start: u32, run: u32, inputs: &[Input]) {
        let from = start.max(self.a);
        let to = (start + run).min(self.b);
        if from < to {
            self.steps.push(RangeStep::Ticks {
                run: to - from,
                inputs: inputs.to_vec(),
            });
        }
    }

    fn picks(&mut self, at: u32, picks: &[Option<u32>]) {
        let inside = (self.a <= at && at < self.b)
            || (self.a < self.b && self.b == self.ticks && at == self.ticks);
        if inside {
            self.steps.push(RangeStep::Picks {
                picks: picks.to_vec(),
            });
        }
    }
}

impl StepLog {
    /// The steps of world ticks `a..b` of an already decoded log, by the rule of
    /// `read_step_range`.
    pub(crate) fn range(&self, a: u32, b: u32) -> Result<StepRange, RunReplayError> {
        let ticks = self.records.iter().fold(0u32, |t, r| match r {
            Record::Ticks { run, .. } => t.saturating_add(*run),
            Record::Picks { .. } => t,
        });
        check_range(ticks, a, b)?;
        let mut sink = RangeSink::new(ticks, a, b);
        let mut world = 0u32;
        for record in &self.records {
            match record {
                Record::Ticks { run, inputs } => {
                    sink.ticks(world, *run, inputs);
                    world += run;
                }
                Record::Picks { picks } => sink.picks(world, picks),
            }
        }
        Ok(sink.finish(self.present))
    }
}

/// What the scanner reports for each record, by reference: nothing is allocated for it.
trait Sink {
    /// `run` world ticks from world tick `start`, with one input per present slot.
    fn ticks(&mut self, start: u32, run: u32, inputs: &[Input]);
    /// One PICKS step after `at` world ticks.
    fn picks(&mut self, at: u32, picks: &[Option<u32>]);
}

/// The sink of `decode`: every record, pushed.
struct Collect {
    records: Vec<Record>,
}

impl Sink for Collect {
    fn ticks(&mut self, _start: u32, run: u32, inputs: &[Input]) {
        self.records.push(Record::Ticks {
            run,
            inputs: inputs.to_vec(),
        });
    }

    fn picks(&mut self, _at: u32, picks: &[Option<u32>]) {
        self.records.push(Record::Picks {
            picks: picks.to_vec(),
        });
    }
}

/// Walks the schema 2 bytes of a replay of `ticks` world ticks, at most `max_ticks`, with every
/// check of the format, and reports each record to `sink`. Returns the `present` byte. It is
/// the only decoder, so every reader refuses the same bytes with the same error. It keeps the
/// blocks of the run before in one reused buffer and allocates nothing per record.
fn scan(
    bytes: &[u8],
    ticks: u32,
    max_ticks: u32,
    sink: &mut impl Sink,
) -> Result<u8, RunReplayError> {
    let bad = RunReplayError::BadInputs;
    if ticks > max_ticks {
        return Err(RunReplayError::TooManyTicks {
            ticks,
            limit: max_ticks,
        });
    }
    let cut = bad("ends in the middle of a record");
    let present = *bytes.first().ok_or(cut.clone())?;
    if present == 0 {
        return Err(bad("no player is present"));
    }
    if present & !0b111 != 0 {
        return Err(bad("a player slot beyond the party"));
    }
    let players = StepLog::players(present);
    let mut pos = 1;
    let mut world = 0u32;
    let mut picks_seen = 0u32;
    // The inputs of the record before, when it was a TICKS record.
    let mut before: Vec<Input> = Vec::with_capacity(players);
    let mut after_ticks = false;
    let mut inputs: Vec<Input> = Vec::with_capacity(players);
    let mut picks: Vec<Option<u32>> = Vec::with_capacity(players);
    while pos < bytes.len() {
        let opcode = bytes[pos];
        pos += 1;
        match opcode {
            TICKS => {
                let run = read_varint(bytes, &mut pos).map_err(bad)?;
                if run == 0 {
                    return Err(bad("a run has length 0"));
                }
                if run > ticks - world {
                    return Err(bad("there are more inputs than ticks"));
                }
                inputs.clear();
                for _ in 0..players {
                    let block = *bytes
                        .get(pos..pos + 3)
                        .and_then(|b| <&[u8; 3]>::try_from(b).ok())
                        .ok_or(cut.clone())?;
                    pos += 3;
                    inputs.push(input_of_block(block, true).map_err(bad)?);
                }
                if after_ticks && before == inputs {
                    return Err(bad("a run repeats the input of the run before"));
                }
                sink.ticks(world, run, &inputs);
                world += run;
                std::mem::swap(&mut before, &mut inputs);
                after_ticks = true;
            }
            PICKS => {
                if picks_seen >= MAX_COMMANDS {
                    return Err(RunReplayError::TooManyCommands {
                        limit: MAX_COMMANDS,
                    });
                }
                picks.clear();
                for _ in 0..players {
                    let marker = *bytes.get(pos).ok_or(cut.clone())?;
                    pos += 1;
                    picks.push(match marker {
                        0 => None,
                        1 => Some(read_varint(bytes, &mut pos).map_err(bad)?),
                        _ => return Err(bad("a pick marker is neither 0 nor 1")),
                    });
                }
                if picks.iter().all(Option::is_none) {
                    return Err(bad("a record has no pick"));
                }
                picks_seen += 1;
                sink.picks(world, &picks);
                after_ticks = false;
            }
            JOIN | LEAVE => return Err(bad("a join or leave this build cannot run")),
            _ => return Err(bad("an unknown record")),
        }
    }
    if world != ticks {
        return Err(bad("there are fewer inputs than ticks"));
    }
    Ok(present)
}

#[cfg(test)]
mod tests {
    use super::{RangeStep, Record, StepLog, StepRange, read_step_range};
    use crate::base64;
    use crate::engine::ChoiceError;
    use crate::input::Input;
    use crate::run_replay::{MAX_COMMANDS, RunReplay, RunReplayError, verify_run};

    fn hex(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    fn main_held() -> Input {
        Input {
            main: true,
            ..Input::default()
        }
    }

    fn decode(text: &str, ticks: u32) -> Result<StepLog, RunReplayError> {
        StepLog::decode(&hex(text), ticks, 1_000_000)
    }

    fn bad_inputs(text: &str, ticks: u32) -> &'static str {
        match decode(text, ticks) {
            Err(RunReplayError::BadInputs(why)) => why,
            other => panic!("{text}: {other:?}"),
        }
    }

    // --- The worked bytes ------------------------------------------------------------------

    #[test]
    fn a_solo_run_of_five_ticks_with_main_held() {
        let bytes = "01 00 05 02 00 00";
        let expected = StepLog {
            present: 0b1,
            records: vec![Record::Ticks {
                run: 5,
                inputs: vec![main_held()],
            }],
        };
        assert_eq!(decode(bytes, 5), Ok(expected.clone()));
        assert_eq!(expected.encode(), hex(bytes));
    }

    #[test]
    fn a_pick_between_two_runs() {
        let bytes = "01 00 01 02 00 00 01 01 00 00 02 00 00 00";
        let expected = StepLog {
            present: 0b1,
            records: vec![
                Record::Ticks {
                    run: 1,
                    inputs: vec![main_held()],
                },
                Record::Picks {
                    picks: vec![Some(0)],
                },
                Record::Ticks {
                    run: 2,
                    inputs: vec![Input::default()],
                },
            ],
        };
        assert_eq!(decode(bytes, 3), Ok(expected.clone()));
        assert_eq!(expected.encode(), hex(bytes));
    }

    #[test]
    fn three_players_are_one_block_each_in_slot_order() {
        let bytes = "07 00 01 02 00 7f 00 00 00 00 00 00";
        let expected = StepLog {
            present: 0b111,
            records: vec![Record::Ticks {
                run: 1,
                inputs: vec![
                    Input {
                        main: true,
                        move_y: 127,
                        ..Input::default()
                    },
                    Input::default(),
                    Input::default(),
                ],
            }],
        };
        assert_eq!(decode(bytes, 1), Ok(expected.clone()));
        assert_eq!(expected.encode(), hex(bytes));
    }

    #[test]
    fn a_party_with_a_gap_has_blocks_for_the_present_slots_only() {
        // Slots 0 and 2; slot 2 holds interact (bit 6) and the stick at -127.
        let bytes = "05 00 02 00 00 00 40 81 00 01 01 05 00";
        let log = decode(bytes, 2).unwrap();
        assert_eq!(log.present, 0b101);
        let Record::Ticks { inputs, .. } = &log.records[0] else {
            panic!("a run")
        };
        assert_eq!(inputs.len(), 2);
        assert!(inputs[1].interact && inputs[1].move_x == -127);
        let Record::Picks { picks } = &log.records[1] else {
            panic!("picks")
        };
        assert_eq!(picks, &[Some(5), None]);
        assert_eq!(log.encode(), hex(bytes));
    }

    #[test]
    fn a_play_may_end_on_a_pick() {
        assert!(decode("01 00 01 00 00 00 01 01 02", 1).is_ok());
    }

    // --- Refusals --------------------------------------------------------------------------

    #[test]
    fn every_refusal_has_its_message() {
        let cases: [(&str, u32, &str); 15] = [
            ("00 00 01 00 00 00", 1, "no player is present"),
            ("09 00 01 00 00 00", 1, "a player slot beyond the party"),
            ("01 00 01 80 00 00", 1, "a run has unknown flags"),
            ("01 07 01 00 00 00", 1, "an unknown record"),
            ("01 02 00", 0, "a join or leave this build cannot run"),
            ("01 03 00", 0, "a join or leave this build cannot run"),
            ("01 00 00 00 00 00", 0, "a run has length 0"),
            (
                "01 00 01 00 00 00 00 01 00 00 00",
                2,
                "a run repeats the input of the run before",
            ),
            ("01 01 00", 0, "a record has no pick"),
            ("01 00 05 02 00", 5, "ends in the middle of a record"),
            ("01 00", 0, "ends in the middle of a number"),
            ("01 00 04 00 00 00", 5, "there are fewer inputs than ticks"),
            ("01 00 06 00 00 00", 5, "there are more inputs than ticks"),
            (
                "01 00 85 00 00 00 00",
                5,
                "a number is not in its shortest form",
            ),
            ("01 00 01 00 80 00", 1, "a stick axis is out of range"),
        ];
        for (bytes, ticks, why) in cases {
            assert_eq!(bad_inputs(bytes, ticks), why, "{bytes}");
        }
    }

    #[test]
    fn a_huge_run_is_refused_before_it_is_expanded() {
        // 2^32 - 1 ticks in a run, three ticks of room.
        assert_eq!(
            bad_inputs("01 00 ff ff ff ff 0f 00 00 00", 3),
            "there are more inputs than ticks"
        );
    }

    #[test]
    fn ticks_over_the_cap_are_too_many() {
        assert_eq!(
            StepLog::decode(&hex("01"), 11, 10),
            Err(RunReplayError::TooManyTicks {
                ticks: 11,
                limit: 10
            })
        );
    }

    #[test]
    fn the_pick_records_are_capped() {
        let mut bytes = vec![1u8];
        for _ in 0..MAX_COMMANDS {
            bytes.extend([1, 1, 0]);
        }
        assert!(StepLog::decode(&bytes, 0, 10).is_ok());
        bytes.extend([1, 1, 0]);
        assert_eq!(
            StepLog::decode(&bytes, 0, 10),
            Err(RunReplayError::TooManyCommands {
                limit: MAX_COMMANDS
            })
        );
    }

    // --- The corpus ------------------------------------------------------------------------

    /// The schema 1 file of a run, whichever schema it is at: the corpus' `current/` runs are
    /// written at the schema writers write, and these tests take the old form to hand-damage.
    fn schema_1_form(replay: &RunReplay) -> RunReplay {
        if replay.setup.schema_version < 2 {
            return replay.clone();
        }
        let (inputs, commands) = replay.step_log(u32::MAX).unwrap().to_solo().unwrap();
        let mut one = replay.clone();
        one.setup.schema_version = 1;
        one.inputs = crate::replay::pack_inputs(&inputs);
        one.commands = crate::run_replay::pack_commands(&commands);
        one
    }

    fn corpus_runs() -> Vec<(String, Vec<u8>)> {
        corpus_files()
            .into_iter()
            .map(|(name, bytes)| {
                let one = schema_1_form(&RunReplay::from_bytes(&bytes).unwrap());
                (name, one.to_bytes())
            })
            .collect()
    }

    /// Every run case of the corpus, current and frozen, as (name, bytes) as they are filed.
    fn corpus_files() -> Vec<(String, Vec<u8>)> {
        let mut runs: Vec<(String, Vec<u8>)> = Vec::new();
        macro_rules! case {
            ($tree:literal, $name:literal) => {
                runs.push((
                    concat!($tree, "/", $name).to_owned(),
                    include_bytes!(concat!(
                        "../tests/corpus/",
                        $tree,
                        "/runs/",
                        $name,
                        "/run.replay"
                    ))
                    .to_vec(),
                ));
            };
        }
        case!("current", "easy");
        case!("current", "elites");
        case!("current", "rainy");
        case!("current", "stage_skill");
        case!("current", "time_limit");
        case!("current", "time_limit_kill");
        case!("current", "time_limit_overtime");
        case!("current", "trial");
        case!("current", "tutorial");
        case!("v1", "easy");
        case!("v1", "elites");
        case!("v1", "rainy");
        case!("v1", "stage_skill");
        case!("v1", "time_limit");
        case!("v1", "time_limit_kill");
        case!("v1", "time_limit_overtime");
        case!("v1", "trial");
        case!("v1", "tutorial");
        runs
    }

    #[test]
    fn schema_1_migrates_in_memory() {
        let mut with_picks = 0;
        for (name, bytes) in corpus_runs() {
            let replay = RunReplay::from_bytes(&bytes).unwrap();
            let inputs = replay.unpack_inputs(u32::MAX).unwrap();
            let commands = replay.unpack_commands().unwrap();
            let log = replay.step_log(u32::MAX).unwrap();
            assert_eq!(log, StepLog::from_schema1(&inputs, &commands), "{name}");
            assert_eq!(log.to_solo(), Ok((inputs, commands.clone())), "{name}");
            assert_eq!(
                StepLog::decode(&log.encode(), replay.ticks, u32::MAX),
                Ok(log),
                "{name}"
            );
            with_picks += usize::from(!commands.is_empty());
        }
        assert!(with_picks >= 2, "the corpus has runs with picks");
    }

    #[test]
    fn a_command_sits_after_the_tick_it_followed() {
        use crate::run_replay::{CommandKind, TickCommand};
        let choose = |tick, index| TickCommand {
            tick,
            kind: CommandKind::Choose,
            index,
        };
        let a = Input::default();
        let b = main_held();
        let log = StepLog::from_schema1(&[a, a, b, b], &[choose(2, 1), choose(2, 0), choose(4, 3)]);
        let picks = |i| Record::Picks {
            picks: vec![Some(i)],
        };
        assert_eq!(
            log.records,
            vec![
                Record::Ticks {
                    run: 2,
                    inputs: vec![a]
                },
                picks(1),
                picks(0),
                Record::Ticks {
                    run: 2,
                    inputs: vec![b]
                },
                picks(3),
            ]
        );
        let start = StepLog::from_schema1(&[a], &[choose(0, 2)]);
        assert_eq!(start.records[0], picks(2));
    }

    #[test]
    fn a_schema_1_command_after_the_last_tick_is_damage() {
        let (_, bytes) = corpus_runs().remove(0);
        let mut replay = RunReplay::from_bytes(&bytes).unwrap();
        // One command a tick past the end: delta from 0 is `ticks + 1`.
        let mut late = Vec::new();
        crate::replay::write_varint(&mut late, replay.ticks + 1);
        late.extend([0, 0]);
        replay.commands = base64::encode(&late);
        assert_eq!(
            replay.step_log(u32::MAX),
            Err(RunReplayError::BadCommands(
                "a command is after the last tick"
            ))
        );
    }

    // --- The verifier over both schemas ----------------------------------------------------

    fn schema_2_of(replay: &RunReplay) -> RunReplay {
        let log = replay.step_log(u32::MAX).unwrap();
        schema_2_with(replay, &log)
    }

    fn schema_2_with(replay: &RunReplay, log: &StepLog) -> RunReplay {
        let mut two = replay.clone();
        two.setup.schema_version = 2;
        two.inputs = base64::encode(&log.encode());
        two.commands = String::new();
        two
    }

    /// A corpus run with picks, as (schema 1 replay).
    fn run_with_picks() -> RunReplay {
        corpus_runs()
            .into_iter()
            .map(|(_, bytes)| RunReplay::from_bytes(&bytes).unwrap())
            .find(|r| !r.unpack_commands().unwrap().is_empty() && verify_run(&r.to_bytes()).is_ok())
            .expect("a run with picks")
    }

    #[test]
    fn a_schema_2_replay_verifies_like_the_schema_1_one() {
        let mut verified = 0;
        for (name, bytes) in corpus_runs() {
            let one = RunReplay::from_bytes(&bytes).unwrap();
            let two = schema_2_of(&one);
            assert_eq!(two.setup.schema_version, 2);
            let expected = verify_run(&bytes);
            // A case pinned to another pack is checked by the corpus test with its pack.
            if expected == Err(RunReplayError::ContentChanged) {
                assert_eq!(verify_run(&two.to_bytes()), expected, "{name}");
                continue;
            }
            assert!(expected.is_ok(), "{name}: {expected:?}");
            assert_eq!(verify_run(&two.to_bytes()), expected, "{name}");
            verified += 1;
        }
        assert!(
            verified >= 5,
            "{verified} cases verified with the bundled pack"
        );
    }

    #[test]
    fn schema_2_has_no_command_list() {
        let one = run_with_picks();
        let mut two = schema_2_of(&one);
        two.commands = one.commands.clone();
        assert_eq!(
            verify_run(&two.to_bytes()),
            Err(RunReplayError::BadCommands("schema 2 has no command list"))
        );
    }

    #[test]
    fn a_refused_pick_in_schema_2_reads_like_the_schema_1_one() {
        let one = run_with_picks();
        let mut log = one.step_log(u32::MAX).unwrap();
        let first = log
            .records
            .iter_mut()
            .find_map(|r| match r {
                Record::Picks { picks } => Some(picks),
                Record::Ticks { .. } => None,
            })
            .unwrap();
        first[0] = Some(999);
        let refused = verify_run(&schema_2_with(&one, &log).to_bytes());
        let Err(RunReplayError::ChoiceRefused { error, .. }) = refused else {
            panic!("{refused:?}")
        };
        assert!(matches!(error, ChoiceError::OutOfRange { index: 999, .. }));
    }

    #[test]
    fn a_pick_with_no_offer_open_is_refused() {
        let one = run_with_picks();
        let mut log = one.step_log(u32::MAX).unwrap();
        log.records.insert(
            0,
            Record::Picks {
                picks: vec![Some(0)],
            },
        );
        assert_eq!(
            verify_run(&schema_2_with(&one, &log).to_bytes()),
            Err(RunReplayError::ChoiceRefused {
                tick: 0,
                error: ChoiceError::NoOffer
            })
        );
    }

    #[test]
    fn a_party_with_a_hole_is_refused() {
        // Joining and leaving are later work: the party is the slots from slot 0.
        let one = run_with_picks();
        for present in [0b010u8, 0b100, 0b101, 0b110] {
            let log = StepLog {
                present,
                records: vec![Record::Ticks {
                    run: one.ticks,
                    inputs: vec![Input::default(); present.count_ones() as usize],
                }],
            };
            assert_eq!(
                verify_run(&schema_2_with(&one, &log).to_bytes()),
                Err(RunReplayError::BadInputs(
                    "the party does not match the run"
                ))
            );
        }
    }

    #[test]
    fn a_schema_1_stream_cannot_carry_interact() {
        let one = run_with_picks();
        let mut bytes = base64::decode(&one.inputs).unwrap();
        let mut at = 0;
        crate::replay::read_varint(&bytes, &mut at).unwrap();
        bytes[at] |= 0b100_0000;
        let mut flagged = one.clone();
        flagged.inputs = base64::encode(&bytes);
        assert_eq!(
            verify_run(&flagged.to_bytes()),
            Err(RunReplayError::BadInputs("a run has unknown flags"))
        );
    }

    #[test]
    fn the_recorder_writes_schema_2_whatever_it_started_or_resumed_from() {
        use crate::pack::Pack;
        use crate::run_replay::RunRecorder;
        use crate::run_setup::RunSetup;
        for started_at in [1, 2] {
            let setup = RunSetup {
                schema_version: started_at,
                ..RunSetup::default()
            };
            let mut rec = RunRecorder::start(&setup).unwrap();
            for tick in 0..90u8 {
                rec.step(&[
                    Input {
                        move_x: i8::try_from(tick).unwrap(),
                        ..Input::default()
                    },
                    Default::default(),
                    Default::default(),
                ])
                .unwrap();
            }
            let two = rec.replay();
            assert_eq!(two.setup.schema_version, 2, "started at {started_at}");
            assert_eq!(two.commands, "");
            assert!(verify_run(&two.to_bytes()).is_ok());
            // A schema 1 prefix of the same run resumes into a schema 2 recorder, whose replay
            // is the schema 2 form of the same run.
            let one = schema_1_form(&two);
            assert_eq!(one.setup.schema_version, 1);
            assert!(verify_run(&one.to_bytes()).is_ok());
            let resumed = RunRecorder::resume_with_pack(&one, Pack::bundled())
                .unwrap()
                .replay();
            assert_eq!(resumed, two, "resumed from a schema 1 prefix");
            let from_two = RunRecorder::resume_with_pack(&two, Pack::bundled())
                .unwrap()
                .replay();
            assert_eq!(from_two, two, "resumed from a schema 2 prefix");
        }
    }

    #[test]
    fn the_commands_of_a_huge_run_are_read_without_expanding_its_inputs() {
        use crate::run_replay::{CommandKind, TickCommand};
        let mut forged = run_with_picks();
        forged.setup.schema_version = 2;
        forged.ticks = 4_000_000_000;
        forged.commands = String::new();
        let log = StepLog {
            present: 1,
            records: vec![
                Record::Ticks {
                    run: 4_000_000_000,
                    inputs: vec![Input::default()],
                },
                Record::Picks {
                    picks: vec![Some(3)],
                },
            ],
        };
        forged.inputs = base64::encode(&log.encode());
        assert_eq!(
            forged.unpack_commands(),
            Ok(vec![TickCommand {
                tick: 4_000_000_000,
                kind: CommandKind::Choose,
                index: 3
            }])
        );
    }

    #[test]
    fn a_recorded_pick_sits_inside_its_step() {
        use crate::run_replay::{CommandKind, TickCommand};
        // Main held for 5 ticks, a pick of option 0, main held for 2 more: the worked vector of
        // the module documentation. The pick is a PICKS record of its own and `commands` is empty.
        let held = main_held();
        let log = StepLog::from_schema1(
            &[held; 7],
            &[TickCommand {
                tick: 5,
                kind: CommandKind::Choose,
                index: 0,
            }],
        );
        assert_eq!(
            log.encode(),
            hex("01 00 05 02 00 00 01 01 00 00 02 02 00 00")
        );
    }

    // ---- the range reader (ADR-REPL-10) ----

    const MAX: u32 = u32::MAX;

    fn idle() -> Input {
        Input::default()
    }

    fn range(text: &str, ticks: u32, a: u32, b: u32) -> Result<StepRange, RunReplayError> {
        read_step_range(&hex(text), ticks, MAX, a, b)
    }

    fn steps(text: &str, ticks: u32, a: u32, b: u32) -> Vec<RangeStep> {
        range(text, ticks, a, b).unwrap().steps
    }

    fn run_of(run: u32, input: Input) -> RangeStep {
        RangeStep::Ticks {
            run,
            inputs: vec![input],
        }
    }

    fn pick_of(index: u32) -> RangeStep {
        RangeStep::Picks {
            picks: vec![Some(index)],
        }
    }

    /// One entry per tick, and the picks where they fall: the oracle's expanded form.
    #[derive(Debug, PartialEq)]
    enum Entry {
        Tick(Vec<Input>),
        Picks(Vec<Option<u32>>),
    }

    /// The full decode, expanded to one entry per tick and cut to `a..b` by the rule written out
    /// again from the semantics: a PICKS after `p` ticks is in when `a <= p < b`, and a range
    /// ending at `ticks` also takes the picks after the last tick; an empty range holds nothing.
    fn oracle(log: &StepLog, ticks: u32, a: u32, b: u32) -> Vec<Entry> {
        let mut out = Vec::new();
        let mut world = 0u32;
        for record in &log.records {
            match record {
                Record::Ticks { run, inputs } => {
                    for t in world..world + run {
                        if a <= t && t < b {
                            out.push(Entry::Tick(inputs.clone()));
                        }
                    }
                    world += run;
                }
                Record::Picks { picks } => {
                    let inside =
                        (a <= world && world < b) || (a < b && b == ticks && world == ticks);
                    if inside {
                        out.push(Entry::Picks(picks.clone()));
                    }
                }
            }
        }
        out
    }

    fn expand(steps: &[RangeStep]) -> Vec<Entry> {
        let mut out = Vec::new();
        for step in steps {
            match step {
                RangeStep::Ticks { run, inputs } => {
                    for _ in 0..*run {
                        out.push(Entry::Tick(inputs.clone()));
                    }
                }
                RangeStep::Picks { picks } => out.push(Entry::Picks(picks.clone())),
            }
        }
        out
    }

    #[test]
    fn a_range_that_starts_inside_a_run_is_clipped() {
        let log = "01 00 05 02 00 00";
        assert_eq!(steps(log, 5, 2, 4), [run_of(2, main_held())]);
        assert_eq!(steps(log, 5, 0, 5), [run_of(5, main_held())]);
        assert_eq!(steps(log, 5, 4, 5), [run_of(1, main_held())]);
        assert_eq!(steps(log, 5, 0, 1), [run_of(1, main_held())]);
        let whole = range(log, 5, 0, 5).unwrap();
        assert_eq!((whole.present, whole.first, whole.end), (1, 0, 5));
    }

    #[test]
    fn an_empty_range_is_ok_and_holds_nothing() {
        let log = "01 00 05 02 00 00";
        for at in [0, 3, 5] {
            assert_eq!(steps(log, 5, at, at), []);
        }
        // Even trailing picks are not in an empty range, and the log is still validated.
        let trailing = "01 00 02 02 00 00 01 01 07";
        assert_eq!(steps(trailing, 2, 2, 2), []);
        assert_eq!(
            range("01 00 05 02 00", 5, 0, 0),
            Err(RunReplayError::BadInputs("ends in the middle of a record"))
        );
    }

    #[test]
    fn a_pick_comes_with_the_tick_after_it() {
        // Tick 0 held, a pick of index 0 after it (position 1), then two idle ticks.
        let log = "01 00 01 02 00 00 01 01 00 00 02 00 00 00";
        assert_eq!(steps(log, 3, 0, 1), [run_of(1, main_held())]);
        assert_eq!(steps(log, 3, 1, 3), [pick_of(0), run_of(2, idle())]);
        assert_eq!(
            steps(log, 3, 0, 3),
            [run_of(1, main_held()), pick_of(0), run_of(2, idle())]
        );
        assert_eq!(steps(log, 3, 1, 2), [pick_of(0), run_of(1, idle())]);
        assert_eq!(
            steps(log, 3, 0, 2),
            [run_of(1, main_held()), pick_of(0), run_of(1, idle())]
        );
        assert_eq!(steps(log, 3, 2, 3), [run_of(1, idle())]);
    }

    #[test]
    fn a_pick_before_the_first_tick_is_in_a_range_that_starts_at_0() {
        let log = "01 01 01 00 00 05 02 00 00";
        assert_eq!(steps(log, 5, 0, 2), [pick_of(0), run_of(2, main_held())]);
        assert_eq!(steps(log, 5, 1, 2), [run_of(1, main_held())]);
        assert_eq!(steps(log, 5, 0, 0), []);
    }

    #[test]
    fn the_picks_after_the_last_tick_belong_to_a_range_ending_there() {
        let log = "01 00 02 02 00 00 01 01 07";
        assert_eq!(steps(log, 2, 0, 2), [run_of(2, main_held()), pick_of(7)]);
        assert_eq!(steps(log, 2, 0, 1), [run_of(1, main_held())]);
        assert_eq!(steps(log, 2, 1, 2), [run_of(1, main_held()), pick_of(7)]);
    }

    #[test]
    fn several_picks_at_one_position_keep_their_order() {
        let log = "01 00 01 02 00 00 01 01 03 01 01 04 00 01 00 00 00";
        assert_eq!(
            steps(log, 2, 1, 2),
            [pick_of(3), pick_of(4), run_of(1, idle())]
        );
        assert_eq!(steps(log, 2, 0, 1), [run_of(1, main_held())]);
    }

    #[test]
    fn a_party_of_three_gets_three_inputs_per_run() {
        let log = "07 00 02 02 00 00 00 00 00 00 00 00";
        assert_eq!(
            steps(log, 2, 0, 1),
            [RangeStep::Ticks {
                run: 1,
                inputs: vec![main_held(), idle(), idle()]
            }]
        );
        assert_eq!(range(log, 2, 0, 1).unwrap().present, 7);
    }

    #[test]
    fn a_bad_range_is_refused_without_reading_the_log() {
        let log = "01 00 05 02 00 00";
        let backwards = Err(RunReplayError::BadInputs("a tick range that is backwards"));
        let past = Err(RunReplayError::BadInputs("a tick range past the last tick"));
        assert_eq!(range(log, 5, 3, 2), backwards);
        assert_eq!(range(log, 5, 0, 6), past);
        assert_eq!(range(log, 5, 6, 6), past);
        // The bytes are not even looked at.
        assert_eq!(range("", 5, 3, 2), backwards);
        assert_eq!(range("", 5, 0, 6), past);
        assert_eq!(
            read_step_range(&hex(log), 11, 10, 0, 0),
            Err(RunReplayError::TooManyTicks {
                ticks: 11,
                limit: 10
            })
        );
    }

    #[test]
    fn the_range_reader_refuses_what_the_decoder_refuses() {
        let cases: [(&str, u32); 15] = [
            ("00 00 01 00 00 00", 1),
            ("09 00 01 00 00 00", 1),
            ("01 00 01 80 00 00", 1),
            ("01 07 01 00 00 00", 1),
            ("01 02 00", 0),
            ("01 03 00", 0),
            ("01 00 00 00 00 00", 0),
            ("01 00 01 00 00 00 00 01 00 00 00", 2),
            ("01 01 00", 0),
            ("01 00 05 02 00", 5),
            ("01 00", 0),
            ("01 00 04 00 00 00", 5),
            ("01 00 06 00 00 00", 5),
            ("01 00 85 00 00 00 00", 5),
            ("01 00 01 00 80 00", 1),
        ];
        for (bytes, ticks) in cases {
            let want = StepLog::decode(&hex(bytes), ticks, MAX).unwrap_err();
            assert_eq!(range(bytes, ticks, 0, 0).unwrap_err(), want, "{bytes}");
            assert_eq!(range(bytes, ticks, 0, ticks).unwrap_err(), want, "{bytes}");
        }
        // Damage after `b` is still refused.
        let damaged = "01 00 02 02 00 00 00 03 00 00 00 07";
        let want = StepLog::decode(&hex(damaged), 5, MAX).unwrap_err();
        assert_eq!(range(damaged, 5, 0, 1).unwrap_err(), want);
        // The 1001st PICKS is refused after the range ends.
        let mut many = String::from("01");
        for _ in 0..=MAX_COMMANDS {
            many.push_str(" 01 01 00");
        }
        let want = StepLog::decode(&hex(&many), 0, MAX).unwrap_err();
        assert!(matches!(want, RunReplayError::TooManyCommands { .. }));
        assert_eq!(range(&many, 0, 0, 0).unwrap_err(), want);
    }

    #[test]
    fn a_huge_run_is_clipped_not_expanded() {
        let log = "01 00 ff ff ff ff 0f 02 00 00";
        assert_eq!(steps(log, MAX, 0, 10), [run_of(10, main_held())]);
        assert_eq!(
            steps(log, MAX, 4_000_000_000, 4_000_000_010),
            [run_of(10, main_held())]
        );
        assert_eq!(steps(log, MAX, 0, MAX), [run_of(MAX, main_held())]);
    }

    #[test]
    fn the_range_read_is_the_oracle_on_every_corpus_case() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for (name, bytes) in corpus_files()
            .into_iter()
            .filter(|(n, _)| n.starts_with("current"))
        {
            let replay = RunReplay::from_bytes(&bytes).unwrap();
            let raw = base64::decode(&replay.inputs).unwrap();
            let ticks = replay.ticks;
            let log = StepLog::decode(&raw, ticks, MAX).unwrap();
            let mut ranges = vec![
                (0, 0),
                (0, ticks),
                (ticks, ticks),
                (ticks - 1, ticks),
                (0, 1),
            ];
            let mut k = 0;
            while k < ticks {
                ranges.push((k, (k + 60).min(ticks)));
                k += 60;
            }
            for _ in 0..200 {
                let x = u32::try_from(next() % (u64::from(ticks) + 1)).unwrap();
                let y = u32::try_from(next() % (u64::from(ticks) + 1)).unwrap();
                ranges.push((x.min(y), x.max(y)));
            }
            for (a, b) in ranges {
                let got = read_step_range(&raw, ticks, MAX, a, b).unwrap();
                assert_eq!(got.present, log.present, "{name}");
                assert_eq!((got.first, got.end), (a, b), "{name}");
                assert!(got.steps.len() <= log.records.len(), "{name} {a}..{b}");
                assert_eq!(
                    expand(&got.steps),
                    oracle(&log, ticks, a, b),
                    "{name} {a}..{b}"
                );
            }
        }
    }

    #[test]
    fn the_whole_range_is_the_decoded_log() {
        for (name, bytes) in corpus_files()
            .into_iter()
            .filter(|(n, _)| n.starts_with("current"))
        {
            let replay = RunReplay::from_bytes(&bytes).unwrap();
            let raw = base64::decode(&replay.inputs).unwrap();
            let log = StepLog::decode(&raw, replay.ticks, MAX).unwrap();
            let got = read_step_range(&raw, replay.ticks, MAX, 0, replay.ticks).unwrap();
            let want: Vec<RangeStep> = log
                .records
                .iter()
                .map(|r| match r {
                    Record::Ticks { run, inputs } => RangeStep::Ticks {
                        run: *run,
                        inputs: inputs.clone(),
                    },
                    Record::Picks { picks } => RangeStep::Picks {
                        picks: picks.clone(),
                    },
                })
                .collect();
            assert_eq!(got.steps, want, "{name}");
        }
    }
}
