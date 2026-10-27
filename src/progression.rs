//! What a finished run pays and opens (CS1c, DS49 D-ECON): a functional core the app only
//! persists. `settle` works out the run's Stardust (paid, banked, lost, the bank after), its Rune
//! Shards and the difficulties a clear opens; `open_difficulties` answers the preset screen and
//! `starting_unlocks` a new save. Every number comes from the run's content (`stardust.json`,
//! `difficulties.json`, `starting_unlocks.json`), none from Rust.
//!
//! The rule is ADR-101's, restated by ADR-SAVE-4: a clear pays its own Stardust and the whole
//! bank of its mode and difficulty; a fail pays `pay_pct` of it and banks `bank_pct`, up to
//! `bank_cap`. Stardust is in tenths, as core counts it (`Engine::stardust`); rounding is to a
//! tenth. Nothing here is kept, read from a file or random: the caller passes the bank the save
//! holds and stores the bank returned. The bank is untrusted (a save can be edited): a value
//! over the cap counts as the cap, and all arithmetic is `u64`. The ad multiplier never reaches
//! core (ADR-PLAT-3); it multiplies `Settlement::paid_tenths` only (ADR-101 point 6). The
//! simulation reads nothing from this module, so no state hash depends on it.

use crate::engine::{Engine, Outcome};
use crate::id::ContentId;
use crate::run_content::RunContent;
use crate::run_setup::RunSetup;

/// How an ended run ended (a running run has no result).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Cleared,
    Failed,
}

/// What a settlement needs to know of an ended run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunResult {
    pub mode: ContentId,
    pub difficulty: ContentId,
    pub outcome: RunOutcome,
    /// The Stardust the run earned, times 10 (0 in a stage without a factor).
    pub stardust_tenths: u32,
}

impl RunResult {
    /// The result of the run `setup` started, read from its `engine`: `None` while it runs.
    pub fn from_run(setup: &RunSetup, engine: &Engine) -> Option<RunResult> {
        let outcome = match engine.outcome() {
            Outcome::Running => return None,
            Outcome::Cleared => RunOutcome::Cleared,
            Outcome::Failed => RunOutcome::Failed,
        };
        Some(RunResult {
            mode: setup.mode.clone(),
            difficulty: setup.difficulty.clone(),
            outcome,
            stardust_tenths: engine.stardust(),
        })
    }
}

type Key = (ContentId, ContentId);

/// The Stardust banked by failed runs, in tenths, per (mode, difficulty). A pair it holds no
/// entry for reads as 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bank {
    entries: Vec<(Key, u64)>,
}

impl Bank {
    /// A bank of `entries`, sorted by key; for a key given twice the last value is kept.
    pub fn new(entries: Vec<(Key, u64)>) -> Bank {
        let mut bank = Bank::default();
        for (key, value) in entries {
            bank.set(key, value);
        }
        bank
    }

    /// The tenths banked for `mode` at `difficulty`.
    pub fn get(&self, mode: &ContentId, difficulty: &ContentId) -> u64 {
        let key = (mode.clone(), difficulty.clone());
        match self.entries.binary_search_by(|(k, _)| k.cmp(&key)) {
            Ok(i) => self.entries[i].1,
            Err(_) => 0,
        }
    }

    /// Every entry, sorted by key.
    pub fn entries(&self) -> &[(Key, u64)] {
        &self.entries
    }

    fn set(&mut self, key: Key, value: u64) {
        match self.entries.binary_search_by(|(k, _)| k.cmp(&key)) {
            Ok(i) => self.entries[i].1 = value,
            Err(i) => self.entries.insert(i, (key, value)),
        }
    }
}

/// What a run pays and opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settlement {
    /// The run's own Stardust paid now (what an ad multiplies, ADR-101 point 6).
    pub paid_tenths: u64,
    /// The bank a clear paid out.
    pub bank_paid_tenths: u64,
    /// `paid_tenths + bank_paid_tenths`: what the save's total gains.
    pub stardust_tenths: u64,
    /// What a fail added to the bank, after the cap.
    pub banked_tenths: u64,
    /// What was earned and neither paid nor banked.
    pub lost_tenths: u64,
    /// The whole bank, with the run's entry updated.
    pub bank_after: Bank,
    /// The Rune Shards a clear pays.
    pub rune_shards: u32,
    /// The difficulties this clear opens, in file order, whether or not they were open already.
    pub opened: Vec<ContentId>,
}

