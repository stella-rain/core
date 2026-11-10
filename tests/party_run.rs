//! A party of 2 and 3 recorded natively: the engine starts it, the recorder steps and picks per
//! player, and record, resume, `replay_at`, save and verify agree (ADR-REPL-2 point 4,
//! ADR-REPL-5, ADR-SAVE-4). Slots 1 and 2 act in the rules (ADR-SIM-7), so the party cases pin
//! what their play does to the run.

use crate::common::run::trial_setup;
use crate::common::run_steps::{RunSteps, Step, apply_to};
use crate::run_play::party_cases;
use crate::zone_bot::run_input;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::{RunRecorder, RunReplay, RunReplayError, verify_run};
use stella_rain_core::savepoint::SavepointError;
use stella_rain_core::{RangeStep, StepRange};

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

fn step_party(rec: &mut RunRecorder, n: u8, tick: u32) {
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
    let rec = RunRecorder::start_party(&trial_setup(), 2).unwrap();
    assert_eq!(rec.engine().tick(), 0);
    assert_eq!(rec.replay().ticks, 0);
}

// C. Resume, replay_at, save.

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
    let steps = RunSteps::of(&recorded);
    assert_eq!(steps.party, 1);
    for step in steps.steps() {
        apply_to(&mut rec, step);
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
            // The steps carry picks and actions, which the solo views refuse in an equipment run.
            assert_eq!(RunSteps::of(&now), RunSteps::of(&was), "{name}");
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
const TABLE: [(&str, u32, Outcome, [u32; 3], u32); 19] = [
    ("two_time_limit", 3000, Outcome::Running, [0, 0, 0], 1040),
    ("three_time_limit", 3000, Outcome::Running, [0, 0, 0], 1140),
    ("three_tutorial", 4420, Outcome::Cleared, [0, 0, 0], 0),
    ("join_two", 3000, Outcome::Running, [0, 0, 0], 890),
    ("leave_hole", 3000, Outcome::Running, [0, 0, 0], 990),
    ("equipment_solo", 3000, Outcome::Running, [0, 0, 0], 0),
    ("equipment_two", 3000, Outcome::Running, [0, 0, 0], 0),
    ("equipment_equip", 3000, Outcome::Running, [0, 0, 0], 0),
    ("equipment_levels", 3000, Outcome::Running, [0, 0, 0], 890),
    ("equipment_craft", 3000, Outcome::Running, [0, 0, 0], 0),
    ("equipment_lumine", 3000, Outcome::Running, [0, 0, 0], 890),
    ("equipment_party_1", 3000, Outcome::Running, [0, 0, 0], 720),
    ("equipment_party_2", 3000, Outcome::Running, [0, 0, 0], 770),
    ("equipment_party_3", 3000, Outcome::Running, [0, 0, 0], 740),
    ("equipment_rig_1", 3000, Outcome::Running, [0, 0, 0], 1020),
    ("equipment_rig_2", 3000, Outcome::Running, [0, 0, 0], 1040),
    ("equipment_rig_3", 3000, Outcome::Running, [0, 0, 0], 1140),
    ("two_revive", 3000, Outcome::Running, [0, 0, 0], 1040),
    ("three_revive", 3000, Outcome::Running, [0, 0, 0], 1100),
];

/// The equipment actions each of the seven moved cases holds per slot (Some entries of its ACTIONS
/// steps). From the blessed files, not edited.
const ACTIONS: [(&str, [u32; 3]); 7] = [
    ("two_time_limit", [51, 56, 0]),
    ("three_time_limit", [67, 70, 67]),
    ("three_tutorial", [3, 3, 3]),
    ("join_two", [49, 48, 0]),
    ("leave_hole", [64, 14, 45]),
    ("two_revive", [51, 56, 0]),
    ("three_revive", [56, 59, 55]),
];

fn actions_per_slot(replay: &RunReplay) -> [u32; 3] {
    let mut per = [0u32; 3];
    for step in RunSteps::of(replay).steps() {
        if let Step::Actions(actions) = step {
            for (n, action) in per.iter_mut().zip(actions) {
                *n += u32::from(action.is_some());
            }
        }
    }
    per
}

/// What the rig cases hold of downs and revives: per case the counts of `PlayerDowned`,
/// `PlayerRevived` and `PlayerOut`, the tick of the first down and of the first revive (the
/// engine's tick count after the step), and the final hash. From the seed search, not edited.
const RIG_EVENTS: [(&str, [u32; 3], u32, u32, &str); 2] = [
    ("two_revive", [2, 2, 0], 2074, 2254, "35b4c256b1da2087"),
    ("three_revive", [3, 3, 0], 2074, 2254, "365e06c6e25687c9"),
];

/// What the building rig cases hold: per case and slot the actions the bot made, as `[equip,
/// replace, extract, craft, tree_pick]`. From the seed search, not edited.
const RIG_BUILDS: [(&str, [[u32; 5]; 3]); 3] = [
    ("equipment_rig_1", [[12, 1, 47, 3, 1], [0; 5], [0; 5]]),
    (
        "equipment_rig_2",
        [[12, 0, 37, 1, 1], [11, 0, 41, 3, 1], [0; 5]],
    ),
    (
        "equipment_rig_3",
        [[12, 2, 50, 2, 1], [11, 1, 54, 4, 0], [12, 2, 50, 2, 1]],
    ),
];

fn parties_dir() -> std::path::PathBuf {
    crate::data_root::data_root().join("tests/corpus/current/parties")
}

fn picks_per_slot(replay: &RunReplay) -> [u32; 3] {
    let mut per = [0u32; 3];
    for step in RunSteps::of(replay).steps() {
        if let Step::Picks(picks) = step {
            for (n, pick) in per.iter_mut().zip(picks) {
                *n += u32::from(pick.is_some());
            }
        }
    }
    per
}

/// How many equipment actions slots 1 and 2 made in all.
fn others_actions(steps: &RunSteps) -> usize {
    steps
        .steps()
        .iter()
        .map(|s| match s {
            Step::Actions(a) => a[1..].iter().flatten().count(),
            _ => 0,
        })
        .sum()
}

/// The first promise of the party case `name` that `replay` misses, if any (`None` is a hit).
/// Every case: the run verifies, at most 3000 ticks, under 64 KiB. An offers-era case with no joins
/// or leaves: slots 1 and 2 answer at least 2 offers. An equipment case (ADR-SIM-7) by its name:
/// `two_time_limit` and `three_time_limit` run on at 3000 ticks and slots 1 and 2 make at least 2
/// actions, `three_tutorial` clears and the same, `join_two` takes slot 1 at tick 600 and slot 1
/// acts after it, `leave_hole` has the masks 0b111, 0b101, 0b001, 0b101 and slot 2 acts inside the
/// first hole (900..=1500), `two_revive` and `three_revive` are `equipment_revive_miss`. A name
/// starting `equipment_` adds no promise of its own; any other name is a miss, so a case added to the
/// table needs its promise here.
pub(crate) fn party_promise_miss(name: &str, replay: &RunReplay) -> Option<String> {
    let bytes = replay.to_bytes();
    let Ok(verified) = verify_run(&bytes) else {
        return Some("the run verifies".into());
    };
    // The tutorial is played to its own time limit, the rest to 3000 ticks.
    let cap = if name == "three_tutorial" {
        crate::run_play::TUTORIAL_TICKS
    } else {
        3000
    };
    if replay.ticks > cap {
        return Some(format!("at most {cap} ticks, not {}", replay.ticks));
    }
    if bytes.len() >= 64 * 1024 {
        return Some("the file is under 64 KiB".into());
    }
    let steps = RunSteps::of(replay);
    let changes = steps
        .steps()
        .iter()
        .any(|s| matches!(s, Step::Join(_) | Step::Leave(_)));
    if replay.setup.sim_version == 1 {
        if !changes {
            let per = picks_per_slot(replay);
            if per[1] + per[2] < 2 {
                return Some(format!("slots 1 and 2 answer offers: {per:?}"));
            }
        }
        return None;
    }
    let running = |outcome: Outcome| {
        if replay.ticks == 3000 && outcome == Outcome::Running {
            None
        } else {
            Some(format!(
                "Running at 3000 ticks, not {outcome:?} at {}",
                replay.ticks
            ))
        }
    };
    let others = others_actions(&steps);
    match name {
        "two_time_limit" | "three_time_limit" => {
            if let Some(miss) = running(verified.outcome) {
                return Some(miss);
            }
            (others < 2).then(|| format!("slots 1 and 2 make at least 2 actions, not {others}"))
        }
        "three_tutorial" => {
            if verified.outcome != Outcome::Cleared {
                return Some(format!("Cleared, not {:?}", verified.outcome));
            }
            (others < 2).then(|| format!("slots 1 and 2 make at least 2 actions, not {others}"))
        }
        "join_two" => {
            if let Some(miss) = running(verified.outcome) {
                return Some(miss);
            }
            let (mut world, mut mask, mut joined, mut after) = (0u32, 0b1u8, None, false);
            for step in steps.steps() {
                match step {
                    Step::Tick(_) => world += 1,
                    Step::Join(slot) => {
                        joined = Some((slot.get(), world));
                        mask |= 1u8 << slot.get();
                    }
                    Step::Actions(a) if mask == 0b011 && a[1].is_some() && world > 600 => {
                        after = true;
                    }
                    _ => {}
                }
            }
            if joined != Some((1, 600)) {
                return Some(format!("slot 1 joins at tick 600, not {joined:?}"));
            }
            (!after).then(|| "slot 1 acts after its join".to_string())
        }
        "leave_hole" => {
            if let Some(miss) = running(verified.outcome) {
                return Some(miss);
            }
            let (mut world, mut masks, mut acts) = (0u32, vec![(1u8 << steps.party) - 1], false);
            for step in steps.steps() {
                match step {
                    Step::Tick(_) => world += 1,
                    Step::Join(slot) => masks.push(masks.last().unwrap() | 1u8 << slot.get()),
                    Step::Leave(slot) => masks.push(masks.last().unwrap() & !(1u8 << slot.get())),
                    Step::Actions(a)
                        if a[2].is_some()
                            && *masks.last().unwrap() == 0b101
                            && (900..=1500).contains(&world) =>
                    {
                        acts = true;
                    }
                    _ => {}
                }
            }
            if masks != [0b111, 0b101, 0b001, 0b101] {
                return Some(format!("the masks 7, 5, 1, 5, not {masks:?}"));
            }
            (!acts).then(|| "slot 2 acts inside the first hole".to_string())
        }
        "two_revive" | "three_revive" => {
            crate::party_bot::equipment_revive_miss(replay, steps.party).map(str::to_string)
        }
        n if n.starts_with("equipment_") => None,
        other => Some(format!("no promise for the case `{other}`")),
    }
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
        assert_eq!(
            party_promise_miss(case.name, &replay),
            None,
            "{}",
            case.name
        );
        if let Some(want) = ACTIONS.iter().find(|row| row.0 == case.name) {
            assert_eq!(actions_per_slot(&replay), want.1, "{}: actions", case.name);
            assert!(RunSteps::of(&replay).has_actions(), "{}", case.name);
        }
    }
    assert_eq!(party_cases().len(), TABLE.len());
}

