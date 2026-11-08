//! The equipment test seam (ADR-SIM-7, ADR-SIM-3): `Engine::new_equipped` starts a run in the
//! setup's build mode on a stage as given, and `Engine::script_equipment` fills the inventory and
//! seeds the level of empty slots before the first tick. Neither changes the simulation: a run
//! `new_equipped` starts on the stage a setup derives is the run `start_with` starts. The helper
//! `common::equip` is tested below the seam's own tests.

use crate::common;
use crate::common::equip;
use crate::common::run::{plain_content, plain_setup};
use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::limits::MAX_INVENTORY;
use stella_rain_core::rune::Rune;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::EquipAction;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{self, BuildMode, RunSetup};
use stella_rain_core::snapshot::SlotKindView;
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

/// A slot and the level to seed it with.
type Level = (usize, u8);

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

    let trace = |e: &mut Engine| -> Vec<usize> {
        (0..12)
            .map(|_| {
                e.step(&[common::pressing_main(), common::idle(), common::idle()]);
                e.snapshot().entities.len()
            })
            .collect()
    };
    let (own, plain) = (trace(&mut equipped), trace(&mut offers));
    assert!(own.iter().any(|n| *n > 0), "the stage weapon fired");
    assert_eq!(own, plain, "the same weapon as the stage's own run");
    let mut trial = Engine::start_with(&setup, &content).expect("starts");
    assert_ne!(
        trace(&mut trial),
        own,
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
    let p = &e.snapshot().players[0];
    assert_eq!(p.slots[0].level, 3, "the slot carries the seeded level");
    assert_eq!(
        p.crit_chance_pct, 0,
        "and gives nothing until a star is equipped"
    );
    e.act(me, equip(0, 0)).expect("c fits the Attack slot 0");
    let p = &e.snapshot().players[0];
    assert_eq!(p.inventory.len(), 1);
    assert_eq!(p.slots[0].level, 3, "the seeded level stays");
    assert!(p.crit_chance_pct > 0, "the star is in the build");

    let before = e.state_hash();
    let rune = e.snapshot().players[0]
        .slots
        .iter()
        .position(|s| s.kind == SlotKindView::Rune)
        .expect("the layout has rune slots");
    let refused: [(&[&str], &[Level]); 5] = [
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

/// The first rune slot and the one after it, in the layout.
fn rune_slots_of(e: &Engine) -> (u32, u32) {
    let at: Vec<u32> = e.snapshot().players[0]
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SlotKindView::Rune)
        .map(|(i, _)| u32::try_from(i).unwrap())
        .collect();
    (at[0], at[1])
}

/// T-r1: scripted runes fill the inventory in order; an equip into a rune slot puts the rune
/// into the build ready, in slot order; the same rune twice is refused the second time.
#[test]
fn script_runes_fills_the_inventory_and_a_rune_equips_into_a_rune_slot() {
    let (stage, content, setup) = (stage(), content(), setup());
    let mut e = Engine::new_equipped(&stage, &content, &setup).expect("starts");
    let me = PlayerIdx::SOLO;
    let (first, second) = rune_slots_of(&e);
    e.script_runes(me, &[Rune::Thunder, Rune::Bleed, Rune::Thunder])
        .expect("scripts");
    assert_eq!(e.snapshot().players[0].inventory.len(), 3);
    e.act(me, equip(0, first)).expect("thunder fits a rune slot");
    let runes = &e.snapshot().players[0].runes;
    assert_eq!(runes.iter().map(|r| r.rune).collect::<Vec<_>>(), [Rune::Thunder]);
    assert_eq!(runes[0].cooldown_left, 0, "a rune equipped arrives ready");
    e.act(me, equip(0, second)).expect("bleed fits the next");
    let runes = &e.snapshot().players[0].runes;
    assert_eq!(
        runes.iter().map(|r| r.rune).collect::<Vec<_>>(),
        [Rune::Thunder, Rune::Bleed]
    );
    assert!(
        e.act(me, equip(0, second + 1)).is_err(),
        "the same rune twice is refused"
    );
}

/// T-r2: a rune in a category slot is refused; too many items, and the old rules, are
/// `Error::Invalid` and change nothing.
#[test]
fn script_runes_refuses_what_it_cannot_do() {
    let (stage, content, setup) = (stage(), content(), setup());
    let mut e = Engine::new_equipped(&stage, &content, &setup).expect("starts");
    let me = PlayerIdx::SOLO;
    e.script_runes(me, &[Rune::Thunder]).expect("scripts");
    assert!(e.act(me, equip(0, 0)).is_err(), "a rune does not fit slot 0");

    let before = e.state_hash();
    let too_many = vec![Rune::Thunder; MAX_INVENTORY];
    assert!(matches!(
        e.script_runes(me, &too_many),
        Err(Error::Invalid(_))
    ));
    assert_eq!(e.state_hash(), before, "a refusal changes nothing");
    assert_eq!(e.snapshot().players[0].inventory.len(), 1);

    let mut old = stage.clone();
    old.sim_version = 1;
    let mut v1 = Engine::new_equipped(&old, &content, &setup).expect("starts");
    let Err(Error::Invalid(issues)) = v1.script_runes(me, &[Rune::Thunder]) else {
        panic!("the old rules hold no equipment");
    };
    assert_eq!(issues[0].path, "$.run");
}

/// A stage whose one wave gives 10 points per kill, and the expertise file with thresholds
/// 10, 20 .. 100.
fn expertise_content() -> RunContent {
    let mut file: Value = serde_json::from_slice(common::bundled_file("expertise")).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    let expertise = Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test expertise");
    content().with_expertise(expertise)
}

fn expertise_stage() -> Stage {
    let mut w = common::wave(
        1,
        1,
        0,
        common::enemy(1, common::hold(common::START_X, common::START_Y - 90_000), json!([])),
        common::START_X,
        common::START_Y - 90_000,
    );
    w["expertise"] = json!({ "hit": 0, "kill": 10 });
    let mut stage = common::stage_full(600, Some(common::far_boss()), vec![w], vec![]);
    stage.sim_version = 2;
    stage
}

fn idle_tick(e: &mut Engine) {
    e.step(&[common::idle(), common::idle(), common::idle()]);
}

/// T-x1: 210 points are levels 1 to 6 (10+20+..+60), and the level rises on the next tick.
#[test]
fn script_expertise_adds_points_that_level_on_the_next_tick() {
    let (stage, content, setup) = (expertise_stage(), expertise_content(), setup());
    let mut e = Engine::new_equipped(&stage, &content, &setup).expect("starts");
    e.script_expertise(PlayerIdx::SOLO, 210).expect("scripts");
    idle_tick(&mut e);
    let gauge = e.snapshot().expertise.clone().expect("a gauge");
    assert_eq!((gauge.level, gauge.points), (6, 0));
}

/// T-x2: seeds add: 100 and 100 are 200.
#[test]
fn script_expertise_adds_to_the_points_a_kill_gave() {
    let (stage, content, setup) = (expertise_stage(), expertise_content(), setup());
    let run = |seeds: &[u32]| {
        let mut e = Engine::new_equipped(&stage, &content, &setup).expect("starts");
        for &n in seeds {
            e.script_expertise(PlayerIdx::SOLO, n).expect("scripts");
        }
        idle_tick(&mut e);
        let gauge = e.snapshot().expertise.clone().expect("a gauge");
        ((gauge.level, gauge.points), e.state_hash())
    };
    assert_eq!(run(&[100, 100]), run(&[200]));
}

/// T-x3: a stage that gives no points has no gauge to seed.
#[test]
fn script_expertise_refuses_a_stage_without_a_gauge() {
    let (stage, content, setup) = (stage(), expertise_content(), setup());
    let mut e = Engine::new_equipped(&stage, &content, &setup).expect("starts");
    let before = e.state_hash();
    let Err(Error::Invalid(issues)) = e.script_expertise(PlayerIdx::SOLO, 10) else {
        panic!("no gauge");
    };
    assert_eq!(issues[0].path, "$.expertise");
    assert_eq!(e.state_hash(), before);
}

// The helper `common::equip`: stars on a hand-built stage, written without categories.

/// Bare stars, as a test writes them: no category, no `equipment` object. `a` and `b` share the
/// option `max_hp`; `c` has another, whose amount floors differently at each level.
fn bare_stars() -> Value {
    let bare = |id: &str, effect: &str, amount: u32| {
        json!({ "id": id, "rarity": "common", "limit": 3,
                "effects": [{ "effect": effect, "amount": amount }] })
    };
    json!({ "rarities": common::rarities(), "rewards": [
        bare("a", "max_hp", 100), bare("b", "max_hp", 50), bare("c", "crit_chance_pct", 40)
    ] })
}

/// The slots of the layout the stars go in: Attack first (`c`), then Defence (`a`, `b`).
const ATTACK: usize = 0;
const DEFENCE: usize = 5;

fn crit_of(e: &Engine) -> u32 {
    e.snapshot().players[0].crit_chance_pct
}

/// T4: a star equipped at Lv1 is the star picked from an offer: the same stat as the run that
/// carries it in as if picked, different from the run with nothing equipped; the slot is Lv1.
#[test]
fn lv1_equals_one_offer_pick_for_the_same_star() {
    use stella_rain_core::run_setup::CarriedStar;
    let (stage, content) = (stage(), content());
    let equipped = equip::equipped(&stage, bare_stars(), &[("c", ATTACK)], 1);
    let nothing = equip::equipped(&stage, bare_stars(), &[], 1);
    let cost = content
        .rewards()
        .lumine_cost(content.rewards().rewards()[2].rarity);
    let offers = RunSetup {
        carried: vec![CarriedStar { reward: "c".into() }],
        lumine: cost,
        star_pool: Some(vec!["c".into()]),
        ..plain_setup()
    };
    let picked = Engine::start_with(&offers, &content).expect("starts");
    assert_eq!(crit_of(&nothing), 0);
    assert_eq!(crit_of(&equipped), 40);
    assert_eq!(crit_of(&equipped), crit_of(&picked));
    assert_eq!(equipped.snapshot().players[0].slots[ATTACK].level, 1);
}

/// T5: a star at Lv n adds `level_scaled`: the amount plus `level_pct` for each level above 1,
/// floored (restated here from the pack's rules, not read from the simulation).
#[test]
fn lv_n_equals_level_scaled() {
    let rules = Rewards::bundled().equipment().expect("the pack's rules");
    let (stage, amount) = (stage(), 40u32);
    let mut floored = false;
    for n in [1u8, 2, 10] {
        let e = equip::equipped(&stage, bare_stars(), &[("c", ATTACK)], n);
        let times = 100 + rules.level_pct * (u32::from(n) - 1);
        floored |= amount * times % 100 != 0;
        assert_eq!(crit_of(&e), amount * times / 100, "Lv{n}");
        assert_eq!(e.snapshot().players[0].slots[ATTACK].level, n);
    }
    assert!(floored, "some level has a floor that matters");
}

/// T6: a star sharing an option with an equipped one is refused with `OptionHeld`, naming the
/// slot that holds it; `equipped` panics naming the star, the slot and the refusal.
#[test]
fn a_clashing_star_is_refused_with_option_held() {
    use stella_rain_core::engine::ActionError;
    let stage = stage();
    let clash = [("a", DEFENCE), ("b", DEFENCE + 1)];
    let refused = equip::try_equipped(&stage, bare_stars(), &clash, 1).err();
    assert_eq!(refused, Some(ActionError::OptionHeld { slot: 5 }));
    let fine = [("a", DEFENCE), ("c", ATTACK)];
    assert!(equip::try_equipped(&stage, bare_stars(), &fine, 1).is_ok());
}

#[test]
#[should_panic(expected = "equip `b` into slot 6")]
fn equipped_panics_on_a_clash_naming_the_star_and_the_slot() {
    let clash = [("a", DEFENCE), ("b", DEFENCE + 1)];
    equip::equipped(&stage(), bare_stars(), &clash, 1);
}

/// T7: the helper gives a bare reward the category the bundled pack gives the same first effect
/// (the table is built from the pack); a first effect no bundled reward has panics naming it.
#[test]
fn the_helper_gives_a_hand_written_pack_its_categories() {
    for r in Rewards::bundled().rewards() {
        let effect = serde_json::to_value(r.effect()).unwrap();
        let wanted = r.category.as_ref().map(|c| c.as_str());
        assert_eq!(
            equip::category_of_effect(effect.as_str().unwrap()),
            wanted,
            "{}",
            r.id.as_str()
        );
    }
}

#[test]
#[should_panic(expected = "no_such_effect")]
fn an_effect_the_pack_never_uses_panics_naming_it() {
    equip::category_of_effect("no_such_effect");
}

// The helper's variant for a test with its own stage skills or constellations.

/// A stage skill star `reward` that gives the skill `skill`, written without a category.
fn skill_star(reward: &str, skill: &str, amount: u32) -> Value {
    json!({ "id": reward, "rarity": "common", "effect": "stage_skill", "skill": skill,
            "amount": amount, "limit": 5 })
}

/// A skills file's list: one skill `pulse` whose levels each have a 40 tick cooldown.
fn pulse_skills() -> Value {
    let level = json!({ "cooldown_ticks": 40,
        "effects": [{ "type": "self_status", "status": "atk_up", "value_pct": 50,
                      "duration_ticks": 60 }] });
    json!([{ "id": "pulse", "icon": "icon_nova", "ahead": 0, "levels": [level, level, level] }])
}

/// T-a: a stage skill star of the test's own skills file is equipped at the level the star's
/// amount gives, and arrives cooling, not ready; it is ready once its cooldown has run.
#[test]
fn a_custom_skill_star_is_equipped_and_cooling() {
    let rewards =
        json!({ "rarities": common::rarities(), "rewards": [skill_star("p", "pulse", 2)] });
    let parts = equip::Parts {
        skills: Some(pulse_skills()),
        ..equip::Parts::default()
    };
    let mut e = equip::equipped_in(&stage(), rewards, parts, &[("p", ATTACK)], 1);
    let skill = e.snapshot().players[0].skill.clone().expect("a skill");
    assert_eq!((skill.skill.as_str(), skill.level), ("pulse", 2));
    assert!(!skill.ready, "a skill star arrives cooling");
    for _ in 0..skill.cooldown_left {
        common::fire_until(&mut e, common::idle(), 1, |_| false);
    }
    assert!(e.snapshot().players[0].skill.as_ref().unwrap().ready);
}

/// T-b: the sets of `Parts` count their members: the tier at two of two applies with both
/// equipped, not with one; without the sets nothing applies.
#[test]
fn custom_sets_count_their_members() {
    let named = |id: &str, effect: &str, amount: u32| {
        json!({ "id": id, "rarity": "epic", "named": true, "star": id, "limit": 1,
                "effects": [{ "effect": effect, "amount": amount }] })
    };
    let rewards = || {
        json!({ "rarities": common::rarities(), "rewards": [
            named("m1", "crit_chance_pct", 10), named("m2", "max_hp", 10)
        ] })
    };
    let sets = || {
        let tier = json!({ "threshold": 2,
            "effects": [{ "effect": "burst_depth", "amount": 2 }] });
        let duo = json!({ "id": "duo", "name": "The duo", "members": ["m1", "m2"],
                          "tiers": [tier] });
        equip::Parts {
            sets: Some(json!([duo])),
            ..equip::Parts::default()
        }
    };
    let depth = |e: &Engine| e.snapshot().players[0].burst_depth;
    let both = [("m1", ATTACK), ("m2", DEFENCE)];
    let one = [("m1", ATTACK)];
    assert_eq!(
        depth(&equip::equipped_in(&stage(), rewards(), sets(), &both, 1)),
        2
    );
    assert_eq!(
        depth(&equip::equipped_in(&stage(), rewards(), sets(), &one, 1)),
        0
    );
    let bare = equip::Parts::default();
    assert_eq!(
        depth(&equip::equipped_in(&stage(), rewards(), bare, &both, 1)),
        0
    );
}
