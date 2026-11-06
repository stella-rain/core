//! A party of 2 and 3 recorded natively: the engine starts it, the recorder steps and picks per
//! player, and record, resume, `replay_at`, save and verify agree (ADR-REPL-2 point 4,
//! ADR-REPL-5, ADR-SAVE-4). Slots 1 and 2 act in the rules (ADR-SIM-7), so the party cases pin
//! what their play does to the run.

use crate::common::run::trial_setup;
use crate::run_play::party_cases;
use crate::zone_bot::run_input;
use stella_rain_core::engine::{ChoiceError, Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::{RunRecorder, RunReplay, RunReplayError, verify_run};
use stella_rain_core::savepoint::SavepointError;
use stella_rain_core::{RangeStep, StepRange};

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).unwrap()
}

/// The scripted input of `slot` at `tick`: three distinct patterns, so the log carries each slot.
fn input_of(slot: usize, tick: u32) -> Input {
    match slot {
        0 => Input {
            move_x: if tick / 40 % 2 == 0 { 100 } else { -100 },
            main: true,
            ..Input::default()
        },
        1 => Input {
            move_y: 50,
            skill: tick % 90 < 30,
            ..Input::default()
        },
        _ => Input {
            move_x: -30,
            evade: tick % 120 < 10,
            ..Input::default()
        },
    }
}

/// Answers every open offer, each by the slot that holds it, one `choose` at a time.
fn answer_offers(rec: &mut RunRecorder, n: u8) {
    while rec.engine().offer_open() {
        let holder = (0..n)
            .find(|i| rec.engine().check_pick(slot(*i), 0).is_ok())
            .expect("an open offer belongs to a slot");
        rec.choose(slot(holder), 0).unwrap();
    }
}

/// The inputs of every slot at `tick`: slot 0 played by the zone bot, the others scripted.
fn inputs_at(rec: &RunRecorder, n: u8, tick: u32) -> [Input; 3] {
    let mut inputs = [Input::default(); 3];
    for s in 0..n {
        inputs[usize::from(s)] = match s {
            0 => run_input(rec.engine(), tick),
            _ => input_of(usize::from(s), tick),
        };
    }
    inputs
}

/// A record's dense entries, one per slot of `mask`, placed at their slots.
fn slots<T: Copy + Default>(mask: u8, dense: &[T]) -> [T; 3] {
    let mut all = [T::default(); 3];
    let mut from = dense.iter();
    for (slot, to) in all.iter_mut().enumerate() {
        if mask >> slot & 1 == 1 {
            *to = *from.next().unwrap();
        }
    }
    all
}

fn step_party(rec: &mut RunRecorder, n: u8, tick: u32) {
    answer_offers(rec, n);
    let inputs = inputs_at(rec, n, tick);
    rec.step(&inputs).unwrap();
}

fn play(n: u8, ticks: u32) -> RunRecorder {
    let mut rec = RunRecorder::start_party(&trial_setup(), n).unwrap();
    for t in 0..ticks {
        if rec.engine().outcome() != Outcome::Running {
            break;
        }
        step_party(&mut rec, n, t);
    }
    answer_offers(&mut rec, n);
    rec
}

fn pack() -> &'static Pack {
    Pack::bundled()
}

// A. The engine starts a party.

#[test]
fn the_engine_starts_a_party_of_one_to_three() {
    let setup = trial_setup();
    for n in 1..=3u8 {
        let engine = Engine::start_party_with(&setup, pack().content(), n).unwrap();
        assert_eq!(engine.players(), usize::from(n));
    }
    assert!(Engine::start_party_with(&setup, pack().content(), 0).is_err());
    assert!(Engine::start_party_with(&setup, pack().content(), 4).is_err());
}

