//! The mimic (ADR-090 point 5, app#259): an elite in disguise. Disguised it is an object: it
//! does not move or attack, nothing damages it, aim assist and the selectors never pick it, the
//! snapshot shows its disguise sprite and the minimap does not know it is an elite. The first
//! tick the main character's centre is within `reveal_radius` of it, it reveals (`MimicRevealed`,
//! once): it takes damage from that tick's collisions on, and acts like a grade elite from the
//! next tick. The boss's call skips a disguised mimic and calls a revealed one.

use serde_json::{Value, json};
use stella_rain_core::behaviour::Elite;
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::{EntityKind, EntityView};
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{self, Error, limits};

use crate::common;

use common::*;

/// The mimic's disguise in these tests.
const DISGUISE: &str = "golem";
/// 50 pixels.
const REVEAL: i32 = 12_800;
/// 100 pixels to the right of the main character: out of its line of fire, and out of reveal.
const SIDE: (i32, i32) = (START_X + 25_600, START_Y);
/// Above the main character, in the line of its shots and out of reveal.
const AHEAD: (i32, i32) = (START_X, 100_000);

/// A mimic of `base` hp 10 that holds where it is, with `rules`.
fn mimic_with(reveal_radius: i32, movement: Value, rules: Value) -> Value {
    let mut e = enemy(10, movement, rules);
    e["elite"] = json!({
        "range": 51_200,
        "mimic": { "disguise": DISGUISE, "reveal_radius": reveal_radius }
    });
    e
}

/// A mimic that follows the main character and shoots it every 30 ticks once it acts.
fn mimic(reveal_radius: i32) -> Value {
    mimic_with(
        reveal_radius,
        json!({ "type": "follow", "distance": 0 }),
        json!([attack_rule(always(), 30)]),
    )
}

fn quiet_stage(waves: Vec<Value>) -> Stage {
    stage_full(18_000, Some(far_boss()), waves, vec![])
}

fn view_of(e: &Engine, elite: bool) -> Vec<EntityView> {
    e.snapshot()
        .entities
        .into_iter()
        .filter(|v| v.kind == EntityKind::Enemy && v.elite == elite)
        .collect()
}

/// The enemy at `at` (the mimic, which does not move while disguised).
fn mimic_view(e: &Engine) -> EntityView {
    let all: Vec<_> = e
        .snapshot()
        .entities
        .into_iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .collect();
    all.into_iter().next().expect("the mimic is on the field")
}

fn disguised(v: &EntityView) -> bool {
    serde_json::to_value(v).unwrap()["disguised"] == json!(true)
}

/// The event, by its written form: `{"type": "MimicRevealed", "data": {"elite": id}}`.
fn revealed_event(ev: &DomainEvent) -> Option<u32> {
    let v = serde_json::to_value(ev).ok()?;
    v.get("type").filter(|t| *t == "MimicRevealed")?;
    v.get("data")?.get("elite")?.as_u64().map(|id| id as u32)
}

fn revealed(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| revealed_event(ev).is_some())
}

fn fired(events: &[(u32, DomainEvent)], agent: stella_rain_core::id::EntityId) -> Vec<u32> {
    ticks_of(
        events,
        |ev| matches!(ev, DomainEvent::RuleFired { agent: a, .. } if *a == agent),
    )
}

fn dist2(e: &Engine, at: (i32, i32)) -> i64 {
    let p = player_at(e);
    let (dx, dy) = (i64::from(p.0 - at.0), i64::from(p.1 - at.1));
    dx * dx + dy * dy
}

// --- Disguised ---------------------------------------------------------------------------------

