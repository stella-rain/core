//! The stage validator (ADR-SVC-1): a byte-size limit before parsing, strict parsing, registry
//! lookups with the introducing `sim_version`, and the static budgets of ADR-SVC-1 and ADR-SIM-4.
//! The app and `stage-verify` call the same code, and an invalid stage is not playable (fail
//! closed).
//!
//! Errors name where the problem is (`$.boss.phases[1].timeline[3]`) and what is wrong. They
//! never repeat text from the file, so a hostile stage cannot put its own words in front of a
//! player.
//!
//! Budgets here are tuning values; their existence is the decision. Dynamic budgets (orbs
//! alive, enemies alive) are the simulation's runtime caps, and so is any count that an
//! expression computes: the validator checks constants, the engine clamps the rest.

use crate::id::ContentId;
use crate::registry::{self, Entry, Kind};

mod attack;
mod behaviour;
mod boss;
mod entry;
mod error;
mod stage;
mod weapon;

pub use entry::{
    parse_and_validate, validate, validate_attack, validate_attack_ref, validate_levels_and_stars,
    validate_smash_effect, validate_step_grant, validate_waves, validate_weapon_preset,
    validate_with,
};
pub use error::{Error, Issue};
pub(crate) use weapon::TREE_ID_PROBLEM;

/// Limits a stage must keep. The first block is ADR-SVC-1, the second ADR-SIM-4, the rest the
/// loadout and numeric ranges that ADR-SIM-3 ask for.
pub mod limits {
    pub const MAX_STAGE_BYTES: usize = 256 * 1024;
    /// Five minutes at 60 ticks per second.
    pub const MAX_TICKS: u32 = 5 * 60 * 60;
    pub const MAX_WAVES: usize = 200;
    pub const MAX_COMPANIONS: usize = 4;
    pub const MAX_PHASES: usize = 8;
    pub const MAX_TITLE_CHARS: usize = 60;
    pub const MAX_DESCRIPTION_CHARS: usize = 500;
    /// Stage IDs are slugs (ADR-SVC-7).
    pub const MAX_STAGE_ID_CHARS: usize = 40;

    pub const MAX_RULES: usize = 8;
    pub const MAX_TIMELINE_STEPS: usize = 64;
    pub const MAX_PATTERN_DEPTH: u32 = 4;
    pub const MAX_PATTERN_NODES: u32 = 64;
    pub const MAX_EXPR_NODES: u32 = 16;
    pub const MAX_REPEAT: i32 = 100;
    /// Phases in one attack, and the ticks one lasts: 10 seconds.
    pub const MAX_ATTACK_PHASES: usize = 8;
    pub const MAX_PHASE_TICKS: u32 = 600;

