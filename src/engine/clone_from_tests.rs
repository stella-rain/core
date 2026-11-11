//! `Clone::clone_from` on the engine and its mutable state copies exactly what `clone` does, into a
//! slot that already holds an engine, whatever that slot held.

use super::*;
use crate::bench_fixtures::{equip_case, party_case};
use crate::id::PlayerIdx;

/// The engines the copy is tried on: the three-player equipment run and the dense party minute,
/// both at tick 10,800.
fn cases() -> Vec<(&'static str, Engine)> {
    vec![
        ("equip", equip_case::saved()),
        ("party", party_case::saved()),
    ]
}

fn step_n(engine: &mut Engine, ticks: u32) {
    for _ in 0..ticks {
        engine.step(&party_case::minute_inputs(engine.tick()));
    }
}

/// Everything an engine shows from outside, as one string, so a mismatch names the field.
fn shown(engine: &Engine) -> String {
    format!(
        "hash {} tick {} present {} snapshot {:?} events {:?} caps {:?} ledger {:?} stats {:?}",
        engine.state_hash(),
        engine.tick(),
        engine.present(),
        engine.snapshot(),
        engine.events(),
        engine.caps(),
        engine.ledger(),
        engine.player_stats(),
    )
}

/// `slot` and `reference` step 20 ticks on the same inputs and agree at every one.
fn same_for_20_ticks(slot: &mut Engine, reference: &mut Engine, why: &str) {
    for _ in 0..20 {
        let inputs = party_case::minute_inputs(reference.tick());
        slot.step(&inputs);
        reference.step(&inputs);
        assert_eq!(
            slot.state_hash(),
            reference.state_hash(),
            "{why}: tick {}",
            reference.tick()
        );
    }
    assert_eq!(shown(slot), shown(reference), "{why}: after 20 ticks");
}

#[test]
fn clone_from_equals_clone_and_steps_the_same() {
    for (name, engine) in cases() {
        // A slot made by a clone and stepped on, so that every list differs in length.
        let mut slot = engine.clone();
        step_n(&mut slot, 30);
        assert_ne!(slot.state_hash(), engine.state_hash(), "{name}: differs");
        slot.clone_from(&engine);
        let mut reference = engine.clone();
        assert_eq!(shown(&slot), shown(&reference), "{name}: copied");
        same_for_20_ticks(&mut slot, &mut reference, name);
    }
}

#[test]
fn clone_from_copies_a_shorter_and_a_longer_source() {
    for (name, engine) in cases() {
        let mut earlier = engine.clone();
        let mut later = engine.clone();
        step_n(&mut later, 30);
        // The slot is the engine itself: the source is 30 ticks behind, then 30 ticks ahead.
        let mut slot = engine.clone();
        step_n(&mut slot, 15);
        for (source, how) in [(&engine, "behind"), (&later, "ahead")] {
            slot.clone_from(source);
            assert_eq!(shown(&slot), shown(source), "{name}: source {how}");
        }
        let mut slot = later.clone();
        slot.clone_from(&engine);
        assert_eq!(shown(&slot), shown(&engine), "{name}: back to the start");
        same_for_20_ticks(&mut slot, &mut earlier, name);
    }
}

#[test]
fn clone_from_restores_presence() {
    let one = PlayerIdx::new(1).unwrap();
    let two = PlayerIdx::new(2).unwrap();
    for (name, engine) in cases() {
        let mut slot = engine.clone();
        slot.leave(two).expect("slot 2 leaves");
        step_n(&mut slot, 5);
        slot.leave(one).expect("slot 1 leaves");
        slot.join(two).expect("slot 2 rejoins");
        assert_ne!(slot.present(), engine.present(), "{name}: the mask moved");
        slot.clone_from(&engine);
        let mut reference = engine.clone();
        assert_eq!(slot.present(), reference.present(), "{name}: the mask");
        assert_eq!(shown(&slot), shown(&reference), "{name}: presence copied");
        same_for_20_ticks(&mut slot, &mut reference, name);

        // And the other way: the source is the one that left and rejoined.
        let mut source = engine.clone();
        source.leave(two).expect("slot 2 leaves");
        step_n(&mut source, 5);
        source.join(two).expect("slot 2 rejoins");
        let mut slot = engine.clone();
        slot.clone_from(&source);
        assert_eq!(shown(&slot), shown(&source), "{name}: source's presence");
        same_for_20_ticks(&mut slot, &mut source, name);
    }
}

