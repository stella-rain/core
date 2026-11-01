//! The live state's hash and savepoint bytes, pinned before the state holds its players as a
//! list (ADR-SIM-12): a refactor of the state moves neither at one player.

use crate::bench_fixtures::{hit_case, time_limit_case, worst_case};
use crate::engine::Engine;
use crate::hash::StateHasher;
use crate::input::Input;
use crate::savepoint::encode_state;

const TICKS: u32 = 600;

/// The hash and the savepoint bytes at tick 0 and every 25th tick of `TICKS` ticks, an open offer
/// taking option 0.
fn pin(mut e: Engine, input: fn(u32) -> Input) -> u64 {
    assert_eq!(e.run().version(), 2, "the pin is of the live rules");
    let mut h = StateHasher::new();
    let mut feed = |e: &Engine| {
        h.write_u64(e.state_hash());
        h.write_bytes(&encode_state(e));
    };
    feed(&e);
    for i in 1..=TICKS {
        while e.offer_open() {
            e.choose(0).expect("option 0 exists");
        }
        let tick = e.tick() + 1;
        e.step(input(tick));
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
            15_108_713_697_578_387_562,
            6_803_785_181_847_753_420,
            14_234_592_711_951_511_813,
        ],
        "worst_case, hit_case, time_limit_case; never regenerated"
    );
}
