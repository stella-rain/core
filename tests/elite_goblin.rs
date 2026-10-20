//! The Treasure Goblin (ADR-090 points 4 and 5, app#259): an elite with no attack that holds its
//! place until it is disturbed, then flees from the main character and leaves the map unkilled
//! after `leave_after_ticks`, or when a boss comes. The boss's call skips it: `boss_call` is
//! `leave` for it and `join` (the default) for every other elite.

use serde_json::{Value, json};
use stella_rain_core::behaviour::{BossCall, Elite};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{self, Error, limits};

use crate::common;

use common::*;

/// Far above the main character, in the line of its shots.
const FAR_Y: i32 = 100_000;
/// 100 pixels to the side of the main character: out of its line of fire.
const SIDE: (i32, i32) = (START_X + 25_600, START_Y);
/// 40 pixels: nearer than `SIDE`.
const NEAR: i32 = 10_240;
/// 150 pixels: farther than `SIDE`.
const FAR: i32 = 38_400;
/// Two pixels a tick.
const SPEED_FX: i32 = 512;

fn flee(disturb_radius: i32, leave_after_ticks: u32) -> Value {
    json!({ "type": "flee_when_disturbed", "disturb_radius": disturb_radius,
            "speed": SPEED_FX, "leave_after_ticks": leave_after_ticks })
}

/// A goblin of `hp`: no rules, so no attack.
fn goblin(hp: u32, disturb_radius: i32, leave_after_ticks: u32) -> Value {
    let mut e = enemy(hp, flee(disturb_radius, leave_after_ticks), json!([]));
    e["elite"] = json!({ "range": 256, "boss_call": "leave" });
    e
}

/// An ordinary elite, which the boss calls.
fn joiner() -> Value {
    let mut e = enemy(
        10,
        json!({ "type": "follow", "distance": 0 }),
        json!([rule(always(), json!("player"), telegraph(), 0)]),
    );
    e["elite"] = json!({ "range": 256 });
    e
}

fn rewards() -> Rewards {
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": [
        { "id": "c", "rarity": "common", "effect": "max_hp", "amount": 1, "limit": 9,
          "title": "Reward", "description": "" } ] });
    Rewards::parse(file.to_string().as_bytes()).expect("the test rewards validate")
}

fn stage(waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).expect("levels parse"));
    stage
}

