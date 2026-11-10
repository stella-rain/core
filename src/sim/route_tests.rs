//! The players that are up steer the world (ADR-SIM-7 points 5 and 6, ADR-SIM-8): the leash,
//! the spawn edge, the boss's arrival and the ranged tokens read their bounding box; targets
//! and owner-keyed rules read the player they name. A downed or out player is present (the
//! party factor) but not up.

use super::attacks::in_view;
use super::horde::arrival_point;
use super::leash;
use super::select::Up;
use super::state::{PlayerIdx, Players, State};
use super::{Area, World, edge_point, view_centre};
use crate::engine::{Engine, Run};
use crate::fixed::{Fx, Point};
use crate::rng::SplitMix64;
use crate::run_setup::RunSetup;
use crate::stage::{Map, Side, ViewEdge};

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

fn px(x: i32, y: i32) -> Point {
    Point {
        x: Fx::px(x),
        y: Fx::px(y),
    }
}

fn big_map() -> Map {
    Map {
        width: Fx::px(4000),
        height: Fx::px(4000),
        start: None,
        facing: None,
    }
}

/// A three-player run, stepped until an enemy stands, on a 4000 by 4000 px map; the three
/// players are at `at`, all up.
fn party(at: [Point; 3]) -> (World, State) {
    let setup = RunSetup {
        seed: 20_261_104,
        ..RunSetup::default()
    };
    let mut e = Engine::start_party(&setup, 3).expect("a party");
    for _ in 0..1200 {
        if matches!(e.run(), Run::V2 { state, .. } if !state.enemies.is_empty()) {
            break;
        }
        e.step(&Default::default());
    }
    let Run::V2 { world, state } = e.run() else {
        panic!("the live rules")
    };
    let (mut world, mut s) = (world.clone(), state.clone());
    assert!(!s.enemies.is_empty(), "an enemy to move");
    world.stage.map = Some(big_map());
    for (i, p) in at.into_iter().enumerate() {
        s.players[slot(u8::try_from(i).unwrap())].at = p;
    }
    s.runs.clear();
    (world, s)
}

fn down(s: &mut State, i: u8) {
    s.players[slot(i)].hp = 0;
}

fn out(s: &mut State, i: u8) {
    s.players[slot(i)].hp = 0;
    s.players[slot(i)].out = true;
}

#[test]
fn up_and_targets_leave_out_the_downed_and_the_out() {
    let (_, mut s) = party([px(100, 100); 3]);
    assert_eq!((s.players.up_mask(), s.players.targets()), (0b111, 0b111));
    down(&mut s, 1);
    assert_eq!((s.players.up_mask(), s.players.targets()), (0b101, 0b101));
    out(&mut s, 2);
    assert_eq!((s.players.up_mask(), s.players.targets()), (0b001, 0b001));
    assert_eq!(
        s.players.len(),
        3,
        "present still counts for the party factor"
    );
    down(&mut s, 0);
    assert_eq!(s.players.up_mask(), 0);
    assert_eq!(s.players.targets(), 0b111, "no one up: every present slot");
}

#[test]
fn a_solo_run_that_is_down_still_targets_slot_zero() {
    let p = super::state::PlayerState {
        hp: 0,
        ..party([px(1, 1); 3]).1.players[PlayerIdx::SOLO].clone()
    };
    let players = Players::with_mask(0b001, p);
    assert_eq!((players.up_mask(), players.targets()), (0, 0b001));
}

#[test]
fn the_area_is_the_box_of_the_up_players_view_centres() {
    let map = big_map();
    let (_, mut s) = party([px(100, 100), px(1000, 100), px(3000, 3000)]);
    down(&mut s, 2);
    // View centres x 420 and 1000 (the view's half width holds the first inside the map), y 180.
    let a = Area::of_view(&map, &s.players);
    assert_eq!(a.centre, px(710, 180));
    assert_eq!((a.half_x, a.half_y), (i64::from(Fx::px(290).0), 0));

    down(&mut s, 1);
    let a = Area::of_view(&map, &s.players);
    assert_eq!(a.centre, view_centre(&map, px(100, 100)));
    assert_eq!((a.half_x, a.half_y), (0, 0));
}

fn tuning(w: &World) -> (i64, i64, i64) {
    (
        i64::from(w.tuning.leash_reach_x.0),
        i64::from(w.tuning.leash_land_x.0),
        i64::from(w.tuning.spawn_edge_x.0),
    )
}

