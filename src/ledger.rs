//! Run ledger (ADR-BRDG-6): counts kills, bestiary bits, picks and skill uses
//! inside the simulation from domain events and state observations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::attack::DamageType;
use crate::event::DomainEvent;
use crate::id::{ContentId, EntityId};
use crate::kinds::KindTable;
use crate::rewards::Rarity;
use crate::snapshot::EntityKind;

/// What an enemy or boss on the map is: its entity ID and sprite content ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub id: EntityId,
    pub kind: ContentId,
}

/// A star in the run's build (core#NS3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarPick {
    pub rarity: Rarity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A run's ledger of counts and bestiary encounters, verified with the replay (ADR-BRDG-6).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunLedger {
    /// Kill statistics: `kills.<weapon>.<class>` and `slain.<kind_id>`.
    pub kills: Vec<(String, u32)>,
    /// Bestiary bits per enemy kind ID: bit 0 encountered, bits 1..=10 for `DamageType::ALL`.
    pub bestiary: Vec<(ContentId, u16)>,
    /// The stars the main character took, in pick order: the run's build.
    pub picks: Vec<StarPick>,
    /// Stage skill uses per stage skill ID.
    pub skill_uses: Vec<(ContentId, u32)>,
    /// Every entity ID seen so far, so that its kind is met once. Not serialized.
    #[serde(skip)]
    known: BTreeSet<EntityId>,
}

impl RunLedger {
    /// Observes the enemies met and the domain events raised in a tick or choice (ADR-BRDG-6).
    pub fn observe(
        &mut self,
        kinds: &KindTable,
        weapon: &str,
        seen: &[Seen],
        events: &[DomainEvent],
    ) {
        for s in seen {
            if self.known.insert(s.id) {
                self.mark_seen(&s.kind);
            }
        }
        for event in events {
            match event {
                DomainEvent::Hit {
                    target,
                    target_kind: Some(kind),
                    damage_type,
                    ..
                } => {
                    if let Some(id) = kinds.id(*kind) {
                        if self.known.insert(*target) {
                            self.mark_seen(id);
                        }
                        if let Some(ty) = damage_type {
                            let bit = DamageType::ALL.iter().position(|t| t == ty).unwrap_or(0) + 1;
                            self.add_bestiary_bit(id, 1 << bit);
                        }
                    }
                }
                DomainEvent::Died {
                    entity,
                    kind: Some(kind),
                    class,
                    elite,
                } => {
                    let class_opt = match class {
                        EntityKind::Enemy if *elite => Some("elite"),
                        EntityKind::Enemy => Some("enemy"),
                        EntityKind::Boss => Some("boss"),
                        EntityKind::Companion | EntityKind::BossPart | EntityKind::Shrine => None,
                    };
                    if let Some(id) = kinds.id(*kind) {
                        if self.known.insert(*entity) {
                            self.mark_seen(id);
                        }
                        if let Some(class_str) = class_opt {
                            self.add_kill(format!("kills.{weapon}.{class_str}"));
                            self.add_kill(format!("slain.{}", id.0));
                        }
                    }
                }
                DomainEvent::StageSkillUsed { skill, .. } => {
                    self.add_skill_use(skill.clone());
                }
                DomainEvent::StarTaken { rarity, .. } => {
                    self.picks.push(StarPick {
                        rarity: *rarity,
                        name: None,
                    });
                }
                DomainEvent::RewardTaken {
                    star: Some(name), ..
                } => {
                    if let Some(pick) = self.picks.last_mut().filter(|p| p.name.is_none()) {
                        pick.name = Some(name.clone());
                    }
                }
                DomainEvent::RewardTaken {
                    rune: Some(rune), ..
                } => {
                    self.add_kill(format!("rune_taken.{}", rune.id()));
                }
                _ => {}
            }
        }
    }

    /// Sets bit 0 (encountered) for `kind`.
    pub fn mark_seen(&mut self, kind: &ContentId) {
        self.add_bestiary_bit(kind, 1);
    }

    /// Adds bits to the bestiary entry for `kind`, keeping `bestiary` sorted by ID.
    pub fn add_bestiary_bit(&mut self, kind: &ContentId, bit: u16) {
        match self.bestiary.binary_search_by(|(k, _)| k.0.cmp(&kind.0)) {
            Ok(idx) => self.bestiary[idx].1 |= bit,
            Err(idx) => self.bestiary.insert(idx, (kind.clone(), bit)),
        }
    }

    /// Increments a kill statistic, keeping `kills` sorted by key.
    pub fn add_kill(&mut self, key: String) {
        match self.kills.binary_search_by(|(k, _)| k.cmp(&key)) {
            Ok(idx) => self.kills[idx].1 = self.kills[idx].1.saturating_add(1),
            Err(idx) => self.kills.insert(idx, (key, 1)),
        }
    }

    /// Increments a skill use count, keeping `skill_uses` sorted by ID.
    pub fn add_skill_use(&mut self, skill: ContentId) {
        match self.skill_uses.binary_search_by(|(s, _)| s.0.cmp(&skill.0)) {
            Ok(idx) => self.skill_uses[idx].1 = self.skill_uses[idx].1.saturating_add(1),
            Err(idx) => self.skill_uses.insert(idx, (skill, 1)),
        }
    }

    /// Looks up a stat value by string ID (`kills.*`, `slain.*`, `skill_uses.*`, `rune_taken.*`,
    /// `stars_picked.*`).
    pub fn stat(&self, id: &str) -> u32 {
        if let Some((_, count)) = self.kills.iter().find(|(k, _)| k == id) {
            return *count;
        }
        if let Some(skill) = id.strip_prefix("skill_uses.") {
            if let Some((_, count)) = self.skill_uses.iter().find(|(k, _)| k.0 == skill) {
                return *count;
            }
        }
        if let Some(rarity) = id.strip_prefix("stars_picked.") {
            return self
                .picks
                .iter()
                .filter(|p| rarity_str(p.rarity) == rarity)
                .count() as u32;
        }
        0
    }

    /// Looks up bestiary bits for an enemy kind ID.
    pub fn bits(&self, kind: &str) -> Option<u16> {
        self.bestiary
            .iter()
            .find(|(k, _)| k.0 == kind)
            .map(|(_, b)| *b)
    }
}

pub(crate) fn rarity_str(rarity: Rarity) -> &'static str {
    match rarity {
        Rarity::Common => "common",
        Rarity::Uncommon => "uncommon",
        Rarity::Rare => "rare",
        Rarity::Epic => "epic",
        Rarity::Legendary => "legendary",
        Rarity::Supernova => "supernova",
    }
}
