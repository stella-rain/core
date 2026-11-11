//! The step log: one in-memory model of a run's inputs for both replay schemas (ADR-REPL-2 point 4, ADR-REPL-5). A run holds one input per present player per world tick, and the
//! picks made on the steps in between; a schema 1 replay (inputs and a command list) is
//! migrated into it on read, and a schema 2 replay's `inputs` field is its bytes.
//!
//! The schema 2 bytes: a `present` byte (bit `i` for player slot `i`, at least one set, bits 3
//! to 7 zero), then records until the end. `0x00` TICKS: a varint `run` of at least 1, then one
//! 3-byte block (`flags`, `move_x`, `move_y`) per present slot in slot order, repeated for `run`
//! world ticks; two TICKS in a row never hold equal blocks. `0x01` PICKS: reserved, refused at
//! schema 2 (`BadInputs("a PICKS record is reserved at schema 2")`) and never given another
//! meaning: a run of the live rules opens no offer. `0x02 <slot>` JOIN and `0x03 <slot>` LEAVE
//! change the presence mask between two records (before the first, after the last): the records after one hold a block per slot
//! of the mask in force; slot 0 never leaves; at most `MAX_PRESENCE_CHANGES` of them. The runs
//! sum to `ticks`.
//!
//! `0x04` ACTIONS (ADR-REPL-5 point 8): one step that does not advance the world, one entry per
//! present slot in slot order, at least one not `0x00`. An entry is `0x00` (no action) or a kind
//! byte and its operands, each a LEB128 varint `u32`: `0x01` Equip (item, slot), `0x02` Replace
//! (item, slot), `0x03` Extract (item), `0x04` Craft (star), `0x05` TreePick (option). An unknown
//! kind is refused; at most `MAX_ACTION_STEPS` ACTIONS records, counted apart from the schema 1 picks. A log
//! with one has no solo view. Record bytes `0x05` to `0xFF` are unknown; `0x01` is never given
//! another meaning (a retired kind keeps its byte reserved). A schema 1 replay's picks are
//! migrated into `Record::Picks` in memory; `encode` refuses a log that holds one (a frozen
//! version 1 run is written at schema 1, by its `commands`).

use crate::input::{EquipAction, Input};
use crate::replay::{block_of, input_of_block, read_varint, write_varint};
use crate::run_replay::{CommandKind, MAX_ACTION_STEPS, RunReplayError, TickCommand};

const TICKS: u8 = 0;
const PICKS: u8 = 1;
const JOIN: u8 = 2;
const LEAVE: u8 = 3;
const ACTIONS: u8 = 4;

