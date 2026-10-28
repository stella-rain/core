//! The run setup (ADR-051 §1): a run starts from the setup alone, and the setup is untrusted
//! (ADR-020).

use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::registry::{ENTRIES, Entry, Kind};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::{
    self, MAX_EXTRA_REROLLS, MAX_SETUP_BYTES, MAX_STAR_POOL, RunSetup,
};
use stella_rain_core::validate::{self, Error, Issue};
use stella_rain_core::weapon::WeaponRef;

use crate::common;

fn setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "trial".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 20_261_010,
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
    }
}

fn bytes(setup: &RunSetup) -> Vec<u8> {
    serde_json::to_vec(setup).unwrap()
}

/// Moves, evades, charges and presses the Sub skill, so the weapons and the RNG are used.
fn script(tick: u32) -> Input {
    Input {
        move_x: if (tick / 40) % 2 == 0 { 127 } else { -127 },
        move_y: 0,
        evade: tick % 50 == 25,
        main: (tick / 45) % 3 == 1,
        smash: tick % 9 == 0,
        ..Input::default()
    }
}

/// The hash after each of `ticks` ticks.
fn hashes(mut engine: Engine, ticks: u32) -> Vec<u64> {
    (0..ticks)
        .map(|t| {
            engine.step(script(t));
            engine.state_hash()
        })
        .collect()
}

/// The snapshot after each of `ticks` ticks.
fn views(mut engine: Engine, ticks: u32) -> Vec<String> {
    (0..ticks)
        .map(|t| {
            engine.step(script(t));
            serde_json::to_string(&engine.snapshot()).unwrap()
        })
        .collect()
}

fn issues(error: Error) -> Vec<Issue> {
    match error {
        Error::Invalid(issues) => issues,
        other => panic!("not Invalid: {other:?}"),
    }
}

fn issue(path: &str, message: &str) -> Issue {
    Issue {
        path: path.into(),
        message: message.into(),
    }
}

#[test]
fn a_setup_file_reads_back_as_the_setup() {
    assert_eq!(run_setup::parse_and_validate(&bytes(&setup())), Ok(setup()));
}

#[test]
fn a_run_is_the_mode_s_stage_with_the_setup_s_weapons_and_seed() {
    let mut by_hand =
        validate::parse_and_validate(run_setup::mode_stage("trial").unwrap()).unwrap();
    by_hand.player.weapon = WeaponRef::Preset {
        id: "giant_sword".into(),
        tree: None,
    };
    by_hand.seed = 20_261_010;
    assert_eq!(run_setup::stage(&setup()), Ok(by_hand.clone()));
    // Under a perk set with no effects, as a run is without a setup.
    let (plain, content) = (common::run::plain_setup(), common::run::plain_content());
    // The state hash holds the difficulty's setup (its rarity range), which a stage run alone
    // does not have, so the runs are compared by what they show, tick by tick.
    let started = views(Engine::start_with(&plain, &content).unwrap(), 600);
    assert_eq!(
        started,
        views(Engine::new_with(&by_hand, &content).unwrap(), 600)
    );
}

#[test]
fn the_same_setup_always_runs_the_same() {
    let a = hashes(Engine::start(&setup()).unwrap(), 600);
    let b = hashes(Engine::start(&setup()).unwrap(), 600);
    assert_eq!(a, b);
}

#[test]
fn the_setup_s_seed_seeds_the_run_all_64_bits_of_it() {
    let run = |seed: u64| hashes(Engine::start(&RunSetup { seed, ..setup() }).unwrap(), 60);
    assert_ne!(run(1), run(2), "another seed, another run");
    assert_ne!(run(7), run((1 << 32) | 7), "the high 32 bits count");
}

#[test]
fn the_setup_s_weapons_arm_the_run() {
    let run = |weapon: &str| {
        hashes(
            Engine::start(&RunSetup {
                weapon: weapon.into(),
                ..setup()
            })
            .unwrap(),
            300,
        )
    };
    assert_ne!(run("giant_sword"), run("giant_hammer"));
}

