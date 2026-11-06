//! Equip and replace in play, in the recorder and in a replay (ADR-SIM-7, ADR-REPL-5): a run
//! that has dropped items gets an action made between two ticks, the recorder applies and
//! records it, a replay applies it again and ends on the recorded hash, and a savepoint cut after
//! it resumes to the straight run's hashes.

use crate::common::run::trial_setup;
use crate::run_play::PartyCase;
use stella_rain_core::RangeStep;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::{RunRecorder, verify_run};
use stella_rain_core::run_setup::{BuildMode, RunSetup};

/// The slots of the layout a drop can go to: the five Attack, two Defence, two Mobility and the
/// three rune slots.
const SLOTS: u32 = 12;

fn case(seed: u64, ticks: u32) -> PartyCase {
    PartyCase {
        name: "equipment_equip",
        setup: RunSetup {
            mode: "tutorial".into(),
            seed,
            build_mode: BuildMode::Equipment,
            ..trial_setup()
        },
        party: 1,
        ticks,
        changes: vec![],
        answer_after: None,
        bot: None,
    }
}

fn inventory(e: &Engine) -> usize {
    e.snapshot().players[0].inventory.len()
}

/// The first action of `make` the engine accepts for slot 0: items and slots in order.
fn first(e: &Engine, make: fn(u32, u32) -> EquipAction) -> Option<EquipAction> {
    let items = u32::try_from(inventory(e)).unwrap();
    (0..items)
        .flat_map(|item| (0..SLOTS + 1).map(move |slot| make(item, slot)))
        .find(|a| e.check_action(PlayerIdx::SOLO, *a).is_ok())
}

fn equip(item: u32, slot: u32) -> EquipAction {
    EquipAction::Equip { item, slot }
}

fn replace(item: u32, slot: u32) -> EquipAction {
    EquipAction::Replace { item, slot }
}

fn acting(a: EquipAction) -> [Input; 3] {
    [
        Input {
            action: Some(a),
            ..Input::default()
        },
        Input::default(),
        Input::default(),
    ]
}

/// A recording after `ticks` ticks of a seed whose inventory lets an equip and then a replace
/// through, with the two actions.
fn recorded() -> (RunRecorder, EquipAction, EquipAction) {
    for seed in 1..=60u64 {
        let rec = case(seed, 3000).play();
        if rec.engine().outcome() != Outcome::Running {
            continue;
        }
        let Some(a) = first(rec.engine(), equip) else {
            continue;
        };
        let mut probe = rec.clone();
        probe.step(&acting(a)).unwrap();
        if let Some(b) = first(probe.engine(), replace) {
            return (rec, a, b);
        }
    }
    panic!("no seed of 1..=60 drops an item that equips and one that replaces it");
}

fn actions_in(rec: &RunRecorder) -> usize {
    let replay = rec.replay();
    let range = replay.step_range(0, replay.ticks, 1_000_000).unwrap();
    range
        .steps
        .iter()
        .filter(|s| matches!(s, RangeStep::Actions { .. }))
        .count()
}

/// T22: the recorder applies and records an accepted action; a refused one records nothing.
#[test]
fn the_recorder_applies_and_records_an_accepted_action() {
    let (mut rec, a, _) = recorded();
    let before = inventory(rec.engine());
    rec.step(&acting(a)).unwrap();
    assert_eq!(
        inventory(rec.engine()),
        before - 1,
        "the item left the inventory"
    );
    assert_eq!(actions_in(&rec), 1);
    // A refused action is ignored like a refused pick: the tick runs, nothing is recorded.
    let refused = acting(equip(9_999, 0));
    rec.step(&refused).unwrap();
    assert_eq!(actions_in(&rec), 1);
    assert_eq!(inventory(rec.engine()), before - 1);
}

/// T23: a replay applies the actions and ends on the recorded hash and the same inventory.
#[test]
fn a_replay_applies_actions_and_ends_on_the_recorded_hash() {
    let (mut rec, a, b) = recorded();
    rec.step(&acting(a)).unwrap();
    for _ in 0..60 {
        rec.step(&[Input::default(); 3]).unwrap();
    }
    assert_eq!(rec.engine().check_action(PlayerIdx::SOLO, b), Ok(()));
    rec.step(&acting(b)).unwrap();
    for _ in 0..60 {
        rec.step(&[Input::default(); 3]).unwrap();
    }
    assert_eq!(actions_in(&rec), 2);
    let replay = rec.replay();
    let verified = verify_run(&replay.to_bytes()).expect("the replay verifies");
    assert_eq!(verified.final_hash, rec.engine().state_hash());
}

/// T24: a run resumed from a savepoint after the equips matches the straight run's hashes.
#[test]
fn a_resumed_equipment_run_matches_the_straight_run() {
    let (mut live, a, b) = recorded();
    live.step(&acting(a)).unwrap();
    for _ in 0..60 {
        live.step(&[Input::default(); 3]).unwrap();
    }
    live.step(&acting(b)).unwrap();
    let prefix = live.replay();
    let saved = live.save();
    let pack = Pack::bundled();
    let mut by_savepoint = RunRecorder::resume_from_savepoint(&saved, &prefix, pack).unwrap();
    let mut by_replay = RunRecorder::resume_with_pack(&prefix, pack).unwrap();
    for t in 0..240 {
        assert_eq!(
            by_savepoint.engine().state_hash(),
            live.engine().state_hash(),
            "{t}"
        );
        assert_eq!(
            by_replay.engine().state_hash(),
            live.engine().state_hash(),
            "{t}"
        );
        for rec in [&mut live, &mut by_savepoint, &mut by_replay] {
            rec.step(&[Input::default(); 3]).unwrap();
        }
    }
    assert_eq!(by_savepoint.replay(), live.replay());
}
