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
    let (saved, restored, bytes) = harness::savepoint_round_trip(1500);
    assert_eq!(restored, saved);
    assert!(bytes > 0, "the savepoint holds a payload");
}

#[test]
fn the_csv_has_every_cell_and_no_max_below_its_p99() {
    let config = harness::Config {
        samples: 3,
        warm_up: 1,
    };
    let mut csv = Vec::new();
    harness::run(&mut csv, config, "test").expect("writes");
    let text = String::from_utf8(csv).expect("text");
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(harness::HEADER));
    let rows: Vec<Vec<&str>> = lines.map(|l| l.split(',').collect()).collect();
    assert_eq!(rows.len(), harness::EXPECTED_ROWS);
    for row in &rows {
        assert_eq!(row.len(), 10, "{row:?}");
        let n: Vec<u64> = row[5..]
            .iter()
            .map(|v| v.parse().expect("a number"))
            .collect();
        assert!(n[3] >= n[2] && n[2] >= n[1] && n[1] >= n[0], "{row:?}");
        assert_eq!(n[4], 3);
    }
    let cells: std::collections::BTreeSet<_> =
        rows.iter().map(|r| (r[1], r[2], r[3], r[4])).collect();
    assert_eq!(cells.len(), rows.len(), "no cell twice");
}