#[test]
fn a_disguised_mimic_takes_no_damage_and_the_main_character_s_shots_pass_through() {
    // The mimic is in the line of fire, with an ordinary enemy of 10 hp behind it.
    let behind = wave(
        1,
        1,
        0,
        enemy(10, hold(START_X, 60_000), json!([])),
        START_X,
        60_000,
    );
    let mimic = wave(1, 1, 0, mimic(REVEAL), AHEAD.0, AHEAD.1);
    let mut e = engine(&quiet_stage(vec![mimic, behind]));
    let events = fire_until(&mut e, idle(), 300, |_| false);
    let hits: Vec<_> = events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Hit { target, .. } => Some(*target),
            _ => None,
        })
        .collect();
    assert!(!hits.is_empty(), "the enemy behind it was hit: {events:?}");
    let v = e
        .snapshot()
        .entities
        .into_iter()
        .find(|v| v.kind == EntityKind::Enemy && v.at.y.0 == AHEAD.1)
        .expect("the mimic is still there");
    assert_eq!(v.hp, 10, "undamaged");
    assert!(hits.iter().all(|t| *t != v.id), "{events:?}");
    assert!(disguised(&v));
    assert!(revealed(&events).is_empty());
}

#[test]
fn a_disguised_mimic_takes_no_damage_from_a_hit_box() {
    // A companion swings a hit box wide enough for a nearby enemy and the mimic; only the enemy
    // is hit.
    let target = wave(
        1,
        1,
        0,
        enemy(50, hold(20_000, 30_000), json!([])),
        20_000,
        30_000,
    );
    let hidden = wave(1, 1, 0, mimic(REVEAL), 20_000 + 3_000, 30_000);
    let swing = json!([{ "kind": "active", "ticks": 1, "damage": 3,
                         "hit_box": { "type": "circle", "radius": 20 * 256,
                                      "offset": { "forward": 0, "right": 0 } } }]);
    let fighter = companion(
        50,
        hold(20_000, 30_000),
        json!([{ "when": always(), "target": "nearest_enemy", "once": false, "cooldown_ticks": 0,
                 "do": { "type": "attack", "attack": { "inline": swing } } }]),
    );
    let st = quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![target, hidden],
        vec![fighter],
    ));
    let mut e = engine(&st);
    let events = run_until(&mut e, idle(), 5, |_| false);
    let hit: Vec<_> = events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Hit { target, .. } => Some(*target),
            _ => None,
        })
        .collect();
    assert!(!hit.is_empty(), "the enemy was hit: {events:?}");
    let mimic = e
        .snapshot()
        .entities
        .into_iter()
        .find(|v| v.at.x.0 == 23_000)
        .unwrap();
    assert_eq!(mimic.hp, 10);
    assert!(hit.iter().all(|t| *t != mimic.id), "{events:?}");
}

#[test]
fn a_disguised_mimic_does_not_move_or_attack() {
    // It follows and shoots once it acts; disguised, it does neither.
    let mut e = engine(&quiet_stage(vec![wave(1, 1, 0, mimic(1), SIDE.0, SIDE.1)]));
    let events = run_until(&mut e, idle(), 600, |_| false);
    let v = mimic_view(&e);
    assert_eq!((v.at.x.0, v.at.y.0), SIDE);
    assert!(disguised(&v));
    assert!(
        events.iter().all(|(_, ev)| !matches!(
            ev,
            DomainEvent::RuleFired { .. } | DomainEvent::Hit { .. } | DomainEvent::Died { .. }
        )),
        "{events:?}"
    );
}

#[test]
fn a_disguised_mimic_is_not_pushed_apart_or_pulled() {
    // An ordinary enemy stands on it; the other enemy is pushed off, the mimic is fixed.
    let on_top = wave(
        1,
        1,
        0,
        enemy(10, hold(SIDE.0, SIDE.1), json!([])),
        SIDE.0 + 100,
        SIDE.1,
    );
    let mut e = engine(&quiet_stage(vec![
        wave(1, 1, 0, mimic(1), SIDE.0, SIDE.1),
        on_top,
    ]));
    run_until(&mut e, idle(), 60, |_| false);
    assert_eq!((mimic_view(&e).at.x.0, mimic_view(&e).at.y.0), SIDE);
}

