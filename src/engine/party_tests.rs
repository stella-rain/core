//! A party starts from a setup (ADR-SIM-12): the same state as the one-player start grown to
//! `n` by the one builder, and version 1 stays one player.

use crate::run_setup::RunSetup;

use super::*;

fn setup() -> RunSetup {
    RunSetup {
        seed: 20_261_010,
        pick_stats: true,
        ..RunSetup::default()
    }
}

#[test]
fn a_started_party_is_the_solo_start_grown_to_n() {
    for n in 1..=3u8 {
        let started = Engine::start_party(&setup(), n).unwrap();
        let grown = Engine::start(&setup()).unwrap().into_party(n);
        assert_eq!(started.players(), usize::from(n));
        assert_eq!(started.state_hash(), grown.state_hash(), "party {n}");
    }
}

#[test]
fn a_party_outside_one_to_three_is_invalid() {
    for n in [0u8, 4, 255] {
        assert!(matches!(
            Engine::start_party(&setup(), n),
            Err(Error::Invalid(issues)) if issues[0].path == "$.party"
        ));
    }
}

#[test]
fn version_one_has_one_player() {
    let party = Engine::forced(1, || Engine::start_party(&setup(), 2));
    assert!(matches!(party, Err(Error::Invalid(issues)) if issues[0].path == "$.party"));
    assert!(Engine::forced(1, || Engine::start_party(&setup(), 1)).is_ok());
}
