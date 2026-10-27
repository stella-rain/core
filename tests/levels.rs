//! Experience, levels and stat points (ADR-079 §2, §4; DS18): defeated enemies give experience,
//! and a level-up raises a run stat (ADR-067) by its step: the first of the stat priority below
//! its limit at once, or with `pick_stats` the one a stat offer's choice names. Only a stage with
//! `levels` (a mode's) has them.

use serde_json::{Value, json};
use stella_rain_core::engine::{ChoiceError, Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::ContentId;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::run_stats::{RunStats, StatEffect};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error, limits};
use stella_rain_core::version::Versions;
use stella_rain_core::version::{SCHEMA_VERSION, SIM_VERSION};

use crate::common;
use crate::common::run::time_limit_setup;

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

fn spent(e: &Engine) -> Vec<u8> {
    e.snapshot().progress.expect("a stage with levels").spent
}

/// Steps idle, firing, until the second level, returning the events of that tick.
fn until_level_up(e: &mut Engine, max_ticks: u32) -> Vec<DomainEvent> {
    for _ in 0..max_ticks {
        e.step(firing(idle(), e.tick()));
        if level(e) > 1 {
            return e.events().to_vec();
        }
    }
    panic!("no level-up within {max_ticks} ticks");
}

/// The ID of the bundled run stat with `effect`.
fn id_of(effect: StatEffect) -> ContentId {
    RunStats::bundled()
        .stats()
        .iter()
        .find(|s| s.effect == effect)
        .expect("a bundled stat of each effect")
        .id
        .clone()
}

// --- Experience, levels and points -------------------------------------------------------------

#[test]
fn a_stage_without_levels_has_no_progress_and_nothing_to_choose() {
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
fn a_level_up_raises_the_first_stat_and_the_run_goes_on() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    assert_eq!(level(&e), 1);
    let events: Vec<_> = until_level_up(&mut e, 600)
        .into_iter()
        .filter(|ev| {
            matches!(
                ev,
                DomainEvent::LeveledUp { .. } | DomainEvent::StatRaised { .. }
            )
        })
        .collect();
    assert_eq!(
        events,
        [
            DomainEvent::LeveledUp { level: 2 },
            DomainEvent::StatRaised {
                stat: ContentId("max_hp".into()),
                spent: 1
            }
        ]
    );
    let progress = e.snapshot().progress.unwrap();
    let mut want = vec![0; RunStats::bundled().stats().len()];
    want[0] = 1;
    assert_eq!((progress.level, progress.xp, progress.spent), (2, 0, want));
    // No offer opens: the next step is the next tick, and there is nothing to choose.
    assert_eq!(e.choose(0), Err(ChoiceError::NoOffer));
    let tick = e.tick();
    e.step(idle());
    assert_eq!(e.tick(), tick + 1);
}

#[test]
fn two_levels_in_one_tick_raise_two_stats() {
    let mut e = engine(&leveled(&[1], vec![targets(1, Some(2))]));
    let events = until_level_up(&mut e, 600);
    let levels = events
        .iter()
        .filter(|ev| matches!(ev, DomainEvent::LeveledUp { .. }))
        .count();
    assert_eq!(levels, 2);
    assert_eq!((level(&e), spent(&e)[0]), (3, 2));
}

// --- What each effect does ---------------------------------------------------------------------

fn effect_stage() -> Stage {
    leveled(&[1], vec![targets(1, None)])
}

/// An engine on `stage` at its first level-up, which raised the stat with `effect`.
fn after_spending(stage: &Stage, effect: StatEffect) -> Engine {
    let mut e =
        Engine::new_with_stats(stage, RunContent::bundled(), &[id_of(effect)], false).unwrap();
    until_level_up(&mut e, 600);
    e
}

#[test]
fn max_hp_raises_the_most_hp_and_heals_as_much() {
    let with = after_spending(&effect_stage(), StatEffect::MaxHp);
    let without = after_spending(&effect_stage(), StatEffect::SpeedPct);
    // A point of Max HP is 100 shown hit points, stored ×100.
    assert_eq!(
        with.snapshot().player.hp,
        without.snapshot().player.hp + 100 * 100
    );
}

