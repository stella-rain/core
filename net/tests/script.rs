//! The test oracle itself: a straight run of the bot's script is repeatable, plays more than one
//! kind of equipment action, and a leave changes the run.

mod support;

use stella_rain_core::input::EquipAction;
use support::{script, slot, straight};

fn kind(a: &EquipAction) -> u8 {
    match a {
        EquipAction::Equip { .. } => 0,
        EquipAction::Replace { .. } => 1,
        EquipAction::Extract { .. } => 2,
        EquipAction::Craft { .. } => 3,
        EquipAction::TreePick { .. } => 4,
    }
}

#[test]
fn the_script_plays_actions_and_the_oracle_is_repeatable() {
    let s = script(3, 1500, 2, None);
    let actions: Vec<_> = s
        .eff
        .iter()
        .flat_map(|row| row.iter().filter_map(|i| i.action))
        .collect();
    let mut kinds: Vec<u8> = actions.iter().map(kind).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert!(actions.len() >= 3, "only {} actions", actions.len());
    assert!(kinds.len() >= 2, "only kinds {kinds:?}");
    let (a, b) = (straight(&s, 3), straight(&s, 3));
    assert_eq!(a, b);
    assert_eq!(a.len(), 1501);
    assert_ne!(a[0], a[600]);
    assert_ne!(a[600], a[1500]);
}

#[test]
fn a_leave_changes_the_run_after_its_tick_and_silences_the_leaver() {
    let plain = script(3, 600, 2, None);
    let left = script(3, 600, 2, Some((slot(2), 300)));
    let (a, b) = (straight(&plain, 3), straight(&left, 3));
    assert_eq!(a[300], b[300]);
    assert_ne!(a[301], b[301]);
    for row in &left.eff[300..] {
        assert_eq!(row[2], Default::default());
    }
}
