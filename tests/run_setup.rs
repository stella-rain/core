//! The run setup (ADR-REPL-5): a run starts from the setup alone, and the setup is untrusted
//! (ADR-SVC-1).

use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::registry::{ENTRIES, Entry, Kind};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::Command;
use stella_rain_core::run_setup::{
    self, BuildMode, MAX_EXTRA_REROLLS, MAX_SETUP_BYTES, MAX_STAR_POOL, RunSetup,
};
use stella_rain_core::validate::{self, Error, Issue};
use stella_rain_core::version::{SCHEMA_VERSION, SIM_VERSION};
use stella_rain_core::weapon::WeaponRef;

use crate::common;

fn setup() -> RunSetup {
    RunSetup {
        schema_version: 1,
        sim_version: 1,
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
        difficulty: "normal".into(),
        ..RunSetup::default()
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
            engine.step(&[script(t), Default::default(), Default::default()]);
            engine.state_hash()
        })
        .collect()
}

/// The snapshot after each of `ticks` ticks, as text. `live` is an engine of the newest rules,
/// whose entity numbers leave the player slots 0 to 2 reserved: they are mapped to the numbers
/// of a version 1 engine, which does not, so that the two are compared by what they show.
fn views(mut engine: Engine, ticks: u32, live: bool) -> Vec<String> {
    (0..ticks)
        .map(|t| {
            engine.step(&[script(t), Default::default(), Default::default()]);
            let text = format!("{:?}", engine.snapshot());
            if live {
                crate::gameplay_observables::rewrite_ids(&text)
            } else {
                text
            }
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
    let started = views(Engine::start_with(&plain, &content).unwrap(), 600, true);
    assert_eq!(
        started,
        views(Engine::new_with(&by_hand, &content).unwrap(), 600, false)
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
    // Every run starts the expertise gauge at level 0 (ADR-REPL-5): the setup carries none.
    let without = r#"{"schema_version":1,"sim_version":1,"mode":"trial","weapon":"giant_sword","seed":20261010}"#;
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
        sim_version: SIM_VERSION + 1,
        ..setup()
    };
    assert_eq!(
        run_setup::parse_and_validate(&bytes(&newer)),
        Err(Error::UnsupportedVersion {
            // The refused setup's own schema (1), not the one writers write.
            schema_version: setup().schema_version,
            sim_version: SIM_VERSION + 1
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
                SIM_VERSION + 1
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
            &format!(
                "introduced in sim_version {}, newer than this setup's 1",
                SIM_VERSION + 1
            )
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

// --- The star pool and extra rerolls (ADR-REPL-5) ---------------------------------------------

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
            Command::Step(input) => engine.step(&[input, Default::default(), Default::default()]),
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
        .map(|c| c.reward.as_str())
        .collect();
    ids.sort_unstable();
    let mut want = pool.to_vec();
    want.sort_unstable();
    assert_eq!(ids, want);
    assert_eq!(narrow.rerolls_left, wide.rerolls_left + 3);
}

// --- Permanent upgrades (ADR-REPL-5) -----------------------------------------

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
            e.step(&[script(tick), Default::default(), Default::default()]);
        }
        let ids = e.kinds().ids();
        assert!(
            ids.windows(2).all(|w| w[0].as_str() < w[1].as_str()),
            "{mode}: sorted"
        );
        for v in e.snapshot().entities {
            let index = v.kind_index.expect("a kind");
            assert!(e.kinds().id(index).is_some(), "{mode}");
        }
    }
}

#[test]
fn a_setup_at_version_0_is_refused() {
    // No migration from version 0.
    for (schema_version, sim_version) in [(0, 0), (SCHEMA_VERSION, 0), (0, SIM_VERSION)] {
        let old = RunSetup {
            schema_version,
            sim_version,
            ..setup()
        };
        assert_eq!(
            run_setup::parse_and_validate(&bytes(&old)),
            Err(Error::UnsupportedVersion {
                schema_version,
                sim_version
            })
        );
    }
}

// The build mode and the Lumine star (ADR-SIM-7 points 4 and 7, ADR-SAVE-7 points 3 and 9).

const REQUIRED: &str =
    r#""schema_version":2,"sim_version":2,"mode":"trial","weapon":"giant_sword","seed":1"#;

fn parse(extra: &str) -> Result<RunSetup, Error> {
    let text = format!("{{{REQUIRED}{extra}}}");
    run_setup::parse_and_validate(text.as_bytes())
}

#[test]
fn an_offers_setup_writes_neither_new_key() {
    let s = RunSetup::default();
    assert_eq!(s.build_mode, BuildMode::Offers);
    assert_eq!(s.lumine_star, None);
    let text = serde_json::to_string(&s).unwrap();
    assert!(!text.contains("build_mode"), "{text}");
    assert!(!text.contains("lumine_star"), "{text}");
}

#[test]
fn a_setup_file_written_before_the_fields_keeps_its_keys() {
    for name in ["trial", "trial_loadout", "trial_upgrades"] {
        let path =
            crate::data_root::data_root().join(format!("tests/fixtures/run_setup/{name}.json"));
        let text = std::fs::read(&path).unwrap();
        let parsed = run_setup::parse_and_validate(&text).unwrap();
        assert_eq!(parsed.build_mode, BuildMode::Offers, "{name}");
        let written: serde_json::Value = serde_json::to_value(&parsed).unwrap();
        let again = run_setup::parse_and_validate(&serde_json::to_vec(&parsed).unwrap());
        assert_eq!(again, Ok(parsed), "{name}");
        for key in written.as_object().unwrap().keys() {
            assert!(
                key != "build_mode" && key != "lumine_star",
                "{name}: new key {key}"
            );
        }
    }
}

#[test]
fn the_build_mode_and_the_lumine_star_round_trip() {
    let equipment = parse(r#","build_mode":"equipment""#).unwrap();
    assert_eq!(equipment.build_mode, BuildMode::Equipment);
    let text = serde_json::to_string(&equipment).unwrap();
    assert!(text.contains(r#""build_mode":"equipment""#), "{text}");

    let offers = parse(r#","build_mode":"offers""#).unwrap();
    assert_eq!(offers.build_mode, BuildMode::Offers);
    assert!(
        !serde_json::to_string(&offers)
            .unwrap()
            .contains("build_mode")
    );

    assert!(parse(r#","build_mode":"cards""#).is_err());

    let star = RunSetup {
        build_mode: BuildMode::Equipment,
        lumine: 1,
        lumine_star: Some("max_hp_common".into()),
        ..RunSetup::default()
    };
    let text = serde_json::to_string(&star).unwrap();
    assert!(text.contains(r#""build_mode":"equipment""#), "{text}");
    assert!(text.contains(r#""lumine_star":"max_hp_common""#), "{text}");
    assert_eq!(serde_json::from_str::<RunSetup>(&text).unwrap(), star);
}

fn equipment() -> RunSetup {
    RunSetup {
        build_mode: BuildMode::Equipment,
        ..RunSetup::default()
    }
}

fn found(setup: &RunSetup) -> Vec<Issue> {
    issues(run_setup::validate(setup).unwrap_err())
}

#[test]
fn equipment_mode_needs_sim_version_2() {
    let old = RunSetup {
        schema_version: 1,
        sim_version: 1,
        ..equipment()
    };
    assert_eq!(
        found(&old),
        [issue("$.build_mode", "equipment mode needs sim_version 2")]
    );
}

#[test]
fn equipment_mode_refuses_what_offers_mode_uses() {
    use stella_rain_core::run_setup::CarriedStar;
    let carried = issue(
        "$.carried",
        "none in equipment mode: the Lumine star is the one star a run starts with",
    );
    let rerolls = issue(
        "$.extra_rerolls",
        "none in equipment mode: there are no rerolls",
    );
    let star = CarriedStar {
        reward: "max_hp_common".into(),
    };
    let one = |s: RunSetup| found(&s);
    assert_eq!(
        one(RunSetup {
            carried: vec![star.clone()],
            ..equipment()
        }),
        vec![carried.clone()]
    );
    assert_eq!(
        one(RunSetup {
            extra_rerolls: 2,
            ..equipment()
        }),
        vec![rerolls.clone()]
    );
    assert_eq!(
        one(RunSetup {
            carried: vec![star],
            extra_rerolls: 2,
            ..equipment()
        }),
        [carried, rerolls]
    );
    // `stop` is the default and reads as absent; a plain equipment setup is valid.
    let plain = RunSetup {
        lumine: 10,
        ..equipment()
    };
    assert_eq!(run_setup::validate(&plain), Ok(()));
}

#[test]
fn a_lumine_star_needs_equipment_mode_and_a_well_formed_id() {
    let named = |s: &str| Some(s.into());
    assert_eq!(
        found(&RunSetup {
            lumine_star: named("max_hp_common"),
            ..RunSetup::default()
        }),
        [issue("$.lumine_star", "needs equipment mode")]
    );
    assert_eq!(
        found(&RunSetup {
            lumine_star: named("Bad Id"),
            ..equipment()
        }),
        [issue(
            "$.lumine_star",
            "not a valid ID (lowercase snake_case, at most 32 characters)"
        )]
    );
}

fn star_setup(lumine: u8, star: &str) -> RunSetup {
    RunSetup {
        lumine,
        lumine_star: Some(star.into()),
        ..equipment()
    }
}

/// `validate_in` on the bundled content: the content-dependent checks of a setup.
fn content_check(setup: &RunSetup) -> Result<(), Error> {
    run_setup::validate_in(setup, RunContent::bundled())
}

fn found_in(setup: &RunSetup) -> Vec<Issue> {
    issues(run_setup::validate_in(setup, RunContent::bundled()).unwrap_err())
}

#[test]
fn the_lumine_level_caps_the_stars_rarity() {
    let ok = content_check;
    assert_eq!(ok(&star_setup(1, "max_hp_common")), Ok(()));
    assert_eq!(
        found_in(&star_setup(0, "max_hp_common")),
        [issue(
            "$.lumine_star",
            "costs 1 Lumine, more than the level of 0"
        )]
    );
    for (rarity, cost) in [
        ("common", 1u8),
        ("uncommon", 2),
        ("rare", 3),
        ("epic", 4),
        ("legendary", 6),
        ("supernova", 10),
    ] {
        let id = format!("max_hp_{rarity}");
        assert_eq!(ok(&star_setup(cost, &id)), Ok(()), "{id} at {cost}");
        assert_eq!(
            found_in(&star_setup(cost - 1, &id)),
            [issue(
                "$.lumine_star",
                &format!("costs {cost} Lumine, more than the level of {}", cost - 1)
            )],
            "{id}"
        );
    }
}

#[test]
fn the_lumine_star_is_a_star_of_the_pool_and_never_a_rune() {
    assert_eq!(
        found_in(&star_setup(10, "no_such_reward")),
        [issue(
            "$.lumine_star",
            "no such reward in the run's rewards file"
        )]
    );
    let rune = RunContent::bundled()
        .rewards()
        .rewards()
        .iter()
        .find(|r| r.is_random_rune())
        .expect("a random rune reward")
        .id
        .clone();
    assert_eq!(
        found_in(&star_setup(10, rune.as_str())),
        [issue("$.lumine_star", "a rune is not a Lumine star")]
    );
    let pooled = |star: &str| RunSetup {
        star_pool: Some(vec!["max_hp_common".into()]),
        ..star_setup(10, star)
    };
    assert_eq!(
        found_in(&pooled("attack_common")),
        [issue("$.lumine_star", "not in the star pool")]
    );
    assert_eq!(content_check(&pooled("max_hp_common")), Ok(()));
    // A pool entry unknown to the rewards file is refused as before.
    let bad_pool = RunSetup {
        star_pool: Some(vec!["no_such_reward".into()]),
        ..equipment()
    };
    assert_eq!(
        found_in(&bad_pool),
        [issue(
            "$.star_pool[0]",
            "no such reward in the run's rewards file"
        )]
    );
}

#[test]
fn start_agrees_with_validate_in_on_the_lumine_star() {
    for setup in [
        star_setup(0, "max_hp_common"),
        star_setup(10, "no_such_reward"),
        RunSetup {
            lumine_star: Some("max_hp_common".into()),
            ..RunSetup::default()
        },
    ] {
        let expected = found_in(&setup);
        assert_eq!(issues(Engine::start(&setup).err().unwrap()), expected);
    }
}

#[test]
fn the_checked_in_schema_names_the_new_fields() {
    let text =
        std::fs::read_to_string(crate::data_root::data_root().join("schema/content.schema.json"))
            .unwrap();
    assert!(text.contains("\"build_mode\""));
    assert!(text.contains("\"lumine_star\""));
    assert!(text.contains("\"offers\""));
    assert!(text.contains("\"equipment\""));
}

#[test]
fn an_equipment_setup_starts_and_any_other_issue_is_reported() {
    let valid = equipment();
    assert_eq!(run_setup::validate(&valid), Ok(()));
    assert_eq!(content_check(&valid), Ok(()));
    assert!(Engine::start(&valid).is_ok());
    // Any other issue is reported alone.
    assert_eq!(
        found_in(&star_setup(0, "max_hp_common")),
        [issue(
            "$.lumine_star",
            "costs 1 Lumine, more than the level of 0"
        )]
    );
    // An offers setup is unaffected.
    assert!(Engine::start(&RunSetup::default()).is_ok());
}

/// A default setup's JSON with `offer_mode` added (the mode left core: the key is unknown).
fn setup_json_naming_offer_mode(mode: &str) -> serde_json::Value {
    let mut v = serde_json::to_value(RunSetup::default()).unwrap();
    assert!(
        v.get("offer_mode").is_none(),
        "a default setup names no mode"
    );
    v["offer_mode"] = mode.into();
    v
}

#[test]
fn a_setup_naming_offer_mode_is_refused_as_an_unknown_field() {
    for mode in ["stop", "timed"] {
        let v = setup_json_naming_offer_mode(mode);
        assert!(
            serde_json::from_value::<RunSetup>(v.clone()).is_err(),
            "{mode}"
        );
        let err = run_setup::parse_and_validate(v.to_string().as_bytes()).unwrap_err();
        assert!(matches!(err, Error::Parse { .. }), "{mode}: {err:?}");
    }
}

#[test]
fn a_replay_whose_setup_names_offer_mode_is_refused() {
    use stella_rain_core::run_replay::{RunRecorder, RunReplay};
    let bytes = RunRecorder::start(&RunSetup::default())
        .unwrap()
        .replay()
        .to_bytes();
    assert!(RunReplay::from_bytes(&bytes).is_ok());
    let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["setup"]["offer_mode"] = "stop".into();
    assert!(RunReplay::from_bytes(v.to_string().as_bytes()).is_err());
}

#[test]
fn the_checked_in_schema_has_no_offer_mode() {
    let text =
        std::fs::read_to_string(crate::data_root::data_root().join("schema/content.schema.json"))
            .unwrap();
    assert!(!text.contains("\"offer_mode\""));
    assert!(!text.contains("\"OfferMode\""));
}

/// A default setup's JSON with `pick_stats` added (the stat offer left core: the key is unknown).
fn setup_json_naming_pick_stats(on: bool) -> serde_json::Value {
    let mut v = serde_json::to_value(RunSetup::default()).unwrap();
    assert!(
        v.get("pick_stats").is_none(),
        "a default setup names no stat offer"
    );
    v["pick_stats"] = on.into();
    v
}

#[test]
fn a_setup_naming_pick_stats_is_refused_as_an_unknown_field() {
    for on in [false, true] {
        let v = setup_json_naming_pick_stats(on);
        assert!(
            serde_json::from_value::<RunSetup>(v.clone()).is_err(),
            "{on}"
        );
        let err = run_setup::parse_and_validate(v.to_string().as_bytes()).unwrap_err();
        assert!(matches!(err, Error::Parse { .. }), "{on}: {err:?}");
    }
}

#[test]
fn a_replay_whose_setup_names_pick_stats_is_refused() {
    use stella_rain_core::run_replay::{RunRecorder, RunReplay};
    let bytes = RunRecorder::start(&RunSetup::default())
        .unwrap()
        .replay()
        .to_bytes();
    assert!(RunReplay::from_bytes(&bytes).is_ok());
    let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["setup"]["pick_stats"] = true.into();
    assert!(RunReplay::from_bytes(v.to_string().as_bytes()).is_err());
}

#[test]
fn the_checked_in_schema_names_no_level_offer() {
    let text =
        std::fs::read_to_string(crate::data_root::data_root().join("schema/content.schema.json"))
            .unwrap();
    for name in [
        "pick_stats",
        "stat_offer",
        "StatOfferView",
        "StatOfferOpened",
        "ExpertiseOfferView",
        "ExpertiseCandidateView",
        "ExpertiseOfferOpened",
        "ExpertiseOptionTaken",
        "ExpertiseCandidateReplaced",
    ] {
        assert!(!text.contains(&format!("\"{name}\"")), "{name}");
    }
}

// --- `build_mode` left core: a setup naming it is refused, `sim_version` is the mode --------------

/// The trial fixture's text with `extra` (a JSON member) added before the closing brace.
fn trial_text_with(extra: &str) -> String {
    let path = crate::data_root::data_root().join("tests/fixtures/run_setup/trial.json");
    let text = std::fs::read_to_string(path).unwrap();
    let body = text.trim_end().strip_suffix('}').expect("an object");
    format!("{body}{extra}}}")
}

#[test]
fn a_setup_naming_build_mode_is_refused() {
    let unknown =
        run_setup::parse_and_validate(trial_text_with(r#","unknown":1"#).as_bytes()).unwrap_err();
    assert!(matches!(unknown, Error::Parse { .. }), "{unknown:?}");
    for mode in ["equipment", "offers", "cards"] {
        let text = trial_text_with(&format!(r#","build_mode":"{mode}""#));
        let err = run_setup::parse_and_validate(text.as_bytes())
            .expect_err(&format!("`build_mode` {mode} is accepted"));
        assert_eq!(
            std::mem::discriminant(&err),
            std::mem::discriminant(&unknown),
            "{mode}: {err:?}"
        );
    }
}

#[test]
fn a_saved_setup_naming_build_mode_is_refused() {
    for mode in ["equipment", "offers", "cards"] {
        let text = trial_text_with(&format!(r#","build_mode":"{mode}""#));
        assert!(serde_json::from_str::<RunSetup>(&text).is_err(), "{mode}");
    }
    for name in ["trial", "trial_loadout", "trial_upgrades"] {
        let path =
            crate::data_root::data_root().join(format!("tests/fixtures/run_setup/{name}.json"));
        let parsed = run_setup::parse_and_validate(&std::fs::read(&path).unwrap()).unwrap();
        let written = serde_json::to_value(&parsed).unwrap();
        assert!(written.get("build_mode").is_none(), "{name}");
        let again = serde_json::from_value::<RunSetup>(written).unwrap();
        assert_eq!(again, parsed, "{name}");
    }
}

#[test]
fn a_version_2_setup_refuses_carried_stars_and_rerolls_a_version_1_setup_keeps_them() {
    use stella_rain_core::run_setup::CarriedStar;
    let star = CarriedStar {
        reward: "max_hp_common".into(),
    };
    let live = RunSetup {
        schema_version: 2,
        sim_version: 2,
        ..RunSetup::default()
    };
    assert_eq!(
        found(&RunSetup {
            carried: vec![star.clone()],
            ..live.clone()
        }),
        [issue(
            "$.carried",
            "none in equipment mode: the Lumine star is the one star a run starts with"
        )]
    );
    assert_eq!(
        found(&RunSetup {
            extra_rerolls: 1,
            ..live
        }),
        [issue(
            "$.extra_rerolls",
            "none in equipment mode: there are no rerolls"
        )]
    );
    let frozen = RunSetup {
        schema_version: 1,
        sim_version: 1,
        carried: vec![star],
        extra_rerolls: 1,
        ..RunSetup::default()
    };
    assert_eq!(run_setup::validate(&frozen), Ok(()));
}

#[test]
fn a_lumine_star_needs_sim_version_2() {
    assert_eq!(
        found(&RunSetup {
            schema_version: 1,
            sim_version: 1,
            lumine_star: Some("max_hp_common".into()),
            ..RunSetup::default()
        }),
        [issue("$.lumine_star", "needs sim_version 2")]
    );
}

#[test]
fn a_version_1_pool_may_not_name_an_equipment_only_reward() {
    let bad = RunSetup {
        schema_version: 1,
        sim_version: 1,
        star_pool: Some(vec!["rune_haste_star_rare".into()]),
        ..RunSetup::default()
    };
    let issues = issues(run_setup::validate_in(&bad, RunContent::bundled()).unwrap_err());
    assert_eq!(
        issues,
        [issue(
            "$.star_pool[0]",
            "an equipment-only reward is not offered at sim_version 1"
        )]
    );
}
