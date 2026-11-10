//! The equipment window of a party in a recorded run (ADR-SIM-7, ADR-REPL-5, ADR-SAVE-4): the
//! savepoint of a party with a downed member resumes to the live run's hashes, and so does the
//! one cut after a leave (a hole in the mask).

use crate::common::run::trial_setup;
use stella_rain_core::engine::Outcome;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::RunRecorder;
use stella_rain_core::run_setup::RunSetup;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

fn pack() -> &'static Pack {
    Pack::bundled()
}

fn setup() -> RunSetup {
    RunSetup { ..trial_setup() }
}

/// One tick: slot 0 walks right, the rest stand; an open offer is answered first.
fn tick(rec: &mut RunRecorder, party: u8) {
    while rec.engine().offer_open() {
        let holder = (0..party)
            .find(|i| rec.engine().check_pick(slot(*i), 0).is_ok())
            .expect("an open offer belongs to a slot");
        rec.choose(slot(holder), 0).unwrap();
    }
    let walk = Input {
        move_x: 100,
        ..Input::default()
    };
    rec.step(&[walk, Input::default(), Input::default()])
        .unwrap();
}

/// A recorder of `party` that has played until slot 1 is downed.
fn until_downed(party: u8) -> RunRecorder {
    let mut rec = RunRecorder::start_party(&setup(), party).unwrap();
    while rec.engine().snapshot().players[1].down.is_none() {
        assert!(rec.engine().tick() < 4_000, "slot 1 goes down");
        tick(&mut rec, party);
    }
    assert_eq!(rec.engine().outcome(), Outcome::Running);
    rec
}

/// L9: a party with a downed member resumes from its savepoint to the same hashes, regeneration
/// and rolls included (they are in the hash).
#[test]
fn a_downed_party_resumes_with_its_rolls() {
    let live = until_downed(2);
    let (prefix, bytes) = (live.replay(), live.save());
    let mut resumed = RunRecorder::resume_from_savepoint(&bytes, &prefix, pack()).unwrap();
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
    let mut live = live;
    for t in 0..180 {
        tick(&mut live, 2);
        tick(&mut resumed, 2);
        if t % 60 == 0 {
            assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        }
    }
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
    assert_eq!(resumed.replay(), live.replay());
}

/// L9: and the savepoint cut after a leave (mask `0b101`) restores the hole with no equipment.
#[test]
fn a_savepoint_after_a_leave_resumes_with_the_hole() {
    let mut live = RunRecorder::start_party(&setup(), 3).unwrap();
    for _ in 0..300 {
        tick(&mut live, 3);
    }
    live.leave(slot(1)).unwrap();
    for _ in 0..60 {
        tick(&mut live, 3);
    }
    assert_eq!(live.engine().present(), 0b101);
    let (prefix, bytes) = (live.replay(), live.save());
    let mut resumed = RunRecorder::resume_from_savepoint(&bytes, &prefix, pack()).unwrap();
    assert_eq!(resumed.engine().present(), 0b101);
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
    for _ in 0..120 {
        tick(&mut live, 3);
        tick(&mut resumed, 3);
    }
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
}

// --- the corpus cases (ADR-SIM-3, ADR-SIM-7) ----------------------------------------------------

use crate::run_play::{EQUIP_PARTY_JOIN, EQUIP_PARTY_LEAVE, PartyCase, equipment_party};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::run_replay::RunReplay;
use stella_rain_core::snapshot::HeldView;
use stella_rain_core::{RangeStep, run_replay::verify_run};

/// What a case did, read back from its replay: the events with the tick they came on, the
/// inventory of every slot after each tick (`None` for an absent slot), and the end.
struct Trace {
    events: Vec<(u32, DomainEvent)>,
    held: Vec<[Option<Vec<HeldView>>; 3]>,
}

impl Trace {
    fn inventory(&self, tick: u32, slot: usize) -> &[HeldView] {
        self.held[tick as usize - 1][slot].as_deref().unwrap_or(&[])
    }

    fn has(&self, tick: u32, slot: usize) -> bool {
        self.held[tick as usize - 1][slot].is_some()
    }

    fn first(&self, want: impl Fn(&DomainEvent) -> bool) -> Option<u32> {
        self.events.iter().find(|e| want(&e.1)).map(|e| e.0)
    }
}

