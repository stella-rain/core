//! Tests for the equipment level-ups and the weapon tree (ADR-SIM-7, ADR-SIM-2 point 2): a
//! level-up spends by the stat priority and opens no offer, a common expertise level takes its
//! stat by the chosen tree, and the style path is held tree points.

use super::levels;
use super::state::{PlayerIdx, State};
use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::run_setup::{BuildMode, RunSetup};
use crate::run_stats::StatEffect;

fn setup(mode: &str) -> RunSetup {
    RunSetup {
        mode: mode.into(),
        build_mode: BuildMode::Equipment,
        ..RunSetup::default()
    }
}

fn state_of(e: &Engine) -> &State {
    match e.run() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// An engine whose run has levels and the expertise gauge, in the build mode `mode`.
fn engine(build_mode: BuildMode) -> Engine {
    Engine::start(&RunSetup {
        mode: "time_limit".into(),
        build_mode,
        ..RunSetup::default()
    })
    .expect("starts")
}

/// Gives player 0 the experience of `n` levels at once, returning the events and leaving the
/// priority as `priority` (places of the run stats) with the stat offer switch `pick_stats`.
fn level_up(e: &mut Engine, priority: &[u8], pick_stats: bool, n: u32) -> Vec<DomainEvent> {
    let Run::V2 { world, state } = e.run_mut() else {
        panic!("the live rules");
    };
    let p = state
        .players
        .career_mut(PlayerIdx::SOLO)
        .progress
        .as_mut()
        .expect("levels");
    p.priority = priority.to_vec();
    p.pick_stats = pick_stats;
    let from = p.level;
    let xp: u32 = (0..n).map(|i| levels::xp_next(world, from + i)).sum();
    let mut events = Vec::new();
    levels::gain(world, state, PlayerIdx::SOLO, xp, &mut events);
    events
}

fn progress(e: &Engine) -> &super::state::Progress {
    state_of(e)
        .players
        .career(PlayerIdx::SOLO)
        .progress
        .as_ref()
        .expect("levels")
}

/// T1: in equipment mode a level-up spends by the priority and opens no stat offer, whatever
/// the `pick_stats` switch says.
#[test]
fn an_equipment_level_up_spends_by_priority_and_opens_no_offer() {
    let mut e = engine(BuildMode::Equipment);
    let (attack, speed) = (1u8, 4u8);
    let events = level_up(&mut e, &[attack, speed], true, 2);
    let p = progress(&e);
    let raised: Vec<u8> = events
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::StatRaised { spent, .. } => Some(*spent),
            _ => None,
        })
        .collect();
    let leveled = events
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::LeveledUp { .. }))
        .count();
    assert_eq!((leveled, raised), (2, vec![1, 2]));
    assert_eq!((p.offers_due, p.offer_open), (0, false));
    assert!(!super::offer_open_for(state_of(&e), PlayerIdx::SOLO));
    let step = p.stats[usize::from(attack)].step;
    assert_eq!(p.totals[StatEffect::AttackPct as usize], 2 * step);
    assert_eq!(p.spent[usize::from(attack)], 2);
}

/// T2: pin: an offers run with the switch queues the stat offers and spends nothing.
#[test]
fn an_offers_level_up_with_the_switch_queues_the_stat_offer() {
    let mut e = engine(BuildMode::Offers);
    let events = level_up(&mut e, &[1, 4], true, 2);
    let p = progress(&e);
    assert_eq!(p.offers_due, 2);
    assert!(
        !events
            .iter()
            .any(|ev| matches!(ev, DomainEvent::StatRaised { .. }))
    );
    assert!(p.spent.iter().all(|n| *n == 0));
}

/// T3: with every stat at its limit a level gives nothing but the level.
#[test]
fn an_equipment_level_with_every_stat_at_its_limit_gives_nothing() {
    let mut e = engine(BuildMode::Equipment);
    {
        let Run::V2 { state, .. } = e.run_mut() else {
            panic!("the live rules");
        };
        let p = state
            .players
            .career_mut(PlayerIdx::SOLO)
            .progress
            .as_mut()
            .expect("levels");
        for (n, rule) in p.spent.iter_mut().zip(&p.stats) {
            *n = rule.limit;
        }
    }
    let totals = progress(&e).totals;
    let events = level_up(&mut e, &[0, 1, 2, 3, 4], true, 1);
    let p = progress(&e);
    assert!(
        events
            .iter()
            .all(|ev| matches!(ev, DomainEvent::LeveledUp { .. }))
    );
    assert_eq!(events.len(), 1);
    assert_eq!((p.totals, p.offers_due), (totals, 0));
}
