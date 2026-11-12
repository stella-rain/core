//! Tuning limits catalogue and single source of truth for runtime checks (ADR-SIM-5, ADR-CONT-3).
//!
//! Values here define the simulation's authoritative runtime limits, file budgets, and tuning
//! caps. `content-pack catalogue` generates `docs/tuning.md` from the catalogue table below.
//! It lists caps and code numbers only; the numbers a designer tunes are `content/rules.json`
//! keys (ADR-CONT-3).

/// An entry in the tuning limits catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitEntry {
    pub name: &'static str,
    pub value: &'static str,
    pub unit: &'static str,
    pub adr: &'static str,
}

impl LimitEntry {
    pub const fn new(
        name: &'static str,
        value: &'static str,
        unit: &'static str,
        adr: &'static str,
    ) -> Self {
        Self {
            name,
            value,
            unit,
            adr,
        }
    }
}

// ---------------------------------------------------------------------------
// Authoritative constants (DS138 Q1 = A)
// ---------------------------------------------------------------------------

// Dynamic budgets of a tick (ADR-SIM-5)
pub const MAX_ENEMY_CAP: u16 = 500;
pub const MIN_CAP_HEADROOM_PCT: u8 = 25;
pub const MAX_CAP_HEADROOM_PCT: u8 = 30;
pub const MAX_CROWD_TARGET: u16 = 400;
pub const MAX_RULES_BYTES: usize = 4 * 1024;
pub const MAX_ZONES_ALIVE: u32 = 128;
pub const MAX_RANGED_ATTACKERS: u32 = 4;
/// Half the widest supported view, 21:9 of the 360 px design height: `360 * 21 / 9 / 2`.
pub const VIEW_HALF_WIDTH: i32 = 420;
pub const VIEW_HALF_HEIGHT: i32 = 180;
pub const MAX_ORBS_PER_OWNER: u32 = 8;
pub const MAX_ORBS_ALIVE: u32 = 32;
pub const MAX_ORB_TICKS: u32 = 600;
pub const MAX_ATTACK_TASKS: u32 = 256;
pub const MAX_EMITTER_STEPS_PER_TICK: u32 = 4096;
pub const MAX_ELITES_ALIVE: u16 = 32;

// The push-apart (ADR-SIM-5)
pub const PUSH_STEP_PX: i64 = 4;
pub const PUSH_LEAST_RADIUS_PX: i32 = 8;
pub const PUSH_CANDIDATES: usize = 32;
pub const PUSH_NEIGHBOURS: usize = 6;
pub const PUSH_CROWDED_OVER_PX: i64 = 1;

// The leash: the cap stays code, its numbers are content (ADR-CONT-3)
pub const MAX_LEASH_MOVES_PER_TICK: u32 = 16;

// Reward effects (ADR-SIM-5, ADR-SVC-1, ADR-REPL-5)
pub const MAX_ECHOES_PER_TICK: usize = 64;
pub const MAX_BURSTS_PER_TICK: usize = 8;
pub const MAX_REWARD_EFFECTS: usize = 4;
pub const MAX_BURST_DEPTH: u32 = 5;
pub const MAX_OFFER_CANDIDATES: usize = 4;
pub const MAX_REWARDS: usize = 192;
pub const MAX_STACK: u8 = 20;

// Drop tiers and difficulties
pub const MAX_TIERS: usize = 16;
pub const MAX_TIER_LEVEL: u32 = 100;
pub const MAX_RARITY_WEIGHT: u16 = 10_000;
pub const MAX_DIFFICULTIES: usize = 8;
pub const MAX_PCT: u32 = 1_000;

