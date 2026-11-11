//! Tests of the step log.

use super::{RangeStep, Record, StepLog, StepRange, read_step_range};
use crate::base64;
use crate::input::Input;
use crate::run_replay::{RunReplay, RunReplayError, RunVerified, verify_run_with};

fn reserved() -> RunReplayError {
    RunReplayError::BadInputs("a PICKS record is reserved at schema 2")
}

#[test]
fn encode_refuses_a_picks_record() {
    let log = StepLog {
        present: 0b1,
        records: vec![Record::Picks {
            picks: vec![Some(0)],
        }],
    };
    assert_eq!(log.encode(), Err(reserved()));
}

/// The bytes of a log as a hostile file could hold them, a PICKS record included (`encode`
/// refuses it): each record by `encode`, the PICKS bytes by hand.
fn raw_encode(log: &StepLog) -> Vec<u8> {
    let mut out = vec![log.present];
    for record in &log.records {
        if let Record::Picks { picks } = record {
            out.push(super::PICKS);
            for pick in picks {
                match pick {
                    None => out.push(0),
                    Some(index) => {
                        out.push(1);
                        crate::replay::write_varint(&mut out, *index);
                    }
                }
            }
        } else {
            let one = StepLog {
                present: log.present,
                records: vec![record.clone()],
            };
            out.extend(&one.encode().unwrap()[1..]);
        }
    }
    out
}

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
    assert_eq!(expected.encode().unwrap(), hex(bytes));
}

/// PICKS (`0x01`) is reserved at schema 2: the byte stays assigned and every reader
/// refuses it with the same error (ADR-REPL-5).
#[test]
fn a_picks_record_is_reserved_at_schema_2() {
    const WHY: &str = "a PICKS record is reserved at schema 2";
    for (bytes, ticks) in [
        ("01 01 01 00", 0),
        ("01 00 05 02 00 00 01 01 00 00 02 02 00 00", 7),
        ("01 01 01 00 02 02 00 00", 2),
    ] {
        assert_eq!(bad_inputs(bytes, ticks), WHY, "{bytes}");
        assert_eq!(
            read_step_range(&hex(bytes), ticks, 1_000_000, 0, ticks),
            Err(RunReplayError::BadInputs(WHY)),
            "{bytes}"
        );
        assert_eq!(
            StepLog::presence_at_end(&hex(bytes), ticks),
            None,
            "{bytes}"
        );
    }
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
    assert_eq!(expected.encode().unwrap(), hex(bytes));
}

#[test]
fn a_party_with_a_gap_has_blocks_for_the_present_slots_only() {
    // Slots 0 and 2; slot 2 holds interact (bit 6) and the stick at -127.
    let bytes = "05 00 02 00 00 00 40 81 00";
    let log = decode(bytes, 2).unwrap();
    assert_eq!(log.present, 0b101);
    let Record::Ticks { inputs, .. } = &log.records[0] else {
        panic!("a run")
    };
    assert_eq!(inputs.len(), 2);
    assert!(inputs[1].interact && inputs[1].move_x == -127);
    assert_eq!(log.encode().unwrap(), hex(bytes));
}

#[test]
fn a_play_may_not_end_on_a_pick() {
    assert_eq!(
        bad_inputs("01 00 01 00 00 00 01 01 02", 1),
        "a PICKS record is reserved at schema 2"
    );
}

// --- Refusals --------------------------------------------------------------------------

