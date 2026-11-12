//! Stars equipped at a level on a hand-built stage, for tests (ADR-SIM-7): `equipped` starts an
//! equipment-mode run on the stage as given, with the chosen stars in the chosen slots at the
//! chosen level, through the engine's own `Equip`. It rests on the bench-only seam
//! `Engine::new_equipped` and `Engine::script_equipment`; a test that needs a drop, a kill or a
//! level-up plays the run instead.
use crate::common;
use crate::common::run::plain_content;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use stella_rain_core::constellations::Constellations;
use stella_rain_core::engine::{ActionError, Engine};
use stella_rain_core::expertise::Expertise;
use stella_rain_core::id::{ContentId, PlayerIdx};
use stella_rain_core::input::EquipAction;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::Runes;
use stella_rain_core::stage::Stage;
use stella_rain_core::stage_skill::StageSkills;
use stella_rain_core::version::Versions;

/// The category the bundled pack gives each first effect (`None`: Growth and the random rune,
/// which fit no category slot). One effect with two categories in the pack is a panic.
fn categories() -> &'static BTreeMap<String, Option<String>> {
    static TABLE: OnceLock<BTreeMap<String, Option<String>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table: BTreeMap<String, Option<String>> = BTreeMap::new();
        for r in Rewards::bundled().rewards() {
            let effect = serde_json::to_value(r.effect()).expect("an effect name");
            let effect = effect.as_str().expect("a string").to_owned();
            let category = r.category.as_ref().map(|c| c.as_str().to_owned());
            if let Some(seen) = table.insert(effect.clone(), category.clone()) {
                assert_eq!(seen, category, "the pack gives `{effect}` two categories");
            }
        }
        table
    })
}

/// The category of a reward whose first effect is `effect`, as the bundled pack gives it. An
/// effect no bundled reward has panics naming it.
pub fn category_of_effect(effect: &str) -> Option<&'static str> {
    match categories().get(effect) {
        Some(category) => category.as_deref(),
        None => panic!("no bundled reward has the effect `{effect}`: give the reward a category"),
    }
}

/// `rewards`, a hand-written rewards file, with the bundled `equipment` rules and each reward
/// that has no `category` (a file that brings its own `equipment` keeps it) given the bundled one of its first effect.
fn with_equipment_rules(mut rewards: Value) -> Value {
    if rewards.get("equipment").is_none() {
        let rules = Rewards::bundled().equipment().expect("the pack's rules");
        rewards["equipment"] = serde_json::to_value(rules).expect("the rules serialise");
    }
    for r in rewards["rewards"].as_array_mut().expect("a rewards list") {
        let first = r["effects"][0]["effect"].as_str().or(r["effect"].as_str());
        let first = first.expect("a reward has an effect").to_owned();
        if r.get("category").is_none()
            && let Some(category) = category_of_effect(&first)
        {
            r["category"] = category.into();
        }
    }
    rewards
}

/// What a test brings besides the rewards file: its own stage skills (the `stage_skills` array of
/// a skills file) and its own constellations (the `constellations` array). `None` is the bundled
/// skills and no sets.
#[derive(Default)]
pub struct Parts {
    pub skills: Option<Value>,
    pub sets: Option<Value>,
    /// The runes file of the run (`RunContent::with_runes`); `None` is the bundled one.
    pub runes: Option<Runes>,
    /// The expertise file of the run (`RunContent::with_expertise`); `None` is the bundled one.
    pub expertise: Option<Expertise>,
    /// Each rune and the layout slot (`rune_slot`) it is equipped into, after the stars.
    pub equip_runes: Vec<(Rune, usize)>,
    /// Weapon expertise points seeded before the first tick (`Engine::script_expertise`); the
    /// level rises on the first tick. 0 seeds nothing.
    pub expertise_points: u32,
    /// The setup's `unlocked_styles` (absent: every style), applied through the setup's JSON so
    /// the file's shape is what is tested. The setup's weapon is the stage's preset weapon.
    pub unlocked_styles: Option<Vec<ContentId>>,
}

/// The layout slot of rune slot `n` of the `equipment` object `rewards` brings (the bundled one
/// when it brings none): the categories' slots, then `n`.
pub fn rune_slot(rewards: &Value, n: usize) -> usize {
    let categories = match rewards.get("equipment") {
        Some(rules) => rules["categories"].clone(),
        None => serde_json::to_value(
            &Rewards::bundled()
                .equipment()
                .expect("the pack's rules")
                .categories,
        )
        .expect("the categories serialise"),
    };
    let slots: u64 = categories
        .as_array()
        .expect("a category list")
        .iter()
        .map(|c| c["slots"].as_u64().expect("a slot count"))
        .sum();
    usize::try_from(slots).expect("a count") + n
}

/// Starts an equipment-mode, one-player run on `stage` (its `sim_version` is set to the live
/// rules) over the hand-written rewards file `rewards`, with each `(reward ID, slot)` of `stars`
/// equipped at `level` (1 to 10). The slots are the layout's: the categories' slots in list
/// order, then the rune slots. The perk set is `plain`, the star pool the pack's own (`default_pool`), and the
/// stats are not picked. A star the engine refuses is the `ActionError`.
pub fn try_equipped(
    stage: &Stage,
    rewards: Value,
    stars: &[(&str, usize)],
    level: u8,
) -> Result<Engine, ActionError> {
    try_equipped_in(stage, rewards, Parts::default(), stars, level)
}

