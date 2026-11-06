//! Tests for the equipment level-ups and the weapon tree (ADR-SIM-7, ADR-SIM-2 point 2): a
//! level-up spends by the stat priority and opens no offer, a common expertise level takes its
//! stat by the chosen tree, and the style path is held tree points.

use super::TreeError;
use super::expertise::{self, TREE_COMMON};
use super::levels;
use super::state::{PlayerIdx, State};
use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::expertise::{Expertise, ExpertiseEffect, WeaponExpertise};
use crate::run_setup::{BuildMode, RunSetup};
use crate::run_stats::StatEffect;
use crate::snapshot::ExpertiseOfferKind;
use crate::stage::LevelTable;

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
    let before = progress(&e).totals[StatEffect::AttackPct as usize];
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
    assert_eq!(p.totals[StatEffect::AttackPct as usize], before + 2 * step);
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

// --- the automatic common levels ----------------------------------------------------------------

/// A time-limit equipment engine with the hammer, whose trees are `crush`, `quake` and `bulwark`.
fn hammer() -> Engine {
    Engine::start(&RunSetup {
        mode: "time_limit".into(),
        build_mode: BuildMode::Equipment,
        weapon: "giant_hammer".into(),
        ..RunSetup::default()
    })
    .expect("starts")
}

/// Sets player 0's gauge to `level` with the levels up to `offered` passed, `style` chosen, and
/// the stat priority; then passes the levels due, returning the events they made.
fn pass(
    e: &mut Engine,
    offered: u32,
    level: u32,
    style: Option<u8>,
    priority: &[u8],
) -> Vec<DomainEvent> {
    {
        let Run::V2 { state, .. } = e.run_mut() else {
            panic!("the live rules");
        };
        let c = state.players.career_mut(PlayerIdx::SOLO);
        let g = c.expertise.as_mut().expect("a gauge");
        (g.offered, g.level, g.style) = (offered, level, style);
        c.progress.as_mut().expect("levels").priority = priority.to_vec();
    }
    e.open_offers_for_test();
    e.events().to_vec()
}

fn gauge(e: &Engine) -> &super::state::ExpertiseGauge {
    state_of(e)
        .players
        .career(PlayerIdx::SOLO)
        .expertise
        .as_ref()
        .expect("a gauge")
}

fn total(e: &Engine, effect: StatEffect) -> u32 {
    progress(e).totals[effect as usize]
}

const ATTACK: u8 = 1;
const SPEED: u8 = 4;

/// T4: a common level takes the chosen tree's best option, whatever the priority says.
#[test]
fn a_common_level_follows_the_chosen_trees_table() {
    // (style place, the pool place taken)
    for (style, place) in [(0u8, 0usize), (2, 3), (1, 2)] {
        let mut e = hammer();
        let (hp, atk) = (
            total(&e, StatEffect::MaxHp),
            total(&e, StatEffect::AttackPct),
        );
        let events = pass(&mut e, 3, 4, Some(style), &[SPEED, ATTACK]);
        let g = gauge(&e);
        let mut want = vec![0u8; 5];
        want[place] = 1;
        assert_eq!(g.taken, want, "style {style}");
        assert_eq!((g.offered, g.offer.is_none()), (4, true));
        assert_eq!(
            events
                .iter()
                .filter(|ev| matches!(ev, DomainEvent::ExpertiseOptionTaken { taken: 1, .. }))
                .count(),
            1
        );
        match style {
            0 => assert_eq!(total(&e, StatEffect::AttackPct), atk + 10),
            2 => assert_eq!(total(&e, StatEffect::MaxHp), hp + 10_000),
            _ => {
                assert_eq!(g.range_pct, 10);
                assert_eq!(total(&e, StatEffect::AttackPct), atk);
            }
        }
    }
}

/// T5: before a style is chosen the stat priority decides.
#[test]
fn before_a_style_the_priority_list_decides() {
    let mut e = hammer();
    pass(&mut e, 0, 1, None, &[SPEED, ATTACK]);
    assert_eq!(gauge(&e).taken, [0, 0, 0, 0, 1]);
    assert_eq!(total(&e, StatEffect::SpeedPct) % 5, 0);
    let mut e = hammer();
    pass(&mut e, 0, 1, None, &[ATTACK, SPEED]);
    assert_eq!(gauge(&e).taken, [1, 0, 0, 0, 0]);
}

