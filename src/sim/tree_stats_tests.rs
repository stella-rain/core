//! Tests for the two new stat effects and the tenths array a weapon tree's stat set is written
//! to (ADR-SIM-7, ADR-SIM-3): the type, the reads, and the state layout.

use super::levels::{self, attack_of};
use super::levels_tests::{hammer, progress, state_of, world_state};
use super::skill;
use super::state::PlayerIdx;
use crate::engine::Engine;
use crate::run_setup::{BuildMode, RunSetup};
use crate::run_stats::StatEffect;

fn set_progress(e: &mut Engine, f: impl FnOnce(&mut super::state::Progress)) {
    let (_, state) = world_state(e);
    f(state
        .players
        .career_mut(PlayerIdx::SOLO)
        .progress
        .as_mut()
        .expect("levels"));
}

/// T3: the combo's range is the player's own, the expertise's and the stat's whole percent.
#[test]
fn range_stat_widens_the_combo() {
    let mut e = hammer();
    let me = PlayerIdx::SOLO;
    assert_eq!(levels::range_pct(state_of(&e), me), 0);
    set_progress(&mut e, |p| p.totals[StatEffect::RangePct as usize] = 25);
    assert_eq!(levels::range_pct(state_of(&e), me), 25);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::RangePct as usize] = 0;
        p.set_tenths[StatEffect::RangePct as usize] = 250;
    });
    assert_eq!(levels::range_pct(state_of(&e), me), 25);
    set_progress(&mut e, |p| {
        p.set_tenths[StatEffect::RangePct as usize] = 259
    });
    assert_eq!(levels::range_pct(state_of(&e), me), 25, "one round down");
    set_progress(&mut e, |p| {
        p.totals[StatEffect::RangePct as usize] = 25;
        p.set_tenths[StatEffect::RangePct as usize] = 10;
    });
    assert_eq!(levels::range_pct(state_of(&e), me), 26);
    // The sum of the sources: the player's own, the expertise's and the stat's.
    set_progress(&mut e, |p| {
        p.totals[StatEffect::RangePct as usize] = 20;
        p.set_tenths[StatEffect::RangePct as usize] = 0;
    });
    {
        let (_, s) = world_state(&mut e);
        s.players[me].range_pct = 50;
        s.players
            .career_mut(me)
            .expertise
            .as_mut()
            .expect("gauge")
            .range_pct = 10;
    }
    assert_eq!(levels::range_pct(state_of(&e), me), 80);
}

/// T4: the stat shortens the stage skill's cooldown, added to the reward's percent, and the
/// sum is held to the existing cap.
#[test]
fn skill_cooldown_stat_shortens_the_skill() {
    let mut e = hammer();
    let me = PlayerIdx::SOLO;
    let at = |e: &Engine| skill::cooldown_after(state_of(e), me, 1000);
    assert_eq!(at(&e), 1000);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 10
    });
    assert_eq!(at(&e), 900);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 60
    });
    assert_eq!(at(&e), 400);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 90
    });
    assert_eq!(at(&e), 400);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 10;
    });
    world_state(&mut e).1.players[me].skill_cooldown_pct = 20;
    assert_eq!(at(&e), 700);
    set_progress(&mut e, |p| {
        p.totals[StatEffect::SkillCooldownPct as usize] = 0;
        p.set_tenths[StatEffect::SkillCooldownPct as usize] = 100;
    });
    assert_eq!(at(&e), 700, "10.0 percent of the set reads as 10");
}

/// T6: an offers run never writes the new totals or the tenths.
#[test]
fn the_new_values_are_zero_in_an_offers_run() {
    let mut e = Engine::start(&RunSetup {
        mode: "time_limit".into(),
        build_mode: BuildMode::Offers,
        ..RunSetup::default()
    })
    .expect("starts");
    for _ in 0..600 {
        e.step(&[crate::input::Input::default(); crate::id::MAX_PLAYERS as usize]);
    }
    let p = progress(&e);
    assert_eq!((p.totals[11], p.totals[12]), (0, 0));
    assert!(p.set_tenths.iter().all(|t| *t == 0));
}

/// T7: with no set the tenths reads are today's numbers.
#[test]
fn a_set_free_read_is_todays_number() {
    let e = hammer();
    let mut p = progress(&e).clone();
    for w in 0..=100u32 {
        p.totals[StatEffect::AttackPct as usize] = w;
        for atk in [0u32, 1, 7, 99, 1000, 123_457, 4_000_000] {
            let old = u32::try_from(u64::from(atk) * u64::from(100 + w) / 100).unwrap();
            assert_eq!(attack_of(Some(&p), atk), old, "w {w} atk {atk}");
        }
        p.totals[StatEffect::RangePct as usize] = w;
        assert_eq!(levels::set_pct(&p, StatEffect::RangePct), w);
    }
    // A set value is exact on attack and rounds down once on the whole-percent reads.
    p.totals = [0; StatEffect::ALL.len()];
    p.set_tenths[StatEffect::AttackPct as usize] = 165;
    assert_eq!(attack_of(Some(&p), 1000), 1165);
    p.set_tenths[StatEffect::AttackSpeedPct as usize] = 35;
    assert_eq!(levels::set_pct(&p, StatEffect::AttackSpeedPct), 3);
}