/// Re-simulates `replay` step by step, as `RigRun` does but through joins and leaves.
fn trace(replay: &RunReplay) -> Trace {
    let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
    let n = u8::try_from(range.present.count_ones()).unwrap();
    let mut e = Engine::start_party_with(&replay.setup, Pack::bundled().content(), n).unwrap();
    let mut out = Trace {
        events: vec![],
        held: vec![],
    };
    let spread = |mask: u8, dense: &[Input]| {
        let mut all = [Input::default(); 3];
        let mut it = dense.iter();
        for (s, slot) in all.iter_mut().enumerate() {
            if mask >> s & 1 == 1 {
                *slot = *it.next().unwrap();
            }
        }
        all
    };
    let note = |e: &Engine, out: &mut Trace| {
        out.events
            .extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        let snap = e.snapshot();
        let mut row: [Option<Vec<HeldView>>; 3] = [None, None, None];
        let mut players = snap.players.iter();
        for (s, cell) in row.iter_mut().enumerate() {
            if e.present() >> s & 1 == 1 {
                *cell = Some(players.next().unwrap().inventory.clone());
            }
        }
        out.held.push(row);
    };
    for step in &range.steps {
        match step {
            RangeStep::Ticks { run, inputs } => {
                let all = spread(e.present(), inputs);
                for _ in 0..*run {
                    e.step(&all);
                    note(&e, &mut out);
                }
            }
            RangeStep::Picks { picks } => {
                let mut all = [Input::default(); 3];
                let mut it = picks.iter();
                for (s, slot) in all.iter_mut().enumerate() {
                    if e.present() >> s & 1 == 1 {
                        if let Some(k) = it.next().unwrap() {
                            *slot = Input::picking(*k);
                        }
                    }
                }
                e.step(&all);
            }
            RangeStep::Join { slot: s } => e.join(slot(*s)).unwrap(),
            RangeStep::Leave { slot: s } => e.leave(slot(*s)).unwrap(),
            RangeStep::Actions { .. } => unreachable!("no equipment action here"),
        }
    }
    out
}

fn holds_star_and_rune(items: &[HeldView]) -> bool {
    items.iter().any(|h| matches!(h, HeldView::Star { .. }))
        && items.iter().any(|h| matches!(h, HeldView::Rune { .. }))
}

fn downed(slot_: u8) -> impl Fn(&DomainEvent) -> bool {
    move |e| matches!(e, DomainEvent::PlayerDowned { player } if player.get() == usize::from(slot_))
}

fn revived(slot_: u8) -> impl Fn(&DomainEvent) -> bool {
    move |e| matches!(e, DomainEvent::PlayerRevived { player, .. } if player.get() == usize::from(slot_))
}

/// Whether `case` holds the rules it is in the corpus for (C1 to C3 of the equipment party).
fn holds(case: &PartyCase, n: u8) -> bool {
    let rec = case.play();
    let replay = rec.replay();
    if replay.ticks != 3000 || rec.engine().outcome() != Outcome::Running {
        return false;
    }
    let t = trace(&replay);
    let end = replay.ticks;
    let any = |f: &dyn Fn(&DomainEvent) -> bool| t.first(f).is_some();
    let out = any(&|e| matches!(e, DomainEvent::PlayerOut { .. }));
    let members_hold = (0..usize::from(n)).all(|s| holds_star_and_rune(t.inventory(end, s)));
    match n {
        1 => members_hold && !any(&|e| matches!(e, DomainEvent::PlayerDowned { .. })),
        2 => {
            let (Some(d), Some(r)) = (t.first(downed(1)), t.first(revived(1))) else {
                return false;
            };
            !out && d < r && members_hold && t.inventory(r, 1).len() > t.inventory(d, 1).len()
        }
        _ => {
            let (Some(d), Some(r)) = (t.first(downed(1)), t.first(revived(1))) else {
                return false;
            };
            d < r
                && !out
                && t.has(EQUIP_PARTY_LEAVE, 2)
                && !t.inventory(EQUIP_PARTY_LEAVE, 2).is_empty()
                && !t.has(EQUIP_PARTY_LEAVE + 1, 2)
                && t.inventory(EQUIP_PARTY_JOIN + 1, 2).is_empty()
                && members_hold
        }
    }
}

/// A seed search, run by hand: `cargo test --release --locked --test integration
/// search_equipment_party_seeds -- --ignored --nocapture`. World seeds 1..=100 x bot seeds
/// 1..=10, per party size the first that `holds`; widened once to 1..=300 x 1..=20.
#[test]
#[ignore = "a seed search, run by hand"]
fn search_equipment_party_seeds() {
    use crate::party_bot::RigParty;
    use crate::run_play::Change;
    for n in 1..=3u8 {
        let mut found = None;
        'scan: for (worlds, bots) in [(1..=100u64, 1..=10u64), (1..=300, 1..=20)] {
            for seed in worlds {
                for bot_seed in bots.clone() {
                    let rig = RigParty::new(n).equipment().seed(seed).bot_seed(bot_seed);
                    let rig = if n == 3 {
                        rig.changes(vec![
                            (EQUIP_PARTY_LEAVE, Change::Leave(2)),
                            (EQUIP_PARTY_JOIN, Change::Join(2)),
                        ])
                    } else {
                        rig
                    };
                    if holds(&rig.case("x"), n) {
                        found = Some((seed, bot_seed));
                        break 'scan;
                    }
                }
            }
        }
        println!("N={n}: {found:?}");
    }
}

/// C0: each case's replay verifies to the final hash its recorder reached, and it holds what it
/// is in the corpus for.
#[test]
fn the_equipment_party_cases_hold_the_rules() {
    for n in 1..=3u8 {
        let case = equipment_party(n);
        assert!(holds(&case, n), "{}", case.name);
        let replay = case.play().replay();
        let verified = verify_run(&replay.to_bytes()).unwrap();
        assert_eq!(
            format!("{:016x}", verified.final_hash),
            replay.final_hash,
            "{}",
            case.name
        );
    }
}