fn weapon(json: serde_json::Value) -> WeaponExpertise {
    serde_json::from_value(json).expect("a weapon's expertise")
}

fn style(id: &str) -> serde_json::Value {
    let one = |id: &str| serde_json::json!([{ "id": id, "effect": "attack_pct", "amount": 1 }]);
    serde_json::json!({ "id": id, "effect": "attack_pct", "amount": 1,
        "upgrade_1": one("u1"), "upgrade_2": one("u2"), "capstone": one("cap") })
}

fn option(id: &str, effect: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "effect": effect, "amount": 1 })
}

/// T6: an unlisted tree, an unmatched list, a priority that names nothing the pool carries, an
/// empty priority, a range option and a tie all fall to the priority's rank, then the pool order.
#[test]
fn an_unlisted_tree_or_an_unmatched_list_falls_back_to_the_priority() {
    let e = hammer();
    let p = progress(&e).clone();
    let with = |priority: &[u8]| {
        let mut p = p.clone();
        let mut order = priority.to_vec();
        order.extend((0..5).filter(|i| !priority.contains(i)));
        p.priority = order;
        p
    };
    let w = weapon(serde_json::json!({
        "weapon": "x",
        "styles": [style("nowhere"), style("crush"), style("edge")],
        "common": [option("range", "range_pct"), option("hp", "max_hp"),
                   option("speed", "speed_pct"), option("speed_too", "speed_pct")],
    }));
    // A style whose id is not in the table: the priority [speed, attack] ranks `speed` first.
    assert_eq!(
        expertise::auto_option(&w, Some(0), &with(&[SPEED, ATTACK])),
        Some(2)
    );
    // Ties by pool order: both speed options rank alike, the first wins.
    assert_eq!(expertise::auto_option(&w, None, &with(&[SPEED])), Some(2));
    // `crush` lists attack, max_hp, attack speed: the pool has max_hp only among them.
    assert_eq!(
        expertise::auto_option(&w, Some(1), &with(&[SPEED])),
        Some(1)
    );
    // `edge` lists attack, attack speed, range: only range is in the pool.
    assert_eq!(
        expertise::auto_option(&w, Some(2), &with(&[SPEED])),
        Some(0)
    );
    // A priority naming a stat the pool lacks (xp, attack): every option ranks last or by the
    // stats it does carry; the range option, with no stat, ranks last of all.
    assert_eq!(expertise::auto_option(&w, None, &with(&[ATTACK])), Some(1));
    let only_range = weapon(serde_json::json!({
        "weapon": "x", "styles": [style("a"), style("b"), style("c")],
        "common": [option("range", "range_pct"), option("hp", "max_hp")],
    }));
    assert_eq!(
        expertise::auto_option(&only_range, None, &with(&[0])),
        Some(1)
    );
    let just_range = weapon(serde_json::json!({
        "weapon": "x", "styles": [style("a"), style("b"), style("c")],
        "common": [option("range", "range_pct")],
    }));
    assert_eq!(
        expertise::auto_option(&just_range, None, &with(&[0])),
        Some(0)
    );
    // The bundled order (max_hp first) with an empty priority list.
    let bundled = Expertise::bundled().weapon(&"giant_hammer".into()).unwrap();
    assert_eq!(
        expertise::auto_option(bundled, None, &with(&[0, 1, 2, 3, 4])),
        Some(3)
    );
}

/// T7: the pass opens no offer and draws nothing from the offer stream.
#[test]
fn a_common_level_opens_no_offer_and_draws_nothing() {
    let mut e = hammer();
    let before = state_of(&e).players.career(PlayerIdx::SOLO).offer.state();
    pass(&mut e, 0, 9, None, &[ATTACK]);
    let c = state_of(&e).players.career(PlayerIdx::SOLO);
    assert_eq!(c.offer.state(), before);
    assert!(!super::offer_open_for(state_of(&e), PlayerIdx::SOLO));
}

