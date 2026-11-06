//! Equip and replace in play, in the recorder and in a replay (ADR-SIM-7, ADR-REPL-5): a run
//! that has dropped items gets an action made between two ticks, the recorder applies and
//! records it, a replay applies it again and ends on the recorded hash, and a savepoint cut after
//! it resumes to the straight run's hashes.

use crate::common::run::trial_setup;
use crate::run_play::PartyCase;
use stella_rain_core::RangeStep;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::pack::Pack;
use stella_rain_core::run_replay::{RunRecorder, verify_run};
use stella_rain_core::run_setup::{BuildMode, RunSetup};

/// The slots of the layout a drop can go to: the five Attack, two Defence, two Mobility and the
/// three rune slots.
const SLOTS: u32 = 12;

fn case(seed: u64, ticks: u32) -> PartyCase {
    PartyCase {
        name: "equipment_equip",
        setup: RunSetup {
            mode: "tutorial".into(),
            seed,
            build_mode: BuildMode::Equipment,
            ..trial_setup()
        },
        party: 1,
        ticks,
        changes: vec![],
        answer_after: None,
        bot: None,
        script: None,
    }
}

fn inventory(e: &Engine) -> usize {
    e.snapshot().players[0].inventory.len()
}

/// The first action of `make` the engine accepts for slot 0: items and slots in order.
fn first(e: &Engine, make: fn(u32, u32) -> EquipAction) -> Option<EquipAction> {
    let items = u32::try_from(inventory(e)).unwrap();
    (0..items)
        .flat_map(|item| (0..SLOTS + 1).map(move |slot| make(item, slot)))
        .find(|a| e.check_action(PlayerIdx::SOLO, *a).is_ok())
}

fn equip(item: u32, slot: u32) -> EquipAction {
    EquipAction::Equip { item, slot }
}

fn replace(item: u32, slot: u32) -> EquipAction {
    EquipAction::Replace { item, slot }
}

fn acting(a: EquipAction) -> [Input; 3] {
    [
        Input {
            action: Some(a),
            ..Input::default()
        },
        Input::default(),
        Input::default(),
    ]
}

/// A recording after `ticks` ticks of a seed whose inventory lets an equip and then a replace
/// through, with the two actions.
fn recorded() -> (RunRecorder, EquipAction, EquipAction) {
    for seed in 1..=60u64 {
        let rec = case(seed, 3000).play();
        if rec.engine().outcome() != Outcome::Running {
            continue;
        }
        let Some(a) = first(rec.engine(), equip) else {
            continue;
        };
        let mut probe = rec.clone();
        probe.step(&acting(a)).unwrap();
        if let Some(b) = first(probe.engine(), replace) {
            return (rec, a, b);
        }
    }
    panic!("no seed of 1..=60 drops an item that equips and one that replaces it");
}

fn actions_in(rec: &RunRecorder) -> usize {
    let replay = rec.replay();
    let range = replay.step_range(0, replay.ticks, 1_000_000).unwrap();
    range
        .steps
        .iter()
        .filter(|s| matches!(s, RangeStep::Actions { .. }))
        .count()
}

/// T22: the recorder applies and records an accepted action; a refused one records nothing.
#[test]
fn the_recorder_applies_and_records_an_accepted_action() {
    let (mut rec, a, _) = recorded();
    let before = inventory(rec.engine());
    rec.step(&acting(a)).unwrap();
    assert_eq!(
        inventory(rec.engine()),
        before - 1,
        "the item left the inventory"
    );
    assert_eq!(actions_in(&rec), 1);
    // A refused action is ignored like a refused pick: the tick runs, nothing is recorded.
    let refused = acting(equip(9_999, 0));
    rec.step(&refused).unwrap();
    assert_eq!(actions_in(&rec), 1);
    assert_eq!(inventory(rec.engine()), before - 1);
}

/// T23: a replay applies the actions and ends on the recorded hash and the same inventory.
#[test]
fn a_replay_applies_actions_and_ends_on_the_recorded_hash() {
    let (mut rec, a, b) = recorded();
    rec.step(&acting(a)).unwrap();
    for _ in 0..60 {
        rec.step(&[Input::default(); 3]).unwrap();
    }
    assert_eq!(rec.engine().check_action(PlayerIdx::SOLO, b), Ok(()));
    rec.step(&acting(b)).unwrap();
    for _ in 0..60 {
        rec.step(&[Input::default(); 3]).unwrap();
    }
    assert_eq!(actions_in(&rec), 2);
    let replay = rec.replay();
    let verified = verify_run(&replay.to_bytes()).expect("the replay verifies");
    assert_eq!(verified.final_hash, rec.engine().state_hash());
}

