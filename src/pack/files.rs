//! The content files a pack holds and where each sits in core's tree.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::value::RawValue;

use super::{
    BorrowedPack, Pack, PackError, PackFile, check_holds, check_style_grants, file_bytes,
    introduced_at, location,
};

use crate::attack_presets::AttackPresets;
use crate::constellations::Constellations;
use crate::difficulties::Difficulties;
use crate::expertise::Expertise;
use crate::match_ups::MatchUps;
use crate::perk_sets::PerkSets;
use crate::rewards::Rewards;
use crate::rules::Rules;
use crate::run_content::RunContent;
use crate::run_stats::RunStats;
use crate::runes::Runes;
use crate::stage_skill::StageSkills;
use crate::star_chart::StarChart;
use crate::stardust::Stardust;
use crate::starting_unlocks::StartingUnlocks;
use crate::time_limit::TimeLimit;
use crate::validate;
use crate::version::{SCHEMA_VERSION, SIM_VERSION, Versions};
use crate::weapons::Weapons;

/// Where a content file sits in core's tree, and whether a pack may leave it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSlot {
    /// The pack's key for the file, and its name there: `<name>.json`.
    pub name: &'static str,
    /// `modes/` (beside `content/`) rather than `content/`.
    pub in_modes: bool,
    /// A pack released before the file may lack it.
    pub optional: bool,
}

impl FileSlot {
    /// Where the file is in core's tree, given its `content/` directory: `<name>.json` there, or
    /// in `modes/` beside it.
    pub fn path(&self, content_dir: &Path) -> PathBuf {
        let dir = if self.in_modes {
            content_dir.join("..").join("modes")
        } else {
            content_dir.to_path_buf()
        };
        dir.join(format!("{}.json", self.name))
    }
}

const fn slot(name: &'static str, in_modes: bool, optional: bool) -> FileSlot {
    FileSlot {
        name,
        in_modes,
        optional,
    }
}

/// The content files of a pack, as the bytes of the file on its own (a final newline included):
/// what `content/` and `modes/` hold, and what a pack is made from and unpacked into (ADR-CONT-4).
#[derive(Clone, Debug, Default)]
pub struct ContentFiles {
    files: BTreeMap<&'static str, Cow<'static, [u8]>>,
}

impl ContentFiles {
    /// Every file, in the order the pack lists them.
    pub const LAYOUT: [FileSlot; 18] = [
        slot("run_stats", false, false),
        slot("rewards", false, false),
        slot("stage_skills", false, false),
        slot("expertise", false, false),
        slot("match_ups", false, false),
        slot("runes", false, false),
        slot("perk_sets", false, false),
        slot("difficulties", false, false),
        slot("attack_presets", false, false),
        slot("weapons", false, false),
        slot("rules", false, false),
        slot("starting_unlocks", false, true),
        slot("constellations", false, true),
        slot("stardust", false, true),
        slot("star_chart", false, true),
        slot("trial", true, false),
        slot("time_limit", true, false),
        slot("tutorial", true, false),
    ];

    /// Sets a file; false for a name the layout does not have.
    pub fn insert(&mut self, name: &str, bytes: Vec<u8>) -> bool {
        match Self::LAYOUT.iter().find(|s| s.name == name) {
            Some(s) => {
                self.files.insert(s.name, Cow::Owned(bytes));
                true
            }
            None => false,
        }
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files.get(name).map(AsRef::as_ref)
    }

