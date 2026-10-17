//! The run replay (ADR-066 §5): a run recorded as its setup, the content hash, one input per
//! tick and one stream of commands between ticks (`choose` and `spend`), re-simulated from the
//! file alone and refused, failing closed, whenever the file and the run disagree (ADR-020).

use serde_json::{Value, json};
use stella_rain_core::base64;
use stella_rain_core::content_hash::to_hex;
use stella_rain_core::engine::{ChoiceError, Engine, Outcome, SpendError};
use stella_rain_core::input::Input;
use stella_rain_core::replay::{MAX_REPLAY_BYTES, hash_to_hex, pack_inputs};
use stella_rain_core::run_replay::{
    Command, CommandKind, MAX_COMMANDS, RunRecorder, RunReplay, RunReplayError, TickCommand,
    content_sha256, pack_commands, record_run, verify_run, verify_run_against,
};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::validate::Error;

mod common;

use common::run::{
    SPEND_INDEX, TRIAL_LENGTH, first_point_tick, play_run, run_input, run_script, trial_setup,
};

fn recorded(ticks: u32) -> RunReplay {
    record_run(&trial_setup(), &play_run(ticks)).unwrap()
}

/// The replay's bytes with one field changed.
fn with(replay: &RunReplay, change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut v = serde_json::to_value(replay).unwrap();
    change(&mut v);
    v.to_string().into_bytes()
}

fn with_commands(replay: &RunReplay, commands: &[TickCommand]) -> Vec<u8> {
    with(replay, |v| v["commands"] = json!(pack_commands(commands)))
}

fn commands_of(replay: &RunReplay) -> Vec<TickCommand> {
    replay.unpack_commands().unwrap()
}

fn spend(tick: u32, index: u32) -> TickCommand {
    TickCommand {
        tick,
        kind: CommandKind::Spend,
        index,
    }
}

fn choose(tick: u32, index: u32) -> TickCommand {
    TickCommand {
        tick,
        kind: CommandKind::Choose,
        index,
    }
}

// --- Recording and verifying ------------------------------------------------------------------

#[test]
fn a_recorded_run_that_spends_points_verifies_from_its_bytes_alone() {
    let point = first_point_tick();
    let replay = recorded(point + 100);
    assert_eq!(replay.ticks, point + 100);
    let commands = commands_of(&replay);
    assert_eq!(commands.first(), Some(&spend(point, SPEND_INDEX)));

    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.ticks, point + 100);
    assert_eq!(verified.commands, u32::try_from(commands.len()).unwrap());
    assert_eq!(verified.outcome, Outcome::Running);
    assert_eq!(hash_to_hex(verified.final_hash), replay.final_hash);
}

#[test]
fn the_replay_holds_the_setup_and_the_content_it_ran() {
    let replay = recorded(10);
    assert_eq!(replay.setup, trial_setup());
    assert_eq!(
        replay.content_sha256,
        to_hex(&content_sha256("trial").unwrap())
    );
}

#[test]
fn the_content_hash_is_the_same_every_time_and_only_modes_have_one() {
    assert_eq!(content_sha256("trial"), content_sha256("trial"));
    assert_eq!(content_sha256("no_such_mode"), None);
}

#[test]
fn a_run_that_ends_is_recorded_to_the_tick_it_ends() {
    let replay = recorded(TRIAL_LENGTH);
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.outcome, Outcome::Failed);
    assert!(replay.ticks < TRIAL_LENGTH, "the script's run ends early");
    assert_eq!(verified.ticks, replay.ticks);

    // Commands after the end are left out, as a stage replay leaves them out.
    let mut more = play_run(TRIAL_LENGTH);
    more.extend([Command::Step(run_script(0)), Command::Spend(0)]);
    assert_eq!(record_run(&trial_setup(), &more).unwrap(), replay);
}

