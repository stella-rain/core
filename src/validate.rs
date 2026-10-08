//! The stage validator (ADR-020): a byte-size limit before parsing, strict parsing, registry
//! lookups with the introducing `sim_version`, and the static budgets of ADR-020 and ADR-036.
//! The app and `stage-verify` call the same code, and an invalid stage is not playable (fail
//! closed).
//!
//! Errors name where the problem is (`$.boss.phases[1].timeline[3]`) and what is wrong. They
//! never repeat text from the file, so a hostile stage cannot put its own words in front of a
//! player.
//!
//! Budgets here are tuning values; their existence is the decision. Dynamic budgets (bullets
//! alive, enemies alive) are the simulation's runtime caps, and so is any count that an
//! expression computes: the validator checks constants, the engine clamps the rest.

use std::fmt;

use serde::Deserialize;

use crate::attack::{AttackRef, Direction, Emitter, Expr};
use crate::behaviour::{Action, Agent, Condition, Movement, Rule, Selector, Skill};
use crate::id::ContentId;
use crate::registry::{self, Entry, Kind};
use crate::stage::{Boss, BossPart, Phase, Player, SkillSlot, Stage, Step, Transition, Wave};
use crate::version::{SCHEMA_VERSION, SIM_VERSION};

/// Limits a stage must keep. The first block is ADR-020, the second ADR-036, the rest the
/// loadout and numeric ranges that ADR-012 and ADR-019 ask for.
pub mod limits {
    pub const MAX_STAGE_BYTES: usize = 256 * 1024;
    /// Five minutes at 60 ticks per second.
    pub const MAX_TICKS: u32 = 5 * 60 * 60;
    pub const MAX_WAVES: usize = 200;
    pub const MAX_COMPANIONS: usize = 4;
    pub const MAX_PHASES: usize = 8;
    pub const MAX_TITLE_CHARS: usize = 60;
    pub const MAX_DESCRIPTION_CHARS: usize = 500;
    /// Stage IDs are slugs (ADR-010).
    pub const MAX_STAGE_ID_CHARS: usize = 40;

    pub const MAX_RULES: usize = 8;
    pub const MAX_TIMELINE_STEPS: usize = 64;
    pub const MAX_PATTERN_DEPTH: u32 = 4;
    pub const MAX_PATTERN_NODES: u32 = 64;
    pub const MAX_EXPR_NODES: u32 = 16;
    pub const MAX_REPEAT: i32 = 100;

    pub const MAX_SKILL_SLOTS: usize = 3;
    pub const MAX_BOSS_PARTS: usize = 8;
    pub const MAX_TRANSITIONS: usize = 4;
    pub const MAX_PRESET_ARGS: usize = 8;
    pub const MAX_WAVE_COUNT: u16 = 100;
    pub const MAX_SUMMON_COUNT: u8 = 8;

    pub const MAX_PLAYER_HP: u32 = 99;
    pub const MAX_PLAYER_ATK: u32 = 999;
    pub const MAX_AGENT_HP: u32 = 100_000;
    pub const MAX_BOSS_HP: u32 = 1_000_000;
    pub const MAX_DAMAGE: u32 = 10_000;
    pub const MAX_BUFF_PCT: u16 = 500;
    pub const MAX_AGGRO: u32 = 1_000_000;
    /// A number in an expression, an argument or a velocity-like field.
    pub const MAX_NUMBER: i32 = 1_000_000;
    /// A length in 1/256 pixels: 1,024 pixels.
    pub const MAX_LEN: i32 = 262_144;
    /// A hurt circle radius in 1/256 pixels: 256 pixels.
    pub const MAX_RADIUS: i32 = 65_536;
    /// The playfield in 1/256 pixels (360 by 640 pixels), and how far outside it a point may be.
    pub const FIELD_W: i32 = 92_160;
    pub const FIELD_H: i32 = 163_840;
    pub const FIELD_MARGIN: i32 = 16_384;

