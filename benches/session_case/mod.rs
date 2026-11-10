//! The fixtures and measured bodies of `benches/rollback_session.rs` (ADR-SIM-6, ADR-SIM-7); not a
//! target itself (Cargo discovers only `benches/*.rs`). `tests/bench_fixture_pins.rs` includes
//! this file, so the bench and its pins run one definition, as `support::tick` is shared.
//!
//! A `Session` starts at tick 0, so the run of the `rollback_equip` case is driven through its
//! public calls up to `EQUIP_PEAK_TICK`: every tick the three slots' inputs of that case, the
//! ramp-up action of each slot riding in its input (delay 0). Then `window` ticks are predicted
//! with both remotes silent, and their real inputs arrive: the next `advance` re-simulates
//! exactly those `window` ticks and steps one new one. That `advance` is the measured unit.

use stella_rain_core::bench_fixtures::equip_case::{self, EQUIP_PEAK_TICK, ROLLBACK_ACTIONS};
use stella_rain_core::bench_fixtures::party_case::minute_inputs;
use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_net::save::{CloneRing, Save};
use stella_rain_net::session::{Config, Session};
use stella_rain_net::wire::{InputFrame, WireInput};

/// The capacity of the ring of `clone_ring_save`: any capacity above 1 never evicts.
pub const RING_CAPACITY: usize = 5;

/// A session one `advance` away from the rollback, and the input that `advance` is given.
pub struct Prepared {
    pub session: Session,
    pub local: Input,
}

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot of the party")
}

/// The input of slot `s` at `tick`, carrying the ramp-up action the engine accepts now.
fn ramped(session: &Session, s: u8, tick: u32) -> Input {
    Input {
        action: equip_case::ramp(session.engine(), s, tick),
        ..minute_inputs(tick)[usize::from(s)]
    }
}

/// One input of a remote slot as a frame of one tick.
fn frame(s: u8, tick: u32, input: &Input) -> InputFrame {
    InputFrame {
        slot: slot(s),
        start_tick: tick,
        inputs: vec![WireInput::from_input(input)],
    }
}

/// The window's recorded action of slot `s` that comes before window tick `at`, if any.
fn window_action(s: usize, at: u32) -> Option<stella_rain_core::input::EquipAction> {
    ROLLBACK_ACTIONS
        .iter()
        .find(|(when, _)| *when == at)
        .and_then(|(_, actions)| actions[s])
}

/// The input of slot `s` at window tick `at`, with the window's recorded action.
fn window_input(s: usize, at: u32) -> Input {
    Input {
        action: window_action(s, at),
        ..minute_inputs(EQUIP_PEAK_TICK + at)[s]
    }
}

/// A session of the equipment run at tick `EQUIP_PEAK_TICK`, every tick confirmed.
pub fn at_peak(window: u32) -> Session {
    let config = Config {
        local: slot(0),
        delay: 0,
        window,
    };
    let mut session = Session::new(equip_case::start(), config).expect("the session starts");
    for t in 0..EQUIP_PEAK_TICK {
        let local = ramped(&session, 0, t);
        for s in [1, 2] {
            let input = ramped(&session, s, t);
            session
                .receive_inputs(slot(s), &frame(s, t, &input))
                .expect("the remote input is taken");
        }
        let advance = session.advance(&local).expect("advances");
        assert!(advance.waiting.is_none() && advance.rolled_back == 0);
        for s in [1, 2] {
            session.ack(slot(s), t + 1).expect("the ack is taken");
        }
    }
    session
}

/// The fixture of one case: the session at the peak, `window` ticks predicted with both remotes
/// silent, and then their real inputs received.
pub fn driven(window: u32) -> Prepared {
    let mut session = at_peak(window);
    for i in 0..window {
        let advance = session.advance(&window_input(0, i)).expect("advances");
        assert!(advance.waiting.is_none() && advance.rolled_back == 0);
    }
    for s in [1u8, 2] {
        let inputs = (0..window)
            .map(|i| WireInput::from_input(&window_input(usize::from(s), i)))
            .collect();
        let frame = InputFrame {
            slot: slot(s),
            start_tick: EQUIP_PEAK_TICK,
            inputs,
        };
        session
            .receive_inputs(slot(s), &frame)
            .expect("the real inputs are taken");
    }
    Prepared {
        session,
        local: window_input(0, window),
    }
}

/// The measured body of the rollback cases: one `advance`, which restores the first mispredicted
/// tick, re-simulates the window and steps one new tick. The fixture is handed back, so the bench
/// function drops nothing inside the counted region.
#[inline]
pub fn rollback(mut prepared: Prepared) -> Prepared {
    prepared.session.advance(&prepared.local).expect("advances");
    prepared
}

/// The measured body of `clone_ring_save`: one save of the engine into the ring, both handed back.
#[inline]
pub fn save_once((mut ring, engine): (CloneRing, Engine)) -> (CloneRing, Engine) {
    ring.save(engine.tick(), &engine);
    (ring, engine)
}
