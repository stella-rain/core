//! Tests for a stage's `try_styles` (ADR-SIM-9 points 3, 5, 8 and 9): every tree point of the
//! level table is a style pick again; the pick rewrites the weapon's set at the weapon level the
//! gauge has, the gauge's `path` is the log of the picks after the first, and no upgrade effect is
//! ever applied.

use super::TreeError;
use super::expertise;
use super::levels_tests::{
    gauge, hammer, next, progress, reach, state_of, total, tree_pick, world_state,
};
use super::state::PlayerIdx;
use crate::engine::Engine;
use crate::expertise::Expertise;
use crate::id::ContentId;
use crate::pack::Pack;
use crate::run_setup::RunSetup;
use crate::run_stats::StatEffect;
use crate::snapshot::ExpertiseOfferKind;
use serde_json::{Value, json};

const STYLES: [&str; 3] = ["crush", "quake", "bulwark"];

/// A `tutorial`-mode run on the Hammer, from a pack whose tutorial names `try_styles` and a
/// stand-in table of points at levels 1 to 4, with level 10 reached so four points are held.
fn try_hammer(unlocked: Option<Vec<ContentId>>) -> Engine {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    v["modes"]["tutorial"]["try_styles"] = true.into();
    v["modes"]["tutorial"]["level_table"] =
        json!({ "style": 1, "upgrades": [2, 3], "capstone": 4 });
    let pack = Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack");
    let setup = RunSetup {
        mode: "tutorial".into(),
        weapon: "giant_hammer".into(),
        unlocked_styles: unlocked,
        ..RunSetup::default()
    };
    let mut e = Engine::start_with(&setup, pack.content()).expect("starts");
    reach(&mut e, 10);
    e
}

fn ids(list: &[&str]) -> Option<Vec<ContentId>> {
    Some(list.iter().map(|s| ContentId::from(*s)).collect())
}

fn styles() -> Vec<String> {
    STYLES.iter().map(|s| (*s).to_owned()).collect()
}

/// (1) Four points in order, each a style pick; then none.
#[test]
fn every_table_level_is_a_style_pick() {
    use ExpertiseOfferKind::Style;
    let mut e = try_hammer(None);
    assert_eq!(next(&mut e), Some((Style, 1, 4, styles())));
    for (pick, level, held) in [(0, 2, 3), (1, 3, 2), (2, 4, 1)] {
        tree_pick(&mut e, pick).expect("taken");
        assert_eq!(next(&mut e), Some((Style, level, held, styles())));
    }
    tree_pick(&mut e, 0).expect("the last point");
    assert_eq!(next(&mut e), None);
    assert_eq!(tree_pick(&mut e, 0), Err(TreeError::NoTreePoint));
}

#[test]
fn an_option_past_the_styles_is_refused() {
    let mut e = try_hammer(None);
    let (world, state) = world_state(&mut e);
    assert_eq!(
        super::check_tree_pick(world, Expertise::bundled(), state, PlayerIdx::SOLO, 3),
        Err(TreeError::NoTreeOption {
            option: 3,
            options: 3
        })
    );
}

/// (2) The style switches and the gauge stands.
#[test]
fn a_re_pick_switches_the_style_and_leaves_the_gauge() {
    let mut e = try_hammer(None);
    let g = gauge(&e);
    let (level, points, nxt, range) = (g.level, g.points, g.next, g.range_pct);
    let totals = progress(&e).totals;
    tree_pick(&mut e, 0).unwrap();
    tree_pick(&mut e, 1).unwrap();
    let g = gauge(&e);
    assert_eq!(g.style, Some(1));
    assert_eq!(
        (g.level, g.points, g.next, g.range_pct),
        (level, points, nxt, range)
    );
    assert_eq!(range, 0);
    let mut expected = totals;
    expected[StatEffect::MaxHp as usize] = progress(&e).totals[StatEffect::MaxHp as usize];
    assert_eq!(progress(&e).totals, expected);
}

/// (3) The set is recomputed at the weapon level, never stacked, and max_hp moves by the
/// difference.
#[test]
fn the_set_is_rewritten_at_the_weapon_level_not_stacked() {
    let mut e = try_hammer(None);
    let hp0 = state_of(&e).players[PlayerIdx::SOLO].max_hp;
    let mut at = hp0;
    let mut was: Option<u8> = None; // the base set before the first pick
    for pick in [0u8, 2, 0] {
        tree_pick(&mut e, u32::from(pick)).unwrap();
        let (world, state) = world_state(&mut e);
        let g = state
            .players
            .career(PlayerIdx::SOLO)
            .expertise
            .as_ref()
            .unwrap();
        let (style, level) = (g.style, g.level);
        let want = expertise::set_for(world, Expertise::bundled(), style, level);
        let p = state
            .players
            .career(PlayerIdx::SOLO)
            .progress
            .as_ref()
            .unwrap();
        let mut tenths = p.set_tenths;
        // max_hp lives in the totals; its tenths entry stays 0.
        tenths[StatEffect::MaxHp as usize] = want[StatEffect::MaxHp as usize];
        assert_eq!(tenths, want);
        let cur = state.players[PlayerIdx::SOLO].max_hp;
        let max_hp = |style: Option<u8>| {
            i64::from(
                expertise::set_for(world, Expertise::bundled(), style, level)
                    [StatEffect::MaxHp as usize],
            )
        };
        let (old, new) = (max_hp(was), max_hp(Some(pick)));
        // Switching a style moves the maximum by the difference of the two sets' entries.
        let moved = new - old;
        // A fall is held at 1 (`levels::remove_effect`); the harness reaches level 10 without the
        // level-up that would have written the set, so the first fall can be clamped.
        let want_hp = (i64::from(at) + moved).max(1);
        assert_eq!(i64::from(cur), want_hp, "pick {pick}");
        assert!(state_of(&e).players[PlayerIdx::SOLO].hp <= cur);
        at = cur;
        was = Some(pick);
    }
}