#[test]
fn a_spend_after_the_run_ended_is_refused() {
    // The run ends with its points unspent; they are lost, so a spend after the end is refused
    // like a step, and a replay holding one does not verify (recording would leave it out).
    let mut commands: Vec<Command> = play_run(TRIAL_LENGTH)
        .into_iter()
        .filter(|c| !matches!(c, Command::Spend(_)))
        .collect();
    let mut engine = Engine::start(&trial_setup()).unwrap();
    for c in &commands {
        match *c {
            Command::Step(input) => engine.step(input),
            Command::Choose(index) => engine.choose(index).unwrap(),
            Command::Spend(_) => unreachable!("filtered out"),
        }
    }
    // Without the spends the run may outlast `play_run`'s commands: play it to its end,
    // taking candidate 0 of every offer.
    while engine.outcome() == Outcome::Running {
        if engine.offer_open() {
            engine.choose(0).unwrap();
            commands.push(Command::Choose(0));
            continue;
        }
        let input = run_input(&engine, engine.tick());
        engine.step(input);
        commands.push(Command::Step(input));
    }
    let replay = record_run(&trial_setup(), &commands).unwrap();
    assert_ne!(engine.outcome(), Outcome::Running);
    assert!(engine.snapshot().progress.unwrap().points > 0);
    let hash = engine.state_hash();
    assert_eq!(engine.spend(0), Err(SpendError::RunOver));
    assert_eq!(engine.state_hash(), hash);
    let mut late = commands_of(&replay);
    late.push(spend(replay.ticks, 0));
    assert_eq!(
        verify_run(&with_commands(&replay, &late)),
        Err(RunReplayError::SpendRefused {
            tick: replay.ticks,
            error: SpendError::RunOver
        })
    );
}

#[test]
fn the_final_hash_is_the_one_after_the_last_spend() {
    let point = first_point_tick();
    let replay = recorded(point);
    assert_eq!(commands_of(&replay), vec![spend(point, SPEND_INDEX)]);

    let mut engine = Engine::start(&trial_setup()).unwrap();
    for t in 0..point {
        engine.step(run_script(t));
    }
    let before = engine.state_hash();
    engine.spend(SPEND_INDEX).unwrap();
    assert_ne!(engine.state_hash(), before, "a spend changes the state");
    assert_eq!(replay.final_hash, hash_to_hex(engine.state_hash()));
}

#[test]
fn a_run_with_unspent_points_verifies() {
    let point = first_point_tick();
    let commands: Vec<Command> = play_run(point + 50)
        .into_iter()
        .filter(|c| !matches!(c, Command::Spend(_)))
        .collect();
    let replay = record_run(&trial_setup(), &commands).unwrap();
    assert!(commands_of(&replay).is_empty());
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!((verified.ticks, verified.commands), (point + 50, 0));
}

#[test]
fn recording_fails_closed_on_a_refused_command() {
    let mut commands = play_run(10);
    commands.insert(3, Command::Choose(0));
    assert_eq!(
        record_run(&trial_setup(), &commands),
        Err(RunReplayError::ChoiceRefused {
            tick: 3,
            error: ChoiceError::NoOffer
        })
    );

    let mut commands = play_run(10);
    commands.insert(3, Command::Spend(0));
    assert_eq!(
        record_run(&trial_setup(), &commands),
        Err(RunReplayError::SpendRefused {
            tick: 3,
            error: SpendError::NoPoint
        })
    );

    let point = first_point_tick();
    let mut commands = play_run(point + 1);
    let at = commands
        .iter()
        .position(|c| matches!(c, Command::Spend(_)))
        .unwrap();
    commands[at] = Command::Spend(99);
    let stats = u32::try_from(RunStats::bundled().stats().len()).unwrap();
    assert_eq!(
        record_run(&trial_setup(), &commands),
        Err(RunReplayError::SpendRefused {
            tick: point,
            error: SpendError::OutOfRange { index: 99, stats }
        })
    );
}

#[test]
fn the_recorder_keeps_only_what_the_engine_took() {
    let point = first_point_tick();
    let mut recorder = RunRecorder::start(&trial_setup()).unwrap();
    for t in 0..point {
        recorder.step(run_script(t)).unwrap();
    }
    let before = recorder.replay();

    assert!(matches!(
        recorder.spend(99),
        Err(RunReplayError::SpendRefused { .. })
    ));
    assert!(matches!(
        recorder.choose(0),
        Err(RunReplayError::ChoiceRefused { .. })
    ));
    assert_eq!(
        recorder.replay(),
        before,
        "refused commands are not recorded"
    );

    recorder.spend(SPEND_INDEX).unwrap();
    for t in point..point + 100 {
        let input = run_input(recorder.engine(), t);
        recorder.step(input).unwrap();
        while recorder
            .engine()
            .snapshot()
            .progress
            .is_some_and(|p| p.points > 0)
        {
            recorder.spend(SPEND_INDEX).unwrap();
        }
    }
    assert_eq!(recorder.replay(), recorded(point + 100));
}