#[test]
fn every_refusal_has_its_message() {
    let cases: [(&str, u32, &str); 14] = [
        ("00 00 01 00 00 00", 1, "no player is present"),
        ("09 00 01 00 00 00", 1, "a player slot beyond the party"),
        ("01 00 01 80 00 00", 1, "a run has unknown flags"),
        ("01 07 01 00 00 00", 1, "an unknown record"),
        ("01 02 00", 0, "a join of a slot that is present"),
        ("01 03 00", 0, "slot 0 never leaves"),
        ("01 00 00 00 00 00", 0, "a run has length 0"),
        (
            "01 00 01 00 00 00 00 01 00 00 00",
            2,
            "a run repeats the input of the run before",
        ),
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

/// `verify_run` for a run on the bundled content, or else for a frozen run or a copy of one:
/// with the first frozen content that does not refuse it as another content (the runs are pinned to the bundled pack or to one of the
/// frozen packs, and verify only with the content they were frozen with).
fn verify_run(bytes: &[u8]) -> Result<RunVerified, RunReplayError> {
    let mut last = crate::run_replay::verify_run(bytes);
    if last != Err(RunReplayError::ContentChanged) {
        return last;
    }
    for (name, _) in corpus_files()
        .into_iter()
        .filter(|(n, _)| n.starts_with("v1/"))
    {
        last = verify_run_with(bytes, &content_of(&name));
        if last != Err(RunReplayError::ContentChanged) {
            break;
        }
    }
    last
}

/// `verify_run` for a frozen version 1 recording: with the version 1 pack it was recorded on,
/// never the bundled files (which version 2 edits). The first pack that does not refuse it
/// as another content decides.
fn verify_v1(bytes: &[u8]) -> Result<RunVerified, RunReplayError> {
    let mut last = Err(RunReplayError::ContentChanged);
    for (name, _) in corpus_files()
        .into_iter()
        .filter(|(n, _)| n.starts_with("v1/"))
    {
        last = verify_run_with(bytes, &content_of(&name));
        if last != Err(RunReplayError::ContentChanged) {
            break;
        }
    }
    last
}

#[allow(dead_code, reason = "shared with tests/, which uses more of it")]
mod data_root {
    include!("../tests/data_root.rs");
}

/// The content a corpus run (`"v1/easy"`) plays with: the pack its `case.json` pins, which is
/// in its tree's `packs/` when it is not the bundled one. The frozen runs verify only with the
/// content they were frozen with.
fn content_of(name: &str) -> crate::run_content::RunContent {
    let (tree, case) = name.split_once('/').unwrap();
    let root = data_root::data_root().join("tests/corpus");
    let pin: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join(tree).join("runs").join(case).join("case.json")).unwrap(),
    )
    .unwrap();
    let file = root
        .join(tree)
        .join("packs")
        .join(format!("{}.json", pin["pack_sha256"].as_str().unwrap()));
    match std::fs::read(file) {
        Ok(bytes) => crate::pack::Pack::from_bytes(&bytes)
            .unwrap()
            .content()
            .clone(),
        Err(_) => crate::run_content::RunContent::bundled().clone(),
    }
}

/// The frozen `v1/` runs in their schema 1 form: they are offers-era runs by design. A
/// `current/` run is an equipment run, which has no solo view (`to_solo` refuses its actions).
fn corpus_runs() -> Vec<(String, Vec<u8>)> {
    let runs: Vec<_> = corpus_files()
        .into_iter()
        .filter(|(name, _)| name.starts_with("v1/"))
        .map(|(name, bytes)| {
            let one = schema_1_form(&RunReplay::from_bytes(&bytes).unwrap());
            (name, one.to_bytes())
        })
        .collect();
    assert_eq!(runs.len(), 9, "the frozen run cases are read");
    runs
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
        // A log that holds picks has no schema 2 bytes: PICKS is reserved at schema 2, and
        // `encode` refuses it, so no writer can emit the reserved byte.
        if commands.is_empty() {
            let decoded = StepLog::decode(&log.encode().unwrap(), replay.ticks, u32::MAX);
            assert_eq!(decoded, Ok(log), "{name}");
        } else {
            assert_eq!(log.encode(), Err(reserved()), "{name}");
        }
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
    two.inputs = base64::encode(&raw_encode(log));
    two.commands = String::new();
    two
}

/// A corpus run with picks, as (schema 1 replay).
fn run_with_picks() -> RunReplay {
    corpus_runs()
        .into_iter()
        .map(|(name, bytes)| (name, RunReplay::from_bytes(&bytes).unwrap()))
        .find(|(name, r)| {
            !r.unpack_commands().unwrap().is_empty()
                && verify_run_with(&r.to_bytes(), &content_of(name)).is_ok()
        })
        .map(|(_, r)| r)
        .expect("a run with picks")
}

#[test]
fn a_schema_2_form_of_a_frozen_run_is_reserved() {
    // Every frozen run holds picks, so none has a schema 2 form that verifies: PICKS is
    // reserved at schema 2 and the verifier refuses the file as the decoder does. The runs
    // themselves verify through their schema 1 form (`corpus::`).
    let mut refused = 0;
    for (name, bytes) in corpus_runs() {
        let one = RunReplay::from_bytes(&bytes).unwrap();
        assert!(!one.unpack_commands().unwrap().is_empty(), "{name}");
        let two = schema_2_of(&one);
        assert_eq!(two.setup.schema_version, 2);
        assert_eq!(
            verify_run_with(&two.to_bytes(), &content_of(&name)),
            Err(reserved()),
            "{name}"
        );
        refused += 1;
    }
    assert_eq!(refused, 9);
}

