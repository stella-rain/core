//! The setup and the loop of an equipment-mode run played through a `RunRecorder` (ADR-SIM-7):
//! no offer ever opens, so nothing is answered; each slot's `Input.action` is applied and
//! recorded by the recorder, never by a bare `Engine::step`.

use stella_rain_core::engine::Outcome;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::RunRecorder;
use stella_rain_core::run_setup::RunSetup;

/// `setup` as it is: `sim_version` 2 is the equipment mode. Kept so a call site says which
/// rules it means.
pub fn equipment_setup(setup: RunSetup) -> RunSetup {
    setup
}

/// Steps `rec` for up to `ticks` ticks, or to the run's end. `inputs` gives each present slot's
/// input of tick `t`, its `action` included.
pub fn play(
    rec: &mut RunRecorder,
    ticks: u32,
    mut inputs: impl FnMut(&RunRecorder, u32) -> [Input; 3],
) {
    for t in 0..ticks {
        if rec.engine().outcome() != Outcome::Running {
            break;
        }
        assert!(
            !rec.engine().offer_open(),
            "an offer is open at tick {t}: equipment mode opens none"
        );
        let step = inputs(rec, t);
        rec.step(&step).unwrap();
    }
}