    /// Problems reported before the rest are left out.
    pub const MAX_ISSUES: usize = 32;
}

use limits::*;

/// One problem and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// For example `$.boss.phases[1].timeline[3]`; `$` is the file.
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Over `MAX_STAGE_BYTES`; the file is not parsed.
    TooLarge {
        size: usize,
        limit: usize,
    },
    /// Not JSON, or not the stage format (unknown or missing fields, wrong types).
    Parse {
        line: usize,
        column: usize,
    },
    /// Written by a newer game: listed greyed out as "update the app" (ADR-017).
    UnsupportedVersion {
        schema_version: u32,
        sim_version: u32,
    },
    Invalid(Vec<Issue>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLarge { size, limit } => {
                write!(f, "the stage file is {size} bytes; the limit is {limit}")
            }
            Error::Parse { line, column } => {
                write!(f, "not a valid stage file (line {line}, column {column})")
            }
            Error::UnsupportedVersion {
                schema_version,
                sim_version,
            } => write!(
                f,
                "needs a newer version of the game (schema_version {schema_version}, \
                 sim_version {sim_version})"
            ),
            Error::Invalid(issues) => {
                for (i, issue) in issues.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}: {}", issue.path, issue.message)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

/// Checks the size, parses strictly and validates: the one call for a stage file's bytes.
pub fn parse_and_validate(bytes: &[u8]) -> Result<Stage, Error> {
    if bytes.len() > MAX_STAGE_BYTES {
        return Err(Error::TooLarge {
            size: bytes.len(),
            limit: MAX_STAGE_BYTES,
        });
    }
    // Versions first: a file from a newer game may have fields this one rejects.
    #[derive(Deserialize)]
    struct Versions {
        schema_version: u32,
        sim_version: u32,
    }
    let parse_error = |e: serde_json::Error| Error::Parse {
        line: e.line(),
        column: e.column(),
    };
    let versions: Versions = serde_json::from_slice(bytes).map_err(parse_error)?;
    check_versions(versions.schema_version, versions.sim_version)?;
    let stage: Stage = serde_json::from_slice(bytes).map_err(parse_error)?;
    validate(&stage)?;
    Ok(stage)
}

fn check_versions(schema_version: u32, sim_version: u32) -> Result<(), Error> {
    if schema_version == SCHEMA_VERSION && sim_version == SIM_VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedVersion {
            schema_version,
            sim_version,
        })
    }
}

/// Validates a parsed stage against the registry.
pub fn validate(stage: &Stage) -> Result<(), Error> {
    validate_with(stage, registry::ENTRIES)
}

/// As `validate`, against a given registry (tests use one with newer and deprecated IDs).
pub fn validate_with(stage: &Stage, registry: &[Entry]) -> Result<(), Error> {
    check_versions(stage.schema_version, stage.sim_version)?;
    let mut ctx = Ctx {
        registry,
        sim_version: stage.sim_version,
        parts: stage
            .boss
            .iter()
            .flat_map(|b| b.parts.iter().map(|p| p.id.0.as_str()))
            .collect(),
        path: Vec::new(),
        issues: Vec::new(),
        left_out: false,
    };
    ctx.stage(stage);
    if ctx.left_out {
        ctx.path.clear();
        ctx.issues.push(Issue {
            path: "$".into(),
            message: "more problems were found; fix these first".into(),
        });
    }
    if ctx.issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(ctx.issues))
    }
}

enum Seg {
    Key(&'static str),
    Index(usize),
}

struct Ctx<'a> {
    registry: &'a [Entry],
    sim_version: u32,
    /// Boss part IDs, which conditions and selectors may name.
    parts: Vec<&'a str>,
    path: Vec<Seg>,
    issues: Vec<Issue>,
    left_out: bool,
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

