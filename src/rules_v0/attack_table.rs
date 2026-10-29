//! The run's attacks, compiled once when it starts: every attack reference the run
//! can reach is expanded through `presets::phases`, with the run's own attack presets, into an `AttackDef` of `World::attacks`, and
//! the nodes of each phase into `World::emitters`, so an attack that starts keeps an index
//! instead of a copy of its phases. An attack is found again by its reference (`AttackKeys`),
//! which compares in place and allocates nothing.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::attack::{AttackPhase, AttackRef};
use crate::attack_presets::AttackPresets;
use crate::behaviour::{Action, Rule, Skill};
use crate::id::ContentId;
use crate::presets;
use crate::run_content::RunContent;
use crate::rune::Rune;
use crate::stage::Step as BossStep;
use crate::stage_skill::SkillEffect;
use crate::validate::Issue;
use crate::weapon::{Branches, Step};

use super::world::{AttackDef, AttackIndex, ProgramIndex, RangedPool, World};

/// What orders the attacks of a run: a preset by its ID and arguments, an inline attack by its
/// number of phases and their total ticks. Two different attacks may share a key (inline ones
/// do); the entry's `source` tells them apart.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Preset(String, Vec<i32>),
    Inline(usize, u64),
}

impl Key {
    fn of(attack: &AttackRef) -> Key {
        match attack {
            AttackRef::Preset { id, args } => Key::Preset(id.as_str().to_owned(), args.clone()),
            AttackRef::Inline(phases) => Key::Inline(phases.len(), total_ticks(phases)),
        }
    }

    /// The order of `self` against the key `attack` would have, without making it.
    fn cmp_attack(&self, attack: &AttackRef) -> Ordering {
        match (self, attack) {
            (
                Key::Preset(id, args),
                AttackRef::Preset {
                    id: other,
                    args: other_args,
                },
            ) => id
                .as_str()
                .cmp(other.as_str())
                .then_with(|| args.as_slice().cmp(other_args.as_slice())),
            (Key::Preset(..), AttackRef::Inline(_)) => Ordering::Less,
            (Key::Inline(..), AttackRef::Preset { .. }) => Ordering::Greater,
            (Key::Inline(n, ticks), AttackRef::Inline(phases)) => n
                .cmp(&phases.len())
                .then_with(|| ticks.cmp(&total_ticks(phases))),
        }
    }
}

fn total_ticks(phases: &[AttackPhase]) -> u64 {
    phases.iter().map(|p| u64::from(p.ticks)).sum()
}

#[derive(Clone, Debug)]
struct Entry {
    key: Key,
    source: AttackRef,
    at: AttackIndex,
}

/// The attacks of a run by the reference that names them, sorted by key like `AgentDefs::keys`.
#[derive(Clone, Debug, Default)]
pub(super) struct AttackKeys {
    entries: Vec<Entry>,
}

impl AttackKeys {
    /// The first entry that may hold `attack`.
    fn first(&self, attack: &AttackRef) -> usize {
        self.entries
            .partition_point(|e| e.key.cmp_attack(attack) == Ordering::Less)
    }

    pub fn find(&self, attack: &AttackRef) -> Option<AttackIndex> {
        self.entries[self.first(attack)..]
            .iter()
            .take_while(|e| e.key.cmp_attack(attack) == Ordering::Equal)
            .find(|e| e.source == *attack)
            .map(|e| e.at)
    }

