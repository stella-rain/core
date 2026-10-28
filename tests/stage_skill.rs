//! The stage skill (ADR-075, ADR-098): one slot, empty at the start of a run, filled from
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
use stella_rain_core::snapshot::{EntityKind, OfferOption};
use stella_rain_core::stage::Stage;
use stella_rain_core::stage_skill::StageSkills;
use stella_rain_core::version::Versions;

use crate::common;

use common::*;

/// 32 pixels above the main character: inside the pickup circle, so a star dropped here is
/// taken at once.
const NEAR_Y: i32 = START_Y - 8_192;
/// How far ahead the test skills act: 100 pixels.
const AHEAD: i32 = 25_600;
const COOLDOWN: u32 = 120;
const DURATION: u32 = 60;

/// `count` enemies of 10 hp (one shot each), one every 6 ticks from tick 1, straight above the
/// main character in the line of its shots: each drops a star it takes at once.
fn targets(count: u16) -> Value {
    wave(
        1,
        count,
        6,
        enemy(10, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )
}

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

/// Fires until an offer is open, then takes the candidate that is `reward`.
fn take(e: &mut Engine, reward: &str) {
    fire_until(e, idle(), 1200, Engine::offer_open);
    let offer = e.snapshot().progress.unwrap().offer.expect("an open offer");
    let candidate = offer
        .candidates
        .iter()
        .position(|c| c.reward.0 == reward)
        .unwrap_or_else(|| panic!("{reward} is not offered"));
    let option = OfferOption::Take {
        candidate: u8::try_from(candidate).unwrap(),
    };
    let index = offer.options.iter().position(|o| *o == option).unwrap();
    e.choose(u32::try_from(index).unwrap()).unwrap();
}

fn pressing_skill() -> Input {
    Input {
        skill: true,
        ..idle()
    }
}

/// Presses the skill on one tick, then lets go on the next.
fn use_skill(e: &mut Engine) -> Vec<DomainEvent> {
    e.step(pressing_skill());
    let events = e.events().to_vec();
    e.step(idle());
    events
}

fn used(events: &[DomainEvent]) -> Option<(String, u8)> {
    events.iter().find_map(|ev| match ev {
        DomainEvent::StageSkillUsed { skill, level } => Some((skill.0.clone(), *level)),
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
            DomainEvent::StatusApplied { target, status, .. } => Some((*target, status.0.clone())),
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
    assert_eq!(e.snapshot().player.skill, None);
    e.step(pressing_skill());
    assert_eq!(used(e.events()), None);
}

#[test]
fn a_skill_taken_arrives_ready_at_the_level_its_reward_gives() {
    let skills = vec![skill("aura", &[buff(10), buff(20), buff(30)])];
    let mut e = with(
        &starry(vec![targets(1)]),
        content(skills, &[("aura_r", "aura", 2)]),
    );
    take(&mut e, "aura_r");
    let view = e.snapshot().player.skill.expect("a skill in the slot");
    assert_eq!(view.skill, ContentId("aura".into()));
    assert_eq!(view.icon, ContentId(ICON.into()));
    assert_eq!((view.level, view.ready, view.cooldown_left), (2, true, 0));
}

#[test]
fn a_press_uses_it_and_its_level_s_cooldown_counts_from_the_use() {
    let skills = vec![skill("aura", &[buff(10), buff(20)])];
    let mut e = with(
        &starry(vec![targets(1)]),
        content(skills, &[("aura_r", "aura", 2)]),
    );
    take(&mut e, "aura_r");
    e.step(pressing_skill());
    let at = e.tick();
    assert_eq!(used(e.events()), Some(("aura".into(), 2)));
    e.step(idle());
    // Level 2's cooldown is 110 ticks; one tick has passed since the use.
    let view = e.snapshot().player.skill.unwrap();
    assert_eq!((view.ready, view.cooldown_left), (false, COOLDOWN - 10 - 1));
    // Pressed on every other tick from then on: used again 110 ticks after the first use.
    let mut again = None;
    for _ in 0..COOLDOWN {
        e.step(pressing_skill());
        if again.is_none() && used(e.events()).is_some() {
            again = Some(e.tick());
        }
        e.step(idle());
    }
    assert_eq!(again, Some(at + COOLDOWN - 10));
}

#[test]
fn the_same_skill_again_raises_its_level_up_to_its_highest() {
    let skills = vec![skill("aura", &[buff(10), buff(20)])];
    let mut e = with(
        &starry(vec![targets(3)]),
        content(skills, &[("aura_r", "aura", 1)]),
    );
    let mut levels = Vec::new();
    for _ in 0..3 {
        take(&mut e, "aura_r");
        levels.push(e.snapshot().player.skill.unwrap().level);
    }
    assert_eq!(levels, [1, 2, 2]);
}

#[test]
fn the_same_skill_taken_while_it_cools_down_keeps_cooling_down() {
    let skills = vec![skill("aura", &[buff(10), buff(20)])];
    let mut e = with(
        &starry(vec![targets(1), standing(60, 10, START_X, NEAR_Y)]),
        content(skills, &[("aura_r", "aura", 1)]),
    );
    take(&mut e, "aura_r");
    use_skill(&mut e);
    take(&mut e, "aura_r");
    let view = e.snapshot().player.skill.unwrap();
    assert_eq!(view.level, 2);
    assert!(!view.ready && view.cooldown_left > 0);
}

#[test]
fn another_skill_replaces_the_one_held_and_arrives_ready() {
    let skills = vec![
        skill("aura", &[buff(10)]),
        skill("ward", &[shield(5, DURATION), shield(10, DURATION)]),
    ];
    let mut e = with(
        &starry(vec![targets(1), standing(60, 10, START_X, NEAR_Y)]),
        content(skills, &[("aura_r", "aura", 1), ("ward_r", "ward", 1)]),
    );
    take(&mut e, "aura_r");
    use_skill(&mut e);
    take(&mut e, "ward_r");
    let view = e.snapshot().player.skill.unwrap();
    assert_eq!(view.skill, ContentId("ward".into()));
    assert_eq!(view.icon, ContentId(ICON.into()));
    assert_eq!((view.level, view.ready, view.cooldown_left), (1, true, 0));
}

// --- The building blocks ---------------------------------------------------------------------

#[test]
fn a_buff_raises_the_main_character_s_attack_for_its_time() {
    let skills = vec![skill("aura", &[buff(50)])];
    let mut e = with(
        &starry(vec![targets(1)]),
        content(skills, &[("aura_r", "aura", 1)]),
    );
    take(&mut e, "aura_r");
    let events = use_skill(&mut e);
    assert!(events.contains(&DomainEvent::StatusApplied {
        target: EntityId(0),
        status: ContentId("atk_up".into()),
        stacks: 1,
        duration_ticks: DURATION,
    }));
}

#[test]
fn an_area_status_makes_the_enemies_near_its_point_vulnerable() {
    let debuff = area("vulnerable", 16_384);
    // The point is 100 pixels up the map from the main character: one enemy 40 pixels from it,
    // one 120 pixels from it.
    let point_y = START_Y - AHEAD;
    let mut e = with(
        &starry(vec![
            targets(1),
            standing(40, 50, START_X + 10_240, point_y),
            standing(40, 50, START_X - 30_720, point_y),
        ]),
        content(vec![skill("hex", &[debuff])], &[("hex_r", "hex", 1)]),
    );
    take(&mut e, "hex_r");
    while enemies_at(&e).len() < 2 {
        e.step(idle());
    }
    let near = enemies_at(&e)[0].0;
    let events = use_skill(&mut e);
    assert_eq!(applied(&events), [(near, "vulnerable".to_string())]);
}

#[test]
fn a_pull_draws_the_enemies_near_its_point_to_it_and_holds_them_there() {
    // 10 pixels right of the point, pulled 1 pixel a tick: there in 10 ticks, and held.
    let point_y = START_Y - AHEAD;
    let mut e = with(
        &starry(vec![
            targets(1),
            standing(40, 50, START_X + 2_560, point_y),
            standing(40, 50, START_X - 51_200, point_y),
        ]),
        content(
            vec![skill("well", &[pull(16_384, 256)])],
            &[("well_r", "well", 1)],
        ),
    );
    take(&mut e, "well_r");
    while enemies_at(&e).len() < 2 {
        e.step(idle());
    }
    let far = enemies_at(&e)[1];
    e.step(pressing_skill());
    assert_eq!(enemies_at(&e)[0].1, START_X + 2_560 - 256, "the first pull");
    let view = e.snapshot().player.pull.expect("a pull");
    assert_eq!((view.at.x.0, view.at.y.0), (START_X, point_y));
    assert_eq!((view.radius.0, view.ticks_left), (16_384, DURATION));
    for _ in 0..20 {
        e.step(idle());
    }
    let near = enemies_at(&e)[0];
    assert_eq!((near.1, near.2), (START_X, point_y), "pulled in and held");
    assert_eq!(enemies_at(&e)[1], far, "out of its radius");
    for _ in 21..DURATION {
        e.step(idle());
    }
    assert!(e.snapshot().player.pull.is_some(), "its last tick");
    e.step(idle());
    assert_eq!(e.snapshot().player.pull, None, "over after its time");
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
    let mut e = with(
        &starry(vec![
            targets(1),
            wave(60, 1, 0, shooter, START_X, START_Y - 40_000),
        ]),
        content(
            vec![skill("ward", &[shield(100, 600)])],
            &[("ward_r", "ward", 1)],
        ),
    );
    take(&mut e, "ward_r");
    let hp = e.snapshot().player.hp;
    use_skill(&mut e);
    assert_eq!(
        e.snapshot().player.shield.map(|s| s.amount),
        Some(100),
        "the shield is up"
    );
    let mut hits = Vec::new();
    for _ in 0..200 {
        e.step(idle());
        for ev in e.events() {
            if let DomainEvent::PlayerHit { hp_left } = ev {
                hits.push(*hp_left);
            }
        }
    }
    // The first shot's 100 damage (the default, 1 before the scale of ADR-103) is absorbed; the shield is gone, and the next one hurts.
    assert_eq!(hits.first(), Some(&hp));
    assert!(hits.iter().any(|&h| h < hp), "{hits:?}");
    assert_eq!(e.snapshot().player.shield, None);
}

#[test]
fn an_attack_runs_in_its_own_slot_beside_a_combo_step() {
    let area = json!({ "type": "attack", "attack": { "inline": [
        { "kind": "active", "ticks": 20, "damage": 7,
          "hit_box": { "type": "circle", "offset": { "forward": AHEAD }, "radius": 8_192 } }
    ] } });
    let point_y = START_Y - AHEAD;
    let mut e = with(
        &starry(vec![targets(1), standing(40, 50, START_X, point_y)]),
        content(vec![skill("nova", &[area])], &[("nova_r", "nova", 1)]),
    );
    take(&mut e, "nova_r");
    while enemies_at(&e).is_empty() {
        e.step(idle());
    }
    e.step(Input {
        main: true,
        skill: true,
        ..idle()
    });
    assert!(
        e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ComboStep { .. })),
        "the combo step on the same tick"
    );
    let slots: Vec<u8> = e.snapshot().player.attacks.iter().map(|a| a.slot).collect();
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
    let mut stage = serde_json::to_value(starry(vec![
        targets(1),
        standing(40, 50, START_X + 51_200, START_Y),
    ]))
    .unwrap();
    stage["player"]["weapon"]["inline"]["assist_range"] = json!(65_536);
    let mut e = with(
        &serde_json::from_value(stage).unwrap(),
        content(
            vec![skill("well", &[pull(4_096, 0)])],
            &[("well_r", "well", 1)],
        ),
    );
    take(&mut e, "well_r");
    while enemies_at(&e).is_empty() {
        e.step(idle());
    }
    e.step(Input {
        assist: true,
        ..pressing_skill()
    });
    let view = e.snapshot().player.pull.expect("a pull");
    assert_eq!((view.at.x.0, view.at.y.0), (START_X + AHEAD, START_Y));
}

#[test]
fn the_slot_is_in_the_state_hash() {
    let skills = vec![skill("aura", &[buff(10), buff(20)])];
    let mut a = with(
        &starry(vec![targets(1)]),
        content(skills.clone(), &[("aura_r", "aura", 1)]),
    );
    let mut b = with(
        &starry(vec![targets(1)]),
        content(skills, &[("aura_r", "aura", 2)]),
    );
    take(&mut a, "aura_r");
    take(&mut b, "aura_r");
    assert_eq!(a.tick(), b.tick());
    assert_ne!(a.state_hash(), b.state_hash(), "the level differs");
}

// --- Effects together (ADR-098) --------------------------------------------------------------

/// A stage where the skill `it` is taken from a star, then two enemies stand 10 pixels right of
/// the test skills' point and 120 pixels left of it; it returns once both are there.
fn two_standing(skills: Vec<Value>) -> Engine {
    let point_y = START_Y - AHEAD;
    let mut e = with(
        &starry(vec![
            targets(1),
            standing(40, 50, START_X + 2_560, point_y),
            standing(40, 50, START_X - 30_720, point_y),
        ]),
        content(skills, &[("it_r", "it", 1)]),
    );
    take(&mut e, "it_r");
    while enemies_at(&e).len() < 2 {
        e.step(idle());
    }
    e
}

#[test]
fn a_level_s_effects_apply_in_list_order() {
    let player = EntityId(0);
    let mut e = two_standing(vec![skill(
        "it",
        &[json!([buff(10), area("slow", 16_384)])],
    )]);
    let near = enemies_at(&e)[0].0;
    assert_eq!(
        applied(&use_skill(&mut e)),
        [(player, "atk_up".into()), (near, "slow".into())]
    );
    let mut e = two_standing(vec![skill(
        "it",
        &[json!([area("slow", 16_384), buff(10)])],
    )]);
    let near = enemies_at(&e)[0].0;
    assert_eq!(
        applied(&use_skill(&mut e)),
        [(near, "slow".into()), (player, "atk_up".into())]
    );
}

#[test]
fn a_pull_and_an_area_status_act_at_the_one_point() {
    let mut e = two_standing(vec![skill(
        "it",
        &[json!([pull(16_384, 256), area("vulnerable", 16_384)])],
    )]);
    let (near, far) = (enemies_at(&e)[0], enemies_at(&e)[1]);
    e.step(pressing_skill());
    assert_eq!(
        applied(e.events()),
        [(near.0, "vulnerable".to_string())],
        "only the near one is within the radius"
    );
    assert_eq!(enemies_at(&e)[0].1, near.1 - 256, "and pulled");
    assert_eq!(enemies_at(&e)[1], far);
    let view = e.snapshot().player.pull.expect("a pull");
    assert_eq!((view.at.x.0, view.at.y.0), (START_X, START_Y - AHEAD));
}

#[test]
fn each_level_brings_its_own_effects() {
    let skills = vec![skill(
        "aura",
        &[json!([buff(10)]), json!([buff(20), shield(4, DURATION)])],
    )];
    let mut e = with(
        &starry(vec![targets(1), standing(60, 10, START_X, NEAR_Y)]),
        content(skills, &[("aura_r", "aura", 1)]),
    );
    take(&mut e, "aura_r");
    use_skill(&mut e);
    assert_eq!(e.snapshot().player.shield, None, "level 1 has no shield");
    take(&mut e, "aura_r");
    for _ in 0..COOLDOWN {
        e.step(idle());
    }
    let events = use_skill(&mut e);
    assert_eq!(used(&events), Some(("aura".into(), 2)));
    assert_eq!(applied(&events), [(EntityId(0), "atk_up".to_string())]);
    assert_eq!(e.snapshot().player.shield.map(|s| s.amount), Some(4));
}

#[test]
fn a_new_pull_shield_or_attack_replaces_the_one_still_going() {
    let attack = json!({ "type": "attack", "attack": { "inline": [
        { "kind": "active", "ticks": 60, "damage": 1,
          "hit_box": { "type": "circle", "offset": { "forward": AHEAD }, "radius": 1_024 } }
    ] } });
    let mut e = with(
        &starry(vec![targets(1)]),
        content(
            vec![quick(
                "it",
                &[json!([pull(4_096, 0), attack, shield(3, DURATION)])],
                5,
            )],
            &[("it_r", "it", 1)],
        ),
    );
    take(&mut e, "it_r");
    use_skill(&mut e);
    for _ in 0..10 {
        e.step(idle());
    }
    let p = e.snapshot().player;
    assert_eq!(p.pull.unwrap().ticks_left, DURATION - 11);
    assert_eq!(p.shield.unwrap().ticks_left, DURATION - 11);
    e.step(pressing_skill());
    let p = e.snapshot().player;
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
    let mut e = with(
        &starry(vec![targets(1), standing(40, 50, START_X, point_y)]),
        content(vec![skill("nova", &[area])], &[("nova_r", "nova", 1)]),
    );
    take(&mut e, "nova_r");
    while enemies_at(&e).is_empty() {
        e.step(idle());
    }
    e.step(pressing_skill());
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
