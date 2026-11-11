//! Tests of the run replay.

use super::{ActionError, RunReplayError};
use super::{RunRecorder, hash_content, verify_run};
use crate::engine::Engine;
use crate::input::EquipAction;
use crate::input::Input;
use crate::run_setup::RunSetup;

const ATTACK_PRESETS_SHA: [u8; 32] = [14; 32];
const STATS: [u8; 32] = [7; 32];
const REWARDS: [u8; 32] = [9; 32];
const SKILLS: [u8; 32] = [5; 32];
const EXPERTISE: [u8; 32] = [3; 32];
const MATCH_UPS: [u8; 32] = [1; 32];
const RUNES: [u8; 32] = [2; 32];
const PERKS: [u8; 32] = [4; 32];
const DIFFICULTIES: [u8; 32] = [6; 32];
const WEAPONS_SHA: [u8; 32] = [8; 32];
const RULES_SHA: [u8; 32] = [12; 32];

fn hash(mode: &str, file: &[u8], attack_presets: &[u8; 32]) -> [u8; 32] {
    hash_content(
        mode,
        file,
        &STATS,
        &REWARDS,
        &SKILLS,
        &EXPERTISE,
        &MATCH_UPS,
        &RUNES,
        &PERKS,
        &DIFFICULTIES,
        &WEAPONS_SHA,
        &RULES_SHA,
        None,
        attack_presets,
    )
}

fn files(
    stats: &[u8; 32],
    rewards: &[u8; 32],
    skills: &[u8; 32],
    expertise: &[u8; 32],
) -> [u8; 32] {
    hash_content(
        "trial",
        b"{}",
        stats,
        rewards,
        skills,
        expertise,
        &MATCH_UPS,
        &RUNES,
        &PERKS,
        &DIFFICULTIES,
        &WEAPONS_SHA,
        &RULES_SHA,
        None,
        &ATTACK_PRESETS_SHA,
    )
}

