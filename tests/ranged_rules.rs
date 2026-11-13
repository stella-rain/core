//! The ranged picks: a per-target grace after a hit (`ranged_grace_ticks`), and the view bar for
//! elites and companions. Every tick is worked out by hand from the placeholders of each test.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::pack::ContentFiles;
use stella_rain_core::rules::Rules;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::stage::Stage;
use stella_rain_core::version::Versions;

use crate::common::*;

/// The harness rules: the bundled file with `patch` laid over it; a `null` removes a key.
fn content_with(patch: Value) -> RunContent {
    let mut v: Value = serde_json::from_slice(ContentFiles::bundled_file("rules")).unwrap();
    for (k, x) in patch.as_object().unwrap() {
        if x.is_null() {
            v.as_object_mut().unwrap().remove(k);
        } else {
            v[k] = x.clone();
        }
    }
    let rules = Rules::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();
    design_content().with_rules(rules)
}

fn stage_of(waves: Vec<Value>, boss: Value, companions: Vec<Value>) -> Stage {
    let mut stage = quiet(stage_full(900, Some(boss), waves, companions));
    stage.sim_version = 2;
    stage
}

/// What a run showed: every event with the tick it came in.
struct Run {
    e: Engine,
    events: Vec<(u32, DomainEvent)>,
    hashes: Vec<u64>,
}

impl Run {
    fn new(stage: &Stage, patch: Value) -> Run {
        let e = Engine::new_with(stage, &content_with(patch)).expect("the run starts");
        Run {
            e,
            events: vec![],
            hashes: vec![],
        }
    }

    fn to(&mut self, tick: u32) {
        while self.e.tick() < tick {
            let before = self.e.tick();
            self.e
                .step(&[idle(), Default::default(), Default::default()]);
            let now = self.e.tick();
            self.events
                .extend(self.e.events().iter().map(|ev| (now, ev.clone())));
            self.hashes.push(self.e.state_hash());
            if self.e.tick() == before {
                break; // the run is over
            }
        }
    }

    fn ticks(&self, pick: impl Fn(&DomainEvent) -> bool) -> Vec<u32> {
        ticks_of(&self.events, pick)
    }
}

// --- The grace ------------------------------------------------------------------------------

/// A regular enemy's ranged attack: a 2-tick wind-up laying a warning zone, then 1 tick of
/// recovery, so the enemy is ready again at once; the token's cooldown is the only wait.
fn warning_zone() -> Value {
    json!({ "inline": [
        { "kind": "wind_up", "ticks": 2, "spawn": [
            { "type": "zone", "shape": { "type": "circle", "radius": 4096 }, "anchor": "target",
              "wind_up_ticks": 40, "warn_only": true }] },
        { "kind": "recovery", "ticks": 1 },
    ] })
}

fn shooter() -> Value {
    enemy(
        777,
        hold(START_X, near_start(0, -40).1),
        json!([{ "when": always(), "target": "player", "cooldown_ticks": 0,
                 "do": { "type": "attack", "attack": warning_zone(), "ranged": true } }]),
    )
}

/// An enemy that hurts the main character once, a tick or so after it appears.
fn striker() -> Value {
    enemy(
        778,
        hold(START_X, near_start(0, -16).1),
        json!([{ "when": always(), "target": "player", "cooldown_ticks": 1000,
                 "do": { "type": "attack", "attack": { "inline": [
                     { "kind": "active", "ticks": 1, "damage": 1,
                       "hit_box": { "type": "circle", "radius": 8_192 } },
                     { "kind": "recovery", "ticks": 4 }] } } }]),
    )
}

fn telegraph_ticks(r: &Run) -> Vec<u32> {
    r.ticks(|e| matches!(e, DomainEvent::Telegraph { .. }))
}

fn grace_run(waves: Vec<Value>, row: Value) -> Run {
    let mut r = Run::new(&stage_of(waves, far_boss(), vec![]), row);
    r.to(400);
    r
}

#[test]
fn g6_a_hit_keeps_a_ranged_start_off_the_player_for_the_rows_ticks() {
    let waves = vec![
        wave(100, 1, 0, shooter(), START_X, near_start(0, -40).1),
        wave(200, 1, 0, striker(), START_X, near_start(0, -16).1),
    ];
    let with = grace_run(waves.clone(), json!({ "ranged_grace_ticks": 30 }));
    let hit = *with
        .ticks(|e| matches!(e, DomainEvent::PlayerHit { .. }))
        .first()
        .expect("the striker hits once");
    let after = |r: &Run| telegraph_ticks(r).into_iter().find(|t| *t > hit);
    assert_eq!(after(&with), Some(hit + 30), "refused on hit+1 to hit+29");
    let without = grace_run(waves, json!({ "ranged_grace_ticks": null }));
    let hit0 = *without
        .ticks(|e| matches!(e, DomainEvent::PlayerHit { .. }))
        .first()
        .unwrap();
    assert_eq!(hit0, hit, "the same hit");
    assert!(
        after(&without).is_some_and(|t| t <= hit + 4),
        "without the row the next start follows at once: {:?}",
        after(&without)
    );
}

