//! View-edge spawns (VS1, DS109-4): a wave gives `edge { side, along? }` instead of a map point
//! and each of its enemies spawns on the spawn rectangle, 64 px outside the 840 by 360 px view
//! (±484 by ±244 px) around the camera's centre (the main character's point held inside the map,
//! as the app's camera holds it), drawn only on its on-map part; the bundled modes
//! give `edge`, and no spawn of theirs is moved by the leash on its spawn tick.
//! Positions are in 1/256 px.

use crate::common;
use stella_rain_core::version::Versions;

use common::run::{answer_all, run_input, time_limit_setup, trial_setup};
use common::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::input::Input;
use stella_rain_core::rng::{TAG_SPAWN, derive};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::{Map, Stage};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{Error, parse_and_validate};

/// The spawn rectangle's half sides: 420 + 64 and 180 + 64 px.
const EX: i64 = 484 * 256;
const EY: i64 = 244 * 256;

/// The camera's centre on one axis (px): the map's middle when it is no larger than the view,
/// else the main character's coordinate held `half` px inside the map. Worked from the app's
/// `camera_follow.gd` `_axis`, not from the code under test.
fn axis(target: i64, map: i64, half: i64) -> i64 {
    if map <= 2 * half {
        map / 2
    } else {
        target.clamp(half, map - half)
    }
}

/// The camera's centre in 1/256 px for a main character at (`x`, `y`) px on a `w` by `h` px map.
fn centre(w: i64, h: i64, x: i64, y: i64) -> (i64, i64) {
    (axis(x, w, 420) * 256, axis(y, h, 180) * 256)
}

/// A `side x side` px map with the main character starting at (`x`, `y`) px.
fn map_stage(w: i32, h: i32, x: i32, y: i32, waves: Vec<Value>) -> Stage {
    let mut stage = quiet(stage_full(18_000, None, waves, vec![]));
    stage.map = Some(Map {
        width: Fx::px(w),
        height: Fx::px(h),
        start: Some(Point {
            x: Fx::px(x),
            y: Fx::px(y),
        }),
        facing: None,
    });
    stage
}

/// A wave of `count` enemies that stand still, from tick `at_tick` one every tick, on `edge`.
fn edge_wave(at_tick: u32, count: u16, edge: Value) -> Value {
    let mut w = wave(
        at_tick,
        count,
        1,
        enemy(
            10,
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
            json!([]),
        ),
        0,
        0,
    );
    w.as_object_mut().unwrap().remove("spawn");
    w["edge"] = edge;
    w
}

/// Where the enemies stand after `ticks` steps, in entity-ID order.
fn enemy_places(stage: &Stage, ticks: u32) -> Vec<(i32, i32)> {
    let mut e = engine(stage);
    for _ in 0..ticks {
        e.step(idle());
    }
    let mut v: Vec<_> = e
        .snapshot()
        .entities
        .into_iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .map(|v| (v.id.0, v.at.x.0, v.at.y.0))
        .collect();
    v.sort();
    v.into_iter().map(|(_, x, y)| (x, y)).collect()
}

/// The first spawn of the stage under each of `n` seeds (a pile of enemies would be pushed apart).
fn first_spawns(stage: &Stage, n: u32) -> Vec<(i32, i32)> {
    (0..n)
        .map(|seed| {
            let mut stage = stage.clone();
            stage.seed = seed;
            enemy_places(&stage, 1)[0]
        })
        .collect()
}

fn place_of(side: &str, along: Option<i16>) -> (i32, i32) {
    let edge = match along {
        Some(a) => json!({ "side": side, "along": a }),
        None => json!({ "side": side }),
    };
    let stage = map_stage(4000, 4000, 2000, 2000, vec![edge_wave(0, 1, edge)]);
    enemy_places(&stage, 1)[0]
}

