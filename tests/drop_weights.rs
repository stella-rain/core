//! The player's drop weights per perk category (ADR-SAVE-17): a weight changes no draw, so a
//! setup with none, and a setup with an empty map, play the same bytes; the weighting itself is
//! pinned in `src/sim/drops_tests.rs` (`drop_weight_scales_every_star_of_its_category`).

use serde_json::json;
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::run_setup::RunSetup;

fn weighted(weights: Option<serde_json::Value>) -> RunSetup {
    let mut v = serde_json::to_value(RunSetup {
        seed: 7,
        ..RunSetup::default()
    })
    .unwrap();
    if let Some(w) = weights {
        v["drop_weights"] = w;
    }
    serde_json::from_value(v).expect("a setup with drop weights reads")
}

fn input(tick: u32) -> Input {
    Input {
        move_x: if (tick / 40) % 2 == 0 { 127 } else { -127 },
        main: (tick / 45) % 3 == 1,
        smash: tick % 9 == 0,
        ..Input::default()
    }
}

/// The state hash at every 60th tick of 1,200 ticks.
fn hashes(setup: &RunSetup) -> Vec<u64> {
    let mut engine = Engine::start(setup).expect("starts");
    (0..1200)
        .filter_map(|t| {
            engine.step(&[input(t), Input::default(), Input::default()]);
            (t % 60 == 59).then(|| engine.state_hash())
        })
        .collect()
}

#[test]
fn an_empty_map_and_no_map_are_the_same_run() {
    assert_eq!(hashes(&weighted(Some(json!({})))), hashes(&weighted(None)));
}

#[test]
fn a_weighted_run_is_deterministic_and_takes_the_same_steps() {
    let w = weighted(Some(json!({ "attack": 100 })));
    let a = hashes(&w);
    assert_eq!(a, hashes(&w));
    assert_eq!(a.len(), hashes(&weighted(None)).len());
}