// --- Offers (ADR-066 §3) -------------------------------------------------------------------------

/// The commands of the trial run up to its first offer: the input of the tick that opened it
/// is the last.
fn to_first_offer() -> Vec<Command> {
    let all = play_run(TRIAL_LENGTH);
    let first = all
        .iter()
        .position(|c| matches!(c, Command::Choose(_)))
        .expect("the trial run opens an offer");
    all[..first].to_vec()
}

#[test]
fn a_run_that_answers_offers_verifies_and_uses_every_kind_of_option() {
    let commands = play_run(TRIAL_LENGTH);
    let choices = commands
        .iter()
        .filter(|c| matches!(c, Command::Choose(_)))
        .count();
    assert!(choices >= 4, "{choices} choices");
    let replay = record_run(&trial_setup(), &commands).unwrap();
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.final_hash, replay_hash(&replay));
}

fn replay_hash(replay: &RunReplay) -> u64 {
    stella_rain_core::replay::hash_from_hex(&replay.final_hash).unwrap()
}

#[test]
fn the_recorder_refuses_an_input_while_an_offer_is_open() {
    let mut recorder = RunRecorder::start(&trial_setup()).unwrap();
    for c in to_first_offer() {
        match c {
            Command::Step(input) => recorder.step(input).unwrap(),
            Command::Choose(index) => recorder.choose(index).unwrap(),
            Command::Spend(index) => recorder.spend(index).unwrap(),
        }
    }
    assert!(recorder.engine().offer_open());
    let before = recorder.replay();
    let tick = recorder.engine().tick();
    assert_eq!(
        recorder.step(run_script(0)),
        Err(RunReplayError::OfferOpen { tick })
    );
    assert_eq!(recorder.replay(), before, "the input is not recorded");
    assert!(matches!(
        recorder.spend(SPEND_INDEX),
        Err(RunReplayError::SpendRefused {
            error: SpendError::OfferOpen,
            ..
        })
    ));
    recorder.choose(0).unwrap();
    recorder.step(run_script(0)).unwrap();
}

#[test]
fn a_replay_with_an_unanswered_offer_is_refused() {
    let replay = recorded(TRIAL_LENGTH);
    let commands = commands_of(&replay);
    let first = commands
        .iter()
        .position(|c| c.kind == CommandKind::Choose)
        .unwrap();
    let tick = commands[first].tick;
    // Without the offer's choices, the next input comes while it is open.
    let unanswered: Vec<TickCommand> = commands
        .iter()
        .filter(|c| !(c.kind == CommandKind::Choose && c.tick == tick))
        .copied()
        .collect();
    assert_eq!(
        verify_run(&with_commands(&replay, &unanswered)),
        Err(RunReplayError::OfferOpen { tick })
    );
}

#[test]
fn a_replay_that_ends_on_an_open_offer_verifies() {
    // A bug report may stop while the player looks at an offer.
    let replay = record_run(&trial_setup(), &to_first_offer()).unwrap();
    assert!(verify_run(&replay.to_bytes()).is_ok());
}

#[test]
fn a_bad_setup_does_not_start_a_recording() {
    let bad = RunSetup {
        main: "no_such_weapon".into(),
        ..trial_setup()
    };
    assert!(matches!(
        record_run(&bad, &[]),
        Err(RunReplayError::Setup(Error::Invalid(_)))
    ));
}

// --- Verification fails closed ----------------------------------------------------------------

#[test]
fn a_spend_before_its_point_is_refused() {
    let point = first_point_tick();
    let replay = recorded(point + 50);
    assert_eq!(
        verify_run(&with_commands(&replay, &[spend(point - 1, SPEND_INDEX)])),
        Err(RunReplayError::SpendRefused {
            tick: point - 1,
            error: SpendError::NoPoint
        })
    );
    assert_eq!(
        verify_run(&with_commands(&replay, &[spend(0, SPEND_INDEX)])),
        Err(RunReplayError::SpendRefused {
            tick: 0,
            error: SpendError::NoPoint
        })
    );
}