#[test]
fn attack_raises_the_damage_of_the_main_character_s_attacks() {
    // Orbs already flying keep the damage of the attack that made them.
    let hit_after = |mut e: Engine| {
        let mut damages = Vec::new();
        for _ in 0..120 {
            e.step(firing(idle(), e.tick()));
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
    let with = hit_after(after_spending(&stage, StatEffect::AttackPct));
    let without = hit_after(after_spending(&stage, StatEffect::SpeedPct));
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
        moved(after_spending(&stage, StatEffect::SpeedPct)),
        SPEED * 108 / 100
    );
    assert_eq!(moved(after_spending(&stage, StatEffect::EvadePct)), SPEED);
}

#[test]
fn evade_shortens_the_evade_s_cooldown() {
    let cooldown = |mut e: Engine| {
        e.step(Input {
            evade: true,
            ..idle()
        });
        e.snapshot().player.evade.recovering[0]
    };
    let stage = effect_stage();
    assert_eq!(
        cooldown(after_spending(&stage, StatEffect::EvadePct)),
        EVADE_COOLDOWN * 90 / 100
    );
    assert_eq!(
        cooldown(after_spending(&stage, StatEffect::SpeedPct)),
        EVADE_COOLDOWN
    );
}

#[test]
fn attack_speed_shortens_a_combo_step_spread_over_its_phases() {
    // A step of 36 ticks: a wind-up of 12, an active phase of 4 with a hit box, a
    // recovery of 20. One step of attack speed, 8%: 36 / 1.08 = 33.3, so 33 ticks; each phase
    // rounded down (11, 3, 18) and the tick left over to the largest remainder, the active
    // phase's (0.70 against 0.11 and 0.52).
    let step = json!({ "inline": [
        { "kind": "wind_up", "ticks": 12 },
        { "kind": "active", "ticks": 4,
          "hit_box": { "type": "line", "length": 65_536, "width": 2_048 } },
        { "kind": "recovery", "ticks": 20 }
    ] });
    let stage = armed(effect_stage(), weapon_ref(step));
    let phases = |mut e: Engine| {
        while !e.snapshot().player.attacks.is_empty() {
            e.step(idle());
        }
        let mut seen: Vec<(u8, u32)> = Vec::new();
        e.step(pressing_main());
        while let Some(a) = e.snapshot().player.attacks.first().cloned() {
            if seen.last().is_none_or(|p| p.0 != a.phase) {
                seen.push((a.phase, a.ticks));
            }
            e.step(idle());
        }
        seen
    };
    assert_eq!(
        phases(after_spending(&stage, StatEffect::AttackSpeedPct)),
        [(0, 11), (1, 4), (2, 18)]
    );
    assert_eq!(
        phases(after_spending(&stage, StatEffect::SpeedPct)),
        [(0, 12), (1, 4), (2, 20)]
    );
}

// --- Stats are content: a new one needs no code ------------------------------------------------

/// The bundled run stats with one more on an existing effect: `attack_plus`, 25% a point, at
/// most 2 points.
fn with_attack_plus() -> RunStats {
    let mut file: Value = serde_json::from_slice(common::bundled_file("run_stats"))
        .expect("the bundled stats are JSON");
    file["stats"].as_array_mut().unwrap().push(json!({
        "id": "attack_plus",
        "effect": "attack_pct",
        "step": 25,
        "limit": 2,
        "title": "Attack+",
        "description": "Much more damage from every attack"
    }));
    RunStats::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the stats validate")
}

#[test]
fn a_stat_added_in_the_content_works_with_no_code() {
    let stats = with_attack_plus();
    let content = RunContent::new(stats.clone(), Rewards::bundled().clone());
    let stage = leveled(&[1], vec![targets(20, None)]);
    let mut e = Engine::new_with_stats(&stage, &content, &["attack_plus".into()], false).unwrap();
    let seen = fire_until(&mut e, idle(), 1200, |e| level(e) > 3);
    assert_eq!(spent(&e).len(), stats.stats().len());
    let raised: Vec<_> = seen
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::StatRaised { stat, spent } => Some((stat.0.as_str(), *spent)),
            _ => None,
        })
        .collect();
    // At its limit of 2 the stat gives way to the stats the priority leaves out, in file order.
    assert_eq!(
        raised,
        [("attack_plus", 1), ("attack_plus", 2), ("max_hp", 1)]
    );
}

#[test]
fn stats_with_the_same_effect_add_up() {
    // Attack 10 with two steps of 25%: 15; with one step of the bundled 10% as well: 16.
    let stats = with_attack_plus();
    let damages = |priority: &[&str]| {
        let tough = wave(
            200,
            1,
            0,
            enemy(100_000, hold(START_X, ENEMY_Y), json!([])),
            START_X,
            ENEMY_Y,
        );
        let mut stage = leveled(&[1], vec![targets(3, None)]);
        stage.waves.push(serde_json::from_value(tough).unwrap());
        let priority: Vec<ContentId> = priority.iter().map(|id| (*id).into()).collect();
        let content = RunContent::new(stats.clone(), Rewards::bundled().clone());
        let mut e = Engine::new_with_stats(&stage, &content, &priority, false).unwrap();
        fire_until(&mut e, idle(), 199, |e| level(e) >= 4);
        let mut seen = Vec::new();
        for _ in 0..200 {
            e.step(firing(idle(), e.tick()));
            seen.extend(e.events().iter().filter_map(|ev| match ev {
                DomainEvent::Hit { damage, .. } => Some(*damage),
                _ => None,
            }));
        }
        seen
    };
    assert!(damages(&["attack_plus", "speed"]).contains(&15));
    assert!(damages(&["attack_plus", "attack"]).contains(&16));
}

