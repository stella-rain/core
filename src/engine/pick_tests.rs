//! An offer pick travels inside the step's input (ADR-SIM-7, ADR-REPL-5): `step` takes one
//! input per slot, `check_pick` says why a pick is refused, and a pick changes nothing it
//! should not.

use crate::bench_fixtures::worst_case;
use crate::input::Input;

#[test]
fn a_pick_on_a_running_tick_is_ignored() {
    let mut a = worst_case::warmed_up();
    let mut b = a.clone();
    a.step(&[
        Input {
            pick: Some(3),
            ..Input::default()
        },
        Default::default(),
        Default::default(),
    ]);
    b.step(&[Input::default(), Default::default(), Default::default()]);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.events(), b.events());
}
