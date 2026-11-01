//! The rollback harness (`spikes/harness.rs`): a restore and a re-simulation give the state a
//! straight run gives.

#[path = "../spikes/harness.rs"]
mod harness;

use harness::rollback_cycle;
use stella_rain_core::bench_fixtures::worst_minute_case::{
    OFFER_CHOICE, input_at, worst_minute_peak,
};
use stella_rain_core::engine::Engine;

fn step_minute(engine: &mut Engine) {
    while engine.offer_open() {
        engine.choose(OFFER_CHOICE).expect("the first option");
    }
    engine.step(input_at(engine.tick()));
}

#[test]
fn rollback_resim_matches_straight_run() {
    let mut run = worst_minute_peak();
    let cycle = rollback_cycle(&mut run.engine, &mut step_minute, 8);
    assert_ne!(cycle.straight, cycle.at_clone, "8 ticks moved the state");
    assert_eq!(cycle.resimulated, cycle.straight);
    assert_eq!(cycle.restored, cycle.at_clone);
}

#[test]
fn savepoint_round_trip_matches() {
    let (saved, restored, bytes) = harness::savepoint_round_trip(600);
    assert_eq!(restored, saved);
    assert!(bytes > 0, "the savepoint holds a payload");
}