// Run replay and setup (ADR-REPL-5, ADR-SVC-1)
pub const MAX_COMMANDS: u32 = 1_000;
pub const MAX_PRESENCE_CHANGES: u32 = 64;
pub const MAX_RUN_REPLAY_BYTES: usize = 1_048_576;
pub const MAX_SETUP_BYTES: usize = 4_096;
pub const MAX_EXTRA_REROLLS: u8 = 20;
pub const MAX_CARRIED_STARS: usize = 5;
pub const MAX_LUMINE: u8 = 10;
/// Held stars and runes per player: a bound no run reaches (ADR-SIM-5 point 8).
pub const MAX_INVENTORY: usize = 1024;
/// Equipment slots per player.
pub const MAX_EQUIP_SLOTS: usize = 32;
/// The level cap of an equipment slot.
pub const MAX_SLOT_LEVEL: u8 = 10;
/// The most slot categories the rewards file's equipment rules list.
pub const MAX_EQUIP_CATEGORIES: usize = 8;
/// The most slots of one category, and the most rune slots.
pub const MAX_CATEGORY_SLOTS: u8 = 16;
/// The most stats in one weapon base set or style set.
pub const MAX_SET_STATS: usize = 8;
pub const MAX_STARS_ON_MAP: u16 = 64;

// The combat state's caps (ADR-SIM-5 point 9): they bound the combat fields of the live state
// at decode and are tuning values; a cap fixes no behaviour. A tick cap stays below 65,536 (a
// `u16`); a raise past a field's type is another hash move.
pub const MAX_FLINCH_TICKS: u16 = 60;
pub const MAX_FLINCH_IMMUNE_TICKS: u16 = 120;
pub const MAX_CONTROL_REPEATS: u8 = 8;
pub const MAX_CONTROL_RESET_TICKS: u16 = 600;
pub const MAX_DOWN_TICKS: u16 = 180;
pub const MAX_STAGGER: u32 = 10_000;
pub const MAX_STAGGER_BREAKS: u16 = 1_000;
/// The most entries of the rules' `stagger_refill_pcts`: how many bars fill at their own rate.
pub const MAX_STAGGER_REFILLS: u8 = 8;
pub const MAX_STAGGER_DRAIN_TICKS: u16 = 600;
pub const MAX_STAGGERED_TICKS: u16 = 600;
pub const MAX_PERFECT_WINDOW_TICKS: u16 = 30;
pub const MAX_PERFECT_SLOW_TICKS: u16 = 300;
pub const MAX_PERFECT_COOLDOWN_TICKS: u16 = 3_600;
/// The farthest a perfect evade's reach may be, in px: the view's half width.
pub const MAX_PERFECT_REACH: i32 = VIEW_HALF_WIDTH;
/// The most a slowed entity's `perfect_evade_slow_every` may be: it acts one tick in this many.
pub const MAX_PERFECT_SLOW_EVERY: u8 = 8;
/// The longest slide a combo phase's `advance` may name, in px.
pub const MAX_ADVANCE_DISTANCE: i32 = 240;
/// The most the slide moves the player in one tick, in px (the player walks 3 px a tick).
pub const MAX_ADVANCE_PER_TICK: i32 = 12;
pub const MAX_WEAPON_GAUGE: u32 = 10_000;
pub const MAX_RALLY: u32 = crate::validate::limits::MAX_PLAYER_HP;
pub const MAX_RALLY_TICKS: u16 = 600;
pub const MAX_SPIRIT_LEVEL: u8 = 3;
pub const MAX_SPIRIT_DECAY_TICKS: u16 = 3_600;
pub const MAX_COUNTER_WINDOW_TICKS: u16 = 60;
pub const MAX_TEMPO_STACKS: u8 = 10;
pub const MAX_TRIGGER_TICKS: u16 = 3_600;
pub const MAX_RANGED_GRACE_TICKS: u16 = 600;
pub const MAX_REPEATS_QUEUED: usize = 64;
pub const MAX_REPEAT_TARGETS: usize = 16;

// Time limit (ADR-SVC-1)
pub const MAX_WAVES: usize = 16;
pub const MAX_TIME_LIMIT_BYTES: usize = 128 * 1024;

// ---------------------------------------------------------------------------
// The limits catalogue table
// ---------------------------------------------------------------------------