#[test]
fn a_file_over_the_limit_is_not_parsed() {
    let big = vec![b' '; MAX_SETUP_BYTES + 1];
    assert_eq!(
        run_setup::parse_and_validate(&big),
        Err(Error::TooLarge {
            size: MAX_SETUP_BYTES + 1,
            limit: MAX_SETUP_BYTES
        })
    );
}

#[test]
fn only_the_setup_format_parses() {
    let text = String::from_utf8(bytes(&setup())).unwrap();
    for bad in [
        "not json".to_owned(),
        text.replace("\"seed\"", "\"secret\":1,\"seed\""),
        text.replace(",\"weapon\":\"giant_sword\"", ""),
        text.replace("20261010", "-1"),
        text.replace("20261010", "18446744073709551616"),
    ] {
        let result = run_setup::parse_and_validate(bad.as_bytes());
        assert!(
            matches!(result, Err(Error::Parse { .. })),
            "{bad}: {result:?}"
        );
    }
}

#[test]
fn a_setup_holds_no_expertise_level() {
    // Every run starts the expertise gauge at level 0 (ADR-079 §1): the setup carries none.
    let without = r#"{"schema_version":0,"sim_version":0,"mode":"trial","weapon":"giant_sword","seed":20261010}"#;
    assert!(run_setup::parse_and_validate(without.as_bytes()).is_ok());
    for field in ["\"main_expertise\":0,", "\"sub_expertise\":0,"] {
        let with = without.replace("\"seed\"", &format!("{field}\"seed\""));
        let result = run_setup::parse_and_validate(with.as_bytes());
        assert!(
            matches!(result, Err(Error::Parse { .. })),
            "{with}: {result:?}"
        );
    }
}

#[test]
fn a_setup_from_a_newer_game_is_refused() {
    let newer = RunSetup {
        sim_version: 1,
        ..setup()
    };
    assert_eq!(
        run_setup::parse_and_validate(&bytes(&newer)),
        Err(Error::UnsupportedVersion {
            schema_version: 0,
            sim_version: 1
        })
    );
    assert!(matches!(
        Engine::start(&newer),
        Err(Error::UnsupportedVersion { .. })
    ));
}

#[test]
fn every_problem_is_named_with_its_path() {
    let bad = RunSetup {
        mode: "no_such_mode".into(),
        weapon: "Not-An-Id".into(),
        ..setup()
    };
    let found = issues(run_setup::parse_and_validate(&bytes(&bad)).unwrap_err());
    assert_eq!(
        found,
        vec![
            issue("$.mode", "no such Mode ID in the registry"),
            issue(
                "$.weapon",
                "not a valid content ID (lowercase snake_case, at most 32 characters)"
            ),
        ]
    );
    assert_eq!(issues(Engine::start(&bad).unwrap_err()), found);
}

#[test]
fn an_id_newer_than_the_setup_s_sim_version_is_refused() {
    let newer: Vec<Entry> = ENTRIES
        .iter()
        .map(|e| Entry {
            introduced: if e.id == "giant_sword" {
                1
            } else {
                e.introduced
            },
            ..*e
        })
        .collect();
    let found = issues(run_setup::validate_with(&setup(), &newer).unwrap_err());
    assert_eq!(
        found,
        vec![issue(
            "$.weapon",
            "introduced in sim_version 1, newer than this setup's 0"
        )]
    );
}

#[test]
fn every_mode_has_content_that_validates_and_nothing_else_has() {
    let modes: Vec<&str> = ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Mode)
        .map(|e| e.id)
        .collect();
    assert!(modes.contains(&"trial"));
    for id in modes.iter().filter(|id| **id != "time_limit") {
        let content = run_setup::mode_stage(id).unwrap_or_else(|| panic!("{id} has no content"));
        if let Err(e) = validate::parse_and_validate(content) {
            panic!("mode {id}: {e}");
        }
    }
    for id in &modes {
        assert!(run_setup::mode_file(id).is_some(), "{id} has no file");
    }
    assert_eq!(run_setup::mode_stage("giant_sword"), None);
    assert_eq!(run_setup::mode_file("giant_sword"), None);
}