/// T24: a run resumed from a savepoint after the equips matches the straight run's hashes.
#[test]
fn a_resumed_equipment_run_matches_the_straight_run() {
    let (mut live, a, b) = recorded();
    live.step(&acting(a)).unwrap();
    for _ in 0..60 {
        live.step(&[Input::default(); 3]).unwrap();
    }
    live.step(&acting(b)).unwrap();
    let prefix = live.replay();
    let saved = live.save();
    let pack = Pack::bundled();
    let mut by_savepoint = RunRecorder::resume_from_savepoint(&saved, &prefix, pack).unwrap();
    let mut by_replay = RunRecorder::resume_with_pack(&prefix, pack).unwrap();
    for t in 0..240 {
        assert_eq!(
            by_savepoint.engine().state_hash(),
            live.engine().state_hash(),
            "{t}"
        );
        assert_eq!(
            by_replay.engine().state_hash(),
            live.engine().state_hash(),
            "{t}"
        );
        for rec in [&mut live, &mut by_savepoint, &mut by_replay] {
            rec.step(&[Input::default(); 3]).unwrap();
        }
    }
    assert_eq!(by_savepoint.replay(), live.replay());
}

/// The slots of player 0 as the snapshot's JSON shows them.
fn slots_json(e: &Engine) -> Vec<serde_json::Value> {
    let player = serde_json::to_value(&e.snapshot().players[0]).unwrap();
    player
        .get("slots")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default()
}

