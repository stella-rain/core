//! The soft tether through the engine (ADR-SIM-7): two players, one walking away, stay within a
//! few pixels of the range; runs are deterministic.

use crate::common::run::trial_setup;
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;

fn gap_px2(e: &Engine) -> i128 {
    let p = e.snapshot().players;
    let dx = i128::from(p[0].at.x.0 - p[1].at.x.0);
    let dy = i128::from(p[0].at.y.0 - p[1].at.y.0);
    dx * dx + dy * dy
}

fn px(n: i128) -> i128 {
    n * 256
}

#[test]
fn a_walker_is_held_near_the_range() {
    let mut e = Engine::start_party(&trial_setup(), 2).unwrap();
    // The map is 1280 px wide: slot 0 walks to the left edge first (640 px from slot 1, in
    // range), then slot 1 walks to the right, so the gap would pass 1,200 px without the tether.
    let mut inputs = [Input::default(); 3];
    inputs[0].move_x = -127;
    for _ in 0..400 {
        e.step(&inputs);
    }
    assert!(gap_px2(&e) <= px(840) * px(840));
    inputs[0].move_x = 0;
    inputs[1].move_x = 127;
    let mut worst = 0;
    for _ in 0..600 {
        e.step(&inputs);
        worst = worst.max(gap_px2(&e));
    }
    assert!(worst <= px(856) * px(856), "gap {}", worst.isqrt() / 256);
    assert!(worst > px(800) * px(800), "the walker reached the range");
}

#[test]
fn the_same_inputs_give_the_same_hashes() {
    let run = || {
        let mut e = Engine::start_party(&trial_setup(), 3).unwrap();
        let mut hashes = Vec::new();
        for t in 0..600_u32 {
            let mut inputs = [Input::default(); 3];
            inputs[1].move_x = 127;
            inputs[2].move_y = if t % 200 < 100 { -127 } else { 127 };
            e.step(&inputs);
            hashes.push(e.state_hash());
        }
        (
            hashes,
            e.snapshot()
                .players
                .iter()
                .map(|p| p.at)
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(run(), run());
}