#[test]
fn the_leash_reaches_and_lands_around_the_box() {
    let (w, mut s) = party([px(1000, 2000), px(2000, 2000), px(0, 0)]);
    s.players =
        Players::of(vec![s.players[slot(0)].clone(), s.players[slot(1)].clone()]).expect("two");
    s.enemies.truncate(1);
    assert!(w.defs.get(s.enemies[0].def).leash, "a roaming enemy");
    s.enemies[0].elite = None;
    s.enemies[0].leave = None;
    let run = |s: &State, x: i32| {
        let mut s = s.clone();
        s.enemies[0].at = px(x, 2000);
        let mut events = Vec::new();
        let moved = leash::relocate(&w, &mut s, &mut events);
        (moved, s.enemies[0].at)
    };
    // The box is centred at 1500 with half 500: the reach is 840 + 500.
    assert_eq!(run(&s, 2800), (0, px(2800, 2000)));
    assert_eq!(run(&s, 2900), (1, px(1500 - 984, 2000)));
    // With slot 1 downed the box is slot 0's view centre alone.
    down(&mut s, 1);
    assert_eq!(run(&s, 1800), (0, px(1800, 2000)));
    assert_eq!(run(&s, 2800), (1, px(1000 - 484, 2000)));
}

#[test]
fn the_spawn_edge_grows_with_the_box_and_the_draws_do_not_change() {
    let (w, _) = party([px(0, 0); 3]);
    let (_, _, edge_x) = tuning(&w);
    let right = ViewEdge {
        side: Side::Right,
        along: Some(0),
    };
    let wide = Area {
        centre: px(1500, 2000),
        half_x: i64::from(Fx::px(500).0),
        half_y: 0,
    };
    let one = Area {
        centre: px(1000, 2000),
        half_x: 0,
        half_y: 0,
    };
    let mut rng = SplitMix64::new(7);
    let at = edge_point(&w, wide, &mut rng, right);
    assert_eq!(
        i64::from(at.x.0),
        i64::from(Fx::px(1500).0) + edge_x + i64::from(Fx::px(500).0)
    );
    let at = edge_point(&w, one, &mut rng, right);
    assert_eq!(i64::from(at.x.0), i64::from(Fx::px(1000).0) + edge_x);
    assert_eq!(rng, SplitMix64::new(7), "`along` draws nothing");

    let any = ViewEdge {
        side: Side::Any,
        along: None,
    };
    for area in [wide, one] {
        let mut rng = SplitMix64::new(7);
        edge_point(&w, area, &mut rng, any);
        let mut once = SplitMix64::new(7);
        once.next_u64();
        assert_eq!(rng, once, "Any draws once");
        let mut rng = SplitMix64::new(7);
        edge_point(
            &w,
            area,
            &mut rng,
            ViewEdge {
                side: Side::Top,
                along: None,
            },
        );
        assert_eq!(rng, once, "an unfixed side draws once");
    }
}

#[test]
fn the_boss_arrives_over_the_up_players() {
    let (w, mut s) = party([px(1000, 1500), px(2000, 1000), px(3000, 400)]);
    down(&mut s, 2);
    let above = w.tuning.boss_arrival_above;
    let at = arrival_point(&w, &s.players);
    assert_eq!(at, px(1500, 1000 - above / 256));
    down(&mut s, 1);
    assert_eq!(arrival_point(&w, &s.players), px(1000, 1500 - above / 256));
}

#[test]
fn the_tokens_see_the_view_of_the_box_and_the_nearest_up_player() {
    let area = Area {
        centre: px(1500, 2000),
        half_x: i64::from(Fx::px(500).0),
        half_y: 0,
    };
    assert!(in_view(px(2420, 2000), area));
    assert!(!in_view(px(2421, 2000), area));
    let (_, mut s) = party([px(0, 0), px(1000, 2000), px(2000, 2000)]);
    s.players = Players::of(vec![
        s.players[slot(1)].clone(),
        s.players[slot(1)].clone(),
        s.players[slot(2)].clone(),
    ])
    .expect("three");
    // Slots at x 1000 and 1000 and 2000: the least distance is to the nearer player.
    let d = |s: &State, x| i64::try_from(Up::of(&s.players).least_d2(px(x, 2000))).unwrap();
    assert_eq!(d(&s, 2100), i64::from(Fx::px(100).0).pow(2));
    assert_eq!(d(&s, 1300), i64::from(Fx::px(300).0).pow(2));
    down(&mut s, 2);
    assert_eq!(d(&s, 2100), i64::from(Fx::px(1100).0).pow(2));
}

