//! The rules file (DIR1, DS103, CS1b-3): the numbers of the simulation that live in the content
//! pack. It starts with the crowd: `crowd_target`, the number of enemies a horde run keeps alive,
//! and `cap_headroom_pct`, how far above the target the enemy cap stands. The designer-facing
//! rules constants follow (CS1b-3): the main character's speed and hurt circle, the orbs' radius,
//! the invulnerable ticks after a hit, aggro, the leash and the spawn margin, the ranged tokens,
//! the orbiting agents' turn, where a horde boss arrives, the default experience, the size of an
//! offer and the damage curve, and the table of how much of a player's damage applies for each
//! party size (`party_damage_pct`, ADR-SIM-8). `World` reads them once when a run is built.
//! Caps, the grid, the push-apart, the view and the units stay in Rust. The cap is
//! `target * (100 + headroom) / 100`, rounded down, derived when a run is built and never stored
//! (260 for the bundled file); the headroom is the room the specials (elites, escorts, a
//! splitter's children, guardians, the boss's summons) spawn in while the waves are held at the
//! target. A mode or a stage may override the target, not the headroom.
//!
//! The perfect evade is five optional keys, all or none (`perfect_evade_window_ticks`,
//! `perfect_evade_slow_ticks`, `perfect_evade_cooldown_ticks`, `perfect_evade_reach`,
//! `perfect_evade_slow_every`): none, and the rule is off (today's pack); some, and the file is
//! refused at each missing key; any of them at `sim_version` 1 is refused. Each is bounded by a
//! cap in `limits` and clamped again into `Tuning.perfect_evade` (ADR-SVC-1); the cooldown is
//! not below the slow, so a slow cannot restart inside itself. Only enemies in the evader's reach
//! are slowed (`sim::player`), never other players (ADR-SIM-8).
//!
//! Unlike `stardust`, the simulation reads this file, so the pack must hold it (a required slot)
//! and the derived content hash covers it: a replay never depends on the client's build.
//!
//! The file is untrusted (ADR-SVC-1), as every content file is: a size limit before parsing, strict
//! parsing, the current versions only, and every number bounded. The pack's own target must keep
//! its derived cap within `limits::MAX_ENEMY_CAP` (ADR-SIM-5): a pack over the ceiling is
//! refused. An override is bounded by its own file (1 to `limits::MAX_CROWD_TARGET`) and clamped
//! when the run is built (`Rules::enemy_cap_for`), so the cap never passes the ceiling.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fixed::Fx;
use crate::limits::{
    MAX_CONTROL_REPEATS, MAX_CONTROL_RESET_TICKS, MAX_FLINCH_IMMUNE_TICKS, MAX_FLINCH_TICKS,
    MAX_PCT, MAX_PERFECT_COOLDOWN_TICKS, MAX_PERFECT_REACH, MAX_PERFECT_SLOW_EVERY,
    MAX_PERFECT_SLOW_TICKS, MAX_PERFECT_WINDOW_TICKS, MAX_STAGGER, MAX_STAGGER_DRAIN_TICKS,
    MAX_STAGGER_REFILLS, MAX_STAGGERED_TICKS, VIEW_HALF_HEIGHT, VIEW_HALF_WIDTH,
};
use crate::pack::Pack;
use crate::rewards::MAX_OFFER_CANDIDATES;
use crate::run_stats::MAX_STUN_IMMUNE_TICKS;
use crate::sim::MAX_RANGED_ATTACKERS;
use crate::validate::limits::{
    MAX_AGGRO, MAX_ENEMY_CAP, MAX_ENEMY_XP, MAX_POISE_DAMAGE, MAX_RADIUS,
};
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest rules file read (a placeholder; ADR-SVC-1 decides that a limit exists).
pub const MAX_RULES_BYTES: usize = 4 * 1024;
/// The least and the most the cap stands above the target, in percent (DS101: 125 to 130%).
pub const MIN_CAP_HEADROOM_PCT: u8 = 25;
pub const MAX_CAP_HEADROOM_PCT: u8 = 30;
/// The most ticks a timed offer waits for a pick before the first option is taken: 60 s at 60
/// ticks (ADR-SIM-7 point 4). A number in code, the bound of the pack's `offer_timer_ticks`
/// (ADR-CONT-3 point 4).
pub const MAX_OFFER_TIMER_TICKS: u32 = 3600;
/// The countdown of a pack written before the key existed: 5 s at 60 ticks.
const DEFAULT_OFFER_TIMER_TICKS: u32 = 300;

/// The candidates of an offer in every released pack.
const DEFAULT_OFFER_SIZE: u8 = 3;

fn default_offer_size() -> u8 {
    DEFAULT_OFFER_SIZE
}

fn default_offer_timer_ticks() -> u32 {
    DEFAULT_OFFER_TIMER_TICKS
}

/// The most a party-size entry of `party_damage_pct` can be, in percent: a bigger party only
/// scales a player's damage down, never up (ADR-SIM-8, ADR-CONT-3 point 4).
pub const MAX_PARTY_DAMAGE_PCT: u32 = 100;
/// The players a `party_damage_pct` list has an entry for.
const PARTY_SIZES: usize = crate::id::MAX_PLAYERS as usize;

fn default_party_damage_pct() -> Vec<u32> {
    vec![MAX_PARTY_DAMAGE_PCT; PARTY_SIZES]
}

/// What a control adds to the target's damage taken as `vulnerable`, in percent, by the kind of
/// control that landed. Only the stun for now; the other controls add their own key later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VulnerablePcts {
    /// A stun that lands puts `vulnerable` of this many percent on its target for the stun's
    /// own ticks: 1 to `limits::MAX_PCT` (1,000). Absent: a stun puts none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::limits::MAX_PCT))]
    pub stun: Option<u32>,
    /// A stagger puts `vulnerable` of this many percent on the staggered elite or boss for the
    /// stagger's own ticks: 1 to `limits::MAX_PCT` (1,000). Absent: a stagger puts none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::limits::MAX_PCT))]
    pub stagger: Option<u32>,
}

