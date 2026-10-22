//! What a run setup carries in besides its mode, weapon and seed (ADR-107): a perk set, the
//! rune pool, the carried stars under the Lumine budget and the starting rune.
//! The setup is untrusted (ADR-020): known IDs, sorted and unique where a list says so, the
//! budget kept, and anything else fails closed with one issue per problem and its path.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::perk_sets::PerkSets;
use stella_rain_core::rewards::{Rarity, Rewards};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::{
    self, CarriedStar, DEFAULT_PERK_SET, MAX_CARRIED_STARS, MAX_LUMINE, MAX_RUNE_POOL, RunSetup,
};
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::rune::Rune;
use stella_rain_core::validate::limits::DAMAGE_SCALE;
use stella_rain_core::validate::{Error, Issue};

use crate::common;

use common::run::{plain_content, plain_setup as plain, trial_setup};

fn issues(error: Error) -> Vec<Issue> {
    match error {
        Error::Invalid(issues) => issues,
        other => panic!("not Invalid: {other:?}"),
    }
}

fn paths(error: Error) -> Vec<String> {
    issues(error).into_iter().map(|i| i.path).collect()
}

fn start(setup: &RunSetup) -> Result<Engine, Error> {
    Engine::start_with(setup, &plain_content())
}

fn carry(ids: &[&str]) -> Vec<CarriedStar> {
    ids.iter()
        .map(|id| CarriedStar {
            reward: (*id).into(),
        })
        .collect()
}

fn names(runes: &[&str]) -> Vec<stella_rain_core::id::ContentId> {
    runes.iter().map(|r| (*r).into()).collect()
}

fn hp(e: &Engine) -> u32 {
    e.snapshot().player.hp
}

fn held(e: &Engine) -> Vec<Rune> {
    e.snapshot().player.runes.iter().map(|r| r.rune).collect()
}

// --- The file: defaults and bounds --------------------------------------------------------

#[test]
fn a_setup_from_before_these_fields_reads_with_the_defaults() {
    let old =
        br#"{"schema_version":0,"sim_version":0,"mode":"trial","weapon":"giant_sword","seed":7}"#;
    let setup = run_setup::parse_and_validate(old).unwrap();
    assert_eq!(setup.perk_set.0, DEFAULT_PERK_SET);
    assert_eq!(DEFAULT_PERK_SET, "duelist");
    assert!(setup.rune_pool.is_empty());
    assert!(setup.carried.is_empty());
    assert_eq!(setup.lumine, 0);
    assert_eq!(setup.starting_rune, None);
}

#[test]
fn a_new_setup_writes_its_perk_set_and_only_the_fields_it_uses() {
    let text = serde_json::to_string(&trial_setup()).unwrap();
    assert!(text.contains("\"perk_set\":\"duelist\""), "{text}");
    for field in ["rune_pool", "carried", "lumine", "starting_rune"] {
        assert!(!text.contains(field), "{field} in {text}");
    }
    let full = RunSetup {
        rune_pool: names(&["bleed", "explosion"]),
        carried: carry(&["max_hp_common"]),
        lumine: 1,
        starting_rune: Some("explosion".into()),
        difficulty: "normal".into(),
        ..trial_setup()
    };
    let bytes = serde_json::to_vec(&full).unwrap();
    assert_eq!(run_setup::parse_and_validate(&bytes), Ok(full));
}

#[test]
fn the_rune_pool_names_known_runes_sorted_each_once() {
    let at = |runes: &[&str]| {
        run_setup::validate(&RunSetup {
            rune_pool: names(runes),
            ..trial_setup()
        })
    };
    assert!(at(&["bleed", "explosion", "thunder"]).is_ok());
    let all: Vec<&str> = {
        let mut v: Vec<&str> = vec![
            "bleed",
            "boomerang",
            "burn",
            "explosion",
            "homing_missiles",
            "poison",
            "thunder",
        ];
        v.sort_unstable();
        v
    };
    assert_eq!(all.len(), MAX_RUNE_POOL);
    assert!(at(&all).is_ok());
    assert_eq!(
        paths(at(&["explosion", "bleed"]).unwrap_err()),
        ["$.rune_pool[1]"]
    );
    assert_eq!(
        paths(at(&["bleed", "bleed"]).unwrap_err()),
        ["$.rune_pool[1]"]
    );
    assert_eq!(
        paths(at(&["bleed", "no_such_rune"]).unwrap_err()),
        ["$.rune_pool[1]"]
    );
    let many = vec!["bleed"; MAX_RUNE_POOL + 1];
    assert_eq!(paths(at(&many).unwrap_err())[0], "$.rune_pool");
}