    /// A content ID of `kind` that exists and is available to this stage's `sim_version`.
    fn content_id(&mut self, key: &'static str, id: &ContentId, kind: Kind) -> Option<Entry> {
        self.key(key, |c| {
            if !registry::is_well_formed(&id.0) {
                c.fail("not a valid content ID (lowercase snake_case, at most 32 characters)");
                return None;
            }
            match registry::find(c.registry, kind, &id.0) {
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
                i64::from(FIELD_W) + i64::from(FIELD_MARGIN),
            );
            c.within(
                "y",
                i64::from(p.y.0),
                -i64::from(FIELD_MARGIN),
                i64::from(FIELD_H) + i64::from(FIELD_MARGIN),
            );
        });
    }

    fn ticks(&mut self, key: &'static str, ticks: u32, lo: u32) {
        self.within(key, i64::from(ticks), i64::from(lo), i64::from(MAX_TICKS));
    }

    fn stage(&mut self, s: &Stage) {
        self.key("id", |c| {
            let ok = (1..=MAX_STAGE_ID_CHARS).contains(&s.id.len())
                && s.id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !s.id.starts_with('-')
                && !s.id.ends_with('-');
            if !ok {
                c.fail(format!(
                    "a stage ID is 1 to {MAX_STAGE_ID_CHARS} characters of a-z, 0-9 and -, \
                     not starting or ending with -"
                ));
            }
        });
        let title = s.title.chars().count();
        if title == 0 {
            self.key("title", |c| c.fail("must not be empty"));
        } else {
            self.within("title", title as i64, 1, MAX_TITLE_CHARS as i64);
        }
        self.within(
            "description",
            s.description.chars().count() as i64,
            0,
            MAX_DESCRIPTION_CHARS as i64,
        );
        self.ticks("length_ticks", s.length_ticks, 1);
        self.key("player", |c| c.player(&s.player));
        if self.at_most(
            "companions",
            s.companions.len(),
            MAX_COMPANIONS,
            "companions",
        ) {
            self.key("companions", |c| {
                for (i, a) in s.companions.iter().enumerate() {
                    c.idx(i, |c| c.agent(a, true));
                }
            });
        }
        if self.at_most("waves", s.waves.len(), MAX_WAVES, "waves") {
            self.key("waves", |c| {
                for (i, w) in s.waves.iter().enumerate() {
                    c.idx(i, |c| c.wave(w, s.length_ticks));
                }
            });
        }
        if let Some(boss) = &s.boss {
            self.key("boss", |c| c.boss(boss));
        }
    }

    fn player(&mut self, p: &Player) {
        self.content_id("base", &p.base, Kind::Asset);
        self.within("hp", i64::from(p.hp), 1, i64::from(MAX_PLAYER_HP));
        self.within("atk", i64::from(p.atk), 1, i64::from(MAX_PLAYER_ATK));
        self.key("shot", |c| c.attack_ref(&p.shot));
        if self.at_most("skills", p.skills.len(), MAX_SKILL_SLOTS, "skill slots") {
            self.key("skills", |c| {
                let mut used = [false; MAX_SKILL_SLOTS];
                for (i, slot) in p.skills.iter().enumerate() {
                    c.idx(i, |c| c.skill_slot(slot, &mut used));
                }
            });
        }
    }

    fn skill_slot(&mut self, s: &SkillSlot, used: &mut [bool; MAX_SKILL_SLOTS]) {
        self.within("slot", i64::from(s.slot), 1, MAX_SKILL_SLOTS as i64);
        if let Some(seen) = usize::from(s.slot)
            .checked_sub(1)
            .and_then(|i| used.get_mut(i))
        {
            if *seen {
                self.key("slot", |c| c.fail("two skills use this slot"));
            }
            *seen = true;
        }
        self.key("target", |c| c.selector(&s.target));
        self.key("skill", |c| c.skill(&s.skill));
        self.ticks("cooldown_ticks", s.cooldown_ticks, 0);
        self.charges(s.charges);
    }

    fn charges(&mut self, charges: Option<u16>) {
        if let Some(n) = charges {
            self.within("charges", i64::from(n), 1, i64::from(u16::MAX));
        }
    }

