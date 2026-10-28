//! Stage skills (ADR-075, ADR-098): what the `skill` button uses. A run's one slot starts empty
//! and is filled from falling stars: a reward with the `stage_skill` effect names one of the
//! stage skills file's skills. Taking a skill other than the one held replaces it, and it
//! arrives ready; taking the same skill again raises its level, up to its highest.
//!
//! Data picks, code acts (ADR-067): each level's effects are building blocks from a closed list
//! built into `core`, with their numbers here, at most one of each block and at most
//! `MAX_SKILL_EFFECTS`. A new block is a code change and a new `sim_version`. A skill's look is
//! content too: its icon, title and description.
//!
//! The file is untrusted (ADR-020), as the rewards are: a size limit before parsing, strict
//! parsing, the current versions only, and every field bounded; the bounds are tuning values.
//! The bundled file, `content/stage_skills.json`, is the default; the SHA-256 of whatever bytes
//! a run used is part of its content hash.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attack::AttackRef;
use crate::fixed::Fx;
use crate::id::ContentId;
use crate::pack::Pack;
use crate::registry::{self, Kind};
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};

/// The largest stage skills file read.
pub const MAX_STAGE_SKILLS_BYTES: usize = 32 * 1024;
/// The most stage skills a file lists.
pub const MAX_STAGE_SKILLS: usize = 16;
/// The most levels a skill has.
pub const MAX_SKILL_LEVELS: usize = 5;
/// The most effects a level combines, of the five blocks.
pub const MAX_SKILL_EFFECTS: usize = 3;
/// The longest cooldown: one minute.
pub const MAX_SKILL_COOLDOWN_TICKS: u32 = 3_600;
/// The longest a pull, a status or a shield lasts: 10 seconds.
pub const MAX_SKILL_DURATION_TICKS: u32 = 600;
/// The largest area: a radius of 256 pixels.
pub const MAX_SKILL_RADIUS: i32 = 65_536;
/// The farthest ahead a skill acts: 512 pixels.
pub const MAX_SKILL_AHEAD: i32 = 131_072;
/// The strongest pull: 8 pixels a tick.
pub const MAX_PULL: i32 = 2_048;
/// The largest status: +1000%.
pub const MAX_SKILL_PCT: u16 = 1_000;
/// The most damage a shield absorbs: 10,000 at the design values, scaled (ADR-103).
pub const MAX_SHIELD: u32 = 10_000 * crate::validate::limits::DAMAGE_SCALE;

/// One stage skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StageSkill {
    /// The skill's own ID, unique within the file; a reward names it.
    pub id: ContentId,
    /// The asset the skill button draws (ADR-026): an asset ID of the registry.
    pub icon: ContentId,
    /// How far ahead of the main character its point is, along the facing, or toward the aim
    /// assist's target while `assist` is on (ADR-072); 0, the default, is at the main
    /// character. An attack places its own hit boxes instead.
    #[serde(default, skip_serializing_if = "is_zero_fx")]
    pub ahead: Fx,
    /// Level 1 first.
    pub levels: Vec<SkillLevel>,
}

fn is_zero_fx(v: &Fx) -> bool {
    v.0 == 0
}

/// One level of a stage skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillLevel {
    /// Ticks after a use before it is ready again, from 1 to `MAX_SKILL_COOLDOWN_TICKS`.
    pub cooldown_ticks: u32,
    /// What a use does, in this order: 1 to `MAX_SKILL_EFFECTS` effects, at most one of each
    /// block.
    pub effects: Vec<SkillEffect>,
}