#[test]
fn the_content_hash_covers_the_mode_the_content_files_and_the_attack_presets() {
    let base = hash("trial", b"{}", &ATTACK_PRESETS_SHA);
    assert_eq!(base, hash("trial", b"{}", &ATTACK_PRESETS_SHA));
    assert_ne!(
        base,
        hash("trial", b"{ }", &ATTACK_PRESETS_SHA),
        "the mode's file"
    );
    assert_ne!(
        base,
        hash("other", b"{}", &ATTACK_PRESETS_SHA),
        "the mode's ID"
    );
    assert_eq!(base, files(&STATS, &REWARDS, &SKILLS, &EXPERTISE));
    assert_ne!(
        base,
        files(&[8; 32], &REWARDS, &SKILLS, &EXPERTISE),
        "the run stats"
    );
    assert_ne!(
        base,
        files(&STATS, &[8; 32], &SKILLS, &EXPERTISE),
        "the rewards"
    );
    assert_ne!(
        base,
        files(&STATS, &REWARDS, &[8; 32], &EXPERTISE),
        "the stage skills"
    );
    assert_ne!(
        base,
        files(&STATS, &REWARDS, &SKILLS, &[8; 32]),
        "the expertise"
    );
    assert_ne!(
        base,
        hash_content(
            "trial",
            b"{}",
            &STATS,
            &REWARDS,
            &SKILLS,
            &EXPERTISE,
            &[8; 32],
            &RUNES,
            &PERKS,
            &DIFFICULTIES,
            &WEAPONS_SHA,
            &RULES_SHA,
            None,
            &ATTACK_PRESETS_SHA,
        ),
        "the match-ups"
    );
    assert_ne!(
        base,
        hash_content(
            "trial",
            b"{}",
            &STATS,
            &REWARDS,
            &SKILLS,
            &EXPERTISE,
            &MATCH_UPS,
            &[8; 32],
            &PERKS,
            &DIFFICULTIES,
            &WEAPONS_SHA,
            &RULES_SHA,
            None,
            &ATTACK_PRESETS_SHA,
        ),
        "the runes"
    );
    assert_ne!(
        base,
        hash_content(
            "trial",
            b"{}",
            &STATS,
            &REWARDS,
            &SKILLS,
            &EXPERTISE,
            &MATCH_UPS,
            &RUNES,
            &[8; 32],
            &DIFFICULTIES,
            &WEAPONS_SHA,
            &RULES_SHA,
            None,
            &ATTACK_PRESETS_SHA,
        ),
        "the perk sets"
    );
    assert_ne!(
        base,
        hash_content(
            "trial",
            b"{}",
            &STATS,
            &REWARDS,
            &SKILLS,
            &EXPERTISE,
            &MATCH_UPS,
            &RUNES,
            &PERKS,
            &[8; 32],
            &WEAPONS_SHA,
            &RULES_SHA,
            None,
            &ATTACK_PRESETS_SHA,
        ),
        "the difficulties"
    );
    assert_ne!(
        base,
        hash_content(
            "trial",
            b"{}",
            &STATS,
            &REWARDS,
            &SKILLS,
            &EXPERTISE,
            &MATCH_UPS,
            &RUNES,
            &PERKS,
            &DIFFICULTIES,
            &[11; 32],
            &RULES_SHA,
            None,
            &ATTACK_PRESETS_SHA,
        ),
        "the weapons"
    );
    assert_ne!(
        base,
        hash_content(
            "trial",
            b"{}",
            &STATS,
            &REWARDS,
            &SKILLS,
            &EXPERTISE,
            &MATCH_UPS,
            &RUNES,
            &PERKS,
            &DIFFICULTIES,
            &WEAPONS_SHA,
            &[13; 32],
            None,
            &ATTACK_PRESETS_SHA,
        ),
        "the rules"
    );
    // Sets of named stars change it, and no file leaves it as it was.
    let with = |sets: Option<&[u8; 32]>| {
        hash_content(
            "trial",
            b"{}",
            &STATS,
            &REWARDS,
            &SKILLS,
            &EXPERTISE,
            &MATCH_UPS,
            &RUNES,
            &PERKS,
            &DIFFICULTIES,
            &WEAPONS_SHA,
            &RULES_SHA,
            sets,
            &ATTACK_PRESETS_SHA,
        )
    };
    assert_eq!(base, with(None));
    assert_ne!(base, with(Some(&[9; 32])), "the constellations");
    assert_ne!(with(Some(&[9; 32])), with(Some(&[10; 32])));
    assert_ne!(
        base,
        files(&REWARDS, &STATS, &SKILLS, &EXPERTISE),
        "which file is which"
    );
    assert_ne!(
        base,
        files(&STATS, &SKILLS, &REWARDS, &EXPERTISE),
        "which file is which"
    );
    assert_ne!(
        base,
        files(&STATS, &REWARDS, &EXPERTISE, &SKILLS),
        "which file is which"
    );
    assert_ne!(base, hash("trial", b"{}", &[15; 32]), "the attack presets");
}

#[test]
fn replay_at_reads_nothing_from_the_engine() {
    let setup = RunSetup {
        seed: 20_261_010,
        ..RunSetup::default()
    };
    let mut r = RunRecorder::start(&setup).unwrap();
    for _ in 0..130 {
        r.step(&[Input::default(), Default::default(), Default::default()])
            .unwrap();
    }
    let hash = r.engine().state_hash();
    let bytes_before = r.replay().to_bytes();

    // A recorder whose engine is unrelated to the run: a fresh one at tick 0.
    let mut bad = r.clone();
    bad.engine = Engine::start(&setup).unwrap();
    assert_eq!(bad.replay().ticks, 0, "replay() reads the engine");

    let replay = bad.replay_at(130, hash);
    assert!(
        replay.to_bytes() == bytes_before,
        "the replay is the one the good recorder gave"
    );
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.ticks, 130);
    assert_eq!(verified.final_hash, hash);
}

/// T23: a replay whose ACTIONS step the engine refuses is `ActionRefused` at that tick, with
/// the refusal's own message; a step with no action is still a damaged log.
#[test]
fn a_replay_with_a_refused_equip_names_the_refusal() {
    use crate::run_replay::Record;
    let setup = RunSetup {
        ..RunSetup::default()
    };
    let mut rec = RunRecorder::start(&setup).unwrap();
    rec.step(&[Input::default(); 3]).unwrap();
    rec.records.push(Record::Actions {
        actions: vec![Some(EquipAction::Equip { item: 0, slot: 0 })],
    });
    let refused = RunReplayError::ActionRefused {
        tick: 1,
        error: ActionError::NoItem { item: 0 },
    };
    assert_eq!(
        refused.to_string(),
        "an equipment action on tick 1 names inventory item 0, which is not in the inventory"
    );
    assert_eq!(verify_run(&rec.replay().to_bytes()).unwrap_err(), refused);
    rec.records.pop();
    rec.records.push(Record::Actions {
        actions: vec![None],
    });
    assert_eq!(
        verify_run(&rec.replay().to_bytes()).unwrap_err(),
        RunReplayError::BadInputs("a record has no action")
    );
}

