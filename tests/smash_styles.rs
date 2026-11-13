//! A style's `smash_effect` kind (ADR-SIM-9 points 4 to 7): the effect it names replaces the
//! weapon's smash effect once the style, or the upgrade that names it, is taken, read at the press
//! from the style and path already recorded (nothing is added to the state). `reap` adds a bleed
//! stack as bleed does and, on a hit at its stack count, cashes the stacks. The runs are equipment
//! runs on the Sword preset (whose own smash is bleed) with test expertise; the numbers are the
//! tests' own.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::expertise::Expertise;
use stella_rain_core::id::{EntityId, PlayerIdx};
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::rune::Rune;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::version::Versions;

use crate::common;
use crate::common::equip::{Parts, equipped_in};

use common::on_hit::TARGET_TICK;
use common::*;

/// The bundled runes' bleed: 300 a stack every tick it is due.
const BLEED_DAMAGE: u32 = 300;
/// The target stands this far ahead, inside the Sword's smash and outside the characters' touch.
const TARGET_Y: i32 = START_Y - 12_000;

fn pull() -> Value {
    json!({ "type": "pull", "distance": 4_096, "ticks": 10 })
}

fn knockback() -> Value {
    json!({ "type": "knockback", "distance": 4_096, "ticks": 10 })
}

fn reap(stacks: u32, boss_pct: u32) -> Value {
    json!({ "type": "reap", "stacks": stacks, "boss_pct": boss_pct })
}

fn sets(effect: Value) -> Value {
    json!({ "type": "smash_effect", "effect": effect })
}

fn level(id: &str, effect: Value) -> Value {
    json!({ "id": id, "effect": "none", "kinds": [sets(effect)] })
}

/// Style 0 sets `s0`, style 1 sets `s1` (when named), style 2 names no effect and its first
/// upgrade sets `up`.
fn expertise_json(s0: Value, s1: Option<Value>, up: Option<Value>) -> Value {
    let mut file: Value = serde_json::from_slice(crate::before::EXPERTISE).unwrap();
    file["thresholds"] = json!([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    for w in file["weapons"].as_array_mut().unwrap() {
        if w["weapon"] != "giant_sword" {
            continue;
        }
        w["base"] = json!([]);
        for s in w["styles"].as_array_mut().unwrap() {
            s["stats"] = json!([]);
        }
        w["styles"][0]["kinds"] = json!([sets(s0.clone())]);
        if let Some(s1) = &s1 {
            w["styles"][1]["kinds"] = json!([sets(s1.clone())]);
        }
        if let Some(up) = &up {
            w["styles"][2]["upgrade_1"] = json!([level("swap_effect", up.clone())]);
        }
    }
    file
}

fn expertise(s0: Value, s1: Option<Value>, up: Option<Value>) -> Expertise {
    let file = expertise_json(s0, s1, up);
    Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the test expertise")
}

fn build(expertise: Expertise, target_hp: u32, boss: bool) -> (Engine, EntityId) {
    let (boss_json, waves) = if boss {
        // The stage must give expertise points: a harmless enemy far off does.
        let e = enemy(10, hold(START_X, START_Y - 90_000), json!([]));
        let mut w = wave(TARGET_TICK, 1, 0, e, START_X, START_Y - 90_000);
        w["expertise"] = json!({ "hit": 0, "kill": 0 });
        (boss_at(target_hp, TARGET_Y), vec![w])
    } else {
        let e = enemy(target_hp, hold(START_X, TARGET_Y), json!([]));
        let mut w = wave(TARGET_TICK, 1, 0, e, START_X, TARGET_Y);
        w["expertise"] = json!({ "hit": 0, "kill": 0 });
        (far_boss(), vec![w])
    };
    let mut stage = armed(
        stage_full(18_000, Some(boss_json), waves, vec![]),
        json!({ "preset": { "id": "giant_sword" } }),
    );
    stage.player.hp = 99;
    stage.sim_version = 2;
    let parts = Parts {
        expertise: Some(expertise),
        expertise_points: 550,
        ..Parts::default()
    };
    let file = json!({ "rarities": common::rarities(), "rewards": [
        { "id": "reach", "rarity": "common", "effect": "range_pct", "amount": 30, "limit": 2 }
    ] });
    let mut e = equipped_in(&stage, file, parts, &[], 1);
    for _ in 0..TARGET_TICK + 2 {
        step(&mut e, idle());
    }
    let kind = if boss {
        EntityKind::Boss
    } else {
        EntityKind::Enemy
    };
    let id = e
        .snapshot()
        .entities
        .iter()
        .find(|en| en.kind == kind)
        .map(|en| en.id)
        .expect("the target");
    (e, id)
}

fn step(e: &mut Engine, input: Input) {
    e.step(&[input, Default::default(), Default::default()]);
}

fn pick(e: &mut Engine, option: u32) {
    e.act(PlayerIdx::SOLO, EquipAction::TreePick { option })
        .expect("a pick");
}

fn entity(e: &Engine, id: EntityId) -> Option<stella_rain_core::snapshot::EntityView> {
    e.snapshot().entities.into_iter().find(|en| en.id == id)
}

fn bleed_stacks(e: &Engine, id: EntityId) -> u16 {
    entity(e, id)
        .and_then(|en| {
            en.damage_over_time
                .into_iter()
                .find(|d| d.status.as_str() == "bleed")
        })
        .map_or(0, |d| d.stacks)
}

/// How far the target is from the main character, in 1/256 pixels (squared away: monotone).
fn gap(e: &Engine, id: EntityId) -> i64 {
    let s = e.snapshot();
    let (p, t) = (s.players[0].at, entity(e, id).expect("the target").at);
    let (dx, dy) = (i64::from(p.x.0 - t.x.0), i64::from(p.y.0 - t.y.0));
    dx * dx + dy * dy
}

/// The smallest and the largest gap to `id` over one smash press and the 80 ticks after it (the
/// target walks back to its post, so the end gap says nothing).
fn smash_gaps(e: &mut Engine, id: EntityId) -> (i64, i64) {
    let (mut lo, mut hi) = (gap(e, id), gap(e, id));
    step(
        e,
        Input {
            smash: true,
            ..idle()
        },
    );
    for _ in 0..80 {
        let g = gap(e, id);
        (lo, hi) = (lo.min(g), hi.max(g));
        step(e, idle());
    }
    (lo, hi)
}

/// One smash press from neutral, then the combo runs out; the events of every tick.
fn smash(e: &mut Engine) -> Vec<DomainEvent> {
    let mut events = Vec::new();
    step(
        e,
        Input {
            smash: true,
            ..idle()
        },
    );
    events.extend(e.events().iter().cloned());
    for _ in 0..80 {
        step(e, idle());
        events.extend(e.events().iter().cloned());
    }
    events
}

fn hit_on(events: &[DomainEvent], id: EntityId, rune: Option<Rune>) -> Vec<u32> {
    events
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::Hit {
                target,
                damage,
                rune: r,
                ..
            } if *target == id && *r == rune => Some(*damage),
            _ => None,
        })
        .collect()
}

