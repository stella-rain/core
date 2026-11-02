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
        let mut records: Vec<Record> = Vec::new();
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
                    let mut inputs = Vec::with_capacity(players);
                    for _ in 0..players {
                        let block = *bytes
                            .get(pos..pos + 3)
                            .and_then(|b| <&[u8; 3]>::try_from(b).ok())
                            .ok_or(cut.clone())?;
                        pos += 3;
                        inputs.push(input_of_block(block, true).map_err(bad)?);
                    }
                    if matches!(records.last(), Some(Record::Ticks { inputs: before, .. }) if *before == inputs)
                    {
                        return Err(bad("a run repeats the input of the run before"));
                    }
                    world += run;
                    records.push(Record::Ticks { run, inputs });
                }
                PICKS => {
                    if picks_seen >= MAX_COMMANDS {
                        return Err(RunReplayError::TooManyCommands {
                            limit: MAX_COMMANDS,
                        });
                    }
                    let mut picks = Vec::with_capacity(players);
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
                    records.push(Record::Picks { picks });
                }
                JOIN | LEAVE => return Err(bad("a join or leave this build cannot run")),
                _ => return Err(bad("an unknown record")),
            }
        }
        if world != ticks {
            return Err(bad("there are fewer inputs than ticks"));
        }
        Ok(StepLog { present, records })
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

    /// The solo inputs and commands of a log, the inverse of `from_schema1`: only a party of
    /// one.
    pub(crate) fn to_solo(&self) -> Result<(Vec<Input>, Vec<TickCommand>), RunReplayError> {
        if self.present != 0b1 {
            return Err(RunReplayError::BadInputs(
                "a party this recorder cannot resume yet",
            ));
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

#[cfg(test)]
mod tests {
    use super::{Record, StepLog};
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

    /// Every run case of the corpus, current and frozen, as (name, bytes).
    fn corpus_runs() -> Vec<(String, Vec<u8>)> {
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
    fn a_party_other_than_the_engines_is_refused() {
        let one = run_with_picks();
        for players in [2u8, 3] {
            let log = StepLog {
                present: (1 << players) - 1,
                records: vec![Record::Ticks {
                    run: one.ticks,
                    inputs: vec![Input::default(); usize::from(players)],
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
    fn the_recorder_writes_schema_1_whatever_it_started_or_resumed_from() {
        use crate::pack::Pack;
        use crate::run_replay::RunRecorder;
        use crate::run_setup::RunSetup;
        let setup = RunSetup {
            schema_version: 2,
            ..RunSetup::default()
        };
        let mut rec = RunRecorder::start(&setup).unwrap();
        for tick in 0..30u8 {
            rec.step(Input {
                move_x: i8::try_from(tick).unwrap(),
                ..Input::default()
            })
            .unwrap();
        }
        let one = rec.replay();
        assert_eq!(one.setup.schema_version, 1);
        assert!(verify_run(&one.to_bytes()).is_ok());
        // A schema 2 prefix of the same run resumes into a schema 1 recorder.
        let two = schema_2_of(&one);
        let resumed = RunRecorder::resume_with_pack(&two, Pack::bundled()).unwrap();
        assert_eq!(resumed.replay(), one);
    }
}