#[test]
fn a_disguised_mimic_counts_toward_the_elite_cap() {
    let mut st = quiet_stage(vec![wave(1, 1, 0, mimic(1), SIDE.0, SIDE.1)]);
    st.elite_cap = Some(1);
    let mut joiner = enemy(10, json!({ "type": "follow", "distance": 0 }), json!([]));
    joiner["elite"] = json!({ "range": 256 });
    st.waves
        .push(serde_json::from_value(wave(1, 1, 0, joiner, SIDE.0, SIDE.1 - 50_000)).unwrap());
    let mut e = engine(&st);
    run_until(&mut e, idle(), 5, |_| false);
    let all: Vec<_> = e
        .snapshot()
        .entities
        .into_iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .collect();
    assert_eq!(all.len(), 1, "the cap of 1 is spent on the mimic");
}

// --- The snapshot ------------------------------------------------------------------------------

#[test]
fn the_snapshot_shows_the_disguise_and_hides_that_it_is_an_elite() {
    let mut e = engine(&quiet_stage(vec![wave(1, 1, 0, mimic(1), SIDE.0, SIDE.1)]));
    run_until(&mut e, idle(), 3, |_| false);
    let v = mimic_view(&e);
    assert_eq!(v.asset.0, DISGUISE);
    assert!(disguised(&v));
    assert!(!v.elite, "the minimap does not show a disguised mimic");
    assert_eq!(v.aura_radius, None);
    assert_eq!(view_of(&e, true).len(), 0);
}

#[test]
fn a_view_that_is_not_disguised_does_not_write_the_flag() {
    let mut e = engine(&quiet_stage(vec![wave(
        1,
        1,
        0,
        enemy(10, hold(SIDE.0, SIDE.1), json!([])),
        SIDE.0,
        SIDE.1,
    )]));
    run_until(&mut e, idle(), 3, |_| false);
    let v = serde_json::to_value(mimic_view(&e)).unwrap();
    assert!(v.get("disguised").is_none(), "{v}");
}

// --- Revealing ---------------------------------------------------------------------------------

/// Walks the main character to the right, onto the mimic at `SIDE`, until it reveals; the events
/// so far. Checks each tick that the reveal comes on the first tick the main character's centre
/// is within `REVEAL` (the edge counts).
fn walk_in(e: &mut Engine) -> Vec<(u32, DomainEvent)> {
    let mut seen = Vec::new();
    let mut within_before = false;
    for _ in 0..100 {
        seen.extend(run_until(e, moving(127, 0), 1, |_| false));
        let within = dist2(e, SIDE) <= i64::from(REVEAL) * i64::from(REVEAL);
        assert_eq!(
            !revealed(&seen).is_empty(),
            within || within_before,
            "tick {}: {seen:?}",
            e.tick()
        );
        within_before |= within;
        if within {
            return seen;
        }
    }
    panic!("the main character never came within reach");
}

#[test]
fn the_main_character_coming_within_the_radius_reveals_it_once() {
    let mut e = engine(&quiet_stage(vec![wave(
        1,
        1,
        0,
        mimic(REVEAL),
        SIDE.0,
        SIDE.1,
    )]));
    let mut events = walk_in(&mut e);
    let at = revealed(&events)[0];
    assert_eq!(e.tick(), at);
    let id = mimic_view(&e).id;
    assert!(
        events
            .iter()
            .any(|(_, ev)| revealed_event(ev) == Some(id.0)),
        "{events:?}"
    );
    // The main character stays within it: it does not reveal again.
    events.extend(run_until(&mut e, idle(), 120, |_| false));
    assert_eq!(revealed(&events), [at]);
}

#[test]
fn a_revealed_mimic_shows_its_own_sprite_and_is_an_elite_to_the_minimap() {
    let mut e = engine(&quiet_stage(vec![wave(
        1,
        1,
        0,
        mimic(REVEAL),
        SIDE.0,
        SIDE.1,
    )]));
    walk_in(&mut e);
    let v = mimic_view(&e);
    assert_eq!(v.asset.0, "bat");
    assert!(!disguised(&v));
    assert!(v.elite);
    assert!(
        serde_json::to_value(&v).unwrap().get("disguised").is_none(),
        "absent when false"
    );
}