// --- The trial mode and the validator ----------------------------------------------------------

#[test]
fn a_trial_run_levels_up_and_raises_its_stats() {
    let setup = RunSetup {
        schema_version: SCHEMA_VERSION,
        sim_version: SIM_VERSION,
        mode: "trial".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 1,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        stat_priority: Vec::new(),
        pick_stats: false,
        difficulty: "normal".into(),
    };
    // The trial run's script plays it, taking candidate 0 of every offer.
    let mut e = Engine::start(&setup).unwrap();
    assert_eq!(e.snapshot().progress.unwrap().level, 1);
    let mut raised = 0;
    while e.outcome() == Outcome::Running && e.tick() < 5400 {
        if e.offer_open() {
            e.choose(0).unwrap();
            continue;
        }
        e.step(common::run::run_input(&e, e.tick()));
        raised += e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::StatRaised { .. }))
            .count();
    }
    assert!(raised > 0, "the trial raises at least one stat");
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

// --- Time Limit's own curve and experience (XP2, DS21) ------------------------------------------

#[test]
fn time_limit_reads_its_own_curve() {
    let e = Engine::start(&time_limit_setup()).expect("a Time Limit run starts");
    let p = e.snapshot().progress.expect("a Time Limit run has levels");
    assert_eq!((p.level, p.xp_next), (1, 30));
    // The curve is 30, then 90 more for each level; the 26th level takes 27,750 in all.
    let xp = &RunContent::bundled().time_limit().levels().xp;
    assert_eq!((xp[0], xp[1], xp.len()), (30, 120, 25));
    assert_eq!(xp.iter().sum::<u32>(), 27_750);
}

/// The bundled run stats with `xp_plus`: 10% more experience a point, at most 1 point.
fn with_xp_plus() -> RunStats {
    let mut file: Value = serde_json::from_slice(common::bundled_file("run_stats"))
        .expect("the bundled stats are JSON");
    file["stats"].as_array_mut().unwrap().push(json!({
        "id": "xp_plus",
        "effect": "xp_pct",
        "step": 10,
        "limit": 1,
        "title": "Experience+",
        "description": "More experience from every kill"
    }));
    RunStats::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the stats validate")
}

#[test]
fn xp_pct_acts_on_one_kill() {
    let stats = with_xp_plus();
    let mut stage = leveled(
        &[1, 1000],
        vec![
            targets(1, Some(1)),
            wave(
                300,
                1,
                0,
                enemy(10, hold(START_X, ENEMY_Y), json!([])),
                START_X,
                ENEMY_Y,
            ),
        ],
    );
    stage.waves[1].xp = Some(10);
    let content = RunContent::new(stats, Rewards::bundled().clone());
    let mut e = Engine::new_with_stats(&stage, &content, &["xp_plus".into()], false).unwrap();
    fire_until(&mut e, idle(), 200, |e| level(e) >= 2);
    fire_until(&mut e, idle(), 600, |e| {
        e.snapshot().progress.unwrap().xp > 0
    });
    // A kill worth 10, with +10%: 11.
    assert_eq!(e.snapshot().progress.unwrap().xp, 11);
}

// --- The stat priority and the stat offer (DS18) -----------------------------------------------

/// An engine on `stage` whose level-ups follow `priority` (run stat IDs), opening an offer per
/// level with `pick_stats`.
fn engine_with_stats(stage: &Stage, priority: &[&str], pick_stats: bool) -> Engine {
    let priority: Vec<ContentId> = priority.iter().map(|id| (*id).into()).collect();
    Engine::new_with_stats(stage, RunContent::bundled(), &priority, pick_stats).unwrap()
}

fn level(e: &Engine) -> u32 {
    e.snapshot().progress.expect("a stage with levels").level
}

fn stat_offer(e: &Engine) -> Option<Vec<ContentId>> {
    e.snapshot()
        .progress
        .expect("a stage with levels")
        .stat_offer
        .map(|o| o.options)
}

fn stat_ids() -> Vec<ContentId> {
    RunStats::bundled()
        .stats()
        .iter()
        .map(|s| s.id.clone())
        .collect()
}