    /// The files of core's tree: each slot of `LAYOUT` read from `content_dir` (core's
    /// `content/`; the mode files from `modes/` beside it). A file that is not there leaves its
    /// slot empty, so that an optional one stays out of the pack and a required one is
    /// `PackError::Missing` from `export`. Reads nothing else (`pack.json` is not a slot).
    pub fn read_dir(content_dir: &Path) -> std::io::Result<ContentFiles> {
        let mut files = ContentFiles::default();
        for slot in Self::LAYOUT {
            match std::fs::read(slot.path(content_dir)) {
                Ok(bytes) => {
                    files.insert(slot.name, bytes);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(files)
    }

    /// The bundled pack's files, each the bytes of the file on its own (a final newline
    /// included): equal to `content/` and `modes/` (tested). Built once from `Pack::BUNDLED`.
    pub fn bundled() -> &'static ContentFiles {
        static BUNDLED: OnceLock<ContentFiles> = OnceLock::new();
        // Split, not re-checked: `Pack::bundled()` reads the same bytes with every check, and
        // tests prove that `export` writes them back. The files share one buffer, made with
        // no temporaries, so that the first call leaves the heap as it found it save one block
        // (the bench cases count the instructions of a tick, which a different heap layout moves).
        BUNDLED.get_or_init(|| {
            let pack: BorrowedPack =
                serde_json::from_slice(Pack::BUNDLED).expect("the bundled pack");
            let members = [
                ("run_stats", Some(pack.run_stats)),
                ("rewards", Some(pack.rewards)),
                ("stage_skills", Some(pack.stage_skills)),
                ("expertise", Some(pack.expertise)),
                ("match_ups", Some(pack.match_ups)),
                ("runes", Some(pack.runes)),
                ("perk_sets", Some(pack.perk_sets)),
                ("difficulties", Some(pack.difficulties)),
                ("attack_presets", Some(pack.attack_presets)),
                ("weapons", Some(pack.weapons)),
                ("rules", Some(pack.rules)),
                ("starting_unlocks", pack.starting_unlocks),
                ("constellations", pack.constellations),
                ("stardust", pack.stardust),
                ("star_chart", pack.star_chart),
                ("trial", Some(pack.modes.trial)),
                ("time_limit", Some(pack.modes.time_limit)),
                ("tutorial", Some(pack.modes.tutorial)),
            ];
            // Each file, then its final newline.
            let total: usize = members
                .iter()
                .flat_map(|(_, raw)| raw)
                .map(|r| r.get().len() + 1)
                .sum();
            let mut buffer = Vec::with_capacity(total);
            for raw in members.iter().flat_map(|(_, raw)| raw) {
                buffer.extend_from_slice(raw.get().as_bytes());
                buffer.push(b'\n');
            }
            let mut rest: &'static [u8] = Box::leak(buffer.into_boxed_slice());
            let mut files = ContentFiles::default();
            for (name, raw) in members {
                if let Some(raw) = raw {
                    let (file, tail) = rest.split_at(raw.get().len() + 1);
                    files.files.insert(name, Cow::Borrowed(file));
                    rest = tail;
                }
            }
            files
        })
    }

    /// The bytes of one bundled file: the one call for library code and tests that need a
    /// content file on its own. Panics for a name the layout lacks: the bundled pack has every
    /// slot (tested).
    pub fn bundled_file(name: &str) -> &'static [u8] {
        Self::bundled()
            .get(name)
            .unwrap_or_else(|| panic!("the bundled pack has no file `{name}`"))
    }

    /// The files of a pack, read with the checks of `Pack::from_bytes`. It is refused when
    /// `export` would not write these bytes back (other whitespace or order, a different
    /// `introduced` table, CRLF), so that the files give the same pack and so the same hash.
    pub fn from_pack(bytes: &[u8]) -> Result<ContentFiles, PackError> {
        Pack::from_bytes(bytes)?;
        let parse_error = |e: serde_json::Error| PackError::Parse {
            line: e.line(),
            column: e.column(),
        };
        let pack: PackFile = serde_json::from_slice(bytes).map_err(parse_error)?;
        let mut files = ContentFiles::default();
        for (name, raw) in members(&pack) {
            files.insert(name, file_bytes(raw));
        }
        for slot in Self::LAYOUT {
            if files.get(slot.name).is_some_and(|b| b.contains(&b'\r')) {
                return Err(PackError::NotExportForm {
                    path: location(slot.name),
                });
            }
        }
        let exported = files.export()?;
        if exported != bytes {
            let other: PackFile = serde_json::from_slice(&exported).map_err(parse_error)?;
            let same_table = serde_json::to_string(&pack.introduced).ok()
                == serde_json::to_string(&other.introduced).ok();
            let path = if !same_table {
                "$.introduced".to_string()
            } else {
                members(&pack)
                    .into_iter()
                    .zip(members(&other))
                    .find(|((_, a), (_, b))| a.get() != b.get())
                    .map_or("$".to_string(), |((name, _), _)| location(name))
            };
            return Err(PackError::NotExportForm { path });
        }
        Ok(files)
    }

    /// The files as a pack: what the content repository's first pack is made from. Each file is
    /// written as it is, without its final newline, so that the pack gives the files' hashes
    /// (see the module's documentation). A file an optional slot lacks is left out of the
    /// pack. The result is checked as `Pack::from_bytes` checks any pack.
    pub fn export(&self) -> Result<Vec<u8>, PackError> {
        let get = |name: &'static str| self.get(name).ok_or(PackError::Missing { file: name });
        let optional = |name: &'static str| self.get(name);
        let file = |bytes: &[u8]| String::from_utf8_lossy(bytes.trim_ascii_end()).into_owned();

        let stage_skills = StageSkills::parse(get("stage_skills")?, Versions::CURRENT)
            .map_err(PackError::in_file("stage_skills"))?;
        let rewards = Rewards::parse_with(get("rewards")?, Versions::CURRENT, &stage_skills)
            .map_err(PackError::in_file("rewards"))?;
        let rewards = match optional("constellations") {
            Some(bytes) => {
                let sets = Constellations::parse_with(bytes, Versions::CURRENT, &rewards)
                    .map_err(PackError::in_file("constellations"))?;
                rewards.with_constellations(sets)
            }
            None => rewards,
        };
        let perk_sets = PerkSets::parse(get("perk_sets")?, Versions::CURRENT)
            .map_err(PackError::in_file("perk_sets"))?;
        let runes =
            Runes::parse(get("runes")?, Versions::CURRENT).map_err(PackError::in_file("runes"))?;
        let attack_presets = AttackPresets::parse(get("attack_presets")?, Versions::CURRENT)
            .map_err(PackError::in_file("attack_presets"))?;
        let weapons = Weapons::parse(get("weapons")?, Versions::CURRENT)
            .map_err(PackError::in_file("weapons"))?;
        check_holds(&weapons, &attack_presets)?;
        let expertise = Expertise::parse(get("expertise")?, Versions::CURRENT)
            .map_err(PackError::in_file("expertise"))?;
        check_style_grants(&expertise, &weapons, &attack_presets)?;
        let rules =
            Rules::parse(get("rules")?, Versions::CURRENT).map_err(PackError::in_file("rules"))?;
        let starting_unlocks = optional("starting_unlocks")
            .map(|bytes| {
                StartingUnlocks::parse_with(
                    bytes,
                    Versions::CURRENT,
                    &perk_sets,
                    &stage_skills,
                    &runes,
                    &weapons,
                )
            })
            .transpose()
            .map_err(PackError::in_file("starting_unlocks"))?;
        let stardust = optional("stardust")
            .map(|b| Stardust::parse(b, Versions::CURRENT))
            .transpose()
            .map_err(PackError::in_file("stardust"))?
            .unwrap_or_else(|| Stardust::bundled().clone());
        let run_stats = RunStats::parse(get("run_stats")?, Versions::CURRENT)
            .map_err(PackError::in_file("run_stats"))?;
        let star_chart = optional("star_chart")
            .map(|bytes| {
                StarChart::parse_with(
                    bytes,
                    Versions::CURRENT,
                    &run_stats,
                    &perk_sets,
                    &stage_skills,
                    &weapons,
                    starting_unlocks.as_ref(),
                )
            })
            .transpose()
            .map_err(PackError::in_file("star_chart"))?;
        let trial = get("trial")?;
        let tutorial = get("tutorial")?;
        let content = RunContent::from_parts(
            run_stats,
            rewards,
            Expertise::parse(get("expertise")?, Versions::CURRENT)
                .map_err(PackError::in_file("expertise"))?,
            MatchUps::parse(get("match_ups")?, Versions::CURRENT)
                .map_err(PackError::in_file("match_ups"))?,
            runes,
            perk_sets,
            Difficulties::parse(get("difficulties")?, Versions::CURRENT)
                .map_err(PackError::in_file("difficulties"))?,
            attack_presets,
            weapons,
            rules,
            starting_unlocks,
            stardust,
            star_chart,
            TimeLimit::parse(get("time_limit")?, Versions::CURRENT)
                .map_err(PackError::in_file("time_limit"))?,
            validate::parse_and_validate(trial).map_err(PackError::in_file("trial"))?,
            trial.to_vec(),
            validate::parse_and_validate(tutorial).map_err(PackError::in_file("tutorial"))?,
            tutorial.to_vec(),
            None,
        );
        let introduced = introduced_at(&content, SIM_VERSION);
        let table = serde_json::to_string_pretty(&introduced).expect("the table serialises");

        let mut out = String::new();
        out.push_str(&format!(
            "{{\n  \"schema_version\": {SCHEMA_VERSION},\n  \"sim_version\": {SIM_VERSION},\n"
        ));
        out.push_str(&format!(
            "  \"introduced\": {},\n",
            table.replace('\n', "\n  ")
        ));
        for slot in Self::LAYOUT.iter().filter(|s| !s.in_modes) {
            if let Some(bytes) = self.get(slot.name) {
                out.push_str(&format!("  \"{}\": {},\n", slot.name, file(bytes)));
            }
        }
        out.push_str(&format!(
            "  \"modes\": {{\n    \"trial\": {},\n    \"time_limit\": {},\n    \"tutorial\": {}\n  }}\n}}\n",
            file(trial),
            file(get("time_limit")?),
            file(tutorial)
        ));
        let out = out.into_bytes();
        Pack::from_bytes(&out)?;
        Ok(out)
    }
}

/// A pack's content files by name, in the layout's order.
fn members(pack: &PackFile) -> Vec<(&'static str, &RawValue)> {
    let mut out: Vec<(&'static str, &RawValue)> = vec![
        ("run_stats", &pack.run_stats),
        ("rewards", &pack.rewards),
        ("stage_skills", &pack.stage_skills),
        ("expertise", &pack.expertise),
        ("match_ups", &pack.match_ups),
        ("runes", &pack.runes),
        ("perk_sets", &pack.perk_sets),
        ("difficulties", &pack.difficulties),
        ("attack_presets", &pack.attack_presets),
        ("weapons", &pack.weapons),
        ("rules", &pack.rules),
    ];
    for (name, raw) in [
        ("starting_unlocks", &pack.starting_unlocks),
        ("constellations", &pack.constellations),
        ("stardust", &pack.stardust),
        ("star_chart", &pack.star_chart),
    ] {
        if let Some(raw) = raw {
            out.push((name, raw));
        }
    }
    out.push(("trial", &pack.modes.trial));
    out.push(("time_limit", &pack.modes.time_limit));
    out.push(("tutorial", &pack.modes.tutorial));
    out
}