    fn agent(&mut self, a: &Agent, summon_ok: bool) {
        self.content_id("base", &a.base, Kind::Asset);
        if let Some(p) = &a.palette {
            self.content_id("palette", p, Kind::Palette);
        }
        self.within("hp", i64::from(a.hp), 1, i64::from(MAX_AGENT_HP));
        self.within("radius", i64::from(a.radius.0), 1, i64::from(MAX_RADIUS));
        self.key("movement", |c| c.movement(&a.movement));
        self.rules("rules", &a.rules, summon_ok);
    }

    fn movement(&mut self, m: &Movement) {
        match m {
            Movement::Orbit { radius } => {
                self.within("radius", i64::from(radius.0), 1, i64::from(MAX_LEN))
            }
            Movement::Follow { distance } => {
                self.within("distance", i64::from(distance.0), 0, i64::from(MAX_LEN))
            }
            Movement::Hold { at } => self.point("at", at),
            Movement::Straight { vx, vy } => {
                let max = i64::from(MAX_LEN);
                self.within("vx", i64::from(vx.0), -max, max);
                self.within("vy", i64::from(vy.0), -max, max);
            }
        }
    }

    fn rules(&mut self, key: &'static str, rules: &[Rule], summon_ok: bool) {
        if self.at_most(key, rules.len(), MAX_RULES, "rules") {
            self.key(key, |c| {
                for (i, r) in rules.iter().enumerate() {
                    c.idx(i, |c| c.rule(r, summon_ok));
                }
            });
        }
    }

    fn rule(&mut self, r: &Rule, summon_ok: bool) {
        self.key("when", |c| c.condition(&r.when));
        self.key("target", |c| c.selector(&r.target));
        self.key("do", |c| c.action(&r.action, summon_ok));
        self.ticks("cooldown_ticks", r.cooldown_ticks, 0);
        self.charges(r.charges);
    }

    fn condition(&mut self, cond: &Condition) {
        match cond {
            Condition::Always | Condition::PlayerFiring => {}
            Condition::HpBelow { who, pct } => {
                self.key("who", |c| c.selector(who));
                self.within("pct", i64::from(*pct), 0, 100);
            }
            Condition::HasStatus { who, status } => {
                self.key("who", |c| c.selector(who));
                self.content_id("status", status, Kind::Status);
            }
            Condition::AggroAbove { who, value } => {
                self.key("who", |c| c.selector(who));
                self.within("value", i64::from(*value), 0, i64::from(MAX_AGGRO));
            }
            Condition::PartBroken { part } => self.part("part", part),
            Condition::TimeAbove { ticks } => self.ticks("ticks", *ticks, 0),
        }
    }

    fn selector(&mut self, s: &Selector) {
        if let Selector::Part(id) = s {
            self.part("part", id);
        }
    }

    /// A boss part the stage defines.
    fn part(&mut self, key: &'static str, id: &ContentId) {
        if !self.parts.contains(&id.0.as_str()) {
            self.key(key, |c| c.fail("the boss has no part with this ID"));
        }
    }

    fn action(&mut self, a: &Action, summon_ok: bool) {
        match a {
            Action::Cast { skill } => self.key("skill", |c| c.skill(skill)),
            Action::Attack { attack } => self.key("attack", |c| c.attack_ref(attack)),
            Action::Summon { agent, count } => {
                if !summon_ok {
                    self.fail("a summoned agent cannot summon");
                    return;
                }
                self.within("count", i64::from(*count), 1, i64::from(MAX_SUMMON_COUNT));
                self.key("agent", |c| c.agent(agent, false));
            }
            Action::MoveTo { to, ticks } => {
                self.point("to", to);
                self.ticks("ticks", *ticks, 1);
            }
            Action::Telegraph { ticks } => self.ticks("ticks", *ticks, 1),
        }
    }