#[test]
fn it_takes_damage_from_the_tick_it_reveals_and_is_never_hidden_again() {
    // A bullet of the main character is on the mimic when it reveals: the walk is slow enough
    // that the test shoots from where the mimic is. Simpler: the mimic reveals at once, on tick
    // 1, and the shots that reach it afterwards hurt it.
    let mut e = engine(&quiet_stage(vec![wave(
        1,
        1,
        0,
        mimic(REVEAL * 10),
        AHEAD.0,
        AHEAD.1,
    )]));
    let events = fire_until(&mut e, idle(), 200, |_| false);
    assert_eq!(revealed(&events), [1], "it reveals on its first tick");
    let hits = events
        .iter()
        .filter(|(_, ev)| matches!(ev, DomainEvent::Hit { .. }))
        .count();
    assert!(hits > 0, "{events:?}");
    assert!(
        events
            .iter()
            .any(|(_, ev)| matches!(ev, DomainEvent::Died { .. })),
        "a revealed mimic dies like any elite: {events:?}"
    );
}

#[test]
fn it_attacks_from_the_tick_after_it_reveals() {
    let mut e = engine(&quiet_stage(vec![wave(
        1,
        1,
        0,
        mimic(REVEAL),
        SIDE.0,
        SIDE.1,
    )]));
    let mut events = walk_in(&mut e);
    let at = revealed(&events)[0];
    let id = mimic_view(&e).id;
    events.extend(run_until(&mut e, idle(), 40, |_| false));
    assert_eq!(
        fired(&events, id).first().copied(),
        Some(at + 1),
        "not on the tick it reveals, on the next: {events:?}"
    );
}

#[test]
fn a_revealed_mimic_moves_by_its_own_movement() {
    let mut e = engine(&quiet_stage(vec![wave(
        1,
        1,
        0,
        mimic(REVEAL),
        SIDE.0,
        SIDE.1,
    )]));
    walk_in(&mut e);
    let before = mimic_view(&e).at;
    assert_eq!(
        (before.x.0, before.y.0),
        SIDE,
        "it did not move on the reveal tick"
    );
    run_until(&mut e, idle(), 10, |_| false);
    let after = mimic_view(&e).at;
    assert!(
        after.x.0 < SIDE.0,
        "it follows the main character: {after:?}"
    );
}

// --- Targets -----------------------------------------------------------------------------------

/// 100 pixels.
const RANGE: i32 = 25_600;

fn assisted_stage(reveal_radius: i32) -> Stage {
    let near = |dx: i32| {
        let (x, y) = (START_X + dx, START_Y);
        (x, y)
    };
    let (mx, my) = near(RANGE / 4);
    let (bx, by) = near(-RANGE / 2);
    let shot = json!({ "inline": [
        { "kind": "active", "ticks": 1, "spawn": [{ "type": "fire", "speed": 2048,
            "direction": { "type": "aimed", "offset": 0 } }] },
        { "kind": "recovery", "ticks": 1 }
    ] });
    let weapon = json!({ "inline": { "combo": { "main": { "attack": shot } },
                                     "window_ticks": 0, "assist_range": RANGE } });
    armed(
        stage_with(
            600,
            Some(far_boss()),
            vec![
                wave(1, 1, 0, mimic(reveal_radius), mx, my),
                wave(1, 1, 0, enemy(100_000, hold(bx, by), json!([])), bx, by),
            ],
        ),
        weapon,
        quiet_sub(),
    )
}

fn assisted() -> Input {
    Input {
        assist: true,
        ..idle()
    }
}

fn target_x(e: &Engine) -> Option<i32> {
    let s = e.snapshot();
    let id = s.player.target?;
    s.entities
        .iter()
        .find(|v| v.id == id)
        .map(|v| v.at.x.0 - START_X)
}