#[test]
fn g7_the_row_alone_changes_nothing_until_a_hit() {
    let shooter_only = vec![wave(100, 1, 0, shooter(), START_X, near_start(0, -40).1)];
    let absent = grace_run(shooter_only.clone(), json!({ "ranged_grace_ticks": null }));
    let named = grace_run(shooter_only, json!({ "ranged_grace_ticks": 1 }));
    assert!(!telegraph_ticks(&absent).is_empty());
    assert!(
        absent.hashes == named.hashes,
        "no hit: the same state every tick"
    );
    // With a hit, a named row moves the state (the timer is set), an absent one does not.
    let waves = vec![
        wave(100, 1, 0, shooter(), START_X, near_start(0, -40).1),
        wave(200, 1, 0, striker(), START_X, near_start(0, -16).1),
    ];
    let hit_absent = grace_run(waves.clone(), json!({ "ranged_grace_ticks": null }));
    let hit_named = grace_run(waves, json!({ "ranged_grace_ticks": 30 }));
    assert!(hit_absent.hashes != hit_named.hashes, "the timer is hashed");
}

// --- The view bar ----------------------------------------------------------------------------
//
// The field is 360 by 640 px; the start is 180, 576. The view is centred on the main character
// but held inside the map, so it covers y from 280 to 640 (x is covered whole): a point at
// y = 40 is off view, one at y = 500 is in view.

const OFF_Y: i32 = 40;
const IN_Y: i32 = 500;

fn at_y(y_px: i32) -> (i32, i32) {
    (START_X, y_px * 256)
}

/// A zone attack: a wind-up of 50 ticks laying a warning zone, then a tick of recovery.
fn zone_attack() -> Value {
    json!({ "inline": [
        { "kind": "wind_up", "ticks": 50, "spawn": [
            { "type": "zone", "shape": { "type": "circle", "radius": 4096 }, "anchor": "target",
              "wind_up_ticks": 10, "warn_only": true }] },
        { "kind": "recovery", "ticks": 1 },
    ] })
}

/// A melee swing: no `spawn`.
fn slash() -> Value {
    json!({ "inline": [
        { "kind": "active", "ticks": 1, "damage": 1,
          "hit_box": { "type": "circle", "radius": 1_024 } },
        { "kind": "recovery", "ticks": 1 },
    ] })
}

fn rule_of(target: &str, attack: Value) -> Value {
    json!({ "when": always(), "target": target, "cooldown_ticks": 60,
            "do": { "type": "attack", "attack": attack } })
}

fn elite(movement: Value, rules: Value) -> Value {
    let mut e = enemy(900_000, movement, rules);
    e["elite"] = json!({ "range": 262_144 });
    e
}

fn elite_run(movement: Value, rules: Value, spawn_y: i32, ticks: u32) -> Run {
    let (x, y) = at_y(spawn_y);
    let waves = vec![wave(1, 1, 0, elite(movement, rules), x, y)];
    let mut r = Run::new(
        &stage_of(waves, far_boss(), vec![]),
        json!({ "ranged_grace_ticks": null }),
    );
    r.to(ticks);
    r
}

fn rules_fired(r: &Run) -> Vec<(u32, u8)> {
    r.events
        .iter()
        .filter_map(|(t, e)| match e {
            DomainEvent::RuleFired { rule, .. } => Some((*t, *rule)),
            _ => None,
        })
        .collect()
}

#[test]
fn v1_an_elite_off_view_lays_nothing_and_one_in_view_starts_at_once() {
    let rules = json!([rule_of("player", zone_attack())]);
    let (x, y) = at_y(OFF_Y);
    let off = elite_run(hold(x, y), rules.clone(), OFF_Y, 100);
    assert_eq!(telegraph_ticks(&off), Vec::<u32>::new());
    assert_eq!(rules_fired(&off), []);
    let (x, y) = at_y(IN_Y);
    let inside = elite_run(hold(x, y), rules, IN_Y, 100);
    assert!(telegraph_ticks(&inside).first().is_some_and(|t| *t <= 3));
}

