//! Every `State` type: the whole state of a run, hashed by `StableHash` (`state_hash.rs`, ADR-SIM-3).

mod attack_run;
mod foes;
mod orbs_zones;
mod player_state;
mod players;
mod progress;
mod run;
mod status;

pub(crate) use attack_run::*;
pub(crate) use foes::*;
pub(crate) use orbs_zones::*;
pub(crate) use player_state::*;
pub(crate) use players::*;
pub(crate) use progress::*;
pub(crate) use run::*;
pub(crate) use status::*;

#[cfg(test)]
mod status_kind_tests {
    use super::StatusKind;

    #[test]
    fn every_status_kind_is_listed() {
        // An exhaustive match: a kind added without a number here does not compile, and one
        // without a place in `ALL` fails the assert below, so `Statuses` is sized for all.
        let number = |k: StatusKind| match k {
            StatusKind::AtkUp => 0,
            StatusKind::Vulnerable => 1,
            StatusKind::Slow => 2,
            StatusKind::Bleed => 3,
            StatusKind::Poison => 4,
            StatusKind::Burn => 5,
            StatusKind::Stun => 6,
        };
        let numbers: Vec<usize> = StatusKind::ALL.into_iter().map(number).collect();
        assert_eq!(numbers, (0..StatusKind::COUNT).collect::<Vec<_>>());
    }

    #[test]
    fn a_status_kind_is_its_own_id() {
        for (i, kind) in StatusKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, i);
            assert_eq!(kind.id().as_str(), kind.name());
        }
    }
}