#[test]
fn the_base_effect_applies_before_any_pick() {
    let (mut e, t) = build(expertise(pull(), Some(knockback()), None), 1_000_000, false);
    smash(&mut e);
    assert_eq!(bleed_stacks(&e, t), 1, "the Sword's own smash is bleed");
}

#[test]
fn a_style_sets_the_effect_of_the_next_smash() {
    // Style 0: pull, no bleed stack.
    let (mut e, t) = build(expertise(pull(), Some(knockback()), None), 1_000_000, false);
    pick(&mut e, 0);
    let before = gap(&e, t);
    let (lo, _) = smash_gaps(&mut e, t);
    assert!(lo < before, "pulled");
    assert_eq!(bleed_stacks(&e, t), 0);
    // Style 1: knockback, no bleed stack.
    let (mut e, t) = build(expertise(pull(), Some(knockback()), None), 1_000_000, false);
    pick(&mut e, 1);
    let before = gap(&e, t);
    let (_, hi) = smash_gaps(&mut e, t);
    assert!(hi > before, "pushed away");
    assert_eq!(bleed_stacks(&e, t), 0);
    // Style 2 names no effect: the base's bleed stays.
    let (mut e, t) = build(expertise(pull(), Some(knockback()), None), 1_000_000, false);
    pick(&mut e, 2);
    smash(&mut e);
    assert_eq!(bleed_stacks(&e, t), 1);
}

