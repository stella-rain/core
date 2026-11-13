//! The live state's hash and savepoint bytes, pinned. They were pinned before the state held its
//! players as a list (ADR-SIM-7) and re-pinned once, when the hash and the savepoint moved to a
//! presence byte and one block per player (ADR-SIM-3, ADR-SAVE-4), and again when three
//! generation bytes followed the presence byte: a refactor of the state moves neither at one
//! player.

use crate::bench_fixtures::{hit_case, time_limit_case, worst_case};
use crate::engine::Engine;
use crate::hash::StateHasher;
use crate::input::Input;
use crate::savepoint::encode_state;

const TICKS: u32 = 600;

/// The hash and the savepoint bytes at tick 0 and every 25th tick of `TICKS` ticks.
fn pin(mut e: Engine, input: fn(u32) -> Input) -> u64 {
    assert_eq!(e.run().version(), 2, "the pin is of the live rules");
    let mut h = StateHasher::new();
    let mut feed = |e: &Engine| {
        h.write_u64(e.state_hash());
        h.write_bytes(&encode_state(e));
    };
    feed(&e);
    for i in 1..=TICKS {
        let tick = e.tick() + 1;
        e.step(&[input(tick), Default::default(), Default::default()]);
        if i % 25 == 0 {
            feed(&e);
        }
    }
    h.finish()
}

#[test]
fn solo_state_hash_and_savepoint_bytes_are_pinned() {
    let got = [
        pin(worst_case::warmed_up(), worst_case::input),
        pin(hit_case::warmed_up(), hit_case::input),
        pin(time_limit_case::warmed_up(), time_limit_case::input),
    ];
    assert_eq!(
        got,
        [
            832_365_378_412_316_342,
            13_856_148_155_711_942_191,
            275_873_420_812_326_658,
        ],
        "worst_case, hit_case, time_limit_case; re-pinned for down/out player state fields, then for the equipment's 17 bytes, then for the 5 longer stat totals, then for the 13 totals and the 13 weapon-set tenths (worst_case and time_limit_case have levels, hit_case has none), then for the six new rewards that lengthen `taken` and `banished`, then for the three cases starting in equipment mode, then for the removal of the level-offer fields (worst_case and time_limit_case move; hit_case has no levels and no gauge and does not); then for the bundled time_limit mode's chasing bosses (time_limit_case moves); then for the removal of the star-offer fields (worst_case and time_limit_case have a `stars` section and move; hit_case has none and does not); then for the combat fields (every case has agents and players, so all three move, hit_case included); then for the counter-bonus mark (every case has a player, so all three move); then for the worst-case fixture's companions standing 21 px lower (worst_case only)"
    );
}