/// What a resisted control adds to a stagger gauge, by kind of control (each 1 to
/// `limits::MAX_STAGGER`); a kind left out adds nothing. The other controls add their own key later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StaggerValues {
    /// A smash's stun on an elite or the boss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::limits::MAX_STAGGER))]
    pub stun: Option<u32>,
    /// A smash's pull.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::limits::MAX_STAGGER))]
    pub pull: Option<u32>,
    /// A smash's knockback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::limits::MAX_STAGGER))]
    pub knockback: Option<u32>,
}

/// The rules file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesFile {
    /// The enemies a horde run keeps alive: wave spawns that fall due at it are held. From 1 to
    /// the most that keeps the cap within the engine ceiling (384 at a headroom of 30).
    pub crowd_target: u16,
    /// How far above the target the cap stands, in percent, from 25 to 30.
    pub cap_headroom_pct: u8,
    /// The main character's speed at full stick, in 1/256 px a tick: 1 to 4,096.
    pub player_speed: Fx,
    /// The main character's hurt circle, in 1/256 px: 1 to `limits::MAX_RADIUS`.
    pub player_radius: Fx,
    /// A friendly orb's hit circle, in 1/256 px: 1 to `limits::MAX_RADIUS`.
    pub orb_radius: Fx,
    /// Ticks of invulnerability after the main character is hit: 0 to 600.
    pub hit_invulnerable_ticks: u32,
    /// Aggro raised per point of damage dealt (ADR-SIM-4), in stored units: 0 to 100.
    pub aggro_per_damage: u32,
    /// Aggro raised by a buff (ADR-SIM-4), in stored units: 0 to `limits::MAX_AGGRO`.
    pub aggro_per_buff: u32,
    /// Aggro raised per point healed (ADR-SIM-4), in stored units: 0 to 100.
    pub aggro_per_heal_point: u32,
    /// Aggro lost every tick (ADR-SIM-4), in stored units: 0 to `limits::MAX_AGGRO`.
    pub aggro_decay: u32,
    /// How far to the side of the view's centre a regular enemy may be before the leash moves it, in 1/256 px: 1 to 4,096 px.
    pub leash_reach_x: Fx,
    /// The same above or below, in 1/256 px: 1 to 4,096 px.
    pub leash_reach_y: Fx,
    /// Where the leash lands an enemy to the side of the view's centre, in 1/256 px: the spawn edge (420 px + `spawn_margin`) to `leash_reach_x`.
    pub leash_land_x: Fx,
    /// The same above or below, in 1/256 px: the spawn edge (180 px + `spawn_margin`) to `leash_reach_y`.
    pub leash_land_y: Fx,
    /// How far outside the view an enemy spawns, in 1/256 px: 0 to 1,024 px.
    pub spawn_margin: Fx,
    /// Ticks a ranged token cools before its pool grants it again: 0 to 600.
    pub ranged_token_cooldown_ticks: u32,
    /// Tokens of the zone attacks' pool: at least 1; with the line pool at most the engine ceiling of 4 ranged attackers.
    pub ranged_zone_tokens: u32,
    /// Tokens of the line attacks' pool: at least 1; the two pools sum to at most 4.
    pub ranged_line_tokens: u32,
    /// An orbiting agent's turn a tick, in 1/256 degree: 1 to 2,560 (10 degrees).
    pub orbit_step: u32,
    /// How far above the main character a horde boss arrives, in 1/256 px: 0 to 1,024 px.
    pub boss_arrival_above: Fx,
    /// The experience of a summon or a wave that gives none: 0 to `limits::MAX_ENEMY_XP`.
    pub default_xp: u32,
    /// The base candidates of an offer (ADR-REPL-5): 1 to `rewards::MAX_OFFER_CANDIDATES`.
    /// Deprecated (ADR-CONT-2): absent in a pack of `sim_version` 2, where only 3 is accepted;
    /// read only by the frozen version 1 rules. Absent: 3.
    #[serde(default = "default_offer_size")]
    pub offer_size: u8,
    /// The defence that halves a hit, in stored units: 1 to 1,000,000.
    pub defence_k: u32,
    /// What a critical hit deals before any bonus, in percent: 100 to 1,000.
    pub crit_damage_base_pct: u32,
    /// Ticks a timed run's open offer waits for a pick before its first option is taken
    /// (ADR-SIM-7 point 4): 1 to 3,600 (`MAX_OFFER_TIMER_TICKS`). A `stop` run never reads it.
    /// Deprecated (ADR-CONT-2): read by no run; absent in a pack of `sim_version` 2, where only
    /// 300 is accepted. Absent: 300.
    // Read by no run since the offer timer left; kept for pack compatibility. Its doc is the
    // schema's text and moves with the key.
    #[serde(default = "default_offer_timer_ticks")]
    pub offer_timer_ticks: u32,
    /// The percent of a player's damage that applies when 1, 2 or 3 players are present
    /// (ADR-SIM-8): exactly 3 entries, each 1 to 100 (`MAX_PARTY_DAMAGE_PCT`), read at each hit.
    /// Absent: `[100, 100, 100]`, so a pack published before the key existed still reads.
    #[serde(default = "default_party_damage_pct")]
    pub party_damage_pct: Vec<u32>,
    /// The poise damage of a main character's combo hit of an A (`main`) step when its phase
    /// names none: 0 to `validate::limits::MAX_POISE_DAMAGE` (1,000). `sim_version` 2 only.
    /// Absent: 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poise_damage_a: Option<u32>,
    /// The same for a smash step (B): 0 to 1,000. `sim_version` 2 only. Absent: 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poise_damage_b: Option<u32>,
    /// Ticks an enemy that names a `poise` flinches for when a hit deals at least that much
    /// poise damage: 1 to `limits::MAX_FLINCH_TICKS` (60). `sim_version` 2 only. Absent: no
    /// enemy flinches, and no enemy may name a `poise`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flinch_ticks: Option<u16>,
    /// Ticks after a flinch ends in which the enemy cannot flinch again: 0 to
    /// `limits::MAX_FLINCH_IMMUNE_TICKS` (120). Only with `flinch_ticks`. Absent: 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flinch_immune_ticks: Option<u16>,
    /// What percent of its ticks each further hard control (a stun) on one enemy keeps: 1 to
    /// `limits::MAX_CONTROL_REPEATS` (8) entries of 1 to 100; the first control keeps the first
    /// entry, and a control past the last entry is refused. Named together with
    /// `control_reset_ticks` and `control_immune_ticks`, `sim_version` 2 only. Absent: the stun
    /// immunity of the run stats alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_repeat_pcts: Option<Vec<u32>>,
    /// Ticks without a hard control after which an enemy's count of them starts over: 1 to
    /// `limits::MAX_CONTROL_RESET_TICKS` (600). Only with `control_repeat_pcts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_reset_ticks: Option<u16>,
    /// Ticks an enemy is immune to hard controls after one is refused for its count: 1 to 600.
    /// Only with `control_repeat_pcts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_immune_ticks: Option<u32>,
    /// The `vulnerable` percent a landing control puts on its target, by kind of control.
    /// `sim_version` 2 only. Absent: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vulnerable_pcts: Option<VulnerablePcts>,
    /// Ticks an elite or the boss stays staggered when its own definition names none: 1 to
    /// `limits::MAX_STAGGERED_TICKS` (600). `sim_version` 2 only. Absent: each elite or boss that
    /// names a `stagger_max` must name its own `staggered_ticks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staggered_ticks: Option<u16>,
    /// The stagger a main character's smash (B) step adds to the gauge of an elite or the boss
    /// when its phase names none: 0 to `limits::MAX_STAGGER` (10,000). `sim_version` 2 only.
    /// Absent: 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = crate::limits::MAX_STAGGER))]
    pub stagger_smash: Option<u32>,
    /// What a control a smash puts on an elite or the boss, and that it resists, adds to the
    /// gauge. `sim_version` 2 only. Absent: none adds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stagger_values: Option<StaggerValues>,
    /// What percent of its value a fill keeps by the number of bars already broken: 1 to
    /// `limits::MAX_STAGGER_REFILLS` (8) entries of 1 to 100; past the last entry the last one
    /// holds. `sim_version` 2 only. Absent: every bar fills at 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stagger_refill_pcts: Option<Vec<u32>>,
    /// Ticks after a fill before the gauge drains: 1 to `limits::MAX_STAGGER_DRAIN_TICKS` (600).
    /// Only with `stagger_drain_per_tick`, `sim_version` 2 only. Absent: the gauge never drains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stagger_drain_delay_ticks: Option<u16>,
    /// What an idle gauge loses each tick once the delay is over: 1 to `limits::MAX_STAGGER`.
    /// Only with `stagger_drain_delay_ticks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::limits::MAX_STAGGER))]
    pub stagger_drain_per_tick: Option<u32>,
    /// The percent a staggered elite or boss (and its parts) takes more damage: 1 to
    /// `limits::MAX_PCT` (1,000). `sim_version` 2 only. Absent: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = crate::limits::MAX_PCT))]
    pub stagger_damage_pct: Option<u32>,
    /// The perfect evade (`sim_version` 2; all five `perfect_evade_*` keys or none; absent: the
    /// rule is off). Ticks after an effective evade press in which a hit the evade alone stops is
    /// a near miss: 1 to `MAX_PERFECT_WINDOW_TICKS` (30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfect_evade_window_ticks: Option<u32>,
    /// Ticks the enemies in the evader's reach are slowed: 1 to `MAX_PERFECT_SLOW_TICKS` (300).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfect_evade_slow_ticks: Option<u32>,
    /// Ticks before the evader's next perfect evade: from `perfect_evade_slow_ticks` to
    /// `MAX_PERFECT_COOLDOWN_TICKS` (3,600).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfect_evade_cooldown_ticks: Option<u32>,
    /// How near the evader an enemy's centre is slowed, in 1/256 px: 1 px to `MAX_PERFECT_REACH`
    /// (420) px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfect_evade_reach: Option<Fx>,
    /// A slowed enemy acts on the ticks where the slow's remaining ticks are a multiple of this:
    /// 2 to `MAX_PERFECT_SLOW_EVERY` (8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfect_evade_slow_every: Option<u8>,
}