#[test]
fn along_fixes_the_point_on_the_side_with_no_draw() {
    assert_eq!(place_of("top", Some(0)), (512_000, 449_536));
    assert_eq!(place_of("top", Some(-1000)), (388_096, 449_536));
    // Truncation toward zero: 52,163 and not 52,164.
    assert_eq!(place_of("top", Some(421)), (564_163, 449_536));
    assert_eq!(place_of("top", Some(-421)), (459_837, 449_536));
    assert_eq!(place_of("bottom", Some(500)), (573_952, 574_464));
    assert_eq!(place_of("left", Some(-1000)), (388_096, 449_536));
    assert_eq!(place_of("right", Some(1000)), (635_904, 574_464));
}

#[test]
fn an_along_wave_draws_nothing_so_a_later_drawn_spawn_lands_where_it_would_alone() {
    let later = edge_wave(5, 1, json!({ "side": "any" }));
    let last = |waves: Vec<Value>| {
        *enemy_places(&map_stage(4000, 4000, 2000, 2000, waves), 6)
            .last()
            .unwrap()
    };
    let with = last(vec![
        edge_wave(0, 1, json!({ "side": "top", "along": 100 })),
        later.clone(),
    ]);
    assert_eq!(with, last(vec![later]));
}

/// The point of the walk over the sides of the spawn rectangle around (`px`, `py`) (1/256 px) on
/// a map of `w` by `h`: `t` from the start of the first on-map part, in the order top, right,
/// bottom, left. Top and right run `[lo, hi)`, bottom and left `(lo, hi]`: each corner is on
/// one side.
fn walk(px: i64, py: i64, w: i64, h: i64, t: i64) -> (i64, i64) {
    walk_over(px, py, Some((w, h)), t)
}

/// `walk` over the whole sides when `map` is `None`.
fn walk_over(px: i64, py: i64, map: Option<(i64, i64)>, mut t: i64) -> (i64, i64) {
    for (vertical, fixed, lo, hi) in parts(px, py, map) {
        if t < hi - lo {
            return if vertical {
                (fixed, lo + t)
            } else {
                (lo + t, fixed)
            };
        }
        t -= hi - lo;
    }
    panic!("t past the sides");
}

/// How many draw values the walk has.
fn walk_total(px: i64, py: i64, map: Option<(i64, i64)>) -> i64 {
    parts(px, py, map)
        .iter()
        .map(|(_, _, lo, hi)| hi - lo)
        .sum()
}

/// The sides with a length, as `(vertical, fixed, lo, hi)`, on the map or whole when `map` is
/// `None`.
fn parts(px: i64, py: i64, map: Option<(i64, i64)>) -> Vec<(bool, i64, i64, i64)> {
    let mut out = Vec::new();
    let sides = [
        (false, py - EY, px - EX, px + EX),
        (true, px + EX, py - EY, py + EY),
        (false, py + EY, px - EX + 1, px + EX + 1),
        (true, px - EX, py - EY + 1, py + EY + 1),
    ];
    for (vertical, fixed, lo, hi) in sides {
        let (lo, hi) = match map {
            Some((w, h)) => {
                let (across, along) = if vertical { (w, h) } else { (h, w) };
                if !(0..=across).contains(&fixed) {
                    continue;
                }
                (lo.max(0), hi.min(along))
            }
            None => (lo, hi),
        };
        if hi > lo {
            out.push((vertical, fixed, lo, hi));
        }
    }
    out
}

#[test]
fn any_is_one_draw_over_the_whole_perimeter() {
    let mut rng = derive(7, TAG_SPAWN);
    let ts: Vec<i64> = (0..4).map(|_| i64::from(rng.below(745_472))).collect();
    assert_eq!(ts, [310_920, 564_291, 324_789, 84_484]);
    let stage = map_stage(
        4000,
        4000,
        2000,
        2000,
        vec![edge_wave(0, 4, json!({ "side": "any" }))],
    );
    let want: Vec<(i32, i32)> = ts
        .iter()
        .map(|t| {
            let (x, y) = walk(512_000, 512_000, 1_024_000, 1_024_000, *t);
            (i32::try_from(x).unwrap(), i32::try_from(y).unwrap())
        })
        .collect();
    assert_eq!(enemy_places(&stage, 4), want);
}

