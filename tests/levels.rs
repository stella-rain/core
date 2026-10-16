//! Experience, levels and stat points (ADR-066 §2, §4): defeated enemies give experience, a
//! level-up gives one stat point and opens no offer, and `spend` raises a run stat (ADR-067) by
//! its step between ticks. Only a stage with `levels` (a mode's) has them.

use serde_json::{Value, json};
use stella_rain_core::engine::{ChoiceError, Engine, Outcome, SpendError};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::ContentId;
use stella_rain_core::input::Input;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::run_stats::{RunStats, StatEffect};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error, limits};
use stella_rain_core::version::{SCHEMA_VERSION, SIM_VERSION};

mod common;

use common::*;

/// Where the enemies wait: straight above the start, in the line of the Main weapon's shots.
const ENEMY_Y: i32 = 100_000;

/// `count` enemies of 10 hp (one shot each), one every 6 ticks from tick 1, waiting above the
/// main character, each worth `xp` experience (the wave's default when `None`).
fn targets(count: u16, xp: Option<u32>) -> Value {
    let mut w = wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, ENEMY_Y), json!([])),
        START_X,
        ENEMY_Y,
    );
    if let Some(xp) = xp {
        w["xp"] = json!(xp);
    }
    w
}

/// A stage with levels that need `xp` experience each (the last entry repeating), and `waves`.
fn leveled(xp: &[u32], waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    stage.levels = Some(serde_json::from_value(json!({ "xp": xp })).expect("levels parse"));
    stage
}

fn points(e: &Engine) -> u32 {
    e.snapshot().progress.expect("a stage with levels").points
}

fn spent(e: &Engine) -> Vec<u8> {
    e.snapshot().progress.expect("a stage with levels").spent
}

/// Steps idle until a point is unspent, returning the events of that tick.
fn until_point(e: &mut Engine, max_ticks: u32) -> Vec<DomainEvent> {
    for _ in 0..max_ticks {
        e.step(idle());
        if points(e) > 0 {
            return e.events().to_vec();
        }
    }
    panic!("no point within {max_ticks} ticks");
}

/// The index of the bundled run stat with `effect`.
fn stat_of(effect: StatEffect) -> u32 {
    let i = RunStats::bundled()
        .stats()
        .iter()
        .position(|s| s.effect == effect)
        .expect("a bundled stat of each effect");
    u32::try_from(i).unwrap()
}

// --- Experience, levels and points -------------------------------------------------------------

#[test]
fn a_stage_without_levels_has_no_progress_and_nothing_to_spend_or_choose() {
    let mut e = engine(&stage_full(
        18_000,
        Some(far_boss()),
        vec![targets(5, None)],
        vec![],
    ));
    for _ in 0..300 {
        e.step(idle());
    }
    assert!(e.snapshot().progress.is_none());
    assert_eq!(e.spend(0), Err(SpendError::NoPoint));
    assert_eq!(e.choose(0), Err(ChoiceError::NoOffer));
}

#[test]
fn defeated_enemies_give_their_wave_s_experience() {
    for (xp, each) in [(None, 1), (Some(3), 3)] {
        let mut e = engine(&leveled(&[1000], vec![targets(4, xp)]));
        let events = run_until(&mut e, idle(), 600, |e| {
            e.snapshot().progress.unwrap().xp >= 4 * each
        });
        let died = ticks_of(&events, |ev| matches!(ev, DomainEvent::Died { .. })).len();
        let progress = e.snapshot().progress.unwrap();
        assert_eq!((died as u32 * each, progress.level), (progress.xp, 1));
        assert_eq!(progress.xp_next, 1000);
    }
}

#[test]
fn a_level_up_gives_one_point_and_the_run_goes_on() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    assert_eq!(points(&e), 0);
    let events = until_point(&mut e, 600);
    assert!(
        events.contains(&DomainEvent::LeveledUp { level: 2 }),
        "{events:?}"
    );
    let progress = e.snapshot().progress.unwrap();
    assert_eq!((progress.level, progress.xp, progress.points), (2, 0, 1));
    assert_eq!(progress.spent, vec![0; RunStats::bundled().stats().len()]);
    // No offer opens: the next step is the next tick, and there is nothing to choose.
    assert_eq!(e.choose(0), Err(ChoiceError::NoOffer));
    let tick = e.tick();
    e.step(idle());
    assert_eq!(e.tick(), tick + 1);
}