/// The perfect evade's numbers (ADR-CONT-3), read once into `Tuning` and clamped to the caps
/// (ADR-SVC-1). Present only when the pack names all five keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerfectEvade {
    pub window_ticks: u16,
    pub slow_ticks: u16,
    pub cooldown_ticks: u16,
    /// In 1/256 px.
    pub reach: i32,
    pub slow_every: u8,
}

/// The five keys of the perfect evade, in the order the file documents them.
const PERFECT_EVADE_KEYS: [&str; 5] = [
    "perfect_evade_window_ticks",
    "perfect_evade_slow_ticks",
    "perfect_evade_cooldown_ticks",
    "perfect_evade_reach",
    "perfect_evade_slow_every",
];

/// A rules file that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    file: RulesFile,
    sha256: [u8; 32],
}

/// The most a target can be at `headroom_pct` for its cap to stay within the engine ceiling.
fn max_target(headroom_pct: u8) -> u32 {
    u32::from(MAX_ENEMY_CAP) * 100 / (100 + u32::from(headroom_pct))
}

impl Rules {
    /// The bundled file.
    pub fn bundled() -> &'static Rules {
        Pack::bundled().content().rules()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<Rules, Error> {
        let file: RulesFile = version::read_member(bytes, MAX_RULES_BYTES, versions)?;
        let mut issues = match validate(&file) {
            Ok(()) => Vec::new(),
            Err(Error::Invalid(issues)) => issues,
            Err(other) => return Err(other),
        };
        // sim_version >= 2: the two offer keys are deprecated, only their released values read.
        if versions.sim_version >= 2 {
            for (path, deprecated) in [
                ("$.offer_size", file.offer_size != DEFAULT_OFFER_SIZE),
                (
                    "$.offer_timer_ticks",
                    file.offer_timer_ticks != DEFAULT_OFFER_TIMER_TICKS,
                ),
            ] {
                if deprecated && !issues.iter().any(|i| i.path == path) {
                    issues.push(Issue {
                        path: path.into(),
                        message: "deprecated: absent or the default at sim_version 2".into(),
                    });
                }
            }
        }
        // sim_version >= 2: the flinch, control and vulnerable keys are read at version 2 only.
        if versions.sim_version < 2 {
            for path in named_combat_keys(&file) {
                issues.retain(|i| !i.path.starts_with(path));
                issues.push(Issue {
                    path: path.into(),
                    message: "needs sim_version 2".into(),
                });
            }
        }
        // sim_version >= 2: the perfect evade is a rule of the open version.
        if versions.sim_version < 2 {
            for (key, present) in PERFECT_EVADE_KEYS.iter().zip(file.perfect_evade_present()) {
                let path = format!("$.{key}");
                if present && !issues.iter().any(|i| i.path == path) {
                    issues.push(Issue {
                        path,
                        message: "needs sim_version 2".into(),
                    });
                }
            }
        }
        if !issues.is_empty() {
            return Err(Error::Invalid(issues));
        }
        Ok(Rules {
            file,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The poise damage of an A step's hit that names none: 0 when absent.
    pub fn poise_damage_a(&self) -> u32 {
        self.file.poise_damage_a.unwrap_or(0)
    }

    /// The same for a smash step's hit.
    pub fn poise_damage_b(&self) -> u32 {
        self.file.poise_damage_b.unwrap_or(0)
    }

    /// The ticks of a flinch; 0 when no enemy flinches.
    pub fn flinch_ticks(&self) -> u16 {
        self.file.flinch_ticks.unwrap_or(0)
    }

    /// The ticks of immunity after a flinch ends; 0 when absent.
    pub fn flinch_immune_ticks(&self) -> u16 {
        self.file.flinch_immune_ticks.unwrap_or(0)
    }

    /// The percent of its ticks each further hard control keeps; empty when no list is named.
    pub fn control_repeat_pcts(&self) -> &[u32] {
        self.file.control_repeat_pcts.as_deref().unwrap_or(&[])
    }

    /// Ticks after which an enemy's hard-control count starts over; 0 when no list is named.
    pub fn control_reset_ticks(&self) -> u16 {
        self.file.control_reset_ticks.unwrap_or(0)
    }

    /// Ticks of immunity to hard controls once the list is used up; 0 when no list is named.
    pub fn control_immune_ticks(&self) -> u32 {
        self.file.control_immune_ticks.unwrap_or(0)
    }

    /// The `vulnerable` percent a stun that lands puts on its target; 0 when absent.
    pub fn vulnerable_stun_pct(&self) -> u32 {
        self.file
            .vulnerable_pcts
            .as_ref()
            .and_then(|v| v.stun)
            .unwrap_or(0)
    }

    /// The ticks of a stagger an elite or boss that names none has; 0 when absent.
    pub fn staggered_ticks(&self) -> u16 {
        self.file.staggered_ticks.unwrap_or(0)
    }

    /// The stagger a smash step adds when its phase names none; 0 when absent.
    pub fn stagger_smash(&self) -> u32 {
        self.file.stagger_smash.unwrap_or(0)
    }

    /// The gauge value of a resisted `[stun, pull, knockback]`; 0 for a kind left out.
    pub fn stagger_values(&self) -> [u32; 3] {
        let v = self.file.stagger_values.as_ref();
        [
            v.and_then(|v| v.stun).unwrap_or(0),
            v.and_then(|v| v.pull).unwrap_or(0),
            v.and_then(|v| v.knockback).unwrap_or(0),
        ]
    }

    /// The fill percent by bars broken; empty when every bar fills in full.
    pub fn stagger_refill_pcts(&self) -> &[u32] {
        self.file.stagger_refill_pcts.as_deref().unwrap_or(&[])
    }

    /// The drain of an idle gauge as `(delay ticks, per tick)`; `(0, 0)` when it never drains.
    pub fn stagger_drain(&self) -> (u16, u32) {
        match (
            self.file.stagger_drain_delay_ticks,
            self.file.stagger_drain_per_tick,
        ) {
            (Some(delay), Some(per_tick)) => (delay, per_tick),
            _ => (0, 0),
        }
    }

    /// The percent a staggered target takes more damage; 0 when absent.
    pub fn stagger_damage_pct(&self) -> u32 {
        self.file.stagger_damage_pct.unwrap_or(0)
    }

    /// The `vulnerable` percent a stagger puts on its target; 0 when absent.
    pub fn vulnerable_stagger_pct(&self) -> u32 {
        self.file
            .vulnerable_pcts
            .as_ref()
            .and_then(|v| v.stagger)
            .unwrap_or(0)
    }

    /// The crowd target of a run that overrides nothing.
    pub fn crowd_target(&self) -> u16 {
        self.file.crowd_target
    }

    /// How far above the target the cap stands, in percent.
    pub fn cap_headroom_pct(&self) -> u8 {
        self.file.cap_headroom_pct
    }

    /// The main character's speed at full stick, in 1/256 px a tick.
    pub fn player_speed(&self) -> Fx {
        self.file.player_speed
    }

    /// The main character's hurt circle, in 1/256 px.
    pub fn player_radius(&self) -> Fx {
        self.file.player_radius
    }

    /// A friendly orb's hit circle, in 1/256 px.
    pub fn orb_radius(&self) -> Fx {
        self.file.orb_radius
    }

    /// Ticks of invulnerability after the main character is hit.
    pub fn hit_invulnerable_ticks(&self) -> u32 {
        self.file.hit_invulnerable_ticks
    }

    /// Aggro raised per point of damage dealt (ADR-SIM-4), in stored units.
    pub fn aggro_per_damage(&self) -> u32 {
        self.file.aggro_per_damage
    }

    /// Aggro raised by a buff (ADR-SIM-4), in stored units.
    pub fn aggro_per_buff(&self) -> u32 {
        self.file.aggro_per_buff
    }

    /// Aggro raised per point healed (ADR-SIM-4), in stored units.
    pub fn aggro_per_heal_point(&self) -> u32 {
        self.file.aggro_per_heal_point
    }

    /// Aggro lost every tick (ADR-SIM-4), in stored units.
    pub fn aggro_decay(&self) -> u32 {
        self.file.aggro_decay
    }

    /// How far to the side of the view's centre a regular enemy may be before the leash moves it, in 1/256 px.
    pub fn leash_reach_x(&self) -> Fx {
        self.file.leash_reach_x
    }

    /// The same above or below, in 1/256 px.
    pub fn leash_reach_y(&self) -> Fx {
        self.file.leash_reach_y
    }

    /// Where the leash lands an enemy to the side of the view's centre, in 1/256 px.
    pub fn leash_land_x(&self) -> Fx {
        self.file.leash_land_x
    }

    /// The same above or below, in 1/256 px.
    pub fn leash_land_y(&self) -> Fx {
        self.file.leash_land_y
    }

    /// How far outside the view an enemy spawns, in 1/256 px.
    pub fn spawn_margin(&self) -> Fx {
        self.file.spawn_margin
    }

    /// Ticks a ranged token cools before its pool grants it again.
    pub fn ranged_token_cooldown_ticks(&self) -> u32 {
        self.file.ranged_token_cooldown_ticks
    }

    /// Tokens of the zone attacks' pool.
    pub fn ranged_zone_tokens(&self) -> u32 {
        self.file.ranged_zone_tokens
    }

    /// Tokens of the line attacks' pool.
    pub fn ranged_line_tokens(&self) -> u32 {
        self.file.ranged_line_tokens
    }

    /// An orbiting agent's turn a tick, in 1/256 degree.
    pub fn orbit_step(&self) -> u32 {
        self.file.orbit_step
    }

    /// How far above the main character a horde boss arrives, in 1/256 px.
    pub fn boss_arrival_above(&self) -> Fx {
        self.file.boss_arrival_above
    }

    /// The experience of a summon or a wave that gives none.
    pub fn default_xp(&self) -> u32 {
        self.file.default_xp
    }

    /// The base candidates of an offer (ADR-REPL-5).
    pub fn offer_size(&self) -> u8 {
        self.file.offer_size
    }

    /// The pack's `offer_timer_ticks`; read by no run (kept for pack compatibility).
    pub fn offer_timer_ticks(&self) -> u32 {
        self.file.offer_timer_ticks
    }

    /// The percent of a player's damage that applies for 1, 2 and 3 players present
    /// (ADR-SIM-8). The list has three entries once validated.
    pub fn party_damage_pct(&self) -> [u32; PARTY_SIZES] {
        let mut table = [MAX_PARTY_DAMAGE_PCT; PARTY_SIZES];
        for (to, from) in table.iter_mut().zip(&self.file.party_damage_pct) {
            *to = *from;
        }
        table
    }

    /// The perfect evade's numbers, clamped to the caps; `None` when the pack names none.
    pub fn perfect_evade(&self) -> Option<PerfectEvade> {
        let f = &self.file;
        let cap = |v: u32, most: u16| u16::try_from(v).map_or(most, |v| v.min(most));
        Some(PerfectEvade {
            window_ticks: cap(f.perfect_evade_window_ticks?, MAX_PERFECT_WINDOW_TICKS),
            slow_ticks: cap(f.perfect_evade_slow_ticks?, MAX_PERFECT_SLOW_TICKS),
            cooldown_ticks: cap(f.perfect_evade_cooldown_ticks?, MAX_PERFECT_COOLDOWN_TICKS),
            reach: f.perfect_evade_reach?.0.min(Fx::px(MAX_PERFECT_REACH).0),
            slow_every: f.perfect_evade_slow_every?.min(MAX_PERFECT_SLOW_EVERY),
        })
    }

    /// The defence that halves a hit, in stored units.
    pub fn defence_k(&self) -> u32 {
        self.file.defence_k
    }

    /// What a critical hit deals before any bonus, in percent.
    pub fn crit_damage_base_pct(&self) -> u32 {
        self.file.crit_damage_base_pct
    }

    /// The target a run resolves: `target` (an override) or the file's, kept so that its cap
    /// stays within the engine ceiling.
    pub fn resolved_target(&self, target: Option<u16>) -> u32 {
        u32::from(target.unwrap_or(self.file.crowd_target))
            .min(max_target(self.file.cap_headroom_pct))
    }

    /// The enemy cap of a run whose target is `target` (clamped as `resolved_target` does):
    /// `target * (100 + headroom) / 100`, rounded down. Never stored; at most the ceiling.
    pub fn enemy_cap_for(&self, target: u16) -> u32 {
        self.resolved_target(Some(target)) * (100 + u32::from(self.file.cap_headroom_pct)) / 100
    }

    /// The SHA-256 of the bytes it was parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

impl RulesFile {
    /// Which of the five perfect-evade keys the file names.
    fn perfect_evade_present(&self) -> [bool; 5] {
        [
            self.perfect_evade_window_ticks.is_some(),
            self.perfect_evade_slow_ticks.is_some(),
            self.perfect_evade_cooldown_ticks.is_some(),
            self.perfect_evade_reach.is_some(),
            self.perfect_evade_slow_every.is_some(),
        ]
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &RulesFile) -> Result<(), Error> {
    let mut issues = Vec::new();
    let headroom = file.cap_headroom_pct;
    let headroom_ok = (MIN_CAP_HEADROOM_PCT..=MAX_CAP_HEADROOM_PCT).contains(&headroom);
    if !headroom_ok {
        issues.push(Issue {
            path: "$.cap_headroom_pct".into(),
            message: format!("{MIN_CAP_HEADROOM_PCT} to {MAX_CAP_HEADROOM_PCT} (found {headroom})"),
        });
    } else {
        let most = max_target(headroom);
        let target = u32::from(file.crowd_target);
        if target == 0 || target > most {
            issues.push(Issue {
                path: "$.crowd_target".into(),
                message: format!("1 to {most} at headroom {headroom} (found {target})"),
            });
        }
    }
    validate_rules(file, &mut issues);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// The paths of the flinch, control and vulnerable keys the file names (`sim_version` 2 only).
fn named_combat_keys(file: &RulesFile) -> Vec<&'static str> {
    let named = [
        ("$.poise_damage_a", file.poise_damage_a.is_some()),
        ("$.poise_damage_b", file.poise_damage_b.is_some()),
        ("$.flinch_ticks", file.flinch_ticks.is_some()),
        ("$.flinch_immune_ticks", file.flinch_immune_ticks.is_some()),
        ("$.control_repeat_pcts", file.control_repeat_pcts.is_some()),
        ("$.control_reset_ticks", file.control_reset_ticks.is_some()),
        (
            "$.control_immune_ticks",
            file.control_immune_ticks.is_some(),
        ),
        ("$.vulnerable_pcts", file.vulnerable_pcts.is_some()),
        ("$.staggered_ticks", file.staggered_ticks.is_some()),
        ("$.stagger_smash", file.stagger_smash.is_some()),
        ("$.stagger_values", file.stagger_values.is_some()),
        ("$.stagger_refill_pcts", file.stagger_refill_pcts.is_some()),
        (
            "$.stagger_drain_delay_ticks",
            file.stagger_drain_delay_ticks.is_some(),
        ),
        (
            "$.stagger_drain_per_tick",
            file.stagger_drain_per_tick.is_some(),
        ),
        ("$.stagger_damage_pct", file.stagger_damage_pct.is_some()),
    ];
    named
        .into_iter()
        .filter(|(_, named)| *named)
        .map(|(path, _)| path)
        .collect()
}

/// A number outside `least` to `most` is one issue at `$.<field>`.
fn within(issues: &mut Vec<Issue>, field: &str, value: i64, least: i64, most: i64) -> bool {
    let ok = (least..=most).contains(&value);
    if !ok {
        issues.push(Issue {
            path: format!("$.{field}"),
            message: format!("{least} to {most} (found {value})"),
        });
    }
    ok
}

/// The checks of the designer-facing fields (CS1b-3).
fn validate_rules(file: &RulesFile, issues: &mut Vec<Issue>) {
    let px = |n: i32| i64::from(Fx::px(n).0);
    let radius = i64::from(MAX_RADIUS);
    let max_aggro = i64::from(MAX_AGGRO);
    within(
        issues,
        "player_speed",
        i64::from(file.player_speed.0),
        1,
        4096,
    );
    within(
        issues,
        "player_radius",
        i64::from(file.player_radius.0),
        1,
        radius,
    );
    within(
        issues,
        "orb_radius",
        i64::from(file.orb_radius.0),
        1,
        radius,
    );
    within(
        issues,
        "hit_invulnerable_ticks",
        i64::from(file.hit_invulnerable_ticks),
        0,
        600,
    );
    within(
        issues,
        "aggro_per_damage",
        i64::from(file.aggro_per_damage),
        0,
        100,
    );
    within(
        issues,
        "aggro_per_buff",
        i64::from(file.aggro_per_buff),
        0,
        max_aggro,
    );
    within(
        issues,
        "aggro_per_heal_point",
        i64::from(file.aggro_per_heal_point),
        0,
        100,
    );
    within(
        issues,
        "aggro_decay",
        i64::from(file.aggro_decay),
        0,
        max_aggro,
    );
    // A land outside where the leash triggers would move an enemy every tick; a land inside the
    // spawn edge (the view's half plus `spawn_margin`) would put a fresh spawn beyond it and
    // land a relocated enemy inside the view (unless neither side fits and the border clamps it
    // there anyway).
    // The edge is checked only once the margin is in range (else the margin's issue is the one
    // reported).
    let margin = i64::from(file.spawn_margin.0);
    let margin_most = px(1024);
    let margin_ok = (0..=margin_most).contains(&margin);
    for (reach, land, half, reach_name, land_name) in [
        (
            file.leash_reach_x,
            file.leash_land_x,
            VIEW_HALF_WIDTH,
            "leash_reach_x",
            "leash_land_x",
        ),
        (
            file.leash_reach_y,
            file.leash_land_y,
            VIEW_HALF_HEIGHT,
            "leash_reach_y",
            "leash_land_y",
        ),
    ] {
        let reach_ok = within(issues, reach_name, i64::from(reach.0), 1, px(4096));
        let land_ok = if !reach_ok {
            within(issues, land_name, i64::from(land.0), 1, px(4096))
        } else if i64::from(land.0) > i64::from(reach.0) {
            issues.push(Issue {
                path: format!("$.{land_name}"),
                message: format!("at most {reach_name} (found {})", land.0),
            });
            false
        } else {
            within(issues, land_name, i64::from(land.0), 1, i64::from(reach.0))
        };
        let edge = px(half) + margin;
        if land_ok && margin_ok && i64::from(land.0) < edge {
            issues.push(Issue {
                path: format!("$.{land_name}"),
                message: format!(
                    "at least the spawn edge, {half} px + spawn_margin: {edge} (found {})",
                    land.0
                ),
            });
        }
    }
    within(
        issues,
        "spawn_margin",
        i64::from(file.spawn_margin.0),
        0,
        margin_most,
    );
    within(
        issues,
        "ranged_token_cooldown_ticks",
        i64::from(file.ranged_token_cooldown_ticks),
        0,
        600,
    );
    let ceiling = i64::from(MAX_RANGED_ATTACKERS);
    let zone = i64::from(file.ranged_zone_tokens);
    let line = i64::from(file.ranged_line_tokens);
    let zone_ok = zone >= 1;
    let line_ok = line >= 1;
    if !zone_ok {
        issues.push(Issue {
            path: "$.ranged_zone_tokens".into(),
            message: format!("at least 1 (found {zone})"),
        });
    }
    if !line_ok {
        issues.push(Issue {
            path: "$.ranged_line_tokens".into(),
            message: format!("at least 1 (found {line})"),
        });
    }
    if zone_ok && line_ok && zone + line > ceiling {
        issues.push(Issue {
            path: "$.ranged_line_tokens".into(),
            message: format!(
                "the two pools sum to at most {ceiling} (found {})",
                zone + line
            ),
        });
    }
    within(issues, "orbit_step", i64::from(file.orbit_step), 1, 2560);
    within(
        issues,
        "boss_arrival_above",
        i64::from(file.boss_arrival_above.0),
        0,
        px(1024),
    );
    within(
        issues,
        "default_xp",
        i64::from(file.default_xp),
        0,
        i64::from(MAX_ENEMY_XP),
    );
    within(
        issues,
        "offer_size",
        i64::from(file.offer_size),
        1,
        i64::try_from(MAX_OFFER_CANDIDATES).unwrap_or(i64::MAX),
    );
    within(issues, "defence_k", i64::from(file.defence_k), 1, 1_000_000);
    within(
        issues,
        "crit_damage_base_pct",
        i64::from(file.crit_damage_base_pct),
        100,
        1000,
    );
    within(
        issues,
        "offer_timer_ticks",
        i64::from(file.offer_timer_ticks),
        1,
        i64::from(MAX_OFFER_TIMER_TICKS),
    );
    if file.party_damage_pct.len() != PARTY_SIZES {
        issues.push(Issue {
            path: "$.party_damage_pct".into(),
            message: format!(
                "{PARTY_SIZES} entries (found {})",
                file.party_damage_pct.len()
            ),
        });
    } else {
        for (i, pct) in file.party_damage_pct.iter().enumerate() {
            within(
                issues,
                &format!("party_damage_pct[{i}]"),
                i64::from(*pct),
                1,
                i64::from(MAX_PARTY_DAMAGE_PCT),
            );
        }
    }
    validate_combat_keys(file, issues);
    validate_perfect_evade(file, issues);
}

/// The checks of the flinch, control and vulnerable keys: each bound, and each key with its
/// partners (the control list, its reset and its immunity come together; the flinch immunity
/// needs the flinch).
fn validate_combat_keys(file: &RulesFile, issues: &mut Vec<Issue>) {
    let max_damage = i64::from(MAX_POISE_DAMAGE);
    for (field, value) in [
        ("poise_damage_a", file.poise_damage_a),
        ("poise_damage_b", file.poise_damage_b),
    ] {
        if let Some(n) = value {
            within(issues, field, i64::from(n), 0, max_damage);
        }
    }
    if let Some(n) = file.flinch_ticks {
        within(
            issues,
            "flinch_ticks",
            i64::from(n),
            1,
            i64::from(MAX_FLINCH_TICKS),
        );
    }
    if let Some(n) = file.flinch_immune_ticks {
        if file.flinch_ticks.is_none() {
            issues.push(Issue {
                path: "$.flinch_immune_ticks".into(),
                message: "needs flinch_ticks".into(),
            });
        } else {
            within(
                issues,
                "flinch_immune_ticks",
                i64::from(n),
                0,
                i64::from(MAX_FLINCH_IMMUNE_TICKS),
            );
        }
    }
    if let Some(list) = &file.control_repeat_pcts {
        let most = usize::from(MAX_CONTROL_REPEATS);
        if list.is_empty() || list.len() > most {
            issues.push(Issue {
                path: "$.control_repeat_pcts".into(),
                message: format!("1 to {most} entries (found {})", list.len()),
            });
        } else if file.control_reset_ticks.is_none() || file.control_immune_ticks.is_none() {
            issues.push(Issue {
                path: "$.control_repeat_pcts".into(),
                message: "needs control_reset_ticks and control_immune_ticks".into(),
            });
        }
        for (i, pct) in list.iter().take(most).enumerate() {
            within(
                issues,
                &format!("control_repeat_pcts[{i}]"),
                i64::from(*pct),
                1,
                100,
            );
        }
    }
    for (field, value, most) in [
        (
            "control_reset_ticks",
            file.control_reset_ticks.map(i64::from),
            i64::from(MAX_CONTROL_RESET_TICKS),
        ),
        (
            "control_immune_ticks",
            file.control_immune_ticks.map(i64::from),
            i64::from(MAX_STUN_IMMUNE_TICKS),
        ),
    ] {
        let Some(n) = value else { continue };
        if file.control_repeat_pcts.is_none() {
            issues.push(Issue {
                path: format!("$.{field}"),
                message: "needs control_repeat_pcts".into(),
            });
        } else {
            within(issues, field, n, 1, most);
        }
    }
    if let Some(stun) = file.vulnerable_pcts.as_ref().and_then(|v| v.stun) {
        within(
            issues,
            "vulnerable_pcts.stun",
            i64::from(stun),
            1,
            i64::from(MAX_PCT),
        );
    }
    validate_stagger_keys(file, issues);
}

/// The checks of the stagger keys: each bound, and the drain pair together.
fn validate_stagger_keys(file: &RulesFile, issues: &mut Vec<Issue>) {
    if let Some(stagger) = file.vulnerable_pcts.as_ref().and_then(|v| v.stagger) {
        within(
            issues,
            "vulnerable_pcts.stagger",
            i64::from(stagger),
            1,
            i64::from(MAX_PCT),
        );
    }
    if let Some(n) = file.staggered_ticks {
        within(
            issues,
            "staggered_ticks",
            i64::from(n),
            1,
            i64::from(MAX_STAGGERED_TICKS),
        );
    }
    if let Some(n) = file.stagger_smash {
        within(
            issues,
            "stagger_smash",
            i64::from(n),
            0,
            i64::from(MAX_STAGGER),
        );
    }
    if let Some(v) = &file.stagger_values {
        for (field, value) in [
            ("stagger_values.stun", v.stun),
            ("stagger_values.pull", v.pull),
            ("stagger_values.knockback", v.knockback),
        ] {
            if let Some(n) = value {
                within(issues, field, i64::from(n), 1, i64::from(MAX_STAGGER));
            }
        }
    }
    if let Some(list) = &file.stagger_refill_pcts {
        let most = usize::from(MAX_STAGGER_REFILLS);
        if list.is_empty() || list.len() > most {
            issues.push(Issue {
                path: "$.stagger_refill_pcts".into(),
                message: format!("1 to {most} entries (found {})", list.len()),
            });
        }
        for (i, pct) in list.iter().take(most).enumerate() {
            within(
                issues,
                &format!("stagger_refill_pcts[{i}]"),
                i64::from(*pct),
                1,
                100,
            );
        }
    }
    match (file.stagger_drain_delay_ticks, file.stagger_drain_per_tick) {
        (Some(_), None) => issues.push(Issue {
            path: "$.stagger_drain_delay_ticks".into(),
            message: "needs stagger_drain_per_tick".into(),
        }),
        (None, Some(_)) => issues.push(Issue {
            path: "$.stagger_drain_per_tick".into(),
            message: "needs stagger_drain_delay_ticks".into(),
        }),
        (Some(delay), Some(per_tick)) => {
            within(
                issues,
                "stagger_drain_delay_ticks",
                i64::from(delay),
                1,
                i64::from(MAX_STAGGER_DRAIN_TICKS),
            );
            within(
                issues,
                "stagger_drain_per_tick",
                i64::from(per_tick),
                1,
                i64::from(MAX_STAGGER),
            );
        }
        (None, None) => {}
    }
    if let Some(n) = file.stagger_damage_pct {
        within(
            issues,
            "stagger_damage_pct",
            i64::from(n),
            1,
            i64::from(MAX_PCT),
        );
    }
}

/// The perfect evade's keys: all five or none, each bounded; the cooldown is not below the slow,
/// so a slow cannot restart inside itself.
fn validate_perfect_evade(file: &RulesFile, issues: &mut Vec<Issue>) {
    let present = file.perfect_evade_present();
    if present.iter().all(|p| !p) {
        return;
    }
    if present.iter().any(|p| !p) {
        for (key, here) in PERFECT_EVADE_KEYS.iter().zip(present) {
            if !here {
                issues.push(Issue {
                    path: format!("$.{key}"),
                    message: "all five perfect_evade keys or none".into(),
                });
            }
        }
        return;
    }
    let (Some(window), Some(slow), Some(cooldown), Some(reach), Some(every)) = (
        file.perfect_evade_window_ticks,
        file.perfect_evade_slow_ticks,
        file.perfect_evade_cooldown_ticks,
        file.perfect_evade_reach,
        file.perfect_evade_slow_every,
    ) else {
        return;
    };
    within(
        issues,
        "perfect_evade_window_ticks",
        i64::from(window),
        1,
        i64::from(MAX_PERFECT_WINDOW_TICKS),
    );
    let slow_ok = within(
        issues,
        "perfect_evade_slow_ticks",
        i64::from(slow),
        1,
        i64::from(MAX_PERFECT_SLOW_TICKS),
    );
    within(
        issues,
        "perfect_evade_cooldown_ticks",
        i64::from(cooldown),
        if slow_ok { i64::from(slow) } else { 1 },
        i64::from(MAX_PERFECT_COOLDOWN_TICKS),
    );
    within(
        issues,
        "perfect_evade_reach",
        i64::from(reach.0),
        i64::from(Fx::px(1).0),
        i64::from(Fx::px(MAX_PERFECT_REACH).0),
    );
    within(
        issues,
        "perfect_evade_slow_every",
        i64::from(every),
        2,
        i64::from(MAX_PERFECT_SLOW_EVERY),
    );
}
