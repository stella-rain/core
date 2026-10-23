//! More Stardust earners (ADR-109, app#336 SD2): the Treasure Goblin's kill pays 10,000 stored
//! (1,000 shown) times the factor and its leaving pays nothing; a won shrine challenge pays the
//! elite base, 200, times the factor at the win tick, a lost or untouched one nothing; an enemy
//! summoned by a boss or a splitter pays the regular base, 10, times the factor. Guardians,
//! escorts and a boss's parts earn nothing. The bases are the mode file's.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::run_replay::{Command, record_run, verify_run};
use stella_rain_core::run_setup;
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::{self, Error, limits};

use crate::common;

use crate::run_play::play_run_from;
use common::run::time_limit_setup;
use common::*;

const ENEMY_Y: i32 = 100_000;
/// 100 pixels to the side of the main character: out of its line of fire.
const SIDE: (i32, i32) = (START_X + 25_600, START_Y);
/// Tick 60 changes the factor, so a kill before it and one after it differ.
const TABLE: [(u32, u32); 2] = [(0, 100), (60, 250)];

fn pct_at(tick: u32) -> u32 {
    TABLE
        .iter()
        .rev()
        .find(|&&(start, _)| start <= tick)
        .map(|&(_, pct)| pct)
        .unwrap()
}

/// `stage` with the time factor `TABLE` and, if any, a base for summoned enemies.
fn counting(mut stage: Stage, summon_base: Option<u32>) -> Stage {
    let tiers: Vec<Value> = TABLE
        .iter()
        .map(|&(start_tick, pct)| json!({ "start_tick": start_tick, "pct": pct }))
        .collect();
    let mut factor = json!({ "tiers": tiers });
    if let Some(base) = summon_base {
        factor["summon_base"] = json!(base);
    }
    stage.stardust = Some(serde_json::from_value(factor).expect("a factor parses"));
    stage
}

fn stage(waves: Vec<Value>, summon_base: Option<u32>) -> Stage {
    counting(
        stage_full(18_000, Some(far_boss()), waves, vec![]),
        summon_base,
    )
}

/// Steps with `input` (firing) for `ticks` ticks, returning each tick's gain as (tick, gain).
fn gains(e: &mut Engine, input: Input, ticks: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        let before = e.stardust();
        e.step(firing(input, e.tick()));
        if e.stardust() > before {
            out.push((e.tick(), e.stardust() - before));
        }
    }
    out
}

use stella_rain_core::input::Input;

// --- The bases in the bundled mode -------------------------------------------------------------

fn elite_base(id: &str) -> Option<u32> {
    TimeLimit::bundled()
        .elite_types()
        .iter()
        .find(|t| t.id.0 == id)
        .unwrap_or_else(|| panic!("no elite type {id}"))
        .stardust
}

#[test]
fn the_bundled_mode_pays_the_goblin_the_shrine_and_summons() {
    assert_eq!(elite_base("elite_goblin"), Some(10_000));
    assert_eq!(elite_base("elite_shrine"), Some(200));
    assert_eq!(TimeLimit::bundled().stardust_summon_base(), Some(10));
    let stage = run_setup::stage(&time_limit_setup()).unwrap();
    assert_eq!(stage.stardust.unwrap().summon_base, Some(10));
}

#[test]
fn the_other_elites_keep_their_base() {
    for id in ["elite_low", "elite_mid", "elite_high"] {
        assert_eq!(elite_base(id), Some(200), "{id}");
    }
}

// --- The Treasure Goblin -----------------------------------------------------------------------

fn goblin(hp: u32, leave_after_ticks: u32) -> Value {
    let mut e = enemy(
        hp,
        json!({ "type": "flee_when_disturbed", "disturb_radius": 10_240, "speed": 512,
                "leave_after_ticks": leave_after_ticks }),
        json!([]),
    );
    e["elite"] = json!({ "range": 256, "boss_call": "leave" });
    e
}

fn with_base(mut wave: Value, base: u32) -> Value {
    wave["stardust"] = json!(base);
    wave
}

#[test]
fn killing_the_goblin_pays_ten_thousand_times_the_factor() {
    // It holds still in the line of fire (not disturbed until hit) and dies to the first shots.
    let w = with_base(wave(1, 1, 0, goblin(10, 600), START_X, ENEMY_Y), 10_000);
    let mut e = engine(&stage(vec![w], Some(10)));
    let got = gains(&mut e, idle(), 120);
    assert_eq!(got.len(), 1, "{got:?}");
    let (tick, gained) = got[0];
    assert_eq!(gained, 10_000 * pct_at(tick) / 100);
}