#[test]
fn two_levels_in_one_tick_give_two_points() {
    let mut e = engine(&leveled(&[1], vec![targets(1, Some(2))]));
    let events = until_point(&mut e, 600);
    let levels = events
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::LeveledUp { .. }))
        .count();
    assert_eq!(levels, 2);
    assert_eq!((e.snapshot().progress.unwrap().level, points(&e)), (3, 2));
}

#[test]
fn unspent_points_stay_for_the_rest_of_the_run() {
    let mut e = engine(&leveled(&[1], vec![targets(3, None)]));
    run_until(&mut e, idle(), 600, |e| points(e) == 3);
    for _ in 0..300 {
        e.step(idle());
    }
    assert_eq!(points(&e), 3);
}

#[test]
fn spend_raises_a_stat_by_one_step_between_ticks() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    until_point(&mut e, 600);
    let (tick, hash) = (e.tick(), e.state_hash());
    let attack = stat_of(StatEffect::AttackPct);
    assert_eq!(e.spend(attack), Ok(()));
    assert_eq!(
        e.events(),
        [DomainEvent::StatRaised {
            stat: ContentId("attack".into()),
            spent: 1
        }]
    );
    assert_eq!(e.tick(), tick, "spending is between ticks");
    assert_ne!(e.state_hash(), hash, "a spend changes the state");
    let mut expected = vec![0; RunStats::bundled().stats().len()];
    expected[attack as usize] = 1;
    assert_eq!((points(&e), spent(&e)), (0, expected));
    e.step(idle());
    assert_eq!(e.tick(), tick + 1);
}

#[test]
fn spend_is_refused_without_a_point_and_changes_nothing() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    e.step(idle());
    let (hash, events) = (e.state_hash(), e.events().to_vec());
    assert_eq!(e.spend(0), Err(SpendError::NoPoint));
    assert_eq!((e.state_hash(), e.events()), (hash, events.as_slice()));
}

#[test]
fn spend_is_refused_for_an_index_outside_the_stats_and_changes_nothing() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    until_point(&mut e, 600);
    let hash = e.state_hash();
    let stats = u32::try_from(RunStats::bundled().stats().len()).unwrap();
    for index in [stats, u32::MAX] {
        assert_eq!(e.spend(index), Err(SpendError::OutOfRange { index, stats }));
    }
    assert_eq!((e.state_hash(), points(&e)), (hash, 1));
}

#[test]
fn spend_is_refused_for_a_stat_at_its_limit_and_changes_nothing() {
    let limit = RunStats::bundled().stats()[0].limit;
    let mut e = engine(&leveled(&[1], vec![targets(20, None)]));
    run_until(&mut e, idle(), 1200, |e| points(e) > u32::from(limit));
    for _ in 0..limit {
        e.spend(0).unwrap();
    }
    let (hash, left) = (e.state_hash(), points(&e));
    assert_eq!(e.spend(0), Err(SpendError::AtLimit { index: 0, limit }));
    assert_eq!((e.state_hash(), points(&e)), (hash, left));
    assert_eq!(e.spend(1), Ok(()), "another stat still takes the point");
}

// --- What each effect does ---------------------------------------------------------------------

fn effect_stage() -> Stage {
    leveled(&[1], vec![targets(1, None)])
}

/// An engine on `stage` at its first point, after spending it on the stat with `effect` (or
/// keeping it with `None`).
fn after_spending(stage: &Stage, effect: Option<StatEffect>) -> Engine {
    let mut e = engine(stage);
    until_point(&mut e, 600);
    if let Some(effect) = effect {
        e.spend(stat_of(effect)).unwrap();
    }
    e
}

#[test]
fn max_hp_raises_the_most_hp_and_heals_as_much() {
    let with = after_spending(&effect_stage(), Some(StatEffect::MaxHp));
    let without = after_spending(&effect_stage(), None);
    assert_eq!(with.snapshot().player.hp, without.snapshot().player.hp + 1);
}