pub const LIMITS: &[LimitEntry] = &[
    LimitEntry::new("MAX_ENEMY_CAP", "500", "enemies alive at once", "ADR-SIM-5"),
    LimitEntry::new("MIN_CAP_HEADROOM_PCT", "25", "percent", "ADR-SIM-5"),
    LimitEntry::new("MAX_CAP_HEADROOM_PCT", "30", "percent", "ADR-SIM-5"),
    LimitEntry::new("MAX_CROWD_TARGET", "400", "enemies", "ADR-SIM-5"),
    LimitEntry::new("MAX_RULES_BYTES", "4096", "bytes", "ADR-SVC-1"),
    LimitEntry::new("MAX_ZONES_ALIVE", "128", "zones alive at once", "ADR-SIM-5"),
    LimitEntry::new(
        "MAX_RANGED_ATTACKERS",
        "4",
        "regular ranged attackers",
        "ADR-SIM-5",
    ),
    LimitEntry::new("VIEW_HALF_WIDTH", "420", "px", "ADR-SIM-5"),
    LimitEntry::new("VIEW_HALF_HEIGHT", "180", "px", "ADR-SIM-5"),
    LimitEntry::new(
        "MAX_ORBS_PER_OWNER",
        "8",
        "slow orbs per attacker",
        "ADR-SIM-5",
    ),
    LimitEntry::new(
        "MAX_ORBS_ALIVE",
        "32",
        "slow orbs alive at once",
        "ADR-SIM-5",
    ),
    LimitEntry::new("MAX_ORB_TICKS", "600", "ticks", "ADR-SIM-5"),
    LimitEntry::new(
        "MAX_ATTACK_TASKS",
        "256",
        "attacks running at once",
        "ADR-SIM-5",
    ),
    LimitEntry::new(
        "MAX_EMITTER_STEPS_PER_TICK",
        "4096",
        "steps per tick",
        "ADR-SIM-5",
    ),
    LimitEntry::new(
        "MAX_ELITES_ALIVE",
        "32",
        "elites among enemies",
        "ADR-SIM-5",
    ),
    LimitEntry::new("STEP", "4", "px/tick", "ADR-SIM-5"),
    LimitEntry::new("LEAST_RADIUS", "8", "px", "ADR-SIM-5"),
    LimitEntry::new("CANDIDATES", "32", "enemies", "ADR-SIM-5"),
    LimitEntry::new("NEIGHBOURS", "6", "enemies", "ADR-SIM-5"),
    LimitEntry::new("CROWDED_OVER", "1", "px", "ADR-SIM-5"),
    LimitEntry::new(
        "MAX_LEASH_MOVES_PER_TICK",
        "16",
        "moves per tick",
        "ADR-SIM-5",
    ),
    LimitEntry::new("MAX_ECHOES_PER_TICK", "64", "echoes per tick", "ADR-SIM-5"),
    LimitEntry::new("MAX_BURSTS_PER_TICK", "8", "bursts per tick", "ADR-SIM-5"),
    LimitEntry::new("MAX_REWARD_EFFECTS", "4", "effects per reward", "ADR-SVC-1"),
    LimitEntry::new("MAX_BURST_DEPTH", "5", "burst depth", "ADR-SVC-1"),
    LimitEntry::new("MAX_OFFER_CANDIDATES", "4", "candidates", "ADR-REPL-5"),
    LimitEntry::new("MAX_OFFER_TIMER_TICKS", "3600", "ticks", "ADR-SIM-7"),
    LimitEntry::new("MAX_PARTY_DAMAGE_PCT", "100", "percent", "ADR-SIM-8"),
    LimitEntry::new("MAX_REWARDS", "192", "rewards", "ADR-SVC-1"),
    LimitEntry::new("MAX_STACK", "20", "stacks", "ADR-SVC-1"),
    LimitEntry::new("MAX_TIERS", "16", "tiers", "ADR-CONT-3"),
    LimitEntry::new("MAX_TIER_LEVEL", "100", "level", "ADR-CONT-3"),
    LimitEntry::new("MAX_RARITY_WEIGHT", "10000", "weight", "ADR-CONT-3"),
    LimitEntry::new("MAX_DIFFICULTIES", "8", "difficulties", "ADR-CONT-1"),
    LimitEntry::new("MAX_PCT", "1000", "percent", "ADR-CONT-1"),
    LimitEntry::new("MAX_COMMANDS", "1000", "commands", "ADR-REPL-5"),
    LimitEntry::new("MAX_PRESENCE_CHANGES", "64", "changes", "ADR-REPL-5"),
    LimitEntry::new("MAX_RUN_REPLAY_BYTES", "1048576", "bytes", "ADR-SVC-1"),
    LimitEntry::new("MAX_SETUP_BYTES", "4096", "bytes", "ADR-SVC-1"),
    LimitEntry::new("MAX_EXTRA_REROLLS", "20", "rerolls", "ADR-REPL-5"),
    LimitEntry::new("MAX_CARRIED_STARS", "5", "stars", "ADR-REPL-5"),
    LimitEntry::new("MAX_LUMINE", "10", "level", "ADR-REPL-5"),
    LimitEntry::new("MAX_INVENTORY", "1024", "held stars and runes", "ADR-SIM-5"),
    LimitEntry::new("MAX_EQUIP_SLOTS", "32", "slots per player", "ADR-SIM-5"),
    LimitEntry::new("MAX_SLOT_LEVEL", "10", "level", "ADR-SIM-5"),
    LimitEntry::new("MAX_EQUIP_CATEGORIES", "8", "categories", "ADR-CONT-3"),
    LimitEntry::new("MAX_CATEGORY_SLOTS", "16", "slots", "ADR-CONT-3"),
    LimitEntry::new("MAX_SET_STATS", "8", "stats", "ADR-CONT-3"),
    LimitEntry::new("MAX_STARS_ON_MAP", "64", "stars", "ADR-REPL-5"),
    LimitEntry::new("MAX_WAVES", "16", "waves", "ADR-SVC-1"),
    LimitEntry::new("MAX_FLINCH_TICKS", "60", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_FLINCH_IMMUNE_TICKS", "120", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_CONTROL_REPEATS", "8", "hard controls", "ADR-SIM-5"),
    LimitEntry::new("MAX_CONTROL_RESET_TICKS", "600", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_DOWN_TICKS", "180", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_STAGGER", "10000", "stagger", "ADR-SIM-5"),
    LimitEntry::new("MAX_STAGGER_BREAKS", "1000", "breaks", "ADR-SIM-5"),
    LimitEntry::new("MAX_STAGGER_REFILLS", "8", "bars", "ADR-SIM-5"),
    LimitEntry::new("MAX_STAGGER_DRAIN_TICKS", "600", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_STAGGERED_TICKS", "600", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_PERFECT_WINDOW_TICKS", "30", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_PERFECT_SLOW_TICKS", "300", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_PERFECT_COOLDOWN_TICKS", "3600", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_PERFECT_REACH", "420", "px", "ADR-SIM-5"),
    LimitEntry::new("MAX_PERFECT_SLOW_EVERY", "8", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_ADVANCE_DISTANCE", "240", "px", "ADR-SIM-5"),
    LimitEntry::new("MAX_ADVANCE_PER_TICK", "12", "px", "ADR-SIM-5"),
    LimitEntry::new("MAX_WEAPON_GAUGE", "10000", "gauge", "ADR-SIM-5"),
    LimitEntry::new("MAX_RALLY", "99900", "hit points", "ADR-SIM-5"),
    LimitEntry::new("MAX_RALLY_TICKS", "600", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_SPIRIT_LEVEL", "3", "level", "ADR-SIM-5"),
    LimitEntry::new("MAX_SPIRIT_DECAY_TICKS", "3600", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_COUNTER_WINDOW_TICKS", "60", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_TEMPO_STACKS", "10", "stacks", "ADR-SIM-5"),
    LimitEntry::new("MAX_TRIGGER_TICKS", "3600", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_RANGED_GRACE_TICKS", "600", "ticks", "ADR-SIM-5"),
    LimitEntry::new("MAX_REPEATS_QUEUED", "64", "entries", "ADR-SIM-5"),
    LimitEntry::new("MAX_REPEAT_TARGETS", "16", "targets", "ADR-SIM-5"),
    LimitEntry::new("MAX_TIME_LIMIT_BYTES", "131072", "bytes", "ADR-SVC-1"),
];

/// Formats the limits catalogue as a GitHub-flavored Markdown table for `docs/tuning.md`.
pub fn tuning_catalogue_markdown() -> String {
    let mut s = String::from(
        "# Tuning limits catalogue\n\n\
         <!-- Generated by `cargo run --bin content-pack catalogue > docs/tuning.md`. Do not edit by hand. -->\n\n\
         | Name | Value | Unit | ADR |\n\
         |---|---|---|---|\n",
    );
    for entry in LIMITS {
        s.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            entry.name, entry.value, entry.unit, entry.adr
        ));
    }
    s.push_str(
        "\nThe numbers a designer tunes (the leash, the spawn margin, the ranged tokens and the others) \
         are content, not listed here: see content/rules.json (ADR-CONT-3).\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code, reason = "shared with tests/, which uses more of it")]
    mod data_root {
        include!("../tests/data_root.rs");
    }

    #[test]
    fn the_presence_cap_has_a_row() {
        let row = LIMITS
            .iter()
            .find(|e| e.name == "MAX_PRESENCE_CHANGES")
            .expect("a row for MAX_PRESENCE_CHANGES");
        assert_eq!(row.value, crate::step_log::MAX_PRESENCE_CHANGES.to_string());
        assert_eq!(row.unit, "changes");
        assert_eq!(row.adr, "ADR-REPL-5");
    }

    #[test]
    fn the_presence_changes_fit_the_replay_budget_with_room() {
        // A JOIN or LEAVE record is 2 bytes: at most 128 bytes of the 1 MiB budget, and no
        // PICK (`MEASURED_MOST` counts those only).
        assert!((2 * MAX_PRESENCE_CHANGES as usize) < MAX_RUN_REPLAY_BYTES / 1000);
    }

    #[test]
    fn no_row_cites_a_bare_number() {
        for e in LIMITS {
            let id = e
                .adr
                .strip_prefix("ADR-")
                .unwrap_or_else(|| panic!("{}: {}", e.name, e.adr));
            assert!(
                !id.chars().all(|c| c.is_ascii_digit()),
                "{} cites {}, a number with no area",
                e.name,
                e.adr
            );
        }
        let leash = LIMITS
            .iter()
            .find(|e| e.name == "MAX_LEASH_MOVES_PER_TICK")
            .expect("leash row");
        assert_eq!(leash.adr, "ADR-SIM-5");
    }

    #[test]
    fn limits_match_runtime_constants() {
        assert_eq!(
            i64::from(VIEW_HALF_WIDTH) * 256,
            i64::from(crate::sim::VIEW_HALF_WIDTH.0)
        );
        assert_eq!(
            i64::from(VIEW_HALF_HEIGHT) * 256,
            i64::from(crate::sim::VIEW_HALF_HEIGHT.0)
        );
        assert_eq!(
            MAX_LEASH_MOVES_PER_TICK,
            crate::sim::MAX_LEASH_MOVES_PER_TICK
        );
        assert_eq!(MAX_ENEMY_CAP, crate::validate::limits::MAX_ENEMY_CAP);
        assert_eq!(MAX_CROWD_TARGET, crate::validate::limits::MAX_CROWD_TARGET);
        assert_eq!(MIN_CAP_HEADROOM_PCT, crate::rules::MIN_CAP_HEADROOM_PCT);
        assert_eq!(MAX_CAP_HEADROOM_PCT, crate::rules::MAX_CAP_HEADROOM_PCT);
        assert_eq!(MAX_RULES_BYTES, crate::rules::MAX_RULES_BYTES);
        assert_eq!(MAX_ORB_TICKS, crate::validate::limits::MAX_ORB_TICKS);
        assert_eq!(MAX_ELITES_ALIVE, crate::validate::limits::MAX_ELITES_ALIVE);
        assert_eq!(MAX_ECHOES_PER_TICK, crate::rewards::MAX_ECHOES_PER_TICK);
        assert_eq!(MAX_BURSTS_PER_TICK, crate::rewards::MAX_BURSTS_PER_TICK);
        assert_eq!(MAX_REWARD_EFFECTS, crate::rewards::MAX_REWARD_EFFECTS);
        assert_eq!(MAX_BURST_DEPTH, crate::rewards::MAX_BURST_DEPTH);
        assert_eq!(MAX_OFFER_CANDIDATES, crate::rewards::MAX_OFFER_CANDIDATES);
        assert_eq!(3600, crate::rules::MAX_OFFER_TIMER_TICKS);
        assert_eq!(100, crate::rules::MAX_PARTY_DAMAGE_PCT);
        assert_eq!(MAX_REWARDS, crate::rewards::MAX_REWARDS);
        assert_eq!(MAX_STACK, crate::rewards::MAX_STACK);
        assert_eq!(MAX_TIERS, crate::validate::limits::MAX_TIERS);
        assert_eq!(MAX_TIER_LEVEL, crate::validate::limits::MAX_TIER_LEVEL);
        assert_eq!(
            MAX_RARITY_WEIGHT,
            crate::validate::limits::MAX_RARITY_WEIGHT
        );
        assert_eq!(MAX_DIFFICULTIES, crate::difficulties::MAX_DIFFICULTIES);
        assert_eq!(MAX_PCT, crate::difficulties::MAX_PCT);
        assert_eq!(MAX_COMMANDS, crate::run_replay::MAX_COMMANDS);
        assert_eq!(MAX_PRESENCE_CHANGES, crate::step_log::MAX_PRESENCE_CHANGES);
        assert_eq!(
            MAX_RUN_REPLAY_BYTES,
            crate::run_replay::MAX_RUN_REPLAY_BYTES
        );
        assert_eq!(MAX_SETUP_BYTES, crate::run_setup::MAX_SETUP_BYTES);
        assert_eq!(MAX_EXTRA_REROLLS, crate::run_setup::MAX_EXTRA_REROLLS);
        assert_eq!(MAX_CARRIED_STARS, crate::run_setup::MAX_CARRIED_STARS);
        assert_eq!(MAX_LUMINE, crate::run_setup::MAX_LUMINE);
        assert_eq!(MAX_INVENTORY, 1024);
        assert_eq!(MAX_EQUIP_SLOTS, 32);
        assert_eq!(MAX_SLOT_LEVEL, 10);
        assert_eq!(MAX_EQUIP_CATEGORIES, 8);
        assert_eq!(MAX_CATEGORY_SLOTS, 16);
        assert_eq!(MAX_SET_STATS, 8);
        assert_eq!(MAX_STARS_ON_MAP, crate::validate::limits::MAX_STARS_ON_MAP);
        assert_eq!(MAX_WAVES, crate::time_limit::MAX_WAVES);
        assert_eq!(
            MAX_TIME_LIMIT_BYTES,
            crate::time_limit::MAX_TIME_LIMIT_BYTES
        );
    }

    /// The combat state's caps (ADR-SIM-5 point 9) and the evade and advance caps: 27 rows, `MAX_WEAPON_GAUGE` bounding two
    /// fields. Tuning values: a cap bounds a decode and fixes no behaviour.
    #[test]
    fn the_combat_caps_have_rows() {
        let rows = [
            ("MAX_FLINCH_TICKS", "60", "ticks"),
            ("MAX_FLINCH_IMMUNE_TICKS", "120", "ticks"),
            ("MAX_CONTROL_REPEATS", "8", "hard controls"),
            ("MAX_CONTROL_RESET_TICKS", "600", "ticks"),
            ("MAX_DOWN_TICKS", "180", "ticks"),
            ("MAX_STAGGER", "10000", "stagger"),
            ("MAX_STAGGER_BREAKS", "1000", "breaks"),
            ("MAX_STAGGER_REFILLS", "8", "bars"),
            ("MAX_STAGGER_DRAIN_TICKS", "600", "ticks"),
            ("MAX_STAGGERED_TICKS", "600", "ticks"),
            ("MAX_PERFECT_WINDOW_TICKS", "30", "ticks"),
            ("MAX_PERFECT_SLOW_TICKS", "300", "ticks"),
            ("MAX_PERFECT_COOLDOWN_TICKS", "3600", "ticks"),
            ("MAX_WEAPON_GAUGE", "10000", "gauge"),
            (
                "MAX_RALLY",
                &crate::validate::limits::MAX_PLAYER_HP.to_string(),
                "hit points",
            ),
            ("MAX_RALLY_TICKS", "600", "ticks"),
            ("MAX_SPIRIT_LEVEL", "3", "level"),
            ("MAX_SPIRIT_DECAY_TICKS", "3600", "ticks"),
            ("MAX_COUNTER_WINDOW_TICKS", "60", "ticks"),
            ("MAX_TEMPO_STACKS", "10", "stacks"),
            ("MAX_TRIGGER_TICKS", "3600", "ticks"),
            ("MAX_RANGED_GRACE_TICKS", "600", "ticks"),
            ("MAX_REPEATS_QUEUED", "64", "entries"),
            ("MAX_REPEAT_TARGETS", "16", "targets"),
            ("MAX_PERFECT_REACH", "420", "px"),
            ("MAX_PERFECT_SLOW_EVERY", "8", "ticks"),
            ("MAX_ADVANCE_DISTANCE", "240", "px"),
            ("MAX_ADVANCE_PER_TICK", "12", "px"),
        ];
        assert_eq!(rows.len(), 27);
        assert_eq!(MAX_RALLY, crate::validate::limits::MAX_PLAYER_HP);
        for (name, value, unit) in rows {
            let row = LIMITS
                .iter()
                .find(|e| e.name == name)
                .unwrap_or_else(|| panic!("no row for {name}"));
            assert_eq!(row.value, value, "{name}");
            assert_eq!(row.unit, unit, "{name}");
            assert_eq!(row.adr, "ADR-SIM-5", "{name}");
        }
    }

    #[test]
    fn the_view_rows_equal_their_constants() {
        let row = |name: &str| LIMITS.iter().find(|e| e.name == name).unwrap().value;
        for (name, value) in [
            ("VIEW_HALF_WIDTH", VIEW_HALF_WIDTH),
            ("VIEW_HALF_HEIGHT", VIEW_HALF_HEIGHT),
        ] {
            assert_eq!(row(name), value.to_string(), "{name}");
        }
    }

    #[test]
    fn the_catalogue_lists_no_number_of_rules_json() {
        let path = data_root::data_root().join("content/rules.json");
        let rules: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let keys: Vec<&String> = rules.as_object().unwrap().keys().collect();
        for key in ["leash_reach_x", "spawn_margin"] {
            assert!(
                keys.iter().any(|k| k.as_str() == key),
                "{key} not in rules.json"
            );
        }
        for key in keys {
            assert!(
                !LIMITS.iter().any(|e| e.name.eq_ignore_ascii_case(key)),
                "the catalogue lists {key}, a number of content/rules.json"
            );
        }
    }

    #[test]
    fn the_catalogue_points_to_rules_json() {
        assert!(tuning_catalogue_markdown().ends_with("see content/rules.json (ADR-CONT-3).\n"));
    }

    #[test]
    fn catalogue_markdown_has_header_and_entries() {
        let md = tuning_catalogue_markdown();
        assert!(md.starts_with("# Tuning limits catalogue\n"));
        assert!(md.contains("| Name | Value | Unit | ADR |\n|---|---|---|---|\n"));
        for entry in LIMITS {
            let row = format!(
                "| `{}` | {} | {} | {} |",
                entry.name, entry.value, entry.unit, entry.adr
            );
            assert!(
                md.contains(&row),
                "markdown missing row for {}: {row}",
                entry.name
            );
        }
    }
}
