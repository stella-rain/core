//! Rune Haste (ADR-099 point 5): a star reward, `rune_haste`, with an amount `h` in percent, a
//! stack limit and a rarity like the others. The amounts taken add up, held to a hard cap, and
//! shorten each on-hit rune's cooldown, as it is set, and each damage-over-time status's tick
//! interval, as it is applied, to `ticks × 100 / (100 + h)` rounded up, at least 1 tick. No
//! evade, stage-skill or combo timing changes with it. Every number here is worked out by hand.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::{
    DEFAULT_MAX_RUNE_HASTE_PCT, MAX_RUNE_HASTE_PCT, Rewards, rune_haste_ticks,
};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{RunRecorder, verify_run_with};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;
use stella_rain_core::snapshot::DamageOverTimeView;
use stella_rain_core::version::Versions;

use crate::common;

use common::on_hit::*;
use common::run::{run_input, trial_setup};
use common::*;

/// The most one Rune Haste reward gives, and the most the picks add up to, in a rewards file
/// that names no cap (the cap before it was content).
const MAX_HASTE: u32 = DEFAULT_MAX_RUNE_HASTE_PCT;

fn haste(id: &str, pct: u32, limit: u8) -> Value {
    json!({
        "id": id, "rarity": "common", "effect": "rune_haste", "amount": pct, "limit": limit
    })
}

/// A runes file whose four on-hit runes all have `cooldown` and land far away, and whose bleed
/// ticks every `interval` (poison and burn never do).
fn runes(cooldown: u32, interval: u32) -> Runes {
    let on_hit = ["explosion", "thunder", "boomerang", "homing_missiles"]
        .into_iter()
        .map(|r| {
            let mut e = far(r);
            e["cooldown_ticks"] = json!(cooldown);
            e
        })
        .collect();
    runes_with(
        on_hit,
        far_dots_but(dot("bleed", interval, interval * 6, 3, 5)),
    )
}

/// The engine of the rune stage, the seven runes taken, then each of `picks` Rune Haste
/// rewards of the list (`haste{n}`) in turn.
fn holding(runes: Runes, rewards: Vec<Value>, picks: &[&str]) -> Engine {
    let mut list = vec![random_rune()];
    list.extend(rewards);
    let mut e = with(
        &stage(
            7 + u16::try_from(picks.len()).unwrap(),
            tough(0, None, None),
        ),
        &content(list, None, runes),
    );
    take_all(&mut e);
    for p in picks {
        take(&mut e, p);
    }
    e
}

/// The cooldowns left, right after the first step's hit, of the four on-hit runes.
fn cooldowns_after_a_hit(e: &mut Engine) -> Vec<u32> {
    wait_for_target(e);
    let start = e.tick();
    e.step(once(start)(start));
    [
        Rune::Explosion,
        Rune::Thunder,
        Rune::Boomerang,
        Rune::HomingMissiles,
    ]
    .into_iter()
    .map(|r| cooldown_of(e, r))
    .collect()
}

fn bleed_of(e: &Engine, id: EntityId) -> DamageOverTimeView {
    e.snapshot()
        .entities
        .into_iter()
        .find(|en| en.id == id)
        .and_then(|en| {
            en.damage_over_time
                .into_iter()
                .find(|d| d.status.as_str() == "bleed")
        })
        .expect("the bleed on the target")
}

#[test]
fn a_rune_haste_reward_is_a_stat_like_effect_bounded_like_the_others() {
    let parse = |r: Value| {
        let file = json!({ "rarities": common::rarities(), "rewards": [r] });
        Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
    };
    assert!(parse(haste("h", MAX_HASTE, 20)).is_ok());
    assert!(parse(haste("h", 1, 2)).is_ok());
    assert!(parse(haste("h", 0, 1)).is_err(), "an amount of 0");
    assert!(parse(haste("h", MAX_HASTE + 1, 1)).is_err(), "past the cap");
    assert!(parse(haste("h", 10, 0)).is_err(), "a limit of 0");
    assert!(parse(haste("h", 10, 21)).is_err(), "a limit past MAX_STACK");
    let mut no_amount = haste("h", 10, 2);
    no_amount.as_object_mut().unwrap().remove("amount");
    assert!(parse(no_amount).is_err(), "no amount");
}

#[test]
fn without_haste_the_cooldowns_and_the_interval_are_the_files() {
    let mut e = holding(runes(120, 30), vec![], &[]);
    assert_eq!(cooldowns_after_a_hit(&mut e), [120; 4]);
    let target = wait_for_target(&mut e);
    assert_eq!(bleed_of(&e, target).next_tick_in, 30);
}

#[test]
fn haste_shortens_each_on_hit_rune_s_cooldown_to_ticks_times_100_over_100_plus_h() {
    // 120 × 100 / 150 = 80; 7 × 100 / 200 = 3.5, up to 4; 30 × 100 / 125 = 24.
    for (cooldown, h, left) in [(120, 50, 80), (7, 100, 4), (30, 25, 24), (31, 25, 25)] {
        let mut e = holding(runes(cooldown, 30), vec![haste("h1", h, 2)], &["h1"]);
        assert_eq!(
            cooldowns_after_a_hit(&mut e),
            [left; 4],
            "{cooldown} at {h}"
        );
    }
}

