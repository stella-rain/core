//! The integration tests, built as one binary (`autotests = false` in Cargo.toml): every other
//! file in `tests/` is a module here, so the engine is compiled and linked once for all of them
//! instead of once per file. Run one file's tests with a filter on its module:
//! `cargo test --test integration corpus::`.

mod common;

#[global_allocator]
static ALLOCATOR: alloc::Counting = alloc::Counting;

mod agents;
mod alloc;
mod assist;
mod attack_presets;
mod attacks;
mod auto;
mod base64;
mod bench_fixture_pins;
mod bench_region;
mod bench_teardown;
mod big_patterns;
mod bleed;
mod boomerang;
mod boss;
mod boss_chase;
mod boss_chase_content;
mod boss_zones;
mod bosses;
mod charge;
mod charge_pulse_cage_content;
mod combat_reward_kinds;
mod command_cap;
mod constellations;
mod content_hash;
mod content_pack;
mod content_pack_compare;
mod content_pack_unpack;
mod controls;
mod corpus;
mod corpus_contract;
mod corpus_keeps;
mod corpus_pins;
mod corpus_seeds;
mod counter_window;
mod crowd;
mod cwd_free_paths;
mod damage_types;
mod dash;
mod data_root;
mod defence;
mod died_class;
mod difficulties;
mod difficulty_run;
mod drop_weights;
mod economy_pack;
mod elite_goblin;
mod elite_mimic;
mod elite_pack_leader;
mod elite_shrine;
mod elite_traits;
mod elites;
mod enemy_damage;
mod enemy_role_content;
mod enemy_roles;
mod engine;
mod equipment_drops;
mod equipment_helper;
mod equipment_input;
mod equipment_pack;
mod equipment_party;
mod equipment_run;
mod equipment_slots;
mod evade;
mod expertise;
mod expertise_counter;
mod expertise_gauge;
mod expertise_spirit;
mod expertise_tempo;
mod explosion;
mod extremes;
mod frozen_v1;
mod fuzz_regression;
mod gameplay_observables;
mod golden;
mod hash;
mod healing;
mod hit_boxes;
mod hit_case;
mod hit_marks;
mod hit_type_kind;
mod hold;
mod homing_missiles;
mod knockdown;
mod leash;
mod ledger;
mod level_table;
mod levels;
mod map;
mod match_ups;
mod named_stars;
mod numbers_298;
mod numbers_report;
mod one_source;
mod orb_cage;
mod orb_content;
mod orbit;
mod orbs;
mod overflow;
mod pack;
mod pack_pins;
mod packs;
mod parsed_stage_hash;
mod party_bot;
mod party_damage;
mod party_down;
mod party_join;
mod party_run;
mod party_tether;
mod perk_sets;
mod phases;
mod pick_input;
mod pick_pins;
mod places;
mod player_stats;
mod players_api;
mod poise;
mod poison_burn;
mod progression;
mod properties;
mod push_apart;
mod range_reward;
mod ranged_cap;
mod ranged_report;
mod ranged_rules;
mod ranged_tokens;
mod rarities;
mod rarity_rows_d6c;
mod recording;
mod registry;
mod repeat;
mod replay;
mod reward_kinds;
mod rewards;
mod rng;
mod rules;
mod run_loadout;
mod run_play;
mod run_replay;
mod run_setup;
mod run_stats;
mod run_steps;
mod rune_haste;
mod runes;
mod runes_file;
mod savepoint;
mod savepoint_proof;
mod schema;
mod schema2_corpus;
mod schema2_range;
mod schema2_write;
mod share;
mod shot;
mod smash_styles;
mod stage_skill;
mod stage_skill_file;
mod stage_verify;
mod stagger;
mod star_chart;
mod stardust;
mod stardust_earners;
mod stardust_file;
mod stardust_tiers;
mod stars;
mod starting_unlocks;
mod statuses;
mod step_three;
mod stun;
mod style_grants;
mod swap_rating;
mod sync_rig;
mod tally;
mod text_table;
mod threat_colours;
mod thunder;
mod time_limit;
mod time_limit_end;
mod time_limit_run;
mod trig;
mod try_styles;
mod tuning;
mod tutorial;
mod tutorial_script;
mod type_bonuses;
mod types;
mod unlocked_styles;
mod validate;
mod version_flag;
mod version_two;
mod view_edge;
mod warn_anchor;
mod weapon_tree;
mod weapons;
mod weapons_content;
mod worst_case;
mod worst_minute;
mod x100;
mod zone_bot;
mod zones;

/// With `autotests = false`, a new file in `tests/` builds and runs only once it is a module
/// here; a missing `mod` line would leave its tests out silently.
#[test]
fn every_test_file_is_a_module_here() {
    let dir = crate::data_root::data_root().join("tests");
    let declared: Vec<&str> = include_str!("integration.rs")
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .collect();
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("tests/ is readable") {
        let path = entry.expect("a tests/ entry").path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().is_some_and(|e| e == "rs")
            && stem != "integration"
            && !declared.contains(&stem)
        {
            missing.push(stem.to_owned());
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "tests/ files without `mod` here: {missing:?}"
    );
}
mod time_limit_case;