/// (4) The log: one place per pick after the first, and no upgrade effect is ever applied.
#[test]
fn the_path_is_the_log_of_the_picks_after_the_first() {
    let mut e = try_hammer(None);
    let (attack, range) = (total(&e, StatEffect::AttackPct), gauge(&e).range_pct);
    for pick in [1, 0, 2, 2] {
        tree_pick(&mut e, pick).unwrap();
    }
    assert_eq!(gauge(&e).path, [0, 2, 2]);
    assert_eq!(total(&e, StatEffect::AttackPct), attack);
    assert_eq!(gauge(&e).range_pct, range);
}

/// (5) `unlocked_styles` filters every point; absent offers every style, the active one included.
#[test]
fn unlocked_styles_filter_every_point() {
    use ExpertiseOfferKind::Style;
    let mut e = try_hammer(ids(&["bulwark", "crush"]));
    for level in 1..=4 {
        let (kind, l, _, choices) = next(&mut e).expect("a point");
        assert_eq!((kind, l), (Style, level));
        assert_eq!(choices, ["crush", "bulwark"]);
        tree_pick(&mut e, 1).unwrap();
        assert_eq!(gauge(&e).style, Some(2));
    }
    let mut e = try_hammer(ids(&[]));
    assert_eq!(next(&mut e), None);
    assert_eq!(tree_pick(&mut e, 0), Err(TreeError::NoTreePoint));
}

#[test]
fn re_picking_the_active_style_spends_the_point_and_changes_nothing_else() {
    let mut e = try_hammer(None);
    tree_pick(&mut e, 1).unwrap();
    let (tenths, totals) = (progress(&e).set_tenths, progress(&e).totals);
    tree_pick(&mut e, 1).unwrap();
    assert_eq!(gauge(&e).path, [1]);
    assert_eq!(
        (progress(&e).set_tenths, progress(&e).totals),
        (tenths, totals)
    );
    assert_eq!(next(&mut e).map(|n| n.2), Some(2));
}

/// (6) `check_tree` under the field and without it.
#[test]
fn check_tree_holds_the_log_to_the_styles() {
    let mut e = try_hammer(None);
    let mut plain = hammer();
    reach(&mut plain, 10);
    let set = |e: &mut Engine, style: Option<u8>, level: u32, path: &[u8]| {
        let (world, state) = world_state(e);
        let mut g = state
            .players
            .career(PlayerIdx::SOLO)
            .expertise
            .clone()
            .unwrap();
        (g.style, g.level, g.path) = (style, level, path.to_vec());
        expertise::check_tree(world, Expertise::bundled(), &g)
    };
    assert_eq!(set(&mut e, Some(1), 10, &[2, 0]), Ok(()));
    assert_eq!(
        set(&mut plain, Some(1), 10, &[2, 0]),
        Err("expertise tree place out of range")
    );
    assert_eq!(
        set(&mut e, Some(1), 10, &[3]),
        Err("expertise tree place out of range")
    );
    let past = Err("expertise tree taken past the level");
    assert_eq!(set(&mut e, Some(0), 1, &[0]), past);
    assert_eq!(set(&mut e, Some(0), 10, &[0, 0, 0, 0]), past);
}

/// (7) The view reports an empty path under the field; without it the upgrade path shows.
#[test]
fn the_view_reports_no_path_under_the_field() {
    let mut e = try_hammer(None);
    for pick in [0, 1, 2] {
        tree_pick(&mut e, pick).unwrap();
    }
    let (world, state) = world_state(&mut e);
    let g = state
        .players
        .career(PlayerIdx::SOLO)
        .expertise
        .as_ref()
        .unwrap();
    assert_eq!(
        super::expertise_styles_view(world, Expertise::bundled(), g),
        (Some("bulwark".into()), Vec::new())
    );
    let mut plain = hammer();
    reach(&mut plain, 10);
    for _ in 0..2 {
        tree_pick(&mut plain, 0).unwrap();
    }
    let (world, state) = world_state(&mut plain);
    let g = state
        .players
        .career(PlayerIdx::SOLO)
        .expertise
        .as_ref()
        .unwrap();
    assert!(
        !super::expertise_styles_view(world, Expertise::bundled(), g)
            .1
            .is_empty()
    );
}