#[test]
fn an_upgrade_sets_its_effect_only_once_it_is_taken() {
    let x = expertise(pull(), None, Some(knockback()));
    let (mut e, t) = build(x, 1_000_000, false);
    pick(&mut e, 2);
    smash(&mut e);
    assert_eq!(bleed_stacks(&e, t), 1, "before the upgrade: the base's");
    pick(&mut e, 0);
    let before = gap(&e, t);
    let (_, hi) = smash_gaps(&mut e, t);
    assert!(hi > before, "after it: the upgrade's knockback");
    assert_eq!(bleed_stacks(&e, t), 1, "no stack added by the knockback");
}

/// The ticks a bleed status still has due: as the rule counts them.
fn due(remaining: u32, next_in: u32, interval: u32) -> u32 {
    if remaining < next_in {
        0
    } else {
        (remaining - next_in) / interval + 1
    }
}

/// One smash press and the 80 ticks after it, tick by tick: the events of each tick and the
/// bleed status of `id` (ticks left, ticks to the next tick, stacks) as it stood before it.
type Ticks = Vec<(Vec<DomainEvent>, Option<(u32, u32, u16)>, u32)>;

fn smash_ticks(e: &mut Engine, id: EntityId) -> Ticks {
    let mut out = Vec::new();
    for t in 0..81 {
        let hp = entity(e, id).map_or(0, |en| en.hp);
        let before = entity(e, id).and_then(|en| {
            en.damage_over_time
                .into_iter()
                .find(|d| d.status.as_str() == "bleed")
                .map(|d| (d.ticks_left, d.next_tick_in, d.stacks))
        });
        step(
            e,
            Input {
                smash: t == 0,
                ..idle()
            },
        );
        out.push((e.events().to_vec(), before, hp));
    }
    out
}

/// The bleed damage dealt on the tick the smash's own hit landed, and the status before it.
fn burst(ticks: &Ticks, id: EntityId) -> (u32, Option<(u32, u32, u16)>) {
    let (events, before, _) = ticks
        .iter()
        .find(|(ev, ..)| !hit_on(ev, id, None).is_empty())
        .expect("the smash hit");
    (hit_on(events, id, Some(Rune::Bleed)).iter().sum(), *before)
}

#[test]
fn reap_adds_stacks_and_cashes_at_its_count() {
    let (mut e, t) = build(expertise(reap(3, 50), None, None), 1_000_000, false);
    pick(&mut e, 0);
    for n in 1..=3u16 {
        smash(&mut e);
        assert_eq!(bleed_stacks(&e, t), n);
    }
    // The fourth smash finds 3 stacks and cashes the bleed ticks still due.
    let ticks = smash_ticks(&mut e, t);
    let (dealt, before) = burst(&ticks, t);
    let (left, next_in, stacks) = before.expect("the bleed");
    assert_eq!(stacks, 3);
    // The tick may run before or after the hit within the tick: one tick's difference.
    let by = |left, next_in| due(left, next_in, 20) * 3 * BLEED_DAMAGE;
    assert!(
        dealt == by(left, next_in) || (next_in > 1 && dealt == by(left - 1, next_in - 1)),
        "{dealt} for {left} left, next in {next_in}"
    );
    assert!(dealt > 0);
    assert_eq!(bleed_stacks(&e, t), 0, "the status is consumed");
}

#[test]
fn a_boss_takes_its_share_of_the_burst() {
    let run = |boss: bool| {
        let (mut e, t) = build(expertise(reap(2, 50), None, None), 1_000_000, boss);
        pick(&mut e, 0);
        smash(&mut e);
        smash(&mut e);
        assert_eq!(bleed_stacks(&e, t), 2);
        let ticks = smash_ticks(&mut e, t);
        assert_eq!(bleed_stacks(&e, t), 0);
        burst(&ticks, t).0
    };
    let regular = run(false);
    let boss = run(true);
    assert!(regular > 0);
    assert_eq!(boss, regular * 50 / 100, "{boss} of {regular}");
}

#[test]
fn a_smash_that_kills_neither_stacks_nor_cashes() {
    let (mut e, t) = build(expertise(reap(1, 50), None, None), 5, false);
    pick(&mut e, 0);
    let events = smash(&mut e);
    assert!(entity(&e, t).is_none(), "the enemy died");
    assert!(hit_on(&events, t, Some(Rune::Bleed)).is_empty());
}