/// T8: levels crossed in one tick equal levels crossed in three; the ones above the table are
/// one batch.
#[test]
fn levels_crossed_in_one_tick_equal_levels_crossed_in_two() {
    let (mut one, mut three) = (hammer(), hammer());
    pass(&mut one, 0, 12, None, &[ATTACK]);
    pass(&mut three, 0, 4, None, &[ATTACK]);
    pass(&mut three, 4, 8, None, &[ATTACK]);
    pass(&mut three, 8, 12, None, &[ATTACK]);
    assert_eq!(one.state_hash(), three.state_hash());
    // 12 levels less the 4 of the table are common.
    assert_eq!(gauge(&one).taken, [8, 0, 0, 0, 0]);
    let mut batch = hammer();
    let events = pass(&mut batch, 10, 12, None, &[ATTACK]);
    let taken: Vec<u8> = events
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::ExpertiseOptionTaken { taken, .. } => Some(*taken),
            _ => None,
        })
        .collect();
    assert_eq!(taken, [2]);
}

/// T9: a run with no stat points passes the levels and takes nothing; the table names every
/// bundled tree.
#[test]
fn a_stage_without_levels_takes_nothing_and_the_table_names_every_tree() {
    let mut e = hammer();
    {
        let Run::V2 { state, .. } = e.run_mut() else {
            panic!("the live rules");
        };
        state.players.career_mut(PlayerIdx::SOLO).progress = None;
    }
    let events = pass_without_progress(&mut e);
    assert!(events.is_empty());
    let g = gauge(&e);
    assert_eq!((g.offered, g.taken.iter().sum::<u8>()), (6, 0));

    assert_eq!(TREE_COMMON.len(), 6);
    for (id, list) in TREE_COMMON {
        assert!(!list.is_empty(), "{id}");
        for (i, effect) in list.iter().enumerate() {
            assert!(!list[..i].contains(effect), "{id} repeats {effect:?}");
        }
    }
    let _ = ExpertiseEffect::MaxHp;
    for w in Expertise::bundled().weapons() {
        for st in &w.styles {
            assert!(
                TREE_COMMON.iter().any(|(id, _)| *id == st.id.as_str()),
                "{}",
                st.id
            );
        }
    }
}

fn pass_without_progress(e: &mut Engine) -> Vec<DomainEvent> {
    {
        let Run::V2 { state, .. } = e.run_mut() else {
            panic!("the live rules");
        };
        let g = state
            .players
            .career_mut(PlayerIdx::SOLO)
            .expertise
            .as_mut()
            .expect("a gauge");
        (g.offered, g.level) = (0, 6);
    }
    e.open_offers_for_test();
    e.events().to_vec()
}

// --- the tree points ------------------------------------------------------------------------------

