//! The equipment test seam (ADR-SIM-7, ADR-SIM-3): `Engine::new_equipped` starts a run in the
//! setup's build mode on a stage as given, and `Engine::script_equipment` fills the inventory and
//! seeds the level of empty slots before the first tick. Neither changes the simulation: a run
//! `new_equipped` starts on the stage a setup derives is the run `start_with` starts. The helper
//! `common::equip` is tested below the seam's own tests.

use crate::common;
use crate::common::run::{plain_content, plain_setup};
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::EquipAction;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{self, BuildMode, RunSetup};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::Error;
use stella_rain_core::version::Versions;

/// A star with one effect and its category.
fn star(id: &str, category: &str, effect: &str, amount: u32) -> Value {
    json!({ "id": id, "rarity": "common", "limit": 3, "category": category,
            "effects": [{ "effect": effect, "amount": amount }] })
}

/// `a` and `b` share the option `max_hp` (Defence); `c` is a Crit Chance star (Attack).
fn stars() -> Value {
    json!([
        star("a", "defence", "max_hp", 100),
        star("b", "defence", "max_hp", 50),
        star("c", "attack", "crit_chance_pct", 40),
    ])
}

fn content() -> RunContent {
    let mut file = json!({ "rarities": common::rarities(), "rewards": stars() });
    file["equipment"] = serde_json::to_value(Rewards::bundled().equipment().unwrap()).unwrap();
    let rewards = Rewards::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap();
    RunContent::new(common::design_run_stats(), rewards)
        .with_perk_sets(plain_content().perk_sets().clone())
}

/// The equipment setup of the tests: the `plain` perk set, every star in the pool.
fn setup() -> RunSetup {
    RunSetup {
        build_mode: BuildMode::Equipment,
        star_pool: Some(vec!["a".into(), "b".into(), "c".into()]),
        ..plain_setup()
    }
}

/// A hand-built stage of the live rules, with the test weapon.
fn stage() -> Stage {
    let mut stage = common::stage_full(600, Some(common::far_boss()), vec![], vec![]);
    stage.sim_version = 2;
    stage
}

fn equip(item: u32, slot: u32) -> EquipAction {
    EquipAction::Equip { item, slot }
}

/// T1: the run starts in equipment mode (the layout is there, an offers run has none) and the
/// weapon is the stage's own, not the setup's.
#[test]
fn new_equipped_starts_in_equipment_mode_with_the_stage_weapon() {
    let (stage, content, setup) = (stage(), content(), setup());
    let mut equipped = Engine::new_equipped(&stage, &content, &setup).expect("starts");
    let mut offers = Engine::new_with(&stage, &content).expect("starts");
    let layout = equipped.snapshot().players[0].slots.len();
    assert!(layout > 0, "equipment mode lays the slots out");
    assert!(offers.snapshot().players[0].slots.is_empty());

    for _ in 0..12 {
        let press = [common::pressing_main(), common::idle(), common::idle()];
        equipped.step(&press);
        offers.step(&press);
    }
    let attacks = equipped.snapshot().players[0].attacks.clone();
    assert!(!attacks.is_empty(), "the stage weapon fired");
    assert_eq!(attacks, offers.snapshot().players[0].attacks);
    let trial = Engine::start_with(&setup, &content).expect("starts");
    assert_ne!(
        trial.snapshot().players[0].attacks,
        attacks,
        "the setup's own stage has another weapon"
    );
}

/// T2: on the stage the setup derives, `new_equipped` is `start_with`: the same hash at tick 0
/// and after 120 ticks. A change to `start_party_with` that the copy misses fails here.
#[test]
fn new_equipped_on_the_setups_own_stage_is_start() {
    let (content, setup) = (content(), setup());
    let stage = run_setup::stage_in(&setup, &content).expect("the setup's stage");
    let mut copy = Engine::new_equipped(&stage, &content, &setup).expect("starts");
    let mut start = Engine::start_with(&setup, &content).expect("starts");
    assert_eq!(copy.state_hash(), start.state_hash());
    for tick in 0..120 {
        let input = common::firing(common::moving(1, 0), tick);
        let inputs = [input, common::idle(), common::idle()];
        copy.step(&inputs);
        start.step(&inputs);
    }
    assert_eq!(copy.state_hash(), start.state_hash());
}

/// T3: the scripted inventory and level: two stars in, an equip into slot 0 leaves one and the
/// slot at the seeded level; what the script refuses changes nothing.
#[test]
fn script_equipment_fills_the_inventory_and_seeds_the_level() {
    let (stage, content, setup) = (stage(), content(), setup());
    let mut e = Engine::new_equipped(&stage, &content, &setup).expect("starts");
    let me = PlayerIdx::SOLO;
    e.script_equipment(me, &["c".into(), "a".into()], &[(0, 3)])
        .expect("scripts");
    assert_eq!(e.snapshot().players[0].inventory.len(), 2);
    assert_eq!(
        e.snapshot().players[0].slots[0].level,
        0,
        "nothing until an equip"
    );
    e.act(me, equip(0, 0)).expect("c fits the Attack slot 0");
    let p = &e.snapshot().players[0];
    assert_eq!(p.inventory.len(), 1);
    assert_eq!(p.slots[0].level, 3, "the seeded level stays");

    let before = e.state_hash();
    let rune = 9; // the first rune slot of the layout: 5 + 2 + 2 categories' slots
    let refused: [(&[&str], &[(usize, u8)]); 5] = [
        (&["nope"], &[]),
        (&[], &[(rune, 2)]),
        (&[], &[(0, 2)]),
        (&[], &[(1, 0)]),
        (&[], &[(1, 11)]),
    ];
    for (ids, levels) in refused {
        let ids: Vec<_> = ids.iter().map(|id| (*id).into()).collect();
        assert!(
            matches!(e.script_equipment(me, &ids, levels), Err(Error::Invalid(_))),
            "{ids:?} {levels:?}"
        );
        assert_eq!(e.state_hash(), before, "a refusal changes nothing");
    }
}
