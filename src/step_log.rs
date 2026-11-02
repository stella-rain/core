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
            ("01 00", 0, "ends in the middle of a record"),
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
            .find(|r| !r.unpack_commands().unwrap().is_empty())
            .expect("a run with picks")
    }

    #[test]
    fn a_schema_2_replay_verifies_like_the_schema_1_one() {
        for (name, bytes) in corpus_runs() {
            let one = RunReplay::from_bytes(&bytes).unwrap();
            let two = schema_2_of(&one);
            assert_eq!(two.setup.schema_version, 2);
            let expected = verify_run(&bytes);
            assert!(expected.is_ok(), "{name}: {expected:?}");
            assert_eq!(verify_run(&two.to_bytes()), expected, "{name}");
        }
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
}