#[test]
fn spawns_land_outside_the_21_9_view() {
    // The 840 by 360 px view is ±420 by ±180 px; the right side is 484 px out, not 384.
    let right = map_stage(
        4000,
        4000,
        2000,
        2000,
        vec![edge_wave(0, 50, json!({ "side": "right" }))],
    );
    let mut e = engine(&right);
    for _ in 0..50 {
        e.step(idle());
    }
    let places: Vec<_> = e
        .snapshot()
        .entities
        .into_iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .map(|v| v.at.x.0)
        .collect();
    assert!(!places.is_empty());
    assert!(
        places.iter().all(|x| *x == 512_000 + 484 * 256),
        "{places:?}"
    );
    let any = map_stage(
        4000,
        4000,
        2000,
        2000,
        vec![edge_wave(0, 1, json!({ "side": "any" }))],
    );
    for (x, y) in first_spawns(&any, 200) {
        let (dx, dy) = (
            (i64::from(x) - 512_000).abs(),
            (i64::from(y) - 512_000).abs(),
        );
        assert!(
            dx >= 484 * 256 || dy >= 244 * 256,
            "({x},{y}) is near the view"
        );
        assert!(
            !(dx <= 420 * 256 && dy <= 180 * 256),
            "({x},{y}) is in view"
        );
    }
}

#[test]
fn near_a_map_edge_the_spawn_rectangle_is_the_cameras_not_the_main_characters() {
    let at = |x: i32, y: i32, side: &str| {
        let stage = map_stage(
            4000,
            4000,
            x,
            y,
            vec![edge_wave(0, 1, json!({ "side": side }))],
        );
        enemy_places(&stage, 1)[0]
    };
    // Main character at (100, 2000): the camera's centre is (420, 2000).
    let (x, _) = at(100, 2000, "right");
    assert_eq!(x, (420 + 484) * 256);
    let (x, _) = at(100, 2000, "left");
    assert_eq!(x, (420 + 484) * 256);
    let (x, y) = at(100, 2000, "top");
    assert_eq!(y, (2000 - 244) * 256);
    assert!((0..=904 * 256).contains(&x), "x = {x}");
    // Main character at (3900, 100): the centre is (3580, 180).
    let (x, _) = at(3900, 100, "left");
    assert_eq!(x, (3580 - 484) * 256);
    let (_, y) = at(3900, 100, "bottom");
    assert_eq!(y, (180 + 244) * 256);
}

#[test]
fn no_spawn_near_a_map_edge_is_relocated_on_its_spawn_tick() {
    // (main character, edge, spawn point in px); `along` fixes the point with no draw. The
    // far-side spawns are 874 x or 394 y px from
    // the main character, past the reach of 840 x 360, and 484 or 244 px from the centre.
    type Case = ((i32, i32), Value, (i64, i64));
    let cases: [Case; 6] = [
        (
            (30, 2000),
            json!({ "side": "right", "along": 0 }),
            (904, 2000),
        ),
        (
            (3970, 2000),
            json!({ "side": "left", "along": 0 }),
            (3096, 2000),
        ),
        (
            (2000, 30),
            json!({ "side": "bottom", "along": 0 }),
            (2000, 424),
        ),
        (
            (2000, 3970),
            json!({ "side": "top", "along": 0 }),
            (2000, 3576),
        ),
        (
            (30, 30),
            json!({ "side": "right", "along": 1000 }),
            (904, 424),
        ),
        // Mid-map, the centre is the main character.
        (
            (2000, 2000),
            json!({ "side": "right", "along": 0 }),
            (2484, 2000),
        ),
    ];
    for ((x, y), edge, (sx, sy)) in cases {
        let stage = map_stage(4000, 4000, x, y, vec![edge_wave(0, 1, edge.clone())]);
        let mut e = engine(&stage);
        e.step(idle());
        let (cx, cy) = centre(4000, 4000, i64::from(x), i64::from(y));
        let spawn = (sx * 256, sy * 256);
        // The spawn point is on the centre's spawn rectangle.
        assert!(
            (spawn.0 - cx).abs() == EX || (spawn.1 - cy).abs() == EY,
            "({x}, {y}) {edge}"
        );
        let relocated = e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::EnemyRelocated { .. }))
            .count();
        assert_eq!(relocated, 0, "({x}, {y}) {edge}: relocated");
        assert_eq!(e.load().relocated, 0, "({x}, {y}) {edge}");
        let at = e
            .snapshot()
            .entities
            .into_iter()
            .find(|v| v.kind == EntityKind::Enemy)
            .expect("the enemy spawned")
            .at;
        assert_eq!(
            (i64::from(at.x.0), i64::from(at.y.0)),
            spawn,
            "({x}, {y}) {edge}"
        );
    }
}