    /// The index of `attack`, compiling it into `attacks` and `emitters` when it is new.
    pub fn compile(
        &mut self,
        attacks: &mut Vec<AttackDef>,
        emitters: &mut Vec<Vec<crate::attack::Emitter>>,
        presets: &AttackPresets,
        attack: &AttackRef,
    ) -> Result<AttackIndex, Issue> {
        if let Some(at) = self.find(attack) {
            return Ok(at);
        }
        let at = u16::try_from(attacks.len()).map_err(|_| Issue {
            path: "$".into(),
            message: format!("a run brings more than {} attacks", u16::MAX),
        })?;
        // An attack the registry does not define, or with no phases, starts nothing.
        let phases = presets::phases(attack, presets)
            .filter(|p| !p.is_empty())
            .map_or_else(Vec::new, |p| p.to_vec());
        let mut programs = Vec::with_capacity(phases.len());
        for phase in &phases {
            let program = u32::try_from(emitters.len()).map_err(|_| Issue {
                path: "$".into(),
                message: "a run brings too many attack programs".into(),
            })?;
            emitters.push(phase.spawn.clone());
            programs.push(ProgramIndex(program));
        }
        let preset: Option<ContentId> = match attack {
            AttackRef::Preset { id, .. } => Some(id.clone()),
            AttackRef::Inline(_) => None,
        };
        let pool = RangedPool::of(&phases);
        attacks.push(AttackDef {
            phases,
            preset_id: preset.as_ref().map(|p| Arc::from(p.as_str())),
            preset,
            programs,
            pool,
        });
        let at = AttackIndex(at);
        let first = self.first(attack);
        let after = self.entries[first..]
            .iter()
            .take_while(|e| e.key.cmp_attack(attack) == Ordering::Equal)
            .count();
        self.entries.insert(
            first + after,
            Entry {
                key: Key::of(attack),
                source: attack.clone(),
                at,
            },
        );
        Ok(at)
    }
}

/// Compiles every attack `world`'s run can start: its agents' and bosses', the weapon's, the
/// orbits', the stage skills' and the on-hit runes'.
pub(super) fn compile_run(world: &mut World, content: &RunContent) -> Result<(), Issue> {
    let mut refs: Vec<AttackRef> = Vec::new();
    for agent in world.defs.agents() {
        from_rules(&agent.rules, &mut refs);
    }
    for def in &world.bosses {
        for phase in &def.boss.phases {
            from_rules(&phase.rules, &mut refs);
            for step in &phase.timeline {
                if let BossStep::Attack { attack, .. } = step {
                    refs.push(attack.clone());
                }
            }
        }
    }
    if let Some(weapon) = world.weapon() {
        branches(&weapon.combo, &mut refs);
        branches(&weapon.dash, &mut refs);
    }
    for orbit in content.rewards().orbits() {
        refs.push(orbit.attack.clone());
    }
    for skill in content.stage_skills().skills() {
        for level in &skill.levels {
            for effect in &level.effects {
                if let SkillEffect::Attack { attack } = effect {
                    refs.push(attack.clone());
                }
            }
        }
    }
    for attack in &refs {
        world.compile_attack(attack)?;
    }
    for rune in Rune::ALL {
        let at = match content.runes().on_hit(rune) {
            Some(entry) => Some(world.compile_attack(&AttackRef::Inline(entry.attack.clone()))?),
            None => None,
        };
        world.rune_attacks.push(at);
    }
    Ok(())
}

fn from_rules(list: &[Rule], out: &mut Vec<AttackRef>) {
    for rule in list {
        match &rule.action {
            Action::Attack { attack, .. } => out.push(attack.clone()),
            Action::Cast {
                skill: Skill::Shot { pattern, .. },
            } => out.push(pattern.clone()),
            Action::Summon { agent, .. } => from_rules(&agent.rules, out),
            _ => {}
        }
    }
}

fn branches(branches: &Branches, out: &mut Vec<AttackRef>) {
    for step in [&branches.main, &branches.smash].into_iter().flatten() {
        combo_step(step, out);
    }
}

fn combo_step(step: &Step, out: &mut Vec<AttackRef>) {
    out.push(step.attack.clone());
    if let Some(charge) = &step.charge {
        for level in &charge.levels {
            out.push(level.attack.clone());
            out.extend(level.perfect_attack.clone());
        }
    }
    if let Some(hold) = &step.hold {
        out.push(hold.loop_attack.clone());
        out.push(hold.release.clone());
    }
    branches(&step.next, out);
}
