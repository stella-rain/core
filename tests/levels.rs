//! Experience, levels and reward offers (ADR-051 §2): defeated enemies give experience, a level-up
//! opens an offer at the end of its tick, the world stops until a choice, and a choice applies
//! its upgrade at once. Only a stage with `levels` (a mode's) has them.

use serde_json::{Value, json};
use stella_rain_core::engine::{ChoiceError, Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::stage::Stage;
use stella_rain_core::upgrade::{MAX_STACKS, OFFER_SIZE, Upgrade};
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

fn with_seed(mut stage: Stage, seed: u32) -> Stage {
    stage.seed = seed;
    stage
}

/// Steps idle until an offer opens, returning the events of that tick.
fn until_offer(e: &mut Engine, max_ticks: u32) -> Vec<DomainEvent> {
    for _ in 0..max_ticks {
        e.step(idle());
        if e.offer().is_some() {
            return e.events().to_vec();
        }
    }
    panic!("no offer within {max_ticks} ticks");
}

fn offered(e: &Engine) -> Vec<Upgrade> {
    e.offer().expect("an offer is open").to_vec()
}

/// An engine on `stage` (seed by seed) whose first offer holds `wanted`, after choosing it.
fn chose(stage: &Stage, wanted: Upgrade) -> Engine {
    for seed in 0..200 {
        let mut e = engine(&with_seed(stage.clone(), seed));
        until_offer(&mut e, 600);
        if let Some(i) = offered(&e).iter().position(|u| *u == wanted) {
            e.choose(u32::try_from(i).unwrap()).expect("a valid choice");
            return e;
        }
    }
    panic!("no seed offered {wanted:?}");
}

/// As `chose`, but choosing the first option that is not `wanted`, for the same tick.
fn chose_other(stage: &Stage, wanted: Upgrade) -> Engine {
    let mut e = engine(stage);
    until_offer(&mut e, 600);
    let i = offered(&e).iter().position(|u| *u != wanted).unwrap();
    e.choose(u32::try_from(i).unwrap()).unwrap();
    e
}

// --- Experience and levels --------------------------------------------------------------------

#[test]
fn a_stage_without_levels_has_no_progress_and_never_offers() {
    let mut e = engine(&stage_full(
        18_000,
        Some(far_boss()),
        vec![targets(5, None)],
        vec![],
    ));
    for _ in 0..300 {
        e.step(idle());
        assert!(e.offer().is_none());
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
fn a_level_up_opens_an_offer_of_different_upgrades_at_the_end_of_its_tick() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    let events = until_offer(&mut e, 600);
    let options = offered(&e);
    assert_eq!(options.len(), OFFER_SIZE as usize);
    let mut sorted = options.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), options.len(), "no upgrade twice: {options:?}");
    let level_up = events
        .iter()
        .position(|ev| *ev == DomainEvent::LeveledUp { level: 2 })
        .expect("a LeveledUp event");
    let opened = events
        .iter()
        .position(|ev| {
            *ev == DomainEvent::OfferOpened {
                options: options.clone(),
            }
        })
        .expect("an OfferOpened event");
    assert!(
        level_up < opened && opened == events.len() - 1,
        "{events:?}"
    );
    let progress = e.snapshot().progress.unwrap();
    assert_eq!((progress.level, progress.xp), (2, 0));
    assert_eq!(progress.offer, Some(options));
}

#[test]
fn while_an_offer_is_open_step_does_nothing() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    until_offer(&mut e, 600);
    let (tick, hash, events, snapshot) =
        (e.tick(), e.state_hash(), e.events().to_vec(), e.snapshot());
    for _ in 0..30 {
        e.step(moving(127, 0));
    }
    assert_eq!((e.tick(), e.state_hash()), (tick, hash));
    assert_eq!(e.events(), events.as_slice());
    assert_eq!(e.snapshot(), snapshot);
}

#[test]
fn choose_refuses_an_index_outside_the_offer_and_changes_nothing() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    until_offer(&mut e, 600);
    let hash = e.state_hash();
    for index in [OFFER_SIZE, u32::MAX] {
        assert_eq!(
            e.choose(index),
            Err(ChoiceError::OutOfRange {
                index,
                options: OFFER_SIZE
            })
        );
    }
    assert_eq!(e.state_hash(), hash);
    assert!(e.offer().is_some());
}

#[test]
fn a_choice_applies_its_upgrade_closes_the_offer_and_the_next_step_is_the_next_tick() {
    let mut e = engine(&leveled(&[2], vec![targets(4, None)]));
    until_offer(&mut e, 600);
    let upgrade = offered(&e)[1];
    let tick = e.tick();
    assert_eq!(e.choose(1), Ok(()));
    assert_eq!(e.events(), [DomainEvent::UpgradeChosen { upgrade }]);
    assert_eq!(e.tick(), tick, "a choice is between ticks");
    let progress = e.snapshot().progress.unwrap();
    assert_eq!(progress.offer, None);
    assert_eq!(progress.upgrades, vec![(upgrade, 1)]);
    assert_eq!(e.choose(0), Err(ChoiceError::NoOffer));
    e.step(idle());
    assert_eq!(e.tick(), tick + 1);
}