#[test]
fn aim_assist_never_picks_a_disguised_mimic() {
    // The mimic is nearer and an elite; it is passed over for the ordinary enemy.
    let mut e = engine(&assisted_stage(1));
    e.step(assisted());
    e.step(assisted());
    assert_eq!(target_x(&e), Some(-RANGE / 2));
}

#[test]
fn aim_assist_picks_a_revealed_mimic_before_an_ordinary_enemy() {
    // Within reach of the mimic at once: it reveals on tick 1, and is picked from tick 2.
    let mut e = engine(&assisted_stage(RANGE));
    e.step(assisted());
    assert_eq!(
        target_x(&e),
        Some(-RANGE / 2),
        "tick 1: still disguised when it picks"
    );
    e.step(assisted());
    assert_eq!(target_x(&e), Some(RANGE / 4));
}

#[test]
fn a_companion_s_nearest_enemy_is_never_a_disguised_mimic() {
    let mimic_at = (20_000, 30_000);
    let other = (20_000 + 6_000, 30_000);
    let fighter = companion(
        50,
        hold(20_000, 30_000),
        json!([{ "when": always(), "target": "nearest_enemy", "once": false, "cooldown_ticks": 0,
                 "do": { "type": "attack", "attack": { "inline": [
                     { "kind": "active", "ticks": 1, "damage": 3,
                       "hit_box": { "type": "circle", "radius": 256,
                                    "offset": { "forward": 0, "right": 0 } } }] } } }]),
    );
    let st = quiet(stage_full(
        18_000,
        Some(far_boss()),
        vec![
            wave(1, 1, 0, mimic(1), mimic_at.0, mimic_at.1),
            wave(
                1,
                1,
                0,
                enemy(50, hold(other.0, other.1), json!([])),
                other.0,
                other.1,
            ),
        ],
        vec![fighter],
    ));
    let mut e = engine(&st);
    let events = run_until(&mut e, idle(), 3, |_| false);
    // The hit box (radius 1 pixel) reaches only what is on the companion: the mimic is under
    // it. The companion picks the other enemy, so it attacks it (facing away from the mimic).
    let target = events.iter().find_map(|(_, ev)| match ev {
        DomainEvent::RuleFired { target, .. } => *target,
        _ => None,
    });
    let far = e
        .snapshot()
        .entities
        .into_iter()
        .find(|v| v.at.x.0 == other.0)
        .unwrap();
    assert_eq!(target, Some(far.id), "{events:?}");
}

// --- The boss's call ---------------------------------------------------------------------------

/// A stage whose boss calls its elites at tick `after`, with the mimic at `SIDE` (a range of 1,
/// so that it acts at a distance only when called) and an ordinary elite.
fn calling(after: u32) -> Stage {
    let mut boss = boss_at(1_000_000, 10_000);
    boss["call_elites"] = json!({ "after_ticks": after });
    let mut joiner = enemy(
        10,
        json!({ "type": "follow", "distance": 0 }),
        json!([rule(always(), json!("player"), telegraph(), 0)]),
    );
    joiner["elite"] = json!({ "range": 256 });
    let mut hidden = mimic(REVEAL);
    hidden["elite"]["range"] = json!(1);
    let mut st = quiet_stage(vec![
        wave(1, 1, 0, hidden, SIDE.0, SIDE.1),
        wave(1, 1, 0, joiner, START_X - 25_600, START_Y),
    ]);
    st.boss = Some(serde_json::from_value(boss).expect("boss parses"));
    st
}

#[test]
fn the_boss_s_call_skips_a_disguised_mimic() {
    let mut e = engine(&calling(60));
    let events = run_until(&mut e, idle(), 120, |_| false);
    let calls: Vec<_> = events
        .iter()
        .filter(|(_, ev)| matches!(ev, DomainEvent::ElitesCalled { .. }))
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].1,
        DomainEvent::ElitesCalled { elites: 1 },
        "only the joiner"
    );
    let v = mimic_view(&e);
    assert!(disguised(&v));
    assert_eq!((v.at.x.0, v.at.y.0), SIDE, "it stays where it is");
    assert!(
        events
            .iter()
            .all(|(_, ev)| !matches!(ev, DomainEvent::EliteLeft { .. }))
    );
}