/// Settles `run` against the `bank` the save holds, by the numbers of `content`.
///
/// A mode the Stardust file has no entry for pays every outcome whole and banks nothing: a mode
/// added without an entry pays as the app always has.
pub fn settle(content: &RunContent, run: &RunResult, bank: &Bank) -> Settlement {
    let earned = u64::from(run.stardust_tenths);
    let cleared = run.outcome == RunOutcome::Cleared;
    let key = (run.mode.clone(), run.difficulty.clone());
    let mut bank_after = bank.clone();
    let (mut paid, mut bank_paid, mut banked) = (earned, 0, 0);
    if let Some(rules) = content.stardust().mode(&run.mode.0) {
        let cap = u64::from(rules.bank_cap) * 10;
        // A saved value over the cap counts as the cap.
        let held = bank.get(&key.0, &key.1).min(cap);
        if cleared {
            if rules.bank {
                bank_paid = held;
                bank_after.set(key, 0);
            }
        } else {
            paid = earned * u64::from(rules.pay_pct) / 100;
            if rules.bank {
                let share = (earned * u64::from(rules.bank_pct)).div_ceil(100);
                banked = share.min(earned - paid).min(cap - held);
                bank_after.set(key, held + banked);
            }
        }
    }
    Settlement {
        paid_tenths: paid,
        bank_paid_tenths: bank_paid,
        stardust_tenths: paid + bank_paid,
        banked_tenths: banked,
        lost_tenths: if cleared { 0 } else { earned - paid - banked },
        bank_after,
        rune_shards: match content.difficulties().get(&run.difficulty) {
            Some(d) if cleared => d.rune_shards,
            _ => 0,
        },
        opened: if cleared {
            opened_by(content, &run.difficulty)
        } else {
            Vec::new()
        },
    }
}

/// The difficulties whose `opens_after` is `cleared`, in file order.
fn opened_by(content: &RunContent, cleared: &ContentId) -> Vec<ContentId> {
    let known = content.difficulties().get(cleared).is_some();
    content
        .difficulties()
        .list()
        .iter()
        .filter(|d| known && d.opens_after.as_ref() == Some(cleared))
        .map(|d| d.id.clone())
        .collect()
}

/// The difficulties open to a save that has cleared `cleared` in a mode, in file order: each one
/// with no `opens_after` or whose `opens_after` is in `cleared`. The caller passes that mode's
/// clears (DS128: `opens_after` is unchecked per mode).
pub fn open_difficulties(content: &RunContent, cleared: &[ContentId]) -> Vec<ContentId> {
    content
        .difficulties()
        .list()
        .iter()
        .filter(|d| {
            d.opens_after
                .as_ref()
                .is_none_or(|after| cleared.contains(after))
        })
        .map(|d| d.id.clone())
        .collect()
}

/// What kind of thing a starting unlock is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlockKind {
    Weapon,
    PerkSet,
    StageSkill,
    Rune,
}

/// One thing a new save owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unlock {
    pub kind: UnlockKind,
    pub id: ContentId,
}

/// What a new save starts with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartingSave {
    /// Weapons, perk sets, stage skills, then runes, each in file order.
    pub unlocks: Vec<Unlock>,
    /// The weapon every save may play without owning it.
    pub free_weapon: ContentId,
    /// The perk set every save may use without owning it.
    pub free_perk_set: ContentId,
}

/// The unlocks of a new save: `None` when the pack has no starting unlocks file (never a guessed
/// list).
pub fn starting_unlocks(content: &RunContent) -> Option<StartingSave> {
    let file = content.starting_unlocks()?;
    let of = |kind, ids: &[ContentId]| {
        ids.iter()
            .map(|id| Unlock {
                kind,
                id: id.clone(),
            })
            .collect::<Vec<_>>()
    };
    let mut unlocks = of(UnlockKind::Weapon, file.weapons());
    unlocks.extend(of(UnlockKind::PerkSet, file.perk_sets()));
    unlocks.extend(of(UnlockKind::StageSkill, file.stage_skills()));
    unlocks.extend(of(UnlockKind::Rune, file.runes()));
    Some(StartingSave {
        unlocks,
        free_weapon: file.free_weapon().clone(),
        free_perk_set: file.free_perk_set().clone(),
    })
}