/// One building block of a use (ADR-098 §2), at the skill's point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillEffect {
    /// For `duration_ticks`, every enemy whose circle reaches within `radius` of the point is
    /// pulled `strength` toward it each tick, after the enemies have moved, and so held there.
    /// The boss and its parts are not pulled. A new pull replaces the one still pulling.
    Pull {
        radius: Fx,
        strength: Fx,
        duration_ticks: u32,
    },
    /// `status` (a status ID of the registry, ADR-036) on the main character, `value_pct`
    /// percent, for `duration_ticks`.
    SelfStatus {
        status: ContentId,
        value_pct: u16,
        duration_ticks: u32,
    },
    /// `status` on every enemy, the boss and its parts included, whose circle reaches within
    /// `radius` of the point, `value_pct` percent, for `duration_ticks`.
    AreaStatus {
        status: ContentId,
        radius: Fx,
        value_pct: u16,
        duration_ticks: u32,
    },
    /// `attack` runs in the stage skill's own attack slot (ADR-069), along the aim, at the main
    /// character's attack stat: it neither waits for nor cancels a combo step. A new attack
    /// ends the one still running.
    Attack { attack: AttackRef },
    /// Absorbs up to `amount` damage to the main character for `duration_ticks`. A new shield
    /// replaces the one held.
    Shield { amount: u32, duration_ticks: u32 },
}

impl SkillEffect {
    /// The block's name, as the file writes its `type`.
    pub fn block(&self) -> &'static str {
        match self {
            SkillEffect::Pull { .. } => "pull",
            SkillEffect::SelfStatus { .. } => "self_status",
            SkillEffect::AreaStatus { .. } => "area_status",
            SkillEffect::Attack { .. } => "attack",
            SkillEffect::Shield { .. } => "shield",
        }
    }
}

/// The stage skills file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StageSkillsFile {
    /// The skills, at most `MAX_STAGE_SKILLS`; a skill's place here is what the slot holds.
    pub stage_skills: Vec<StageSkill>,
}

/// Stage skills that passed validation, and the SHA-256 of the bytes they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageSkills {
    skills: Vec<StageSkill>,
    sha256: [u8; 32],
}