#[test]
fn a_level_up_spends_on_the_first_priority_stat_below_its_limit() {
    let ids = stat_ids();
    let (speed, attack) = (4, 1);
    let limit = RunStats::bundled().stats()[speed].limit;
    let stage = leveled(&[1], vec![targets(u16::from(limit) + 2, None)]);
    let mut e = engine_with_stats(&stage, &["speed", "attack"], false);
    let seen = fire_until(&mut e, idle(), 1200, |e| level(e) > u32::from(limit) + 1);
    let raised: Vec<&ContentId> = seen
        .iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::StatRaised { stat, .. } => Some(stat),
            _ => None,
        })
        .collect();
    // The first `limit` levels raise speed, the next raises attack; nothing pauses the run.
    let mut expected = vec![&ids[speed]; usize::from(limit)];
    expected.push(&ids[attack]);
    assert_eq!(raised, expected);
    let mut want = vec![0; ids.len()];
    want[speed] = limit;
    want[attack] = 1;
    assert_eq!(spent(&e), want);
    assert!(!e.offer_open());
    assert_eq!(stat_offer(&e), None);
}

#[test]
fn pick_stats_opens_one_offer_per_level_in_order() {
    let ids = stat_ids();
    // One kill worth 2 experience: two levels in one tick, so two offers, one after the other.
    let stage = leveled(&[1], vec![targets(1, Some(2))]);
    let mut e = engine_with_stats(&stage, &["speed"], true);
    fire_until(&mut e, idle(), 600, |e| e.offer_open());
    assert_eq!(level(&e), 3);
    assert_eq!(
        stat_offer(&e),
        Some(ids.clone()),
        "every stat, in file order"
    );
    assert_eq!(spent(&e), vec![0; ids.len()], "nothing is spent on its own");
    assert!(
        e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::StatOfferOpened { options } if *options == ids))
    );
    let tick = e.tick();
    e.step(idle());
    assert_eq!(
        e.tick(),
        tick,
        "the world is stopped while the offer is open"
    );
    assert_eq!(
        e.choose(u32::try_from(ids.len()).unwrap()).map_err(|_| ()),
        Err(())
    );
    // Choosing closes it, raises that stat, and opens the second level's offer.
    e.choose(1).unwrap();
    assert!(matches!(
        e.events(),
        [
            DomainEvent::StatRaised { stat, spent: 1 },
            DomainEvent::StatOfferOpened { .. }
        ] if *stat == ids[1]
    ));
    assert!(e.offer_open());
    e.choose(0).unwrap();
    assert!(!e.offer_open());
    assert_eq!(stat_offer(&e), None);
    let mut want = vec![0; ids.len()];
    (want[0], want[1]) = (1, 1);
    assert_eq!(spent(&e), want);
    assert_eq!(e.choose(0), Err(ChoiceError::NoOffer));
}

#[test]
fn every_stat_at_its_limit_gives_nothing() {
    let stats = RunStats::bundled().stats();
    let all: u32 = stats.iter().map(|s| u32::from(s.limit)).sum();
    // One kill worth all the points and 5 levels more.
    let stage = leveled(&[1], vec![targets(1, Some(all + 5))]);
    for pick_stats in [false, true] {
        let mut e = engine_with_stats(&stage, &[], pick_stats);
        fire_until(&mut e, idle(), 600, |e| level(e) > 1);
        let mut offers = 0;
        while e.offer_open() {
            e.choose(0).unwrap();
            offers += 1;
        }
        assert_eq!(level(&e), all + 6, "pick_stats {pick_stats}");
        assert_eq!(
            spent(&e),
            stats.iter().map(|s| s.limit).collect::<Vec<u8>>()
        );
        assert_eq!(offers, if pick_stats { all } else { 0 });
        assert_eq!(stat_offer(&e), None);
        e.step(idle());
        assert!(
            !e.offer_open(),
            "no offer opens for the levels past the limits"
        );
    }
}

#[test]
fn a_replay_with_a_spend_command_is_refused() {
    use stella_rain_core::base64;
    use stella_rain_core::run_replay::Command;
    use stella_rain_core::run_replay::{RunReplayError, record_run, verify_run};
    let mut replay = record_run(&common::run::trial_setup(), &[Command::Step(idle())]).unwrap();
    // The retired kind 1: tick 0, spend, stat 0.
    replay.commands = base64::encode(&[0x00, 0x01, 0x00]);
    assert!(matches!(
        verify_run(&replay.to_bytes()),
        Err(RunReplayError::BadCommands(_))
    ));
    assert!(matches!(
        replay.unpack_commands(),
        Err(RunReplayError::BadCommands(_))
    ));
}
