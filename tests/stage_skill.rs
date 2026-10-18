//! The stage skill (ADR-075): one slot, empty at the start of a run, filled from falling stars;
//! a press of `skill` uses a ready skill at its level, and each skill has its own cooldown,
//! counted from its use. Taking another skill replaces the one held and arrives ready; taking
//! the same one again raises its level. Its kinds: black hole, buff, debuff area, damage area
//! and shield. Every number here is worked out by hand from the rules' constants.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{ContentId, EntityId};
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::snapshot::{EntityKind, OfferOption, SkillKind};
use stella_rain_core::stage::Stage;

mod common;

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

/// A skill `id` acting `AHEAD` pixels ahead, whose levels have these effects, each with the
/// cooldown `COOLDOWN` plus 10 ticks a level less.
fn skill(id: &str, effects: &[Value]) -> Value {
    let levels: Vec<Value> = effects
        .iter()
        .enumerate()
        .map(|(i, e)| json!({ "cooldown_ticks": COOLDOWN - 10 * i as u32, "effect": e }))
        .collect();
    json!({ "id": id, "ahead": AHEAD, "levels": levels })
}

/// A rewards file of `skills` and, for each `(reward, skill, amount)`, a common reward that
/// gives that skill, five times a run.
fn content(skills: Vec<Value>, rewards: &[(&str, &str, u32)]) -> Rewards {
    let rewards: Vec<Value> = rewards
        .iter()
        .map(|(id, skill, amount)| {
            json!({ "id": id, "rarity": "common", "effect": "stage_skill", "skill": skill,
                    "amount": amount, "limit": 5, "title": "Skill", "description": "" })
        })
        .collect();
    let file = json!({ "schema_version": 0, "sim_version": 0, "rewards": rewards,
                       "stage_skills": skills });
    Rewards::parse(file.to_string().as_bytes()).expect("the test rewards validate")
}

fn with(stage: &Stage, rewards: Rewards) -> Engine {
    let content = RunContent::new(RunStats::bundled().clone(), rewards);
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

fn black_hole(radius: i32, pull: i32) -> Value {
    json!({ "type": "black_hole", "radius": radius, "pull": pull, "duration_ticks": DURATION })
}

fn buff(pct: u16) -> Value {
    json!({ "type": "buff", "atk_pct": pct, "duration_ticks": DURATION })
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
    assert_eq!(view.kind, SkillKind::Buff);
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
    assert_eq!(view.kind, SkillKind::Shield);
    assert_eq!((view.level, view.ready, view.cooldown_left), (1, true, 0));
}

// --- The kinds -------------------------------------------------------------------------------

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
fn a_debuff_area_makes_the_enemies_near_its_point_vulnerable() {
    let debuff = json!({ "type": "debuff_area", "radius": 16_384, "vulnerable_pct": 25,
                         "duration_ticks": DURATION });
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
    let marked: Vec<EntityId> = events
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::StatusApplied { target, status, .. } if status.0 == "vulnerable" => {
                Some(*target)
            }
            _ => None,
        })
        .collect();
    assert_eq!(marked, [near]);
}

#[test]
fn a_black_hole_pulls_the_enemies_near_its_point_to_it_and_holds_them_there() {
    // 10 pixels right of the point, pulled 1 pixel a tick: there in 10 ticks, and held.
    let point_y = START_Y - AHEAD;
    let mut e = with(
        &starry(vec![
            targets(1),
            standing(40, 50, START_X + 2_560, point_y),
            standing(40, 50, START_X - 51_200, point_y),
        ]),
        content(
            vec![skill("well", &[black_hole(16_384, 256)])],
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
    let view = e.snapshot().player.black_hole.expect("a black hole");
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
    assert!(e.snapshot().player.black_hole.is_some(), "its last tick");
    e.step(idle());
    assert_eq!(e.snapshot().player.black_hole, None, "over after its time");
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
                    "attack": { "preset": { "id": "aimed_single", "args": [768] } } }),
            70
        )]),
    );
    let mut e = with(
        &starry(vec![
            targets(1),
            wave(60, 1, 0, shooter, START_X, START_Y - 40_000),
        ]),
        content(
            vec![skill("ward", &[shield(1, 600)])],
            &[("ward_r", "ward", 1)],
        ),
    );
    take(&mut e, "ward_r");
    let hp = e.snapshot().player.hp;
    use_skill(&mut e);
    assert_eq!(
        e.snapshot().player.shield.map(|s| s.amount),
        Some(1),
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
    // The first shot's 1 damage is absorbed; the shield is gone, and the next one hurts.
    assert_eq!(hits.first(), Some(&hp));
    assert!(hits.iter().any(|&h| h < hp), "{hits:?}");
    assert_eq!(e.snapshot().player.shield, None);
}

#[test]
fn a_damage_area_attacks_in_its_own_slot_beside_a_combo_step() {
    let area = json!({ "type": "damage_area", "attack": { "inline": [
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
    stage["player"]["main"]["inline"]["assist_range"] = json!(65_536);
    let mut e = with(
        &serde_json::from_value(stage).unwrap(),
        content(
            vec![skill("well", &[black_hole(4_096, 0)])],
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
    let view = e.snapshot().player.black_hole.expect("a black hole");
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