/// `try_equipped` over the `parts` the test brings.
pub fn try_equipped_in(
    stage: &Stage,
    rewards: Value,
    parts: Parts,
    stars: &[(&str, usize)],
    level: u8,
) -> Result<Engine, ActionError> {
    build(stage, rewards, parts, stars, level).map_err(|(_, refused)| refused)
}

/// `try_equipped`, with the place of the star the engine refused: in `stars`, then in
/// `parts.equip_runes`.
fn build(
    stage: &Stage,
    rewards: Value,
    parts: Parts,
    stars: &[(&str, usize)],
    level: u8,
) -> Result<Engine, (usize, ActionError)> {
    let rewards = with_equipment_rules(rewards);
    let bytes = rewards.to_string();
    let mut rewards = match parts.skills {
        Some(skills) => {
            let file = serde_json::json!({ "stage_skills": skills }).to_string();
            let skills = StageSkills::parse(file.as_bytes(), Versions::CURRENT)
                .unwrap_or_else(|e| panic!("the skills file validates: {e:?}"));
            Rewards::parse_with(bytes.as_bytes(), Versions::CURRENT, &skills)
        }
        None => Rewards::parse(bytes.as_bytes(), Versions::CURRENT),
    }
    .unwrap_or_else(|e| panic!("the rewards file validates: {e:?}"));
    if let Some(sets) = parts.sets {
        let file = serde_json::json!({ "constellations": sets }).to_string();
        let sets = Constellations::parse_with(file.as_bytes(), Versions::CURRENT, &rewards)
            .unwrap_or_else(|e| panic!("the sets validate: {e:?}"));
        rewards = rewards.with_constellations(sets);
    }
    let mut content = RunContent::new(common::design_run_stats(), rewards)
        .with_perk_sets(plain_content().perk_sets().clone());
    if let Some(runes) = parts.runes {
        content = content.with_runes(runes);
    }
    if let Some(expertise) = parts.expertise {
        content = content.with_expertise(expertise);
    }
    let mut setup = RunSetup {
        perk_set: "plain".into(),
        seed: u64::from(stage.seed),
        ..RunSetup::default()
    };
    if let Some(styles) = &parts.unlocked_styles {
        let weapon = serde_json::to_value(&stage.player.weapon).expect("the weapon serialises");
        let mut v = serde_json::to_value(&setup).expect("the setup serialises");
        v["weapon"] = weapon["preset"]["id"].clone();
        v["unlocked_styles"] = serde_json::to_value(styles).expect("the styles serialise");
        setup = serde_json::from_value(v).expect("a setup with unlocked styles reads");
    }
    let mut stage = stage.clone();
    stage.sim_version = setup.sim_version;
    let mut engine = Engine::new_equipped(&stage, &content, &setup).expect("the run starts");
    let held: Vec<ContentId> = stars.iter().map(|(id, _)| ContentId::from(*id)).collect();
    let levels: Vec<(usize, u8)> = stars.iter().map(|&(_, slot)| (slot, level)).collect();
    engine
        .script_equipment(PlayerIdx::SOLO, &held, &levels)
        .expect("the stars and slots are the layout's");
    let runes: Vec<Rune> = parts.equip_runes.iter().map(|&(r, _)| r).collect();
    engine
        .script_runes(PlayerIdx::SOLO, &runes)
        .expect("the runes fit the inventory");
    if parts.expertise_points > 0 {
        engine
            .script_expertise(PlayerIdx::SOLO, parts.expertise_points)
            .expect("the stage gives expertise points");
    }
    let slots = stars
        .iter()
        .map(|&(_, slot)| slot)
        .chain(parts.equip_runes.iter().map(|&(_, slot)| slot));
    for (at, slot) in slots.enumerate() {
        let slot = u32::try_from(slot).expect("a slot");
        let equip = EquipAction::Equip { item: 0, slot };
        engine
            .act(PlayerIdx::SOLO, equip)
            .map_err(|refused| (at, refused))?;
    }
    Ok(engine)
}

/// As `try_equipped`, and a refusal panics naming the star, the slot and the `ActionError`.
pub fn equipped(stage: &Stage, rewards: Value, stars: &[(&str, usize)], level: u8) -> Engine {
    equipped_in(stage, rewards, Parts::default(), stars, level)
}

/// `equipped` over the `parts` the test brings.
pub fn equipped_in(
    stage: &Stage,
    rewards: Value,
    parts: Parts,
    stars: &[(&str, usize)],
    level: u8,
) -> Engine {
    let runes = parts.equip_runes.clone();
    build(stage, rewards, parts, stars, level).unwrap_or_else(|(at, refused)| match stars.get(at) {
        Some(&(id, slot)) => {
            panic!("equip `{id}` into slot {slot} at Lv{level}: refused with {refused:?}")
        }
        None => {
            let (rune, slot) = runes[at - stars.len()];
            panic!("equip `{rune:?}` into slot {slot}: refused with {refused:?}")
        }
    })
}

/// Steps idle until the tough target of `on_hit` is on the map, and returns its ID: the first
/// enemy with 2 to 100 000 hit points. An equipped run has no small enemies, so the boss (more
/// hit points) is the first one standing and `on_hit::wait_for_target` would take it.
pub fn wait_for_tough(e: &mut Engine) -> stella_rain_core::id::EntityId {
    common::on_hit::wait_for_target(e);
    e.snapshot()
        .entities
        .iter()
        .find(|en| en.hp > 1 && en.hp <= 100_000)
        .map(|en| en.id)
        .expect("the tough target")
}
