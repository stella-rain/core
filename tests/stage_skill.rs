//! The stage skill: one slot, empty at the start of a run, filled from
//! falling stars; a press of `skill` uses a ready skill at its level, and each skill has its own
//! cooldown, counted from its use. Taking another skill replaces the one held and arrives ready;
//! taking the same one again raises its level. A level's effects are building blocks, applied
//! in list order at the skill's one point: pull, self status, area status, attack and shield.
//! Every number here is worked out by hand from the rules' constants.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{ContentId, EntityId};
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;
use stella_rain_core::stage_skill::StageSkills;
use stella_rain_core::version::Versions;

use crate::common;

use common::*;

/// How far ahead the test skills act: 100 pixels.
const AHEAD: i32 = 25_600;
const COOLDOWN: u32 = 120;
const DURATION: u32 = 60;

/// An enemy that never moves or attacks, spawned on `at_tick` at `(x, y)`.
fn standing(at_tick: u32, hp: u32, x: i32, y: i32) -> Value {
    wave(
        at_tick,
        1,
        0,
        enemy(
            hp,
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
            json!([]),
        ),
        x,
        y,
    )
}

/// A stage with one level and stars, where every kill drops a common star, and `waves`.
fn starry(waves: Vec<Value>) -> Stage {
    let mut stage = stage_full(18_000, Some(far_boss()), waves, vec![]);
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 0, "banishes": 0,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

/// The bundled icon the test skills draw with.
const ICON: &str = "icon_nova";

/// A skill `id` acting `AHEAD` pixels ahead, whose levels have these effects (a list, or one
/// effect), each with the cooldown `COOLDOWN` less 10 ticks a level.
fn skill(id: &str, levels: &[Value]) -> Value {
    let levels: Vec<Value> = levels
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let effects = if e.is_array() { e.clone() } else { json!([e]) };
            json!({ "cooldown_ticks": COOLDOWN - 10 * u32::try_from(i).unwrap(), "effects": effects })
        })
        .collect();
    json!({ "id": id, "icon": ICON, "ahead": AHEAD,
            "levels": levels })
}

/// `skill` with a cooldown of `cooldown` ticks at every level.
fn quick(id: &str, levels: &[Value], cooldown: u32) -> Value {
    let mut v = skill(id, levels);
    for level in v["levels"].as_array_mut().unwrap() {
        level["cooldown_ticks"] = json!(cooldown);
    }
    v
}

/// A stage skills file of `skills` and a rewards file that, for each `(reward, skill, amount)`,
/// holds a common reward that gives that skill, five times a run.
fn content(skills: Vec<Value>, rewards: &[(&str, &str, u32)]) -> Rewards {
    let rewards: Vec<Value> = rewards
        .iter()
        .map(|(id, skill, amount)| {
            json!({ "id": id, "rarity": "common", "effect": "stage_skill", "skill": skill,
                    "amount": amount, "limit": 5 })
        })
        .collect();
    let skills = json!({ "stage_skills": skills });
    let skills = StageSkills::parse(skills.to_string().as_bytes(), Versions::CURRENT)
        .expect("the test skills validate");
    let file = json!({ "rarities": common::rarities(), "rewards": rewards });
    Rewards::parse_with(file.to_string().as_bytes(), Versions::CURRENT, &skills)
        .expect("the test rewards validate")
}

fn with(stage: &Stage, rewards: Rewards) -> Engine {
    let content = RunContent::new(common::design_run_stats(), rewards);
    Engine::new_with(stage, &content).expect("the test stage is valid")
}

/// A stage of `waves` with no stars: no drop and no offer.
fn bare(waves: Vec<Value>) -> Stage {
    stage_full(18_000, Some(far_boss()), waves, vec![])
}

