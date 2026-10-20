//! The registry's own rules (ADR-026): well-formed IDs, no duplicates, a stable order.

use stella_rain_core::registry::{ENTRIES, Kind, find, is_well_formed};
use stella_rain_core::version::SIM_VERSION;

#[test]
fn every_id_is_well_formed() {
    for e in ENTRIES {
        assert!(is_well_formed(e.id), "{}", e.id);
    }
}

#[test]
fn entries_are_sorted_by_kind_then_id_without_duplicates() {
    let order = |k: Kind| match k {
        Kind::Asset => 0,
        Kind::Palette => 1,
        Kind::Bullet => 2,
        Kind::Preset => 3,
        Kind::Status => 4,
        Kind::Weapon => 5,
        Kind::Mode => 6,
        Kind::Upgrade => 7,
    };
    for pair in ENTRIES.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(
            (order(a.kind), a.id) < (order(b.kind), b.id),
            "{} before {}",
            a.id,
            b.id
        );
    }
}

#[test]
fn only_presets_take_arguments_and_nothing_is_from_the_future() {
    for e in ENTRIES {
        // `cmp`, not `<=`: while the version is 0 clippy calls the comparison absurd.
        assert!(e.introduced.cmp(&SIM_VERSION).is_le(), "{}", e.id);
        assert!(e.kind == Kind::Preset || e.args == 0, "{}", e.id);
    }
}

#[test]
fn find_matches_the_kind() {
    assert!(find(ENTRIES, Kind::Asset, "bat").is_some());
    assert!(find(ENTRIES, Kind::Palette, "bat").is_none());
    assert!(find(ENTRIES, Kind::Preset, "aimed_single").is_some_and(|e| e.args == 1));
}

#[test]
fn id_format_rules() {
    for ok in ["a", "bat", "horns_2", "a_b_c", &"a".repeat(32)] {
        assert!(is_well_formed(ok), "{ok}");
    }
    for bad in [
        "",
        "A",
        "2bat",
        "_bat",
        "bat-2",
        "bat 2",
        "é",
        &"a".repeat(33),
    ] {
        assert!(!is_well_formed(bad), "{bad}");
    }
}

#[test]
fn the_combo_upgrades_are_registered_in_version_0() {
    // ADR-084 point 7, ADR-026: the permanent upgrades that open the AAB and AAAB steps.
    for id in [
        "giant_hammer_aaab",
        "giant_hammer_aab",
        "giant_sword_aaab",
        "giant_sword_aab",
    ] {
        let e = find(ENTRIES, Kind::Upgrade, id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(e.introduced, 0, "{id}");
    }
    assert!(find(ENTRIES, Kind::Weapon, "giant_sword_aab").is_none());
}

#[test]
fn the_upgrade_mask_has_room_for_every_upgrade() {
    let n = ENTRIES.iter().filter(|e| e.kind == Kind::Upgrade).count();
    assert!(n <= stella_rain_core::registry::MAX_UPGRADES);
    assert_eq!(stella_rain_core::registry::upgrade_bit("giant_sword_aab"), Some(3));
}
