//! View-edge spawns (VS1, DS109-4): a wave gives `edge { side, along? }` instead of a map point
//! and each of its enemies spawns on the spawn rectangle, 64 px outside the 640 by 360 px view
//! (±384 by ±244 px) around the main character, drawn only on its on-map part; the bundled modes
//! give `edge`, and no spawn of theirs is moved by the leash on its spawn tick.
//! Positions are in 1/256 px.

use crate::common;

use common::run::{answer_all, run_input, time_limit_setup, trial_setup};
use common::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::{Fx, Point};
use stella_rain_core::input::Input;
use stella_rain_core::rng::{TAG_SPAWN, derive};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::{Map, Stage};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{Error, parse_and_validate};

/// The spawn rectangle's half sides: 320 + 64 and 180 + 64 px.
const EX: i64 = 384 * 256;
const EY: i64 = 244 * 256;

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
    assert_eq!(place_of("top", Some(-1000)), (413_696, 449_536));
    // Truncation toward zero: 41,582 and not 41,583.
    assert_eq!(place_of("top", Some(423)), (553_582, 449_536));
    assert_eq!(place_of("top", Some(-423)), (470_418, 449_536));
    assert_eq!(place_of("bottom", Some(500)), (561_152, 574_464));
    assert_eq!(place_of("left", Some(-1000)), (413_696, 449_536));
    assert_eq!(place_of("right", Some(1000)), (610_304, 574_464));
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
/// bottom, left.
fn walk(px: i64, py: i64, w: i64, h: i64, mut t: i64) -> (i64, i64) {
    let sides = [
        (false, py - EY, px - EX, px + EX, h),
        (true, px + EX, py - EY, py + EY, w),
        (false, py + EY, px - EX, px + EX, h),
        (true, px - EX, py - EY, py + EY, w),
    ];
    for (vertical, fixed, lo, hi, limit) in sides {
        // `limit` is the extent of the axis the side's fixed coordinate lies on.
        let (fixed_limit, moving_limit) = if vertical { (w, h) } else { (h, w) };
        let _ = limit;
        if !(0..=fixed_limit).contains(&fixed) {
            continue;
        }
        let (lo, hi) = (lo.max(0), hi.min(moving_limit));
        if hi <= lo {
            continue;
        }
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

#[test]
fn any_is_one_draw_over_the_whole_perimeter() {
    let mut rng = derive(7, TAG_SPAWN);
    let ts: Vec<i64> = (0..4).map(|_| i64::from(rng.below(643_072))).collect();
    assert_eq!(ts, [237_192, 68_675, 263_349, 281_092]);
    let stage = map_stage(
        4000,
        4000,
        2000,
        2000,
        vec![edge_wave(0, 4, json!({ "side": "any" }))],
    );
    let want = [
        (610_304, 490_120),
        (482_371, 449_536),
        (610_304, 516_277),
        (610_304, 534_020),
    ];
    assert_eq!(enemy_places(&stage, 4), want);
    for (t, p) in ts.iter().zip(want) {
        let (x, y) = walk(512_000, 512_000, 1_024_000, 1_024_000, *t);
        assert_eq!((x as i32, y as i32), p);
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
    assert_eq!(enemy_places(&stage, 1), [(413_696, 449_536 + t)]);
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
    // (main character px, `any` total, fixed sides)
    #[allow(clippy::type_complexity)]
    let rows: Vec<((i32, i32), u32, Vec<(&str, Option<i16>, Fixed)>)> = vec![
        (
            (2000, 3900),
            372_736,
            vec![
                (
                    "bottom",
                    None,
                    Draw {
                        vertical: false,
                        fixed: 935_936,
                        lo: 413_696,
                        len: 196_608,
                    },
                ),
                ("bottom", Some(500), At(561_152, 935_936)),
            ],
        ),
        (
            (2000, 100),
            372_736,
            vec![
                (
                    "top",
                    None,
                    Draw {
                        vertical: false,
                        fixed: 88_064,
                        lo: 413_696,
                        len: 196_608,
                    },
                ),
                ("top", Some(500), At(561_152, 88_064)),
            ],
        ),
        (
            (100, 2000),
            372_736,
            vec![
                (
                    "left",
                    None,
                    Draw {
                        vertical: true,
                        fixed: 123_904,
                        lo: 449_536,
                        len: 124_928,
                    },
                ),
                ("left", Some(-1000), At(123_904, 449_536)),
            ],
        ),
        (
            (3900, 2000),
            372_736,
            vec![
                (
                    "right",
                    None,
                    Draw {
                        vertical: true,
                        fixed: 900_096,
                        lo: 449_536,
                        len: 124_928,
                    },
                ),
                ("right", Some(1000), At(900_096, 574_464)),
            ],
        ),
        (
            (100, 100),
            211_968,
            vec![
                (
                    "top",
                    None,
                    Draw {
                        vertical: false,
                        fixed: 88_064,
                        lo: 0,
                        len: 123_904,
                    },
                ),
                ("bottom", Some(-1000), At(0, 88_064)),
                ("left", Some(0), At(123_904, 25_600)),
            ],
        ),
        (
            (3900, 100),
            211_968,
            vec![("right", Some(0), At(900_096, 25_600))],
        ),
        (
            (100, 3900),
            211_968,
            vec![("bottom", Some(0), At(25_600, 935_936))],
        ),
        (
            (3900, 3900),
            211_968,
            vec![("left", Some(1000), At(900_096, 1_024_000))],
        ),
    ];
    for ((x, y), total, fixed) in rows {
        let at = |edge: Value| {
            enemy_places(&map_stage(4000, 4000, x, y, vec![edge_wave(0, 1, edge)]), 1)[0]
        };
        // `any` is one draw over the on-map parts, in the walk's order.
        let t = i64::from(derive(7, TAG_SPAWN).below(total));
        let (px, py) = (i64::from(x) * 256, i64::from(y) * 256);
        let (wx, wy) = walk(px, py, 1_024_000, 1_024_000, t);
        assert_eq!(
            at(json!({ "side": "any" })),
            (wx as i32, wy as i32),
            "{x},{y}"
        );
        // And never off the map or on a side that is off it, over 200 spawns.
        let many = map_stage(
            4000,
            4000,
            x,
            y,
            vec![edge_wave(0, 200, json!({ "side": "any" }))],
        );
        for (ex, ey) in enemy_places(&many, 200) {
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
    let t = derive(7, TAG_SPAWN).below(643_072);
    assert!(t < 643_072);
    let stage = map_stage(
        400,
        400,
        200,
        200,
        vec![edge_wave(0, 200, json!({ "side": "any" }))],
    );
    let (full, w) = (102_400, 102_400);
    let places = enemy_places(&stage, 200);
    assert_eq!(places.len(), 200);
    for (x, y) in &places {
        assert!(
            [0, w].contains(x) || [0, full].contains(y),
            "({x},{y}) is not on the map's border"
        );
    }
    // The draw is over the whole perimeter: the first spawn is the walk of `t`, clamped.
    let (wx, wy) = walk(51_200, 51_200, i64::MAX / 4, i64::MAX / 4, i64::from(t));
    let clamp = |v: i64| v.clamp(0, 102_400) as i32;
    assert_eq!(places[0], (clamp(wx), clamp(wy)));
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
    w.remove("around");
    w.remove("spawn");
    let e = TimeLimit::parse(m.to_string().as_bytes())
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

/// The compact form's length and FNV-1a 64, taken on main before VS1.
const PINNED_STAGE_V0: (usize, u64) = (3294, 17_042_691_344_738_360_148);

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