    fn skill(&mut self, s: &Skill) {
        match s {
            Skill::Shot { pattern, damage } => {
                self.key("pattern", |c| c.attack_ref(pattern));
                self.within("damage", i64::from(*damage), 1, i64::from(MAX_DAMAGE));
            }
            Skill::AtkUp {
                value_pct,
                duration_ticks,
            }
            | Skill::Vulnerable {
                value_pct,
                duration_ticks,
            } => {
                self.within(
                    "value_pct",
                    i64::from(*value_pct),
                    1,
                    i64::from(MAX_BUFF_PCT),
                );
                self.ticks("duration_ticks", *duration_ticks, 1);
            }
            Skill::Slow {
                value_pct,
                duration_ticks,
            } => {
                self.within("value_pct", i64::from(*value_pct), 1, 100);
                self.ticks("duration_ticks", *duration_ticks, 1);
            }
            Skill::Heal { amount } => {
                self.within("amount", i64::from(*amount), 1, i64::from(MAX_DAMAGE))
            }
        }
    }

    fn wave(&mut self, w: &Wave, length_ticks: u32) {
        if w.at_tick >= length_ticks {
            self.key("at_tick", |c| c.fail("must be before the stage ends"));
        }
        self.key("enemy", |c| c.agent(&w.enemy, true));
        self.point("spawn", &w.spawn);
        self.within("count", i64::from(w.count), 1, i64::from(MAX_WAVE_COUNT));
        self.ticks("every_ticks", w.every_ticks, u32::from(w.count > 1));
    }

    fn boss(&mut self, b: &Boss) {
        self.content_id("base", &b.base, Kind::Asset);
        if let Some(p) = &b.palette {
            self.content_id("palette", p, Kind::Palette);
        }
        self.within("hp", i64::from(b.hp), 1, i64::from(MAX_BOSS_HP));
        self.within("radius", i64::from(b.radius.0), 1, i64::from(MAX_RADIUS));
        self.point("spawn", &b.spawn);
        if self.at_most("parts", b.parts.len(), MAX_BOSS_PARTS, "boss parts") {
            self.key("parts", |c| {
                for (i, p) in b.parts.iter().enumerate() {
                    let repeated = b.parts[..i].iter().any(|q| q.id == p.id);
                    c.idx(i, |c| c.boss_part(p, repeated));
                }
            });
        }
        if b.phases.is_empty() {
            self.key("phases", |c| c.fail("the boss needs at least one phase"));
        } else if self.at_most("phases", b.phases.len(), MAX_PHASES, "phases") {
            self.key("phases", |c| {
                for (i, p) in b.phases.iter().enumerate() {
                    c.idx(i, |c| c.phase(p, i + 1 == b.phases.len()));
                }
            });
        }
    }

    fn boss_part(&mut self, p: &BossPart, repeated: bool) {
        self.key("id", |c| {
            if !registry::is_well_formed(&p.id.0) {
                c.fail("not a valid part ID (lowercase snake_case, at most 32 characters)");
            } else if repeated {
                c.fail("two parts have this ID");
            }
        });
        self.content_id("asset", &p.asset, Kind::Asset);
        self.within("hp", i64::from(p.hp), 1, i64::from(MAX_AGENT_HP));
        self.within("radius", i64::from(p.radius.0), 1, i64::from(MAX_RADIUS));
        self.key("offset", |c| {
            let max = i64::from(MAX_LEN);
            c.within("x", i64::from(p.offset.x.0), -max, max);
            c.within("y", i64::from(p.offset.y.0), -max, max);
        });
    }