#[test]
fn schema_2_has_no_command_list() {
    let one = run_with_picks();
    let mut two = schema_2_of(&one);
    two.commands = one.commands.clone();
    assert_eq!(
        verify_v1(&two.to_bytes()),
        Err(RunReplayError::BadCommands("schema 2 has no command list"))
    );
}

#[test]
fn a_pick_in_schema_2_is_reserved_whatever_it_picks() {
    let one = run_with_picks();
    let mut log = one.step_log(u32::MAX).unwrap();
    let first = log
        .records
        .iter_mut()
        .find_map(|r| match r {
            Record::Picks { picks } => Some(picks),
            _ => None,
        })
        .unwrap();
    first[0] = Some(999);
    assert_eq!(
        verify_v1(&schema_2_with(&one, &log).to_bytes()),
        Err(reserved())
    );
}

#[test]
fn a_pick_with_no_offer_open_is_reserved() {
    let one = run_with_picks();
    let mut log = one.step_log(u32::MAX).unwrap();
    log.records.insert(
        0,
        Record::Picks {
            picks: vec![Some(0)],
        },
    );
    assert_eq!(
        verify_v1(&schema_2_with(&one, &log).to_bytes()),
        Err(reserved())
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
            verify_v1(&schema_2_with(&one, &log).to_bytes()),
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
        verify_v1(&flagged.to_bytes()),
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
    let mut forged = run_with_picks();
    forged.setup.schema_version = 2;
    forged.ticks = 4_000_000_000;
    forged.commands = String::new();
    let log = StepLog {
        present: 1,
        records: vec![Record::Ticks {
            run: 4_000_000_000,
            inputs: vec![Input::default()],
        }],
    };
    forged.inputs = base64::encode(&log.encode().unwrap());
    assert_eq!(forged.unpack_commands(), Ok(Vec::new()));
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
        raw_encode(&log),
        hex("01 00 05 02 00 00 01 01 00 00 02 02 00 00")
    );
}

// ---- the range reader (ADR-REPL-5) ----

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
    Actions(Vec<Option<EquipAction>>),
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
                let inside = (a <= world && world < b) || (a < b && b == ticks && world == ticks);
                if inside {
                    out.push(Entry::Picks(picks.clone()));
                }
            }
            Record::Actions { actions } => {
                // An ACTIONS step does not advance the world: in a range as a PICKS is.
                let inside = (a <= world && world < b) || (a < b && b == ticks && world == ticks);
                if inside {
                    out.push(Entry::Actions(actions.clone()));
                }
            }
            Record::Join { .. } | Record::Leave { .. } => {
                unreachable!("these logs hold no join or leave")
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
            RangeStep::Actions { actions } => out.push(Entry::Actions(actions.clone())),
            RangeStep::Join { .. } | RangeStep::Leave { .. } => {
                unreachable!("these logs hold no join or leave")
            }
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
    // Even trailing picks are not in an empty range (a log migrated from schema 1).
    let trailing = mem(vec![ticks(2, main_held()), picks(&[7])]);
    assert_eq!(mem_steps(&trailing, 2, 2, 2), []);
    // And the log is still validated.
    assert_eq!(
        range("01 00 05 02 00", 5, 0, 0),
        Err(RunReplayError::BadInputs("ends in the middle of a record"))
    );
}

// The in-memory model of a log migrated from schema 1 still holds PICKS records, and
// `StepLog::range` cuts them by the rule of the range reader; schema 2 bytes hold none.

fn mem(records: Vec<Record>) -> StepLog {
    StepLog {
        present: 1,
        records,
    }
}

fn ticks(run: u32, input: Input) -> Record {
    Record::Ticks {
        run,
        inputs: vec![input],
    }
}

fn picks(indexes: &[u32]) -> Record {
    Record::Picks {
        picks: indexes.iter().map(|i| Some(*i)).collect(),
    }
}

