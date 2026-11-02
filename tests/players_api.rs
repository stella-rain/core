//! The player index is part of the public API (ADR-SIM-12 point 1).

use stella_rain_core::id::{EntityId, MAX_PLAYERS, PlayerIdx};

#[test]
fn player_idx_is_public_and_bounded() {
    assert_eq!(MAX_PLAYERS, 3);
    assert_eq!(PlayerIdx::new(0), Some(PlayerIdx::SOLO));
    assert!(PlayerIdx::new(2).is_some());
    assert!(PlayerIdx::new(3).is_none());
    assert_eq!(PlayerIdx::SOLO.id(), EntityId(0));
}
