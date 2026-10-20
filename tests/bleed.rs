//! The bleed rune and the damage-over-time status (ADR-099 points 2, 3, 7 and 8; ADR-100 point
//! 1; ADR-084 point 8): when a combo step of the Main weapon hits and the main character holds
//! bleed, the enemy hit gets the `bleed` status, or a stack more of it, up to the stack maximum,
//! and its duration starts again. Every interval it deals its damage per stack times its stacks:
//! the main character's damage without a combo step, through the atk-up and the Slash bonus
//! fixed when it was last applied, then the target's match-up and defence at the tick. Its
//! damage gives no expertise, fires no rune and applies no bleed. Every number here is worked
//! out by hand. The on-hit runes are held too, as the random rune draws all seven, but their
//! attacks land far from every target (`far`), and poison and burn never tick (`far_dots`).

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::{RunRecorder, verify_run_with};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::runes::Runes;
use stella_rain_core::snapshot::{DamageOverTimeView, StatusView};

use crate::common;

use common::on_hit::*;
use common::run::{run_input, trial_setup};
use common::*;

/// A runes file whose bleed ticks every `interval` for `duration`, `damage` a stack, up to
/// `max` stacks; every on-hit rune lands far away.
fn runes(interval: u32, duration: u32, damage: u32, max: u32) -> Runes {
    runes_with(
        vec![
            far("explosion"),
            far("thunder"),
            far("boomerang"),
            far("homing_missiles"),
        ],
        far_dots_but(dot("bleed", interval, duration, damage, max)),
    )
}

/// The stage with seven small enemies and the tough target, every rune taken from their stars.
fn holding(runes: Runes, target: Value) -> Engine {
    let mut e = with(
        &stage(7, target),
        &content(vec![random_rune()], None, runes),
    );
    take_all(&mut e);
    e
}

/// The bleed statuses on `id`; poison and burn, held too, are beside them.
fn statuses(e: &Engine, id: EntityId) -> Vec<DamageOverTimeView> {
    e.snapshot()
        .entities
        .into_iter()
        .find(|en| en.id == id)
        .map(|en| en.damage_over_time)
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.status.0 == "bleed")
        .collect()
}

/// Presses of `main` on the ticks `start + d` for each `d` in `at`; each hits on the tick
/// after.
fn presses(start: u32, at: &'static [u32]) -> impl Fn(u32) -> Input {
    move |t| Input {
        main: at.iter().any(|d| start + d == t),
        ..idle()
    }
}

#[test]
fn a_combo_step_hit_applies_bleed() {
    let mut e = holding(runes(30, 180, 3, 5), tough(0, None, None));
    let target = wait_for_target(&mut e);
    assert!(statuses(&e, target).is_empty());
    let start = e.tick();
    e.step(once(start)(start));
    assert!(e.events().iter().any(|ev| matches!(
        ev,
        DomainEvent::Hit { target: t, damage: 10, .. } if *t == target
    )));
    assert!(e.events().iter().any(|ev| matches!(
        ev,
        DomainEvent::StatusApplied { target: t, status, stacks: 1, duration_ticks: 180 }
            if *t == target && status.0 == "bleed"
    )));
    let bleed = statuses(&e, target);
    assert_eq!(bleed.len(), 1);
    assert_eq!(bleed[0].status.0, "bleed");
    assert_eq!(
        (bleed[0].stacks, bleed[0].ticks_left, bleed[0].next_tick_in),
        (1, 180, 30)
    );
}

#[test]
fn bleed_ticks_at_its_interval_for_its_duration() {
    let mut e = holding(runes(30, 180, 3, 5), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 240, once(start));
    // The press on `start` hits on the tick after.
    let hit = start + 1;
    assert_eq!(of(&seen, 10), [hit], "the step's hit");
    let ticks: Vec<u32> = (1..=6).map(|k| hit + 30 * k).collect();
    assert_eq!(of(&seen, 3), ticks, "180 / 30 ticks of 3: {seen:?}");
    assert_eq!(seen.len(), 7, "nothing else: {seen:?}");
    assert!(statuses(&e, target).is_empty(), "run out");
}

