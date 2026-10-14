//! The run replay (ADR-051 §3): a run recorded as its setup, the content hash, one input per
//! tick and one choice per offer, re-simulated from the file alone and refused, failing closed,
//! whenever the file and the run disagree (ADR-020).

use serde_json::{Value, json};
use stella_rain_core::base64;
use stella_rain_core::content_hash::to_hex;
use stella_rain_core::engine::{ChoiceError, Engine, Outcome};
use stella_rain_core::input::Input;
use stella_rain_core::replay::{MAX_REPLAY_BYTES, hash_to_hex, pack_inputs};
use stella_rain_core::run_replay::{
    Choice, Command, MAX_CHOICES, RunRecorder, RunReplay, RunReplayError, content_sha256,
    pack_choices, record_run, verify_run, verify_run_against,
};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::validate::Error;

mod common;

use common::run::{TRIAL_LENGTH, first_offer_tick, play_run, run_script, trial_setup};

fn recorded(ticks: u32) -> RunReplay {
    record_run(&trial_setup(), &play_run(ticks)).unwrap()
}

/// The replay's bytes with one field changed.
fn with(replay: &RunReplay, change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut v = serde_json::to_value(replay).unwrap();
    change(&mut v);
    v.to_string().into_bytes()
}

fn with_choices(replay: &RunReplay, choices: &[Choice]) -> Vec<u8> {
    with(replay, |v| v["choices"] = json!(pack_choices(choices)))
}

fn choices_of(replay: &RunReplay) -> Vec<Choice> {
    replay.unpack_choices().unwrap()
}

fn choice(tick: u32, index: u32) -> Choice {
    Choice { tick, index }
}

// --- Recording and verifying ------------------------------------------------------------------

#[test]
fn a_recorded_run_verifies_from_its_bytes_alone() {
    let offer = first_offer_tick();
    let replay = recorded(offer + 100);
    assert_eq!(replay.ticks, offer + 100);
    assert_eq!(choices_of(&replay), vec![choice(offer, 1)]);

    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.ticks, offer + 100);
    assert_eq!(verified.choices, 1);
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
    more.extend([Command::Step(run_script(0)), Command::Choose(0)]);
    assert_eq!(record_run(&trial_setup(), &more).unwrap(), replay);
}

#[test]
fn the_final_hash_is_the_one_after_the_last_choice() {
    let offer = first_offer_tick();
    let replay = recorded(offer);
    assert_eq!(choices_of(&replay), vec![choice(offer, 1)]);

    let mut engine = Engine::start(&trial_setup()).unwrap();
    for t in 0..offer {
        engine.step(run_script(t));
    }
    let before = engine.state_hash();
    engine.choose(1).unwrap();
    assert_ne!(engine.state_hash(), before, "a choice changes the state");
    assert_eq!(replay.final_hash, hash_to_hex(engine.state_hash()));
}

#[test]
fn a_run_stopped_at_an_open_offer_verifies() {
    let offer = first_offer_tick();
    let mut commands = play_run(offer);
    assert_eq!(commands.pop(), Some(Command::Choose(1)));
    let replay = record_run(&trial_setup(), &commands).unwrap();
    assert!(choices_of(&replay).is_empty());
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!((verified.ticks, verified.choices), (offer, 0));
}

#[test]
fn recording_fails_closed_on_a_step_while_an_offer_is_open() {
    let offer = first_offer_tick();
    let mut commands = play_run(offer + 5);
    let at = commands
        .iter()
        .position(|c| matches!(c, Command::Choose(_)))
        .unwrap();
    commands.remove(at);
    assert_eq!(
        record_run(&trial_setup(), &commands),
        Err(RunReplayError::MissingChoice { tick: offer })
    );
}

