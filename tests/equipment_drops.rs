//! Equipment mode drops (ADR-SIM-2 point 2, ADR-SIM-5 point 8, ADR-SIM-7): a star taken and a boss
//! kill give each member their own roll into their own inventory; offers mode does not move.

use stella_rain_core::engine::Engine;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::validate::Error;

fn equipment() -> RunSetup {
    RunSetup {
        ..RunSetup::default()
    }
}

fn messages(error: Error) -> Vec<(String, String)> {
    match error {
        Error::Invalid(found) => found.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not an Invalid error: {other:?}"),
    }
}

/// T1: a setup starts at `sim_version` 2, the equipment mode; the other refusals stay.
#[test]
fn an_equipment_setup_starts() {
    assert!(Engine::start(&equipment()).is_ok());
    assert!(Engine::start_party(&equipment(), 3).is_ok());

    // A sim_version 1 setup plays the frozen rules: no equipment, so no Lumine star.
    let old = RunSetup {
        sim_version: 1,
        ..equipment()
    };
    // The frozen rules play the version 1 pack, not the bundled files.
    let v1 = crate::frozen_v1::pack().content();
    assert!(Engine::start_with(&old, v1).is_ok());
    let frozen_with_star = RunSetup {
        sim_version: 1,
        lumine: 1,
        lumine_star: Some("max_hp_common".into()),
        ..RunSetup::default()
    };
    assert_eq!(
        messages(Engine::start_with(&frozen_with_star, v1).expect_err("refused")),
        [(
            "$.lumine_star".to_string(),
            "needs sim_version 2".to_string()
        )]
    );
}

/// A played boss kill: the tutorial's golem, chased, falls and its member gets a personal drop on
/// that tick (the placeholder is one drop, ADR-CONT-3 point 8).
#[test]
fn a_boss_kill_drops_for_the_member_who_is_up() {
    use crate::common::run::trial_setup;
    use stella_rain_core::engine::Outcome;
    use stella_rain_core::event::DomainEvent;
    use stella_rain_core::snapshot::EntityKind;
    for seed in 1..=5u64 {
        let setup = RunSetup {
            mode: "tutorial".into(),
            seed,
            ..trial_setup()
        };
        let mut e = Engine::start(&setup).unwrap();
        for t in 0..10_800 {
            if e.outcome() != Outcome::Running {
                break;
            }
            let input = crate::zone_bot::run_input_for(
                &e,
                t,
                &[EntityKind::Enemy, EntityKind::Shrine, EntityKind::Boss],
                u32::MAX,
            );
            let before = e.snapshot().players[0].inventory.len();
            e.step(&[input, Default::default(), Default::default()]);
            if e.events().contains(&DomainEvent::StageCleared) {
                let after = e.snapshot().players[0].inventory.len();
                assert_eq!(after, before + 1, "seed {seed}: one personal drop");
            }
        }
        assert_eq!(e.outcome(), Outcome::Cleared, "seed {seed}");
    }
}

/// The seed search for the two equipment corpus cases. Run it by hand:
/// `cargo test --locked --test integration search_equipment_seeds -- --ignored --nocapture`.
/// The cases are the scripted tutorial parties of 1 and 2 (world seeds 1..=60), 3000 ticks, an
/// equipment setup; the first seed of each size where every inventory ends holding a star and a
/// rune is the case's seed. (The tutorial's boss is not killed by the scripted slots in 3000
/// ticks: its drop is played in `a_boss_kill_drops_for_the_member_who_is_up` instead.)
#[cfg(test)]
mod search {
    use super::*;
    use crate::common::run::trial_setup;
    use crate::run_play::PartyCase;
    use stella_rain_core::snapshot::HeldView;

    pub fn case(name: &'static str, n: u8, seed: u64) -> PartyCase {
        PartyCase {
            name,
            setup: RunSetup {
                mode: "tutorial".into(),
                seed,
                ..trial_setup()
            },
            party: n,
            ticks: 3000,
            changes: vec![],
            bot: None,
            script: None,
        }
    }

    #[test]
    #[ignore = "a seed search, run by hand"]
    fn search_equipment_seeds() {
        for n in [1u8, 2] {
            for seed in 1..=60u64 {
                let rec = case("x", n, seed).play();
                let snap = rec.engine().snapshot();
                let has = |p: &stella_rain_core::snapshot::PlayerView| {
                    let runes = p
                        .inventory
                        .iter()
                        .filter(|h| matches!(h, HeldView::Rune { .. }))
                        .count();
                    runes >= 1 && p.inventory.len() > runes
                };
                if snap.players.iter().all(has) {
                    println!(
                        "N={n} seed={seed} ticks={} final_hash={}",
                        rec.replay().ticks,
                        rec.replay().final_hash
                    );
                }
            }
        }
    }
}