#[test]
fn attack_raises_the_damage_of_the_main_character_s_attacks() {
    // Bullets already flying keep the damage of the attack that made them.
    let hit_after = |mut e: Engine| {
        let mut damages = Vec::new();
        for _ in 0..120 {
            e.step(idle());
            damages.extend(e.events().iter().filter_map(|ev| match ev {
                DomainEvent::Hit { damage, .. } => Some(*damage),
                _ => None,
            }));
        }
        damages
    };
    // A tough enemy from tick 60, after the spend, takes the next shots.
    let tough = wave(
        60,
        1,
        0,
        enemy(1000, hold(START_X, ENEMY_Y), json!([])),
        START_X,
        ENEMY_Y,
    );
    let mut stage = effect_stage();
    stage.waves.push(serde_json::from_value(tough).unwrap());
    let with = hit_after(after_spending(&stage, Some(StatEffect::AttackPct)));
    let without = hit_after(after_spending(&stage, None));
    assert!(
        with.contains(&11) && !without.contains(&11),
        "{with:?} vs {without:?}"
    );
}

#[test]
fn speed_raises_the_movement_speed() {
    let moved = |mut e: Engine| {
        let before = player_at(&e).0;
        e.step(moving(127, 0));
        player_at(&e).0 - before
    };
    let stage = effect_stage();
    assert_eq!(
        moved(after_spending(&stage, Some(StatEffect::SpeedPct))),
        SPEED * 108 / 100
    );
    assert_eq!(moved(after_spending(&stage, None)), SPEED);
}

#[test]
fn evade_shortens_the_evade_s_cooldown() {
    let cooldown = |mut e: Engine| {
        e.step(Input {
            evade: true,
            ..idle()
        });
        e.snapshot().player.evade.cooldown_left
    };
    let stage = effect_stage();
    assert_eq!(
        cooldown(after_spending(&stage, Some(StatEffect::EvadePct))),
        EVADE_COOLDOWN * 90 / 100
    );
    assert_eq!(cooldown(after_spending(&stage, None)), EVADE_COOLDOWN);
}

#[test]
fn haste_shortens_the_weapons_automatic_attack_cooldowns() {
    // The Main weapon attacks every 6 ticks; with one step of haste every 5.
    let starts = |mut e: Engine| {
        let mut n = 0;
        for _ in 0..60 {
            e.step(idle());
            n += e
                .snapshot()
                .player
                .attacks
                .iter()
                .filter(|a| a.slot == 0 && a.phase == 0 && a.tick == 0)
                .count();
        }
        n
    };
    let stage = effect_stage();
    assert_eq!(
        starts(after_spending(&stage, Some(StatEffect::HastePct))),
        12
    );
    assert_eq!(starts(after_spending(&stage, None)), 10);
}

// --- Stats are content: a new one needs no code ------------------------------------------------

/// The bundled run stats with one more on an existing effect: `attack_plus`, 25% a point, at
/// most 2 points.
fn with_attack_plus() -> RunStats {
    let mut file: Value =
        serde_json::from_slice(RunStats::BUNDLED).expect("the bundled stats are JSON");
    file["stats"].as_array_mut().unwrap().push(json!({
        "id": "attack_plus",
        "effect": "attack_pct",
        "step": 25,
        "limit": 2,
        "title": "Attack+",
        "description": "Much more damage from every attack"
    }));
    RunStats::parse(file.to_string().as_bytes()).expect("the stats validate")
}

#[test]
fn a_stat_added_in_the_content_works_with_no_code() {
    let stats = with_attack_plus();
    let plus = u32::try_from(stats.stats().len() - 1).unwrap();
    let mut e = Engine::new_with(&leveled(&[1], vec![targets(20, None)]), &stats).unwrap();
    run_until(&mut e, idle(), 1200, |e| points(e) >= 3);
    assert_eq!(spent(&e).len(), stats.stats().len());
    e.spend(plus).unwrap();
    assert_eq!(
        e.events(),
        [DomainEvent::StatRaised {
            stat: ContentId("attack_plus".into()),
            spent: 1
        }]
    );
    e.spend(plus).unwrap();
    assert_eq!(
        e.spend(plus),
        Err(SpendError::AtLimit {
            index: plus,
            limit: 2
        })
    );
}