#[test]
fn a_party_of_one_is_the_solo_start() {
    let setup = trial_setup();
    let mut a = Engine::start_with(&setup, pack().content()).unwrap();
    let mut b = Engine::start_party_with(&setup, pack().content(), 1).unwrap();
    for t in 0..120 {
        a.step(&[input_of(0, t), Default::default(), Default::default()]);
        b.step(&[input_of(0, t), Default::default(), Default::default()]);
    }
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn a_party_carries_the_setup_loadout_in_every_slot() {
    let setup = stella_rain_core::run_setup::RunSetup {
        stat_priority: vec![],
        ..trial_setup()
    };
    let a = Engine::start_party_with(&setup, pack().content(), 3).unwrap();
    let b = Engine::start_party_with(&setup, pack().content(), 3).unwrap();
    assert_eq!(a.state_hash(), b.state_hash());
    let solo = Engine::start_with(&setup, pack().content()).unwrap();
    assert_ne!(a.state_hash(), solo.state_hash(), "a party is another run");
}

// B. The recorder.

#[test]
fn a_recorded_party_verifies() {
    for n in [2u8, 3] {
        let rec = play(n, 300);
        let replay = rec.replay();
        let verified = verify_run(&replay.to_bytes()).unwrap();
        assert_eq!(verified.ticks, rec.engine().tick());
        let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
        assert_eq!(range.present, (1u8 << n) - 1);
        let first = range
            .steps
            .iter()
            .find_map(|s| match s {
                RangeStep::Ticks { inputs, .. } => Some(inputs.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(first.len(), usize::from(n));
        for (s, got) in first.iter().enumerate().skip(1) {
            assert_eq!(*got, input_of(s, 0), "slot {s}");
        }
    }
}

#[test]
fn a_stick_axis_of_minus_128_is_canonical() {
    let mut rec = RunRecorder::start_party(&trial_setup(), 2).unwrap();
    let a = Input {
        move_x: -128,
        ..Input::default()
    };
    let b = Input {
        move_x: -127,
        ..Input::default()
    };
    rec.step(&[Input::default(), a, Input::default()]).unwrap();
    rec.step(&[Input::default(), b, Input::default()]).unwrap();
    verify_run(&rec.replay().to_bytes()).unwrap();
}

#[test]
fn the_recorder_refuses_what_it_cannot_record() {
    let mut rec = RunRecorder::start_party(&trial_setup(), 2).unwrap();
    assert_eq!(rec.engine().tick(), 0);
    assert_eq!(
        rec.picks(&[None, None, None]).unwrap_err(),
        RunReplayError::BadInputs("a step of picks holds a pick")
    );
    assert!(matches!(
        rec.choose(slot(1), 0).unwrap_err(),
        RunReplayError::ChoiceRefused {
            error: ChoiceError::NoOffer,
            ..
        }
    ));
    assert_eq!(
        rec.choose(slot(2), 0).unwrap_err(),
        RunReplayError::BadInputs("a player outside the party")
    );
    assert_eq!(rec.replay().ticks, 0);
}

// C. Resume, replay_at, save.

fn offer_cut(n: u8) -> (RunRecorder, u32) {
    let mut rec = RunRecorder::start_party(&trial_setup(), n).unwrap();
    for t in 0..5400 {
        assert_eq!(
            rec.engine().outcome(),
            Outcome::Running,
            "no offer before the end"
        );
        step_party_no_answer(&mut rec, n, t);
        if rec.engine().offer_open() {
            return (rec, t);
        }
    }
    panic!("no offer opened");
}

fn step_party_no_answer(rec: &mut RunRecorder, n: u8, tick: u32) {
    let inputs = inputs_at(rec, n, tick);
    rec.step(&inputs).unwrap();
}

#[test]
fn resume_continues_a_party_run_as_the_uninterrupted_run() {
    for n in [2u8, 3] {
        let live = play(n, 400);
        let cut = live.replay_at(live.engine().tick(), live.engine().state_hash());
        let mut resumed = RunRecorder::resume_with_pack(&cut, pack()).unwrap();
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        let mut live = live;
        for t in 400..460 {
            step_party(&mut live, n, t);
            step_party(&mut resumed, n, t);
        }
        assert_eq!(resumed.replay(), live.replay());
    }
}

#[test]
fn resume_works_with_an_offer_open() {
    for n in [2u8, 3] {
        let (live, _) = offer_cut(n);
        let cut = live.replay();
        let mut resumed = RunRecorder::resume_with_pack(&cut, pack()).unwrap();
        assert!(resumed.engine().offer_open());
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        let mut live = live;
        answer_offers(&mut live, n);
        answer_offers(&mut resumed, n);
        assert_eq!(resumed.replay(), live.replay());
    }
}

fn clipped(replay: &RunReplay, ticks: u32) -> StepRange {
    replay
        .step_range(0, ticks.min(replay.ticks), u32::MAX)
        .unwrap()
}

#[test]
fn replay_at_cuts_a_party_log() {
    for n in [2u8, 3] {
        let rec = play(n, 200);
        let full = rec.replay();
        for t in [0, 100, full.ticks] {
            let at = rec.replay_at(t, 0);
            assert_eq!(at.ticks, t);
            assert_eq!(
                at.step_range(0, t, u32::MAX).unwrap().steps,
                clipped(&full, t).steps,
                "party {n} tick {t}"
            );
        }
        // Past the end: nothing panics and everything recorded is kept.
        let past = rec.replay_at(full.ticks + 1000, 0);
        assert_eq!(past.inputs, full.inputs);
    }
}

#[test]
fn a_party_savepoint_round_trips() {
    for n in [2u8, 3] {
        let live = play(n, 300);
        let prefix = live.replay();
        let bytes = live.save();
        let mut resumed = RunRecorder::resume_from_savepoint(&bytes, &prefix, pack()).unwrap();
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        assert_eq!(
            resumed.engine().player_stats(),
            live.engine().player_stats()
        );
        assert_eq!(resumed.save(), bytes);
        let mut live = live;
        for t in 300..400 {
            step_party(&mut live, n, t);
            step_party(&mut resumed, n, t);
        }
        assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
        assert_eq!(resumed.replay(), live.replay());
    }
}

#[test]
fn a_savepoint_of_one_party_size_against_another_prefix_is_refused() {
    let two = play(2, 200);
    let three = play(3, 200);
    let solo = play(1, 200);
    for (saved, other) in [(&two, &three), (&two, &solo), (&three, &two), (&solo, &two)] {
        let err = RunRecorder::resume_from_savepoint(&saved.save(), &other.replay(), pack())
            .expect_err("refused");
        assert!(matches!(
            err,
            SavepointError::Damaged | SavepointError::PrefixMismatch | SavepointError::HashMismatch
        ));
    }
}

// D. Solo views refuse a party.

#[test]
fn the_solo_views_refuse_a_party() {
    for n in [2u8, 3] {
        let replay = play(n, 100).replay();
        let want = RunReplayError::BadInputs("a party has no solo view");
        assert_eq!(replay.unpack_inputs(u32::MAX).unwrap_err(), want);
        assert_eq!(replay.unpack_commands().unwrap_err(), want);
    }
    let solo = play(1, 100).replay();
    assert_eq!(solo.unpack_inputs(u32::MAX).unwrap().len(), 100);
}

// E. Solo bytes unchanged.

/// Re-records `case` through the recorder from its own steps and picks.
fn rerecord(case: &std::path::Path) -> (RunReplay, RunReplay) {
    let bytes = std::fs::read(case.join("run.replay")).unwrap();
    let recorded = RunReplay::from_bytes(&bytes).unwrap();
    let pack = crate::corpus_pins::case_pack(case).unwrap();
    let mut rec = RunRecorder::start_with_pack(&recorded.setup, &pack).unwrap();
    let range = recorded.step_range(0, recorded.ticks, u32::MAX).unwrap();
    assert_eq!(range.present, 0b1);
    for step in &range.steps {
        match step {
            RangeStep::Actions { actions } => rec
                .actions(&slots(rec.engine().present(), actions))
                .unwrap(),
            RangeStep::Ticks { run, inputs } => {
                for _ in 0..*run {
                    rec.step(&slots(rec.engine().present(), inputs)).unwrap();
                }
            }
            RangeStep::Picks { picks } => rec.picks(&slots(rec.engine().present(), picks)).unwrap(),
            RangeStep::Join { slot: s } => rec.join(slot(*s)).unwrap(),
            RangeStep::Leave { slot: s } => rec.leave(slot(*s)).unwrap(),
        }
    }
    (recorded, rec.replay())
}

#[test]
fn the_recorder_rewrites_every_solo_run_case_as_it_was() {
    let root = crate::data_root::data_root().join("tests/corpus");
    let mut seen = 0;
    for tree in ["current", "v1"] {
        let mut cases: Vec<_> = std::fs::read_dir(root.join(tree).join("runs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        cases.sort();
        for case in cases {
            let (was, now) = rerecord(&case);
            let name = case.display();
            assert_eq!(now.final_hash, was.final_hash, "{name}");
            assert_eq!(now.checkpoints, was.checkpoints, "{name}");
            assert_eq!(now.ticks, was.ticks, "{name}");
            assert_eq!(
                now.unpack_inputs(u32::MAX).unwrap(),
                was.unpack_inputs(u32::MAX).unwrap(),
                "{name}"
            );
            assert_eq!(
                now.unpack_commands().unwrap(),
                was.unpack_commands().unwrap(),
                "{name}"
            );
            if was.setup.schema_version >= 2 {
                assert_eq!(now.inputs, was.inputs, "{name}: the bytes");
            }
            seen += 1;
        }
    }
    assert!(
        seen >= 18,
        "the 9 current and the 9 v1 run cases, saw {seen}"
    );
}

// F. The party cases of the corpus.

/// What each case holds, from the recorder in memory (not from the files): ticks, outcome, picks
/// per slot, Stardust.
const TABLE: [(&str, u32, Outcome, [u32; 3], u32); 14] = [
    ("two_time_limit", 3000, Outcome::Running, [81, 3, 0], 1290),
    ("three_time_limit", 3000, Outcome::Running, [78, 2, 2], 1240),
    ("three_tutorial", 1282, Outcome::Cleared, [10, 1, 1], 0),
    ("join_two", 3000, Outcome::Running, [71, 1, 0], 1190),
    ("leave_hole", 3000, Outcome::Running, [69, 1, 5], 1280),
    ("timed_one", 3000, Outcome::Running, [7, 0, 0], 1080),
    ("timed_two", 3000, Outcome::Running, [8, 0, 0], 950),
    ("equipment_solo", 3000, Outcome::Running, [0, 0, 0], 0),
    ("equipment_two", 3000, Outcome::Running, [0, 0, 0], 0),
    ("equipment_equip", 3000, Outcome::Running, [0, 0, 0], 0),
    ("equipment_levels", 3000, Outcome::Running, [0, 0, 0], 780),
    ("equipment_craft", 3000, Outcome::Running, [0, 0, 0], 0),
    ("two_revive", 3000, Outcome::Running, [58, 4, 0], 870),
    ("three_revive", 3000, Outcome::Running, [53, 4, 5], 920),
];

/// What the rig cases hold of downs and revives: per case the counts of `PlayerDowned`,
/// `PlayerRevived` and `PlayerOut`, the tick of the first down and of the first revive (the
/// engine's tick count after the step), and the final hash. From the seed search, not edited.
const RIG_EVENTS: [(&str, [u32; 3], u32, u32, &str); 2] = [
    ("two_revive", [4, 4, 0], 1084, 1264, "a17bf60ddf475042"),
    ("three_revive", [9, 8, 0], 994, 1174, "ca4db244bab44264"),
];

fn parties_dir() -> std::path::PathBuf {
    crate::data_root::data_root().join("tests/corpus/current/parties")
}

fn picks_per_slot(replay: &RunReplay) -> [u32; 3] {
    let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
    let mut per = [0u32; 3];
    let mut mask = range.present;
    for step in &range.steps {
        match step {
            RangeStep::Actions { .. } => {}
            RangeStep::Picks { picks } => {
                for (s, p) in (0..3).filter(|s| mask >> s & 1 == 1).zip(picks) {
                    per[s] += u32::from(p.is_some());
                }
            }
            RangeStep::Join { slot } => mask |= 1 << slot,
            RangeStep::Leave { slot } => mask &= !(1 << slot),
            RangeStep::Ticks { .. } => {}
        }
    }
    per
}

#[test]
fn the_party_cases_are_what_the_table_says() {
    for case in party_cases() {
        let rec = case.play();
        let replay = rec.replay();
        let verified = verify_run(&replay.to_bytes()).unwrap();
        let got = (
            case.name,
            verified.ticks,
            verified.outcome,
            picks_per_slot(&replay),
            verified.stardust,
        );
        let want = TABLE.iter().find(|row| row.0 == case.name).unwrap();
        assert_eq!(got, *want);
        let per = picks_per_slot(&replay);
        // Equipment mode offers no stars, so slots 1 and 2 answer few offers there.
        if case.changes.is_empty()
            && case.setup.offer_mode == stella_rain_core::run_setup::OfferMode::Stop
            && case.setup.build_mode == stella_rain_core::run_setup::BuildMode::Offers
        {
            assert!(per[1] + per[2] >= 2, "slots 1 and 2 answer offers: {per:?}");
        }
        assert!(replay.ticks <= 3000);
        assert!(replay.to_bytes().len() < 64 * 1024);
    }
    assert_eq!(party_cases().len(), TABLE.len());
}

#[test]
fn the_rig_cases_go_down_and_are_revived() {
    use crate::party_bot::RigRun;
    for (name, counts, first_down, first_revive, hash) in RIG_EVENTS {
        let case = party_cases().into_iter().find(|c| c.name == name).unwrap();
        let replay = case.play().replay();
        assert_eq!(replay.final_hash, hash, "{name}");
        let tally = RigRun::of(&replay).tally(&replay);
        assert!(tally.downed >= 1 && tally.revived >= 1, "{name}");
        assert_eq!(
            [tally.downed, tally.revived, tally.out],
            counts,
            "{name}: downed, revived, out"
        );
        assert_eq!(tally.first_down, Some(first_down), "{name}");
        assert_eq!(tally.first_revive, Some(first_revive), "{name}");
        assert!(tally.revived_were_down, "{name}: a revive follows a down");
        assert!(tally.revived_have_hp, "{name}: a revived player has hp");
        if name == "three_revive" {
            assert!(tally.revivers.iter().any(|r| *r != 0), "{name}");
        }
    }
}

#[test]
fn the_party_case_files_are_the_recording_and_verify() {
    for case in party_cases() {
        let dir = parties_dir().join(case.name);
        let bytes = std::fs::read(dir.join("run.replay")).unwrap();
        let fresh = case.play().replay();
        assert_eq!(
            bytes,
            fresh.to_bytes(),
            "{}: bless is the only fix",
            case.name
        );
        let pin = crate::corpus_pins::case_pack(&dir).unwrap();
        assert_eq!(pin.sha256(), pack().sha256(), "{}", case.name);
        let ok = stella_rain_core::run_replay::verify_run_with_pack(&bytes, pack()).unwrap();
        assert_eq!(ok.ticks, fresh.ticks);
    }
}

/// The recorder at world tick `cut` of `recorded`, driven from its own steps, without the picks
/// made at `cut` (so an offer may be open).
fn recorder_at(recorded: &RunReplay, party: u8, cut: u32) -> RunRecorder {
    let mut rec = RunRecorder::start_party_with_pack(&recorded.setup, pack(), party).unwrap();
    drive(&mut rec, &recorded.step_range(0, cut, u32::MAX).unwrap());
    rec
}

fn drive(rec: &mut RunRecorder, range: &StepRange) {
    for step in &range.steps {
        match step {
            RangeStep::Actions { actions } => rec
                .actions(&slots(rec.engine().present(), actions))
                .unwrap(),
            RangeStep::Ticks { run, inputs } => {
                for _ in 0..*run {
                    rec.step(&slots(rec.engine().present(), inputs)).unwrap();
                }
            }
            RangeStep::Picks { picks } => rec.picks(&slots(rec.engine().present(), picks)).unwrap(),
            RangeStep::Join { slot: s } => rec.join(slot(*s)).unwrap(),
            RangeStep::Leave { slot: s } => rec.leave(slot(*s)).unwrap(),
        }
    }
}

/// Three cuts: early, with an offer open, and one tick before the end. The open one is where
/// slot 1 or 2 holds an offer when that is before the last tick, else the first offer.
fn cuts(recorded: &RunReplay, early: u32) -> [u32; 3] {
    let range = recorded.step_range(0, recorded.ticks, u32::MAX).unwrap();
    let mut world = 0;
    let (mut other, mut any) = (None, None);
    for step in &range.steps {
        match step {
            RangeStep::Actions { .. } => {}
            RangeStep::Ticks { run, .. } => world += run,
            RangeStep::Picks { picks } if world + 1 < recorded.ticks => {
                any.get_or_insert(world);
                if picks[1..].iter().any(Option::is_some) {
                    other.get_or_insert(world);
                }
            }
            RangeStep::Picks { .. } | RangeStep::Join { .. } | RangeStep::Leave { .. } => {}
        }
    }
    [
        early,
        other.or(any).expect("an offer before the end"),
        recorded.ticks - 1,
    ]
}

#[test]
fn the_party_cases_resume_and_save_at_three_cuts() {
    for case in party_cases() {
        let bytes = std::fs::read(parties_dir().join(case.name).join("run.replay")).unwrap();
        let recorded = RunReplay::from_bytes(&bytes).unwrap();
        // A case with changes cuts first just inside its first window.
        let early = case.changes.first().map_or(60, |(at, _)| at + 100);
        for cut in cuts(&recorded, early) {
            let live = recorder_at(&recorded, case.party, cut);
            let prefix = live.replay();
            let rest = recorded.step_range(cut, recorded.ticks, u32::MAX).unwrap();
            let mut by_replay = RunRecorder::resume_with_pack(&prefix, pack()).unwrap();
            let mut by_save =
                RunRecorder::resume_from_savepoint(&live.save(), &prefix, pack()).unwrap();
            drive(&mut by_replay, &rest);
            drive(&mut by_save, &rest);
            for done in [&by_replay, &by_save] {
                let replay = done.replay();
                assert_eq!(
                    replay.final_hash, recorded.final_hash,
                    "{} @{cut}",
                    case.name
                );
                assert_eq!(
                    replay.checkpoints, recorded.checkpoints,
                    "{} @{cut}",
                    case.name
                );
                assert_eq!(replay.to_bytes(), bytes, "{} @{cut}", case.name);
            }
        }
    }
}

#[test]
fn the_change_cases_hold_what_they_are_for() {
    let find = |name: &str| {
        let case = party_cases().into_iter().find(|c| c.name == name).unwrap();
        case.play().replay()
    };
    // join_two: slot 1 answers an offer after its join at tick 600.
    let join = find("join_two");
    let range = join.step_range(0, join.ticks, u32::MAX).unwrap();
    let (mut world, mut mask, mut after) = (0u32, range.present, 0);
    for step in &range.steps {
        match step {
            RangeStep::Ticks { run, .. } => world += run,
            RangeStep::Join { slot } => {
                assert_eq!((*slot, world), (1, 600));
                mask |= 1 << slot;
            }
            RangeStep::Picks { picks } if mask == 0b011 && picks[1].is_some() => {
                assert!(world > 600);
                after += 1;
            }
            _ => {}
        }
    }
    assert!(after >= 1);
    // leave_hole: a Picks step of length 2 falls in the first window (0b101), the hole is
    // closed by the second leave and reopened by the join.
    let hole = find("leave_hole");
    let range = hole.step_range(0, hole.ticks, u32::MAX).unwrap();
    let (mut world, mut masks, mut two) = (0u32, vec![range.present], 0);
    for step in &range.steps {
        match step {
            RangeStep::Actions { .. } => unreachable!("no equipment action here"),
            RangeStep::Ticks { run, .. } => world += run,
            RangeStep::Join { slot } => masks.push(masks.last().unwrap() | 1 << slot),
            RangeStep::Leave { slot } => masks.push(masks.last().unwrap() & !(1 << slot)),
            RangeStep::Picks { picks } => {
                if picks.len() == 2 && (900..=1500).contains(&world) {
                    two += 1;
                }
            }
        }
    }
    assert_eq!(masks, [0b111, 0b101, 0b001, 0b101]);
    assert!(two >= 1, "a Picks step inside the hole holds two entries");
}

#[test]
fn the_timed_cases_close_offers_by_the_timer_and_by_a_pick() {
    use stella_rain_core::event::DomainEvent;
    for case in party_cases()
        .into_iter()
        .filter(|c| c.setup.offer_mode == stella_rain_core::run_setup::OfferMode::Timed)
    {
        let replay = case.play().replay();
        let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
        let mut rec = RunRecorder::start_party(&case.setup, case.party).unwrap();
        let mut taken = 0usize;
        let mut count = |rec: &RunRecorder| {
            taken += rec
                .engine()
                .events()
                .iter()
                .filter(|ev| matches!(ev, DomainEvent::RewardTaken { .. }))
                .count();
        };
        for step in &range.steps {
            match step {
                RangeStep::Actions { .. } => unreachable!("no equipment action here"),
                RangeStep::Ticks { run, inputs } => {
                    for _ in 0..*run {
                        rec.step(&slots(rec.engine().present(), inputs)).unwrap();
                        count(&rec);
                    }
                }
                RangeStep::Picks { picks } => {
                    rec.picks(&slots(rec.engine().present(), picks)).unwrap();
                    count(&rec);
                }
                RangeStep::Join { .. } | RangeStep::Leave { .. } => unreachable!(),
            }
        }
        let picked = picks_per_slot(&replay).iter().sum::<u32>() as usize;
        assert!(picked >= 1, "{}: a pick closed an offer", case.name);
        assert!(
            taken > picked,
            "{}: the timer closed an offer ({taken} taken, {picked} picked)",
            case.name
        );
        assert_eq!(rec.replay().to_bytes(), replay.to_bytes(), "{}", case.name);
    }
}

// D. A slot that leaves and joins with no step between.

#[test]
fn a_leave_and_a_join_of_one_slot_with_no_step_between_record_and_verify() {
    let mut rec = RunRecorder::start_party(&trial_setup(), 2).unwrap();
    for t in 0..3 {
        step_party(&mut rec, 2, t);
    }
    rec.leave(PlayerIdx::new(1).unwrap()).unwrap();
    rec.join(PlayerIdx::new(1).unwrap()).unwrap();
    for t in 3..6 {
        step_party(&mut rec, 2, t);
    }
    let verified = verify_run(&rec.replay().to_bytes()).unwrap();
    assert_eq!(verified.final_hash, rec.engine().state_hash());
}

#[test]
fn a_savepoint_after_a_rejoin_resumes_to_the_same_hashes() {
    let slot1 = PlayerIdx::new(1).unwrap();
    let mut live = RunRecorder::start_party(&trial_setup(), 2).unwrap();
    for t in 0..120 {
        step_party(&mut live, 2, t);
    }
    live.leave(slot1).unwrap();
    live.join(slot1).unwrap();
    for t in 120..150 {
        step_party(&mut live, 2, t);
    }
    let prefix = live.replay();
    let mut resumed = RunRecorder::resume_from_savepoint(&live.save(), &prefix, pack()).unwrap();
    assert_eq!(resumed.engine().state_hash(), live.engine().state_hash());
    for t in 150..300 {
        step_party(&mut live, 2, t);
        step_party(&mut resumed, 2, t);
        assert_eq!(
            resumed.engine().state_hash(),
            live.engine().state_hash(),
            "tick {t}"
        );
    }
    assert_eq!(resumed.replay().to_bytes(), live.replay().to_bytes());
}