#[test]
fn bleed_stacks_to_its_maximum_and_each_hit_starts_its_duration_again() {
    // Hits at 0, 10, 20 and 40 (ticks after the first): 3 stacks from the third, the fourth held at the maximum. The
    // tick timer runs on, so it ticks at 30, 60, ..., and the last hit's 180 ticks end at 220.
    let mut e = holding(runes(30, 180, 3, 3), tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let mut stacks = Vec::new();
    let mut seen = Vec::new();
    for _ in 0..=240 {
        let t = e.tick();
        e.step(presses(start, &[0, 10, 20, 40])(t));
        for ev in e.events() {
            match ev {
                DomainEvent::Hit {
                    target: id, damage, ..
                } if *id == target => seen.push((e.tick() - start - 1, *damage)),
                DomainEvent::StatusApplied {
                    target: id,
                    status,
                    stacks: n,
                    duration_ticks,
                } if *id == target && status.0 == "bleed" => {
                    assert_eq!(*duration_ticks, 180);
                    stacks.push(*n);
                }
                _ => {}
            }
        }
        if e.tick() == start + 41 {
            let bleed = statuses(&e, target);
            assert_eq!(bleed.len(), 1, "one bleed on the enemy");
            assert_eq!((bleed[0].stacks, bleed[0].ticks_left), (3, 180));
            assert_eq!(bleed[0].next_tick_in, 20, "the timer is not reset");
        }
    }
    assert_eq!(stacks, [1, 2, 3, 3]);
    let steps: Vec<u32> = seen.iter().filter(|h| h.1 == 10).map(|h| h.0).collect();
    assert_eq!(steps, [0, 10, 20, 40]);
    let bleeds: Vec<u32> = seen.iter().filter(|h| h.1 != 10).map(|h| h.0).collect();
    assert_eq!(bleeds, [30, 60, 90, 120, 150, 180, 210], "{seen:?}");
    assert!(
        seen.iter().all(|h| h.1 == 10 || h.1 == 9),
        "3 × 3: {seen:?}"
    );
}

#[test]
fn each_enemy_has_its_own_bleed() {
    // A second target 8 pixels to the right, within the step's reach.
    let beside = wave(
        TARGET_TICK,
        1,
        0,
        enemy(100_000, hold(START_X + 2_048, NEAR_Y), json!([])),
        START_X + 2_048,
        NEAR_Y,
    );
    let mut s = stage(7, tough(0, None, None));
    s.waves.push(serde_json::from_value(beside).unwrap());
    let mut e = with(
        &s,
        &content(vec![random_rune()], None, runes(30, 180, 3, 5)),
    );
    take_all(&mut e);
    while e.tick() < TARGET_TICK {
        e.step(idle());
    }
    let start = e.tick();
    for t in start..start + 15 {
        e.step(presses(start, &[0, 10])(t));
    }
    let bleeding: Vec<(EntityId, u16)> = e
        .snapshot()
        .entities
        .iter()
        .filter_map(|en| {
            let bleed = en.damage_over_time.iter().find(|d| d.status.0 == "bleed");
            bleed.map(|d| (en.id, d.stacks))
        })
        .collect();
    assert_eq!(bleeding.len(), 2, "{bleeding:?}");
    assert!(bleeding.iter().all(|b| b.1 == 2), "{bleeding:?}");
}

#[test]
fn an_orbit_s_hits_apply_no_bleed() {
    // An orb's circle 40 pixels out, 7 damage, on a target out of the step's reach, which is
    // never pressed.
    let orbit = json!({
        "id": "orbit", "rarity": "common", "effect": "orbit", "amount": 1, "limit": 1,
        "title": "Orbit", "description": "",
        "orbit": {
            "attack": { "inline": [
                { "kind": "active", "ticks": 60, "damage": 7, "rehit_ticks": 10,
                  "hit_box": { "type": "circle", "offset": { "forward": 10_240 },
                               "radius": 1_024 } }
            ] },
            "turn_per_tick": 15 * 256,
            "cooldown_ticks": 60
        }
    });
    let target = json!(wave(
        TARGET_TICK,
        1,
        0,
        enemy(
            100_000,
            json!({ "type": "straight", "vx": 0, "vy": 0 }),
            json!([])
        ),
        START_X + 10_240,
        START_Y
    ));
    let mut e = with(
        &stage(8, target),
        &content(vec![random_rune(), orbit], None, runes(30, 180, 3, 5)),
    );
    take_all(&mut e);
    take(&mut e, "orbit");
    let target = wait_for_target(&mut e);
    let seen = hits(&mut e, target, 240, |_| idle());
    assert!(!of(&seen, 7).is_empty(), "the orb hits: {seen:?}");
    assert!(of(&seen, 3).is_empty(), "{seen:?}");
    assert!(statuses(&e, target).is_empty());
}

#[test]
fn an_on_hit_rune_s_hits_apply_no_bleed_and_bleed_fires_no_rune() {
    // Explosion lands on the target, 7 damage, its cooldown 2: had a bleed tick fired it, a
    // second blast would land; had the blast applied bleed, it would hold 2 stacks.
    let blast = json!([{ "kind": "active", "ticks": 2,
                         "hit_box": { "type": "circle", "radius": 4_096 } }]);
    let runes = runes_with(
        vec![
            entry("explosion", 2, 7, blast),
            far("thunder"),
            far("boomerang"),
            far("homing_missiles"),
        ],
        far_dots_but(dot("bleed", 30, 180, 3, 5)),
    );
    let mut e = holding(runes, tough(0, None, None));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 20, once(start));
    assert_eq!(of(&seen, 7).len(), 1, "the blast landed: {seen:?}");
    assert_eq!(statuses(&e, target)[0].stacks, 1);
    let later = hits(&mut e, target, 200, |_| idle());
    assert_eq!(of(&later, 3).len(), 6, "{later:?}");
    assert!(
        of(&later, 7).is_empty(),
        "no blast from a bleed tick: {later:?}"
    );
}

