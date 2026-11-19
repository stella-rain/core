//! What a player's snapshot view publishes of its own state and of the rules' constants: the most
//! hit points, and on a downed player the lengths of the out timer and the revive hold and the
//! revive range (ADR-SIM-7). Every value is read where the rules read it, never copied.

use super::state::{OUT_TIMER_TICKS, REVIVE_HOLD_TICKS, REVIVE_RANGE};
use super::trigger_tests::{rig, slot, st};
use serde_json::json;

/// V1: each view carries its own player's most hit points, in a party and solo.
#[test]
fn a_view_publishes_each_players_own_max_hp() {
    let mut r = rig(2, vec![], 0);
    assert_eq!(r.view(0)["max_hp"], json!(1000));
    assert_eq!(r.view(1)["max_hp"], json!(1000));
    for i in 0..2 {
        let own = st(&mut r.e).players[slot(i)].max_hp;
        assert_eq!(r.view(usize::from(i))["max_hp"], json!(own));
    }
    st(&mut r.e).players[slot(1)].max_hp = 1750;
    assert_eq!(r.view(1)["max_hp"], json!(1750));
    assert_eq!(r.view(0)["max_hp"], json!(1000));

    let solo = rig(1, vec![], 0);
    assert_eq!(solo.view(0)["max_hp"], json!(1000));
}

/// V2: damage lowers `hp` and leaves the maximum alone.
#[test]
fn damage_lowers_hp_in_the_view_and_leaves_max_hp_alone() {
    let mut r = rig(2, vec![], 0);
    r.hurt(1, 600, false);
    assert_eq!(r.view(1)["hp"], json!(400));
    assert_eq!(r.view(1)["max_hp"], json!(1000));
}

/// V3: a downed view carries the lengths the rules use; an up player has no `down`.
#[test]
fn a_downed_view_carries_the_lengths_the_rules_use() {
    let mut r = rig(2, vec![], 0);
    r.hurt(1, 2_000_000, false);
    let down = r.view(1)["down"].clone();
    assert_eq!(down["out_ticks"], json!(OUT_TIMER_TICKS));
    assert_eq!(down["revive_hold_ticks"], json!(REVIVE_HOLD_TICKS));
    assert_eq!(down["revive_range"], json!(REVIVE_RANGE.0));
    assert_eq!(down["ticks_left"], json!(OUT_TIMER_TICKS));
    assert_eq!(down["revive_hold"], json!(0));

    r.idle(5);
    let down = r.view(1)["down"].clone();
    assert_eq!(down["out_ticks"], json!(OUT_TIMER_TICKS));
    assert_eq!(down["revive_hold_ticks"], json!(REVIVE_HOLD_TICKS));
    assert_eq!(down["revive_range"], json!(REVIVE_RANGE.0));
    assert_eq!(down["ticks_left"], json!(OUT_TIMER_TICKS - 5));

    assert!(r.view(0).get("down").is_none());
    assert!(r.view(0).get("max_hp").is_some());
}