// ---------------------------------------------------------------------------------------------
// Targeting (ADR-SIM-7 point 6): the most aggro among the up players, else the nearest, ties to
// the lowest slot; an enemy that no aggro drives now and then takes a random up player.
// ---------------------------------------------------------------------------------------------

use super::agents;
use super::attacks;
use super::entity::Who;
use super::select::{self, chase};
use crate::behaviour::Selector;
use crate::id::EntityId;
use crate::input::Input;
use crate::pack::ContentFiles;
use crate::run_content::RunContent;
use crate::time_limit::TimeLimit;
use crate::version::Versions;
use serde_json::json;

/// The slot `chase` picks at `from`, with no random switch.
fn nearest(s: &State, x: i32) -> u8 {
    let up = Up::of(&s.players);
    chase(s, &up, px(x, 0), 0, false).get().try_into().unwrap()
}

/// A party whose players stand on the x axis at the given px, with no aggro.
fn line(xs: [i32; 3]) -> State {
    let (_, mut s) = party([px(0, 0); 3]);
    s.companions.clear();
    for (i, x) in xs.into_iter().enumerate() {
        let p = &mut s.players[slot(u8::try_from(i).unwrap())];
        p.at = px(x, 0);
        p.aggro = 0;
    }
    s
}

#[test]
fn the_chase_target_is_the_nearest_up_player_ties_to_the_lowest_slot() {
    let s = line([100, 400, 5000]);
    assert_eq!(nearest(&s, 0), 0);
    assert_eq!(nearest(&s, 300), 1, "100 against 200");
    assert_eq!(nearest(&s, 250), 0, "an equal distance: the lowest slot");
}

#[test]
fn a_rejoined_slot_keeps_its_place_in_a_tie() {
    let mut s = line([9000, 100, 300]);
    s.players.set_generation_for_test(slot(1), 1);
    assert_eq!(nearest(&s, 200), 1);
    let enemy = Who::Enemy(0);
    s.enemies[0].at = px(200, 0);
    let pick = select::select(&s, enemy, &Selector::NearestAlly);
    assert_eq!(
        pick,
        Some(Who::Player(slot(1))),
        "the slot, not the generation's number"
    );
}

#[test]
fn a_downed_player_is_never_the_target_and_a_wipe_falls_back_to_the_lowest_slot() {
    let mut s = line([100, 400, 5000]);
    down(&mut s, 0);
    assert_eq!(nearest(&s, 0), 1);
    out(&mut s, 1);
    assert_eq!(nearest(&s, 0), 2);
    down(&mut s, 2);
    assert_eq!(nearest(&s, 0), 0, "no one up: the lowest present slot");
}

#[test]
fn the_most_aggro_among_the_up_players_decides() {
    let mut s = line([100, 400, 5000]);
    s.players[slot(1)].aggro = 500;
    assert_eq!(nearest(&s, 0), 1);
    s.players[slot(0)].aggro = 500;
    assert_eq!(nearest(&s, 0), 0, "equal aggro: the nearer");
    s.players[slot(2)].aggro = 900;
    down(&mut s, 2);
    assert_eq!(nearest(&s, 0), 0, "a downed player's aggro is ignored");
}

fn engine_with_follow_enemy() -> Engine {
    let mut tl: serde_json::Value =
        serde_json::from_slice(ContentFiles::bundled_file("time_limit")).unwrap();
    tl["waves"] = json!([{
        "at_tick": 1,
        "enemy": {
            "base": "bat", "hp": 9_000_000, "radius": 2048,
            "movement": { "type": "follow", "distance": 0, "speed": 1024 },
            "rules": []
        },
        "spawn": { "x": 524032, "y": 524288 },
        "count": 1,
        "every_ticks": 0
    }]);
    let mode = TimeLimit::parse(tl.to_string().as_bytes(), Versions::CURRENT).unwrap();
    let content = RunContent::bundled().clone().with_time_limit(mode);
    let setup = RunSetup {
        mode: crate::id::ContentId::from("time_limit"),
        seed: 42,
        ..Default::default()
    };
    let mut e = Engine::start_party_with(&setup, &content, 3).unwrap();
    for _ in 0..3 {
        e.step(&[Input::default(), Input::default(), Input::default()]);
    }
    e
}