#[test]
fn the_formula_rounds_up_and_floors_at_one_tick() {
    assert_eq!(rune_haste_ticks(100, 0), 100);
    assert_eq!(rune_haste_ticks(100, 100), 50);
    assert_eq!(rune_haste_ticks(7, 100), 4, "3.5 up");
    assert_eq!(rune_haste_ticks(10, 1), 10, "9.9 up");
    assert_eq!(rune_haste_ticks(1, MAX_HASTE), 1, "0.25 up");
    assert_eq!(rune_haste_ticks(0, 50), 1, "never zero");
    assert_eq!(rune_haste_ticks(3_600, MAX_HASTE), 900);
    assert_eq!(rune_haste_ticks(u32::MAX, 0), u32::MAX, "no overflow");
}

#[test]
fn a_cooldown_never_reaches_zero() {
    // The shortest cooldown a file allows here, its attack's 4 ticks: 4 × 100 / 400 = 1.
    let mut e = holding(runes(4, 30), vec![haste("h1", MAX_HASTE, 2)], &["h1"]);
    assert_eq!(cooldowns_after_a_hit(&mut e), [1; 4]);
}

#[test]
fn haste_shortens_a_damage_over_time_interval_as_it_is_applied() {
    // 30 × 100 / 150 = 20: the first tick 20 after the hit, then every 20.
    let mut e = holding(runes(120, 30), vec![haste("h1", 50, 2)], &["h1"]);
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 100, once(start));
    let hit = start + 1;
    let ticks: Vec<u32> = of(&seen, 3);
    assert_eq!(ticks, [hit + 20, hit + 40, hit + 60, hit + 80], "{seen:?}");
}

#[test]
fn every_damage_over_time_kind_is_shortened() {
    // Poison and burn, 600 ticks: 600 × 100 / 150 = 400.
    let mut e = holding(runes(120, 30), vec![haste("h1", 50, 2)], &["h1"]);
    let target = wait_for_target(&mut e);
    let start = e.tick();
    e.step(once(start)(start));
    let on = e
        .snapshot()
        .entities
        .into_iter()
        .find(|en| en.id == target)
        .unwrap()
        .damage_over_time;
    let next = |kind: &str| {
        on.iter()
            .find(|d| d.status.as_str() == kind)
            .unwrap()
            .next_tick_in
    };
    assert_eq!(
        (next("bleed"), next("poison"), next("burn")),
        (20, 400, 400)
    );
}

#[test]
fn haste_taken_several_times_sums_up_to_the_cap() {
    // Two of 200 add up to 400, held to 300 where it applies: 120 × 100 / 400 would be 30.
    let rewards = vec![haste("h1", 200, 3), haste("h2", 100, 2)];
    let mut e = holding(runes(120, 30), rewards.clone(), &["h1"]);
    assert_eq!(e.snapshot().player.rune_haste_pct, 200);
    assert_eq!(cooldowns_after_a_hit(&mut e), [40; 4], "120 × 100 / 300");
    let mut e = holding(runes(120, 30), rewards.clone(), &["h1", "h1"]);
    assert_eq!(e.snapshot().player.rune_haste_pct, 400, "the sum as taken");
    assert_eq!(cooldowns_after_a_hit(&mut e), [30; 4], "held to 300");
    let mut e = holding(runes(120, 30), rewards, &["h1", "h2", "h1"]);
    assert_eq!(e.snapshot().player.rune_haste_pct, 500);
    assert_eq!(cooldowns_after_a_hit(&mut e), [30; 4]);
}

#[test]
fn haste_leaves_the_combo_the_evade_and_the_stage_skill_alone() {
    // The step's hit ticks, the evade's recovery and the stage skill's cooldown, with and
    // without haste.
    let timing = |haste_picks: &[&str]| {
        let skills = json!([{ "id": "war_cry", "icon": "icon_nova",
            "levels": [{ "cooldown_ticks": 1_000, "effects": [
                { "type": "self_status", "status": "atk_up", "value_pct": 100,
                  "duration_ticks": 20 }
            ] }] }]);
        let skill = json!({ "id": "war_cry_r", "rarity": "common", "effect": "stage_skill",
            "skill": "war_cry", "amount": 1, "limit": 2 });
        let mut list = vec![random_rune(), skill, haste("h1", 100, 2)];
        list.truncate(3);
        let small = 8 + u16::try_from(haste_picks.len()).unwrap();
        let mut e = with(
            &stage(small, tough(0, None, None)),
            &content(list, Some(skills), runes(120, 30)),
        );
        take_all(&mut e);
        take(&mut e, "war_cry_r");
        for p in haste_picks {
            take(&mut e, p);
        }
        let target = wait_for_target(&mut e);
        let start = e.tick();
        e.step(Input {
            skill: true,
            evade: true,
            ..idle()
        });
        let after = e.snapshot().player;
        let skill_cooldown = after.skill.map(|s| s.cooldown_left);
        let press = |t: u32| Input {
            main: [0, 7, 14].iter().any(|d| start + 1 + d == t),
            ..idle()
        };
        let seen = hits(&mut e, target, 40, press);
        (
            of(&seen, 10),
            skill_cooldown,
            after.evade.recovering,
            after.evade.ready,
        )
    };
    let (plain, hasty) = (timing(&[]), timing(&["h1"]));
    assert_eq!(plain, hasty);
    assert!(
        plain.1.is_some_and(|c| c > 900),
        "the skill was used: {plain:?}"
    );
}