/// A run on `stage` with the stage skill star `(reward, skill, level)` equipped at Lv1 in the
/// first attack slot (ADR-SIM-7), waited ready: a skill star arrives cooling, in equipment
/// mode.
fn equipped_skill(stage: &Stage, skills: Vec<Value>, star: (&str, &str, u32)) -> Engine {
    let (id, skill, level) = star;
    let reward = json!({ "id": id, "rarity": "common", "effect": "stage_skill", "skill": skill,
                         "amount": level, "limit": 5 });
    let file = json!({ "rarities": common::rarities(), "rewards": [reward] });
    let parts = common::equip::Parts {
        skills: Some(json!(skills)),
        ..Default::default()
    };
    let mut e = common::equip::equipped_in(stage, file, parts, &[(id, 0)], 1);
    ready(&mut e);
    e
}

/// Steps idle until the skill is ready to press (at most 200 ticks).
fn ready(e: &mut Engine) {
    for _ in 0..200 {
        let skill = e.snapshot().players[0].skill.clone().expect("a skill held");
        if skill.ready {
            return;
        }
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    panic!("the skill is not ready after 200 ticks");
}

fn pressing_skill() -> Input {
    Input {
        skill: true,
        ..idle()
    }
}

/// Presses the skill on one tick, then lets go on the next.
fn use_skill(e: &mut Engine) -> Vec<DomainEvent> {
    e.step(&[pressing_skill(), Default::default(), Default::default()]);
    let events = e.events().to_vec();
    e.step(&[idle(), Default::default(), Default::default()]);
    events
}

fn used(events: &[DomainEvent]) -> Option<(String, u8)> {
    events.iter().find_map(|ev| match ev {
        DomainEvent::StageSkillUsed { skill, level } => Some((skill.to_string(), *level)),
        _ => None,
    })
}

fn enemies_at(e: &Engine) -> Vec<(EntityId, i32, i32)> {
    e.snapshot()
        .entities
        .iter()
        .filter(|x| x.kind == EntityKind::Enemy)
        .map(|x| (x.id, x.at.x.0, x.at.y.0))
        .collect()
}

fn pull(radius: i32, strength: i32) -> Value {
    json!({ "type": "pull", "radius": radius, "strength": strength, "duration_ticks": DURATION })
}

fn buff(pct: u16) -> Value {
    json!({ "type": "self_status", "status": "atk_up", "value_pct": pct,
            "duration_ticks": DURATION })
}

fn area(status: &str, radius: i32) -> Value {
    json!({ "type": "area_status", "status": status, "radius": radius, "value_pct": 25,
            "duration_ticks": DURATION })
}

/// The statuses applied in `events`, in order, as `(target, status)`.
fn applied(events: &[DomainEvent]) -> Vec<(EntityId, String)> {
    events
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::StatusApplied { target, status, .. } => {
                Some((*target, status.to_string()))
            }
            _ => None,
        })
        .collect()
}

fn shield(amount: u32, duration_ticks: u32) -> Value {
    json!({ "type": "shield", "amount": amount, "duration_ticks": duration_ticks })
}

// --- The slot --------------------------------------------------------------------------------

#[test]
fn the_slot_is_empty_at_the_start_and_a_press_does_nothing() {
    let mut e = with(
        &starry(vec![]),
        content(vec![skill("aura", &[buff(50)])], &[("aura_r", "aura", 1)]),
    );
    assert_eq!(e.snapshot().players[0].skill, None);
    e.step(&[pressing_skill(), Default::default(), Default::default()]);
    assert_eq!(used(e.events()), None);
}