#[test]
fn a_command_left_over_is_refused() {
    let point = first_point_tick();
    let replay = recorded(point + 50);
    let mut commands = commands_of(&replay);
    commands.push(spend(point + 50, 0));
    assert_eq!(
        verify_run(&with_commands(&replay, &commands)),
        Err(RunReplayError::SpendRefused {
            tick: point + 50,
            error: SpendError::NoPoint
        })
    );
}

#[test]
fn a_choice_is_refused_while_no_offer_is_open() {
    let replay = recorded(20);
    assert_eq!(
        verify_run(&with_commands(&replay, &[choose(5, 0)])),
        Err(RunReplayError::ChoiceRefused {
            tick: 5,
            error: ChoiceError::NoOffer
        })
    );
}

#[test]
fn a_command_after_the_last_tick_is_damage() {
    let replay = recorded(20);
    assert_eq!(
        verify_run(&with_commands(&replay, &[spend(21, 0)])),
        Err(RunReplayError::BadCommands(
            "a command is after the last tick"
        ))
    );
}

#[test]
fn an_index_outside_the_stats_is_refused() {
    let point = first_point_tick();
    let replay = recorded(point + 50);
    let stats = u32::try_from(RunStats::bundled().stats().len()).unwrap();
    for index in [stats, u32::MAX] {
        assert_eq!(
            verify_run(&with_commands(&replay, &[spend(point, index)])),
            Err(RunReplayError::SpendRefused {
                tick: point,
                error: SpendError::OutOfRange { index, stats }
            })
        );
    }
}

#[test]
fn inputs_after_the_run_ended_are_refused() {
    let replay = recorded(TRIAL_LENGTH);
    let ended = replay.ticks;
    let mut inputs = replay.unpack_inputs(TRIAL_LENGTH).unwrap();
    inputs.push(Input::default());
    let bytes = with(&replay, |v| {
        v["ticks"] = json!(ended + 1);
        v["inputs"] = json!(pack_inputs(&inputs));
    });
    assert_eq!(
        verify_run(&bytes),
        Err(RunReplayError::InputsAfterEnd { ended_at: ended })
    );
}

#[test]
fn a_changed_content_hash_is_refused() {
    let replay = recorded(20);
    let bytes = with(&replay, |v| v["content_sha256"] = json!("00".repeat(32)));
    assert_eq!(verify_run(&bytes), Err(RunReplayError::ContentChanged));
}

#[test]
fn a_wrong_final_hash_is_refused() {
    let replay = recorded(20);
    let actual = replay.final_hash.clone();
    let bytes = with(&replay, |v| v["final_hash"] = json!("0123456789abcdef"));
    assert_eq!(
        verify_run(&bytes),
        Err(RunReplayError::WrongFinalHash {
            expected: 0x0123_4567_89ab_cdef,
            actual: u64::from_str_radix(&actual, 16).unwrap()
        })
    );
    for bad in ["0123456789ABCDEF", "123", ""] {
        let bytes = with(&replay, |v| v["final_hash"] = json!(bad));
        assert_eq!(verify_run(&bytes), Err(RunReplayError::BadHash), "{bad}");
    }
}

#[test]
fn a_bad_or_newer_setup_is_refused() {
    let replay = recorded(20);
    let bytes = with(&replay, |v| v["setup"]["sub"] = json!("no_such_weapon"));
    assert!(matches!(
        verify_run(&bytes),
        Err(RunReplayError::Setup(Error::Invalid(_)))
    ));
    let bytes = with(&replay, |v| v["setup"]["sim_version"] = json!(1));
    assert!(matches!(
        verify_run(&bytes),
        Err(RunReplayError::Setup(Error::UnsupportedVersion { .. }))
    ));
}

#[test]
fn a_changed_input_or_seed_reaches_another_final_hash() {
    let replay = recorded(200);
    let bytes = with(&replay, |v| v["setup"]["seed"] = json!(1));
    assert!(matches!(
        verify_run(&bytes),
        Err(RunReplayError::WrongFinalHash { .. })
    ));
    let mut inputs = replay.unpack_inputs(TRIAL_LENGTH).unwrap();
    inputs[100].move_y = 127;
    let bytes = with(&replay, |v| v["inputs"] = json!(pack_inputs(&inputs)));
    assert!(matches!(
        verify_run(&bytes),
        Err(RunReplayError::WrongFinalHash { .. })
    ));
}