/// L6: the two lock refusals read as the plan names them.
#[test]
fn the_lock_refusals_read_well() {
    let say = |error| RunReplayError::ActionRefused { tick: 7, error }.to_string();
    assert_eq!(
        say(ActionError::Downed),
        "an equipment action on tick 7 is made by a downed player, whose equipment window \
             is locked until a teammate revives it"
    );
    assert_eq!(
        say(ActionError::Out),
        "an equipment action on tick 7 is made by a player who is out of the run"
    );
}

/// L7: the recorder's `step` drops a downed member's action and goes on; `actions` refuses it.
#[test]
fn the_recorder_drops_a_downed_action_and_refuses_it_direct() {
    use crate::engine::Run;
    let setup = RunSetup {
        ..RunSetup::default()
    };
    let mut rec = RunRecorder::start_party(&setup, 2).unwrap();
    if let Run::V2 { state, .. } = rec.engine.run_mut() {
        let one = crate::id::PlayerIdx::new(1).unwrap();
        state.players[one].hp = 0;
    }
    let act = EquipAction::Equip { item: 0, slot: 5 };
    let acting = Input {
        action: Some(act),
        ..Input::default()
    };
    rec.step(&[Input::default(), acting, Input::default()])
        .unwrap();
    assert_eq!(rec.engine().tick(), 1, "the run went on");
    assert!(
        rec.records
            .iter()
            .all(|r| !matches!(r, super::Record::Actions { .. })),
        "no ACTIONS step was recorded"
    );
    assert_eq!(
        rec.actions(&[None, Some(act), None]).unwrap_err(),
        RunReplayError::ActionRefused {
            tick: 1,
            error: ActionError::Downed
        }
    );
}

/// L8: a replay forged with an ACTIONS step of a downed member does not verify.
#[test]
fn a_replay_with_an_action_of_a_downed_player_does_not_verify() {
    use crate::run_replay::Record;
    let setup = RunSetup {
        seed: 20_261_010,
        ..RunSetup::default()
    };
    let mut rec = RunRecorder::start_party(&setup, 2).unwrap();
    let walk = Input {
        move_x: 100,
        ..Input::default()
    };
    let one = crate::id::PlayerIdx::new(1).unwrap();
    while rec.engine().snapshot().players[1].down.is_none() {
        assert!(rec.engine().tick() < 4_000, "slot 1 goes down");
        rec.step(&[walk, Input::default(), Input::default()])
            .unwrap();
    }
    assert!(
        rec.engine()
            .check_action(one, EquipAction::Extract { item: 0 })
            .is_err()
    );
    rec.records.push(Record::Actions {
        actions: vec![None, Some(EquipAction::Extract { item: 0 })],
    });
    assert_eq!(
        verify_run(&rec.replay().to_bytes()).unwrap_err(),
        RunReplayError::ActionRefused {
            tick: rec.engine().tick(),
            error: ActionError::Downed
        }
    );
}

fn v1_pack() -> crate::pack::Pack {
    crate::pack::Pack::from_bytes(include_bytes!(
            "../tests/corpus/v1/packs/32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5.json"
        ))
        .expect("the version 1 pack")
}

#[test]
fn a_version_1_recorder_refuses_interact() {
    let frozen = RunSetup {
        schema_version: 1,
        sim_version: 1,
        ..RunSetup::default()
    };
    let mut rec = RunRecorder::start_with_pack(&frozen, &v1_pack()).unwrap();
    rec.step(&[Input::default(); 3]).unwrap();
    let (replay, tick) = (rec.replay(), rec.engine().tick());
    let mut held = [Input::default(); 3];
    held[0].interact = true;
    assert_eq!(
        rec.step(&held),
        Err(RunReplayError::BadInputs("a version 1 run has no interact"))
    );
    assert_eq!((rec.replay(), rec.engine().tick()), (replay, tick));
    // An absent slot's entry is ignored, as it is at every schema.
    let mut absent = [Input::default(); 3];
    absent[1].interact = true;
    assert_eq!(rec.step(&absent), Ok(()));
    // The live rules take the same input.
    let mut live = RunRecorder::start(&RunSetup::default()).unwrap();
    assert_eq!(live.step(&held), Ok(()));
}

