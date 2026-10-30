//! Run ledger (ADR-BRDG-6): counts kills, bestiary bits, picks and skill uses
//! inside the simulation from domain events and state observations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::attack::DamageType;
use crate::event::DomainEvent;
use crate::id::ContentId;
use crate::kinds::{KindIndex, KindTable};
use crate::rewards::{Rarity, Rewards};
use crate::savepoint::codec::{Decode, DecodeError, Encode, Reader, Writer, decode_vec};
use crate::sim::state::{AgentState, BossState};
use crate::snapshot::EntityKind;
use std::sync::Arc;

/// What an enemy or boss on the map is: its entity ID and sprite content ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub id: crate::id::EntityId,
    pub kind: ContentId,
}

/// A star in the run's build (core#NS3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarPick {
    pub rarity: Rarity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<Arc<str>>,
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
    #[serde(skip)]
    enemy_kill_idx: usize,
    #[serde(skip)]
    elite_kill_idx: usize,
    #[serde(skip)]
    boss_kill_idx: usize,
    #[serde(skip)]
    slain_indices: Vec<usize>,
}

impl RunLedger {
    /// Creates a ledger pre-allocated for an engine run so steady ticks do not allocate (DIR1).
    pub fn for_engine(kinds: &KindTable, weapon: &str) -> RunLedger {
        let mut kills = Vec::with_capacity(3 + kinds.len());
        kills.push((format!("kills.{weapon}.boss"), 0));
        kills.push((format!("kills.{weapon}.elite"), 0));
        kills.push((format!("kills.{weapon}.enemy"), 0));
        let boss_kill_idx = 0;
        let elite_kill_idx = 1;
        let enemy_kill_idx = 2;

        let mut slain_indices = Vec::with_capacity(kinds.len());
        for id in kinds.ids() {
            slain_indices.push(kills.len());
            kills.push((format!("slain.{}", id.as_str()), 0));
        }

        let bestiary = kinds.ids().iter().map(|id| (id.clone(), 0)).collect();

        RunLedger {
            kills,
            bestiary,
            picks: Vec::with_capacity(32),
            skill_uses: Vec::with_capacity(4),
            enemy_kill_idx,
            elite_kill_idx,
            boss_kill_idx,
            slain_indices,
        }
    }
    /// Observes newly met enemies and events from a stepped tick without allocating.
    pub(crate) fn observe_tick(
        &mut self,
        kinds: &KindTable,
        weapon: &str,
        enemies: &[AgentState],
        boss: Option<&BossState>,
        events: &[DomainEvent],
    ) {
        for enemy in enemies {
            if !enemy.crowded && !enemy.burst {
                self.mark_seen_index(kinds, enemy.kind);
            }
        }
        if let Some(b) = boss {
            self.mark_seen_index(kinds, b.kind);
        }
        if !events.is_empty() {
            self.observe_events(kinds, weapon, events);
        }
    }

    /// Observes choices between ticks.
    pub(crate) fn observe_choice(
        &mut self,
        kinds: &KindTable,
        weapon: &str,
        events: &[DomainEvent],
    ) {
        if !events.is_empty() {
            self.observe_events(kinds, weapon, events);
        }
    }

    /// Observes seen enemies and domain events (used by tests and adapters).
    pub fn observe(
        &mut self,
        kinds: &KindTable,
        weapon: &str,
        seen: &[Seen],
        events: &[DomainEvent],
    ) {
        for s in seen {
            self.add_bestiary_bit(&s.kind, 1);
        }
        if !events.is_empty() {
            self.observe_events(kinds, weapon, events);
        }
    }

    fn mark_seen_index(&mut self, kinds: &KindTable, kind: KindIndex) {
        let idx = usize::from(kind.0);
        if let Some(entry) = self.bestiary.get_mut(idx) {
            entry.1 |= 1;
        } else if let Some(id) = kinds.id(kind) {
            self.add_bestiary_bit(id, 1);
        }
    }