#[test]
fn a_side_without_along_is_one_draw_along_it() {
    let t = i32::try_from(derive(7, TAG_SPAWN).below(124_928)).unwrap();
    let stage = map_stage(
        4000,
        4000,
        2000,
        2000,
        vec![edge_wave(0, 1, json!({ "side": "left" }))],
    );
    assert_eq!(enemy_places(&stage, 1), [(388_096, 449_536 + t)]);
}

/// What a fixed side must give.
enum Fixed {
    /// `along`: exactly this point.
    At(i32, i32),
    /// No `along`: one draw `below(len)` along the side at the coordinate `fixed`, from `lo`.
    Draw {
        vertical: bool,
        fixed: i32,
        lo: i32,
        len: u32,
    },
}

#[test]
fn the_maps_edges_and_corners_draw_only_on_the_part_on_the_map() {
    use Fixed::{At, Draw};
    // (main character px, fixed sides)
    #[allow(clippy::type_complexity)]
    let rows: Vec<((i32, i32), Vec<(&str, Option<i16>, Fixed)>)> = vec![
        // The centre is (2000, 3820): the bottom side is off the map and gives way to the top.
        (
            (2000, 3900),
            vec![
                (
                    "bottom",
                    None,
                    Draw {
                        vertical: false,
                        fixed: 915_456,
                        lo: 388_096,
                        len: 247_808,
                    },
                ),
                ("bottom", Some(500), At(573_952, 915_456)),
            ],
        ),
        // Centre (2000, 180).
        (
            (2000, 100),
            vec![
                (
                    "top",
                    None,
                    Draw {
                        vertical: false,
                        fixed: 108_544,
                        lo: 388_096,
                        len: 247_808,
                    },
                ),
                ("top", Some(500), At(573_952, 108_544)),
            ],
        ),
        // Centre (420, 2000): the left side is off the map, the right one at 420 + 484 px.
        (
            (100, 2000),
            vec![
                (
                    "left",
                    None,
                    Draw {
                        vertical: true,
                        fixed: 231_424,
                        lo: 449_536,
                        len: 124_928,
                    },
                ),
                ("left", Some(-1000), At(231_424, 449_536)),
            ],
        ),
        // Centre (3580, 2000).
        (
            (3900, 2000),
            vec![
                (
                    "right",
                    None,
                    Draw {
                        vertical: true,
                        fixed: 792_576,
                        lo: 449_536,
                        len: 124_928,
                    },
                ),
                ("right", Some(1000), At(792_576, 574_464)),
            ],
        ),
        // Centre (420, 180).
        (
            (100, 100),
            vec![
                (
                    "top",
                    None,
                    Draw {
                        vertical: false,
                        fixed: 108_544,
                        lo: 0,
                        len: 231_424,
                    },
                ),
                ("bottom", Some(-1000), At(0, 108_544)),
                ("left", Some(0), At(231_424, 46_080)),
            ],
        ),
        // Centre (3580, 180).
        ((3900, 100), vec![("right", Some(0), At(792_576, 46_080))]),
        // Centre (420, 3820).
        ((100, 3900), vec![("bottom", Some(0), At(107_520, 915_456))]),
        // Centre (3580, 3820): the left side is on the map.
        (
            (3900, 3900),
            vec![("left", Some(1000), At(792_576, 1_024_000))],
        ),
    ];
    for ((x, y), fixed) in rows {
        let at = |edge: Value| {
            enemy_places(&map_stage(4000, 4000, x, y, vec![edge_wave(0, 1, edge)]), 1)[0]
        };
        // `any` is one draw over the on-map parts, in the walk's order.
        let (px, py) = centre(4000, 4000, i64::from(x), i64::from(y));
        let total = walk_total(px, py, Some((1_024_000, 1_024_000)));
        let t = i64::from(derive(7, TAG_SPAWN).below(u32::try_from(total).unwrap()));
        let (wx, wy) = walk(px, py, 1_024_000, 1_024_000, t);
        assert_eq!(
            at(json!({ "side": "any" })),
            (i32::try_from(wx).unwrap(), i32::try_from(wy).unwrap()),
            "{x},{y}"
        );
        // And never off the map or on a side that is off it, over 200 spawns.
        let many = map_stage(
            4000,
            4000,
            x,
            y,
            vec![edge_wave(0, 1, json!({ "side": "any" }))],
        );
        for (ex, ey) in first_spawns(&many, 200) {
            assert!((0..=1_024_000).contains(&ex) && (0..=1_024_000).contains(&ey));
            let (dx, dy) = ((i64::from(ex) - px).abs(), (i64::from(ey) - py).abs());
            assert!(
                (dx == EX && dy <= EY) || (dy == EY && dx <= EX),
                "{x},{y}: ({ex},{ey}) is not on the spawn rectangle"
            );
        }
        for (side, along, want) in fixed {
            let edge = match along {
                Some(a) => json!({ "side": side, "along": a }),
                None => json!({ "side": side }),
            };
            let got = at(edge);
            match want {
                At(wx, wy) => assert_eq!(got, (wx, wy), "{x},{y} {side} {along:?}"),
                Draw {
                    vertical,
                    fixed,
                    lo,
                    len,
                } => {
                    let t = i32::try_from(derive(7, TAG_SPAWN).below(len)).unwrap();
                    let want = if vertical {
                        (fixed, lo + t)
                    } else {
                        (lo + t, fixed)
                    };
                    assert_eq!(got, want, "{x},{y} {side}");
                }
            }
        }
    }
}