#[test]
fn a_changed_spend_reaches_another_final_hash() {
    let point = first_point_tick();
    let replay = recorded(point + 100);
    let mut commands = commands_of(&replay);
    commands[0].index = 0;
    assert!(matches!(
        verify_run(&with_commands(&replay, &commands)),
        Err(RunReplayError::WrongFinalHash { .. })
    ));
}

#[test]
fn the_mode_s_length_bounds_the_ticks() {
    let replay = recorded(20);
    let at_limit = with(&replay, |v| {
        v["ticks"] = json!(TRIAL_LENGTH);
        v["inputs"] = json!(pack_inputs(&vec![Input::default(); TRIAL_LENGTH as usize]));
    });
    let at_limit = RunReplay::from_bytes(&at_limit).unwrap();
    assert_eq!(
        at_limit.unpack_inputs(TRIAL_LENGTH).unwrap().len(),
        TRIAL_LENGTH as usize
    );

    let past = with(&replay, |v| {
        v["ticks"] = json!(TRIAL_LENGTH + 1);
        v["inputs"] = json!(pack_inputs(&vec![
            Input::default();
            TRIAL_LENGTH as usize + 1
        ]));
    });
    assert_eq!(
        verify_run(&past),
        Err(RunReplayError::TooManyTicks {
            ticks: TRIAL_LENGTH + 1,
            limit: TRIAL_LENGTH
        })
    );
}

#[test]
fn per_tick_hashes_name_the_first_tick_that_differs() {
    let replay = recorded(100);
    let bytes = replay.to_bytes();
    let mut engine = Engine::start(&trial_setup()).unwrap();
    let mut hashes: Vec<u64> = (0..100)
        .map(|t| {
            engine.step(run_script(t));
            engine.state_hash()
        })
        .collect();
    assert!(verify_run_against(&bytes, &hashes).is_ok());

    hashes[41] ^= 1;
    assert_eq!(
        verify_run_against(&bytes, &hashes),
        Err(RunReplayError::Diverged { tick: 42 })
    );
    assert_eq!(
        verify_run_against(&bytes, &hashes[..99]),
        Err(RunReplayError::HashCount {
            expected: 100,
            given: 99
        })
    );
}

// --- The file ---------------------------------------------------------------------------------

#[test]
fn a_replay_file_reads_back_as_the_replay() {
    let replay = recorded(first_point_tick() + 10);
    let bytes = replay.to_bytes();
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert_eq!(RunReplay::from_bytes(&bytes).unwrap(), replay);
}

#[test]
fn a_file_over_the_limit_is_not_parsed() {
    let big = vec![b' '; MAX_REPLAY_BYTES + 1];
    assert_eq!(
        RunReplay::from_bytes(&big),
        Err(RunReplayError::TooLarge {
            size: MAX_REPLAY_BYTES + 1,
            limit: MAX_REPLAY_BYTES
        })
    );
    // At the limit it is parsed (and is not a replay).
    let mut padded = recorded(20).to_bytes();
    padded.resize(MAX_REPLAY_BYTES, b' ');
    assert!(RunReplay::from_bytes(&padded).is_ok());
}

#[test]
fn only_the_run_replay_format_parses() {
    let replay = recorded(20);
    let unknown = with(&replay, |v| v["extra"] = json!(1));
    assert!(matches!(
        RunReplay::from_bytes(&unknown),
        Err(RunReplayError::Parse { .. })
    ));
    let missing = with(&replay, |v| {
        v.as_object_mut().unwrap().remove("commands");
    });
    assert!(matches!(
        RunReplay::from_bytes(&missing),
        Err(RunReplayError::Parse { .. })
    ));
    let unknown_in_setup = with(&replay, |v| v["setup"]["extra"] = json!(1));
    assert!(matches!(
        RunReplay::from_bytes(&unknown_in_setup),
        Err(RunReplayError::Parse { .. })
    ));
}

// --- Packed commands --------------------------------------------------------------------------