#[test]
fn a_press_uses_it_and_its_level_s_cooldown_counts_from_the_use() {
    let skills = vec![skill("aura", &[buff(10), buff(20)])];
    let mut e = equipped_skill(&bare(vec![]), skills, ("aura_r", "aura", 2));
    e.step(&[pressing_skill(), Default::default(), Default::default()]);
    let at = e.tick();
    assert_eq!(used(e.events()), Some(("aura".into(), 2)));
    e.step(&[idle(), Default::default(), Default::default()]);
    // Level 2's cooldown is 110 ticks; one tick has passed since the use.
    let view = e.snapshot().players[0].skill.clone().unwrap();
    assert_eq!((view.ready, view.cooldown_left), (false, COOLDOWN - 10 - 1));
    // Pressed on every other tick from then on: used again 110 ticks after the first use.
    let mut again = None;
    for _ in 0..COOLDOWN {
        e.step(&[pressing_skill(), Default::default(), Default::default()]);
        if again.is_none() && used(e.events()).is_some() {
            again = Some(e.tick());
        }
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    assert_eq!(again, Some(at + COOLDOWN - 10));
}

// --- The building blocks ---------------------------------------------------------------------

#[test]
fn a_buff_raises_the_main_character_s_attack_for_its_time() {
    let skills = vec![quick("aura", &[buff(50)], 1)];
    let mut e = equipped_skill(&bare(vec![]), skills, ("aura_r", "aura", 1));
    let events = use_skill(&mut e);
    assert!(events.contains(&DomainEvent::StatusApplied {
        target: EntityId(0),
        status: ContentId::from("atk_up"),
        stacks: 1,
        duration_ticks: DURATION,
        // The live rules name the player as the source; the frozen version 1 events had none.
        source: Some(EntityId(0)),
    }));
}

#[test]
fn an_area_status_makes_the_enemies_near_its_point_vulnerable() {
    let debuff = area("vulnerable", 16_384);
    // The point is 100 pixels up the map from the main character: one enemy 40 pixels from it,
    // one 120 pixels from it.
    let point_y = START_Y - AHEAD;
    let waves = vec![
        standing(40, 50, START_X + 10_240, point_y),
        standing(40, 50, START_X - 30_720, point_y),
    ];
    let skills = vec![quick("hex", &[debuff], 1)];
    let mut e = equipped_skill(&bare(waves), skills, ("hex_r", "hex", 1));
    while enemies_at(&e).len() < 2 {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    let near = enemies_at(&e)[0].0;
    let events = use_skill(&mut e);
    assert_eq!(applied(&events), [(near, "vulnerable".to_string())]);
}

#[test]
fn a_pull_draws_the_enemies_near_its_point_to_it_and_holds_them_there() {
    // 10 pixels right of the point, pulled 1 pixel a tick: there in 10 ticks, and held.
    let point_y = START_Y - AHEAD;
    let waves = vec![
        standing(40, 50, START_X + 2_560, point_y),
        standing(40, 50, START_X - 51_200, point_y),
    ];
    let skills = vec![quick("well", &[pull(16_384, 256)], 1)];
    let mut e = equipped_skill(&bare(waves), skills, ("well_r", "well", 1));
    while enemies_at(&e).len() < 2 {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    let far = enemies_at(&e)[1];
    e.step(&[pressing_skill(), Default::default(), Default::default()]);
    assert_eq!(enemies_at(&e)[0].1, START_X + 2_560 - 256, "the first pull");
    let view = e.snapshot().players[0].pull.expect("a pull");
    assert_eq!((view.at.x.0, view.at.y.0), (START_X, point_y));
    assert_eq!((view.radius.0, view.ticks_left), (16_384, DURATION));
    for _ in 0..20 {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    let near = enemies_at(&e)[0];
    assert_eq!((near.1, near.2), (START_X, point_y), "pulled in and held");
    assert_eq!(enemies_at(&e)[1], far, "out of its radius");
    for _ in 21..DURATION {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    assert!(e.snapshot().players[0].pull.is_some(), "its last tick");
    e.step(&[idle(), Default::default(), Default::default()]);
    assert_eq!(e.snapshot().players[0].pull, None, "over after its time");
}

#[test]
fn a_shield_absorbs_damage_up_to_its_amount_for_its_time() {
    let shooter = enemy(
        1000,
        json!({ "type": "straight", "vx": 0, "vy": 0 }),
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack",
                    "attack": { "preset": { "id": "aimed_single", "args": [15] } } }),
            70
        )]),
    );
    let waves = vec![wave(60, 1, 0, shooter, START_X, START_Y - 40_000)];
    let skills = vec![quick("ward", &[shield(100, 600)], 1)];
    let mut e = equipped_skill(&bare(waves), skills, ("ward_r", "ward", 1));
    let hp = e.snapshot().players[0].hp;
    use_skill(&mut e);
    assert_eq!(
        e.snapshot().players[0].shield.map(|s| s.amount),
        Some(100),
        "the shield is up"
    );
    let mut hits = Vec::new();
    for _ in 0..200 {
        e.step(&[idle(), Default::default(), Default::default()]);
        for ev in e.events() {
            if let DomainEvent::PlayerHit { hp_left, .. } = ev {
                hits.push(*hp_left);
            }
        }
    }
    // The first shot's 100 damage (the default, 1 before the scale) is absorbed; the shield is gone, and the next one hurts.
    assert_eq!(hits.first(), Some(&hp));
    assert!(hits.iter().any(|&h| h < hp), "{hits:?}");
    assert_eq!(e.snapshot().players[0].shield, None);
}

