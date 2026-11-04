//! The players that are up steer the world (ADR-SIM-7 points 5 and 6, ADR-SIM-8): the leash,
//! the spawn edge, the boss's arrival and the ranged tokens read their bounding box; targets
//! and owner-keyed rules read the player they name. A downed or out player is present (the
//! party factor) but not up.

use super::attacks::{in_view, least_d2};
use super::horde::arrival_point;
use super::leash;
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
        pick_stats: true,
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
    let mut run = |s: &State, x: i32| {
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
    let d = |s: &State, x| least_d2(&s.players, px(x, 2000));
    assert_eq!(d(&s, 2100), i64::from(Fx::px(100).0).pow(2));
    assert_eq!(d(&s, 1300), i64::from(Fx::px(300).0).pow(2));
    down(&mut s, 2);
    assert_eq!(d(&s, 2100), i64::from(Fx::px(1100).0).pow(2));
}
