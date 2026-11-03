//! The per-player totals the engine folds from the events (ADR-SIM-7): what a solo run counts,
//! that a replay rebuilds them exactly, and that a run resumed from a savepoint has the totals
//! of the whole run (format 2 holds them, ADR-SAVE-4).

use crate::common::run::trial_setup;
use crate::run_play::play_run;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{EntityId, PlayerIdx};
use stella_rain_core::ledger::SlotStats;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::{Command, RunRecorder};
use stella_rain_core::snapshot::EntityKind;

const TICKS: u32 = 1500;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// Plays `commands` on `rec`, adding up from the raw events what a solo run should count:
/// the damage of the hits the main character made and the enemies defeated.
fn play(rec: &mut RunRecorder, commands: &[Command]) -> (u64, u32) {
    let (mut damage, mut kills) = (0u64, 0u32);
    for c in commands {
        match *c {
            Command::Step(input) => rec
                .step(&[input, Default::default(), Default::default()])
                .unwrap(),
            Command::Choose(index) => rec.choose(PlayerIdx::SOLO, index).unwrap(),
        }
        for e in rec.engine().events() {
            match e {
                DomainEvent::Hit {
                    source, damage: d, ..
                } if *source == EntityId(0) => damage += u64::from(*d),
                DomainEvent::Died {
                    class: EntityKind::Enemy,
                    ..
                } => kills += 1,
                _ => {}
            }
        }
    }
    (damage, kills)
}

fn started() -> RunRecorder {
    RunRecorder::start_with_pack(&trial_setup(), Pack::bundled()).unwrap()
}

#[test]
fn a_solo_run_counts_its_hits_and_kills_to_slot_zero() {
    let mut rec = started();
    let (damage, kills) = play(&mut rec, &play_run(TICKS));
    let stats = rec.engine().player_stats();
    assert!(damage > 0 && kills > 0, "the run fights: {damage}, {kills}");
    let want = SlotStats {
        damage_dealt: damage,
        kills,
        support: 0,
    };
    assert_eq!(stats.slot(slot(0)), want);
    assert_eq!(stats.slot(slot(1)), SlotStats::default());
    assert_eq!(stats.slot(slot(2)), SlotStats::default());
    assert_eq!(kills, rec.engine().tally().kills());
}

#[test]
fn a_replay_rebuilds_the_totals_exactly() {
    let mut rec = started();
    play(&mut rec, &play_run(TICKS));
    let again = RunRecorder::resume_with_pack(&rec.replay(), Pack::bundled()).unwrap();
    assert_eq!(again.engine().player_stats(), rec.engine().player_stats());
    assert_eq!(
        rec.engine().clone().player_stats(),
        rec.engine().player_stats()
    );
}

#[test]
fn a_run_resumed_from_a_savepoint_has_exact_totals() {
    let all = play_run(TICKS);
    let (head, tail) = all.split_at(all.len() / 2);
    let mut rec = started();
    play(&mut rec, head);
    let (prefix, saved) = (rec.replay(), rec.save());
    let mut resumed = RunRecorder::resume_from_savepoint(&saved, &prefix, Pack::bundled()).unwrap();
    assert_eq!(resumed.engine().player_stats(), rec.engine().player_stats());
    play(&mut rec, tail);
    play(&mut resumed, tail);
    assert_eq!(resumed.engine().player_stats(), rec.engine().player_stats());
}