fn solo(sim_version: u32) -> Engine {
    let setup = RunSetup {
        sim_version,
        mode: "time_limit".into(),
        seed: 7,
        ..RunSetup::default()
    };
    // The frozen rules play the pack they were released with.
    if sim_version == 1 {
        Engine::start_with(&setup, super::copy_tests::v1_pack().content())
    } else {
        Engine::start(&setup)
    }
    .expect("the run starts")
}

#[test]
fn clone_from_across_rules_versions() {
    let mut frozen = solo(1);
    let mut live = solo(2);
    step_n(&mut frozen, 40);
    step_n(&mut live, 25);
    assert_eq!(frozen.version(), 1);
    assert_eq!(live.version(), 2);

    // V1 from V1, stepped differently.
    let mut slot = solo(1);
    step_n(&mut slot, 10);
    slot.clone_from(&frozen);
    let mut reference = frozen.clone();
    assert_eq!(shown(&slot), shown(&reference), "v1 from v1");
    same_for_20_ticks(&mut slot, &mut reference, "v1 from v1");

    // V2 into a V1 slot, and V1 into a V2 slot.
    let mut slot = frozen.clone();
    slot.clone_from(&live);
    assert_eq!(slot.version(), 2);
    let mut reference = live.clone();
    assert_eq!(shown(&slot), shown(&reference), "v2 into v1");
    same_for_20_ticks(&mut slot, &mut reference, "v2 into v1");

    let mut slot = live.clone();
    slot.clone_from(&frozen);
    assert_eq!(slot.version(), 1);
    let mut reference = frozen.clone();
    assert_eq!(shown(&slot), shown(&reference), "v1 into v2");
    same_for_20_ticks(&mut slot, &mut reference, "v1 into v2");
}

#[test]
fn the_source_of_a_copy_is_unchanged() {
    for (name, engine) in cases() {
        let before = shown(&engine);
        let mut slot = engine.clone();
        step_n(&mut slot, 30);
        slot.clone_from(&engine);
        step_n(&mut slot, 10);
        assert_eq!(shown(&engine), before, "{name}");
    }
}

/// Every heap-owning type of the engine's mutable state writes `clone` and `clone_from` by hand,
/// each field named: a field added later does not compile until the copy handles it.
#[test]
fn every_heap_owning_type_names_each_field_in_its_copy() {
    let state = [
        include_str!("../sim/state.rs"),
        include_str!("../sim/state/status.rs"),
        include_str!("../sim/state/orbs_zones.rs"),
        include_str!("../sim/state/attack_run.rs"),
    ]
    .concat();
    let state = state.as_str();
    let types: [(&str, &str); 21] = [
        ("State", state),
        ("Horde", state),
        ("ExpertiseGauge", state),
        ("StarField", state),
        ("Offers", state),
        ("Progress", state),
        ("Career", state),
        ("Players", state),
        ("PlayerState", state),
        ("BossState", state),
        ("AgentState", state),
        ("ShrineState", state),
        ("ZoneState", state),
        ("AttackRun", state),
        ("MoveIndex", state),
        ("Equipment", include_str!("../sim/equipment.rs")),
        ("Scratch", include_str!("../sim/scratch.rs")),
        ("RunLedger", include_str!("../ledger.rs")),
        ("Counted", include_str!("../ledger.rs")),
        ("Run", include_str!("run.rs")),
        ("Engine", include_str!("mod.rs")),
    ];
    for (name, source) in types {
        let head = format!("impl Clone for {name} {{");
        let start = source
            .find(&head)
            .unwrap_or_else(|| panic!("`{head}` is not written by hand"));
        let body = &source[start..];
        let end = body.find("\n}\n").expect("the impl ends");
        let body = &body[..end];
        assert!(
            body.contains("fn clone_from("),
            "{name} has no `clone_from`"
        );
        assert!(body.contains("fn clone("), "{name} has no `clone`");
        for line in body.lines() {
            let code = line.split("//").next().unwrap_or("");
            for rest in [", ..", " .. }", "..}", "..Default"] {
                assert!(
                    !code.contains(rest),
                    "{name}: a rest pattern in a copy: `{}`",
                    line.trim()
                );
            }
        }
    }
}