#[test]
fn a_following_enemy_walks_to_the_player_that_is_up() {
    let e = engine_with_follow_enemy();
    let Run::V2 { world, state } = e.run() else {
        panic!("the live rules")
    };
    let (world, mut s) = (world.clone(), state.clone());
    assert_eq!(s.enemies.len(), 1);
    let home = s.enemies[0].at;
    // Slot 0 is downed far to the right, slot 1 stands straight below the enemy.
    s.players[slot(0)].at = Point {
        x: home.x.saturating_add(Fx::px(1000)),
        y: home.y,
    };
    s.players[slot(1)].at = Point {
        x: home.x,
        y: home.y.saturating_add(Fx::px(100)),
    };
    s.players[slot(2)].at = s.players[slot(1)].at;
    down(&mut s, 0);
    let mut events = Vec::new();
    agents::act(&world, &mut s, &mut events);
    let now = s.enemies[0].at;
    assert!(now.y.0 > home.y.0, "it walks toward slot 1");
    assert_eq!(now.x, home.x, "and not toward the downed slot 0");
}

#[test]
fn near_target_reads_the_chase_target() {
    let mut s = line([0, 1000, 9000]);
    let near = crate::behaviour::Condition::NearTarget {
        distance: Fx::px(100),
    };
    let holds = |s: &State| agents::condition_holds(s, Who::Enemy(0), &near);
    s.enemies[0].at = px(950, 0);
    assert!(holds(&s), "50 px from slot 1, its nearest");
    // The enemy stands by slot 0, but slot 1 holds the aggro: its target is far.
    s.enemies[0].at = px(20, 0);
    assert!(holds(&s));
    s.players[slot(1)].aggro = 10;
    assert!(!holds(&s), "near only the player that is not its target");
}

// ---- the random switch -------------------------------------------------------------------

fn two_up() -> State {
    let mut s = line([0, 300, 9000]);
    down(&mut s, 2);
    s
}

fn pick(s: &State, from_x: i32, key: u32, random: bool) -> u8 {
    let up = Up::of(&s.players);
    chase(s, &up, px(from_x, 0), key, random)
        .get()
        .try_into()
        .unwrap()
}

#[test]
fn the_random_switch_reaches_both_up_players_and_is_a_pure_roll() {
    let mut s = two_up();
    s.tick = 4000;
    let mut hits = [0u32; 2];
    let mut off_nearest = 0;
    for key in 0..2000 {
        // The enemy stands at x = 10, nearest to slot 0.
        let p = pick(&s, 10, key, true);
        hits[usize::from(p)] += 1;
        off_nearest += u32::from(p != 0);
        assert_eq!(
            p,
            pick(&s, 10, key, true),
            "the same arguments, the same slot"
        );
        assert_eq!(pick(&s, 10, key, false), 0, "no roll: the nearest");
    }
    // 30 percent of the keys roll, half of them to the player that is not the nearest.
    assert!(
        hits[0] >= 200 && (200..400).contains(&hits[1]),
        "both are reached: {hits:?}"
    );
    assert_eq!(off_nearest, hits[1]);
}

#[test]
fn an_enemy_keeps_its_random_target_for_the_epoch() {
    let mut s = two_up();
    let key = 7u32;
    // The epoch starts where (tick + key % 180) is a multiple of 180.
    let start = 180 * 10 - key % 180;
    let seen = |s: &mut State, tick: u32| {
        s.tick = tick;
        pick(s, 10, key, true)
    };
    let first = seen(&mut s, start);
    for t in start..start + 180 {
        assert_eq!(seen(&mut s, t), first, "tick {t}");
    }
    // Some later epoch differs for this key (a 30% roll over two players).
    assert!((1..200).any(|k| seen(&mut s, start + 180 * k) != first));
}

#[test]
fn aggro_or_a_single_up_player_stops_the_switch() {
    let mut s = two_up();
    s.tick = 4000;
    s.players[slot(1)].aggro = 5;
    for key in 0..200 {
        assert_eq!(pick(&s, 10, key, true), 1, "the most aggro holds the enemy");
    }
    s.players[slot(1)].aggro = 0;
    down(&mut s, 1);
    for key in 0..200 {
        assert_eq!(pick(&s, 10, key, true), 0, "one player up");
    }
}

