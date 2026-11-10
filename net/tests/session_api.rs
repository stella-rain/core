//! A compile and behaviour pin of what the rollback bench fixture calls: the struct literals of
//! `Config` and `InputFrame` (no `..`), `Session::new`, `advance`, `receive_inputs`, `ack`,
//! `engine`, `CloneRing::new` and `Save::save`. A field or a changed signature breaks it here.

mod support;

use stella_rain_net::save::{CloneRing, Save};
use stella_rain_net::session::{Config, Session};
use stella_rain_net::wire::{InputFrame, WireInput};
use support::{engine, script, slot};

#[test]
fn the_bench_fixtures_literals_and_calls_still_compile_and_play() {
    let sc = script(2, 40, 0, None);
    let mut session = Session::new(
        engine(2),
        Config {
            local: slot(0),
            delay: 0,
            window: 4,
        },
    )
    .expect("a valid config");
    let mut ring = CloneRing::new(5);
    ring.save(0, session.engine());
    for k in 0..40u32 {
        let remote = InputFrame {
            slot: slot(1),
            start_tick: k,
            inputs: vec![WireInput::from_input(&sc.eff[k as usize][1])],
        };
        session.receive_inputs(slot(1), &remote).expect("a frame");
        let out = session.advance(&sc.eff[k as usize][0]).expect("advance");
        assert!(out.waiting.is_none(), "tick {k} waited");
        assert_eq!(out.rolled_back, 0, "tick {k} rolled back");
        session
            .ack(slot(1), session.next_tick(slot(0)))
            .expect("an ack");
    }
    assert_eq!(session.tick(), 40);
}
