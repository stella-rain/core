//! The equipment window of a party in a recorded run (ADR-SIM-7, ADR-REPL-5, ADR-SAVE-4): the
//! savepoint of a party with a downed member resumes to the live run's hashes, and so does the
//! one cut after a leave (a hole in the mask).

use crate::common::run::trial_setup;
use stella_rain_core::engine::Outcome;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::RunRecorder;
use stella_rain_core::run_setup::{BuildMode, RunSetup};

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn pack() -> &'static Pack {
    Pack::bundled()
}

fn setup() -> RunSetup {
    RunSetup {
        build_mode: BuildMode::Equipment,
        ..trial_setup()
    }
}

/// One tick: slot 0 walks right, the rest stand; an open offer is answered first.
fn tick(rec: &mut RunRecorder, party: u8) {
    while rec.engine().offer_open() {
        let holder = (0..party)
            .find(|i| rec.engine().check_pick(slot(*i), 0).is_ok())
            .expect("an open offer belongs to a slot");
        rec.choose(slot(holder), 0).unwrap();
    }
    let walk = Input {
        move_x: 100,
        ..Input::default()
    };
    rec.step(&[walk, Input::default(), Input::default()])
        .unwrap();
}

/// A recorder of `party` that has played until slot 1 is downed.
fn until_downed(party: u8) -> RunRecorder {
    let mut rec = RunRecorder::start_party(&setup(), party).unwrap();
    while rec.engine().snapshot().players[1].down.is_none() {
        assert!(rec.engine().tick() < 4_000, "slot 1 goes down");
        tick(&mut rec, party);
    }
    assert_eq!(rec.engine().outcome(), Outcome::Running);
    rec
}

/// L9: a party with a downed member resumes from its savepoint to the same hashes, regeneration
/// and rolls included (they are in the hash).
#[test]
fn a_downed_party_resumes_with_its_rolls() {
    let live = until_downed(2);
    let (prefix, bytes) = (live.replay(), live.save());
    let mut resumed = RunRecorder::resume_from_savepoint(&bytes, &prefix, pack()).unwrap();
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
    let mut live = live;
    for t in 0..180 {
        tick(&mut live, 2);
        tick(&mut resumed, 2);
        if t % 60 == 0 {
            assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        }
    }
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
    assert_eq!(resumed.replay(), live.replay());
}

/// L9: and the savepoint cut after a leave (mask `0b101`) restores the hole with no equipment.
#[test]
fn a_savepoint_after_a_leave_resumes_with_the_hole() {
    let mut live = RunRecorder::start_party(&setup(), 3).unwrap();
    for _ in 0..300 {
        tick(&mut live, 3);
    }
    live.leave(slot(1)).unwrap();
    for _ in 0..60 {
        tick(&mut live, 3);
    }
    assert_eq!(live.engine().present(), 0b101);
    let (prefix, bytes) = (live.replay(), live.save());
    let mut resumed = RunRecorder::resume_from_savepoint(&bytes, &prefix, pack()).unwrap();
    assert_eq!(resumed.engine().present(), 0b101);
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
    for _ in 0..120 {
        tick(&mut live, 3);
        tick(&mut resumed, 3);
    }
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
}