#[test]
fn the_roll_adds_no_draw_and_follows_the_seed() {
    let mut s = two_up();
    let before = s.rng.clone();
    for t in 0..1000 {
        s.tick = t;
        pick(&s, 10, t, true);
    }
    assert_eq!(s.rng, before, "no stream moved");
    let run = |seed: u64| {
        let mut s = two_up();
        s.crit_seed = seed;
        (0..600)
            .map(|t| {
                s.tick = t;
                pick(&s, 10, 3, true)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(5), run(5));
    assert_ne!(run(5), run(6), "a different seed differs somewhere");
}

// ---- owner-keyed -----------------------------------------------------------------------------

#[test]
fn aggro_is_raised_for_any_present_slot_and_ignores_an_absent_one() {
    let mut s = line([0, 0, 0]);
    for generation in [0u8, 1] {
        s.players.set_generation_for_test(slot(2), generation);
        s.players[slot(2)].aggro = 0;
        s.raise_aggro(s.players.id(slot(2)), 40);
        assert_eq!(s.players[slot(2)].aggro, 40);
    }
    s.raise_aggro(s.players.id(slot(2)), u32::MAX);
    assert_eq!(s.players[slot(2)].aggro, super::MAX_AGGRO);
    // A number that is no present slot's changes nothing.
    let stale = slot(1).id_at(9);
    let snapshot: Vec<u32> = s.players.iter().map(|p| p.aggro).collect();
    s.raise_aggro(stale, 10);
    assert_eq!(
        s.players.iter().map(|p| p.aggro).collect::<Vec<_>>(),
        snapshot
    );
}

#[test]
fn every_present_player_fades_and_ages() {
    let mut s = line([0, 0, 0]);
    down(&mut s, 1);
    for i in 0..3 {
        s.players[slot(i)].aggro = 500;
    }
    let mut events = Vec::new();
    super::entity::tick_statuses_and_aggro(&mut s, 100, &mut events);
    for i in 0..3 {
        assert_eq!(s.players[slot(i)].aggro, 400, "slot {i}");
    }
}

#[test]
fn aim_assist_starts_from_the_player_it_is_for() {
    let (world, mut s) = party([px(0, 0), px(3000, 0), px(0, 0)]);
    let range = 200 * 256;
    s.boss = None;
    s.enemies.truncate(1);
    s.enemies[0].at = px(3050, 0);
    s.enemies[0].hp = s.enemies[0].max_hp.max(1);
    assert_eq!(
        select::assist_target(&world, &s, slot(1), range),
        Some(s.enemies[0].id)
    );
    assert_eq!(select::assist_target(&world, &s, slot(0), range), None);
    let targets = |me: u8| {
        let (mut kept, mut out) = (Vec::new(), Vec::new());
        select::assist_targets(&world, &s, slot(me), range, 3, &mut kept, &mut out);
        out
    };
    assert_eq!(targets(1), vec![s.enemies[0].id]);
    assert!(targets(0).is_empty());
}

#[test]
fn a_combo_run_belongs_to_any_present_player() {
    let mut e = Engine::start_party(
        &RunSetup {
            seed: 3,
            ..RunSetup::default()
        },
        3,
    )
    .unwrap();
    let main = Input {
        main: true,
        ..Input::default()
    };
    while !matches!(e.run(), Run::V2 { state, .. } if !state.runs.is_empty()) {
        e.step(&[main, Input::default(), Input::default()]);
    }
    let Run::V2 { state, .. } = e.run() else {
        unreachable!()
    };
    let s = state.clone();
    let mut run = s.runs[0].clone();
    run.slot = super::player::COMBO_SLOT;
    run.owner = s.players.id(slot(2));
    assert!(attacks::is_combo(&s, &run));
    run.owner = s.players.id(slot(0));
    assert!(attacks::is_combo(&s, &run));
    run.owner = EntityId(s.companions.first().map_or(7, |c| c.id.0));
    assert!(!attacks::is_combo(&s, &run), "a companion's");
}

#[test]
fn up_means_is_up_not_just_hit_points() {
    let (_, mut s) = party([px(100, 100); 3]);
    // Out with hit points left (an out flag set by some other path) is not up.
    s.players[slot(1)].out = true;
    s.players[slot(1)].hp = s.players[slot(0)].hp.max(1);
    assert!(s.players[slot(1)].hp > 0);
    assert_eq!((s.players.up_mask(), s.players.targets()), (0b101, 0b101));
    // Downed (no hit points, not out) is not up either.
    s.players[slot(2)].hp = 0;
    assert!(s.players[slot(2)].is_downed());
    assert_eq!(s.players.up_mask(), 0b001);
    assert_eq!(nearest(&s, 0), 0);
}