#[test]
fn a_map_no_larger_than_the_rectangle_clamps_to_its_border() {
    let t = derive(7, TAG_SPAWN).below(745_472);
    assert!(t < 745_472);
    let stage = map_stage(
        400,
        400,
        200,
        200,
        vec![edge_wave(0, 1, json!({ "side": "any" }))],
    );
    let (full, w) = (102_400, 102_400);
    let places = first_spawns(&stage, 200);
    for (x, y) in &places {
        assert!(
            [0, w].contains(x) || [0, full].contains(y),
            "({x},{y}) is not on the map's border"
        );
    }
    // The draw is over the whole perimeter: the spawn of seed 7 is the walk of `t`, clamped.
    let (wx, wy) = walk_over(51_200, 51_200, None, i64::from(t));
    let clamp = |v: i64| v.clamp(0, 102_400) as i32;
    assert_eq!(places[7], (clamp(wx), clamp(wy)));
}

fn stage_json() -> Value {
    serde_json::from_str(include_str!("fixtures/stage_v0.json")).unwrap()
}

fn paths(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

fn stage_with_wave(edit: impl FnOnce(&mut Value)) -> Result<Stage, Error> {
    let mut v = stage_json();
    let w = &mut v["waves"][0];
    edit(w);
    parse_and_validate(v.to_string().as_bytes())
}

#[test]
fn a_wave_gives_spawn_or_edge_and_along_is_checked() {
    let edge = |w: &mut Value, e: Value| {
        w.as_object_mut().unwrap().remove("spawn");
        w["edge"] = e;
    };
    assert!(stage_with_wave(|w| edge(w, json!({ "side": "top" }))).is_ok());
    let both = stage_with_wave(|w| w["edge"] = json!({ "side": "top" })).unwrap_err();
    assert_eq!(paths(both), ["$.waves[0].edge"]);
    let neither = stage_with_wave(|w| {
        w.as_object_mut().unwrap().remove("spawn");
    })
    .unwrap_err();
    assert_eq!(paths(neither), ["$.waves[0].spawn"]);
    for along in [1001, -1001] {
        let e = stage_with_wave(|w| edge(w, json!({ "side": "top", "along": along }))).unwrap_err();
        assert_eq!(paths(e), ["$.waves[0].edge.along"]);
    }
    for along in [1000, -1000] {
        assert!(stage_with_wave(|w| edge(w, json!({ "side": "left", "along": along }))).is_ok());
    }
    let e = stage_with_wave(|w| edge(w, json!({ "side": "any", "along": 0 }))).unwrap_err();
    assert_eq!(paths(e), ["$.waves[0].edge.along"]);
    let e = stage_with_wave(|w| edge(w, json!({ "side": "middle" }))).unwrap_err();
    assert!(matches!(e, Error::Parse { .. }));
    let e = stage_with_wave(|w| w["around"] = json!(1000)).unwrap_err();
    assert!(matches!(e, Error::Parse { .. }));
    // The time_limit file validates the same way.
    let mut m: Value = serde_json::from_slice(bundled_file("time_limit")).unwrap();
    let w = m["waves"][0].as_object_mut().unwrap();
    w.remove("edge");
    let e = TimeLimit::parse(m.to_string().as_bytes(), Versions::CURRENT)
        .map(|_| ())
        .unwrap_err();
    assert_eq!(paths(e), ["$.waves[0].spawn"]);
}

/// FNV-1a 64 of `bytes`, for a pin.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn a_stage_with_only_spawn_points_serialises_as_before() {
    let bytes = include_bytes!("fixtures/stage_v0.json");
    let stage = parse_and_validate(bytes).unwrap();
    let compact = serde_json::to_string(&stage).unwrap();
    assert_eq!((compact.len(), fnv(compact.as_bytes())), PINNED_STAGE_V0);
}