fn mem_steps(log: &StepLog, _ticks: u32, a: u32, b: u32) -> Vec<RangeStep> {
    log.range(a, b).unwrap().steps
}

#[test]
fn a_pick_comes_with_the_tick_after_it() {
    // Tick 0 held, a pick of index 0 after it (position 1), then two idle ticks.
    let log = mem(vec![ticks(1, main_held()), picks(&[0]), ticks(2, idle())]);
    assert_eq!(mem_steps(&log, 3, 0, 1), [run_of(1, main_held())]);
    assert_eq!(mem_steps(&log, 3, 1, 3), [pick_of(0), run_of(2, idle())]);
    assert_eq!(
        mem_steps(&log, 3, 0, 3),
        [run_of(1, main_held()), pick_of(0), run_of(2, idle())]
    );
    assert_eq!(mem_steps(&log, 3, 1, 2), [pick_of(0), run_of(1, idle())]);
    assert_eq!(
        mem_steps(&log, 3, 0, 2),
        [run_of(1, main_held()), pick_of(0), run_of(1, idle())]
    );
    assert_eq!(mem_steps(&log, 3, 2, 3), [run_of(1, idle())]);
}

#[test]
fn a_pick_before_the_first_tick_is_in_a_range_that_starts_at_0() {
    let log = mem(vec![picks(&[0]), ticks(5, main_held())]);
    assert_eq!(
        mem_steps(&log, 5, 0, 2),
        [pick_of(0), run_of(2, main_held())]
    );
    assert_eq!(mem_steps(&log, 5, 1, 2), [run_of(1, main_held())]);
    assert_eq!(mem_steps(&log, 5, 0, 0), []);
}

#[test]
fn the_picks_after_the_last_tick_belong_to_a_range_ending_there() {
    let log = mem(vec![ticks(2, main_held()), picks(&[7])]);
    assert_eq!(
        mem_steps(&log, 2, 0, 2),
        [run_of(2, main_held()), pick_of(7)]
    );
    assert_eq!(mem_steps(&log, 2, 0, 1), [run_of(1, main_held())]);
    assert_eq!(
        mem_steps(&log, 2, 1, 2),
        [run_of(1, main_held()), pick_of(7)]
    );
}

#[test]
fn several_picks_at_one_position_keep_their_order() {
    let log = mem(vec![
        ticks(1, main_held()),
        picks(&[3]),
        picks(&[4]),
        ticks(1, idle()),
    ]);
    assert_eq!(
        mem_steps(&log, 2, 1, 2),
        [pick_of(3), pick_of(4), run_of(1, idle())]
    );
    assert_eq!(mem_steps(&log, 2, 0, 1), [run_of(1, main_held())]);
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
    let cases: [(&str, u32); 14] = [
        ("00 00 01 00 00 00", 1),
        ("09 00 01 00 00 00", 1),
        ("01 00 01 80 00 00", 1),
        ("01 07 01 00 00 00", 1),
        ("01 02 00", 0),
        ("01 03 00", 0),
        ("01 00 00 00 00 00", 0),
        ("01 00 01 00 00 00 00 01 00 00 00", 2),
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
    // A PICKS record after the range ends is refused too: reserved at schema 2.
    let late = "01 00 02 02 00 00 01 01 00";
    assert_eq!(StepLog::decode(&hex(late), 2, MAX).unwrap_err(), reserved());
    assert_eq!(range(late, 2, 0, 1).unwrap_err(), reserved());
    assert_eq!(range(late, 2, 0, 0).unwrap_err(), reserved());
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
                Record::Actions { actions } => RangeStep::Actions {
                    actions: actions.clone(),
                },
                Record::Join { slot } => RangeStep::Join { slot: *slot },
                Record::Leave { slot } => RangeStep::Leave { slot: *slot },
            })
            .collect();
        assert_eq!(got.steps, want, "{name}");
    }
}

// --- Join and leave --------------------------------------------------------------------

fn rec_ticks(run: u32, inputs: Vec<Input>) -> Record {
    Record::Ticks { run, inputs }
}

