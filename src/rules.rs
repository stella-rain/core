//! The rules file (DIR1, DS103, CS1b-3): the numbers of the simulation that live in the content
//! pack. It starts with the crowd: `crowd_target`, the number of enemies a horde run keeps alive,
//! and `cap_headroom_pct`, how far above the target the enemy cap stands. The designer-facing
//! rules constants follow (CS1b-3): the main character's speed and hurt circle, the orbs' radius,
//! the invulnerable ticks after a hit, aggro, the leash and the spawn margin, the ranged tokens,
//! the orbiting agents' turn, where a horde boss arrives, the default experience, the size of an
//! offer and the damage curve. `World` reads them once when a run is built. Caps, the grid, the
//! push-apart, the view and the units stay in Rust. The cap is
//! `target * (100 + headroom) / 100`, rounded down, derived when a run is built and never stored
//! (260 for the bundled file); the headroom is the room the specials (elites, escorts, a
//! splitter's children, guardians, the boss's summons) spawn in while the waves are held at the
//! target. A mode or a stage may override the target, not the headroom.
//!
//! Unlike `stardust`, the simulation reads this file, so the pack must hold it (a required slot)
//! and the derived content hash covers it: a replay never depends on the client's build.
//!
//! The file is untrusted (ADR-020), as every content file is: a size limit before parsing, strict
//! parsing, the current versions only, and every number bounded. The pack's own target must keep
//! its derived cap within `limits::MAX_ENEMY_CAP` (ADR-SIM-7): a pack over the ceiling is
//! refused. An override is bounded by its own file (1 to `limits::MAX_CROWD_TARGET`) and clamped
//! when the run is built (`Rules::enemy_cap_for`), so the cap never passes the ceiling.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fixed::Fx;
use crate::limits::{VIEW_HALF_HEIGHT, VIEW_HALF_WIDTH};
use crate::pack::Pack;
use crate::rewards::MAX_OFFER_CANDIDATES;
use crate::rules_v0::MAX_RANGED_ATTACKERS;
use crate::validate::limits::{MAX_AGGRO, MAX_ENEMY_CAP, MAX_ENEMY_XP, MAX_RADIUS};
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest rules file read (a placeholder; ADR-020 decides that a limit exists).
pub const MAX_RULES_BYTES: usize = 4 * 1024;
/// The least and the most the cap stands above the target, in percent (DS101: 125 to 130%).
pub const MIN_CAP_HEADROOM_PCT: u8 = 25;
pub const MAX_CAP_HEADROOM_PCT: u8 = 30;

/// The rules file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesFile {
    /// The enemies a horde run keeps alive: wave spawns that fall due at it are held. From 1 to
    /// the most that keeps the cap within the engine ceiling (384 at a headroom of 30).
    pub crowd_target: u16,
    /// How far above the target the cap stands, in percent, from 25 to 30.
    pub cap_headroom_pct: u8,
    /// The main character's speed at full stick, in 1/256 px a tick (ADR-050): 1 to 4,096.
    pub player_speed: Fx,
    /// The main character's hurt circle, in 1/256 px (ADR-050): 1 to `limits::MAX_RADIUS`.
    pub player_radius: Fx,
    /// A friendly orb's hit circle, in 1/256 px: 1 to `limits::MAX_RADIUS`.
    pub orb_radius: Fx,
    /// Ticks of invulnerability after the main character is hit: 0 to 600.
    pub hit_invulnerable_ticks: u32,
    /// Aggro raised per point of damage dealt (ADR-036), in stored units: 0 to 100.
    pub aggro_per_damage: u32,
    /// Aggro raised by a buff (ADR-036), in stored units: 0 to `limits::MAX_AGGRO`.
    pub aggro_per_buff: u32,
    /// Aggro raised per point healed (ADR-036), in stored units: 0 to 100.
    pub aggro_per_heal_point: u32,
    /// Aggro lost every tick (ADR-036), in stored units: 0 to `limits::MAX_AGGRO`.
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
    /// The base candidates of an offer (ADR-079): 1 to `rewards::MAX_OFFER_CANDIDATES`.
    pub offer_size: u8,
    /// The defence that halves a hit (ADR-056), in stored units: 1 to 1,000,000.
    pub defence_k: u32,
    /// What a critical hit deals before any bonus, in percent: 100 to 1,000.
    pub crit_damage_base_pct: u32,
}

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
        validate(&file)?;
        Ok(Rules {
            file,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The crowd target of a run that overrides nothing.
    pub fn crowd_target(&self) -> u16 {
        self.file.crowd_target
    }

    /// How far above the target the cap stands, in percent.
    pub fn cap_headroom_pct(&self) -> u8 {
        self.file.cap_headroom_pct
    }

    /// The main character's speed at full stick, in 1/256 px a tick (ADR-050).
    pub fn player_speed(&self) -> Fx {
        self.file.player_speed
    }

    /// The main character's hurt circle, in 1/256 px (ADR-050).
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

    /// Aggro raised per point of damage dealt (ADR-036), in stored units.
    pub fn aggro_per_damage(&self) -> u32 {
        self.file.aggro_per_damage
    }

    /// Aggro raised by a buff (ADR-036), in stored units.
    pub fn aggro_per_buff(&self) -> u32 {
        self.file.aggro_per_buff
    }

    /// Aggro raised per point healed (ADR-036), in stored units.
    pub fn aggro_per_heal_point(&self) -> u32 {
        self.file.aggro_per_heal_point
    }

    /// Aggro lost every tick (ADR-036), in stored units.
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

    /// Where the leash lands an enemy to the side of the main character, in 1/256 px.
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

    /// The base candidates of an offer (ADR-079).
    pub fn offer_size(&self) -> u8 {
        self.file.offer_size
    }

    /// The defence that halves a hit (ADR-056), in stored units.
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
    // spawn edge (the view's half plus `spawn_margin`) would put a fresh spawn beyond it, and one
    // inside the view's half would land an enemy on screen. The edge is checked only once the
    // margin is in range (else the margin's issue is the one reported).
    let margin = i64::from(file.spawn_margin.0);
    let margin_ok = (0..=px(1024)).contains(&margin);
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
        if land_ok && margin_ok && i64::from(land.0) < px(half) + margin {
            issues.push(Issue {
                path: format!("$.{land_name}"),
                message: format!(
                    "at least the spawn edge: {half} px + spawn_margin (found {})",
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
        px(1024),
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
}