impl StageSkills {
    /// The bundled stage skills, the default of every run.
    pub fn bundled() -> &'static StageSkills {
        Pack::bundled().content().stage_skills()
    }

    /// Checks the size and the versions given (the pack's), parses strictly and validates: the one
    /// call for a file's bytes.
    pub fn parse(bytes: &[u8], versions: Versions) -> Result<StageSkills, Error> {
        let file: StageSkillsFile = version::read_member(bytes, MAX_STAGE_SKILLS_BYTES, versions)?;
        validate(&file, versions.sim_version)?;
        Ok(StageSkills {
            skills: file.stage_skills,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The skills, in the file's order.
    pub fn skills(&self) -> &[StageSkill] {
        &self.skills
    }

    /// The skill at `index` in the file.
    pub fn get(&self, index: usize) -> Option<&StageSkill> {
        self.skills.get(index)
    }

    /// The place of the skill `id` in the file, if it has one.
    pub fn index_of(&self, id: &ContentId) -> Option<usize> {
        self.skills.iter().position(|k| &k.id == id)
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(file: &StageSkillsFile, sim_version: u32) -> Result<(), Error> {
    let mut issues = Vec::new();
    stage_skills(&file.stage_skills, &mut Issues(&mut issues, sim_version));
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}

/// Collects one issue per problem, at its path.
struct Issues<'a>(&'a mut Vec<Issue>, u32);

impl Issues<'_> {
    fn add(&mut self, path: String, message: String) {
        self.0.push(Issue { path, message });
    }

    /// `value` must be in `lo..=hi`.
    fn within(&mut self, path: String, value: i64, lo: i64, hi: i64) {
        if value < lo || value > hi {
            self.add(
                path,
                format!("must be between {lo} and {hi} (found {value})"),
            );
        }
    }

    /// `id` must be a registered ID of `kind` that this version has.
    fn registered(&mut self, path: String, id: &ContentId, kind: Kind) {
        if !registry::is_well_formed(&id.0) {
            self.add(
                path,
                "not a valid content ID (lowercase snake_case, at most 32 characters)".into(),
            );
            return;
        }
        match registry::find(registry::ENTRIES, kind, &id.0) {
            Some(e) if e.introduced > self.1 => {
                self.add(path, "newer than the pack's sim_version".into())
            }
            Some(_) => {}
            None => self.add(path, format!("no such {kind:?} ID in the registry")),
        }
    }
}

/// The skills' checks: IDs, looks, levels, effects and every number within its bound.
fn stage_skills(skills: &[StageSkill], out: &mut Issues) {
    if skills.len() > MAX_STAGE_SKILLS {
        out.add(
            "$.stage_skills".into(),
            format!("at most {MAX_STAGE_SKILLS} skills (found {})", skills.len()),
        );
        return;
    }
    for (i, skill) in skills.iter().enumerate() {
        let at = |field: &str| format!("$.stage_skills[{i}].{field}");
        if !registry::is_well_formed(&skill.id.0) {
            out.add(
                at("id"),
                "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
            );
        } else if skills[..i].iter().any(|k| k.id == skill.id) {
            out.add(at("id"), "the same ID as an earlier skill".into());
        }
        out.registered(at("icon"), &skill.icon, Kind::Asset);
        out.within(
            at("ahead"),
            i64::from(skill.ahead.0),
            0,
            i64::from(MAX_SKILL_AHEAD),
        );
        if skill.levels.is_empty() || skill.levels.len() > MAX_SKILL_LEVELS {
            out.add(
                at("levels"),
                format!(
                    "1 to {MAX_SKILL_LEVELS} levels (found {})",
                    skill.levels.len()
                ),
            );
            continue;
        }
        for (j, level) in skill.levels.iter().enumerate() {
            let at = |field: &str| format!("$.stage_skills[{i}].levels[{j}].{field}");
            out.within(
                at("cooldown_ticks"),
                i64::from(level.cooldown_ticks),
                1,
                i64::from(MAX_SKILL_COOLDOWN_TICKS),
            );
            if level.effects.is_empty() || level.effects.len() > MAX_SKILL_EFFECTS {
                out.add(
                    at("effects"),
                    format!(
                        "1 to {MAX_SKILL_EFFECTS} effects (found {})",
                        level.effects.len()
                    ),
                );
                continue;
            }
            for (k, effect) in level.effects.iter().enumerate() {
                let at = format!("$.stage_skills[{i}].levels[{j}].effects[{k}]");
                if level.effects[..k]
                    .iter()
                    .any(|e| e.block() == effect.block())
                {
                    out.add(
                        at,
                        format!("a second `{}`: at most one of each", effect.block()),
                    );
                    continue;
                }
                skill_effect(out, &at, effect);
            }
        }
    }
}

/// One effect, at `at`.
fn skill_effect(out: &mut Issues, at: &str, effect: &SkillEffect) {
    let e = |field: &str| format!("{at}.{field}");
    let radius = |out: &mut Issues, r: Fx| {
        out.within(e("radius"), i64::from(r.0), 0, i64::from(MAX_SKILL_RADIUS))
    };
    let duration = |out: &mut Issues, d: u32| {
        out.within(
            e("duration_ticks"),
            i64::from(d),
            1,
            i64::from(MAX_SKILL_DURATION_TICKS),
        )
    };
    let status = |out: &mut Issues, status: &ContentId, pct: u16| {
        out.registered(e("status"), status, Kind::Status);
        out.within(e("value_pct"), i64::from(pct), 1, i64::from(MAX_SKILL_PCT));
    };
    match effect {
        SkillEffect::Pull {
            radius: r,
            strength,
            duration_ticks,
        } => {
            radius(out, *r);
            out.within(e("strength"), i64::from(strength.0), 0, i64::from(MAX_PULL));
            duration(out, *duration_ticks);
        }
        SkillEffect::SelfStatus {
            status: id,
            value_pct,
            duration_ticks,
        } => {
            status(out, id, *value_pct);
            duration(out, *duration_ticks);
        }
        SkillEffect::AreaStatus {
            status: id,
            radius: r,
            value_pct,
            duration_ticks,
        } => {
            status(out, id, *value_pct);
            radius(out, *r);
            duration(out, *duration_ticks);
        }
        SkillEffect::Attack { attack } => {
            if let Err(found) = crate::validate::validate_attack_ref(attack, registry::ENTRIES) {
                // The attack's paths start at `$`.
                for f in found {
                    out.add(format!("{}{}", e("attack"), &f.path[1..]), f.message);
                }
            }
        }
        SkillEffect::Shield {
            amount,
            duration_ticks,
        } => {
            out.within(e("amount"), i64::from(*amount), 1, i64::from(MAX_SHIELD));
            duration(out, *duration_ticks);
        }
    }
}