#[test]
fn a_join_and_a_leave_decode_and_encode_back() {
    let d = Input::default();
    let bytes = "01 00 01 02 00 00 02 01 00 02 02 00 00 00 00 00";
    let log = decode(bytes, 3).unwrap();
    assert_eq!(
        log.records,
        vec![
            rec_ticks(1, vec![main_held()]),
            Record::Join { slot: 1 },
            rec_ticks(2, vec![main_held(), d]),
        ]
    );
    assert_eq!(log.encode().unwrap(), hex(bytes));
    let bytes = "03 00 03 02 00 00 00 00 00 03 01";
    let log = decode(bytes, 3).unwrap();
    assert_eq!(
        log.records,
        vec![
            rec_ticks(3, vec![main_held(), d]),
            Record::Leave { slot: 1 }
        ]
    );
    assert_eq!(log.encode().unwrap(), hex(bytes));
    let bytes = "07 03 01 00 01 02 00 00 00 00 00";
    let log = decode(bytes, 1).unwrap();
    assert_eq!(
        log.records,
        vec![
            Record::Leave { slot: 1 },
            rec_ticks(1, vec![main_held(), d])
        ]
    );
    assert_eq!(log.encode().unwrap(), hex(bytes));
}

#[test]
fn a_change_is_refused_with_its_message() {
    let cases: [(&str, u32, &str); 11] = [
        ("01 02 01 02 01", 0, "a join of a slot that is present"),
        ("01 02 03", 0, "a slot beyond the party"),
        ("01 03 01", 0, "a leave of a slot that is not present"),
        ("03 03 00", 0, "slot 0 never leaves"),
        ("07 03 01 03 01", 0, "a leave of a slot that is not present"),
        ("01 02", 0, "ends in the middle of a record"),
        ("01 03", 0, "ends in the middle of a record"),
        (
            "01 02 01 00 01 00 00 00",
            1,
            "ends in the middle of a record",
        ),
        ("01 05 00", 0, "an unknown record"),
        ("01 02 00", 0, "a join of a slot that is present"),
        (
            "03 00 01 02 00 00 00 00 00 00 01 02 00 00 00 00 00",
            2,
            "a run repeats the input of the run before",
        ),
    ];
    for (bytes, ticks, why) in cases {
        assert_eq!(bad_inputs(bytes, ticks), why, "{bytes}");
    }
}

#[test]
fn the_cap_on_changes_is_sixty_four() {
    let pairs = |n: usize| {
        let mut text = String::from("01");
        for _ in 0..n {
            text.push_str(" 02 01 03 01");
        }
        text
    };
    assert!(decode(&pairs(32), 0).is_ok());
    let over = format!("{} 02 01", pairs(32));
    assert_eq!(bad_inputs(&over, 0), "more joins and leaves than the cap");
}

#[test]
fn a_change_resets_the_run_repeat_check() {
    let same = "03 00 01 02 00 00 00 00 00 03 01 02 01 00 01 02 00 00 00 00 00";
    assert!(decode(same, 2).is_ok());
}

#[test]
fn a_log_with_a_change_has_no_solo_view() {
    let log = decode("01 00 01 02 00 00 02 01", 1).unwrap();
    assert_eq!(
        log.to_solo().unwrap_err(),
        RunReplayError::BadInputs("a party has no solo view")
    );
    assert_eq!(
        log.solo_commands().unwrap_err(),
        RunReplayError::BadInputs("a party has no solo view")
    );
}

#[test]
fn a_prefix_keeps_a_change_while_the_world_has_not_passed_it() {
    let m = main_held();
    let log = StepLog {
        present: 1,
        records: vec![
            rec_ticks(3, vec![m]),
            Record::Join { slot: 1 },
            rec_ticks(3, vec![m, Input::default()]),
        ],
    };
    let p3 = log.clone().prefix(3);
    assert_eq!(p3.records, log.records[..2].to_vec());
    let p4 = log.clone().prefix(4);
    assert_eq!(p4.records.len(), 3);
    assert_eq!(p4.records[2], rec_ticks(1, vec![m, Input::default()]));
    assert_eq!(log.prefix(2).records, vec![rec_ticks(2, vec![m])]);
}

fn range_of(bytes: &str, ticks: u32, a: u32, b: u32) -> StepRange {
    let got = read_step_range(&hex(bytes), ticks, 1_000_000, a, b).unwrap();
    let log = decode(bytes, ticks).unwrap();
    assert_eq!(log.range(a, b).unwrap(), got, "{bytes} {a}..{b}");
    got
}

