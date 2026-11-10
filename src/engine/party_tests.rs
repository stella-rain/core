//! A party starts from a setup (ADR-SIM-7): the same state as the one-player start grown to
//! `n` by the one builder, and version 1 stays one player.

use crate::run_setup::RunSetup;

use super::*;

fn setup() -> RunSetup {
    RunSetup {
        seed: 20_261_010,
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

// --- each player's result (ADR-SAVE-3, ADR-SAVE-6 point 2) --------------------------------------

mod result_of {
    use super::*;
    use crate::progression::{Bank, RunOutcome, RunResult, settle};

    fn slot(i: u8) -> PlayerIdx {
        PlayerIdx::new(i).unwrap()
    }

    /// A party of `n` whose players hold `stardust` tenths, slot 1 out, the run at `outcome`.
    fn finished(n: u8, stardust: [u32; 3], outcome: Outcome) -> Engine {
        let mut e = Engine::start_party(&setup(), n).unwrap();
        let Run::V2 { state, .. } = e.run_mut() else {
            panic!("the live rules")
        };
        for i in state.players.indices().collect::<Vec<_>>() {
            state.players.career_mut(i).stardust = Some(stardust[i.get()]);
        }
        if n > 1 {
            state.players[slot(1)].hp = 0;
            state.players[slot(1)].out = true;
        }
        state.outcome = outcome;
        e
    }

    #[test]
    fn each_present_slot_reads_its_own_stardust() {
        let e = finished(3, [700, 500, 900], Outcome::Cleared);
        assert_eq!(
            [0, 1, 2].map(|i| e.stardust_of(slot(i))),
            [Some(700), Some(500), Some(900)]
        );
        assert_eq!(e.stardust(), 700, "the run-wide reading is slot 0's");
        let two = finished(2, [700, 500, 900], Outcome::Cleared);
        assert_eq!(two.stardust_of(slot(2)), None);
    }

    #[test]
    fn version_one_answers_slot_zero_only() {
        let e = Engine::forced(1, || Engine::start(&setup())).unwrap();
        assert_eq!(e.stardust_of(slot(0)), Some(e.stardust()));
        assert_eq!(
            (e.stardust_of(slot(1)), e.stardust_of(slot(2))),
            (None, None)
        );
    }

    #[test]
    fn an_out_player_is_paid_as_for_a_clear() {
        let content = RunContent::bundled();
        let e = finished(3, [700, 500, 900], Outcome::Cleared);
        let r = RunResult::from_run_for(&setup(), &e, slot(1)).unwrap();
        assert_eq!((r.outcome, r.stardust_tenths), (RunOutcome::Cleared, 500));
        let s = settle(content, &r, &Bank::default());
        assert_eq!(s.paid_tenths, 500);
        // Slot 0's result is what `from_run` gives.
        assert_eq!(
            RunResult::from_run_for(&setup(), &e, slot(0)),
            RunResult::from_run(&setup(), &e)
        );
    }

    #[test]
    fn a_wipe_fails_every_slot_with_its_own_counter() {
        let content = RunContent::bundled();
        let e = finished(3, [700, 500, 900], Outcome::Failed);
        let rules = content.stardust().mode(setup().mode.as_str());
        for (i, want) in [(0, 700u32), (1, 500), (2, 900)] {
            let r = RunResult::from_run_for(&setup(), &e, slot(i)).unwrap();
            assert_eq!((r.outcome, r.stardust_tenths), (RunOutcome::Failed, want));
            let s = settle(content, &r, &Bank::default());
            let pay_pct = rules.map_or(100, |m| u64::from(m.pay_pct));
            assert_eq!(s.paid_tenths, u64::from(want) * pay_pct / 100, "slot {i}");
        }
    }

    #[test]
    fn no_result_while_running_or_for_an_absent_slot() {
        let running = finished(3, [1, 2, 3], Outcome::Running);
        assert!(RunResult::from_run_for(&setup(), &running, slot(0)).is_none());
        let two = finished(2, [1, 2, 3], Outcome::Cleared);
        assert!(RunResult::from_run_for(&setup(), &two, slot(2)).is_none());
        assert!(RunResult::from_run_for(&setup(), &two, slot(1)).is_some());
    }
}