/// The compact form's length and FNV-1a 64, taken on main before VS1; re-taken at the freeze
/// of version 1, when the fixture's two versions became 1 (same length, the rest unchanged).
const PINNED_STAGE_V0: (usize, u64) = (3294, 10_467_257_119_685_663_348);

/// Plays `ticks` ticks of `setup` with `input`, every offer answered, and returns how many
/// enemies appeared and how many of them the leash moved on the tick they appeared.
fn spawns_and_relocations(
    setup: &RunSetup,
    ticks: u32,
    input: impl Fn(&Engine, u32) -> Input,
) -> (usize, usize) {
    let mut e = Engine::start(setup).unwrap();
    let ids = |e: &Engine| -> BTreeSet<u32> {
        e.snapshot()
            .entities
            .iter()
            .filter(|v| v.kind == EntityKind::Enemy)
            .map(|v| v.id.0)
            .collect()
    };
    let mut known = ids(&e);
    let (mut spawned, mut moved) = (0, 0);
    let (mut offers, mut out) = (0, Vec::new());
    for t in 0..ticks {
        if e.outcome() != Outcome::Running {
            break;
        }
        answer_all(&mut e, &mut offers, &mut out);
        let i = input(&e, t);
        e.step(i);
        let now = ids(&e);
        spawned += now.difference(&known).count();
        for ev in e.events() {
            if let DomainEvent::EnemyRelocated { entity } = ev {
                if !known.contains(&entity.0) {
                    moved += 1;
                }
            }
        }
        known = now;
    }
    (spawned, moved)
}