#[test]
fn an_attack_runs_in_its_own_slot_beside_a_combo_step() {
    let area = json!({ "type": "attack", "attack": { "inline": [
        { "kind": "active", "ticks": 20, "damage": 7,
          "hit_box": { "type": "circle", "offset": { "forward": AHEAD }, "radius": 8_192 } }
    ] } });
    let point_y = START_Y - AHEAD;
    let waves = vec![standing(40, 50, START_X, point_y)];
    let skills = vec![quick("nova", &[area], 1)];
    let mut e = equipped_skill(&bare(waves), skills, ("nova_r", "nova", 1));
    while enemies_at(&e).is_empty() {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    e.step(&[
        Input {
            main: true,
            skill: true,
            ..idle()
        },
        Default::default(),
        Default::default(),
    ]);
    assert!(
        e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ComboStep { .. })),
        "the combo step on the same tick"
    );
    let slots: Vec<u8> = e.snapshot().players[0]
        .attacks
        .iter()
        .map(|a| a.slot)
        .collect();
    assert!(slots.contains(&1), "the stage skill's slot: {slots:?}");
    let hit = e
        .events()
        .iter()
        .any(|ev| matches!(ev, DomainEvent::Hit { damage: 7, .. }));
    assert!(hit, "{:?}", e.events());
}

