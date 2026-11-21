//! The weapon's `press_factor` (ADR-SIM-9 points 3 and 4): a smash's hits deal `pct_per_press`
//! percent more damage for each A press before it in the combo path, added with the smash's other
//! bonuses. The tree is A to AAAAA with a smash after each, and a smash from the root; every step
//! is one two-tick active circle of 100 damage over a distant-proof, high-hit-point enemy that
//! does not move, so a step's `Hit` damage reads the bonus alone.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::stage::Stage;

const TARGET_Y: i32 = START_Y - 12_000;
/// Ticks between presses: the step (two active, two recovery) is over, the window still open.
const GAP: usize = 8;
const WINDOW: u32 = 30;

fn attack() -> Value {
    json!({ "inline": [
        { "kind": "active", "ticks": 2, "damage": 100,
          "hit_box": { "type": "circle", "offset": { "forward": 0, "right": 0 },
                       "radius": 30_000 } },
        { "kind": "recovery", "ticks": 2 }
    ] })
}

/// A step that goes on with A up to `AAAAA` and with a smash at every depth.
fn node(a_presses: u32) -> Value {
    let mut next = json!({ "smash": { "attack": attack() } });
    if a_presses < 5 {
        next["main"] = node(a_presses + 1);
    }
    json!({ "attack": attack(), "next": next })
}

fn combo() -> Value {
    json!({ "main": node(1), "smash": { "attack": attack() } })
}

fn weapon(factor: Option<u32>) -> Value {
    let mut inline = json!({ "combo": combo(), "window_ticks": WINDOW, "assist_range": 0,
                             "dash_strike_ticks": 30 });
    if let Some(n) = factor {
        inline["press_factor"] = json!({ "pct_per_press": n });
    }
    json!({ "inline": inline })
}

fn stage_of(weapon: Value) -> Stage {
    let e = enemy(1_000_000, hold(START_X, TARGET_Y), json!([]));
    let mut stage = armed(
        stage_full(
            18_000,
            Some(far_boss()),
            vec![wave(1, 1, 0, e, START_X, TARGET_Y)],
            vec![],
        ),
        weapon,
    );
    stage.sim_version = 2;
    stage
}

fn pressing(smash: bool) -> Input {
    Input {
        main: !smash,
        smash,
        ..idle()
    }
}

/// One press of A or B, then `GAP` idle ticks.
fn press(smash: bool) -> Vec<Input> {
    let mut v = vec![pressing(smash)];
    v.extend(std::iter::repeat_n(idle(), GAP));
    v
}

fn presses(path: &str) -> Vec<Input> {
    path.chars().flat_map(|c| press(c == 'B')).collect()
}

fn damages(e: &mut Engine, inputs: &[Input]) -> Vec<u32> {
    let mut out = Vec::new();
    for input in inputs {
        e.step(&[*input, Default::default(), Default::default()]);
        for ev in e.events() {
            if let DomainEvent::Hit { damage, .. } = ev {
                out.push(*damage);
            }
        }
    }
    out
}

/// The damage of each step of `path` (one hit per step).
fn along(weapon: Value, path: &str) -> Vec<u32> {
    let mut e = engine(&stage_of(weapon));
    damages(&mut e, &[idle(), idle(), idle()]);
    damages(&mut e, &presses(path))
}

#[test]
fn a_smash_deals_25_percent_more_per_a_press_before_it() {
    for (path, want) in [
        ("B", 100),
        ("AB", 125),
        ("AAB", 150),
        ("AAAB", 175),
        ("AAAAB", 200),
        ("AAAAAB", 225),
    ] {
        assert_eq!(along(weapon(Some(25)), path).last(), Some(&want), "{path}");
    }
}

#[test]
fn without_the_factor_and_for_a_normal_step_nothing_changes() {
    assert_eq!(along(weapon(None), "AAAAAB"), vec![100; 6]);
    assert_eq!(along(weapon(Some(25)), "AAAAA"), vec![100; 5]);
}

