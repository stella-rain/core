//! The weapon tree (ADR-117): a weapon has trees, the run setup names one, and the Sword's and
//! the Hammer's trees have the five-A shape with every smash open. Setups are written as JSON, as
//! a file holds them.

use stella_rain_core::engine::Engine;
use stella_rain_core::event::{DomainEvent, Press};
use stella_rain_core::input::Input;
use stella_rain_core::presets;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::validate::{Error, Issue};
use stella_rain_core::weapon::{Branches, Step};

fn file(weapon: &str, tree: Option<&str>) -> Vec<u8> {
    let tree = tree.map_or(String::new(), |t| format!(r#","weapon_tree":"{t}""#));
    format!(
        r#"{{"schema_version":0,"sim_version":0,"mode":"trial","weapon":"{weapon}","seed":20261010{tree}}}"#
    )
    .into_bytes()
}

fn setup(weapon: &str, tree: Option<&str>) -> RunSetup {
    run_setup::parse_and_validate(&file(weapon, tree)).expect("a valid setup")
}

fn refused(weapon: &str, tree: &str) -> Vec<Issue> {
    match run_setup::parse_and_validate(&file(weapon, Some(tree))) {
        Err(Error::Invalid(issues)) => issues,
        other => panic!("not refused as Invalid: {other:?}"),
    }
}

#[test]
fn a_setup_without_a_tree_reads_as_before_and_names_none() {
    for weapon in ["giant_sword", "giant_hammer"] {
        assert!(run_setup::parse_and_validate(&file(weapon, None)).is_ok());
    }
    // The bytes a setup written before trees wrote are the bytes it writes now.
    let bytes = file("giant_sword", None);
    let again = serde_json::to_vec(&setup("giant_sword", None)).unwrap();
    let a: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let b: serde_json::Value = serde_json::from_slice(&again).unwrap();
    assert!(b.get("weapon_tree").is_none());
    assert_eq!(a["weapon"], b["weapon"]);
}

#[test]
fn a_setup_names_the_weapon_s_first_tree_and_it_is_the_default() {
    for weapon in ["giant_sword", "giant_hammer"] {
        let first = format!("{weapon}_base");
        let first = first.as_str();
        let run = |tree: Option<&str>| {
            let mut e = Engine::start(&setup(weapon, tree)).unwrap();
            (0..300)
                .map(|t| {
                    e.step(Input {
                        main: t % 7 == 0,
                        smash: t % 11 == 5,
                        ..Input::default()
                    });
                    e.state_hash()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(None), run(Some(first)), "{weapon}");
    }
}

#[test]
fn an_unknown_tree_for_the_weapon_is_refused() {
    let issue = |message: &str| Issue {
        path: "$.weapon_tree".into(),
        message: message.into(),
    };
    let message = "no such tree for this weapon";
    assert_eq!(refused("giant_sword", "giant_sword_nope"), [issue(message)]);
    // Another weapon's tree is no tree of this one.
    assert_eq!(
        refused("giant_sword", "giant_hammer_base"),
        [issue(message)]
    );
    assert_eq!(
        refused("giant_hammer", "Not A Tree!"),
        [issue(
            "not a valid ID (lowercase snake_case, at most 32 characters)"
        )]
    );
}

#[test]
fn a_setup_that_names_an_old_upgrade_still_reads_and_the_upgrade_opens_nothing() {
    let bytes = br#"{"schema_version":0,"sim_version":0,"mode":"trial","weapon":"giant_sword","seed":1,"upgrades":["giant_sword_aaab","giant_sword_aab"]}"#;
    assert!(run_setup::parse_and_validate(bytes).is_ok());
}

fn step_at<'a>(branches: &'a Branches, presses: &str) -> Option<&'a Step> {
    let mut at = branches;
    let mut found = None;
    for press in presses.chars() {
        let step = at.get(press == 'B')?;
        found = Some(step);
        at = &step.next;
    }
    found
}

fn every_step<'a>(branches: &'a Branches, out: &mut Vec<&'a Step>) {
    for smash in [false, true] {
        if let Some(step) = branches.get(smash) {
            out.push(step);
            every_step(&step.next, out);
        }
    }
}

#[test]
fn the_sword_and_the_hammer_have_the_five_a_shape_with_no_smash_locked() {
    for weapon in ["giant_sword", "giant_hammer"] {
        let w = presets::weapon_preset(weapon).unwrap();
        for presses in [
            "A", "AA", "AAA", "AAAA", "AAAAA", "B", "AB", "AAB", "AAAB", "AAAAB", "AAAAAB",
        ] {
            assert!(step_at(&w.combo, presses).is_some(), "{weapon} {presses}");
        }
        // Nothing sits under a smash, and AAAAA is the last normal attack.
        assert!(step_at(&w.combo, "AAAAAA").is_none(), "{weapon}");
        let mut all = Vec::new();
        every_step(&w.combo, &mut all);
        every_step(&w.dash, &mut all);
        assert!(all.iter().all(|s| s.unlock.is_none()), "{weapon}: a lock");
    }
}

/// Pulses `main` until the combo reaches depth 5, then `smash` until a smash step: the presses a
/// player makes for AAAAAB, as the depths the engine reports.
fn reach(weapon: &str) -> Vec<(Press, u8)> {
    let mut e = Engine::start(&setup(weapon, None)).unwrap();
    let mut steps = Vec::new();
    for t in 0..900u32 {
        let pressing_smash = steps.last().is_some_and(|(_, d)| *d >= 5);
        e.step(Input {
            main: !pressing_smash && t % 7 == 0,
            smash: pressing_smash && t % 7 == 0,
            ..Input::default()
        });
        for ev in e.events() {
            if let DomainEvent::ComboStep { press, depth } = ev {
                steps.push((*press, *depth));
            }
        }
        if steps
            .last()
            .is_some_and(|(p, d)| *p == Press::Smash && *d == 6)
        {
            break;
        }
    }
    steps
}

#[test]
fn a_recorded_input_reaches_aaaaa_and_aaaaab_in_the_sword_and_the_hammer() {
    for weapon in ["giant_sword", "giant_hammer"] {
        let steps = reach(weapon);
        assert!(steps.contains(&(Press::Main, 5)), "{weapon}: {steps:?}");
        assert_eq!(
            steps.last(),
            Some(&(Press::Smash, 6)),
            "{weapon}: {steps:?}"
        );
    }
}