#[test]
fn killing_the_goblin_late_pays_the_later_factor() {
    // The goblin appears at tick 70, in the second tier.
    let w = with_base(wave(70, 1, 0, goblin(10, 600), START_X, ENEMY_Y), 10_000);
    let mut e = engine(&stage(vec![w], Some(10)));
    let got = gains(&mut e, idle(), 200);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, 25_000);
}

#[test]
fn a_goblin_that_leaves_on_its_timer_pays_nothing() {
    // Disturbed at once (the main character is within the radius), it flees and leaves after 30
    // ticks; the main character does not fire.
    let near = (START_X + 5_000, START_Y);
    let w = with_base(wave(1, 1, 0, goblin(10_000, 30), near.0, near.1), 10_000);
    let mut e = engine(&stage(vec![w], Some(10)));
    let mut left = false;
    for _ in 0..200 {
        e.step(idle());
        left |= e
            .events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::EliteLeft { .. }));
    }
    assert!(left, "the goblin left");
    assert_eq!(e.stardust(), 0);
}

// --- The shrine --------------------------------------------------------------------------------

fn shrine(fight_ticks: u32, guardian_hp: u32) -> Value {
    let guardian = enemy(
        guardian_hp,
        json!({ "type": "straight", "vx": 0, "vy": 0 }),
        json!([]),
    );
    let mut e = enemy(
        10,
        json!({ "type": "shrine", "fight_ticks": fight_ticks,
                "guardians": { "enemy": guardian, "count": 2, "radius": 10_240 } }),
        json!([]),
    );
    e["radius"] = json!(3072);
    e["elite"] = json!({ "range": 256, "boss_call": "stay" });
    e
}

fn shrine_stage(shrine: Value) -> Stage {
    let w = with_base(wave(1, 1, 0, shrine, SIDE.0, SIDE.1), 200);
    stage(vec![w], Some(10))
}

/// Walks onto the shrine, then fights: east first, then west, as the shrine tests do. Returns
/// the gains as (tick, gain).
fn fight(e: &mut Engine) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for _ in 0..200 {
        out.extend(gains(e, moving(127, 0), 1));
        if e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ShrineActivated { .. }))
        {
            break;
        }
    }
    out.extend(gains(e, idle(), 60));
    out.extend(gains(e, moving(-127, 0), 120));
    out
}

#[test]
fn a_won_shrine_pays_the_elite_base_times_the_factor_at_the_win_tick() {
    let mut e = engine(&shrine_stage(shrine(600, 5)));
    let got = fight(&mut e);
    assert_eq!(
        got.len(),
        1,
        "the guardians pay nothing, the shrine once: {got:?}"
    );
    let (tick, gained) = got[0];
    assert_eq!(gained, 200 * pct_at(tick) / 100);
    assert_eq!(e.stardust(), gained);
}

#[test]
fn a_shrine_won_after_the_factor_changes_pays_the_new_factor() {
    // Appears at tick 70, so the win is in the second tier.
    let w = with_base(wave(70, 1, 0, shrine(600, 5), SIDE.0, SIDE.1), 200);
    let mut e = engine(&stage(vec![w], Some(10)));
    for _ in 0..80 {
        e.step(idle());
    }
    let got = fight(&mut e);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, 500);
}

#[test]
fn a_lost_shrine_pays_nothing() {
    // The guardians are out of reach of the fight; the timer runs out.
    let mut e = engine(&shrine_stage(shrine(30, 1_000_000)));
    let mut left = false;
    for _ in 0..300 {
        e.step(firing(moving(127, 0), e.tick()));
        left |= e
            .events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::EliteLeft { .. }));
    }
    assert!(left, "the shrine left unkilled");
    assert_eq!(e.stardust(), 0);
}

#[test]
fn an_untouched_shrine_pays_nothing() {
    let mut e = engine(&shrine_stage(shrine(600, 5)));
    assert_eq!(gains(&mut e, idle(), 300), []);
}

// --- Summoned enemies --------------------------------------------------------------------------

/// A summoner out of the line of fire: its summoned enemies walk into it and die to the shots.
fn splitter() -> Value {
    enemy(
        1_000_000,
        hold(SIDE.0, SIDE.1 - 30_000),
        json!([{ "when": always(), "target": "player", "once": true,
                 "do": { "type": "summon", "count": 2,
                         "agent": enemy(10, hold(START_X, ENEMY_Y), json!([])) } }]),
    )
}

#[test]
fn a_splitters_summons_pay_the_regular_base_times_the_factor() {
    let mut e = engine(&stage(
        vec![wave(1, 1, 0, splitter(), SIDE.0, SIDE.1 - 30_000)],
        Some(10),
    ));
    let got = gains(&mut e, idle(), 400);
    assert_eq!(got.len(), 2, "{got:?}");
    for (tick, gained) in got {
        assert_eq!(gained, 10 * pct_at(tick) / 100, "tick {tick}");
    }
}

