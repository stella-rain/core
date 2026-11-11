//! The test-facing accessors that read the live version 2 state of a `Run` (the frozen tests read
//! version 1 through `Deref`; a live port reads through these).

use super::*;

/// A `time_limit` start at version 2 from the bundled files.
fn live_start() -> Engine {
    super::live_support::live_bundled_run("normal", 1)
}

#[test]
fn the_live_accessors_read_a_version_2_run() {
    let mut e = live_start();
    assert_eq!(e.run.live().tick, 0);
    assert!(!e.run.live_world().bosses.is_empty());
    let solo = e.run.solo();
    assert!(solo.hp > 0 && solo.hp == solo.max_hp);
    e.run.solo_mut().hp = 1;
    assert_eq!(e.run.solo().hp, 1);
    e.run.live_mut().tick = 5;
    let state = e.run.live().clone();
    e.run.set_live_state(state);
    assert_eq!(e.run.live().tick, 5);
    e.run.live_world_mut();
}

#[test]
#[should_panic(expected = "version 2")]
fn the_live_accessors_refuse_a_version_1_run() {
    let setup = RunSetup {
        schema_version: 1,
        sim_version: 1,
        ..RunSetup::default()
    };
    let e = Engine::start_with(&setup, super::copy_tests::v1_pack().content()).unwrap();
    e.run.live();
}

#[test]
#[should_panic(expected = "version 1")]
fn the_version_1_accessors_refuse_a_version_2_run() {
    let e = live_start();
    let _ = e.run.tick;
}