#[test]
fn the_range_reader_carries_the_changes() {
    let m = main_held();
    let d = Input::default();
    // Three solo ticks, a join, three ticks of two.
    let bytes = "01 00 03 02 00 00 02 01 00 03 02 00 00 00 00 00";
    let r = range_of(bytes, 6, 0, 6);
    assert_eq!(
        r.steps,
        vec![
            RangeStep::Ticks {
                run: 3,
                inputs: vec![m]
            },
            RangeStep::Join { slot: 1 },
            RangeStep::Ticks {
                run: 3,
                inputs: vec![m, d]
            },
        ]
    );
    assert_eq!(r.present_at_first, 0b1);
    let r = range_of(bytes, 6, 3, 6);
    assert_eq!(r.steps[0], RangeStep::Join { slot: 1 });
    assert_eq!(r.steps.len(), 2);
    assert_eq!(r.present_at_first, 0b1);
    let r = range_of(bytes, 6, 4, 6);
    assert_eq!(
        r.steps,
        vec![RangeStep::Ticks {
            run: 2,
            inputs: vec![m, d]
        }]
    );
    assert_eq!(r.present_at_first, 0b11);
    let r = range_of(bytes, 6, 0, 3);
    assert_eq!(
        r.steps,
        vec![RangeStep::Ticks {
            run: 3,
            inputs: vec![m]
        }]
    );
    assert!(range_of(bytes, 6, 3, 3).steps.is_empty());
    let r = range_of(bytes, 6, 6, 6);
    assert!(r.steps.is_empty());
    assert_eq!(r.present_at_first, 0b11);
    let leave = "03 00 03 02 00 00 00 00 00 03 01";
    let r = range_of(leave, 3, 0, 3);
    assert_eq!(r.steps.len(), 2);
    assert_eq!(r.steps[1], RangeStep::Leave { slot: 1 });
    assert!(range_of(leave, 3, 3, 3).steps.is_empty());
}

#[test]
fn the_mask_at_the_end_of_a_prefix() {
    let at = |text: &str, ticks| StepLog::presence_at_end(&hex(text), ticks);
    assert_eq!(at("07 03 01", 0), Some(0b101));
    assert_eq!(at("07 03 01 00 01 02 00 00 00 00 00", 1), Some(0b101));
    assert_eq!(at("01 02 01", 0), Some(0b011));
    assert_eq!(at("01 02 00", 0), None);
    assert_eq!(at("", 0), None);
}

// --- Equipment actions (ADR-REPL-5 point 8, ADR-SIM-7) ---------------------------------

use crate::input::EquipAction;

const SOLO_ACTIONS: &str = "01 00 01 02 00 00 04 02 03 07 00 01 00 00 00";

fn replace_3_7() -> EquipAction {
    EquipAction::Replace { item: 3, slot: 7 }
}

fn solo_actions_log() -> StepLog {
    StepLog {
        present: 0b1,
        records: vec![
            Record::Ticks {
                run: 1,
                inputs: vec![main_held()],
            },
            Record::Actions {
                actions: vec![Some(replace_3_7())],
            },
            Record::Ticks {
                run: 1,
                inputs: vec![Input::default()],
            },
        ],
    }
}

#[test]
fn an_actions_record_decodes_and_encodes_back() {
    assert_eq!(decode(SOLO_ACTIONS, 2), Ok(solo_actions_log()));
    assert_eq!(solo_actions_log().encode().unwrap(), hex(SOLO_ACTIONS));
    let party = StepLog {
        present: 0b11,
        records: vec![Record::Actions {
            actions: vec![None, Some(EquipAction::TreePick { option: 4 })],
        }],
    };
    assert_eq!(decode("03 04 00 05 04", 0), Ok(party.clone()));
    assert_eq!(party.encode().unwrap(), hex("03 04 00 05 04"));
}

#[test]
fn every_action_kind_round_trips() {
    let all = [
        EquipAction::Equip { item: 1, slot: 300 },
        EquipAction::Replace { item: 0, slot: 2 },
        EquipAction::Extract { item: 128 },
        EquipAction::Craft { star: u32::MAX },
        EquipAction::TreePick { option: 0 },
    ];
    for action in all {
        let log = StepLog {
            present: 0b1,
            records: vec![Record::Actions {
                actions: vec![Some(action)],
            }],
        };
        assert_eq!(StepLog::decode(&log.encode().unwrap(), 0, 0), Ok(log));
    }
}