/// The first tick after 1260 (the last bat spawn) at which no enemy is on the field in the run of
/// `replay`; `None` when the run ends first.
pub(crate) fn first_empty_field(
    case: &crate::run_play::PartyCase,
    replay: &RunReplay,
) -> Option<u32> {
    use crate::party_bot::RigRun;
    use stella_rain_core::snapshot::EntityKind;
    let run = RigRun::of(replay);
    let mut e = Engine::start_party_with(&case.setup, Pack::bundled().content(), run.n).unwrap();
    let mut empty = None;
    let len = u32::try_from(run.ticks.len()).unwrap();
    run.play_with(&mut e, 0, len, |e| {
        if empty.is_none()
            && e.tick() > 1260
            && !e
                .snapshot()
                .entities
                .iter()
                .any(|v| v.kind == EntityKind::Enemy)
        {
            empty = Some(e.tick());
        }
    });
    empty
}

#[test]
fn the_tutorial_party_clears_with_its_build_out_of_it() {
    for (world, bot) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
        let case = crate::run_play::party_case("three_tutorial", world, bot);
        let replay = case.play().replay();
        let verified = verify_run(&replay.to_bytes()).unwrap();
        assert_eq!(verified.outcome, Outcome::Cleared, "({world}, {bot})");
        // The bots walk to the boss while the field is empty, even before the first bat: when the
        // boss dies before the field is empty after the last spawn, the bound starts at that spawn.
        let empty = first_empty_field(&case, &replay).unwrap_or(1260);
        // The golem comes onto the field at the arrival tick, so no clear can come before it.
        const ARRIVAL: u32 = 4_000;
        assert!(
            verified.ticks <= empty.max(ARRIVAL) + 2000,
            "({world}, {bot}): cleared at {}, field empty at {empty}",
            verified.ticks
        );
        assert!(
            verified.ticks > ARRIVAL,
            "({world}, {bot}): cleared at {} before the golem came",
            verified.ticks
        );
    }
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
fn the_building_rig_cases_hold_their_predicates() {
    use crate::party_bot::{RigRun, rig_builds_miss};
    for (n, (name, want)) in (1u8..).zip(RIG_BUILDS) {
        let case = party_cases().into_iter().find(|c| c.name == name).unwrap();
        let replay = case.play().replay();
        assert_eq!(rig_builds_miss(&replay, n), None, "{name}");
        let got = RigRun::of(&replay)
            .actions_per_slot()
            .map(|c| [c.equip, c.replace, c.extract, c.craft, c.tree_pick]);
        assert_eq!(got, want, "{name}: actions per slot");
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
    let steps = RunSteps::of(recorded);
    drive(&mut rec, &steps.steps()[..steps.at(cut)]);
    rec
}

fn drive(rec: &mut RunRecorder, steps: &[Step]) {
    for step in steps {
        apply_to(rec, step);
    }
}

/// Three cuts: early, with a step pending, and one tick before the end. The middle one is where
/// slot 1 or 2 holds an offer or makes an action when that is before the last tick, else the first
/// such step, else (none) the middle of the run.
fn cuts(recorded: &RunReplay, early: u32) -> [u32; 3] {
    let steps = RunSteps::of(recorded);
    let mut world = 0;
    let (mut other, mut any) = (None, None);
    for step in steps.steps() {
        match step {
            Step::Tick(_) => world += 1,
            Step::Picks(picks) if world + 1 < recorded.ticks => {
                any.get_or_insert(world);
                if picks[1..].iter().any(Option::is_some) {
                    other.get_or_insert(world);
                }
            }
            // An equipment case opens no offer (ADR-SIM-7): its open cut is an action by slot 1 or 2.
            Step::Actions(actions) if world + 1 < recorded.ticks => {
                any.get_or_insert(world);
                if actions[1..].iter().any(Option::is_some) {
                    other.get_or_insert(world);
                }
            }
            _ => {}
        }
    }
    [
        early,
        other.or(any).unwrap_or(recorded.ticks / 2),
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
            let all = RunSteps::of(&recorded);
            let rest = &all.steps()[all.at(cut)..];
            let mut by_replay = RunRecorder::resume_with_pack(&prefix, pack()).unwrap();
            let mut by_save =
                RunRecorder::resume_from_savepoint(&live.save(), &prefix, pack()).unwrap();
            drive(&mut by_replay, rest);
            drive(&mut by_save, rest);
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
    // join_two: slot 1 acts after its join at tick 600.
    let join = find("join_two");
    let steps = RunSteps::of(&join);
    let (mut world, mut mask, mut after) = (0u32, (1u8 << steps.party) - 1, 0);
    for step in steps.steps() {
        match step {
            Step::Tick(_) => world += 1,
            Step::Join(slot) => {
                assert_eq!((slot.get(), world), (1usize, 600));
                mask |= 1u8 << slot.get();
            }
            Step::Actions(actions) if mask == 0b011 && actions[1].is_some() => {
                assert!(world > 600);
                after += 1;
            }
            _ => {}
        }
    }
    assert!(after >= 1, "slot 1 makes an action after its join");
    // leave_hole: slot 2 acts inside the first hole (0b101, 900..=1500); the hole is closed by
    // the second leave and reopened by the join.
    let hole = find("leave_hole");
    let steps = RunSteps::of(&hole);
    let (mut world, mut masks, mut two) = (0u32, vec![(1u8 << steps.party) - 1], 0);
    for step in steps.steps() {
        match step {
            Step::Tick(_) => world += 1,
            Step::Join(slot) => masks.push(masks.last().unwrap() | 1u8 << slot.get()),
            Step::Leave(slot) => masks.push(masks.last().unwrap() & !(1u8 << slot.get())),
            Step::Actions(actions) => {
                if *masks.last().unwrap() == 0b101
                    && actions[2].is_some()
                    && (900..=1500).contains(&world)
                {
                    two += 1;
                }
            }
            Step::Picks(_) => unreachable!("no offer opens in equipment mode"),
        }
    }
    assert_eq!(masks, [0b111, 0b101, 0b001, 0b101]);
    assert!(two >= 1, "slot 2 acts inside the hole");
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

// G. The promises report what they miss.

/// A name no promise knows is a miss, not a hit; an equipment case that adds no promise of its own
/// and an offers-era case (which returns before the name is read) stay hits.
#[test]
fn an_unknown_case_name_is_a_miss() {
    use crate::run_play::party_case;
    let play = |case: crate::run_play::PartyCase| case.play().replay();
    let two = play(party_case("two_time_limit", 1, 1));
    let miss = party_promise_miss("no_such_case", &two).expect("an unknown name");
    assert!(
        miss.contains("no promise for the case `no_such_case`"),
        "{miss}"
    );
    assert_eq!(party_promise_miss("equipment_party_2", &two), None);
    let join = party_cases()
        .into_iter()
        .find(|case| case.name == "join_two")
        .expect("the join_two case")
        .play()
        .replay();
    assert_eq!(party_promise_miss("join_two", &join), None);
}

#[test]
fn the_party_promises_name_what_a_run_misses() {
    use crate::party_bot::{RigParty, equipment_revive_miss};
    use crate::run_play::{Change, party_case};
    let play = |case: crate::run_play::PartyCase| case.play().replay();
    // A run that keeps only the common promises is a hit under a name that adds none.
    let two = play(party_case("two_time_limit", 1, 1));
    assert_eq!(party_promise_miss("equipment_party_2", &two), None);
    // The same runs under promises they cannot meet name the miss.
    let three = play(party_case("three_time_limit", 1, 1));
    let tutorial = party_promise_miss("three_tutorial", &three).expect("a Running run");
    assert!(tutorial.contains("Cleared"), "{tutorial}");
    let hole = party_promise_miss("leave_hole", &two).expect("no leave");
    assert!(hole.contains("masks") || hole.contains("Running"), "{hole}");
    let late = play(
        RigParty::new(1)
            .equipment()
            .builds()
            .seed(1)
            .bot_seed(1)
            .changes(vec![(700, Change::Join(1))])
            .case("join_two"),
    );
    let joined = party_promise_miss("join_two", &late).expect("joined at 700");
    assert!(
        joined.contains("tick 600") || joined.contains("Running"),
        "{joined}"
    );
    // A run that fails early is no Running run: join_two (1, 1) ends Failed before 3000 ticks.
    let failed = play(party_case("join_two", 1, 1));
    assert_ne!(failed.ticks, 3000, "the run this test needs fails early");
    let miss = party_promise_miss("join_two", &failed).expect("not Running at 3000");
    assert!(miss.contains("Running at 3000"), "{miss}");
    // A run with no down is no revive case.
    let solo = play(
        RigParty::new(2)
            .equipment()
            .builds()
            .seed(1)
            .bot_seed(1)
            .ticks(60)
            .case("two_revive"),
    );
    assert_eq!(equipment_revive_miss(&solo, 2), Some("a down"));
}
