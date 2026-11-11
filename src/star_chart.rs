//! The Star Chart price catalogue (ADR-SAVE-17, ADR-CONT-3 points 1 and 8): the Stardust prices
//! of what the chart sells: weapons, perk sets, stage skills, the unlock that lets a player carry
//! a starting rune, run stats, the expertise multiplier and the Lumine levels. The pack lists
//! them; the app prices a purchase from them (`progression::star_chart_price`), so changing a
//! price is a content release. The simulation never reads this file: no outcome depends on it.
//!
//! Prices are Stardust in tenths, as core counts it (`progression`), from 1 to `MAX_PRICE`: a
//! free item is a starting unlock, not a priced one. A levelled item has one price per level, a
//! list that never falls. No star, reroll or banish has a price (every star is unlocked,
//! ADR-SAVE-17).
//!
//! The file is untrusted (ADR-SVC-1), as every content file is: a size limit before parsing,
//! strict parsing, the current versions only and every list bounded. Each ID must be one the
//! pack's own files hold, once per list, and not one the pack's starting unlocks give away.
//! `Pack` holds the file as an optional field: a pack released before it has none, and then
//! reports no catalogue (`None`), never a guessed list.
//! The bundled file, `content/star_chart.json`, is part of the bundled pack.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::id::ContentId;
use crate::perk_sets::PerkSets;
use crate::registry;
use crate::run_setup::MAX_LUMINE;
use crate::run_stats::RunStats;
use crate::stage_skill::StageSkills;
use crate::starting_unlocks::StartingUnlocks;
use crate::validate::{Error, Issue};
use crate::version::{self, Versions};
use crate::weapons::Weapons;

/// The largest star chart file read (a tuning value; ADR-SVC-1 decides that one exists).
pub const MAX_STAR_CHART_BYTES: usize = 16 * 1024;
/// The most entries one list holds (a tuning value).
pub const MAX_ITEMS: usize = 32;
/// The most levels one levelled item has prices for (a tuning value).
pub const MAX_LEVELS: usize = 10;
/// The ceiling of any price, in tenths of Stardust: 1,000,000 shown (ADR-CONT-3 point 1).
pub const MAX_PRICE: u32 = 10_000_000;

/// One item sold at one price.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PricedId {
    /// A weapon, perk set or stage skill of the pack, once in its list and not a starting unlock.
    pub id: ContentId,
    /// The price in tenths of Stardust, from 1 to `MAX_PRICE`.
    pub price: u32,
}

/// A run stat sold level by level.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatPrices {
    /// A stat of the pack's run stats, once in the list.
    pub id: ContentId,
    /// The price of each level in tenths of Stardust: level n costs `prices[n - 1]`. From 1 to
    /// `MAX_LEVELS` prices, each from 1 to `MAX_PRICE`, never falling.
    pub prices: Vec<u32>,
}

/// The star chart file: what the chart sells and at what price.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarChartFile {
    /// Weapon presets for sale, at most `MAX_ITEMS`.
    pub weapons: Vec<PricedId>,
    /// Perk sets for sale, at most `MAX_ITEMS`.
    pub perk_sets: Vec<PricedId>,
    /// Stage skills for sale, at most `MAX_ITEMS`.
    pub stage_skills: Vec<PricedId>,
    /// The price of the unlock that lets a player carry a starting rune, from 1 to `MAX_PRICE`.
    /// The runes themselves are unlocked by Rune Shards, so none is priced here.
    pub starting_rune: u32,
    /// Run stats for sale, at most `MAX_ITEMS`.
    pub stats: Vec<StatPrices>,
    /// The expertise multiplier's price per level: from 1 to `MAX_LEVELS` prices, never falling.
    pub expertise_multiplier: Vec<u32>,
    /// The Lumine levels' price per level: exactly `MAX_LUMINE` prices, never falling.
    pub lumine_levels: Vec<u32>,
}

/// A star chart that passed validation, and the SHA-256 of the bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarChart {
    file: StarChartFile,
    sha256: [u8; 32],
}