    fn observe_events(&mut self, kinds: &KindTable, weapon: &str, events: &[DomainEvent]) {
        for event in events {
            match event {
                DomainEvent::Hit {
                    target_kind: Some(kind),
                    damage_type,
                    ..
                } => {
                    let bit = match damage_type {
                        Some(ty) => {
                            let pos = DamageType::ALL.iter().position(|t| t == ty).unwrap_or(0) + 1;
                            1 | (1 << pos)
                        }
                        None => 1,
                    };
                    let idx = usize::from(kind.0);
                    if let Some(entry) = self.bestiary.get_mut(idx) {
                        entry.1 |= bit;
                    } else if let Some(id) = kinds.id(*kind) {
                        self.add_bestiary_bit(id, bit);
                    }
                }
                DomainEvent::Died {
                    kind: Some(kind),
                    class,
                    elite,
                    ..
                } => {
                    let idx = usize::from(kind.0);
                    if let Some(entry) = self.bestiary.get_mut(idx) {
                        entry.1 |= 1;
                    } else if let Some(id) = kinds.id(*kind) {
                        self.add_bestiary_bit(id, 1);
                    }

                    match class {
                        EntityKind::Enemy if *elite => {
                            if !self.slain_indices.is_empty()
                                && self.elite_kill_idx < self.kills.len()
                            {
                                self.kills[self.elite_kill_idx].1 =
                                    self.kills[self.elite_kill_idx].1.saturating_add(1);
                            } else {
                                self.add_kill(format!("kills.{weapon}.elite"));
                            }
                        }
                        EntityKind::Enemy => {
                            if !self.slain_indices.is_empty()
                                && self.enemy_kill_idx < self.kills.len()
                            {
                                self.kills[self.enemy_kill_idx].1 =
                                    self.kills[self.enemy_kill_idx].1.saturating_add(1);
                            } else {
                                self.add_kill(format!("kills.{weapon}.enemy"));
                            }
                        }
                        EntityKind::Boss => {
                            if !self.slain_indices.is_empty()
                                && self.boss_kill_idx < self.kills.len()
                            {
                                self.kills[self.boss_kill_idx].1 =
                                    self.kills[self.boss_kill_idx].1.saturating_add(1);
                            } else {
                                self.add_kill(format!("kills.{weapon}.boss"));
                            }
                        }
                        EntityKind::Companion | EntityKind::BossPart | EntityKind::Shrine => {}
                    }

                    if matches!(class, EntityKind::Enemy | EntityKind::Boss) {
                        if let Some(&slain_idx) = self.slain_indices.get(idx) {
                            if slain_idx < self.kills.len() {
                                self.kills[slain_idx].1 = self.kills[slain_idx].1.saturating_add(1);
                            }
                        } else if let Some(id) = kinds.id(*kind) {
                            self.add_kill(format!("slain.{}", id.as_str()));
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
        match self
            .bestiary
            .binary_search_by(|(k, _)| k.as_str().cmp(kind.as_str()))
        {
            Ok(idx) => self.bestiary[idx].1 |= bit,
            Err(idx) => self.bestiary.insert(idx, (kind.clone(), bit)),
        }
    }

    /// Increments a kill statistic, keeping `kills` sorted by key.
    pub fn add_kill(&mut self, key: String) {
        match self.kills.binary_search_by(|(k, _)| k.cmp(&key)) {
            Ok(idx) => self.kills[idx].1 = self.kills[idx].1.saturating_add(1),
            Err(idx) => {
                self.kills.insert(idx, (key, 1));
                if idx <= self.boss_kill_idx {
                    self.boss_kill_idx += 1;
                }
                if idx <= self.elite_kill_idx {
                    self.elite_kill_idx += 1;
                }
                if idx <= self.enemy_kill_idx {
                    self.enemy_kill_idx += 1;
                }
                for s_idx in &mut self.slain_indices {
                    if idx <= *s_idx {
                        *s_idx += 1;
                    }
                }
            }
        }
    }

    /// Increments a skill use count, keeping `skill_uses` sorted by ID.
    pub fn add_skill_use(&mut self, skill: ContentId) {
        match self
            .skill_uses
            .binary_search_by(|(s, _)| s.as_str().cmp(skill.as_str()))
        {
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
            if let Some((_, count)) = self.skill_uses.iter().find(|(k, _)| k.as_str() == skill) {
                return *count;
            }
        }
        if let Some(rarity) = id.strip_prefix("stars_picked.") {
            return self
                .picks
                .iter()
                .filter(|p| rarity_str(p.rarity) == rarity)
                .count()
                .try_into()
                .expect("a few picks");
        }
        0
    }

    /// Looks up bestiary bits for an enemy kind ID.
    pub fn bits(&self, kind: &str) -> Option<u16> {
        self.bestiary
            .iter()
            .find(|(k, _)| k.as_str() == kind)
            .and_then(|(_, b)| if *b > 0 { Some(*b) } else { None })
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

/// The run's HUD tallies, counted from the events the way the app counts them and kept outside
/// the hashed state (ADR-BRDG-6): a carried star, which emits no event, is not in them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Counted {
    pub(crate) kills: u32,
    /// The count the last `RewardTaken` of each reward carried, per reward of the run's file; 0 = never.
    pub(crate) taken: Vec<u8>,
    /// The tier the last `Constellation` of each set carried, per set; 0 = none.
    pub(crate) tiers: Vec<u8>,
    pub(crate) named: Vec<Arc<str>>,
}

impl Counted {
    /// Reserves everything a steady tick touches, so observing allocates nothing.
    pub(crate) fn new(rewards: &Rewards) -> Counted {
        Counted {
            kills: 0,
            taken: vec![0; rewards.rewards().len()],
            tiers: vec![0; rewards.constellations().sets().len()],
            named: Vec::with_capacity(32),
        }
    }

    pub(crate) fn observe(&mut self, rewards: &Rewards, events: &[DomainEvent]) {
        for event in events {
            match event {
                DomainEvent::Died {
                    class: EntityKind::Enemy,
                    ..
                } => self.kills = self.kills.saturating_add(1),
                DomainEvent::RewardTaken {
                    reward,
                    taken,
                    star,
                    ..
                } => {
                    if let Some(count) = rewards
                        .index_of(reward)
                        .and_then(|at| self.taken.get_mut(at))
                    {
                        *count = *taken;
                    }
                    if let Some(name) = star.as_ref().filter(|n| !n.is_empty()) {
                        self.named.push(name.clone());
                    }
                }
                DomainEvent::Constellation { set, tier } => {
                    let sets = rewards.constellations().sets();
                    if let Some(at) = sets.iter().position(|s| s.id == *set) {
                        if let Some(held) = self.tiers.get_mut(at) {
                            *held = *tier;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

// ---- The savepoint encoding ----

/// The most entries of the ledger's lists: one per kind and a few more for the weapon's three
/// and the rune keys (`kinds::MAX_KINDS`).
const MAX_LEDGER_ENTRIES: u32 = crate::kinds::MAX_KINDS as u32 + 64;
/// The most stars and names the run's build and tallies hold.
const MAX_PICKS: u32 = 1 << 16;
/// The most tallies of the rewards file (`rewards::MAX_REWARDS`).
const MAX_TALLIES: u32 = crate::rewards::MAX_REWARDS as u32;

impl Encode for StarPick {
    fn encode(&self, w: &mut Writer) {
        let StarPick { rarity, name } = self;
        rarity.encode(w);
        name.encode(w);
    }
}

impl Decode for StarPick {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(StarPick {
            rarity: Decode::decode(r)?,
            name: Decode::decode(r)?,
        })
    }
}

impl Encode for RunLedger {
    fn encode(&self, w: &mut Writer) {
        let RunLedger {
            kills,
            bestiary,
            picks,
            skill_uses,
            // not saved: places in `kills`, found again by their keys (`rebuilt_from`).
            enemy_kill_idx: _,
            elite_kill_idx: _,
            boss_kill_idx: _,
            slain_indices: _,
        } = self;
        kills.encode(w);
        bestiary.encode(w);
        picks.encode(w);
        skill_uses.encode(w);
    }
}

impl Decode for RunLedger {
    /// The places in `kills` are left at 0: `rebuilt_from` sets them.
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(RunLedger {
            kills: decode_vec(r, MAX_LEDGER_ENTRIES)?,
            bestiary: decode_vec(r, MAX_LEDGER_ENTRIES)?,
            picks: decode_vec(r, MAX_PICKS)?,
            skill_uses: decode_vec(r, crate::kinds::MAX_KINDS as u32)?,
            enemy_kill_idx: 0,
            elite_kill_idx: 0,
            boss_kill_idx: 0,
            slain_indices: Vec::new(),
        })
    }
}

impl RunLedger {
    /// The ledger of a decode with the places in `kills` found again, and room reserved as
    /// `for_engine` does, for a run whose fresh ledger is `fresh`: the three kill keys of the
    /// weapon and the `slain.` key of each kind are those `fresh` holds at its places. A key
    /// that is missing is refused. `kills` is not necessarily in key order (`for_engine` lists
    /// the kinds in their own order), so the keys are looked up, not searched.
    pub(crate) fn rebuilt_from(mut self, fresh: &RunLedger) -> Result<RunLedger, DecodeError> {
        let places: std::collections::BTreeMap<&str, usize> = self
            .kills
            .iter()
            .enumerate()
            .map(|(at, (key, _))| (key.as_str(), at))
            .collect();
        let place = |fresh_at: usize| {
            places
                .get(fresh.kills[fresh_at].0.as_str())
                .copied()
                .ok_or(DecodeError::Invalid("a kill key of the run is missing"))
        };
        let (boss, elite, enemy) = (
            place(fresh.boss_kill_idx)?,
            place(fresh.elite_kill_idx)?,
            place(fresh.enemy_kill_idx)?,
        );
        let slain: Vec<usize> = fresh
            .slain_indices
            .iter()
            .map(|at| place(*at))
            .collect::<Result<_, _>>()?;
        (self.boss_kill_idx, self.elite_kill_idx, self.enemy_kill_idx) = (boss, elite, enemy);
        self.slain_indices = slain;
        self.kills
            .reserve((3 + fresh.slain_indices.len()).saturating_sub(self.kills.len()));
        self.picks.reserve(32usize.saturating_sub(self.picks.len()));
        self.skill_uses
            .reserve(4usize.saturating_sub(self.skill_uses.len()));
        Ok(self)
    }
}

impl Encode for Counted {
    fn encode(&self, w: &mut Writer) {
        let Counted {
            kills,
            taken,
            tiers,
            named,
        } = self;
        kills.encode(w);
        taken.encode(w);
        tiers.encode(w);
        named.encode(w);
    }
}

impl Decode for Counted {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(Counted {
            kills: Decode::decode(r)?,
            taken: decode_vec(r, MAX_TALLIES)?,
            tiers: decode_vec(r, MAX_TALLIES)?,
            named: decode_vec(r, MAX_PICKS)?,
        })
    }
}