// --- The star pool and extra rerolls (ADR-079 §1) ---------------------------------------------

fn with_pool(ids: &[&str]) -> RunSetup {
    RunSetup {
        star_pool: Some(ids.iter().map(|id| (*id).into()).collect()),
        ..setup()
    }
}

#[test]
fn a_setup_without_a_pool_or_extra_rerolls_writes_neither() {
    let text = String::from_utf8(bytes(&setup())).unwrap();
    assert!(
        !text.contains("star_pool") && !text.contains("extra_rerolls"),
        "{text}"
    );
    let more = RunSetup {
        extra_rerolls: 2,
        upgrades: Vec::new(),
        ..with_pool(&["max_hp_common"])
    };
    assert_eq!(run_setup::parse_and_validate(&bytes(&more)), Ok(more));
}

#[test]
fn the_pool_names_each_reward_once_and_at_most_max_star_pool() {
    let found = issues(run_setup::validate(&with_pool(&["Bad Id", "a", "a"])).unwrap_err());
    let paths: Vec<&str> = found.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, ["$.star_pool[0]", "$.star_pool[2]"]);
    let ids: Vec<String> = (0..=MAX_STAR_POOL).map(|i| format!("r{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert!(run_setup::validate(&with_pool(&refs[..MAX_STAR_POOL])).is_ok());
    let found = issues(run_setup::validate(&with_pool(&refs)).unwrap_err());
    assert_eq!(found[0].path, "$.star_pool");
}

#[test]
fn extra_rerolls_are_at_most_max_extra_rerolls() {
    let at = |n| RunSetup {
        extra_rerolls: n,
        upgrades: Vec::new(),
        ..setup()
    };
    assert!(run_setup::validate(&at(MAX_EXTRA_REROLLS)).is_ok());
    assert_eq!(
        issues(run_setup::validate(&at(MAX_EXTRA_REROLLS + 1)).unwrap_err()),
        [issue("$.extra_rerolls", "at most 20 (found 21)")]
    );
}

#[test]
fn a_pool_naming_a_reward_the_file_does_not_hold_does_not_start() {
    let found =
        issues(Engine::start(&with_pool(&["max_hp_common", "no_such_reward"])).unwrap_err());
    assert_eq!(
        found,
        [issue(
            "$.star_pool[1]",
            "no such reward in the run's rewards file"
        )]
    );
}

#[test]
fn validate_in_refuses_a_pool_naming_a_reward_the_file_does_not_hold() {
    // The fuzz target `run_setup` found a setup that `validate_in` passed and `Engine::start`
    // refused for this: the two must agree.
    let setup = with_pool(&["max_hp_common", "no_such_reward"]);
    assert_eq!(
        issues(run_setup::validate_in(&setup, RunContent::bundled()).unwrap_err()),
        [issue(
            "$.star_pool[1]",
            "no such reward in the run's rewards file"
        )]
    );
}

/// The trial run of `common::run` up to its first offer, from `setup`.
fn to_first_offer(setup: &RunSetup) -> Engine {
    let mut engine = Engine::start(setup).unwrap();
    for c in crate::run_play::play_run(common::run::TRIAL_LENGTH) {
        match c {
            Command::Step(input) => engine.step(input),
            Command::Choose(_) => break,
        }
        if engine.offer_open() {
            return engine;
        }
    }
    panic!("no offer");
}

#[test]
fn offers_draw_only_from_the_pool_and_the_setup_adds_its_rerolls() {
    let pool = ["speed_common", "xp_common", "evade_common"];
    let narrowed = RunSetup {
        extra_rerolls: 3,
        upgrades: Vec::new(),
        ..with_pool(&pool)
    };
    let offer = |s: &RunSetup| {
        to_first_offer(s)
            .snapshot()
            .progress
            .unwrap()
            .offer
            .unwrap()
    };
    let wide = offer(&common::run::trial_setup());
    let narrow = offer(&RunSetup {
        star_pool: narrowed.star_pool.clone(),
        extra_rerolls: 3,
        upgrades: Vec::new(),
        ..common::run::trial_setup()
    });
    let mut ids: Vec<&str> = narrow
        .candidates
        .iter()
        .map(|c| c.reward.0.as_str())
        .collect();
    ids.sort_unstable();
    let mut want = pool.to_vec();
    want.sort_unstable();
    assert_eq!(ids, want);
    assert_eq!(narrow.rerolls_left, wide.rerolls_left + 3);
}

// --- Permanent upgrades (ADR-079, ADR-084 point 7) -----------------------------------------

fn with_upgrades(ids: &[&str]) -> RunSetup {
    RunSetup {
        upgrades: ids.iter().map(|id| (*id).into()).collect(),
        ..setup()
    }
}

#[test]
fn a_setup_without_upgrades_writes_none() {
    let text = String::from_utf8(bytes(&setup())).unwrap();
    assert!(!text.contains("upgrades"), "{text}");
    assert_eq!(
        run_setup::parse_and_validate(text.as_bytes())
            .unwrap()
            .upgrades,
        Vec::new()
    );
}

#[test]
fn a_setup_names_the_upgrades_the_player_holds() {
    let held = with_upgrades(&["giant_sword_aaab", "giant_sword_aab"]);
    assert_eq!(run_setup::parse_and_validate(&bytes(&held)), Ok(held));
}

#[test]
fn upgrades_are_registered_sorted_and_each_once() {
    let path = |i: usize| format!("$.upgrades[{i}]");
    let found = |ids: &[&str]| issues(run_setup::validate(&with_upgrades(ids)).unwrap_err());
    assert_eq!(
        found(&["no_such_upgrade"]),
        vec![issue(&path(0), "no such Upgrade ID in the registry")]
    );
    // A weapon is not an upgrade.
    assert_eq!(
        found(&["giant_sword"]),
        vec![issue(&path(0), "no such Upgrade ID in the registry")]
    );
    assert_eq!(
        found(&["giant_sword_aab", "giant_sword_aaab"]),
        vec![issue(
            &path(1),
            "not after the entry before it (sorted, each once)"
        )]
    );
    assert_eq!(
        found(&["giant_sword_aab", "giant_sword_aab"]),
        vec![issue(
            &path(1),
            "not after the entry before it (sorted, each once)"
        )]
    );
}

#[test]
fn upgrades_are_at_most_max_upgrades() {
    use stella_rain_core::registry::MAX_UPGRADES;
    let many = RunSetup {
        upgrades: (0..=MAX_UPGRADES)
            .map(|i| format!("u{i:02}").as_str().into())
            .collect(),
        ..setup()
    };
    let found = issues(run_setup::validate(&many).unwrap_err());
    assert_eq!(
        found[0],
        issue(
            "$.upgrades",
            &format!(
                "at most {MAX_UPGRADES} upgrades (found {})",
                MAX_UPGRADES + 1
            )
        )
    );
}

#[test]
fn the_modes_main_characters_hold_shown_hit_points_stored_times_100() {
    for mode in ["trial", "tutorial"] {
        let stage = validate::parse_and_validate(run_setup::mode_stage(mode).unwrap()).unwrap();
        assert_eq!(stage.player.hp, 500 * 100, "{mode}");
    }
}

#[test]
fn every_mode_of_the_bundled_pack_builds_a_kind_table_that_holds_what_it_spawns() {
    // A spawn looks its kind up in the table and stops if it is missing (core#271), so a run
    // through the heaviest modes, bosses included, proves the table holds every kind they bring.
    for mode in ["trial", "time_limit"] {
        let mut s = setup();
        s.mode = mode.into();
        let mut e = Engine::start(&s).unwrap();
        assert!(!e.kinds().is_empty(), "{mode}");
        for tick in 0..7_200 {
            e.step(script(tick));
        }
        let ids = e.kinds().ids();
        assert!(ids.windows(2).all(|w| w[0].0 < w[1].0), "{mode}: sorted");
        for v in e.snapshot().entities {
            let index = v.kind_index.expect("a kind");
            assert!(e.kinds().id(index).is_some(), "{mode}");
        }
    }
}