#[test]
fn at_most_five_stars_are_carried_and_lumine_is_at_most_ten() {
    let carrying = |n: usize, lumine: u8| {
        run_setup::validate(&RunSetup {
            carried: carry(&vec!["max_hp_common"; n]),
            lumine,
            ..trial_setup()
        })
    };
    assert_eq!(MAX_CARRIED_STARS, 5);
    assert_eq!(MAX_LUMINE, 10);
    assert!(carrying(MAX_CARRIED_STARS, 10).is_ok());
    assert_eq!(
        paths(carrying(MAX_CARRIED_STARS + 1, 10).unwrap_err()),
        ["$.carried"]
    );
    assert_eq!(paths(carrying(1, 11).unwrap_err()), ["$.lumine"]);
    let bad_id = RunSetup {
        carried: carry(&["Bad Id"]),
        lumine: 1,
        ..trial_setup()
    };
    assert_eq!(
        paths(run_setup::validate(&bad_id).unwrap_err()),
        ["$.carried[0].reward"]
    );
}

#[test]
fn the_starting_rune_is_a_known_rune_of_the_pool_when_there_is_one() {
    let with = |rune: &str, pool: &[&str]| {
        run_setup::validate(&RunSetup {
            starting_rune: Some(rune.into()),
            difficulty: "normal".into(),
            rune_pool: names(pool),
            ..trial_setup()
        })
    };
    assert!(with("explosion", &[]).is_ok(), "no pool: every rune");
    assert!(with("explosion", &["bleed", "explosion"]).is_ok());
    assert_eq!(
        paths(with("thunder", &["bleed", "explosion"]).unwrap_err()),
        ["$.starting_rune"]
    );
    assert_eq!(
        paths(with("no_such_rune", &[]).unwrap_err()),
        ["$.starting_rune"]
    );
}

#[test]
fn the_costs_run_from_one_to_ten_by_rarity() {
    let costs: Vec<u8> = Rarity::ALL
        .iter()
        .map(|r| run_setup::lumine_cost(*r))
        .collect();
    assert_eq!(costs, [1, 2, 3, 4, 6, 10]);
}

// --- Against the content -------------------------------------------------------------------

#[test]
fn an_unknown_perk_set_does_not_start() {
    let setup = RunSetup {
        perk_set: "no_such_set".into(),
        ..trial_setup()
    };
    assert_eq!(paths(start(&setup).unwrap_err()), ["$.perk_set"]);
}

#[test]
fn carried_stars_are_rewards_of_the_file_and_of_the_pool_within_the_budget() {
    let carrying = |ids: &[&str], lumine: u8| RunSetup {
        carried: carry(ids),
        lumine,
        ..plain()
    };
    assert!(start(&carrying(&["max_hp_common", "speed_common"], 2)).is_ok());
    assert_eq!(
        paths(start(&carrying(&["no_such_reward"], 5)).unwrap_err()),
        ["$.carried[0].reward"]
    );
    // Common 1, rare 3, legendary 6.
    assert!(start(&carrying(&["max_hp_rare"], 3)).is_ok());
    assert_eq!(
        paths(start(&carrying(&["max_hp_rare"], 2)).unwrap_err()),
        ["$.carried"]
    );
    assert!(
        start(&carrying(
            &["max_hp_legendary", "max_hp_rare", "speed_common"],
            10
        ))
        .is_ok()
    );
    assert_eq!(
        paths(
            start(&carrying(
                &["max_hp_legendary", "max_hp_rare", "speed_common"],
                9
            ))
            .unwrap_err()
        ),
        ["$.carried"]
    );
    let outside = RunSetup {
        star_pool: Some(names(&["speed_common"])),
        ..carrying(&["max_hp_common"], 1)
    };
    assert_eq!(paths(start(&outside).unwrap_err()), ["$.carried[0].reward"]);
}