/// Dormant: with the effects named but never taken, every tick's hash is the one of the same
/// expertise with no kind at all.
#[test]
fn an_untaken_smash_effect_changes_no_hash() {
    let run = |x: Expertise| {
        let (mut e, _) = build(x, 1_000_000, false);
        let mut hashes = Vec::new();
        for t in 0..300u32 {
            step(
                &mut e,
                Input {
                    smash: t % 90 == 0,
                    ..idle()
                },
            );
            hashes.push(e.state_hash());
        }
        hashes
    };
    let mut bare: Value = expertise_json(pull(), None, None);
    for w in bare["weapons"].as_array_mut().unwrap() {
        for s in w["styles"].as_array_mut().unwrap() {
            s.as_object_mut().unwrap().remove("kinds");
        }
    }
    let bare = Expertise::parse(bare.to_string().as_bytes(), Versions::CURRENT).unwrap();
    assert_eq!(
        run(expertise(reap(3, 50), Some(knockback()), Some(pull()))),
        run(bare)
    );
}

/// A weapon naming `reap` in its `smash` is `sim_version` 2's, and bounded by its caps.
#[test]
fn a_weapon_naming_reap_needs_sim_version_2() {
    let weapon = |effect: Value| {
        json!({ "inline": {
            "combo": { "smash": { "attack": { "preset": { "id": "slash" } } } },
            "window_ticks": 30, "assist_range": 0, "smash": effect,
        } })
    };
    let check = |version: u32, effect: Value| {
        let mut stage = armed(
            stage_full(600, Some(far_boss()), vec![], vec![]),
            weapon(effect),
        );
        stage.sim_version = version;
        stella_rain_core::validate::validate(&stage)
    };
    assert_eq!(
        stella_rain_core::validate::limits::MAX_REAP_STACKS,
        stella_rain_core::runes::MAX_DOT_STACKS
    );
    assert!(check(1, reap(3, 50)).is_err());
    assert!(check(2, reap(3, 50)).is_ok());
    assert!(check(2, reap(0, 50)).is_err());
    assert!(check(2, reap(11, 50)).is_err());
    assert!(check(2, reap(3, 101)).is_err());
}

/// The views name the effect compactly, no numbers; the bundled expertise names none.
#[test]
fn the_views_name_the_smash_effect() {
    let (e, _) = build(
        expertise(reap(3, 50), Some(knockback()), None),
        1_000_000,
        false,
    );
    let snap = e.snapshot();
    let ready =
        serde_json::to_value(snap.players[0].tree_ready.as_ref().expect("a point")).unwrap();
    let kind = |name: &str| json!([{ "type": "smash_effect", "effect": name }]);
    assert_eq!(ready["kinds"], json!([kind("reap"), kind("knockback"), []]));
    let stats = serde_json::to_value(snap.players[0].weapon_stats.as_ref().unwrap()).unwrap();
    assert_eq!(stats["trees"][0]["grants"][0]["kinds"], kind("reap"));
    let bundled = Expertise::bundled().clone();
    let (e, _) = build(bundled, 1_000_000, false);
    let snap = e.snapshot();
    let ready = serde_json::to_value(snap.players[0].tree_ready.as_ref().unwrap()).unwrap();
    assert!(ready.get("kinds").is_none());
}

/// A burst that kills the target on the cashing hit: the target is gone, nothing more is put on
/// it, and the run goes on. The hit points are worked out from a run with plenty: the cashing
/// hit alone leaves exactly 1, so the burst is what kills.
#[test]
fn a_burst_that_kills_leaves_nothing_on_the_corpse() {
    const BIG: u32 = 1_000_000;
    let (mut e, t) = build(expertise(reap(2, 50), None, None), BIG, false);
    pick(&mut e, 0);
    smash(&mut e);
    smash(&mut e);
    let ticks = smash_ticks(&mut e, t);
    let (burst, _) = burst(&ticks, t);
    let (hit, hp_then) = ticks
        .iter()
        .find_map(|(ev, _, hp)| hit_on(ev, t, None).first().map(|h| (*h, *hp)))
        .unwrap();
    assert!(burst > 0);
    // Bleed ticks in the wind-up lower it a little more before the hit lands.
    let lost_by_hit = BIG - hp_then;
    let (mut e, t) = build(
        expertise(reap(2, 50), None, None),
        lost_by_hit + hit + 1,
        false,
    );
    pick(&mut e, 0);
    smash(&mut e);
    smash(&mut e);
    let ticks = smash_ticks(&mut e, t);
    assert!(entity(&e, t).is_none(), "the burst killed it");
    assert_eq!(burst_of(&ticks, t), burst);
}

fn burst_of(ticks: &Ticks, id: EntityId) -> u32 {
    burst(ticks, id).0
}