impl StarChart {
    /// Checks the size and the versions given (the pack's), parses strictly and validates against
    /// the files the IDs name: the one call for a file's bytes. `starting_unlocks` is the pack's
    /// own, if it holds one: what it gives away is not for sale.
    pub fn parse_with(
        bytes: &[u8],
        versions: Versions,
        run_stats: &RunStats,
        perk_sets: &PerkSets,
        stage_skills: &StageSkills,
        weapons: &Weapons,
        starting_unlocks: Option<&StartingUnlocks>,
    ) -> Result<StarChart, Error> {
        let file: StarChartFile = version::read_member(bytes, MAX_STAR_CHART_BYTES, versions)?;
        validate(
            &file,
            run_stats,
            perk_sets,
            stage_skills,
            weapons,
            starting_unlocks,
        )?;
        Ok(StarChart {
            file,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The weapons for sale and their prices.
    pub fn weapons(&self) -> &[PricedId] {
        &self.file.weapons
    }

    /// The perk sets for sale and their prices.
    pub fn perk_sets(&self) -> &[PricedId] {
        &self.file.perk_sets
    }

    /// The stage skills for sale and their prices.
    pub fn stage_skills(&self) -> &[PricedId] {
        &self.file.stage_skills
    }

    /// The price of the unlock that lets a player carry a starting rune.
    pub fn starting_rune(&self) -> u32 {
        self.file.starting_rune
    }

    /// The run stats for sale and the price of each level.
    pub fn stats(&self) -> &[StatPrices] {
        &self.file.stats
    }

    /// The price of each level of the expertise multiplier.
    pub fn expertise_multiplier(&self) -> &[u32] {
        &self.file.expertise_multiplier
    }

    /// The price of each Lumine level.
    pub fn lumine_levels(&self) -> &[u32] {
        &self.file.lumine_levels
    }

    /// The SHA-256 of the bytes they were parsed from.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// Whether an ID is one the game holds.
trait KnownId: Fn(&ContentId) -> bool {}
impl<F: Fn(&ContentId) -> bool> KnownId for F {}

/// One priced list and how to tell what its IDs are: known to the pack, or given away.
struct Listed<'a> {
    name: &'static str,
    list: &'a [PricedId],
    known: &'a dyn KnownId,
    given: fn(&StartingUnlocks) -> &[ContentId],
}

fn check_price(issues: &mut Vec<Issue>, path: String, price: u32) {
    if !(1..=MAX_PRICE).contains(&price) {
        issues.push(Issue {
            path,
            message: format!("a price from 1 to {MAX_PRICE} (found {price})"),
        });
    }
}

/// A list of prices of levels: its length, each price, and that it never falls.
fn check_levels(
    issues: &mut Vec<Issue>,
    path: &str,
    prices: &[u32],
    min_len: usize,
    max_len: usize,
) {
    if prices.len() < min_len || prices.len() > max_len {
        let wanted = if min_len == max_len {
            format!("exactly {min_len} prices")
        } else {
            format!("{min_len} to {max_len} prices")
        };
        issues.push(Issue {
            path: path.to_owned(),
            message: format!("{wanted} (found {})", prices.len()),
        });
        return;
    }
    for (i, price) in prices.iter().enumerate() {
        check_price(issues, format!("{path}[{i}]"), *price);
    }
    if prices.windows(2).any(|w| w[1] < w[0]) {
        issues.push(Issue {
            path: path.to_owned(),
            message: "a level never costs less than the one before".into(),
        });
    }
}

/// Validates a parsed file: one issue per problem, with its path, never text from the file.
pub fn validate(
    file: &StarChartFile,
    run_stats: &RunStats,
    perk_sets: &PerkSets,
    stage_skills: &StageSkills,
    weapons: &Weapons,
    starting_unlocks: Option<&StartingUnlocks>,
) -> Result<(), Error> {
    let mut issues = Vec::new();
    let weapon_known = |id: &ContentId| weapons.ids().any(|w| w == id);
    let perk_set_known = |id: &ContentId| perk_sets.index_of(id).is_some();
    let stage_skill_known = |id: &ContentId| stage_skills.index_of(id).is_some();
    let stat_known = |id: &ContentId| run_stats.stats().iter().any(|s| &s.id == id);
    let free = |list: fn(&StartingUnlocks) -> &[ContentId], id: &ContentId| {
        starting_unlocks.is_some_and(|u| list(u).contains(id))
    };

    let priced = [
        Listed {
            name: "weapons",
            list: &file.weapons,
            known: &weapon_known,
            given: StartingUnlocks::weapons,
        },
        Listed {
            name: "perk_sets",
            list: &file.perk_sets,
            known: &perk_set_known,
            given: StartingUnlocks::perk_sets,
        },
        Listed {
            name: "stage_skills",
            list: &file.stage_skills,
            known: &stage_skill_known,
            given: StartingUnlocks::stage_skills,
        },
    ];
    for Listed {
        name,
        list,
        known,
        given,
    } in priced
    {
        if list.len() > MAX_ITEMS {
            issues.push(Issue {
                path: format!("$.{name}"),
                message: format!("at most {MAX_ITEMS} entries (found {})", list.len()),
            });
            continue;
        }
        for (i, entry) in list.iter().enumerate() {
            let path = format!("$.{name}[{i}].id");
            if !registry::is_well_formed(entry.id.as_str()) {
                issues.push(Issue {
                    path,
                    message: "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
                });
            } else if list[..i].iter().any(|e| e.id == entry.id) {
                issues.push(Issue {
                    path,
                    message: "the same ID as an earlier entry".into(),
                });
            } else if !known(&entry.id) {
                issues.push(Issue {
                    path,
                    message: "not an ID the game holds".into(),
                });
            } else if free(given, &entry.id) {
                issues.push(Issue {
                    path,
                    message: "free from the start: not for sale".into(),
                });
            }
            check_price(&mut issues, format!("$.{name}[{i}].price"), entry.price);
        }
    }

    check_price(&mut issues, "$.starting_rune".into(), file.starting_rune);

    if file.stats.len() > MAX_ITEMS {
        issues.push(Issue {
            path: "$.stats".into(),
            message: format!("at most {MAX_ITEMS} entries (found {})", file.stats.len()),
        });
    } else {
        for (i, stat) in file.stats.iter().enumerate() {
            let path = format!("$.stats[{i}].id");
            if !registry::is_well_formed(stat.id.as_str()) {
                issues.push(Issue {
                    path,
                    message: "not a valid ID (lowercase snake_case, at most 32 characters)".into(),
                });
            } else if file.stats[..i].iter().any(|s| s.id == stat.id) {
                issues.push(Issue {
                    path,
                    message: "the same ID as an earlier entry".into(),
                });
            } else if !stat_known(&stat.id) {
                issues.push(Issue {
                    path,
                    message: "not an ID the game holds".into(),
                });
            }
            check_levels(
                &mut issues,
                &format!("$.stats[{i}].prices"),
                &stat.prices,
                1,
                MAX_LEVELS,
            );
        }
    }
    check_levels(
        &mut issues,
        "$.expertise_multiplier",
        &file.expertise_multiplier,
        1,
        MAX_LEVELS,
    );
    let lumine = usize::from(MAX_LUMINE);
    check_levels(
        &mut issues,
        "$.lumine_levels",
        &file.lumine_levels,
        lumine,
        lumine,
    );
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(issues))
    }
}
