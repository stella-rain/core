//! `common::run_steps`: the one replayer for a run case, offers-era or equipment (ADR-SIM-7,
//! ADR-REPL-5). The solo views refuse a log with an action; these tests pin what replaces them.

use crate::common::run_steps::{self, RunSteps, Step};
use crate::common::schema_1_form;
use crate::corpus::{current_dir, parties_dir, runs_dir};
use crate::corpus_pins::{case_dirs, run_case_pack};
use crate::party_bot::RigRun;
use crate::run_play::equipment_rig;
use std::path::{Path, PathBuf};
use stella_rain_core::engine::Engine;
use stella_rain_core::pack::Pack;
use stella_rain_core::replay::{hash_from_hex, hash_to_hex};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{Command, RunReplay, RunReplayError, record_run_with};

/// The building equipment rig of one player: a run with equipment actions and no pick.
fn rig_run() -> RunReplay {
    equipment_rig(1).play().replay()
}

fn frozen_runs_dir() -> PathBuf {
    current_dir().parent().unwrap().join("v1/runs")
}

fn load(case: &Path) -> (RunReplay, Vec<u8>, Pack) {
    let bytes = std::fs::read(case.join("run.replay")).unwrap();
    let replay = RunReplay::from_bytes(&bytes).unwrap();
    let pack = run_case_pack(case, &bytes).unwrap();
    (replay, bytes, pack)
}

/// Replays `replay` step by step, checks every 60th tick against its checkpoints and then the
/// final hash. Returns the engine after the last step.
fn replay_to_checkpoints(replay: &RunReplay, content: &RunContent) -> Engine {
    let steps = RunSteps::of(replay);
    let mut engine = run_steps::start(replay, content);
    for step in steps.steps() {
        run_steps::apply(&mut engine, step);
        if matches!(step, Step::Tick(_)) && engine.tick() % 60 == 0 {
            let at = 16 * (engine.tick() as usize / 60 - 1);
            assert_eq!(
                Some(engine.state_hash()),
                hash_from_hex(&replay.checkpoints[at..at + 16]),
                "checkpoint at tick {}",
                engine.tick()
            );
        }
    }
    assert_eq!(engine.tick(), replay.ticks);
    assert_eq!(hash_to_hex(engine.state_hash()), replay.final_hash);
    engine
}

fn count(steps: &RunSteps, f: impl Fn(&Step) -> bool) -> usize {
    steps.steps().iter().filter(|s| f(s)).count()
}

#[test]
fn an_equipment_run_replays_to_its_recorded_hashes() {
    let replay = rig_run();
    let steps = RunSteps::of(&replay);
    let total: u32 = RigRun::of(&replay)
        .actions_per_slot()
        .iter()
        .map(|c| c.equip + c.replace + c.extract + c.craft + c.tree_pick)
        .sum();
    assert_eq!(total, 12 + 1 + 47 + 3 + 1, "the pin of equipment_rig_1");
    assert!(steps.has_actions());
    assert_eq!(
        count(&steps, |s| matches!(s, Step::Tick(_))),
        replay.ticks as usize
    );
    assert_eq!(
        count(&steps, |s| matches!(s, Step::Actions(_))),
        total as usize
    );
    assert_eq!(count(&steps, |s| matches!(s, Step::Picks(_))), 0);
    let content = RunContent::bundled();
    let engine = replay_to_checkpoints(&replay, content);

    // The control: without its actions the run ends elsewhere.
    let mut bare = run_steps::start(&replay, content);
    for step in steps
        .steps()
        .iter()
        .filter(|s| !matches!(s, Step::Actions(_)))
    {
        run_steps::apply(&mut bare, step);
    }
    assert_ne!(bare.state_hash(), engine.state_hash());

    // The refusal this replayer works around.
    let refused = RunReplayError::BadInputs("a log with equipment actions has no solo view");
    assert_eq!(replay.unpack_inputs(u32::MAX).unwrap_err(), refused);
    assert_eq!(replay.unpack_commands().unwrap_err(), refused);
}