    /// An evade's dash: at most 256 pixels, over at most a second, and at most a second of
    /// invulnerability.
    pub const MAX_EVADE_DISTANCE: i32 = 65_536;
    pub const MAX_EVADE_TICKS: u32 = 60;
    /// An evade holds at most 3 charges.
    pub const MAX_EVADE_CHARGES: u8 = 3;
    /// A charge (core#83): at most 8 levels over at most 10 seconds of game time, and a perfect
    /// bonus of at most +1000%.
    pub const MAX_CHARGE_LEVELS: usize = 8;
    pub const MAX_CHARGE_TICKS: u32 = 600;
    /// A held step's hold: at most 10 seconds, as a charge.
    pub const MAX_HOLD_TICKS: u32 = 600;
    /// A Main weapon's combos: at most 32 steps in its tree and dash branch
    /// together, no path from the root deeper than six steps (six attacks, AAAAAB at most); a
    /// window of at most 2 seconds and a dash-strike of at most 1 second.
    pub const MAX_COMBO_STEPS: usize = 32;
    pub const MAX_COMBO_DEPTH: u32 = 6;
    pub const MAX_COMBO_WINDOW_TICKS: u32 = 120;
    pub const MAX_DASH_STRIKE_TICKS: u32 = 60;
    /// Aim assist looks at most 512 pixels away.
    pub const MAX_ASSIST_RANGE: i32 = 131_072;
    /// The Sub weapon's pull: at most 64 pixels, over at most 1 second.
    pub const MAX_PULL_DISTANCE: i32 = 16_384;
    pub const MAX_PULL_TICKS: u32 = 60;
    /// A stun lasts at most 2 seconds (a placeholder bound).
    pub const MAX_STUN_TICKS: u32 = 120;
    /// A reap smash cashes at most at the bleed's stack maximum, and a boss takes at most all of
    /// the burst (placeholder bounds).
    pub const MAX_REAP_STACKS: u32 = crate::runes::MAX_DOT_STACKS;
    pub const MAX_REAP_BOSS_PCT: u32 = 100;
    /// Twenty seconds: an orb at 2 pixels a tick crosses a 4,096-pixel map in 2,048 ticks,
    /// so a lifetime is a shorter one than the margin's (core#147).
    /// A zone warns, and then hurts, for at most 10 seconds each (core Z1, a placeholder bound).
    pub const MAX_ZONE_TICKS: u32 = 600;
    /// An orb flies for at most 10 seconds (core Z2, a tuning row).
    pub const MAX_ORB_TICKS: u32 = 600;
    /// The longest dash of an attack phase, in pixels: a lane charge, not the width of the view.
    pub const MAX_DASH_DISTANCE: i32 = 960;
    pub const MAX_PERFECT_PCT: u16 = 1000;
    pub const MAX_BOSS_PARTS: usize = 8;
    pub const MAX_TRANSITIONS: usize = 4;
    pub const MAX_PRESET_ARGS: usize = 8;
    pub const MAX_WAVE_COUNT: u16 = 100;
    /// Entries in a stage's `levels.xp`, and the experience in each (ADR-REPL-5).
    pub const MAX_LEVELS: usize = 100;
    pub const MAX_LEVEL_XP: u32 = 1_000_000;
    /// The experience an enemy gives.
    pub const MAX_ENEMY_XP: u32 = 1000;
    /// The expertise points an enemy or the boss gives for a hit and for a kill.
    pub const MAX_EXPERTISE_PER_HIT: u32 = 1000;
    pub const MAX_EXPERTISE_PER_KILL: u32 = 100_000;
    /// Stardust (ADR-SVC-1), held times 10: the base an enemy or the boss earns for a kill
    /// (the bundled boss's is 2,000), the percent of a time factor tier (as
    /// `difficulties::MAX_PCT`), the tiers of a time factor table, and the run's total, at which the counter saturates. At the largest base and percent a
    /// kill earns 1,000,000, so the total is reached in ten kills and never wraps a `u32`.
    pub const MAX_STARDUST_BASE: u32 = 100_000;
    pub const MAX_STARDUST_PCT: u32 = 1_000;
    pub const MAX_STARDUST_TIERS: usize = 16;
    pub const MAX_RUN_STARDUST: u32 = 10_000_000;
    /// Falling stars (ADR-REPL-5): the drop chance is in 1000; the pity, the stars on
    /// the map, the pickup circle (64 pixels), the rerolls and banishes, and the drop table's
    /// tiers, each starting at most at the highest level, with weights of at most 10,000.
    pub const MAX_DROP_PER_MILLE: u16 = 1000;
    pub const MAX_PITY: u16 = 1000;
    pub const MAX_STARS_ON_MAP: u16 = 64;
    /// The most pickups of healing on the map at once: a hard cap (ADR-SVC-1).
    pub const MAX_HEAL_PICKUPS: u16 = 16;
    /// The most a boss kill heals, in percent of the most hit points.
    pub const MAX_HEAL_PCT: u16 = 100;
    /// The chance of a heal pickup is at least 1 in this many; 1 in `MAX_HEAL_ONE_IN` is the
    /// rarest a file may give.
    pub const MAX_HEAL_ONE_IN: u32 = 1_000_000;
    /// The most hit points a pickup heals: 1,000 shown hit points, scaled. A run's
    /// main character holds more than a stage's `MAX_PLAYER_HP` (perks, stat points, rewards),
    /// and a heal never goes beyond the most it holds.
    pub const MAX_HEAL_AMOUNT: u32 = 1_000 * DAMAGE_SCALE;
    pub const MAX_PICKUP_RADIUS: i32 = 16_384;
    pub const MAX_REROLLS: u8 = 20;
    pub const MAX_BANISHES: u8 = 20;
    pub const MAX_TIERS: usize = 16;
    pub const MAX_TIER_LEVEL: u32 = 100;
    pub const MAX_RARITY_WEIGHT: u16 = 10_000;
    pub const MAX_SUMMON_COUNT: u8 = 8;
    /// Elites alive at once, among the enemies.
    pub const MAX_ELITES_ALIVE: u16 = 32;
    /// The engine ceiling on the enemy cap (ADR-SIM-5, DS103): a pack whose derived cap is over it
    /// is refused, as `MAX_ELITES_ALIVE` bounds `elite_cap`. The cap of a run is derived from the
    /// pack's rules and is 260 for the bundled pack; this is the most any pack may ask for.
    pub const MAX_ENEMY_CAP: u16 = 500;
    /// The most a stage's or a mode's `crowd_target` override may ask for (DS103): 500 * 100 / 125,
    /// the most any target can be at the smallest headroom. A run clamps it so that its cap stays
    /// within `MAX_ENEMY_CAP` (`rules::Rules::enemy_cap_for`).
    pub const MAX_CROWD_TARGET: u16 = 400;
    /// The most guardians one shrine rings itself with, or escorts one pack leader brings
    ///.
    pub const MAX_SHRINE_GUARDIANS: u8 = 16;
    /// The most a pack leader's aura adds to the damage of the enemies near it, in percent
    ///.
    pub const MAX_AURA_PERCENT: u16 = 200;

