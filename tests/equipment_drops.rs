//! Equipment mode drops (ADR-SIM-2 point 2, ADR-SIM-5 point 8, ADR-SIM-7): a star taken and a boss
//! kill give each member their own roll into their own inventory; offers mode does not move.

use stella_rain_core::engine::Engine;
use stella_rain_core::run_setup::{BuildMode, RunSetup};
use stella_rain_core::validate::Error;

fn equipment() -> RunSetup {
    RunSetup {
        build_mode: BuildMode::Equipment,
        ..RunSetup::default()
    }
}

fn messages(error: Error) -> Vec<(String, String)> {
    match error {
        Error::Invalid(found) => found.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not an Invalid error: {other:?}"),
    }
}

/// T1: an equipment setup starts at `sim_version` 2; the other refusals stay.
#[test]
fn an_equipment_setup_starts() {
    assert!(Engine::start(&equipment()).is_ok());
    assert!(Engine::start_party(&equipment(), 3).is_ok());

    let old = RunSetup {
        sim_version: 1,
        ..equipment()
    };
    assert_eq!(
        messages(Engine::start(&old).expect_err("refused")),
        [(
            "$.build_mode".to_string(),
            "equipment mode needs sim_version 2".to_string()
        )]
    );

    let offers_with_star = RunSetup {
        lumine: 1,
        lumine_star: Some("max_hp_common".into()),
        ..RunSetup::default()
    };
    assert_eq!(
        messages(Engine::start(&offers_with_star).expect_err("refused")),
        [(
            "$.lumine_star".to_string(),
            "needs equipment mode".to_string()
        )]
    );
}

/// A played boss kill: the tutorial's golem, chased, falls and its member gets a personal drop on
/// that tick (the placeholder is one drop, ADR-CONT-3 point 8).
#[test]
fn a_boss_kill_drops_for_the_member_who_is_up() {
    use crate::common::run::{answer_all, trial_setup};
    use stella_rain_core::engine::Outcome;
    use stella_rain_core::event::DomainEvent;
    use stella_rain_core::snapshot::EntityKind;
    for seed in 1..=5u64 {
        let setup = RunSetup {
            mode: "tutorial".into(),
            seed,
            build_mode: BuildMode::Equipment,
            ..trial_setup()
        };
        let mut e = Engine::start(&setup).unwrap();
        let (mut out, mut offers) = (Vec::new(), 0);
        for t in 0..10_800 {
            if e.outcome() != Outcome::Running {
                break;
            }
            answer_all(&mut e, &mut offers, &mut out);
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
                build_mode: BuildMode::Equipment,
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