#[test]
fn bleed_gives_no_expertise() {
    // The target gives 1 point a hit: the step's, not the bleed's six.
    let mut e = holding(runes(30, 180, 3, 5), tough(0, None, Some(1)));
    let target = wait_for_target(&mut e);
    let start = e.tick();
    let seen = hits(&mut e, target, 200, once(start));
    assert_eq!(of(&seen, 3).len(), 6, "{seen:?}");
    assert_eq!(e.snapshot().expertise.unwrap().points, 1);
}

/// Bleed of 10 a stack, one stack at most, on a target weak to Slash with a defence of 100,
/// holding a Slash bonus of 50 and the war cry's atk-up of 100 for 20 ticks, used before the
/// first step; a second step after the atk-up has run out. The hits on the target, by ticks
/// after the first step.
fn boosted(ty: &str) -> Vec<(u32, u32)> {
    let war_cry = json!({ "id": "war_cry", "icon": "icon_nova", "title": "War Cry",
        "description": "",
        "levels": [{ "cooldown_ticks": 1_000, "effects": [
            { "type": "self_status", "status": "atk_up", "value_pct": 100,
              "duration_ticks": 20 }
        ] }] });
    let rewards = vec![
        random_rune(),
        json!({ "id": "war_cry_r", "rarity": "common", "effect": "stage_skill",
                "skill": "war_cry", "amount": 1, "limit": 1, "title": "Skill",
                "description": "" }),
        json!({ "id": "bonus", "rarity": "common", "effect": "type_bonus_pct", "type": ty,
                "amount": 50, "limit": 1, "title": "Bonus", "description": "" }),
    ];
    let mut target = tough(0, Some(json!({ "weak": ["slash"] })), None);
    // K is 10,000 after the scale of ADR-103: a defence of K halves a hit.
    target["enemy"]["defence"] = json!(10_000);
    let mut e = with(
        &stage(9, target),
        &content(rewards, Some(json!([war_cry])), runes(30, 180, 10, 1)),
    );
    take_all(&mut e);
    take(&mut e, "war_cry_r");
    take(&mut e, "bonus");
    let target = wait_for_target(&mut e);
    e.step(Input {
        skill: true,
        ..idle()
    });
    let start = e.tick();
    let seen = hits(&mut e, target, 100, presses(start, &[0, 45]));
    seen.into_iter().map(|(t, d)| (t - start - 1, d)).collect()
}

#[test]
fn bleed_s_damage_goes_through_atk_up_the_slash_bonus_the_match_up_and_defence() {
    // The step, neutral, with atk-up: 10 × 2 = 20, × 100 / 200 = 10; without: 10 / 2 = 5.
    // Bleed, Slash, applied with atk-up: 10 × 2 = 20, × 1.5 = 30, × 1.2 = 36, / 2 = 18, at 30
    // after the atk-up has run out; applied again at 45 without it: 10 × 1.5 = 15, × 1.2 = 18,
    // / 2 = 9, at 60 and on.
    assert_eq!(
        boosted("slash"),
        [(0, 10), (30, 18), (45, 5), (60, 9), (90, 9)]
    );
}

#[test]
fn another_type_s_bonus_does_not_raise_bleed() {
    // A Pyro bonus: bleed 10 × 2 = 20, × 1.2 = 24, / 2 = 12; then 10 × 1.2 = 12, / 2 = 6.
    assert_eq!(
        boosted("pyro"),
        [(0, 10), (30, 12), (45, 5), (60, 6), (90, 6)]
    );
}