#[test]
fn two_levels_in_one_tick_open_two_offers_in_turn_on_that_tick() {
    let mut e = engine(&leveled(&[1], vec![targets(1, Some(2))]));
    let events = until_offer(&mut e, 600);
    assert_eq!(
        events
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::LeveledUp { .. }))
            .count(),
        2
    );
    assert_eq!(e.snapshot().progress.unwrap().level, 3);
    let tick = e.tick();
    e.choose(0).unwrap();
    assert!(e.offer().is_some(), "the second level's offer opens");
    assert!(matches!(
        e.events().last(),
        Some(DomainEvent::OfferOpened { .. })
    ));
    e.choose(0).unwrap();
    assert!(e.offer().is_none());
    assert_eq!(e.tick(), tick);
}

#[test]
fn the_same_seed_draws_the_same_offers_and_other_seeds_other_offers() {
    let stage = leveled(&[2], vec![targets(4, None)]);
    let first = |seed| {
        let mut e = engine(&with_seed(stage.clone(), seed));
        until_offer(&mut e, 600);
        offered(&e)
    };
    assert_eq!(first(7), first(7));
    let offers: Vec<_> = (0..20).map(first).collect();
    assert!(offers.iter().any(|o| *o != offers[0]), "{offers:?}");
}

#[test]
fn an_upgrade_at_its_most_stacks_leaves_the_pool_and_an_empty_pool_opens_no_offer() {
    let all = Upgrade::ALL.len() as u32 * u32::from(MAX_STACKS);
    let mut e = engine(&leveled(&[1], vec![targets(100, None)]));
    let mut chosen = 0;
    let mut levels = 0;
    for _ in 0..1200 {
        e.step(idle());
        levels += e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::LeveledUp { .. }))
            .count() as u32;
        while e.offer().is_some() {
            let options = offered(&e);
            let stacks = e.snapshot().progress.unwrap().upgrades;
            for u in &options {
                let has = stacks.iter().find(|(s, _)| s == u).map_or(0, |(_, n)| *n);
                assert!(has < MAX_STACKS, "{u:?} offered at {has} stacks");
            }
            e.choose(0).unwrap();
            chosen += 1;
        }
    }
    assert_eq!(chosen, all);
    assert!(
        levels > all,
        "levels went on after the pool ran out: {levels}"
    );
    let upgrades = e.snapshot().progress.unwrap().upgrades;
    assert_eq!(upgrades.len(), Upgrade::ALL.len());
    assert!(upgrades.iter().all(|(_, n)| *n == MAX_STACKS));
}

#[test]
fn no_offer_opens_on_the_tick_the_run_ends() {
    // The only enemy is the boss: its death clears the stage and gives no experience, and the
    // stage ends there.
    let mut stage = leveled(&[1], vec![]);
    stage.boss = Some(serde_json::from_value(boss_at(10, ENEMY_Y)).unwrap());
    let mut e = engine(&stage);
    run_until(&mut e, idle(), 600, |e| e.outcome() != Outcome::Running);
    assert_eq!(e.outcome(), Outcome::Cleared);
    assert!(e.offer().is_none());
}

// --- What each upgrade does --------------------------------------------------------------------

fn effect_stage() -> Stage {
    leveled(&[1], vec![targets(1, None)])
}

#[test]
fn max_hp_raises_the_most_hp_and_heals_as_much() {
    let e = chose(&effect_stage(), Upgrade::MaxHp);
    assert_eq!(e.snapshot().player.hp, 100);
}

#[test]
fn attack_raises_the_damage_of_the_main_character_s_attacks() {
    // Later offers are answered without taking `Attack`; bullets already flying keep the
    // damage of the attack that made them.
    let hit_after = |mut e: Engine| {
        let mut damages = Vec::new();
        for _ in 0..120 {
            if let Some(options) = e.offer() {
                let i = options.iter().position(|u| *u != Upgrade::Attack).unwrap();
                e.choose(u32::try_from(i).unwrap()).unwrap();
                continue;
            }
            e.step(idle());
            damages.extend(e.events().iter().filter_map(|ev| match ev {
                DomainEvent::Hit { damage, .. } => Some(*damage),
                _ => None,
            }));
        }
        damages
    };
    // A tough enemy from tick 60, after the choice, takes the next shots.
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
    let with = hit_after(chose(&stage, Upgrade::Attack));
    let without = hit_after(chose_other(&stage, Upgrade::Attack));
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
    assert_eq!(
        moved(chose(&effect_stage(), Upgrade::Speed)),
        SPEED * 108 / 100
    );
    assert_eq!(moved(chose_other(&effect_stage(), Upgrade::Speed)), SPEED);
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
    assert_eq!(
        cooldown(chose(&effect_stage(), Upgrade::Evade)),
        EVADE_COOLDOWN * 90 / 100
    );
    assert_eq!(
        cooldown(chose_other(&effect_stage(), Upgrade::Evade)),
        EVADE_COOLDOWN
    );
}

#[test]
fn haste_shortens_the_weapons_automatic_attack_cooldowns() {
    // The Main weapon attacks every 6 ticks; with one stack of haste every 5.
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
    assert_eq!(starts(chose(&effect_stage(), Upgrade::Haste)), 12);
    assert_eq!(starts(chose_other(&effect_stage(), Upgrade::Haste)), 10);
}

// --- The trial mode and the validator ----------------------------------------------------------

#[test]
fn a_trial_run_levels_up_and_offers() {
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
    let mut offers = 0;
    while e.outcome() == Outcome::Running && e.tick() < 5400 {
        let input = bot(&e);
        e.step(input);
        while e.offer().is_some() {
            e.choose(0).unwrap();
            offers += 1;
        }
    }
    assert!(offers > 0, "the trial offers at least one upgrade");
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