#[test]
fn a_replay_with_an_action_step_loads_and_ranges_but_does_not_verify() {
    use crate::pack::Pack;
    use crate::run_replay::Record;
    use crate::step_log::RangeStep;
    // The frozen rules have no equipment: every action is refused.
    let frozen = RunSetup {
        schema_version: 1,
        sim_version: 1,
        ..RunSetup::default()
    };
    // The frozen rules play the pack they were released with, not the bundled files.
    let v1 = Pack::from_bytes(include_bytes!(
            "../tests/corpus/v1/packs/32ec7296e6d6391476878f95571c434f7a15ed74236d26988dd8d88573bc3dd5.json"
        ))
        .expect("the version 1 pack");
    let mut rec = RunRecorder::start_with_pack(&frozen, &v1).unwrap();
    rec.step(&[Input::default(); 3]).unwrap();
    let mut replay = rec.replay();
    rec.records.push(Record::Actions {
        actions: vec![Some(EquipAction::Extract { item: 0 })],
    });
    // A schema 1 file cannot hold an action: the replay is forged at schema 2, with the log
    // written by hand.
    replay.setup.schema_version = 2;
    replay.inputs = crate::base64::encode(
        &crate::step_log::StepLog {
            present: 0b1,
            records: rec.records.clone(),
        }
        .encode()
        .unwrap(),
    );
    replay.commands = String::new();
    let refused = RunReplayError::ActionRefused {
        tick: 1,
        error: ActionError::NotBuilt,
    };
    assert_eq!(
        refused.to_string(),
        "an equipment action on tick 1 is not built yet"
    );
    assert_eq!(
        crate::run_replay::verify_run_with(&replay.to_bytes(), v1.content()).unwrap_err(),
        refused
    );
    let range = replay.step_range(0, 1, 1_000_000).unwrap();
    assert!(
        range
            .steps
            .iter()
            .any(|s| matches!(s, RangeStep::Actions { .. }))
    );
    assert_eq!(
        RunRecorder::resume_with_pack(&replay, &v1).err(),
        Some(refused)
    );
}

mod party_size_tests {
    use crate::base64;
    use crate::input::Input;
    use crate::run_replay::RunRecorder;
    use crate::run_setup::RunSetup;

    fn setup() -> RunSetup {
        RunSetup {
            seed: 20_261_010,
            ..RunSetup::default()
        }
    }

    #[test]
    fn the_size_is_the_present_byte() {
        for n in 1..=3u8 {
            let mut rec = RunRecorder::start_party(&setup(), n).unwrap();
            rec.step(&[Input::default(); 3]).unwrap();
            assert_eq!(rec.replay().party_size(), Some(n));
        }
    }

    #[test]
    fn a_byte_that_is_not_a_party_has_no_size() {
        let mut rec = RunRecorder::start_party(&setup(), 2).unwrap();
        rec.step(&[Input::default(); 3]).unwrap();
        let mut replay = rec.replay();
        let bytes = base64::decode(&replay.inputs).unwrap();
        for bad in [0u8, 0b010, 0b101, 0b1000, 0b1111, 0xFF] {
            let mut edited = bytes.clone();
            edited[0] = bad;
            replay.inputs = crate::base64::encode(&edited);
            assert_eq!(replay.party_size(), None, "{bad:#b}");
        }
        replay.inputs = String::new();
        assert_eq!(replay.party_size(), None);
        replay.inputs = "!!!!".into();
        assert_eq!(replay.party_size(), None);
    }

    #[test]
    fn a_schema_1_replay_is_one_player() {
        let mut rec = RunRecorder::start(&setup()).unwrap();
        rec.step(&[Input::default(), Default::default(), Default::default()])
            .unwrap();
        let mut replay = rec.replay();
        replay.setup.schema_version = 1;
        assert_eq!(replay.party_size(), Some(1));
    }
}