#[test]
fn recording_fails_closed_on_a_refused_choice() {
    let mut commands = play_run(10);
    commands.insert(3, Command::Choose(0));
    assert_eq!(
        record_run(&trial_setup(), &commands),
        Err(RunReplayError::ChoiceRefused {
            tick: 3,
            error: ChoiceError::NoOffer
        })
    );

    let offer = first_offer_tick();
    let mut commands = play_run(offer + 1);
    let at = commands
        .iter()
        .position(|c| matches!(c, Command::Choose(_)))
        .unwrap();
    commands[at] = Command::Choose(3);
    assert_eq!(
        record_run(&trial_setup(), &commands),
        Err(RunReplayError::ChoiceRefused {
            tick: offer,
            error: ChoiceError::OutOfRange {
                index: 3,
                options: 3
            }
        })
    );
}

#[test]
fn the_recorder_keeps_only_what_the_engine_took() {
    let offer = first_offer_tick();
    let mut recorder = RunRecorder::start(&trial_setup()).unwrap();
    for t in 0..offer {
        recorder.step(run_script(t)).unwrap();
    }
    assert!(recorder.engine().offer().is_some());
    let before = recorder.replay();

    assert_eq!(
        recorder.step(run_script(offer)),
        Err(RunReplayError::MissingChoice { tick: offer })
    );
    assert!(matches!(
        recorder.choose(7),
        Err(RunReplayError::ChoiceRefused { .. })
    ));
    assert_eq!(
        recorder.replay(),
        before,
        "refused commands are not recorded"
    );

    recorder.choose(1).unwrap();
    for t in offer..offer + 100 {
        recorder.step(run_script(t)).unwrap();
    }
    assert_eq!(recorder.replay(), recorded(offer + 100));
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
fn a_missing_choice_is_refused() {
    let offer = first_offer_tick();
    let replay = recorded(offer + 50);
    assert_eq!(
        verify_run(&with_choices(&replay, &[])),
        Err(RunReplayError::MissingChoice { tick: offer })
    );
}

#[test]
fn a_choice_for_another_tick_is_refused() {
    let offer = first_offer_tick();
    let replay = recorded(offer + 50);
    // A tick later: the offer's tick has no choice.
    assert_eq!(
        verify_run(&with_choices(&replay, &[choice(offer + 1, 1)])),
        Err(RunReplayError::MissingChoice { tick: offer })
    );
    // A tick earlier: no offer is open then.
    assert_eq!(
        verify_run(&with_choices(&replay, &[choice(offer - 1, 1)])),
        Err(RunReplayError::ChoiceRefused {
            tick: offer - 1,
            error: ChoiceError::NoOffer
        })
    );
    // Tick 0, before the first tick.
    assert_eq!(
        verify_run(&with_choices(&replay, &[choice(0, 0), choice(offer, 1)])),
        Err(RunReplayError::ChoiceRefused {
            tick: 0,
            error: ChoiceError::NoOffer
        })
    );
}

#[test]
fn a_choice_left_over_is_refused() {
    let offer = first_offer_tick();
    let replay = recorded(offer + 50);
    let mut choices = choices_of(&replay);
    choices.push(choice(offer + 50, 0));
    assert_eq!(
        verify_run(&with_choices(&replay, &choices)),
        Err(RunReplayError::ChoiceRefused {
            tick: offer + 50,
            error: ChoiceError::NoOffer
        })
    );
    // A second choice on the offer's tick, after the offer was answered.
    let doubled = [choice(offer, 1), choice(offer, 0)];
    assert_eq!(
        verify_run(&with_choices(&replay, &doubled)),
        Err(RunReplayError::ChoiceRefused {
            tick: offer,
            error: ChoiceError::NoOffer
        })
    );
}

#[test]
fn a_choice_after_the_last_tick_is_damage() {
    let replay = recorded(20);
    assert_eq!(
        verify_run(&with_choices(&replay, &[choice(21, 0)])),
        Err(RunReplayError::BadChoices(
            "a choice is after the last tick"
        ))
    );
}

#[test]
fn an_index_outside_its_offer_is_refused() {
    let offer = first_offer_tick();
    let replay = recorded(offer + 50);
    assert_eq!(
        verify_run(&with_choices(&replay, &[choice(offer, 3)])),
        Err(RunReplayError::ChoiceRefused {
            tick: offer,
            error: ChoiceError::OutOfRange {
                index: 3,
                options: 3
            }
        })
    );
    assert_eq!(
        verify_run(&with_choices(&replay, &[choice(offer, u32::MAX)])),
        Err(RunReplayError::ChoiceRefused {
            tick: offer,
            error: ChoiceError::OutOfRange {
                index: u32::MAX,
                options: 3
            }
        })
    );
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
    let replay = recorded(first_offer_tick() + 10);
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
        v.as_object_mut().unwrap().remove("choices");
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

// --- Packed choices ---------------------------------------------------------------------------

fn replay_with(choices: &str, ticks: u32) -> RunReplay {
    let mut replay = recorded(1);
    replay.ticks = ticks;
    replay.choices = choices.to_owned();
    replay
}

fn packed(bytes: &[u8]) -> String {
    base64::encode(bytes)
}

#[test]
fn choices_pack_as_tick_deltas_and_indices_in_leb128() {
    let choices = [choice(5, 0), choice(5, 2), choice(300, 1), choice(300, 200)];
    let bytes = base64::decode(&pack_choices(&choices)).unwrap();
    // 5, 0 | 0, 2 | 295 (0xa7 0x02), 1 | 0, 200 (0xc8 0x01)
    assert_eq!(
        bytes,
        [0x05, 0x00, 0x00, 0x02, 0xa7, 0x02, 0x01, 0x00, 0xc8, 0x01]
    );
    assert_eq!(pack_choices(&[]), "");
    let replay = replay_with(&pack_choices(&choices), 300);
    assert_eq!(replay.unpack_choices().unwrap(), choices);
}

#[test]
fn packed_choices_round_trip_at_the_extremes() {
    let choices = [
        choice(1, u32::MAX),
        choice(u32::MAX - 1, 0),
        choice(u32::MAX, 7),
    ];
    let replay = replay_with(&pack_choices(&choices), u32::MAX);
    assert_eq!(replay.unpack_choices().unwrap(), choices);
}

#[test]
fn damaged_choices_are_refused() {
    let cases: [(&[u8], &str); 5] = [
        (&[0x05], "ends in the middle of a choice"),
        (&[0x85], "ends in the middle of a number"),
        (&[0x85, 0x00, 0x01], "a number is not in its shortest form"),
        (
            &[0x05, 0xff, 0xff, 0xff, 0xff, 0x7f],
            "a number is too large",
        ),
        (&[0x06, 0x00], "a choice is after the last tick"),
    ];
    for (bytes, why) in cases {
        let replay = replay_with(&packed(bytes), 5);
        assert_eq!(
            replay.unpack_choices(),
            Err(RunReplayError::BadChoices(why)),
            "{bytes:02x?}"
        );
    }
    // A delta that would carry the tick past u32::MAX.
    let mut bytes = vec![0xff, 0xff, 0xff, 0xff, 0x0f, 0x00];
    bytes.extend([0x01, 0x00]);
    let replay = replay_with(&packed(&bytes), u32::MAX);
    assert_eq!(
        replay.unpack_choices(),
        Err(RunReplayError::BadChoices(
            "a choice is after the last tick"
        ))
    );
    let replay = replay_with("not base64!", 5);
    assert_eq!(
        replay.unpack_choices(),
        Err(RunReplayError::BadChoices("not valid base64"))
    );
}

#[test]
fn at_most_max_choices_unpack() {
    let at_limit: Vec<Choice> = (0..MAX_CHOICES).map(|_| choice(1, 0)).collect();
    let replay = replay_with(&pack_choices(&at_limit), 1);
    assert_eq!(replay.unpack_choices().unwrap().len(), MAX_CHOICES as usize);

    let past: Vec<Choice> = (0..=MAX_CHOICES).map(|_| choice(1, 0)).collect();
    let replay = replay_with(&pack_choices(&past), 1);
    assert_eq!(
        replay.unpack_choices(),
        Err(RunReplayError::TooManyChoices { limit: MAX_CHOICES })
    );
}
