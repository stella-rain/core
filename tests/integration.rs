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
mod attacks;
mod auto;
mod base64;
mod big_patterns;
mod bleed;
mod boomerang;
mod boss;
mod bosses;
mod charge;
mod collide_case;
mod combat_reward_kinds;
mod command_cap;
mod constellations;
mod content_hash;
mod content_pack;
mod content_pack_unpack;
mod controls;
mod corpus;
mod damage_types;
mod defence;
mod died_class;
mod difficulties;
mod difficulty_run;
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
mod evade;
mod expertise;
mod expertise_gauge;
mod expertise_offers;
mod explosion;
mod extremes;
mod golden;
mod hash;
mod healing;
mod hit_boxes;
mod hit_marks;
mod hit_type_kind;
mod hold;
mod homing_missiles;
mod level_table;
mod levels;
mod map;
mod match_ups;
mod named_stars;
mod numbers_298;
mod orbit;
mod overflow;
mod pack;
mod parsed_stage_hash;
mod perk_sets;
mod phases;
mod poison_burn;
mod properties;
mod push_apart;
mod range_reward;
mod ranged_cap;
mod rarities;
mod rarity_rows_d6c;
mod recording;
mod registry;
mod replay;
mod reward_kinds;
mod rewards;
mod rng;
mod run_loadout;
mod run_play;
mod run_replay;
mod run_setup;
mod run_stats;
mod rune_haste;
mod runes;
mod runes_file;
mod schema;
mod share;
mod shot;
mod spike;
mod stage_skill;
mod stage_skill_file;
mod stage_verify;
mod stardust;
mod stardust_earners;
mod stardust_file;
mod stardust_tiers;
mod stars;
mod starting_unlocks;
mod statuses;
mod stun;
mod thunder;
mod time_limit;
mod time_limit_end;
mod time_limit_run;
mod trig;
mod tutorial;
mod type_bonuses;
mod types;
mod validate;
mod weapon_tree;
mod weapons;
mod weapons_content;
mod worst_case;
mod x100;
mod zones;

/// With `autotests = false`, a new file in `tests/` builds and runs only once it is a module
/// here; a missing `mod` line would leave its tests out silently.
#[test]
fn every_test_file_is_a_module_here() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
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
