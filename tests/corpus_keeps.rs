//! What a re-blessed `current/` case must keep (ADR-SIM-3, ADR-REPL-2): the hashes of the
//! `current/` cases are re-recorded when the hash layout moves, but the run they record is the
//! same run. The table is written by hand from the tree before the layout move; the bless never
//! writes it. `gameplay_observables::PINS` guard the same runs by digest, this table says which
//! of the four moved: the ticks played, the outcome, the rewards taken (and the stars named) and
//! the Stardust.

use std::fmt::Write as _;
use std::path::Path;

use stella_rain_core::engine::Engine;
use stella_rain_core::replay::Replay;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::validate::parse_and_validate;

use crate::common::run_steps::{self, RunSteps};
use crate::corpus::current_dir;
use crate::corpus_pins::{case_dirs, case_pack, run_case_pack};

/// One case: name, ticks played, `Debug` of the outcome, the rewards taken as `id=count` and the
/// star names in event order, the Stardust (times 10).
type Keep = (&'static str, u32, &'static str, &'static str, u32);

const KEEPS: &[Keep] = &[
    (
        "runs/easy",
        6000,
        "Running",
        "max_hp_common=5;max_hp_rare=3;attack_common=5;attack_rare=1;attack_speed_rare=2;evade_common=5;evade_rare=1;speed_common=5;speed_rare=2;xp_common=5;xp_rare=1;black_hole_rare=1;war_cry_rare=2;aegis_rare=2;regen_common=5;max_hp_uncommon=1;attack_uncommon=1;attack_speed_uncommon=4;speed_uncommon=2;xp_uncommon=1;regen_uncommon=3;rune_haste_common=5;rune_haste_uncommon=2;rune_haste_rare=1;range_common=5;range_uncommon=3;range_rare=2;type_bonus_slash_common=5;type_bonus_pierce_common=5;type_bonus_strike_common=5;type_bonus_pyro_common=5;type_bonus_hydro_common=5;type_bonus_hydro_uncommon=1;type_bonus_electro_common=5;type_bonus_electro_uncommon=1;type_bonus_cryo_common=5;type_bonus_cryo_rare=1;type_bonus_anemo_common=5;type_bonus_geo_common=5;type_bonus_dendro_common=5;random_rune=7;|",
        12930,
    ),
    (
        "runs/elites",
        48700,
        "Failed",
        "attack_common=1;attack_speed_common=1;regen_common=3;speed_uncommon=1;rune_haste_common=2;rune_haste_uncommon=1;range_common=2;|",
        880,
    ),
    (
        "runs/rainy",
        6000,
        "Running",
        "max_hp_rare=3;attack_rare=3;attack_speed_rare=3;evade_rare=2;speed_rare=3;xp_rare=2;black_hole_rare=3;war_cry_rare=3;hex_rare=3;nova_rare=3;aegis_rare=3;regen_rare=3;max_hp_uncommon=4;attack_uncommon=4;attack_speed_uncommon=4;evade_uncommon=4;speed_uncommon=4;xp_uncommon=4;regen_uncommon=4;rune_haste_uncommon=4;rune_haste_rare=3;range_uncommon=4;range_rare=3;type_bonus_slash_uncommon=4;type_bonus_slash_rare=1;type_bonus_pierce_uncommon=4;type_bonus_strike_uncommon=4;type_bonus_pyro_uncommon=4;type_bonus_hydro_uncommon=4;type_bonus_electro_uncommon=4;type_bonus_electro_rare=1;type_bonus_cryo_uncommon=4;type_bonus_anemo_uncommon=4;type_bonus_anemo_rare=1;type_bonus_dendro_uncommon=4;type_bonus_dendro_rare=1;|",
        25140,
    ),
    ("runs/stage_skill", 5400, "Failed", "black_hole_rare=1;|", 0),
    (
        "runs/time_limit",
        18000,
        "Running",
        "max_hp_common=5;max_hp_rare=3;max_hp_legendary=2;attack_common=5;attack_rare=3;attack_legendary=2;attack_speed_common=5;attack_speed_rare=3;attack_speed_legendary=2;evade_common=5;evade_rare=3;evade_legendary=2;speed_common=5;speed_rare=3;speed_legendary=2;xp_common=5;xp_rare=3;xp_legendary=2;black_hole_rare=3;war_cry_rare=3;hex_rare=3;nova_rare=3;aegis_rare=3;regen_common=5;regen_rare=3;regen_legendary=2;max_hp_uncommon=4;max_hp_epic=2;attack_uncommon=4;attack_epic=2;attack_speed_uncommon=4;attack_speed_epic=2;evade_uncommon=4;evade_epic=2;speed_uncommon=4;speed_epic=2;xp_uncommon=4;xp_epic=2;regen_uncommon=4;regen_epic=2;rune_haste_common=5;rune_haste_uncommon=4;rune_haste_rare=3;rune_haste_epic=2;rune_haste_legendary=2;range_uncommon=4;range_rare=3;range_epic=2;range_legendary=2;type_bonus_slash_common=5;type_bonus_slash_uncommon=4;type_bonus_slash_rare=3;type_bonus_slash_epic=2;type_bonus_slash_legendary=2;type_bonus_pierce_common=5;type_bonus_pierce_uncommon=4;type_bonus_pierce_rare=3;type_bonus_pierce_legendary=2;type_bonus_strike_common=5;type_bonus_strike_uncommon=4;type_bonus_strike_rare=3;type_bonus_strike_epic=2;type_bonus_strike_legendary=2;type_bonus_pyro_common=5;type_bonus_pyro_uncommon=4;type_bonus_pyro_rare=3;type_bonus_pyro_epic=2;type_bonus_pyro_legendary=2;type_bonus_hydro_common=5;type_bonus_hydro_uncommon=4;type_bonus_hydro_rare=3;type_bonus_hydro_epic=1;type_bonus_hydro_legendary=2;type_bonus_electro_common=5;type_bonus_electro_uncommon=4;type_bonus_electro_rare=3;type_bonus_electro_legendary=2;type_bonus_cryo_common=5;type_bonus_cryo_uncommon=4;type_bonus_cryo_rare=3;type_bonus_cryo_epic=1;type_bonus_cryo_legendary=2;type_bonus_anemo_common=5;type_bonus_anemo_uncommon=4;type_bonus_anemo_rare=3;type_bonus_anemo_epic=1;type_bonus_anemo_legendary=2;type_bonus_geo_common=5;type_bonus_geo_uncommon=4;type_bonus_geo_rare=3;type_bonus_geo_epic=1;type_bonus_geo_legendary=2;type_bonus_dendro_common=5;type_bonus_dendro_uncommon=4;type_bonus_dendro_rare=3;type_bonus_dendro_epic=1;type_bonus_dendro_legendary=2;random_rune=7;star_altair=1;star_alnitak=1;star_alnilam=1;star_mintaka=1;star_dubhe=1;star_rigel=1;star_deneb=1;star_vega=1;star_mizar=1;star_alkaid=1;star_procyon=1;star_antares=1;|Antares;Vega;Mizar;Deneb;Rigel;Alkaid;Alnilam;Mintaka;Procyon;Alnitak;Dubhe;Altair;",
        29340,
    ),
    ("runs/time_limit_kill", 1856, "Cleared", "|", 2000),
    ("runs/time_limit_overtime", 600, "Failed", "|", 0),
    (
        "runs/trial",
        2895,
        "Failed",
        "max_hp_common=1;attack_common=1;nova_rare=1;range_common=2;|",
        0,
    ),
    ("runs/tutorial", 297, "Cleared", "|", 0),
    ("stages/agents", 80, "Running", "|", 0),
    ("stages/assist", 360, "Running", "|", 0),
    ("stages/boss", 68, "Cleared", "|", 0),
    ("stages/clear", 35, "Cleared", "|", 0),
    ("stages/dash", 300, "Running", "|", 0),
    ("stages/evade", 300, "Failed", "|", 0),
    ("stages/fail", 16, "Failed", "|", 0),
    ("stages/landscape", 300, "Failed", "|", 0),
    ("stages/leash_edge", 300, "Running", "|", 0),
    ("stages/melee", 300, "Running", "|", 0),
    ("stages/orb_cage", 300, "Running", "|", 0),
    ("stages/orbs", 300, "Running", "|", 0),
    ("stages/ranged_cap", 300, "Running", "|", 0),
    ("stages/timeout", 90, "Failed", "|", 0),
    ("stages/warn_anchor", 300, "Running", "|", 0),
    ("stages/zones", 300, "Running", "|", 0),
];

fn observed(engine: &Engine, inputs: usize) -> (u32, String, String, u32) {
    let tick = engine.tick();
    assert_eq!(
        tick as usize, inputs,
        "the engine played every recorded tick"
    );
    let tally = engine.tally();
    let mut picks = String::new();
    for (id, n) in tally.taken() {
        write!(picks, "{id}={n};").unwrap();
    }
    picks.push('|');
    for name in tally.named() {
        write!(picks, "{name};").unwrap();
    }
    (
        tick,
        format!("{:?}", engine.outcome()),
        picks,
        engine.stardust(),
    )
}

fn stage_row(case: &Path) -> (u32, String, String, u32) {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let stage_bytes = std::fs::read(case.join("stage.json")).expect("stage.json");
    let stage = parse_and_validate(&stage_bytes).expect("the stage parses");
    let recorded = Replay::from_bytes(&std::fs::read(case.join("run.replay")).expect("run.replay"))
        .expect("the replay parses");
    let inputs = recorded.unpack_inputs(stage.length_ticks).expect("inputs");
    let pack = case_pack(case).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut engine = Engine::new_with(&stage, pack.content()).expect("the stage starts");
    for input in &inputs {
        engine.step(&[*input, Default::default(), Default::default()]);
    }
    observed(&engine, inputs.len())
}

fn run_row(case: &Path) -> (u32, String, String, u32) {
    let name = case.file_name().unwrap().to_string_lossy().into_owned();
    let bytes = std::fs::read(case.join("run.replay")).expect("run.replay");
    let recorded = RunReplay::from_bytes(&bytes).expect("the run replay parses");
    let pack = run_case_pack(case, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut engine = run_steps::start(&recorded, pack.content());
    let steps = RunSteps::of(&recorded);
    for step in steps.steps() {
        run_steps::apply(&mut engine, step);
    }
    observed(&engine, steps.ticks as usize)
}

fn rows() -> Vec<(String, (u32, String, String, u32))> {
    let mut out = Vec::new();
    for (sub, row) in [
        ("stages", stage_row as fn(&Path) -> _),
        ("runs", run_row as fn(&Path) -> _),
    ] {
        let dirs = case_dirs(&current_dir().join(sub));
        assert!(!dirs.is_empty(), "no cases in {sub}");
        std::thread::scope(|scope| {
            let handles: Vec<_> = dirs
                .iter()
                .map(|dir| {
                    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
                    (format!("{sub}/{name}"), scope.spawn(move || row(dir)))
                })
                .collect();
            for (name, h) in handles {
                out.push((name, h.join().expect("the case runs")));
            }
        });
    }
    out
}

#[test]
fn every_current_case_keeps_its_ticks_outcome_picks_and_stardust() {
    let mut wrong = String::new();
    for (name, (ticks, outcome, picks, dust)) in rows() {
        let computed = format!("    ({name:?}, {ticks}, {outcome:?}, {picks:?}, {dust}),\n");
        match KEEPS.iter().find(|k| k.0 == name) {
            Some(&(_, t, o, p, d)) => {
                if (t, o, p, d) != (ticks, outcome.as_str(), picks.as_str(), dust) {
                    let _ = write!(wrong, "{name}: moved, computed\n{computed}");
                }
            }
            None => {
                let _ = write!(wrong, "{name}: no row, computed\n{computed}");
            }
        }
    }
    assert!(wrong.is_empty(), "a case moved or has no row:\n{wrong}");
}

#[test]
fn every_keep_names_a_case() {
    let mut names = Vec::new();
    for sub in ["stages", "runs"] {
        for dir in case_dirs(&current_dir().join(sub)) {
            names.push(format!(
                "{sub}/{}",
                dir.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    for k in KEEPS {
        assert!(names.iter().any(|n| n == k.0), "{} is not a case", k.0);
    }
    assert_eq!(KEEPS.len(), names.len(), "one row per case");
}