fn replay_with(commands: &str, ticks: u32) -> RunReplay {
    let mut replay = recorded(1);
    replay.ticks = ticks;
    replay.commands = commands.to_owned();
    replay
}

fn packed(bytes: &[u8]) -> String {
    base64::encode(bytes)
}

#[test]
fn commands_pack_as_tick_deltas_kinds_and_indices() {
    let commands = [spend(5, 0), spend(5, 2), choose(300, 1), spend(300, 200)];
    let bytes = base64::decode(&pack_commands(&commands)).unwrap();
    // 5, spend, 0 | 0, spend, 2 | 295 (0xa7 0x02), choose, 1 | 0, spend, 200 (0xc8 0x01)
    assert_eq!(
        bytes,
        [
            0x05, 0x01, 0x00, 0x00, 0x01, 0x02, 0xa7, 0x02, 0x00, 0x01, 0x00, 0x01, 0xc8, 0x01
        ]
    );
    assert_eq!(pack_commands(&[]), "");
    let replay = replay_with(&pack_commands(&commands), 300);
    assert_eq!(replay.unpack_commands().unwrap(), commands);
}

#[test]
fn packed_commands_round_trip_at_the_extremes() {
    let commands = [
        spend(1, u32::MAX),
        choose(u32::MAX - 1, 0),
        spend(u32::MAX, 7),
    ];
    let replay = replay_with(&pack_commands(&commands), u32::MAX);
    assert_eq!(replay.unpack_commands().unwrap(), commands);
}

#[test]
fn damaged_commands_are_refused() {
    let cases: [(&[u8], &str); 7] = [
        (&[0x05], "ends in the middle of a command"),
        (&[0x05, 0x01], "ends in the middle of a command"),
        (&[0x85], "ends in the middle of a number"),
        (
            &[0x85, 0x00, 0x01, 0x00],
            "a number is not in its shortest form",
        ),
        (
            &[0x05, 0x01, 0xff, 0xff, 0xff, 0xff, 0x7f],
            "a number is too large",
        ),
        (&[0x05, 0x02, 0x00], "an unknown command kind"),
        (&[0x06, 0x01, 0x00], "a command is after the last tick"),
    ];
    for (bytes, why) in cases {
        let replay = replay_with(&packed(bytes), 5);
        assert_eq!(
            replay.unpack_commands(),
            Err(RunReplayError::BadCommands(why)),
            "{bytes:02x?}"
        );
    }
    // A delta that would carry the tick past u32::MAX.
    let mut bytes = vec![0xff, 0xff, 0xff, 0xff, 0x0f, 0x01, 0x00];
    bytes.extend([0x01, 0x01, 0x00]);
    let replay = replay_with(&packed(&bytes), u32::MAX);
    assert_eq!(
        replay.unpack_commands(),
        Err(RunReplayError::BadCommands(
            "a command is after the last tick"
        ))
    );
    let replay = replay_with("not base64!", 5);
    assert_eq!(
        replay.unpack_commands(),
        Err(RunReplayError::BadCommands("not valid base64"))
    );
}

#[test]
fn at_most_max_commands_unpack() {
    let at_limit: Vec<TickCommand> = (0..MAX_COMMANDS).map(|_| spend(1, 0)).collect();
    let replay = replay_with(&pack_commands(&at_limit), 1);
    assert_eq!(
        replay.unpack_commands().unwrap().len(),
        MAX_COMMANDS as usize
    );

    let past: Vec<TickCommand> = (0..=MAX_COMMANDS).map(|_| spend(1, 0)).collect();
    let replay = replay_with(&pack_commands(&past), 1);
    assert_eq!(
        replay.unpack_commands(),
        Err(RunReplayError::TooManyCommands {
            limit: MAX_COMMANDS
        })
    );
}

#[test]
fn the_fuzz_regression_of_a_split_input_run_is_refused() {
    // `fuzz/regressions/replay_load/`: the trial corpus run with its first run of inputs split
    // in two. It verified before inputs had one encoding, and recorded as another replay.
    let bytes = include_bytes!("../fuzz/regressions/replay_load/split-input-run.replay");
    assert!(RunReplay::from_bytes(bytes).is_ok(), "it is a run replay");
    assert_eq!(
        verify_run(bytes),
        Err(RunReplayError::BadInputs(
            "a run repeats the input of the run before"
        ))
    );
}