fn with(stage: &Stage) -> Engine {
    let content = RunContent::new(RunStats::bundled().clone(), rewards());
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

fn elites(e: &Engine) -> Vec<(i32, i32)> {
    e.snapshot()
        .entities
        .iter()
        .filter(|v| v.elite)
        .map(|v| (v.at.x.0, v.at.y.0))
        .collect()
}

fn left(events: &[(u32, DomainEvent)]) -> Vec<u32> {
    ticks_of(events, |ev| matches!(ev, DomainEvent::EliteLeft { .. }))
}

fn harmed(events: &[(u32, DomainEvent)]) -> usize {
    ticks_of(events, |ev| {
        matches!(ev, DomainEvent::RuleFired { .. } | DomainEvent::Hit { .. })
    })
    .len()
}

// --- Holding, disturbance, flight --------------------------------------------------------------

#[test]
fn an_undisturbed_goblin_stays_put_and_never_attacks() {
    let mut e = with(&stage(vec![wave(
        1,
        1,
        0,
        goblin(10, NEAR, 600),
        SIDE.0,
        SIDE.1,
    )]));
    let events = run_until(&mut e, idle(), 900, |_| false);
    assert_eq!(elites(&e), [SIDE]);
    assert_eq!(harmed(&events), 0, "{events:?}");
    assert_eq!(
        left(&events),
        Vec::<u32>::new(),
        "never disturbed, never leaves"
    );
}

#[test]
fn the_main_character_coming_within_the_disturb_radius_disturbs_it() {
    let at = |radius: i32| {
        let mut e = with(&stage(vec![wave(
            1,
            1,
            0,
            goblin(10, radius, 6000),
            SIDE.0,
            SIDE.1,
        )]));
        run_until(&mut e, idle(), 30, |_| false);
        elites(&e)[0]
    };
    assert_eq!(at(NEAR), SIDE, "100 pixels away, 40 is not near");
    let fled = at(FAR);
    assert!(
        fled.0 > SIDE.0,
        "it fled away from the main character: {fled:?}"
    );
}

#[test]
fn walking_up_to_it_disturbs_it() {
    let mut e = with(&stage(vec![wave(
        1,
        1,
        0,
        goblin(10, NEAR, 6000),
        SIDE.0,
        SIDE.1,
    )]));
    // 3 pixels a tick, 60 pixels to cover before it is within 40: well past 20 ticks.
    run_until(&mut e, moving(127, 0), 10, |_| false);
    assert_eq!(elites(&e), [SIDE]);
    run_until(&mut e, moving(127, 0), 60, |_| false);
    assert!(elites(&e)[0].0 > SIDE.0, "{:?}", elites(&e));
}

#[test]
fn damage_disturbs_it_and_it_flees_directly_away() {
    // Straight above the main character, which shoots up at it; it has hit points to spare.
    let mut e = with(&stage(vec![wave(
        1,
        1,
        0,
        goblin(100_000, NEAR, 6000),
        START_X,
        FAR_Y,
    )]));
    let events = fire_until(&mut e, idle(), 200, |_| false);
    assert!(
        events
            .iter()
            .any(|(_, ev)| matches!(ev, DomainEvent::Hit { .. })),
        "the Main weapon hits it"
    );
    let (x, y) = elites(&e)[0];
    assert_eq!(
        x, START_X,
        "directly away: along the line from the main character"
    );
    assert!(y < FAR_Y, "it fled up, away: {y}");
}

#[test]
fn it_flees_at_its_speed_and_stays_on_the_map() {
    let width = with(&stage(vec![])).snapshot().map.width.0;
    // 2 pixels from the right edge, with the main character to its left.
    let at = (width - 512, START_Y);
    let mut e = with(&stage(vec![wave(
        1,
        1,
        0,
        goblin(10, limits::MAX_LEN, 6000),
        at.0,
        at.1,
    )]));
    let mut last = at.0;
    let mut steps = Vec::new();
    for _ in 0..30 {
        e.step(idle());
        let now = elites(&e);
        assert_eq!(
            now.len(),
            1,
            "on the map, not removed with the field's margin"
        );
        steps.push(now[0].0 - last);
        last = now[0].0;
        assert!(now[0].0 <= width, "{now:?}");
    }
    assert_eq!(last, width, "it ran to the edge and stopped there");
    assert!(
        steps.iter().all(|&s| (0..=SPEED_FX).contains(&s)),
        "{steps:?}"
    );
    assert!(
        steps.contains(&SPEED_FX) || steps.iter().any(|&s| s > 0),
        "{steps:?}"
    );
}

#[test]
fn it_slides_along_the_edge_when_the_main_character_is_diagonal() {
    let width = with(&stage(vec![])).snapshot().map.width.0;
    let at = (width, START_Y - 25_600);
    let mut e = with(&stage(vec![wave(
        1,
        1,
        0,
        goblin(10, limits::MAX_LEN, 6000),
        at.0,
        at.1,
    )]));
    run_until(&mut e, idle(), 20, |_| false);
    let (x, y) = elites(&e)[0];
    assert_eq!(x, width, "pinned to the right edge");
    assert!(
        y < at.1,
        "and sliding along it, away from the main character: {y}"
    );
}

// --- Leaving -----------------------------------------------------------------------------------

#[test]
fn it_leaves_after_leave_after_ticks_from_its_disturbance_without_a_reward() {
    let mut e = with(&stage(vec![wave(
        1,
        1,
        0,
        goblin(10, FAR, 90),
        SIDE.0,
        SIDE.1,
    )]));
    let mut events = Vec::new();
    let mut disturbed = 0;
    for _ in 0..400 {
        let before = elites(&e);
        e.step(idle());
        events.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        if disturbed == 0 && elites(&e) != before {
            disturbed = e.tick();
        }
    }
    assert!(disturbed > 0);
    assert_eq!(
        left(&events),
        [disturbed + 90],
        "90 ticks from the disturbance"
    );
    assert!(elites(&e).is_empty());
    assert!(
        events.iter().all(|(_, ev)| !matches!(
            ev,
            DomainEvent::Died { .. } | DomainEvent::StarDropped { .. }
        )),
        "it did not die: {events:?}"
    );
}

#[test]
fn the_leave_timer_counts_from_the_disturbance_not_from_the_spawn() {
    let mut e = with(&stage(vec![wave(
        1,
        1,
        0,
        goblin(10, NEAR, 60),
        SIDE.0,
        SIDE.1,
    )]));
    // Undisturbed for 300 ticks, far more than 60.
    let events = run_until(&mut e, idle(), 300, |_| false);
    assert_eq!(left(&events), Vec::<u32>::new());
    assert_eq!(elites(&e), [SIDE]);
}

#[test]
fn killed_before_it_leaves_it_gives_the_elite_s_reward() {
    let stars = json!({ "drop_per_mille": 0, "pity": 2, "cap": 16, "pickup_radius": 256,
                        "tiers": [{ "from_level": 1, "weights": { "common": 1 } }] });
    let mut st = stage(vec![wave(1, 1, 0, goblin(10, NEAR, 6000), START_X, FAR_Y)]);
    st.stars = Some(serde_json::from_value(stars).expect("stars parse"));
    let mut e = with(&st);
    let events = fire_until(&mut e, idle(), 300, |_| false);
    let died = ticks_of(&events, |ev| matches!(ev, DomainEvent::Died { .. }));
    let stars = ticks_of(&events, |ev| matches!(ev, DomainEvent::StarDropped { .. }));
    assert_eq!(died.len(), 1, "{events:?}");
    assert_eq!(stars.len(), 1, "an elite always drops a star: {events:?}");
    assert_eq!(left(&events), Vec::<u32>::new());
}

// --- The boss ----------------------------------------------------------------------------------

/// A stage whose boss calls its elites at tick 60, with a goblin and an ordinary elite waiting.
fn calling(call_after: u32, goblin_at: u32) -> Stage {
    let mut boss = boss_at(1_000_000, FAR_Y);
    boss["call_elites"] = json!({ "after_ticks": call_after });
    let mut st = stage(vec![
        wave(1, 1, 0, joiner(), SIDE.0, SIDE.1),
        wave(
            goblin_at,
            1,
            0,
            goblin(10, NEAR, 6000),
            START_X - 25_600,
            START_Y,
        ),
    ]);
    st.boss = Some(serde_json::from_value(boss).expect("boss parses"));
    st
}

#[test]
fn the_boss_s_call_calls_the_joining_elites_and_the_goblin_leaves_instead() {
    let mut e = with(&calling(60, 1));
    let events = run_until(&mut e, idle(), 120, |_| false);
    let calls: Vec<_> = events
        .iter()
        .filter(|(_, ev)| matches!(ev, DomainEvent::ElitesCalled { .. }))
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].1,
        DomainEvent::ElitesCalled { elites: 1 },
        "only the one that joins is called"
    );
    assert_eq!(
        left(&events),
        [calls[0].0],
        "the goblin leaves on the tick of the call"
    );
    assert_eq!(elites(&e).len(), 1, "the other elite stays and fights");
}