#[test]
fn with_assist_on_the_point_is_ahead_toward_the_target() {
    // The target stands 200 pixels to the right: the point is 100 pixels toward it.
    let mut stage =
        serde_json::to_value(bare(vec![standing(40, 50, START_X + 51_200, START_Y)])).unwrap();
    stage["player"]["weapon"]["inline"]["assist_range"] = json!(65_536);
    let skills = vec![quick("well", &[pull(4_096, 0)], 1)];
    let stage = serde_json::from_value(stage).unwrap();
    let mut e = equipped_skill(&stage, skills, ("well_r", "well", 1));
    while enemies_at(&e).is_empty() {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    e.step(&[
        Input {
            assist: true,
            ..pressing_skill()
        },
        Default::default(),
        Default::default(),
    ]);
    let view = e.snapshot().players[0].pull.expect("a pull");
    assert_eq!((view.at.x.0, view.at.y.0), (START_X + AHEAD, START_Y));
}

#[test]
fn the_slot_is_in_the_state_hash() {
    let skills = vec![quick("aura", &[buff(10), buff(20)], 1)];
    let a = equipped_skill(&bare(vec![]), skills.clone(), ("aura_r", "aura", 1));
    let b = equipped_skill(&bare(vec![]), skills, ("aura_r", "aura", 2));
    assert_eq!(a.tick(), b.tick());
    assert_ne!(a.state_hash(), b.state_hash(), "the level differs");
}

// --- Effects together --------------------------------------------------------------

/// A stage where the skill `it` is equipped, then two enemies stand 10 pixels right of the test
/// skills' point and 120 pixels left of it; it returns once both are there.
fn two_standing(skills: Vec<Value>) -> Engine {
    let point_y = START_Y - AHEAD;
    let waves = vec![
        standing(40, 50, START_X + 2_560, point_y),
        standing(40, 50, START_X - 30_720, point_y),
    ];
    let mut e = equipped_skill(&bare(waves), skills, ("it_r", "it", 1));
    while enemies_at(&e).len() < 2 {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    e
}

#[test]
fn a_level_s_effects_apply_in_list_order() {
    let player = EntityId(0);
    let mut e = two_standing(vec![quick(
        "it",
        &[json!([buff(10), area("slow", 16_384)])],
        1,
    )]);
    let near = enemies_at(&e)[0].0;
    assert_eq!(
        applied(&use_skill(&mut e)),
        [(player, "atk_up".into()), (near, "slow".into())]
    );
    let mut e = two_standing(vec![quick(
        "it",
        &[json!([area("slow", 16_384), buff(10)])],
        1,
    )]);
    let near = enemies_at(&e)[0].0;
    assert_eq!(
        applied(&use_skill(&mut e)),
        [(near, "slow".into()), (player, "atk_up".into())]
    );
}

#[test]
fn a_pull_and_an_area_status_act_at_the_one_point() {
    let mut e = two_standing(vec![quick(
        "it",
        &[json!([pull(16_384, 256), area("vulnerable", 16_384)])],
        1,
    )]);
    let (near, far) = (enemies_at(&e)[0], enemies_at(&e)[1]);
    e.step(&[pressing_skill(), Default::default(), Default::default()]);
    assert_eq!(
        applied(e.events()),
        [(near.0, "vulnerable".to_string())],
        "only the near one is within the radius"
    );
    assert_eq!(enemies_at(&e)[0].1, near.1 - 256, "and pulled");
    assert_eq!(enemies_at(&e)[1], far);
    let view = e.snapshot().players[0].pull.expect("a pull");
    assert_eq!((view.at.x.0, view.at.y.0), (START_X, START_Y - AHEAD));
}

#[test]
fn a_new_pull_shield_or_attack_replaces_the_one_still_going() {
    let attack = json!({ "type": "attack", "attack": { "inline": [
        { "kind": "active", "ticks": 60, "damage": 1,
          "hit_box": { "type": "circle", "offset": { "forward": AHEAD }, "radius": 1_024 } }
    ] } });
    let skills = vec![quick(
        "it",
        &[json!([pull(4_096, 0), attack, shield(3, DURATION)])],
        5,
    )];
    let mut e = equipped_skill(&bare(vec![]), skills, ("it_r", "it", 1));
    use_skill(&mut e);
    for _ in 0..10 {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    let p = e.snapshot().players[0].clone();
    assert_eq!(p.pull.unwrap().ticks_left, DURATION - 11);
    assert_eq!(p.shield.unwrap().ticks_left, DURATION - 11);
    e.step(&[pressing_skill(), Default::default(), Default::default()]);
    let p = e.snapshot().players[0].clone();
    assert_eq!(p.pull.unwrap().ticks_left, DURATION, "a new pull");
    assert_eq!(p.shield.unwrap().ticks_left, DURATION, "a new shield");
    let skill_runs: Vec<u32> = p
        .attacks
        .iter()
        .filter(|a| a.slot == 1)
        .map(|a| a.tick)
        .collect();
    assert_eq!(skill_runs, [0], "one attack in the slot, the new one");
}

#[test]
fn a_stage_skill_attack_names_its_damage_type_in_the_hit() {
    let area = json!({ "type": "attack", "attack": { "inline": [
        { "kind": "active", "ticks": 20, "damage": 7, "type": "cryo",
          "hit_box": { "type": "circle", "offset": { "forward": AHEAD }, "radius": 8_192 } }
    ] } });
    let point_y = START_Y - AHEAD;
    let waves = vec![standing(40, 50, START_X, point_y)];
    let skills = vec![quick("nova", &[area], 1)];
    let mut e = equipped_skill(&bare(waves), skills, ("nova_r", "nova", 1));
    while enemies_at(&e).is_empty() {
        e.step(&[idle(), Default::default(), Default::default()]);
    }
    e.step(&[pressing_skill(), Default::default(), Default::default()]);
    let types: Vec<_> = e
        .events()
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::Hit {
                damage: 7,
                damage_type,
                ..
            } => Some(*damage_type),
            _ => None,
        })
        .collect();
    assert_eq!(
        types,
        [Some(stella_rain_core::attack::DamageType::Cryo)],
        "{:?}",
        e.events()
    );
}