#[test]
fn a_bad_actions_record_is_refused_with_its_phrase() {
    assert_eq!(bad_inputs("01 04 00", 0), "a record has no action");
    assert_eq!(bad_inputs("01 04 06 00", 0), "an action kind is unknown");
    assert_eq!(bad_inputs("01 04 04", 0), "ends in the middle of a record");
    assert_eq!(bad_inputs("03 04 00", 0), "ends in the middle of a record");
    assert_eq!(bad_inputs("01 05", 0), "an unknown record");
}

#[test]
fn equal_runs_around_an_actions_record_are_allowed() {
    let with = "01 00 01 00 00 00 04 03 00 00 01 00 00 00";
    let log = decode(with, 2).unwrap();
    assert_eq!(log.records.len(), 3);
    assert_eq!(
        bad_inputs("01 00 01 00 00 00 00 01 00 00 00", 2),
        "a run repeats the input of the run before"
    );
}

#[test]
fn the_record_bytes_are_assigned_once() {
    // 0x01 stays assigned to PICKS, reserved: it is refused as that, never as unknown.
    assert_eq!(decode("01 01 01 00 00 01 00 00 00", 1), Err(reserved()));
    for byte in 5..=255u8 {
        let bytes = [1, byte];
        assert_eq!(
            StepLog::decode(&bytes, 0, 0),
            Err(RunReplayError::BadInputs("an unknown record")),
            "{byte}"
        );
    }
}

#[test]
fn actions_in_a_range_follow_the_rule_of_picks() {
    let steps = |a, b| range(SOLO_ACTIONS, 2, a, b).unwrap().steps;
    let first = RangeStep::Ticks {
        run: 1,
        inputs: vec![main_held()],
    };
    let actions = RangeStep::Actions {
        actions: vec![Some(replace_3_7())],
    };
    let second = RangeStep::Ticks {
        run: 1,
        inputs: vec![Input::default()],
    };
    assert_eq!(
        steps(0, 2),
        [first.clone(), actions.clone(), second.clone()]
    );
    assert_eq!(steps(1, 2), [actions.clone(), second]);
    assert_eq!(steps(0, 1), [first]);
    assert_eq!(steps(2, 2), []);
    let last = "01 00 02 02 00 00 04 03 00";
    assert_eq!(range(last, 2, 0, 2).unwrap().steps.len(), 2);
    assert_eq!(range(last, 2, 1, 2).unwrap().steps.len(), 2);
    assert_eq!(range(last, 2, 0, 1).unwrap().steps.len(), 1);
    // The decoded log's range is the reader's.
    assert_eq!(solo_actions_log().range(0, 2), range(SOLO_ACTIONS, 2, 0, 2));
}

#[test]
fn a_prefix_keeps_the_actions_after_its_tick() {
    let at = |t| solo_actions_log().prefix(t).records.len();
    assert_eq!(at(1), 2);
    assert_eq!(at(0), 0);
    assert!(
        !solo_actions_log()
            .prefix(0)
            .records
            .iter()
            .any(|r| matches!(r, Record::Actions { .. }))
    );
    assert!(
        solo_actions_log()
            .prefix(1)
            .records
            .iter()
            .any(|r| matches!(r, Record::Actions { .. }))
    );
}

#[test]
fn a_log_with_actions_has_no_solo_view() {
    let why = RunReplayError::BadInputs("a log with equipment actions has no solo view");
    assert_eq!(solo_actions_log().solo_commands(), Err(why.clone()));
    assert_eq!(solo_actions_log().to_solo(), Err(why));
}

fn actions_log(actions: u32) -> Vec<u8> {
    let mut bytes = vec![1];
    for _ in 0..actions {
        bytes.extend([4, 3, 0]);
    }
    bytes
}

#[test]
fn the_action_steps_are_capped() {
    let max = crate::run_replay::MAX_ACTION_STEPS;
    assert_eq!(max, 4096);
    assert!(StepLog::decode(&actions_log(max), 0, 0).is_ok());
    assert_eq!(
        StepLog::decode(&actions_log(max + 1), 0, 0),
        Err(RunReplayError::TooManyActions { limit: max })
    );
    let log = StepLog::decode(&actions_log(3), 0, 0).unwrap();
    assert_eq!(log.actions_made(), 3);
}