#[test]
fn the_boss_bleeds_too() {
    // The boss 32 pixels above the main character, in the step's reach, beside the small
    // enemies; it is the only target once their stars are taken.
    let waves = vec![wave(
        1,
        7,
        6,
        enemy(1, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    let mut s = armed(
        stage_full(18_000, Some(boss_at(1_000_000, NEAR_Y)), waves, vec![]),
        weapon(),
    );
    let star_stage = stage(7, tough(0, None, None));
    s.levels = star_stage.levels;
    s.stars = star_stage.stars;
    let mut e = with(
        &s,
        &content(vec![random_rune()], None, runes(30, 180, 3, 1)),
    );
    take_all(&mut e);
    // The button let go, the last step over and any bleed from the steps that took the stars
    // run out.
    let boss = EntityId(1);
    for _ in 0..10 {
        e.step(idle());
    }
    while !statuses(&e, boss).is_empty() {
        e.step(idle());
    }
    let start = e.tick();
    let seen = hits(&mut e, boss, 200, once(start));
    let hit = start + 1;
    assert_eq!(of(&seen, 10), [hit], "{seen:?}");
    let ticks: Vec<u32> = (1..=6).map(|k| hit + 30 * k).collect();
    assert_eq!(of(&seen, 3), ticks, "{seen:?}");
}

#[test]
fn the_state_hash_holds_bleed() {
    // The same run with bleed ticking every 30 or 45 ticks: the same until it is applied.
    let mut a = holding(runes(30, 180, 3, 5), tough(0, None, None));
    let mut b = holding(runes(45, 180, 3, 5), tough(0, None, None));
    wait_for_target(&mut a);
    wait_for_target(&mut b);
    assert_eq!(a.state_hash(), b.state_hash());
    let start = a.tick();
    a.step(once(start)(start));
    b.step(once(start)(start));
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn a_run_holding_bleed_records_and_verifies() {
    // The trial with a random rune and one stat reward, the hidden candidate always taken, and
    // the bundled bleed.
    let content = content(
        vec![
            random_rune(),
            json!({
                "id": "might", "rarity": "common", "effect": "attack_pct", "amount": 5,
                "limit": 20, "title": "Might", "description": ""
            }),
        ],
        None,
        Runes::bundled().clone(),
    );
    // A seed whose run draws bleed among the seven runes: the trial has stars for about four.
    let setup = RunSetup {
        seed: 20_261_013,
        ..trial_setup()
    };
    let mut recorder = RunRecorder::start_with(&setup, &content).unwrap();
    let mut bled = false;
    for t in 0..5_400 {
        while recorder.engine().offer_open() {
            let offer = recorder.engine().snapshot().progress.and_then(|p| p.offer);
            let rune = offer.and_then(|o| o.candidates.iter().position(|c| c.hidden));
            recorder
                .choose(u32::try_from(rune.unwrap_or(0)).unwrap())
                .unwrap();
        }
        let input: Input = run_input(recorder.engine(), t);
        if recorder.step(input).is_err() {
            break;
        }
        bled |= recorder.engine().events().iter().any(
            |ev| matches!(ev, DomainEvent::StatusApplied { status, .. } if status.0 == "bleed"),
        );
    }
    assert!(bled, "bleed was applied in the run");
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
}

#[test]
fn the_bleed_status_in_the_snapshot_counts_down_from_its_duration_and_leaves_at_zero() {
    // ADR-103 decision 3: the status is listed with what is left of it and the ticks it was
    // applied for, beside the damage-over-time view it has always had.
    let mut e = holding(runes(30, 180, 3, 5), tough(0, None, None));
    let target = wait_for_target(&mut e);
    // Poison and burn, held on the target too, are beside it in the list.
    let listed = |e: &Engine| -> Vec<StatusView> {
        e.snapshot()
            .entities
            .into_iter()
            .find(|en| en.id == target)
            .map(|en| en.statuses)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.id.0 == "bleed")
            .collect()
    };
    assert!(listed(&e).is_empty());
    let start = e.tick();
    e.step(once(start)(start));
    assert_eq!(
        listed(&e),
        [StatusView {
            id: "bleed".into(),
            remaining: 180,
            total: 180
        }]
    );
    e.step(Input::default());
    assert_eq!(
        listed(&e),
        [StatusView {
            id: "bleed".into(),
            remaining: 179,
            total: 180
        }]
    );
    for _ in 0..178 {
        e.step(Input::default());
    }
    assert_eq!(
        listed(&e),
        [StatusView {
            id: "bleed".into(),
            remaining: 1,
            total: 180
        }]
    );
    e.step(Input::default());
    assert!(
        listed(&e).is_empty(),
        "it runs out on the 180th tick after it was applied"
    );
}