fn world_state(e: &mut Engine) -> (&super::World, &mut State) {
    match e.run_mut() {
        Run::V2 { world, state } => (&*world, state),
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// The player 0's `TreePick` through the sim's entry point.
fn tree_pick(e: &mut Engine, option: u32) -> Result<(), TreeError> {
    let (world, state) = world_state(e);
    super::tree_pick_action(world, Expertise::bundled(), state, PlayerIdx::SOLO, option)
}

fn check_tree_pick(e: &mut Engine, option: u32) -> Result<(), TreeError> {
    let (world, state) = world_state(e);
    super::check_tree_pick(world, Expertise::bundled(), state, PlayerIdx::SOLO, option)
}

/// What the next tree pick is: its kind, its level, the points held and its choices' IDs.
fn next(e: &mut Engine) -> Option<(ExpertiseOfferKind, u32, u8, Vec<String>)> {
    let (world, state) = world_state(e);
    let g = state.players.career(PlayerIdx::SOLO).expertise.as_ref()?;
    let t = expertise::tree_next(world, Expertise::bundled(), g)?;
    Some((
        t.kind,
        t.level,
        t.held,
        t.choices.iter().map(|c| c.id.to_string()).collect(),
    ))
}

fn reach(e: &mut Engine, level: u32) {
    pass(e, 0, level, None, &[ATTACK]);
}

/// T10: the style level holds a point and stops nothing.
#[test]
fn the_style_level_holds_a_tree_point_and_nothing_stops() {
    let mut e = hammer();
    reach(&mut e, 2);
    assert_eq!(next(&mut e), None);
    reach_from(&mut e, 2, 3);
    let (kind, level, held, choices) = next(&mut e).expect("a point");
    assert_eq!((kind, level, held), (ExpertiseOfferKind::Style, 3, 1));
    assert_eq!(choices, ["crush", "quake", "bulwark"]);
    assert!(gauge(&e).offer.is_none());
    assert!(!super::offer_open_for(state_of(&e), PlayerIdx::SOLO));
}

/// Passes the levels from `offered` to `level`, keeping the style already chosen.
fn reach_from(e: &mut Engine, offered: u32, level: u32) {
    let style = gauge(e).style;
    pass(e, offered, level, style, &[ATTACK]);
}

/// T11: points wait and queue; they are taken in path order.
#[test]
fn a_point_waits_and_later_points_queue() {
    let mut e = hammer();
    reach(&mut e, 10);
    assert_eq!(next(&mut e).expect("a point").2, 4);
    let mut kinds = Vec::new();
    for held in (0..4u8).rev() {
        let (kind, _, _, _) = next(&mut e).expect("a point");
        kinds.push(kind);
        tree_pick(&mut e, 0).expect("taken");
        assert_eq!(next(&mut e).map_or(0, |n| n.2), held);
    }
    use ExpertiseOfferKind::{Capstone, Style, Upgrade};
    assert_eq!(kinds, [Style, Upgrade, Upgrade, Capstone]);
    assert_eq!(tree_pick(&mut e, 0), Err(TreeError::NoTreePoint));
}

/// T12: a pick applies the option, records the path, and makes no event and no draw.
#[test]
fn a_tree_pick_applies_the_option_and_records_the_path() {
    let mut e = hammer();
    reach(&mut e, 6);
    let (before, rng) = (
        total(&e, StatEffect::AttackPct),
        state_of(&e).players.career(PlayerIdx::SOLO).offer.state(),
    );
    e.open_offers_for_test();
    tree_pick(&mut e, 0).expect("the style");
    assert_eq!(gauge(&e).style, Some(0));
    assert_eq!(total(&e, StatEffect::AttackPct), before + 15);
    tree_pick(&mut e, 0).expect("the upgrade");
    assert_eq!(gauge(&e).path, [0]);
    assert_eq!(total(&e, StatEffect::AttackPct), before + 35);
    assert_eq!(
        state_of(&e).players.career(PlayerIdx::SOLO).offer.state(),
        rng
    );

    let mut e = hammer();
    reach(&mut e, 3);
    tree_pick(&mut e, 1).expect("quake");
    assert_eq!((gauge(&e).style, gauge(&e).range_pct), (Some(1), 20));
}

/// T13: what the rules cannot take is refused and changes nothing.
#[test]
fn check_tree_pick_refuses_what_it_cannot_take() {
    let mut e = hammer();
    reach(&mut e, 2);
    let h = e.state_hash();
    assert_eq!(check_tree_pick(&mut e, 0), Err(TreeError::NoTreePoint));
    assert_eq!(tree_pick(&mut e, 0), Err(TreeError::NoTreePoint));
    assert_eq!(e.state_hash(), h);
    reach_from(&mut e, 2, 3);
    let h = e.state_hash();
    assert_eq!(
        check_tree_pick(&mut e, 3),
        Err(TreeError::NoTreeOption {
            option: 3,
            options: 3
        })
    );
    assert_eq!(
        tree_pick(&mut e, 99),
        Err(TreeError::NoTreeOption {
            option: 99,
            options: 3
        })
    );
    assert_eq!(e.state_hash(), h);
    tree_pick(&mut e, 2).expect("the last valid option");
    reach_from(&mut e, 3, 6);
    assert_eq!(
        check_tree_pick(&mut e, 1),
        Err(TreeError::NoTreeOption {
            option: 1,
            options: 1
        })
    );
    // No gauge at all.
    let mut e = hammer();
    {
        let (_, state) = world_state(&mut e);
        state.players.career_mut(PlayerIdx::SOLO).expertise = None;
    }
    assert_eq!(check_tree_pick(&mut e, 0), Err(TreeError::NoTreePoint));
}

/// T14: an out player keeps its tree points.
#[test]
fn a_tree_point_survives_an_out_player() {
    let mut e = hammer();
    reach(&mut e, 4);
    {
        let (_, state) = world_state(&mut e);
        super::drop_offers(state, PlayerIdx::SOLO);
    }
    assert_eq!(gauge(&e).offered, 4);
    assert_eq!(next(&mut e).expect("a point").2, 1);
    tree_pick(&mut e, 0).expect("the point can be taken");
}

/// T15: the short table gives its own levels.
#[test]
fn a_short_run_table_gives_its_own_levels() {
    let held: Vec<u8> = (0..=9)
        .map(|l| expertise::tree_earned(&LevelTable::SHORT, l))
        .collect();
    assert_eq!(held, [0, 0, 1, 1, 2, 2, 3, 3, 4, 4]);
}