#[test]
fn a_mimic_revealed_after_the_call_joins_it() {
    // Range 1: before the call a revealed mimic waits, as a grade elite does while the main
    // character is out of its range; called, it acts at any distance.
    let mut e = engine(&calling(30));
    run_until(&mut e, idle(), 40, |_| false);
    let mut events = walk_in(&mut e);
    let at = revealed(&events)[0];
    let id = mimic_view(&e).id;
    events.extend(run_until(&mut e, idle(), 40, |_| false));
    assert_eq!(
        fired(&events, id).first().copied(),
        Some(at + 1),
        "it joined the call that came before it revealed: {events:?}"
    );
}

#[test]
fn a_mimic_revealed_before_the_call_is_called_with_the_others() {
    let mut e = engine(&calling(120));
    let mut events = walk_in(&mut e);
    events.extend(run_until(&mut e, idle(), 150, |_| false));
    let calls: Vec<_> = events
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::ElitesCalled { elites } => Some(*elites),
            _ => None,
        })
        .collect();
    assert_eq!(calls, [2], "the joiner and the revealed mimic: {events:?}");
}

#[test]
fn a_revealed_mimic_beyond_its_range_waits_like_a_grade_elite() {
    // No call: range 1, the main character 50 pixels away: it does not attack.
    let mut st = calling(100_000);
    st.boss = None;
    let mut e = engine(&st);
    let mut events = walk_in(&mut e);
    events.extend(run_until(&mut e, idle(), 60, |_| false));
    let id = mimic_view(&e).id;
    assert_eq!(fired(&events, id), Vec::<u32>::new(), "{events:?}");
}

// --- Time limit runs ---------------------------------------------------------------------------

fn setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        main: "blade".into(),
        sub: "maul".into(),
        seed: 3,
        star_pool: None,
        extra_rerolls: 0,
    }
}