#[test]
fn a_reward_is_carried_no_more_times_than_it_stacks() {
    let rewards = Rewards::parse(
        json!({ "schema_version": 0, "sim_version": 0, "rewards": [{
            "id": "once", "rarity": "common", "effect": "max_hp", "amount": 100, "limit": 2,
            "title": "Once", "description": ""
        }]})
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
    let setup = |n: usize| RunSetup {
        carried: carry(&vec!["once"; n]),
        lumine: 5,
        ..trial_setup()
    };
    assert!(Engine::start_with(&setup(2), &content).is_ok());
    assert_eq!(
        paths(Engine::start_with(&setup(3), &content).unwrap_err()),
        ["$.carried[2].reward"]
    );
}

#[test]
fn the_random_rune_is_not_a_carried_star() {
    let rewards = Rewards::parse(
        json!({ "schema_version": 0, "sim_version": 0, "rewards": [{
            "id": "random_rune", "rarity": "common", "effect": "random_rune",
            "title": "Rune", "description": ""
        }]})
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
    let setup = RunSetup {
        carried: carry(&["random_rune"]),
        lumine: 1,
        ..trial_setup()
    };
    assert_eq!(
        paths(Engine::start_with(&setup, &content).unwrap_err()),
        ["$.carried[0].reward"]
    );
}

// --- Perk sets at tick 0 -------------------------------------------------------------------

#[test]
fn vanguard_adds_300_shown_hit_points_and_duelist_none() {
    let content = plain_content();
    let base = hp(&start(&plain()).unwrap());
    let with = |set: &str| {
        let setup = RunSetup {
            perk_set: set.into(),
            ..trial_setup()
        };
        hp(&Engine::start_with(&setup, &content).unwrap())
    };
    assert_eq!(with("vanguard"), base + 300 * DAMAGE_SCALE);
    assert_eq!(with("duelist"), base);
    assert_eq!(with("scout"), base);
}

/// The trial run from tick 0 under `script` for `ticks` ticks: the hash after each.
fn hashes(mut e: Engine, ticks: u32, script: impl Fn(u32) -> Input) -> Vec<u64> {
    (0..ticks)
        .map(|t| {
            e.step(script(t));
            e.state_hash()
        })
        .collect()
}

fn script(tick: u32) -> Input {
    common::run::run_script(tick)
}

/// `script` with no press of `main` or `smash` by hand, and walking up the field for the first
/// 120 ticks to the bats: auto mode has the combos to itself and a target within the Sword's
/// reach. (With `script`'s own hand combos under the Sword's five-A tree, auto mode finds no
/// target in the first 600 ticks, measured, so the two runs were the same run.)
fn idle_script(tick: u32) -> Input {
    Input {
        main: false,
        smash: false,
        move_y: if tick < 120 { -127 } else { 0 },
        ..script(tick)
    }
}

#[test]
fn a_perk_set_changes_the_run_and_the_same_one_always_runs_the_same() {
    let content = plain_content();
    let run = |set: &str| {
        let setup = RunSetup {
            perk_set: set.into(),
            ..trial_setup()
        };
        hashes(Engine::start_with(&setup, &content).unwrap(), 400, script)
    };
    assert_eq!(run("duelist"), run("duelist"));
    assert_ne!(run("duelist"), run("plain"));
    assert_ne!(run("scout"), run("plain"));
    assert_ne!(run("vanguard"), run("plain"));
}

#[test]
fn regen_gives_back_its_hit_points_every_second_never_beyond_the_most() {
    let content = plain_content();
    // The same run under `vanguard` and under a set with its 300 shown hit points and no regen.
    let mut file: Value = serde_json::from_slice(PerkSets::BUNDLED).unwrap();
    file["perk_sets"].as_array_mut().unwrap().push(json!({
        "id": "no_regen", "title": "No regen", "description": "",
        "effects": [{ "effect": "max_hp", "amount": 300 * DAMAGE_SCALE }]
    }));
    let content_no_regen = content
        .clone()
        .with_perk_sets(PerkSets::parse(file.to_string().as_bytes()).unwrap());
    let trace = |set: &str, content: &RunContent| {
        let setup = RunSetup {
            perk_set: set.into(),
            ..trial_setup()
        };
        let mut e = Engine::start_with(&setup, content).unwrap();
        let mut out = Vec::new();
        for t in 0..1200 {
            e.step(Input::default());
            out.push(hp(&e));
            if e.offer_open() {
                break;
            }
            let _ = t;
        }
        out
    };
    let with = trace("vanguard", &content);
    let without = trace("no_regen", &content_no_regen);
    assert!(
        with.iter().zip(&without).any(|(a, b)| a > b),
        "regen never showed"
    );
    assert!(
        with.iter().zip(&without).all(|(a, b)| a >= b),
        "regen never costs"
    );
    let most = hp(&start(&plain()).unwrap()) + 300 * DAMAGE_SCALE;
    assert!(with.iter().all(|h| *h <= most), "never beyond the most");
}

#[test]
fn scholar_adds_two_rerolls_and_a_banish_to_an_offer() {
    let offer = |set: &str| {
        let setup = RunSetup {
            perk_set: set.into(),
            ..trial_setup()
        };
        let mut e = Engine::start_with(&setup, &plain_content()).unwrap();
        for c in common::run::play_run_in(&setup, &plain_content(), common::run::TRIAL_LENGTH) {
            match c {
                Command::Step(input) => e.step(input),
                Command::Spend(index) => e.spend(index).unwrap(),
                Command::Choose(_) => break,
            }
            if e.offer_open() {
                let o = e.snapshot().progress.unwrap().offer.unwrap();
                return (o.rerolls_left, o.banishes_left);
            }
        }
        panic!("no offer");
    };
    let (rerolls, banishes) = offer("plain");
    assert_eq!(offer("scholar"), (rerolls + 2, banishes + 1));
}

// --- Rune pool, carried stars, starting rune -------------------------------------

/// The setup's bytes with `"auto_combo":<value>` added, as a setup written before the field was
/// removed carries it.
fn with_legacy_auto_combo(setup: &RunSetup, value: bool) -> Vec<u8> {
    let mut json: Value = serde_json::to_value(setup).unwrap();
    json["auto_combo"] = Value::Bool(value);
    serde_json::to_vec(&json).unwrap()
}

#[test]
fn a_setup_still_carrying_auto_combo_false_parses_and_plays_as_before() {
    let setup = plain();
    let old = with_legacy_auto_combo(&setup, false);
    let parsed = run_setup::parse_and_validate(&old).unwrap();
    assert_eq!(parsed, setup);
    assert_eq!(serde_json::from_slice::<RunSetup>(&old).unwrap(), setup);
    let text = serde_json::to_string(&parsed).unwrap();
    assert!(!text.contains("auto_combo"), "{text}");
    assert_eq!(
        hashes(start(&parsed).unwrap(), 300, script),
        hashes(start(&setup).unwrap(), 300, script)
    );
}

#[test]
fn a_setup_carrying_auto_combo_true_is_refused_naming_the_field() {
    let old = with_legacy_auto_combo(&plain(), true);
    assert_eq!(
        paths(run_setup::parse_and_validate(&old).unwrap_err()),
        ["$.auto_combo"]
    );
    let error = serde_json::from_slice::<RunSetup>(&old).unwrap_err();
    assert!(error.to_string().contains("auto_combo"), "{error}");
}

#[test]
fn auto_mode_is_on_only_while_the_input_says_so() {
    let run = |auto: bool| {
        hashes(start(&plain()).unwrap(), 600, |t| Input {
            auto,
            ..idle_script(t)
        })
    };
    let off = run(false);
    let on = run(true);
    assert_ne!(off, on, "auto mode changes the run");
    let from_tick_0 = hashes(start(&plain()).unwrap(), 600, |t| Input {
        auto: true,
        ..idle_script(t)
    });
    assert_eq!(on, from_tick_0);
}

#[test]
fn a_carried_star_applies_at_tick_0_as_if_picked() {
    let base = hp(&start(&plain()).unwrap());
    let setup = RunSetup {
        carried: carry(&["max_hp_common", "max_hp_common", "max_hp_rare"]),
        lumine: 5,
        ..plain()
    };
    let e = start(&setup).unwrap();
    assert_eq!(hp(&e), base + (200 + 200 + 500) * DAMAGE_SCALE);
    assert_eq!(e.tick(), 0);
    assert!(e.events().is_empty());
}

#[test]
fn a_carried_star_counts_toward_its_stack_limit() {
    // `speed_common` stacks 5 times: five carried, and the run's offers never hold it.
    let setup = RunSetup {
        carried: carry(&["speed_common"; 5]),
        lumine: 5,
        star_pool: Some(names(&["speed_common", "xp_common"])),
        ..plain()
    };
    let mut e = start(&setup).unwrap();
    for c in common::run::play_run_in(&setup, &plain_content(), common::run::TRIAL_LENGTH) {
        match c {
            Command::Step(input) => e.step(input),
            Command::Spend(index) => e.spend(index).unwrap(),
            Command::Choose(_) => break,
        }
        if e.offer_open() {
            let o = e.snapshot().progress.unwrap().offer.unwrap();
            let ids: Vec<&str> = o.candidates.iter().map(|c| c.reward.0.as_str()).collect();
            assert_eq!(ids, ["xp_common"]);
            return;
        }
    }
    panic!("no offer");
}

#[test]
fn the_starting_rune_is_held_from_tick_0() {
    let setup = RunSetup {
        starting_rune: Some("explosion".into()),
        difficulty: "normal".into(),
        ..plain()
    };
    let e = start(&setup).unwrap();
    assert_eq!(held(&e), [Rune::Explosion]);
    assert_eq!(held(&start(&plain()).unwrap()), []);
}

/// Rewards that are only the random rune, so that every offer holds it.
fn random_rune_content() -> RunContent {
    let rewards = Rewards::parse(
        json!({ "schema_version": 0, "sim_version": 0, "rewards": [{
            "id": "random_rune", "rarity": "common", "effect": "random_rune",
            "title": "Rune", "description": ""
        }]})
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    RunContent::new(RunStats::bundled().clone(), rewards)
        .with_perk_sets(plain_content().perk_sets().clone())
}

/// The runes held after the trial run of `setup` has taken its offers (the first candidate each
/// time).
fn runes_after_the_run(setup: &RunSetup) -> Vec<Rune> {
    let content = random_rune_content();
    let mut e = Engine::start_with(setup, &content).unwrap();
    for c in common::run::play_run_in(setup, &content, common::run::TRIAL_LENGTH) {
        match c {
            Command::Step(input) => e.step(input),
            Command::Spend(index) => e.spend(index).unwrap(),
            Command::Choose(index) => e.choose(index).unwrap(),
        }
    }
    held(&e)
}

#[test]
fn the_random_rune_draws_only_from_the_rune_pool_and_never_the_starting_rune_again() {
    let narrowed = RunSetup {
        rune_pool: names(&["bleed", "poison"]),
        ..plain()
    };
    let drawn = runes_after_the_run(&narrowed);
    assert!(!drawn.is_empty(), "no rune was drawn");
    assert!(
        drawn
            .iter()
            .all(|r| matches!(r, Rune::Bleed | Rune::Poison)),
        "{drawn:?}"
    );

    let wide = runes_after_the_run(&plain());
    assert!(
        wide.iter()
            .any(|r| !matches!(r, Rune::Bleed | Rune::Poison)),
        "the whole pool draws other runes: {wide:?}"
    );

    let started = RunSetup {
        rune_pool: names(&["bleed", "poison"]),
        starting_rune: Some("bleed".into()),
        difficulty: "normal".into(),
        ..plain()
    };
    let drawn = runes_after_the_run(&started);
    assert_eq!(
        drawn.iter().filter(|r| **r == Rune::Bleed).count(),
        1,
        "{drawn:?}"
    );
    assert!(
        drawn
            .iter()
            .all(|r| matches!(r, Rune::Bleed | Rune::Poison))
    );
}

#[test]
fn a_setup_with_all_of_it_runs_deterministically() {
    let setup = RunSetup {
        perk_set: "vanguard".into(),
        rune_pool: names(&["bleed", "explosion", "thunder"]),
        carried: carry(&["attack_common", "speed_rare"]),
        lumine: 4,
        starting_rune: Some("thunder".into()),
        difficulty: "normal".into(),
        ..trial_setup()
    };
    let run = || hashes(Engine::start(&setup).unwrap(), 400, script);
    assert_eq!(run(), run());
    assert_ne!(
        run(),
        hashes(Engine::start(&trial_setup()).unwrap(), 400, script)
    );
}

#[test]
fn every_seed_setup_of_the_fuzz_targets_starts_a_run() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run_setup");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let setup = run_setup::parse_and_validate(&bytes).unwrap();
        assert!(Engine::start(&setup).is_ok());
        count += 1;
    }
    assert!(count >= 3, "the seeds include the one with every field");
}