    fn phase(&mut self, p: &Phase, last: bool) {
        match (&p.until, last) {
            (Some(cond), false) => self.key("until", |c| c.condition(cond)),
            (None, false) => self.key("until", |c| c.fail("every phase but the last needs one")),
            (Some(_), true) => self.key("until", |c| {
                c.fail("the last phase lasts until the boss dies")
            }),
            (None, true) => {}
        }
        if self.at_most(
            "transition",
            p.transition.len(),
            MAX_TRANSITIONS,
            "transitions",
        ) {
            self.key("transition", |c| {
                for (i, t) in p.transition.iter().enumerate() {
                    if let Transition::InvulnerableTicks { ticks } = t {
                        c.idx(i, |c| c.ticks("ticks", *ticks, 1));
                    }
                }
            });
        }
        if self.at_most(
            "timeline",
            p.timeline.len(),
            MAX_TIMELINE_STEPS,
            "timeline steps",
        ) {
            self.key("timeline", |c| {
                for (i, s) in p.timeline.iter().enumerate() {
                    c.idx(i, |c| c.step(s, i + 1 == p.timeline.len()));
                }
            });
        }
        self.rules("rules", &p.rules, true);
    }

    fn step(&mut self, s: &Step, last: bool) {
        match s {
            Step::MoveTo { to, ticks } => {
                self.point("to", to);
                self.ticks("ticks", *ticks, 1);
            }
            Step::Wait { ticks } | Step::Telegraph { ticks } => self.ticks("ticks", *ticks, 1),
            Step::Attack { attack } => self.key("attack", |c| c.attack_ref(attack)),
            Step::Loop => {
                if !last {
                    self.fail("`loop` must be the last step; later steps would never run");
                }
            }
        }
    }

    fn attack_ref(&mut self, a: &AttackRef) {
        match a {
            AttackRef::Preset { id, args } => {
                self.key("preset", |c| {
                    let entry = c.content_id("id", id, Kind::Preset);
                    if !c.at_most("args", args.len(), MAX_PRESET_ARGS, "arguments") {
                        return;
                    }
                    if let Some(e) = entry {
                        if usize::from(e.args) != args.len() {
                            c.key("args", |c| {
                                c.fail(format!(
                                    "this preset takes {} arguments (found {})",
                                    e.args,
                                    args.len()
                                ))
                            });
                        }
                    }
                    c.key("args", |c| {
                        for (i, v) in args.iter().enumerate() {
                            c.idx(i, |c| c.number(*v));
                        }
                    });
                });
            }
            AttackRef::Inline(nodes) => {
                self.key("inline", |c| {
                    let mut count = 0;
                    c.emitters(nodes, 1, &mut count);
                });
            }
        }
    }

    fn number(&mut self, v: i32) {
        self.within(
            "value",
            i64::from(v),
            -i64::from(MAX_NUMBER),
            i64::from(MAX_NUMBER),
        );
    }

    /// Checks a list of emitters at `depth`, counting nodes in `count`. Returns false once a
    /// limit is hit, so that the rest of the tree is not walked.
    fn emitters(&mut self, list: &[Emitter], depth: u32, count: &mut u32) -> bool {
        for (i, e) in list.iter().enumerate() {
            *count += 1;
            if depth > MAX_PATTERN_DEPTH {
                self.idx(i, |c| {
                    c.fail(format!(
                        "a pattern is at most {MAX_PATTERN_DEPTH} levels deep"
                    ))
                });
                return false;
            }
            if *count > MAX_PATTERN_NODES {
                self.idx(i, |c| {
                    c.fail(format!("a pattern has at most {MAX_PATTERN_NODES} nodes"))
                });
                return false;
            }
            if !self.idx(i, |c| c.emitter(e, depth, count)) {
                return false;
            }
        }
        true
    }