#[test]
fn no_bundled_mode_spawn_is_relocated_on_its_spawn_tick() {
    type Case = (&'static str, RunSetup, u32, usize);
    let cases: [Case; 3] = [
        ("trial", trial_setup(), 5_400, 16),
        (
            "tutorial",
            RunSetup {
                mode: "tutorial".into(),
                ..trial_setup()
            },
            10_800,
            8,
        ),
        ("time_limit", time_limit_setup(), 18_000, 150),
    ];
    for (name, setup, ticks, least) in cases {
        let standing = spawns_and_relocations(&setup, ticks, |_, _| Input::default());
        let bot = spawns_and_relocations(&setup, ticks, run_input);
        for (how, (spawned, moved)) in [("standing", standing), ("bot", bot)] {
            assert_eq!(moved, 0, "{name} {how}: {moved} of {spawned} spawns moved");
        }
        // Not a vacuous pass: the more eventful of the two plays spawns at least `least`.
        assert!(
            standing.0.max(bot.0) >= least,
            "{name}: {} spawns",
            standing.0.max(bot.0)
        );
    }
}

#[test]
fn every_bundled_mode_gives_edge() {
    let tops: [(&str, &[i64]); 2] = [
        ("trial", &[-423, 423, -254, -586, 847]),
        ("tutorial", &[-423, 423, -254]),
    ];
    for (name, alongs) in tops {
        let v: Value = serde_json::from_slice(bundled_file(name)).unwrap();
        let mut got = Vec::new();
        for w in v["waves"].as_array().unwrap() {
            assert!(w.get("spawn").is_none(), "{name}: a wave keeps `spawn`");
            assert_eq!(w["edge"]["side"], "top", "{name}");
            got.push(w["edge"]["along"].as_i64().unwrap());
        }
        got.sort();
        let mut want = alongs.to_vec();
        want.sort();
        assert_eq!(got, want, "{name}");
    }
    let v: Value = serde_json::from_slice(bundled_file("time_limit")).unwrap();
    assert_eq!(v["waves"].as_array().unwrap().len(), 9);
    for w in v["waves"].as_array().unwrap() {
        assert!(w.get("spawn").is_none() && w.get("around").is_none());
        assert_eq!(w["edge"], json!({ "side": "any" }));
    }
}

/// A `time_limit` mode with no waves, standing elites on `elite_cap`, and a run of 5 minutes.
fn elite_mode(cap: u32) -> TimeLimit {
    let mut v: Value = serde_json::from_slice(bundled_file("time_limit")).unwrap();
    v["waves"] = json!([]);
    v["length_ticks"] = json!(5 * 3600);
    v["boss_ticks"] = json!([5 * 3600]);
    v["elite_cap"] = json!(cap);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    for t in v["elite_types"].as_array_mut().unwrap() {
        t["enemy"]["movement"] = json!({ "type": "straight", "vx": 0, "vy": 0 });
        t["enemy"]["rules"] = json!([]);
    }
    TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap_or_else(|e| panic!("{e}"))
}

/// Stands for 5 minutes; where each elite is from the main character on its spawn tick.
fn elite_places(cap: u32) -> Vec<(i32, i32)> {
    let mut e =
        Engine::start_time_limit(&time_limit_setup(), RunContent::bundled(), &elite_mode(cap))
            .unwrap();
    let mut out = Vec::new();
    while e.tick() < 5 * 3600 - 1 {
        e.step(Input::default());
        let me = e.snapshot().players[0].at;
        for ev in e.events() {
            if let DomainEvent::EliteSpawned { elite, .. } = ev {
                let at = e
                    .snapshot()
                    .entities
                    .into_iter()
                    .find(|v| v.id == *elite)
                    .unwrap()
                    .at;
                out.push((at.x.0 - me.x.0, at.y.0 - me.y.0));
            }
        }
    }
    out
}

#[test]
fn elite_minutes_spawn_on_the_spawn_rectangle() {
    let places = elite_places(32);
    assert_eq!(places.len(), 4, "minutes 1 to 4; the boss time has none");
    for (dx, dy) in places {
        let (dx, dy) = (i64::from(dx).abs(), i64::from(dy).abs());
        assert!(
            (dx == 123_904 && dy <= 62_464) || (dy == 62_464 && dx <= 123_904),
            "an elite at ({dx}, {dy}) from the main character is not on the spawn rectangle"
        );
    }
}
