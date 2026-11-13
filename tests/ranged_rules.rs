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
            self.e
                .step(&[idle(), Default::default(), Default::default()]);
            let now = self.e.tick();
            self.events
                .extend(self.e.events().iter().map(|ev| (now, ev.clone())));
            self.hashes.push(self.e.state_hash());
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