#[test]
fn the_count_restarts_at_the_root() {
    let mut e = engine(&stage_of(weapon(Some(25))));
    damages(&mut e, &[idle(), idle(), idle()]);
    damages(&mut e, &presses("AAA"));
    damages(&mut e, &vec![idle(); (WINDOW + 1) as usize]);
    assert_eq!(damages(&mut e, &presses("B")), vec![100]);
    // An evade between the presses ends the combo too.
    damages(&mut e, &vec![idle(); 60]);
    damages(&mut e, &presses("AA"));
    damages(
        &mut e,
        &[Input {
            evade: true,
            ..idle()
        }],
    );
    damages(&mut e, &vec![idle(); 20]);
    assert_eq!(damages(&mut e, &presses("B")), vec![100]);
}

#[test]
fn a_dash_smash_counts_its_own_presses() {
    let dash = json!({ "main": { "attack": attack(),
                                 "next": { "smash": { "attack": attack() } } },
                       "smash": { "attack": attack() } });
    let mut w = weapon(Some(25));
    w["inline"]["dash"] = dash;
    let evade = Input {
        evade: true,
        ..idle()
    };
    let mut e = engine(&stage_of(w.clone()));
    damages(&mut e, &[idle(), idle(), idle()]);
    damages(&mut e, &presses("AAA"));
    damages(&mut e, &[evade]);
    damages(&mut e, &[idle(), idle()]);
    // A then B in the dash branch: one press before the smash.
    assert_eq!(damages(&mut e, &presses("AB")), vec![100, 125]);
    // The dash smash from its root: none, whatever came before the evade.
    let mut e = engine(&stage_of(w));
    damages(&mut e, &[idle(), idle(), idle()]);
    damages(&mut e, &presses("AAA"));
    damages(&mut e, &[evade]);
    damages(&mut e, &[idle(), idle()]);
    assert_eq!(damages(&mut e, &presses("B")), vec![100]);
}

#[test]
fn the_press_term_is_held_to_the_percent_cap() {
    assert_eq!(along(weapon(Some(1_000)), "AAAAAB").last(), Some(&1_100));
}

#[test]
fn each_player_counts_their_own_presses() {
    let mut e = engine(&stage_of(weapon(Some(25))));
    e.join(PlayerIdx::new(1).unwrap()).unwrap();
    damages(&mut e, &[idle(), idle(), idle()]);
    let mut got: [Vec<u32>; 2] = Default::default();
    // Player 0 presses A, A, B; player 1 presses B on the same ticks.
    let script = [(false, true), (false, true), (true, true)];
    for (p0_smash, p1_smash) in script {
        let mut ticks = vec![[
            pressing(p0_smash),
            Input {
                smash: p1_smash,
                ..idle()
            },
        ]];
        ticks.extend(std::iter::repeat_n([idle(), idle()], GAP));
        for [a, b] in ticks {
            e.step(&[a, b, Default::default()]);
            for ev in e.events() {
                if let DomainEvent::Hit { damage, source, .. } = ev {
                    let slot = usize::from(*source == PlayerIdx::new(1).unwrap().id());
                    got[slot].push(*damage);
                }
            }
        }
    }
    assert_eq!(got[0].last(), Some(&150), "{got:?}");
    assert_eq!(got[1].first(), Some(&100), "{got:?}");
}

#[test]
fn a_perfect_charged_release_adds_to_the_press_term() {
    // The smash after AA charges: level 1 at 30 ticks, perfect for 8, +50.
    let mut w = weapon(Some(25));
    let charged = json!({ "attack": attack(), "charge": { "levels": [
        { "at_ticks": 30, "perfect_ticks": 8, "perfect_pct": 50, "attack": attack() }
    ], "max_ticks": 60, "move_pct": 100 } });
    w["inline"]["combo"]["main"]["next"]["main"]["next"]["smash"] = charged;
    let mut e = engine(&stage_of(w));
    damages(&mut e, &[idle(), idle(), idle()]);
    damages(&mut e, &presses("AA"));
    let mut inputs: Vec<Input> = vec![pressing(true); 32];
    inputs.extend(vec![idle(); GAP]);
    // 100 + 50 (perfect) + 2 x 25 (presses), added.
    assert_eq!(damages(&mut e, &inputs), vec![200]);
}