#[test]
fn stats_with_the_same_effect_add_up() {
    // Attack 10 with two steps of 25%: 15; with one step of the bundled 10% as well: 16.
    let stats = with_attack_plus();
    let plus = u32::try_from(stats.stats().len() - 1).unwrap();
    let attack = stat_of(StatEffect::AttackPct);
    let damages = |spends: &[u32]| {
        let tough = wave(
            200,
            1,
            0,
            enemy(100_000, hold(START_X, ENEMY_Y), json!([])),
            START_X,
            ENEMY_Y,
        );
        let mut stage = leveled(&[1], vec![targets(4, None)]);
        stage.waves.push(serde_json::from_value(tough).unwrap());
        let mut e = Engine::new_with(&stage, &stats).unwrap();
        run_until(&mut e, idle(), 199, |e| points(e) >= 3);
        for i in spends {
            e.spend(*i).unwrap();
        }
        let mut seen = Vec::new();
        for _ in 0..200 {
            e.step(idle());
            seen.extend(e.events().iter().filter_map(|ev| match ev {
                DomainEvent::Hit { damage, .. } => Some(*damage),
                _ => None,
            }));
        }
        seen
    };
    assert!(damages(&[plus, plus]).contains(&15));
    assert!(damages(&[plus, plus, attack]).contains(&16));
}

// --- The trial mode and the validator ----------------------------------------------------------

#[test]
fn a_trial_run_levels_up_and_spends_its_points() {
    let setup = RunSetup {
        schema_version: SCHEMA_VERSION,
        sim_version: SIM_VERSION,
        mode: "trial".into(),
        main: "blaster".into(),
        sub: "blaster".into(),
        main_expertise: 0,
        sub_expertise: 0,
        seed: 1,
    };
    // A small bot: line up under the nearest enemy, then face up, where the Main weapon shoots.
    let bot = |e: &Engine| {
        let s = e.snapshot();
        let x = s.player.at.x.0;
        let nearest = s
            .entities
            .iter()
            .filter(|en| en.kind == stella_rain_core::snapshot::EntityKind::Enemy)
            .min_by_key(|en| (en.at.x.0 - x).abs());
        match nearest {
            Some(en) if (en.at.x.0 - x).abs() > 1024 => {
                moving(if en.at.x.0 > x { 127 } else { -127 }, 0)
            }
            _ => moving(0, -1),
        }
    };
    let mut e = Engine::start(&setup).unwrap();
    assert_eq!(e.snapshot().progress.unwrap().level, 1);
    let mut spends = 0;
    while e.outcome() == Outcome::Running && e.tick() < 5400 {
        let input = bot(&e);
        e.step(input);
        while points(&e) > 0 {
            e.spend(spends % 5).unwrap();
            spends += 1;
        }
    }
    assert!(spends > 0, "the trial gives at least one point");
}

fn invalid_at(stage: &Stage, path: &str) {
    match validate::validate(stage) {
        Err(Error::Invalid(issues)) => {
            assert!(issues.iter().any(|i| i.path == path), "{path}: {issues:?}")
        }
        other => panic!("{path}: expected invalid, got {other:?}"),
    }
}

#[test]
fn levels_need_one_to_the_most_entries_each_within_bounds() {
    let max = limits::MAX_LEVEL_XP;
    let n = limits::MAX_LEVELS;
    assert!(validate::validate(&leveled(&vec![max; n], vec![])).is_ok());
    invalid_at(&leveled(&[], vec![]), "$.levels.xp");
    invalid_at(&leveled(&vec![1; n + 1], vec![]), "$.levels.xp");
    invalid_at(&leveled(&[5, 0], vec![]), "$.levels.xp[1]");
    invalid_at(&leveled(&[max + 1], vec![]), "$.levels.xp[0]");
}

#[test]
fn a_wave_s_experience_is_at_most_the_limit() {
    let at = limits::MAX_ENEMY_XP;
    assert!(validate::validate(&leveled(&[5], vec![targets(1, Some(at))])).is_ok());
    invalid_at(
        &leveled(&[5], vec![targets(1, Some(at + 1))]),
        "$.waves[0].xp",
    );
}
