//! Flinch and poise, control repeats and vulnerable after a stun: the keys, their validators and
//! (below) the rules. Every number is worked out by hand from the placeholders of each test.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::pack::ContentFiles;
use stella_rain_core::rules::Rules;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::{self, Error};
use stella_rain_core::version::Versions;

use crate::common::on_hit::*;
use crate::common::*;

/// A smash step of one circle, with `phases` as its attack.
fn smashing(phases: Value) -> Value {
    json!({ "inline": {
        "combo": { "smash": { "attack": { "inline": phases } } },
        "window_ticks": 30,
        "assist_range": 0,
    } })
}

/// A hit phase of the main character's: one active tick with a circle in front.
fn hit_phase(extra: Value) -> Value {
    let mut p = json!({ "kind": "active", "ticks": 1, "damage": 10,
        "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

/// A `sim_version` 2 stage with `waves`, a `boss` and the main character's weapon `weapon`.
fn stage_of(waves: Vec<Value>, boss: Value, weapon: Value) -> Stage {
    let mut stage = armed(stage_full(900, Some(boss), waves, vec![]), weapon);
    stage.sim_version = 2;
    stage
}

fn invalid(stage: &Stage) -> Vec<(String, String)> {
    match validate::validate(stage) {
        Err(Error::Invalid(list)) => list.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("not refused as invalid: {other:?}"),
    }
}

fn poised(poise: u32) -> Value {
    let mut e = enemy(1000, hold(START_X, NEAR_Y), json!([]));
    e["poise"] = json!(poise);
    e
}

fn quiet_stage(waves: Vec<Value>, boss: Value) -> Stage {
    stage_of(waves, boss, quiet_weapon())
}

fn one(e: Value) -> Vec<Value> {
    vec![wave(1, 1, 0, e, START_X, NEAR_Y)]
}

#[test]
fn c6_a_poise_is_an_enemys_or_elites_within_its_bound() {
    for ok in [0, 1_000] {
        let stage = quiet_stage(one(poised(ok)), far_boss());
        validate::validate(&stage).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    let (path, _) = invalid(&quiet_stage(one(poised(1_001)), far_boss())).remove(0);
    assert_eq!(path, "$.waves[0].enemy.poise");
    let mut companion = companion(10, hold(START_X, START_Y), json!([]));
    companion["poise"] = json!(5);
    let mut stage = quiet_stage(vec![], far_boss());
    stage.companions = vec![serde_json::from_value(companion).unwrap()];
    assert_eq!(invalid(&stage)[0].0, "$.companions[0].poise");
    // The boss is not an agent: it has no `poise` key at all (a summon of its phases is a
    // regular enemy and may name one).
}

#[test]
fn c7_a_phases_poise_damage_is_a_hit_phase_of_the_main_characters() {
    let ok = smashing(json!([hit_phase(json!({ "poise_damage": 1_000 }))]));
    validate::validate(&stage_of(vec![], far_boss(), ok)).expect("valid");
    let paths = |weapon: Value| {
        invalid(&stage_of(vec![], far_boss(), weapon))
            .into_iter()
            .map(|(p, _)| p)
            .collect::<Vec<_>>()
    };
    let too_big = paths(smashing(json!([hit_phase(
        json!({ "poise_damage": 1_001 })
    )])));
    let wind_up = paths(smashing(json!([
        { "kind": "wind_up", "ticks": 3, "damage": 0, "poise_damage": 1 }
    ])));
    let no_box = paths(smashing(json!([
        { "kind": "active", "ticks": 1, "damage": 0, "poise_damage": 1 }
    ])));
    for found in [too_big, wind_up, no_box] {
        assert!(
            found.iter().any(|p| p.ends_with(".poise_damage")),
            "{found:?}"
        );
    }
    // An enemy's attack is refused.
    let rules = json!([{ "when": always(), "target": "player", "cooldown_ticks": 0,
        "do": { "type": "attack", "attack": { "inline": [
            hit_phase(json!({ "poise_damage": 1 }))] } } }]);
    let e = enemy(1000, hold(START_X, NEAR_Y), rules);
    let found = invalid(&quiet_stage(one(e), far_boss()));
    assert!(
        found.iter().any(|(p, _)| p.ends_with(".poise_damage")),
        "{found:?}"
    );
}

fn content_naming(patch: Value) -> RunContent {
    let mut v: Value = serde_json::from_slice(ContentFiles::bundled_file("rules")).unwrap();
    for (k, x) in patch.as_object().unwrap() {
        v[k] = x.clone();
    }
    let rules = Rules::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();
    design_content().with_rules(rules)
}

#[test]
fn c8_an_enemy_naming_a_poise_needs_rules_naming_a_flinch() {
    let stage = quiet_stage(one(poised(5)), far_boss());
    let refused = Engine::new_with(&stage, &content_naming(json!({})));
    let Err(Error::Invalid(list)) = refused else {
        panic!("the run started without a flinch in its rules");
    };
    assert_eq!(list[0].path, "$.waves[0].enemy.poise");
    Engine::new_with(&stage, &content_naming(json!({ "flinch_ticks": 20 })))
        .expect("loads with a flinch");
    // No poise, no flinch needed.
    let plain = quiet_stage(
        one(enemy(1000, hold(START_X, NEAR_Y), json!([]))),
        far_boss(),
    );
    Engine::new_with(&plain, &content_naming(json!({}))).expect("loads");
}

// --- The rules ----------------------------------------------------------------------------------
//
// A weapon of an A step (`main`) and a smash step (`smash`): one active tick of 10 damage and 4 ticks
// of recovery, with a combo window of 1 tick so every press starts a step at the root. The
// placeholders of the rules patch: A deals 5 poise damage, B 50, a flinch lasts 20 ticks and
// 40 ticks of immunity follow; a hard control keeps 100, 50 and 25 percent of its ticks, counts
// from 300 ticks back, refuses the fourth and then holds a 120-tick immunity; a stun puts
// 20 percent `vulnerable` on its target.

use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::StatusView;

/// The rules patch every rules test starts from.
fn all_rules() -> Value {
    json!({
        "poise_damage_a": 5,
        "poise_damage_b": 50,
        "flinch_ticks": 20,
        "flinch_immune_ticks": 40,
        "control_repeat_pcts": [100, 50, 25],
        "control_reset_ticks": 300,
        "control_immune_ticks": 120,
        "vulnerable_pcts": { "stun": 20 },
    })
}

/// Only the flinch keys of `all_rules`.
fn flinch_rules() -> Value {
    json!({
        "poise_damage_a": 5,
        "poise_damage_b": 50,
        "flinch_ticks": 20,
        "flinch_immune_ticks": 40,
    })
}

fn step_attack(extra: Value) -> Value {
    json!({ "attack": { "inline": [
        hit_phase(extra),
        { "kind": "recovery", "ticks": 4 }
    ] } })
}

/// The weapon: an A step and a smash step (with `extra` on each hit phase), and a stun of
/// `stun` ticks for the smash when given.
fn flinch_weapon(stun: Option<u32>, a: Value, b: Value) -> Value {
    let mut w = json!({ "inline": {
        "combo": { "main": step_attack(a), "smash": step_attack(b) },
        "window_ticks": 1,
        "assist_range": 0,
    } });
    if let Some(ticks) = stun {
        w["inline"]["smash"] = json!({ "type": "stun", "ticks": ticks });
    }
    w
}

/// An enemy of 100,000 hit points at the target's place that names `poise`.
fn target(poise: Option<u32>, elite: bool) -> Value {
    let mut e = enemy(100_000, hold(START_X, NEAR_Y), json!([]));
    if let Some(p) = poise {
        e["poise"] = json!(p);
    }
    if elite {
        e["elite"] = json!({ "range": 256 });
    }
    wave(TARGET_TICK, 1, 0, e, START_X, NEAR_Y)
}

fn run_stage(weapon: Value, waves: Vec<Value>) -> Stage {
    let mut stage = stage_of(waves, far_boss(), weapon);
    stage.player.hp = 99;
    stage
}

fn start(stage: &Stage, patch: Value) -> (Engine, EntityId) {
    let mut e = Engine::new_with(stage, &content_naming(patch)).expect("the run starts");
    wait_for_target(&mut e);
    // The enemy of 100,000 hit points (the boss has more, and is numbered first).
    let id = e
        .snapshot()
        .entities
        .iter()
        .find(|en| en.hp == 100_000)
        .map(|en| en.id)
        .expect("the target");
    (e, id)
}

fn statuses(e: &Engine, id: EntityId) -> Vec<StatusView> {
    e.snapshot()
        .entities
        .into_iter()
        .find(|en| en.id == id)
        .map(|en| en.statuses)
        .unwrap_or_default()
}

fn named(list: &[StatusView], id: &str) -> Vec<StatusView> {
    list.iter()
        .filter(|s| s.id.as_str() == id)
        .cloned()
        .collect()
}

/// What a run of ticks showed of one enemy.
#[derive(Default)]
struct Played {
    /// Each hit on it: the tick, its damage and the statuses it shows then.
    hits: Vec<(u32, u32, Vec<StatusView>)>,
    flinch_ended: Vec<u32>,
    stun_ended: Vec<u32>,
    fired: Vec<u32>,
    /// `StatusApplied` of `(tick, status id, source)`.
    applied: Vec<(u32, String, Option<EntityId>)>,
    at: Vec<(u32, stella_rain_core::fixed::Point)>,
}

/// Runs `ticks` ticks. A press is `(tick, smash)`: `main` on an A step, `smash` on a B step.
fn play(e: &mut Engine, id: EntityId, presses: &[(u32, bool)], ticks: u32) -> Played {
    let mut p = Played::default();
    for _ in 0..ticks {
        let t = e.tick();
        let press = presses.iter().find(|(at, _)| *at == t);
        e.step(&[
            Input {
                main: press.is_some_and(|(_, smash)| !*smash),
                smash: press.is_some_and(|(_, smash)| *smash),
                ..idle()
            },
            Default::default(),
            Default::default(),
        ]);
        let now = e.tick();
        let mut hit = None;
        for ev in e.events() {
            match ev {
                DomainEvent::Hit { target, damage, .. } if *target == id => hit = Some(*damage),
                DomainEvent::FlinchEnded { target } if *target == id => p.flinch_ended.push(now),
                DomainEvent::StunEnded { target } if *target == id => p.stun_ended.push(now),
                DomainEvent::RuleFired { agent, .. } if *agent == id => p.fired.push(now),
                DomainEvent::StatusApplied {
                    target,
                    status,
                    source,
                    ..
                } if *target == id => p.applied.push((now, status.to_string(), *source)),
                _ => {}
            }
        }
        if let Some(damage) = hit {
            p.hits.push((now, damage, statuses(e, id)));
        }
        if let Some(en) = e.snapshot().entities.into_iter().find(|en| en.id == id) {
            p.at.push((now, en.at));
        }
    }
    p
}

/// The A or B press of a fresh run at the first chance: the `flinch` statuses after the hit.
fn flinches(poise: Option<u32>, a: Value, b: Value, smash: bool) -> Vec<StatusView> {
    let stage = run_stage(flinch_weapon(None, a, b), vec![target(poise, false)]);
    let (mut e, id) = start(&stage, flinch_rules());
    let at = e.tick();
    let p = play(&mut e, id, &[(at, smash)], 4);
    let (_, _, shown) = p.hits.first().expect("the press hits the target");
    named(shown, "flinch")
}

#[test]
fn p1_a_hit_that_deals_the_poise_flinches_the_enemy() {
    let none = json!({});
    // An A press deals 5: it flinches `poise: 5`, not `poise: 6`.
    let f = flinches(Some(5), none.clone(), none.clone(), false);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!((f[0].total, f[0].remaining), (20, 20));
    assert!(flinches(Some(6), none.clone(), none.clone(), false).is_empty());
    // A phase of its own poise damage replaces the press's default.
    let six = json!({ "poise_damage": 6 });
    assert_eq!(flinches(Some(6), six.clone(), none.clone(), false).len(), 1);
    // A B press deals 50: it flinches `poise: 50`, not `poise: 51`.
    assert_eq!(
        flinches(Some(50), none.clone(), none.clone(), true).len(),
        1
    );
    assert!(flinches(Some(51), none.clone(), none.clone(), true).is_empty());
    // A poise of 0 flinches from a hit of 0 poise damage.
    let zero = json!({ "poise_damage": 0 });
    assert_eq!(flinches(Some(0), zero.clone(), zero, false).len(), 1);
}

#[test]
fn p2_nothing_flinches_without_a_poise_or_while_stunned_or_flinching_or_immune() {
    let none = json!({});
    // No poise on the enemy.
    assert!(flinches(None, none.clone(), none.clone(), false).is_empty());
    // A stun that lands on the same hit wins: one `stun`, no `flinch`.
    let stage = run_stage(
        flinch_weapon(Some(30), none.clone(), none.clone()),
        vec![target(Some(50), false)],
    );
    let (mut e, id) = start(&stage, flinch_rules());
    let at = e.tick();
    let p = play(&mut e, id, &[(at, true)], 4);
    let shown = &p.hits[0].2;
    assert_eq!(named(shown, "stun").len(), 1);
    assert!(named(shown, "flinch").is_empty(), "{shown:?}");
    // Already flinching: a second press 8 ticks on leaves the remainder alone.
    let stage = run_stage(
        flinch_weapon(None, none.clone(), none.clone()),
        vec![target(Some(5), false)],
    );
    let (mut e, id) = start(&stage, flinch_rules());
    let at = e.tick();
    let p = play(&mut e, id, &[(at, false), (at + 8, false)], 12);
    assert_eq!(p.hits.len(), 2);
    let second = named(&p.hits[1].2, "flinch");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].remaining, 12, "the second hit did not restart it");
    // No `flinch_ticks` in the rules and no poise on the enemy: nothing flinches.
    let stage = run_stage(
        flinch_weapon(None, none.clone(), none.clone()),
        vec![target(None, false)],
    );
    let (mut e, id) = start(&stage, json!({}));
    let at = e.tick();
    let p = play(&mut e, id, &[(at, false)], 4);
    assert!(named(&p.hits[0].2, "flinch").is_empty());
}

#[test]
fn p3_a_flinching_enemy_holds_still_and_holds_its_fire_and_loses_its_attack() {
    let follow = json!({ "type": "follow", "distance": 0 });
    let weapon = || flinch_weapon(None, json!({}), json!({}));
    let mut poised = enemy(100_000, follow.clone(), json!([]));
    poised["poise"] = json!(5);
    let stage = run_stage(
        weapon(),
        vec![wave(TARGET_TICK, 1, 0, poised, START_X, NEAR_Y)],
    );
    let (mut e, id) = start(&stage, flinch_rules());
    let at = e.tick();
    let p = play(&mut e, id, &[(at, false)], 40);
    let landed = p.hits[0].0;
    let held: Vec<_> =
        p.at.iter()
            .filter(|(t, _)| *t >= landed && *t < landed + 20)
            .map(|(_, a)| *a)
            .collect();
    assert_eq!(held.len(), 20);
    assert!(held.iter().all(|a| *a == held[0]), "it does not move");
    let after = p.at.iter().find(|(t, _)| *t >= landed + 20).unwrap().1;
    assert_ne!(after, held[0], "it moves on the 20th tick");
    // Its rule that is ready 10 ticks after it appears fires at `landed + 20`.
    let rules = json!([attack_rule(
        json!({ "type": "time_above", "ticks": 10 }),
        1000
    )]);
    let mut shooter = enemy(100_000, hold(START_X, NEAR_Y), rules);
    shooter["poise"] = json!(5);
    let stage = run_stage(
        weapon(),
        vec![wave(TARGET_TICK, 1, 0, shooter, START_X, NEAR_Y)],
    );
    let (mut e, id) = start(&stage, flinch_rules());
    let at = e.tick();
    let p = play(&mut e, id, &[(at, false)], 60);
    let landed = p.hits[0].0;
    assert_eq!(p.fired.first(), Some(&(landed + 20)), "{:?}", p.fired);
    // A running attack is cancelled on the hit tick.
    let rules = json!([attack_rule(always(), 1000)]);
    let mut attacker = enemy(100_000, hold(START_X, NEAR_Y), rules);
    attacker["poise"] = json!(5);
    let stage = run_stage(
        weapon(),
        vec![wave(TARGET_TICK, 1, 0, attacker, START_X, NEAR_Y)],
    );
    let (mut e, id) = start(&stage, flinch_rules());
    let attacking = |e: &Engine| {
        e.snapshot()
            .entities
            .iter()
            .find(|en| en.id == id)
            .unwrap()
            .attack
            .is_some()
    };
    assert!(attacking(&e), "it attacks");
    let at = e.tick();
    play(&mut e, id, &[(at, false)], 3);
    assert!(!attacking(&e), "the attack is cancelled");
}

#[test]
fn p4_a_flinch_ends_on_its_tick_and_its_immunity_holds_a_second_one_off() {
    let stage = run_stage(
        flinch_weapon(Some(30), json!({}), json!({})),
        vec![target(Some(5), false)],
    );
    let (mut e, id) = start(&stage, flinch_rules());
    let at = e.tick();
    // Presses 25 ticks after the first (inside the 40 of immunity: the flinch ended at +20, the
    // immunity runs to +60) and 65 after it flinch again, the last only on the 65th.
    let p = play(
        &mut e,
        id,
        &[(at, false), (at + 25, false), (at + 65, false)],
        72,
    );
    let landed = p.hits[0].0;
    assert_eq!(p.flinch_ended.first(), Some(&(landed + 20)));
    assert!(p.stun_ended.is_empty());
    assert_eq!(named(&p.hits[0].2, "flinch").len(), 1);
    assert!(
        named(&p.hits[1].2, "flinch").is_empty(),
        "inside the immunity"
    );
    assert_eq!(named(&p.hits[2].2, "flinch").len(), 1, "after it");
    // A stun that lands on a flinching enemy ends the flinch and wins.
    let (mut e, id) = start(&stage, flinch_rules());
    let at = e.tick();
    let p = play(&mut e, id, &[(at, false), (at + 10, true)], 16);
    let stunned_at = p.hits[1].0;
    let shown = &p.hits[1].2;
    assert_eq!(named(shown, "stun").len(), 1);
    assert!(named(shown, "flinch").is_empty(), "{shown:?}");
    assert_eq!(p.flinch_ended, vec![stunned_at]);
}

/// A run of stuns of 30 ticks on a plain (poise-free) target at `presses` (all smashes).
fn stuns(waves: Vec<Value>, patch: Value, presses: &[u32], ticks: u32) -> Played {
    let stage = run_stage(flinch_weapon(Some(30), json!({}), json!({})), waves);
    let (mut e, id) = start(&stage, patch);
    let at = e.tick();
    let presses: Vec<(u32, bool)> = presses.iter().map(|p| (at + p, true)).collect();
    play(&mut e, id, &presses, presses.last().unwrap().0 - at + 6)
}

fn stun_total(p: &Played, hit: usize) -> Option<u32> {
    named(&p.hits[hit].2, "stun").first().map(|s| s.total)
}

#[test]
fn p5_each_further_hard_control_is_shorter_and_the_fourth_is_refused() {
    let plain = || vec![target(None, false)];
    // Presses 40 apart: 30, 15, 7 ticks, the fourth refused, and a fifth inside the immunity.
    let p = stuns(plain(), all_rules(), &[0, 40, 80, 120, 160], 0);
    let totals: Vec<_> = (0..5).map(|n| stun_total(&p, n)).collect();
    assert_eq!(
        totals,
        [Some(30), Some(15), Some(7), None, None],
        "{totals:?}"
    );
    // 250 ticks after the fourth the immunity is over and the count starts again: 30.
    let p = stuns(plain(), all_rules(), &[0, 40, 80, 120, 370], 0);
    assert_eq!(stun_total(&p, 4), Some(30));
    // Two stuns 400 apart: the reset (300) elapsed, both are 30.
    let p = stuns(plain(), all_rules(), &[0, 400], 0);
    assert_eq!((stun_total(&p, 0), stun_total(&p, 1)), (Some(30), Some(30)));
    // With the list named the run stats' 60-tick immunity no longer applies: a second smash 40
    // ticks after the first stun ended (it ended 30 on, the press is at 70) lands for 15.
    let p = stuns(plain(), all_rules(), &[0, 70], 0);
    assert_eq!(stun_total(&p, 1), Some(15));
    // An elite (50 percent): 30 becomes 15, then 7 (50 percent of 15, floored).
    let p = stuns(vec![target(None, true)], all_rules(), &[0, 40, 80], 0);
    assert_eq!(
        (stun_total(&p, 0), stun_total(&p, 1), stun_total(&p, 2)),
        (Some(15), Some(7), Some(3)),
        "the elite's share comes first, then the repeat"
    );
}

#[test]
fn p6_a_stun_that_lands_puts_vulnerable_on_the_target_for_its_ticks() {
    let plain = || vec![target(None, false)];
    let p = stuns(plain(), all_rules(), &[0, 40, 80, 120], 0);
    let vulnerable = |n: usize| named(&p.hits[n].2, "vulnerable").first().map(|s| s.total);
    // The stun lands inside the hit, after its damage: the status shows from the same tick.
    assert_eq!(
        [vulnerable(0), vulnerable(1), vulnerable(2), vulnerable(3)],
        [Some(30), Some(30), Some(30), None],
        "stuns of 30, 15 and 7 refresh one status; the refused fourth adds none"
    );
    // The source is the hitting player.
    let by: Vec<_> = p
        .applied
        .iter()
        .filter(|(_, status, _)| status == "vulnerable")
        .map(|(_, _, source)| *source)
        .collect();
    assert!(!by.is_empty() && by.iter().all(|s| s.is_some()), "{by:?}");
    // The next hit deals the bonus: 10 raised by 20 percent is 12.
    let damages: Vec<u32> = p.hits.iter().map(|(_, d, _)| *d).collect();
    assert_eq!(damages[0], 10, "the stunning hit is dealt before it");
    assert_eq!(damages[1], 12, "{damages:?}");
    // A flinch puts none, and neither does a run whose rules name no `vulnerable_pcts`.
    let flinched = flinches(Some(5), json!({}), json!({}), false);
    assert_eq!(flinched.len(), 1);
    let mut patch = all_rules();
    patch.as_object_mut().unwrap().remove("vulnerable_pcts");
    let p = stuns(plain(), patch, &[0, 40], 0);
    assert!(named(&p.hits[0].2, "vulnerable").is_empty());
}

#[test]
fn p8_the_keys_move_no_hash_while_no_enemy_names_a_poise() {
    // The `stunning(30)` run, with a list only the flinch names: no list, no vulnerable.
    let weapon = || flinch_weapon(Some(30), json!({}), json!({}));
    let presses = [(TARGET_TICK + 3, true), (TARGET_TICK + 90, true)];
    let hashes = |patch: Value| {
        let stage = run_stage(weapon(), vec![target(None, false)]);
        let mut e = Engine::new_with(&stage, &content_naming(patch)).unwrap();
        let mut out = Vec::new();
        for _ in 0..600 {
            let t = e.tick();
            let press = presses.iter().any(|(at, _)| *at == t);
            e.step(&[
                Input {
                    smash: press,
                    ..idle()
                },
                Default::default(),
                Default::default(),
            ]);
            out.push(e.state_hash());
        }
        out
    };
    let plain = hashes(json!({}));
    let named = hashes(json!({ "flinch_ticks": 20, "flinch_immune_ticks": 40,
        "poise_damage_a": 5, "poise_damage_b": 50 }));
    assert_eq!(plain, named);
}