    /// What every default damage and hit-point value is multiplied by, so
    /// that a percentage of a hit (an aura's +25%) is a whole number. The bounds below, `K` of
    /// the aggro constants and the bundled content all hold the scaled values.
    pub const DAMAGE_SCALE: u32 = 100;
    pub const MAX_PLAYER_HP: u32 = 999 * DAMAGE_SCALE;
    pub const MAX_PLAYER_ATK: u32 = 999 * DAMAGE_SCALE;
    pub const MAX_AGENT_HP: u32 = 100_000 * DAMAGE_SCALE;
    pub const MAX_BOSS_HP: u32 = 1_000_000 * DAMAGE_SCALE;
    /// The largest base damage. With the atk-up, the type bonus and the vulnerability at their
    /// caps and the match-up at its cap, a hit of it is `MAX_DAMAGE × 11 × 11 × 11 × 2`: below
    /// `u32::MAX`, as `sim::entity`'s tests check.
    pub const MAX_DAMAGE: u32 = 10_000 * DAMAGE_SCALE;
    /// At this defence a hit keeps about 1% of its damage (K = 100 × `DAMAGE_SCALE`).
    pub const MAX_DEFENCE: u32 = 10_000 * DAMAGE_SCALE;
    pub const MAX_BUFF_PCT: u16 = 500;
    pub const MAX_AGGRO: u32 = 1_000_000 * DAMAGE_SCALE;
    /// A number in an expression, an argument or a velocity-like field.
    pub const MAX_NUMBER: i32 = 1_000_000;
    /// A length in 1/256 pixels: 1,024 pixels.
    pub const MAX_LEN: i32 = 262_144;
    /// The most a boss walks a tick (1/256 pixel): the top of `rules.json`'s `player_speed`.
    pub const MAX_CHASE_SPEED: i32 = 4_096;
    /// A hurt circle radius in 1/256 pixels: 256 pixels.
    pub const MAX_RADIUS: i32 = 65_536;
    /// The portrait field in 1/256 pixels (360 by 640 pixels), the map of a stage without one,
    /// and how far outside a map a point may be.
    pub const FIELD_W: i32 = 92_160;
    pub const FIELD_H: i32 = 163_840;
    pub const FIELD_MARGIN: i32 = 16_384;
    /// A stage's map: each side from 1 to 4,096 pixels (core#87).
    pub const MIN_MAP_SIDE: i32 = 256;
    pub const MAX_MAP_SIDE: i32 = 1_048_576;

    /// Problems reported before the rest are left out.
    pub const MAX_ISSUES: usize = 32;
}

use limits::*;

/// A length for `within`; a length past `i64::MAX` is out of every range, so it saturates.
fn wide(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

enum Seg {
    Key(&'static str),
    Index(usize),
}

struct Ctx<'a> {
    registry: &'a [Entry],
    sim_version: u32,
    /// Points are checked against it.
    map: crate::stage::Map,
    /// Boss part IDs, which conditions and selectors may name.
    parts: Vec<&'a str>,
    path: Vec<Seg>,
    issues: Vec<Issue>,
    left_out: bool,
    /// How many arguments the attack being checked takes: 0 for an inline attack in a stage.
    arity: u8,
    /// Whose attacks are being checked (core Z2).
    foe: Foe,
    /// The rules being checked are a regular enemy's, which may be `ranged`.
    ranged_ok: bool,
    /// The rounds of the `repeat`, `ring` or `spread` nearest round the node being checked: 0 for
    /// none, `i64::MAX` when its count is computed (an orb's `link` needs at least 2).
    loop_rounds: i64,
}

/// Whose attack the validator is in: orbs are only for enemies' and the boss's, `ranged` only for
/// regular enemies' (core Z2).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Foe {
    /// An ally's or the main character's: weapons, skills, runes, companions.
    Other,
    /// A regular enemy, or an elite.
    Enemy,
    Boss,
}