#[test]
fn summons_pay_nothing_without_a_summon_base() {
    let mut e = engine(&stage(
        vec![wave(1, 1, 0, splitter(), SIDE.0, SIDE.1 - 30_000)],
        None,
    ));
    assert_eq!(gains(&mut e, idle(), 400), []);
}

fn boss_stage(parts: Value, rules: Value, summon_base: Option<u32>) -> Stage {
    let mut boss = json!({ "base": "golem", "hp": 1_000_000, "radius": 8192,
                           "spawn": { "x": START_X, "y": 60_000 },
                           "parts": parts, "phases": [{ "rules": rules }] });
    boss["radius"] = json!(8192);
    counting(stage_full(18_000, Some(boss), vec![], vec![]), summon_base)
}

#[test]
fn a_boss_summons_pay_the_regular_base_times_the_factor() {
    let rules = json!([{ "when": { "type": "always" }, "target": "player", "once": true,
        "do": { "type": "summon", "count": 3,
                "agent": enemy(10, hold(START_X, ENEMY_Y), json!([])) } }]);
    let mut e = engine(&boss_stage(json!([]), rules, Some(10)));
    let got = gains(&mut e, idle(), 400);
    assert_eq!(got.len(), 3, "{got:?}");
    for (tick, gained) in got {
        assert_eq!(gained, 10 * pct_at(tick) / 100, "tick {tick}");
    }
}

#[test]
fn a_broken_boss_part_pays_nothing() {
    let part = json!({ "id": "horn", "asset": "horns_2", "hp": 20,
                       "offset": { "x": 0, "y": 16_000 }, "radius": 6_000 });
    let mut e = engine(&boss_stage(json!([part]), json!([]), Some(10)));
    let mut broken = false;
    for _ in 0..300 {
        e.step(firing(idle(), e.tick()));
        broken |= e
            .events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::PartBroken { .. }));
    }
    assert!(broken, "the part broke");
    assert_eq!(e.stardust(), 0);
}

#[test]
fn an_escort_pays_nothing() {
    let escort = json!({ "enemy": enemy(5, json!({ "type": "straight", "vx": 0, "vy": 0 }), json!([])),
                         "count": 4, "radius": 20_480 });
    let mut leader = enemy(1_000_000, hold(START_X, ENEMY_Y), json!([]));
    leader["radius"] = json!(3072);
    leader["elite"] = json!({ "range": 256, "escort": escort });
    let mut e = engine(&stage(
        vec![wave(1, 1, 0, leader, START_X, ENEMY_Y)],
        Some(10),
    ));
    let mut died = 0;
    for _ in 0..300 {
        e.step(firing(idle(), e.tick()));
        died += e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::Died { .. }))
            .count();
    }
    assert!(died > 0, "an escort died");
    assert_eq!(e.stardust(), 0);
}

// --- The cap, the validator and a whole run ----------------------------------------------------

#[test]
fn the_run_total_still_saturates() {
    let w = with_base(
        wave(
            1,
            40,
            2,
            enemy(10, hold(START_X, ENEMY_Y), json!([])),
            START_X,
            ENEMY_Y,
        ),
        limits::MAX_STARDUST_BASE,
    );
    let mut stage = stage(vec![w], None);
    stage.stardust = Some(
        serde_json::from_value(
            json!({ "tiers": [{ "start_tick": 0, "pct": limits::MAX_STARDUST_PCT }] }),
        )
        .unwrap(),
    );
    let mut e = engine(&stage);
    gains(&mut e, idle(), 600);
    assert_eq!(e.stardust(), limits::MAX_RUN_STARDUST);
}

fn problems(stage: &Stage) -> Vec<String> {
    match validate::validate(stage) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(found)) => found.into_iter().map(|i| i.path).collect(),
        Err(other) => panic!("{other:?}"),
    }
}

#[test]
fn a_summon_base_is_bounded() {
    let at_most = stage(vec![], Some(limits::MAX_STARDUST_BASE));
    assert_eq!(problems(&at_most), Vec::<String>::new());
    let past = stage(vec![], Some(limits::MAX_STARDUST_BASE + 1));
    assert_eq!(problems(&past), ["$.stardust.summon_base"]);
}

#[test]
fn a_full_time_limit_run_verifies_and_pays_the_new_earners() {
    let setup = time_limit_setup();
    let commands = play_run_from(&setup, 3_000);
    let mut e = Engine::start(&setup).unwrap();
    for c in &commands {
        match c {
            Command::Step(input) => e.step(*input),
            Command::Choose(i) => e.choose(*i).unwrap(),
        }
    }
    let replay = record_run(&setup, &commands).unwrap();
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.stardust, e.stardust());
    assert!(e.stardust() > 0);
}