#[test]
fn a_run_with_haste_records_and_verifies() {
    // The trial with a random rune and Rune Haste; the hidden candidate taken, else the first.
    let content = content(
        vec![random_rune(), haste("rune_haste", 25, 20)],
        None,
        Runes::bundled().clone(),
    );
    let setup = RunSetup {
        seed: 20_261_011,
        ..trial_setup()
    };
    let mut recorder = RunRecorder::start_with(&setup, &content).unwrap();
    let mut taken = false;
    for t in 0..5_400 {
        while recorder.engine().offer_open() {
            let offer = recorder.engine().snapshot().progress.and_then(|p| p.offer);
            let at = offer.and_then(|o| {
                o.candidates
                    .iter()
                    .position(|c| !c.hidden && c.reward.as_str() == "rune_haste")
            });
            taken |= at.is_some();
            recorder
                .choose(u32::try_from(at.unwrap_or(0)).unwrap())
                .unwrap();
        }
        let input: Input = run_input(recorder.engine(), t);
        if recorder.step(input).is_err() {
            break;
        }
    }
    assert!(taken, "Rune Haste was taken in the run");
    assert!(recorder.engine().snapshot().player.rune_haste_pct > 0);
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
}

/// A rewards file with the cap `cap` for Rune Haste (`max_rune_haste_pct`).
fn capped(cap: u32, rewards: Vec<Value>) -> Result<Rewards, stella_rain_core::validate::Error> {
    let file = json!({
        "rarities": common::rarities(), "max_rune_haste_pct": cap, "rewards": rewards
    });
    Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT)
}

#[test]
fn the_cap_is_the_rewards_file_s_and_the_code_ceiling_bounds_it() {
    assert_eq!(Rewards::bundled().max_rune_haste_pct(), 500, "core#298");
    assert_eq!(DEFAULT_MAX_RUNE_HASTE_PCT, 300, "a file naming none");
    let none = json!({ "rarities": common::rarities(), "rewards": [haste("h", 1, 2)] });
    let none = Rewards::parse(none.to_string().as_bytes(), Versions::CURRENT).unwrap();
    assert_eq!(none.max_rune_haste_pct(), DEFAULT_MAX_RUNE_HASTE_PCT);
    // The file's cap is 1 to the ceiling.
    assert!(capped(MAX_RUNE_HASTE_PCT, vec![haste("h", 1, 2)]).is_ok());
    assert!(capped(MAX_RUNE_HASTE_PCT + 1, vec![haste("h", 1, 2)]).is_err());
    assert!(capped(0, vec![haste("h", 1, 2)]).is_err());
    // A reward gives at most the file's cap.
    assert!(capped(500, vec![haste("h", 500, 2)]).is_ok());
    assert!(capped(500, vec![haste("h", 501, 2)]).is_err());
    assert!(capped(100, vec![haste("h", 101, 2)]).is_err());
}

#[test]
fn the_picks_add_up_to_the_files_cap_where_it_applies() {
    // Cap 500: 200 + 200 + 200 = 600 is held to 500, 120 × 100 / 600 = 20; at 400 it is 24.
    let run = |cap: u32, picks: &[&str]| {
        let mut list = vec![random_rune()];
        list.push(haste("h1", 200, 3));
        let file = json!({
            "rarities": common::rarities(), "max_rune_haste_pct": cap, "rewards": list
        });
        let rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap();
        let content =
            RunContent::new(RunStats::bundled().clone(), rewards).with_runes(runes(120, 30));
        let mut e = with(
            &stage(
                7 + u16::try_from(picks.len()).unwrap(),
                tough(0, None, None),
            ),
            &content,
        );
        take_all(&mut e);
        for p in picks {
            take(&mut e, p);
        }
        cooldowns_after_a_hit(&mut e)
    };
    assert_eq!(run(500, &["h1", "h1", "h1"]), [20; 4], "600 held to 500");
    assert_eq!(run(400, &["h1", "h1", "h1"]), [24; 4], "600 held to 400");
    assert_eq!(run(500, &["h1", "h1"]), [24; 4], "400 under the cap");
}

#[test]
fn the_ceiling_holds_whatever_the_sum() {
    assert_eq!(
        rune_haste_ticks(3_600, MAX_RUNE_HASTE_PCT + 1),
        rune_haste_ticks(3_600, MAX_RUNE_HASTE_PCT)
    );
}