/// The view's centre y after each tick while the main character walks up from the start, in
/// 1/256 px: the main character's y, held between half the view and the map less half of it.
fn walk_up(r: &mut Run, ticks: u32) -> Vec<i32> {
    let mut centres = vec![];
    for _ in 0..ticks {
        r.e.step(&[moving(0, -127), Default::default(), Default::default()]);
        let now = r.e.tick();
        r.events
            .extend(r.e.events().iter().map(|ev| (now, ev.clone())));
        let y = r.e.snapshot().players[0].at.y.0;
        centres.push(y.clamp(180 * 256, 460 * 256));
    }
    centres
}

#[test]
fn v2_a_refused_start_retries_and_keeps_its_cooldown() {
    // The elite stands still 40 px from the top; the main character walks up, and the view
    // with it, until the elite is inside its edge (180 px above the centre, edges included).
    let rules = json!([rule_of("player", zone_attack())]);
    let (x, y) = at_y(OFF_Y);
    let waves = vec![wave(1, 1, 0, elite(hold(x, y), rules), x, y)];
    let mut r = Run::new(
        &stage_of(waves, far_boss(), vec![]),
        json!({ "ranged_grace_ticks": null }),
    );
    let centres = walk_up(&mut r, 300);
    let first = rules_fired(&r).first().map(|(t, _)| *t).expect("it starts");
    let apart = |t: u32| (centres[(t - 1) as usize] - y).abs();
    let half = 180 * 256;
    assert!(
        apart(first) <= half,
        "in view when it starts: {}",
        apart(first)
    );
    assert!(
        apart(first - 1) > half,
        "out of view the tick before: {}",
        apart(first - 1)
    );
}

fn companion_run(y_px: i32) -> Run {
    let (x, y) = at_y(y_px);
    let target = wave(
        1,
        1,
        0,
        enemy(1_000_000, hold(x, at_y(IN_Y).1), json!([])),
        x,
        at_y(IN_Y).1,
    );
    let c = companion(
        50,
        hold(x, y),
        json!([rule_of("nearest_enemy", zone_attack())]),
    );
    let mut r = Run::new(
        &stage_of(vec![target], far_boss(), vec![c]),
        json!({ "ranged_grace_ticks": null }),
    );
    r.to(100);
    r
}

#[test]
fn v5_a_companion_off_view_lays_nothing_and_one_in_view_starts() {
    assert_eq!(telegraph_ticks(&companion_run(OFF_Y)), Vec::<u32>::new());
    assert!(!telegraph_ticks(&companion_run(IN_Y)).is_empty());
}

#[test]
fn v6_the_boss_and_a_melee_swing_start_off_view() {
    let (x, y) = at_y(OFF_Y);
    let boss = json!({ "base": "golem", "hp": 1_000_000, "radius": 8_192,
        "spawn": { "x": x, "y": y },
        "phases": [{ "timeline": [{ "type": "wait", "ticks": 5 },
                                  { "type": "attack", "attack": zone_attack() }] }] });
    let mut r = Run::new(
        &stage_of(vec![], boss, vec![]),
        json!({ "ranged_grace_ticks": null }),
    );
    r.to(60);
    assert!(
        !telegraph_ticks(&r).is_empty(),
        "the boss starts its zone attack off view"
    );
    let rules = json!([rule_of("player", slash())]);
    let swing = elite_run(hold(x, y), rules, OFF_Y, 100);
    assert!(!rules_fired(&swing).is_empty(), "a swing is never barred");
}

#[test]
fn v7_the_hit_lands_whole_and_the_invulnerability_is_the_only_cap() {
    let (x, y) = (START_X, START_Y - 8_192);
    let striker = enemy(
        778,
        hold(x, y),
        json!([{ "when": always(), "target": "player", "cooldown_ticks": 61,
                 "do": { "type": "attack", "attack": { "inline": [
                     { "kind": "active", "ticks": 1, "damage": 60_000,
                       "hit_box": { "type": "circle", "radius": 8_192 } }] } } }]),
    );
    let mut stage = stage_of(vec![wave(1, 1, 0, striker, x, y)], far_boss(), vec![]);
    stage.player.hp = 99_900;
    let mut r = Run::new(&stage, json!({ "ranged_grace_ticks": null }));
    r.to(200);
    let left: Vec<u32> = r
        .events
        .iter()
        .filter_map(|(_, e)| match e {
            DomainEvent::PlayerHit { hp_left, .. } => Some(*hp_left),
            _ => None,
        })
        .collect();
    assert_eq!(&left[..2], [39_900, 0]);
}