#[test]
fn every_run_case_replays_to_its_checkpoints() {
    let mut cases = case_dirs(&runs_dir());
    cases.extend(case_dirs(&frozen_runs_dir()));
    assert_eq!(cases.len(), 20);
    for case in cases {
        let (replay, _, pack) = load(&case);
        let steps = RunSteps::of(&replay);
        let name = case.display().to_string();
        assert_eq!(steps.party, 1, "{name}");
        // Every current case is an equipment run (only `tutorial_opening`, shorter than the bot's
        // first decision, holds no action); the frozen offers-era runs hold none.
        let frozen = case.starts_with(frozen_runs_dir());
        let acts = count(&steps, |s| matches!(s, Step::Actions(_)));
        assert_eq!(
            acts > 0,
            !frozen && case.file_name().is_some_and(|n| n != "tutorial_opening"),
            "{name}: {acts} ACTIONS steps"
        );
        assert_eq!(steps.has_actions(), acts > 0, "{name}");
        assert_eq!(
            count(&steps, |s| matches!(s, Step::Tick(_))),
            replay.ticks as usize,
            "{name}"
        );
        // An offers run holds one PICKS step per command; an equipment run opens no offer.
        let picks = count(&steps, |s| matches!(s, Step::Picks(_)));
        if frozen {
            assert_eq!(picks, replay.unpack_commands().unwrap().len(), "{name}");
        } else {
            assert_eq!(picks, 0, "{name}: an equipment run holds no PICKS step");
        }
        // No corpus run ends with a trailing pick: a reader may apply every step.
        assert_eq!(steps.at(replay.ticks), steps.steps().len(), "{name}");
        // A frozen run holds PICKS steps the replayer cannot play; those runs verify through
        // `verify_run*`.
        if !frozen {
            replay_to_checkpoints(&replay, pack.content());
        }
    }
}

#[test]
fn a_party_run_replays_the_same_way() {
    for (name, party) in [("three_time_limit", 3), ("leave_hole", 3)] {
        let (replay, _, pack) = load(&parties_dir().join(name));
        let steps = RunSteps::of(&replay);
        assert_eq!(steps.party, party, "{name}");
        replay_to_checkpoints(&replay, pack.content());
    }
    let (replay, _, _) = load(&parties_dir().join("leave_hole"));
    let steps = RunSteps::of(&replay);
    assert!(count(&steps, |s| matches!(s, Step::Leave(_))) > 0);
    assert!(count(&steps, |s| matches!(s, Step::Join(_))) > 0);
}

fn commands_record_the_file(replay: &RunReplay, content: &RunContent, name: &str) {
    let commands = run_steps::commands(replay);
    let again = record_run_with(&replay.setup, content, &commands).unwrap();
    let want = if replay.setup.schema_version < 2 {
        schema_1_form(&again)
    } else {
        again
    };
    assert_eq!(want.to_bytes(), replay.to_bytes(), "{name}");
}

#[test]
fn commands_are_the_inverse_of_record_run() {
    let mut cases = case_dirs(&runs_dir());
    cases.extend(case_dirs(&frozen_runs_dir()));
    for case in cases {
        let (replay, _, pack) = load(&case);
        commands_record_the_file(&replay, pack.content(), &case.display().to_string());
    }
    let replay = rig_run();
    commands_record_the_file(&replay, RunContent::bundled(), "rig_run");
    let actions = run_steps::commands(&replay)
        .iter()
        .filter(|c| matches!(c, Command::Step(i) if i.action.is_some()))
        .count();
    assert_eq!(actions, 12 + 1 + 47 + 3 + 1);
}

#[test]
#[should_panic(expected = "does not directly precede its tick")]
fn commands_refuses_what_a_step_cannot_hold() {
    let action = RunSteps::of(&rig_run())
        .steps()
        .iter()
        .find_map(|s| match s {
            Step::Actions(a) => a[0],
            _ => None,
        })
        .expect("an action");
    let actions = Step::Actions([Some(action), None, None]);
    // Two ACTIONS steps in a row: the recorder cannot write that from one `Command`.
    run_steps::commands_of(1, &[actions, actions, Step::Tick(Default::default())]);
}

#[test]
#[should_panic(expected = "no schema 1 form")]
fn schema_1_form_refuses_an_equipment_run() {
    schema_1_form(&rig_run());
}

#[test]
fn at_finds_the_first_step_of_a_tick() {
    // The frozen offers-era tutorial: its first PICKS step is the first step of tick 230,
    // so every step before it applies to a bare engine.
    let (replay, _, pack) = load(&frozen_runs_dir().join("tutorial"));
    let steps = RunSteps::of(&replay);
    let at = steps.at(230);
    assert!(matches!(steps.steps()[at], Step::Picks(_)));
    assert!(matches!(steps.steps()[steps.at(231)], Step::Tick(_)));
    let mut engine = run_steps::start(&replay, pack.content());
    for step in &steps.steps()[..at] {
        run_steps::apply(&mut engine, step);
    }
    assert_eq!(engine.tick(), 230);
    assert!(engine.offer_open());

    let replay = rig_run();
    let steps = RunSteps::of(&replay);
    let first = steps
        .steps()
        .iter()
        .position(|s| matches!(s, Step::Actions(_)))
        .unwrap();
    let ticks_before = steps.steps()[..first]
        .iter()
        .filter(|s| matches!(s, Step::Tick(_)))
        .count();
    assert_eq!(steps.at(u32::try_from(ticks_before).unwrap()), first);
}