#[test]
fn a_goblin_that_comes_after_the_call_leaves_at_once() {
    let mut e = with(&calling(0, 100));
    let events = run_until(&mut e, idle(), 160, |_| false);
    let gone = left(&events);
    assert_eq!(gone.len(), 1, "{events:?}");
    assert!((100..=101).contains(&gone[0]), "{gone:?}");
    assert_eq!(elites(&e).len(), 1);
}

fn setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        seed: 3,
        star_pool: None,
        extra_rerolls: 0,
    }
}

/// A short run whose only elite is a goblin, which comes at tick 3,600, and whose boss comes at
/// tick `boss`.
fn goblin_run(boss: u32) -> TimeLimit {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    v["length_ticks"] = json!(boss + 600);
    v["boss_ticks"] = json!([boss, boss + 600]);
    v["waves"] = json!([]);
    v["elite_types"] = json!([{ "id": "elite_test_goblin", "enemy": {
        "base": "bat", "hp": 240, "radius": 3072,
        "movement": flee(1024, 6_000),
        "elite": { "range": 256, "boss_call": "leave" } } }]);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_test_goblin", "weight": 1 }] }]);
    TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_goblin_leaves_at_once_when_a_boss_spawns() {
    let mode = goblin_run(4_500);
    let mut e = Engine::start_time_limit(&setup(), RunContent::bundled(), &mode).unwrap();
    let events = run_until(&mut e, idle(), 4_600, |_| false);
    let spawned = ticks_of(&events, |ev| matches!(ev, DomainEvent::EliteSpawned { .. }));
    assert_eq!(spawned, [3_600]);
    assert_eq!(
        left(&events),
        [4_500],
        "it waited, undisturbed, until the boss came"
    );
    assert!(elites(&e).is_empty());
}

// --- Data and schema ---------------------------------------------------------------------------

#[test]
fn boss_call_is_join_by_default_and_leaves_the_file_unchanged() {
    let plain: Elite = serde_json::from_value(json!({ "range": 256 })).unwrap();
    assert_eq!(plain.boss_call, BossCall::Join);
    assert_eq!(
        serde_json::to_value(&plain).unwrap(),
        json!({ "range": 256 })
    );
    let named: Elite =
        serde_json::from_value(json!({ "range": 256, "boss_call": "join" })).unwrap();
    assert_eq!(named, plain);
    let leaving: Elite =
        serde_json::from_value(json!({ "range": 256, "boss_call": "leave" })).unwrap();
    assert_eq!(
        serde_json::to_value(leaving).unwrap(),
        json!({ "range": 256, "boss_call": "leave" })
    );
    assert!(serde_json::from_value::<Elite>(json!({ "range": 1, "boss_call": "x" })).is_err());
}

#[test]
fn the_bundled_mode_has_the_goblin_in_every_tier() {
    let mode = TimeLimit::bundled();
    let t = mode
        .elite_types()
        .iter()
        .find(|t| t.id.0 == "elite_goblin")
        .expect("elite_goblin is defined");
    assert!(t.enemy.rules.is_empty(), "it has no attack");
    assert_eq!(
        t.enemy.elite.as_ref().expect("an elite").boss_call,
        BossCall::Leave
    );
    assert!(matches!(
        t.enemy.movement,
        stella_rain_core::behaviour::Movement::FleeWhenDisturbed { .. }
    ));
    for tier in mode.elite_table() {
        assert!(
            tier.weights.iter().any(|w| w.elite.0 == "elite_goblin"),
            "{tier:?}"
        );
    }
}

fn issues(stage: &Stage) -> Vec<String> {
    match validate::validate(stage) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

fn goblin_stage(movement: Value) -> Stage {
    let mut g = goblin(10, NEAR, 60);
    g["movement"] = movement;
    stage(vec![wave(1, 1, 0, g, 1000, 1000)])
}

#[test]
fn the_flight_is_bounded() {
    let max = limits::MAX_LEN;
    let ticks = limits::MAX_TICKS;
    let flee = |r: i32, s: i32, t: u32| {
        json!({ "type": "flee_when_disturbed", "disturb_radius": r, "speed": s,
                "leave_after_ticks": t })
    };
    let m = "$.waves[0].enemy.movement";
    let cases: [(Value, Vec<String>); 9] = [
        (flee(1, 1, 1), vec![]),
        (flee(max, max, ticks), vec![]),
        (flee(0, 1, 1), vec![format!("{m}.disturb_radius")]),
        (flee(max + 1, 1, 1), vec![format!("{m}.disturb_radius")]),
        (flee(1, 0, 1), vec![format!("{m}.speed")]),
        (flee(1, max + 1, 1), vec![format!("{m}.speed")]),
        (flee(1, 1, 0), vec![format!("{m}.leave_after_ticks")]),
        (
            flee(1, 1, ticks + 1),
            vec![format!("{m}.leave_after_ticks")],
        ),
        (
            flee(0, 0, 0),
            vec![
                format!("{m}.disturb_radius"),
                format!("{m}.speed"),
                format!("{m}.leave_after_ticks"),
            ],
        ),
    ];
    for (movement, expected) in cases {
        assert_eq!(
            issues(&goblin_stage(movement.clone())),
            expected,
            "{movement}"
        );
    }
}

#[test]
fn fleeing_needs_an_elite() {
    let mut g = goblin(10, NEAR, 60);
    g.as_object_mut().unwrap().remove("elite");
    let st = stage(vec![wave(1, 1, 0, g, 1000, 1000)]);
    assert_eq!(issues(&st), ["$.waves[0].enemy.movement"]);
    let mut c = companion(10, flee(NEAR, 60), json!([]));
    c["movement"] = flee(NEAR, 60);
    let mut st = stage_full(600, Some(far_boss()), vec![], vec![c]);
    st.levels = None;
    assert_eq!(issues(&st), ["$.companions[0].movement"]);
}