/// The most JOIN and LEAVE records a log holds: a guard against a crafted file, as
/// `MAX_COMMANDS` is for PICKS. No measure of real use exists yet.
pub(crate) const MAX_PRESENCE_CHANGES: u32 = 64;

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
    /// One step that does not advance the world: the equipment action of each present slot,
    /// if any.
    Actions { actions: Vec<Option<EquipAction>> },
    /// Slot `slot` (1 or 2) joins before the next record.
    Join { slot: u8 },
    /// Slot `slot` (1 or 2) leaves before the next record.
    Leave { slot: u8 },
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

    /// The canonical schema 2 bytes: `decode(encode(x)) == x` for any log `decode` can return.
    /// A PICKS record has none: it is reserved at schema 2 (a schema 1 log's picks live in its
    /// `commands`), so a log that holds one is `BadInputs`, and no writer can emit the byte.
    pub(crate) fn encode(&self) -> Result<Vec<u8>, RunReplayError> {
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
                Record::Join { slot } => out.extend([JOIN, *slot]),
                Record::Leave { slot } => out.extend([LEAVE, *slot]),
                Record::Actions { actions } => {
                    out.push(ACTIONS);
                    for action in actions {
                        write_action(&mut out, *action);
                    }
                }
                Record::Picks { .. } => {
                    return Err(RunReplayError::BadInputs(
                        "a PICKS record is reserved at schema 2",
                    ));
                }
            }
        }
        Ok(out)
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

    /// Whether the log holds a JOIN or a LEAVE.
    pub(crate) fn has_changes(&self) -> bool {
        self.records
            .iter()
            .any(|r| matches!(r, Record::Join { .. } | Record::Leave { .. }))
    }

    /// The presence mask after the last JOIN or LEAVE of the schema 2 `bytes` of a replay of
    /// `ticks` world ticks, or the `present` byte when there is none; `None` for bytes the
    /// scanner refuses. The savepoint's light read of a prefix: the scanner is the one decoder.
    pub(crate) fn presence_at_end(bytes: &[u8], ticks: u32) -> Option<u8> {
        let mut sink = LastMask { mask: 0 };
        scan(bytes, ticks, ticks, &mut sink).ok()?;
        Some(sink.mask)
    }

    /// The number of ACTIONS records, the unit `MAX_ACTION_STEPS` counts.
    pub(crate) fn actions_made(&self) -> u32 {
        let n = self
            .records
            .iter()
            .filter(|r| matches!(r, Record::Actions { .. }))
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
                Record::Picks { .. }
                | Record::Actions { .. }
                | Record::Join { .. }
                | Record::Leave { .. } => {
                    if world <= ticks {
                        records.push(record);
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
        if self.present != 0b1 || self.has_changes() {
            return Err(RunReplayError::BadInputs("a party has no solo view"));
        }
        let mut tick = 0u32;
        let mut commands = Vec::new();
        for record in &self.records {
            match record {
                Record::Ticks { run, .. } => tick = tick.saturating_add(*run),
                Record::Join { .. } | Record::Leave { .. } => {}
                Record::Actions { .. } => {
                    return Err(RunReplayError::BadInputs(
                        "a log with equipment actions has no solo view",
                    ));
                }
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
        if self.present != 0b1 || self.has_changes() {
            return Err(RunReplayError::BadInputs("a party has no solo view"));
        }
        let mut inputs = Vec::new();
        let mut commands = Vec::new();
        for record in &self.records {
            match record {
                Record::Ticks { run, inputs: held } => {
                    inputs.extend(std::iter::repeat_n(held[0], *run as usize));
                }
                Record::Join { .. } | Record::Leave { .. } => {}
                Record::Actions { .. } => {
                    return Err(RunReplayError::BadInputs(
                        "a log with equipment actions has no solo view",
                    ));
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

/// One step of a tick range (ADR-REPL-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeStep {
    /// `run` world ticks, each with these inputs (one per present slot, in slot order).
    Ticks { run: u32, inputs: Vec<Input> },
    /// One step that does not advance the world: the pick of each present slot, if any.
    Picks { picks: Vec<Option<u32>> },
    /// One step that does not advance the world: the equipment action of each present slot, if
    /// any (ADR-REPL-5 point 8).
    Actions { actions: Vec<Option<EquipAction>> },
    /// Slot `slot` joins before the next step.
    Join { slot: u8 },
    /// Slot `slot` leaves before the next step.
    Leave { slot: u8 },
}

/// The steps of world ticks `first..end` of a step log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRange {
    /// Bit `i` set when player slot `i` is present at tick 0 (the log's, not the range's).
    pub present: u8,
    /// The mask after every join and leave before world tick `first`: the party a re-simulation
    /// from the state after `first` ticks holds. A TICKS or PICKS step in `steps` is dense for
    /// the mask in force at that point, this one changed by each Join and Leave before it.
    pub present_at_first: u8,
    pub first: u32,
    pub end: u32,
    pub steps: Vec<RangeStep>,
}

/// The steps of world ticks `a..b` of a schema 2 step log of `ticks` world ticks, at most
/// `max_ticks` (ADR-REPL-5). Ticks are 0-based and the range is half-open: tick `i` takes the
/// state after `i` ticks to the state after `i + 1`.
///
/// - A TICKS run that overlaps the range is returned once, clipped to its overlap: a run is
///   never split into ticks and never expanded, so the memory is bounded by the records in the
///   range, not by `ticks`, `a`, `b` or a claimed run.
/// - A PICKS step, a JOIN or a LEAVE after `p` world ticks goes with the tick after it: it is in
///   the range iff `a <= p < b`; a range with `b == ticks` also holds those after the last
///   tick. An empty
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
    mask_at_first: u8,
}

impl RangeSink {
    fn new(ticks: u32, a: u32, b: u32) -> RangeSink {
        RangeSink {
            ticks,
            a,
            b,
            steps: Vec::new(),
            mask_at_first: 0,
        }
    }

    fn inside(&self, at: u32) -> bool {
        (self.a <= at && at < self.b)
            || (self.a < self.b && self.b == self.ticks && at == self.ticks)
    }

    fn change(&mut self, at: u32, mask: u8, step: RangeStep) {
        if at < self.a {
            self.mask_at_first = mask;
        }
        if self.inside(at) {
            self.steps.push(step);
        }
    }

    fn finish(self, present: u8) -> StepRange {
        StepRange {
            present,
            present_at_first: self.mask_at_first,
            first: self.a,
            end: self.b,
            steps: self.steps,
        }
    }
}

impl Sink for RangeSink {
    fn actions(&mut self, at: u32, actions: &[Option<EquipAction>]) {
        if self.inside(at) {
            self.steps.push(RangeStep::Actions {
                actions: actions.to_vec(),
            });
        }
    }

    fn start(&mut self, present: u8) {
        self.mask_at_first = present;
    }

    fn join(&mut self, at: u32, slot: u8, mask: u8) {
        self.change(at, mask, RangeStep::Join { slot });
    }

    fn leave(&mut self, at: u32, slot: u8, mask: u8) {
        self.change(at, mask, RangeStep::Leave { slot });
    }

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
}

impl RangeSink {
    /// A PICKS step of a log migrated from schema 1, after `at` world ticks (schema 2 bytes
    /// hold none: the scanner refuses the record).
    fn picks(&mut self, at: u32, picks: &[Option<u32>]) {
        if self.inside(at) {
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
            _ => t,
        });
        check_range(ticks, a, b)?;
        let mut sink = RangeSink::new(ticks, a, b);
        sink.start(self.present);
        let mut mask = self.present;
        let mut world = 0u32;
        for record in &self.records {
            match record {
                Record::Join { slot } => {
                    mask |= 1 << slot;
                    sink.join(world, *slot, mask);
                }
                Record::Leave { slot } => {
                    mask &= !(1 << slot);
                    sink.leave(world, *slot, mask);
                }
                Record::Ticks { run, inputs } => {
                    sink.ticks(world, *run, inputs);
                    world += run;
                }
                Record::Picks { picks } => sink.picks(world, picks),
                Record::Actions { actions } => sink.actions(world, actions),
            }
        }
        Ok(sink.finish(self.present))
    }
}

/// The bytes of one ACTIONS entry: `0x00` for none, else a kind byte and its varint operands.
fn write_action(out: &mut Vec<u8>, action: Option<EquipAction>) {
    let (kind, operands): (u8, &[u32]) = match &action {
        None => return out.push(0),
        Some(EquipAction::Equip { item, slot }) => (1, &[*item, *slot]),
        Some(EquipAction::Replace { item, slot }) => (2, &[*item, *slot]),
        Some(EquipAction::Extract { item }) => (3, &[*item]),
        Some(EquipAction::Craft { star }) => (4, &[*star]),
        Some(EquipAction::TreePick { option }) => (5, &[*option]),
    };
    out.push(kind);
    for operand in operands {
        write_varint(out, *operand);
    }
}

/// Reads one ACTIONS entry at `pos`: `0x00` for none, else a kind byte and its varint operands.
fn read_action(bytes: &[u8], pos: &mut usize) -> Result<Option<EquipAction>, RunReplayError> {
    let bad = RunReplayError::BadInputs;
    let cut = bad("ends in the middle of a record");
    let kind = *bytes.get(*pos).ok_or(cut.clone())?;
    *pos += 1;
    let operand = |pos: &mut usize| {
        read_varint(bytes, pos).map_err(|why| {
            if why == "ends in the middle of a number" {
                cut.clone()
            } else {
                bad(why)
            }
        })
    };
    Ok(Some(match kind {
        0 => return Ok(None),
        1 => EquipAction::Equip {
            item: operand(pos)?,
            slot: operand(pos)?,
        },
        2 => EquipAction::Replace {
            item: operand(pos)?,
            slot: operand(pos)?,
        },
        3 => EquipAction::Extract {
            item: operand(pos)?,
        },
        4 => EquipAction::Craft {
            star: operand(pos)?,
        },
        5 => EquipAction::TreePick {
            option: operand(pos)?,
        },
        _ => return Err(bad("an action kind is unknown")),
    }))
}

/// What the scanner reports for each record, by reference: nothing is allocated for it.
trait Sink {
    /// The `present` byte, before any record.
    fn start(&mut self, _present: u8) {}
    /// Slot `slot` joined after `at` world ticks; `mask` is the party after it.
    fn join(&mut self, _at: u32, _slot: u8, _mask: u8) {}
    /// Slot `slot` left after `at` world ticks; `mask` is the party after it.
    fn leave(&mut self, _at: u32, _slot: u8, _mask: u8) {}
    /// `run` world ticks from world tick `start`, with one input per present slot.
    fn ticks(&mut self, start: u32, run: u32, inputs: &[Input]);
    /// One ACTIONS step after `at` world ticks.
    fn actions(&mut self, _at: u32, _actions: &[Option<EquipAction>]) {}
}

/// The sink of `decode`: every record, pushed.
struct Collect {
    records: Vec<Record>,
}

/// The sink of `presence_at_end`: the mask after the last change.
struct LastMask {
    mask: u8,
}

impl Sink for LastMask {
    fn start(&mut self, present: u8) {
        self.mask = present;
    }
    fn join(&mut self, _at: u32, _slot: u8, mask: u8) {
        self.mask = mask;
    }
    fn leave(&mut self, _at: u32, _slot: u8, mask: u8) {
        self.mask = mask;
    }
    fn ticks(&mut self, _start: u32, _run: u32, _inputs: &[Input]) {}
}

impl Sink for Collect {
    fn actions(&mut self, _at: u32, actions: &[Option<EquipAction>]) {
        self.records.push(Record::Actions {
            actions: actions.to_vec(),
        });
    }

    fn join(&mut self, _at: u32, slot: u8, _mask: u8) {
        self.records.push(Record::Join { slot });
    }

    fn leave(&mut self, _at: u32, slot: u8, _mask: u8) {
        self.records.push(Record::Leave { slot });
    }

    fn ticks(&mut self, _start: u32, run: u32, inputs: &[Input]) {
        self.records.push(Record::Ticks {
            run,
            inputs: inputs.to_vec(),
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
    sink.start(present);
    let mut mask = present;
    let mut changes = 0u32;
    let mut players = StepLog::players(mask);
    let mut pos = 1;
    let mut world = 0u32;
    let mut actions_seen = 0u32;
    let mut actions: Vec<Option<EquipAction>> = Vec::with_capacity(players);
    // The inputs of the record before, when it was a TICKS record.
    let mut before: Vec<Input> = Vec::with_capacity(players);
    let mut after_ticks = false;
    let mut inputs: Vec<Input> = Vec::with_capacity(players);
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
            // Reserved: the byte stays assigned and is never given another meaning. A schema 2
            // run holds no offer (ADR-REPL-5); a frozen run's picks are schema 1 data.
            PICKS => return Err(bad("a PICKS record is reserved at schema 2")),
            ACTIONS => {
                if actions_seen >= MAX_ACTION_STEPS {
                    return Err(RunReplayError::TooManyActions {
                        limit: MAX_ACTION_STEPS,
                    });
                }
                actions.clear();
                for _ in 0..players {
                    actions.push(read_action(bytes, &mut pos)?);
                }
                if actions.iter().all(Option::is_none) {
                    return Err(bad("a record has no action"));
                }
                actions_seen += 1;
                sink.actions(world, &actions);
                after_ticks = false;
            }
            JOIN | LEAVE => {
                let slot = *bytes.get(pos).ok_or(cut.clone())?;
                pos += 1;
                if slot >= crate::id::MAX_PLAYERS {
                    return Err(bad("a slot beyond the party"));
                }
                let bit = 1u8 << slot;
                if opcode == JOIN && mask & bit != 0 {
                    return Err(bad("a join of a slot that is present"));
                }
                if opcode == LEAVE && slot == 0 {
                    return Err(bad("slot 0 never leaves"));
                }
                if opcode == LEAVE && mask & bit == 0 {
                    return Err(bad("a leave of a slot that is not present"));
                }
                if changes >= MAX_PRESENCE_CHANGES {
                    return Err(bad("more joins and leaves than the cap"));
                }
                changes += 1;
                if opcode == JOIN {
                    mask |= bit;
                    sink.join(world, slot, mask);
                } else {
                    mask &= !bit;
                    sink.leave(world, slot, mask);
                }
                players = StepLog::players(mask);
                after_ticks = false;
            }
            _ => return Err(bad("an unknown record")),
        }
    }
    if world != ticks {
        return Err(bad("there are fewer inputs than ticks"));
    }
    Ok(present)
}

#[cfg(test)]
#[path = "step_log_tests.rs"]
mod tests;