impl<'a> Ctx<'a> {
    fn fail(&mut self, message: impl Into<String>) {
        if self.issues.len() >= MAX_ISSUES {
            self.left_out = true;
            return;
        }
        let mut path = String::from("$");
        for seg in &self.path {
            match seg {
                Seg::Key(k) => {
                    path.push('.');
                    path.push_str(k);
                }
                Seg::Index(i) => path.push_str(&format!("[{i}]")),
            }
        }
        self.issues.push(Issue {
            path,
            message: message.into(),
        });
    }

    fn key<T>(&mut self, key: &'static str, f: impl FnOnce(&mut Self) -> T) -> T {
        self.path.push(Seg::Key(key));
        let out = f(self);
        self.path.pop();
        out
    }

    fn idx<T>(&mut self, index: usize, f: impl FnOnce(&mut Self) -> T) -> T {
        self.path.push(Seg::Index(index));
        let out = f(self);
        self.path.pop();
        out
    }

    /// `value` must be in `lo..=hi`.
    fn within(&mut self, key: &'static str, value: i64, lo: i64, hi: i64) {
        if value < lo || value > hi {
            self.key(key, |c| {
                c.fail(format!("must be between {lo} and {hi} (found {value})"))
            });
        }
    }

    /// A list must hold at most `max` items; returns whether its items should be checked.
    fn at_most(&mut self, key: &'static str, len: usize, max: usize, what: &str) -> bool {
        if len > max {
            self.key(key, |c| {
                c.fail(format!("at most {max} {what} (found {len})"))
            });
            return false;
        }
        true
    }

    /// As `at_most`, for the list at the current path.
    fn at_most_here(&mut self, len: usize, max: usize, what: &str) -> bool {
        if len > max {
            self.fail(format!("at most {max} {what} (found {len})"));
            return false;
        }
        true
    }

    /// A content ID of `kind` that exists and is available to this stage's `sim_version`.
    fn content_id(&mut self, key: &'static str, id: &ContentId, kind: Kind) -> Option<Entry> {
        self.key(key, |c| {
            if !registry::is_well_formed(id.as_str()) {
                c.fail("not a valid content ID (lowercase snake_case, at most 32 characters)");
                return None;
            }
            match registry::find(c.registry, kind, id.as_str()) {
                None => {
                    c.fail(format!("no such {kind:?} ID in the registry"));
                    None
                }
                Some(e) if e.introduced > c.sim_version => {
                    c.fail(format!(
                        "introduced in sim_version {}, newer than this stage's {}",
                        e.introduced, c.sim_version
                    ));
                    None
                }
                Some(e) => Some(*e),
            }
        })
    }

    fn point(&mut self, key: &'static str, p: &crate::fixed::Point) {
        self.key(key, |c| {
            c.within(
                "x",
                i64::from(p.x.0),
                -i64::from(FIELD_MARGIN),
                i64::from(c.map.width.0) + i64::from(FIELD_MARGIN),
            );
            c.within(
                "y",
                i64::from(p.y.0),
                -i64::from(FIELD_MARGIN),
                i64::from(c.map.height.0) + i64::from(FIELD_MARGIN),
            );
        });
    }

    /// Each side bounded (core#87), and the start on the map itself, not in its margin.
    fn map(&mut self, m: &crate::stage::Map) {
        let side = |c: &mut Self, key, v: crate::fixed::Fx| {
            c.within(
                key,
                i64::from(v.0),
                i64::from(MIN_MAP_SIDE),
                i64::from(MAX_MAP_SIDE),
            );
        };
        side(self, "width", m.width);
        side(self, "height", m.height);
        if let Some(start) = m.start {
            self.key("start", |c| {
                c.within("x", i64::from(start.x.0), 0, i64::from(m.width.0));
                c.within("y", i64::from(start.y.0), 0, i64::from(m.height.0));
            });
        }
        if let Some(facing) = m.facing {
            self.within(
                "facing",
                i64::from(facing),
                0,
                i64::from(crate::trig::PER_TURN) - 1,
            );
        }
    }

    fn ticks(&mut self, key: &'static str, ticks: u32, lo: u32) {
        self.within(key, i64::from(ticks), i64::from(lo), i64::from(MAX_TICKS));
    }
}