/// T26: the snapshot lists the 12 slots in order and the Lumine slot last, with what each holds;
/// an offers run shows none.
#[test]
fn the_snapshot_shows_the_slots() {
    let setup = RunSetup {
        build_mode: BuildMode::Equipment,
        starting_rune: Some("thunder".into()),
        lumine: 1,
        lumine_star: Some("max_hp_common".into()),
        pick_stats: false,
        ..RunSetup::default()
    };
    let e = Engine::start(&setup).unwrap();
    let slots = slots_json(&e);
    assert_eq!(slots.len(), 13);
    let kinds: Vec<String> = slots
        .iter()
        .map(|s| s["kind"]["kind"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(kinds[..9], ["category"; 9]);
    assert_eq!(kinds[9..12], ["rune"; 3]);
    assert_eq!(kinds[12], "lumine");
    let index: Vec<u64> = slots[..9]
        .iter()
        .map(|s| s["kind"]["index"].as_u64().unwrap())
        .collect();
    assert_eq!(index, [0, 0, 0, 0, 0, 1, 1, 2, 2]);
    assert!(slots[0].get("held").is_none());
    assert_eq!(slots[0]["level"], 0);
    assert_eq!(
        slots[9]["held"],
        serde_json::json!({ "kind": "rune", "rune": "thunder" })
    );
    assert_eq!(slots[9]["level"], 1);
    assert_eq!(
        slots[12]["held"],
        serde_json::json!({ "kind": "star", "id": "max_hp_common", "rarity": "common" })
    );
    assert_eq!(slots[12]["expertise"], 0);

    // After an equip the slot shows the star.
    let (mut rec, a, _) = recorded();
    let EquipAction::Equip { item, slot } = a else {
        panic!("an equip")
    };
    let id =
        serde_json::to_value(&rec.engine().snapshot().players[0].inventory[item as usize]).unwrap();
    rec.step(&acting(a)).unwrap();
    let now = slots_json(rec.engine());
    assert_eq!(now[slot as usize]["held"], id);
    assert_eq!(now[slot as usize]["level"], 1);

    // An offers run has no slots key at all.
    let offers = Engine::start(&RunSetup::default()).unwrap();
    let json = serde_json::to_value(&offers.snapshot().players[0]).unwrap();
    assert!(json.get("slots").is_none());

    // The generated schema describes the view.
    let schema = include_str!("../schema/content.schema.json");
    assert!(schema.contains("\"SlotView\""));
}

/// T19: the snapshot shows a slot's level and expertise after kills raised it.
#[test]
fn the_snapshot_shows_level_and_expertise() {
    let mut early = crate::run_play::equipment_levels("x", crate::run_play::EQUIP_LEVELS_SEED);
    early.ticks = 2700;
    let rec = early.play();
    let view = &rec.engine().snapshot().players[0].slots;
    let json = slots_json(rec.engine());
    let at = view
        .iter()
        .position(|s| s.level >= 2)
        .expect("a star slot has risen");
    assert_eq!(json[at]["level"], view[at].level);
    assert_eq!(json[at]["expertise"], view[at].expertise);
    // The kills are at most the need of the level (the curve is code, so only the bound).
    assert!(view[at].level <= 10);
}

/// T20: a run resumed from a savepoint cut while a slot holds expertise matches the straight run.
#[test]
fn a_run_resumed_mid_expertise_matches_the_straight_run() {
    let mut early = crate::run_play::equipment_levels("x", crate::run_play::EQUIP_LEVELS_SEED);
    early.ticks = 1600;
    let mut live = early.play();
    let expertise = |r: &RunRecorder| -> u32 {
        r.engine().snapshot().players[0]
            .slots
            .iter()
            .map(|s| s.expertise + u32::from(s.level))
            .sum()
    };
    assert!(expertise(&live) > 2, "a star slot holds kills");
    let prefix = live.replay();
    let saved = live.save();
    let pack = Pack::bundled();
    let mut by_savepoint = RunRecorder::resume_from_savepoint(&saved, &prefix, pack).unwrap();
    for t in 0..600 {
        assert_eq!(
            by_savepoint.engine().state_hash(),
            live.engine().state_hash(),
            "{t}"
        );
        let inputs = crate::run_play::party_inputs_with(&live, 1600 + t, None);
        for rec in [&mut live, &mut by_savepoint] {
            rec.step(&inputs).unwrap();
        }
    }
}

mod search {
    use super::*;
    use crate::run_play::equipment_equip;
    use stella_rain_core::input::EquipAction;
    use stella_rain_core::snapshot::{HeldView, SlotKindView};

    /// The seed of the `equipment_equip` corpus case: world seeds 1..=60, solo, 3000 ticks, the
    /// script of `EquipScript`; the first seed whose run holds a star in a category slot and a rune
    /// in a rune slot, records at least two ACTIONS steps and makes a replace.
    #[test]
    #[ignore = "a seed search, run by hand"]
    fn search_equipment_equip_seeds() {
        for seed in 1..=60u64 {
            let rec = equipment_equip("x", seed).play();
            let snap = rec.engine().snapshot();
            let slots = &snap.players[0].slots;
            let star = slots.iter().any(|s| {
                matches!(s.kind, SlotKindView::Category { .. })
                    && matches!(s.held, Some(HeldView::Star { .. }))
            });
            let rune = slots.iter().any(|s| {
                matches!(s.kind, SlotKindView::Rune)
                    && matches!(s.held, Some(HeldView::Rune { .. }))
            });
            let replay = rec.replay();
            let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
            let actions: Vec<EquipAction> = range
                .steps
                .iter()
                .filter_map(|s| match s {
                    RangeStep::Actions { actions } => Some(actions.iter().flatten().copied()),
                    _ => None,
                })
                .flatten()
                .collect();
            let replaced = actions
                .iter()
                .any(|a| matches!(a, EquipAction::Replace { .. }));
            println!(
                "seed={seed} star={star} rune={rune} actions={} replace={replaced} ticks={} hash={}",
                actions.len(),
                replay.ticks,
                replay.final_hash
            );
        }
    }

    /// The seed of the `equipment_levels` corpus case: world seeds 1..=60, solo, 3000 ticks, the
    /// script of `equipment_levels`; the first seed where a category slot holds Lv2 or more
    /// before the late replace and a lower level after it.
    #[test]
    #[ignore = "a seed search, run by hand"]
    fn search_equipment_levels_seeds() {
        use crate::run_play::equipment_levels;
        let levels = |rec: &stella_rain_core::run_replay::RunRecorder| -> Vec<u8> {
            rec.engine().snapshot().players[0].slots[..9]
                .iter()
                .map(|s| s.level)
                .collect()
        };
        for seed in 1..=60u64 {
            let mut early = equipment_levels("x", seed);
            early.ticks = 2700;
            let (before, after) = (
                levels(&early.play()),
                levels(&equipment_levels("x", seed).play()),
            );
            let dropped = before.iter().zip(&after).any(|(b, a)| *b >= 2 && a < b);
            let rec = equipment_levels("x", seed).play();
            let ex: Vec<u32> = rec.engine().snapshot().players[0].slots[..9]
                .iter()
                .map(|s| s.expertise)
                .collect();
            if dropped {
                println!("seed={seed} before={before:?} after={after:?} ex={ex:?}");
                break;
            }
        }
    }

    /// The seed of the `equipment_craft` corpus case: world seeds 1..=60, solo, 3000 ticks, the
    /// script of `equipment_craft`; the first seed whose run records at least one Extract and one
    /// Craft among its ACTIONS steps and ends with the crafted star in the inventory.
    #[test]
    #[ignore = "a seed search, run by hand"]
    fn search_equipment_craft_seeds() {
        use crate::run_play::equipment_craft;
        for seed in 1..=60u64 {
            let rec = equipment_craft("x", seed).play();
            let replay = rec.replay();
            let range = replay.step_range(0, replay.ticks, 1_000_000).unwrap();
            let actions: Vec<EquipAction> = range
                .steps
                .iter()
                .filter_map(|s| match s {
                    RangeStep::Actions { actions } => Some(actions.iter().flatten().copied()),
                    _ => None,
                })
                .flatten()
                .collect();
            let extracts = actions
                .iter()
                .filter(|a| matches!(a, EquipAction::Extract { .. }))
                .count();
            let crafts = actions
                .iter()
                .filter(|a| matches!(a, EquipAction::Craft { .. }))
                .count();
            if extracts >= 1 && crafts >= 1 {
                println!(
                    "seed={seed} extracts={extracts} crafts={crafts} outcome={:?} ticks={} hash={}",
                    rec.engine().outcome(),
                    replay.ticks,
                    replay.final_hash
                );
                break;
            }
        }
    }
}

// --- extract and craft (ADR-SIM-7, ADR-REPL-5) -------------------------------------------------

/// The first craft the engine accepts for slot 0.
fn craft_first(e: &Engine) -> Option<EquipAction> {
    (0..256)
        .map(|star| EquipAction::Craft { star })
        .find(|a| e.check_action(PlayerIdx::SOLO, *a).is_ok())
}

/// A recording after 3000 ticks of a seed whose inventory, extracted one item at a time from
/// the front, pays for a craft, with the actions that do it (the extracts, then the craft).
fn extract_then_craft() -> (RunRecorder, Vec<EquipAction>) {
    for seed in 1..=60u64 {
        let rec = case(seed, 3000).play();
        if rec.engine().outcome() != Outcome::Running {
            continue;
        }
        let mut probe = rec.clone();
        let mut actions = Vec::new();
        loop {
            if let Some(c) = craft_first(probe.engine()) {
                actions.push(c);
                // The run must outlive the actions and the idle ticks the test adds.
                let lives = probe.step(&acting(c)).is_ok()
                    && (0..62).all(|_| probe.step(&[Input::default(); 3]).is_ok());
                if lives {
                    return (rec, actions);
                }
                break;
            }
            if inventory(probe.engine()) == 0 {
                break;
            }
            let a = EquipAction::Extract { item: 0 };
            if probe.step(&acting(a)).is_err() {
                break;
            }
            actions.push(a);
        }
    }
    panic!("no seed of 1..=60 drops items that extract into a craft");
}

/// T16: the recorder applies and records extracts and a craft as ACTIONS steps, an action the
/// engine refuses is ignored, and a replay verifies to the recorded hash.
#[test]
fn the_recorder_and_the_replay_make_extract_and_craft() {
    let (mut rec, actions) = extract_then_craft();
    let before = inventory(rec.engine());
    let extracts = actions.len() - 1;
    for a in &actions {
        rec.step(&acting(*a)).unwrap();
    }
    assert_eq!(inventory(rec.engine()), before - extracts + 1);
    assert_eq!(actions_in(&rec), actions.len());
    // A refused action is ignored: the tick runs, nothing is recorded.
    rec.step(&acting(EquipAction::Craft { star: 99_999 }))
        .unwrap();
    rec.step(&acting(EquipAction::Extract { item: 99_999 }))
        .unwrap();
    assert_eq!(actions_in(&rec), actions.len());
    for _ in 0..60 {
        rec.step(&[Input::default(); 3]).unwrap();
    }
    let verified = verify_run(&rec.replay().to_bytes()).expect("the replay verifies");
    assert_eq!(verified.final_hash, rec.engine().state_hash());
}

/// T19: a run resumed from a savepoint cut after an extract and a craft matches the straight
/// run's hashes (the codec carries Starlight and the inventory).
#[test]
fn a_run_resumed_after_extract_and_craft_matches_the_straight_run() {
    let (mut live, actions) = extract_then_craft();
    for a in &actions {
        live.step(&acting(*a)).unwrap();
    }
    let prefix = live.replay();
    let saved = live.save();
    let pack = Pack::bundled();
    let mut resumed = RunRecorder::resume_from_savepoint(&saved, &prefix, pack).unwrap();
    for t in 0..60 {
        assert_eq!(
            resumed.engine().state_hash(),
            live.engine().state_hash(),
            "{t}"
        );
        for rec in [&mut live, &mut resumed] {
            rec.step(&[Input::default(); 3]).unwrap();
        }
    }
    assert_eq!(resumed.replay(), live.replay());
}

// --- the weapon tree pick (ADR-SIM-7, ADR-REPL-5) ---------------------------------------------------

/// `equipment_levels` stopped at `ticks`: its gauge has passed level 3 by tick 1600.
fn tree_run(ticks: u32) -> RunRecorder {
    let mut case = crate::run_play::equipment_levels("x", crate::run_play::EQUIP_LEVELS_SEED);
    case.ticks = ticks;
    case.script = case
        .script
        .map(|s| crate::run_play::EquipScript { tree_at: None, ..s });
    case.play()
}

const TREE_TICK: u32 = 1600;

fn tree_pick(option: u32) -> EquipAction {
    EquipAction::TreePick { option }
}

/// T18: the recorder applies and records an accepted tree pick and refuses a forged one; a replay
/// ends on the recorded hash.
#[test]
fn the_recorder_and_the_replay_make_the_tree_pick() {
    use stella_rain_core::engine::ActionError;
    use stella_rain_core::run_replay::RunReplayError;
    let mut rec = tree_run(TREE_TICK);
    assert_eq!(
        rec.engine().check_action(PlayerIdx::SOLO, tree_pick(0)),
        Ok(()),
        "the run holds a tree point"
    );
    let before = actions_in(&rec);
    let err = rec.actions(&[Some(tree_pick(99)), None, None]).unwrap_err();
    assert!(matches!(
        err,
        RunReplayError::ActionRefused {
            error: ActionError::NoTreeOption { option: 99, .. },
            ..
        }
    ));
    assert_eq!(actions_in(&rec), before);
    rec.step(&acting(tree_pick(0))).unwrap();
    assert_eq!(actions_in(&rec), before + 1);
    assert!(rec.engine().snapshot().expertise.unwrap().style.is_some());
    for _ in 0..60 {
        rec.step(&[Input::default(); 3]).unwrap();
    }
    let verified = verify_run(&rec.replay().to_bytes()).expect("the replay verifies");
    assert_eq!(verified.final_hash, rec.engine().state_hash());
}

/// T22: a run resumed from a savepoint cut with a point held, the pick made after, matches the
/// straight run's hashes (format 5 carries the gauge; the point is derived).
#[test]
fn a_resumed_run_matches_the_straight_run_through_a_tree_pick() {
    let mut live = tree_run(TREE_TICK);
    let prefix = live.replay();
    let saved = live.save();
    let pack = Pack::bundled();
    let mut resumed = RunRecorder::resume_from_savepoint(&saved, &prefix, pack).unwrap();
    for rec in [&mut live, &mut resumed] {
        rec.step(&acting(tree_pick(0))).unwrap();
    }
    for t in 0..240 {
        assert_eq!(
            resumed.engine().state_hash(),
            live.engine().state_hash(),
            "{t}"
        );
        for rec in [&mut live, &mut resumed] {
            rec.step(&[Input::default(); 3]).unwrap();
        }
    }
    assert_eq!(resumed.replay(), live.replay());
}

/// T24: the `equipment_levels` recording holds a tree pick among its ACTIONS steps and ends with a
/// style chosen.
#[test]
fn equipment_levels_records_a_tree_pick() {
    let rec = crate::run_play::equipment_levels("x", crate::run_play::EQUIP_LEVELS_SEED).play();
    let replay = rec.replay();
    let range = replay.step_range(0, replay.ticks, u32::MAX).unwrap();
    let picks = range
        .steps
        .iter()
        .filter_map(|s| match s {
            RangeStep::Actions { actions } => Some(actions.iter().flatten().copied()),
            _ => None,
        })
        .flatten()
        .filter(|a| matches!(a, EquipAction::TreePick { .. }))
        .count();
    assert!(picks >= 1, "{picks}");
    assert!(rec.engine().snapshot().expertise.unwrap().style.is_some());
}

// --- the Lumine slot (ADR-SAVE-7, ADR-SIM-3, ADR-REPL-5) ---------------------------------------

const LUMINE_SLOT: u32 = 12;

fn lumine_setup(seed: u64) -> RunSetup {
    RunSetup {
        lumine: 4,
        lumine_star: Some("star_mintaka".into()),
        ..case(seed, 0).setup
    }
}

fn lumine_case(seed: u64, ticks: u32) -> PartyCase {
    PartyCase {
        setup: lumine_setup(seed),
        ..case(seed, ticks)
    }
}

/// The first `Replace` into the Lumine slot the engine accepts.
fn lumine_replace(e: &Engine) -> Option<EquipAction> {
    let items = u32::try_from(inventory(e)).unwrap();
    (0..items)
        .map(|item| replace(item, LUMINE_SLOT))
        .find(|a| e.check_action(PlayerIdx::SOLO, *a).is_ok())
}

/// A recording of 3000 ticks whose inventory lets a replace of the Lumine star through, with it.
fn lumine_recorded() -> (RunRecorder, EquipAction) {
    for seed in 1..=60u64 {
        let rec = lumine_case(seed, 3000).play();
        if rec.engine().outcome() != Outcome::Running {
            continue;
        }
        if let Some(a) = lumine_replace(rec.engine()) {
            return (rec, a);
        }
    }
    panic!("no seed of 1..=60 drops a star that replaces the Lumine star");
}

/// T6 (Lumine): every member of a party starts with the Lumine star at Lv1, as the solo run does.
#[test]
fn every_member_starts_with_the_lumine_star() {
    let setup = lumine_setup(7);
    let solo = Engine::start(&setup).unwrap();
    let party = Engine::start_party(&setup, 3).unwrap();
    let snap = party.snapshot();
    for (i, p) in snap.players.iter().enumerate() {
        let player = serde_json::to_value(p).unwrap();
        let slot = &player["slots"][12];
        assert_eq!(slot["kind"]["kind"], "lumine", "player {i}");
        assert_eq!(slot["held"]["id"], "star_mintaka", "player {i}");
        assert_eq!(slot["level"], 1, "player {i}");
        assert_eq!(p.hp, solo.snapshot().players[0].hp, "player {i}");
    }
}

/// T7 (Lumine): a run resumed from a savepoint cut after a replace of the Lumine star matches the
/// straight run's hashes.
#[test]
fn a_resumed_lumine_run_matches_the_straight_run() {
    let (mut live, a) = lumine_recorded();
    live.step(&acting(a)).unwrap();
    assert_eq!(
        serde_json::to_value(&live.engine().snapshot().players[0])
            .unwrap()
            .get("slots")
            .unwrap()[12]["level"],
        1
    );
    let prefix = live.replay();
    let saved = live.save();
    let pack = Pack::bundled();
    let mut resumed = RunRecorder::resume_from_savepoint(&saved, &prefix, pack).unwrap();
    for t in 0..120 {
        assert_eq!(
            resumed.engine().state_hash(),
            live.engine().state_hash(),
            "{t}"
        );
        for rec in [&mut live, &mut resumed] {
            rec.step(&[Input::default(); 3]).unwrap();
        }
    }
    assert_eq!(resumed.replay(), live.replay());
    let verified = verify_run(&live.replay().to_bytes()).expect("the replay verifies");
    assert_eq!(verified.final_hash, live.engine().state_hash());
}