/// A short run whose only elite is a mimic, which comes at tick 3,600.
fn mimic_run() -> TimeLimit {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    v["length_ticks"] = json!(8_000);
    v["boss_ticks"] = json!([7_000, 8_000]);
    v["waves"] = json!([]);
    v["elite_types"] = json!([{ "id": "elite_test_mimic", "enemy": {
        "base": "bat", "hp": 240, "radius": 3072,
        "movement": { "type": "follow", "distance": 12288 },
        "elite": { "range": 256, "mimic": { "disguise": DISGUISE, "reveal_radius": REVEAL } } } }]);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_test_mimic", "weight": 1 }] }]);
    TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn the_elite_spawned_event_names_the_disguise() {
    let mode = mimic_run();
    let mut e = Engine::start_time_limit(&setup(), RunContent::bundled(), &mode).unwrap();
    let events = run_until(&mut e, idle(), 3_700, |_| false);
    let spawned: Vec<_> = events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::EliteSpawned { asset, .. } => Some((*t, asset.0.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(spawned, [(3_600, DISGUISE.to_string())]);
}

// --- Data and schema ---------------------------------------------------------------------------

#[test]
fn a_mimic_is_written_beside_the_aura_and_the_escort() {
    let value = json!({ "range": 256, "mimic": { "disguise": "golem", "reveal_radius": 30720 } });
    let elite: Elite = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(elite).unwrap(), value);
    // Absent when there is none.
    let plain: Elite = serde_json::from_value(json!({ "range": 256 })).unwrap();
    assert_eq!(
        serde_json::to_value(plain).unwrap(),
        json!({ "range": 256 })
    );
}

#[test]
fn the_bundled_mode_has_the_mimic_from_minute_6_on() {
    let mode = TimeLimit::bundled();
    let t = mode
        .elite_types()
        .iter()
        .find(|t| t.id.0 == "elite_mimic")
        .expect("elite_mimic is defined");
    let v = serde_json::to_value(&t.enemy).unwrap();
    assert_eq!(v["elite"]["mimic"]["reveal_radius"], json!(30_720));
    assert!(!t.enemy.rules.is_empty(), "it attacks once revealed");
    let weight = |tier: usize| {
        mode.elite_table()[tier]
            .weights
            .iter()
            .find(|w| w.elite.0 == "elite_mimic")
            .map(|w| w.weight)
    };
    assert_eq!(weight(0), None);
    assert_eq!(weight(1), Some(1));
    assert_eq!(weight(2), Some(1));
}

fn issues(stage: &Stage) -> Vec<String> {
    match validate::validate(stage) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

fn mimic_stage(edit: impl FnOnce(&mut Value)) -> Stage {
    let mut m = mimic(REVEAL);
    edit(&mut m);
    quiet_stage(vec![wave(1, 1, 0, m, 1000, 1000)])
}

#[test]
fn the_reveal_radius_and_the_disguise_are_bounded() {
    let p = "$.waves[0].enemy.elite.mimic";
    let with = |disguise: &str, radius: i32| {
        mimic_stage(|m| {
            m["elite"]["mimic"] = json!({ "disguise": disguise, "reveal_radius": radius })
        })
    };
    assert_eq!(issues(&with(DISGUISE, 1)), Vec::<String>::new());
    assert_eq!(
        issues(&with(DISGUISE, limits::MAX_LEN)),
        Vec::<String>::new()
    );
    assert_eq!(issues(&with(DISGUISE, 0)), [format!("{p}.reveal_radius")]);
    assert_eq!(
        issues(&with(DISGUISE, limits::MAX_LEN + 1)),
        [format!("{p}.reveal_radius")]
    );
    assert_eq!(
        issues(&with("no_such_sprite", 1)),
        [format!("{p}.disguise")]
    );
    // A palette is not a sprite.
    assert_eq!(issues(&with("ice", 1)), [format!("{p}.disguise")]);
}

#[test]
fn a_mimic_is_not_a_shrine_a_goblin_or_a_leader() {
    let e = "$.waves[0].enemy.elite";
    let shrine = json!({ "type": "shrine", "fight_ticks": 600, "guardians": {
        "enemy": enemy(5, json!({ "type": "follow", "distance": 0 }), json!([])),
        "count": 1, "radius": 100 } });
    let paths = issues(&mimic_stage(|m| m["movement"] = shrine));
    assert!(
        paths.contains(&"$.waves[0].enemy.movement".to_string()),
        "{paths:?}"
    );
    let flee = json!({ "type": "flee_when_disturbed", "disturb_radius": 100, "speed": 10,
                       "leave_after_ticks": 10 });
    let paths = issues(&mimic_stage(|m| m["movement"] = flee));
    assert!(
        paths.contains(&"$.waves[0].enemy.movement".to_string()),
        "{paths:?}"
    );
    for call in ["stay", "leave"] {
        assert_eq!(
            issues(&mimic_stage(|m| m["elite"]["boss_call"] = json!(call))),
            [format!("{e}.boss_call")],
            "{call}"
        );
    }
    assert_eq!(
        issues(&mimic_stage(
            |m| m["elite"]["aura"] = json!({ "radius": 100, "damage_percent": 10 })
        )),
        [format!("{e}.aura")]
    );
    assert_eq!(
        issues(&mimic_stage(|m| m["elite"]["escort"] = json!({
            "enemy": enemy(5, json!({ "type": "follow", "distance": 0 }), json!([])),
            "count": 1, "radius": 100 }))),
        [format!("{e}.escort")]
    );
}

#[test]
fn a_mimic_must_be_an_enemy_with_an_elite() {
    // `mimic` lives on `Elite`, so a companion or a plain enemy cannot have one: the field is
    // not in their schema.
    let mut m = enemy(10, hold(1000, 1000), json!([]));
    m["mimic"] = json!({ "disguise": DISGUISE, "reveal_radius": 100 });
    assert!(serde_json::from_value::<stella_rain_core::behaviour::Agent>(m).is_err());
}