    fn emitter(&mut self, e: &Emitter, depth: u32, count: &mut u32) -> bool {
        match e {
            Emitter::Fire {
                speed,
                direction,
                bullet,
            } => {
                self.key("speed", |c| c.expr(speed));
                self.key("direction", |c| match direction {
                    Direction::Aimed { offset } => c.key("offset", |c| c.expr(offset)),
                    Direction::Absolute { angle } | Direction::Relative { angle } => {
                        c.key("angle", |c| c.expr(angle))
                    }
                    Direction::Sequential { step } => c.key("step", |c| c.expr(step)),
                });
                if let Some(b) = bullet {
                    self.content_id("bullet", b, Kind::Bullet);
                }
                true
            }
            Emitter::Repeat {
                count: n,
                interval_ticks,
                body,
            } => {
                self.repeat_count("count", n);
                self.constant_ticks("interval_ticks", interval_ticks);
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Ring { count: n, body } => {
                self.repeat_count("count", n);
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Spread {
                count: n,
                angle,
                body,
            } => {
                self.repeat_count("count", n);
                self.key("angle", |c| c.expr(angle));
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Rotate { angle, body } => {
                self.key("angle", |c| c.expr(angle));
                self.key("body", |c| c.emitters(body, depth + 1, count))
            }
            Emitter::Wait { ticks } => {
                self.constant_ticks("ticks", ticks);
                true
            }
            Emitter::OnBullet {
                after_ticks,
                body,
                then,
            } => {
                self.constant_ticks("after_ticks", after_ticks);
                self.key("body", |c| c.emitters(body, depth + 1, count))
                    && self.key("then", |c| c.emitters(then, depth + 1, count))
            }
        }
    }

    /// A repeat, ring or spread count: a constant must be in `1..=MAX_REPEAT`; a computed one
    /// is clamped by the engine.
    fn repeat_count(&mut self, key: &'static str, e: &Expr) {
        self.key(key, |c| c.expr(e));
        if let Expr::Const(v) = e {
            self.within(key, i64::from(*v), 1, i64::from(MAX_REPEAT));
        }
    }

    /// A tick count: a constant must be in `0..=MAX_TICKS`; a computed one is clamped.
    fn constant_ticks(&mut self, key: &'static str, e: &Expr) {
        self.key(key, |c| c.expr(e));
        if let Expr::Const(v) = e {
            self.within(key, i64::from(*v), 0, i64::from(MAX_TICKS));
        }
    }

    /// An expression of at most `MAX_EXPR_NODES` nodes. Stage attacks take no arguments, so
    /// `arg` is never valid here; presets, which do, are defined in `core`.
    fn expr(&mut self, e: &Expr) {
        let mut nodes = 0;
        self.expr_node(e, 0, &mut nodes);
    }

    fn expr_node(&mut self, e: &Expr, args: u8, nodes: &mut u32) {
        *nodes += 1;
        if *nodes > MAX_EXPR_NODES {
            if *nodes == MAX_EXPR_NODES + 1 {
                self.fail(format!("an expression has at most {MAX_EXPR_NODES} nodes"));
            }
            return;
        }
        match e {
            Expr::Arg(i) => {
                if *i >= args {
                    self.fail("there is no such argument in an inline attack");
                }
            }
            Expr::LoopIndex | Expr::PhaseTime => {}
            Expr::Const(v) => self.number(*v),
            Expr::Rand(b) => self.pair("rand", b, args, nodes),
            Expr::Add(b) => self.pair("add", b, args, nodes),
            Expr::Sub(b) => self.pair("sub", b, args, nodes),
            Expr::Mul(b) => self.pair("mul", b, args, nodes),
            Expr::Div(b) => self.pair("div", b, args, nodes),
            Expr::Min(b) => self.pair("min", b, args, nodes),
            Expr::Max(b) => self.pair("max", b, args, nodes),
            Expr::Clamp(b) => self.key("clamp", |c| {
                c.idx(0, |c| c.expr_node(&b.0, args, nodes));
                c.idx(1, |c| c.expr_node(&b.1, args, nodes));
                c.idx(2, |c| c.expr_node(&b.2, args, nodes));
            }),
        }
    }

    fn pair(&mut self, name: &'static str, b: &(Expr, Expr), args: u8, nodes: &mut u32) {
        self.key(name, |c| {
            c.idx(0, |c| c.expr_node(&b.0, args, nodes));
            c.idx(1, |c| c.expr_node(&b.1, args, nodes));
        });
    }
}