/// The refusals of `validate` for `stage`, as `(path, message)`.
fn refused(stage: &Stage) -> Vec<(String, String)> {
    match stella_rain_core::validate::validate(stage) {
        Err(stella_rain_core::validate::Error::Invalid(issues)) => {
            issues.into_iter().map(|i| (i.path, i.message)).collect()
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_stage_at_version_one_refuses_an_inline_press_factor() {
    let mut stage = stage_of(weapon(Some(25)));
    stage.sim_version = 1;
    let found = refused(&stage);
    assert!(
        found
            .iter()
            .any(|(p, m)| p.ends_with("press_factor") && m.contains("needs sim_version 2")),
        "{found:?}"
    );
}

/// The marker of an unshipped field: an inline weapon naming `size_stacks` is refused at a stage.
#[test]
fn a_stage_refuses_an_inline_field_whose_rule_has_not_landed() {
    let mut w = weapon(None);
    w["inline"]["size_stacks"] = json!({ "pct_per_stack": 20, "max_stacks": 3, "ticks": 300 });
    let found = refused(&stage_of(w));
    assert!(
        found.iter().any(|(p, m)| p.ends_with("size_stacks")
            && m.contains("the field size_stacks is not shipped yet")),
        "{found:?}"
    );
}

// --- The ring: consecutive active phases that keep one hit list --------------------------------

use stella_rain_core::id::EntityId;
use stella_rain_core::registry;

/// Distances ahead of the character of the three enemies, and the ring's three radii: each
/// enemy (radius 2048) is first inside the next circle only.
const RING_AT: [i32; 3] = [2_000, 8_000, 14_000];
const RING_RADII: [i32; 3] = [4_096, 8_192, 12_288];

fn ring_phase(radius: i32, keeps: bool) -> Value {
    let mut p = json!({ "kind": "active", "ticks": 2, "damage": 100,
        "hit_box": { "type": "circle", "offset": { "forward": 0, "right": 0 },
                     "radius": radius } });
    if keeps {
        p["keeps_hits"] = json!(true);
    }
    p
}

fn ring(keeps: bool) -> Value {
    let mut phases: Vec<Value> = RING_RADII
        .iter()
        .enumerate()
        .map(|(i, r)| ring_phase(*r, keeps && i > 0))
        .collect();
    phases.push(json!({ "kind": "recovery", "ticks": 2 }));
    json!({ "inline": phases })
}

/// A weapon whose smash from the root, and after AA, is the ring; A is a quiet step.
fn ring_weapon(keeps: bool) -> Value {
    let quiet = || json!({ "attack": quiet_attack() });
    let mut step = quiet();
    step["next"] = json!({ "main": { "attack": quiet_attack(),
                                      "next": { "smash": { "attack": ring(keeps) } } } });
    json!({ "inline": { "combo": { "main": step, "smash": { "attack": ring(keeps) } },
                        "window_ticks": WINDOW, "assist_range": 0,
                        "press_factor": { "pct_per_press": 25 } } })
}

fn ring_stage(weapon: Value, at: [i32; 3]) -> Stage {
    let waves = at
        .iter()
        .map(|d| {
            let y = START_Y - d;
            wave(
                1,
                1,
                0,
                enemy(1_000_000, hold(START_X, y), json!([])),
                START_X,
                y,
            )
        })
        .collect();
    let mut stage = armed(stage_full(18_000, Some(far_boss()), waves, vec![]), weapon);
    stage.sim_version = 2;
    stage
}

/// `(tick, target, damage)` of every hit while `inputs` are played.
fn ring_hits(e: &mut Engine, inputs: &[Input]) -> Vec<(u32, EntityId, u32)> {
    let mut out = Vec::new();
    for input in inputs {
        e.step(&[*input, Default::default(), Default::default()]);
        for ev in e.events() {
            if let DomainEvent::Hit { target, damage, .. } = ev {
                out.push((e.tick(), *target, *damage));
            }
        }
    }
    out
}

fn ring_run(weapon: Value, path: &str) -> Vec<(u32, EntityId, u32)> {
    let mut e = engine(&ring_stage(weapon, RING_AT));
    ring_hits(&mut e, &[idle(), idle(), idle()]);
    ring_hits(&mut e, &presses(path))
}

#[test]
fn a_ring_hits_each_enemy_once_as_it_spreads() {
    let hits = ring_run(ring_weapon(true), "B");
    let targets: std::collections::BTreeSet<_> = hits.iter().map(|h| h.1).collect();
    assert_eq!((hits.len(), targets.len()), (3, 3), "{hits:?}");
    // In the order the ring reaches them: three phases of two ticks, so a tick apart pair by pair.
    assert!(hits[0].0 < hits[1].0 && hits[1].0 < hits[2].0, "{hits:?}");
    // Enemies spawn near to far, so their IDs rise in the order they are hit.
    let ids: Vec<EntityId> = hits.iter().map(|h| h.1).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

#[test]
fn without_the_flag_a_growing_phase_hits_again() {
    let hits = ring_run(ring_weapon(false), "B");
    // The enemy inside every circle is hit by each of the three phases; the middle one by two.
    assert_eq!(hits.len(), 6, "{hits:?}");
}

#[test]
fn the_press_factor_applies_to_every_ring_hit_alike() {
    let hits = ring_run(ring_weapon(true), "AAB");
    assert_eq!(
        hits.iter().map(|h| h.2).collect::<Vec<_>>(),
        vec![150, 150, 150],
        "{hits:?}"
    );
}

#[test]
fn an_enemy_past_the_last_circle_is_missed() {
    let mut e = engine(&ring_stage(
        ring_weapon(true),
        [RING_AT[0], RING_AT[1], 16_000],
    ));
    ring_hits(&mut e, &[idle(), idle(), idle()]);
    // The third enemy stands at 16,000: past 12,288 plus its radius.
    let hits = ring_hits(&mut e, &presses("B"));
    assert_eq!(hits.len(), 2, "{hits:?}");
}

fn keeps_in_stage(weapon_phase: Value, sim_version: u32) -> Vec<(String, String)> {
    let weapon = json!({ "inline": {
        "combo": { "main": { "attack": { "inline": [weapon_phase] } } },
        "window_ticks": 10, "assist_range": 0 } });
    let mut stage = armed(stage_with(600, None, vec![]), weapon);
    stage.sim_version = sim_version;
    match stella_rain_core::validate::validate(&stage) {
        Ok(()) => Vec::new(),
        Err(stella_rain_core::validate::Error::Invalid(issues)) => {
            issues.into_iter().map(|i| (i.path, i.message)).collect()
        }
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn keeps_hits_stands_on_a_players_active_phase_at_version_two() {
    let active = ring_phase(4_096, true);
    assert!(keeps_in_stage(active.clone(), 2).is_empty());
    // Version 1.
    let old = keeps_in_stage(active, 1);
    assert!(
        old.iter()
            .any(|(p, m)| p.ends_with("keeps_hits") && m.contains("needs sim_version 2")),
        "{old:?}"
    );
    // A wind-up phase.
    let wind_up = json!({ "kind": "wind_up", "ticks": 2, "keeps_hits": true });
    let found = keeps_in_stage(wind_up, 2);
    assert!(
        found.iter().any(|(p, m)| p.ends_with("keeps_hits")
            && m.contains("only an active phase with a hit box keeps its hits")),
        "{found:?}"
    );
    // An enemy's or the boss's attack.
    let phases: Vec<stella_rain_core::attack::AttackPhase> =
        serde_json::from_value(json!([ring_phase(8_192, true)])).expect("the phase parses");
    let found =
        stella_rain_core::validate::validate_attack(&phases, 0, registry::ENTRIES).unwrap_err();
    assert!(
        found.iter().any(|i| i
            .message
            .contains("a main character's or an ally's attack only")),
        "{found:?}"
    );
}
