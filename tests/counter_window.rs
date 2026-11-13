//! The counter: a smash step whose hit box hits an attacker during a `counterable` wind-up cancels
//! the attack, stuns a regular enemy or an elite, staggers an elite or the boss, and reports
//! `Countered` once. Dormant until content flags a phase. Every number is worked out by hand from the
//! placeholders of each test (a 30-tick wind-up, a stun of 40, 60 of gauge, a share of 50 percent).

use std::sync::Arc;

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::{EntityId, PlayerIdx};
use stella_rain_core::input::Input;
use stella_rain_core::rules::Rules;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::snapshot::{EntityKind, EntityView};
use stella_rain_core::stage::Stage;
use stella_rain_core::version::Versions;

use crate::common::on_hit::*;
use crate::common::*;

const HP: u32 = 100_000;

/// A hit phase of the main character's: one active tick with a circle in front.
fn hit_phase(extra: Value) -> Value {
    let mut p = json!({ "kind": "active", "ticks": 1, "damage": 10,
        "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

/// A step: one active tick of 10 damage, then 4 ticks of recovery.
fn step_attack() -> Value {
    json!({ "attack": { "inline": [hit_phase(json!({})), { "kind": "recovery", "ticks": 4 }] } })
}

/// The weapon: an A step and a smash step.
fn weapon() -> Value {
    json!({ "inline": {
        "combo": { "main": step_attack(), "smash": step_attack() },
        "window_ticks": 1,
        "assist_range": 0,
    } })
}

/// The harness rules: a counter stuns 40 ticks and fills 60 (an elite's stun share 50 percent);
/// a stun's resisted fill is 30; a stagger lasts 20 ticks.
fn rules_counter() -> Value {
    json!({
        "counter_stun_ticks": 40,
        "stagger_values": { "stun": 30, "counter": 60 },
        "staggered_ticks": 20,
    })
}

fn content_with(patch: Value) -> RunContent {
    let mut v: Value = dormant_rules_value();
    for (k, x) in patch.as_object().unwrap() {
        v[k] = x.clone();
    }
    let rules = Rules::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();
    design_content().with_rules(rules)
}

/// An attack of a 30-tick wind-up (counterable when `flag`), one active tick of 1 damage on the
/// player, then 4 ticks of recovery.
fn attack(flag: bool, wind_up: u32) -> Value {
    json!({ "inline": [
        { "kind": "wind_up", "ticks": wind_up, "counterable": flag },
        { "kind": "active", "ticks": 1, "damage": 1,
          "hit_box": { "type": "circle", "radius": 8_192 } },
        { "kind": "recovery", "ticks": 4 },
    ] })
}

fn rule_of(attack: Value) -> Value {
    json!([{ "when": always(), "target": "player", "cooldown_ticks": 10,
             "do": { "type": "attack", "attack": attack } }])
}

fn plain(hp: u32, rules: Value) -> Value {
    enemy(hp, hold(START_X, NEAR_Y), rules)
}

fn wave_of(e: Value) -> Vec<Value> {
    vec![wave(TARGET_TICK, 1, 0, e, START_X, NEAR_Y)]
}

fn elite_of(rules: Value, stagger_max: u32) -> Vec<Value> {
    let mut e = plain(HP, rules);
    e["elite"] = json!({ "range": 100_000 });
    e["stagger_max"] = json!(stagger_max);
    wave_of(e)
}

/// A boss whose body the swing reaches, naming a gauge of 100, whose timeline waits, then makes
/// `attack`.
fn boss_with(attack: Value, parts: bool) -> Value {
    let mut b = json!({ "base": "golem", "hp": 900_000, "radius": 8_192,
        "spawn": { "x": START_X, "y": NEAR_Y }, "stagger_max": 100,
        "phases": [{ "timeline": [{ "type": "wait", "ticks": 290 },
                                  { "type": "attack", "attack": attack }] }] });
    if parts {
        b["spawn"]["y"] = json!(START_Y - 40_000);
        b["parts"] = json!([{ "id": "horn", "asset": "horns_2", "hp": 1_000,
            "offset": { "x": 0, "y": 30_000 }, "radius": 2_048 }]);
    }
    b
}

fn stage_of(waves: Vec<Value>, boss: Value, weapon: Value) -> Stage {
    let mut stage = armed(stage_full(900, Some(boss), waves, vec![]), weapon);
    stage.sim_version = 2;
    stage.player.hp = 99;
    stage
}

#[derive(Clone, Copy)]
enum Target {
    Boss,
    Enemy(u32),
}

/// What a run showed of one target, and the engine.
struct Rig {
    e: Engine,
    id: EntityId,
    /// `(tick, attacker, player, attack)` of every `Countered`.
    countered: Vec<(u32, EntityId, PlayerIdx, Option<Arc<str>>)>,
    /// The ticks a `PlayerHit` was reported.
    player_hits: Vec<u32>,
    /// `(tick, status, duration, source)` of every `StatusApplied` on the target.
    applied: Vec<(u32, String, u32, Option<EntityId>)>,
    /// `(tick, damage)` of every hit on the target.
    hits: Vec<(u32, u32)>,
    fired: Vec<u32>,
}

impl Rig {
    fn new(weapon: Value, waves: Vec<Value>, boss: Value, patch: Value, target: Target) -> Rig {
        let stage = stage_of(waves, boss, weapon);
        let mut e = Engine::new_with(&stage, &content_with(patch)).expect("the run starts");
        while e.tick() < TARGET_TICK {
            e.step(&[idle(), Default::default(), Default::default()]);
        }
        let id = e
            .snapshot()
            .entities
            .iter()
            .find(|en| match target {
                Target::Boss => en.kind == EntityKind::Boss,
                Target::Enemy(hp) => en.kind == EntityKind::Enemy && en.hp == hp,
            })
            .map(|en| en.id)
            .expect("the target");
        Rig {
            e,
            id,
            countered: vec![],
            player_hits: vec![],
            applied: vec![],
            hits: vec![],
            fired: vec![],
        }
    }

    fn enemy(weapon: Value, attack: Value, patch: Value) -> Rig {
        Rig::new(
            weapon,
            wave_of(plain(HP, rule_of(attack))),
            far_boss(),
            patch,
            Target::Enemy(HP),
        )
    }

    fn step(&mut self, input: Input) {
        self.e
            .step(&[input, Default::default(), Default::default()]);
        let now = self.e.tick();
        for ev in self.e.events() {
            match ev {
                DomainEvent::Countered {
                    attacker,
                    player,
                    attack,
                } => self
                    .countered
                    .push((now, *attacker, *player, attack.clone())),
                DomainEvent::PlayerHit { .. } => self.player_hits.push(now),
                DomainEvent::Hit { target, damage, .. } if *target == self.id => {
                    self.hits.push((now, *damage));
                }
                DomainEvent::RuleFired { agent, .. } if *agent == self.id => self.fired.push(now),
                DomainEvent::StatusApplied {
                    target,
                    status,
                    duration_ticks,
                    source,
                    ..
                } if *target == self.id => {
                    self.applied
                        .push((now, status.to_string(), *duration_ticks, *source));
                }
                _ => {}
            }
        }
    }

    fn idle(&mut self, n: u32) {
        for _ in 0..n {
            self.step(idle());
        }
    }

    /// Idles until the target's attack is in its wind-up, so the next press lands inside it.
    fn until_wind_up(&mut self) {
        for _ in 0..400 {
            if self
                .view()
                .and_then(|v| v.attack)
                .is_some_and(|a| a.kind == stella_rain_core::attack::PhaseKind::WindUp)
            {
                return;
            }
            self.idle(1);
        }
        panic!("no wind-up");
    }

    /// The press alone: the tick the hit lands.
    fn hit(&mut self, smash: bool) {
        self.step(Input {
            main: !smash,
            smash,
            ..idle()
        });
    }

    fn view(&self) -> Option<EntityView> {
        self.e
            .snapshot()
            .entities
            .into_iter()
            .find(|en| en.id == self.id)
    }

    fn attack_running(&self) -> bool {
        self.view().is_some_and(|v| v.attack.is_some())
    }

    fn gauge(&self) -> Option<u32> {
        self.view().and_then(|v| v.stagger)
    }

    fn named(&self, status: &str) -> Vec<&(u32, String, u32, Option<EntityId>)> {
        self.applied.iter().filter(|a| a.1 == status).collect()
    }
}

// --- C1 ---------------------------------------------------------------------------------------

#[test]
fn c1_a_smash_in_a_counterable_wind_up_counters_a_regular_enemy() {
    let mut r = Rig::enemy(weapon(), attack(true, 30), rules_counter());
    r.until_wind_up();
    r.hit(true);
    let h = r.e.tick();
    assert!(!r.attack_running(), "its attack is gone on the hit tick");
    assert_eq!(r.countered.len(), 1, "{:?}", r.countered);
    let (tick, attacker, player, preset) = r.countered[0].clone();
    assert_eq!(
        (tick, attacker, player, preset),
        (h, r.id, PlayerIdx::First, None)
    );
    let stuns = r.named("stun");
    assert_eq!(stuns.len(), 1, "{:?}", r.applied);
    assert_eq!(stuns[0].2, 40);
    assert!(stuns[0].3.is_some(), "the player is its source");
    assert_eq!(r.hits, [(h, 10)], "the hit's damage lands as usual");
    r.idle(60);
    assert!(r.player_hits.is_empty(), "{:?}", r.player_hits);
    // Its rule fires again on the 40th tick after the hit (the stun's end).
    assert!(r.fired.contains(&(h + 40)), "{:?} vs {h}", r.fired);

    // The same run without the flag: the attack goes on and hurts.
    let mut plain = Rig::enemy(weapon(), attack(false, 30), rules_counter());
    plain.until_wind_up();
    plain.hit(true);
    plain.idle(60);
    assert!(plain.countered.is_empty());
    assert!(!plain.player_hits.is_empty(), "the active tick hurts");
}

// --- C2 ---------------------------------------------------------------------------------------

#[test]
fn c2_what_does_not_counter() {
    // Not flagged.
    let mut r = Rig::enemy(weapon(), attack(false, 30), rules_counter());
    r.until_wind_up();
    r.hit(true);
    assert!(r.countered.is_empty() && r.named("stun").is_empty());
    assert!(r.attack_running());
    // An A step in a flagged wind-up.
    let mut a = Rig::enemy(weapon(), attack(true, 30), rules_counter());
    a.until_wind_up();
    a.hit(false);
    assert!(a.countered.is_empty() && a.named("stun").is_empty());
    assert!(a.attack_running());
    // The flagged phase's following active phase: a 5-tick wind-up, the smash on the 6th tick.
    let mut late = Rig::enemy(weapon(), attack(true, 5), rules_counter());
    late.until_wind_up();
    late.idle(5);
    late.hit(true);
    assert!(late.countered.is_empty(), "{:?}", late.countered);
    assert!(late.named("stun").is_empty());
}

// --- C4 ---------------------------------------------------------------------------------------

#[test]
fn c4_an_elite_is_stunned_for_its_share_and_fills_its_gauge() {
    let run = |patch: Value, stagger_max: u32| {
        let mut r = Rig::new(
            weapon(),
            elite_of(rule_of(attack(true, 30)), stagger_max),
            far_boss(),
            patch,
            Target::Enemy(HP),
        );
        r.until_wind_up();
        r.hit(true);
        r
    };
    // The elite's share of the stun is 50 percent of 40; its gauge shows the stun's resisted
    // share (30 x 50 percent = 15) plus the counter's 60.
    let r = run(rules_counter(), 100);
    assert_eq!(r.countered.len(), 1);
    assert_eq!(r.named("stun").len(), 1);
    assert_eq!(r.named("stun")[0].2, 20, "the elite's share of 40");
    assert_eq!(r.gauge(), Some(75));
    // A gauge of 70 staggers it on the hit, once, and its attack is gone.
    let r = run(rules_counter(), 70);
    assert_eq!(r.named("staggered").len(), 1, "{:?}", r.applied);
    assert!(!r.attack_running());
    // `counter` absent: the gauge is the stun's share alone.
    let mut no_counter = rules_counter();
    no_counter["stagger_values"] = json!({ "stun": 30 });
    assert_eq!(run(no_counter, 100).gauge(), Some(15));
    // `counter_stun_ticks` absent: no stun, the gauge is 60.
    let mut no_stun = rules_counter();
    no_stun
        .as_object_mut()
        .unwrap()
        .remove("counter_stun_ticks");
    let r = run(no_stun, 100);
    assert!(r.named("stun").is_empty());
    assert_eq!(r.gauge(), Some(60));
    assert_eq!(r.countered.len(), 1);
}

// --- C5 ---------------------------------------------------------------------------------------

#[test]
fn c5_the_boss_and_a_part_are_countered_and_fill_the_gauge_once() {
    for parts in [false, true] {
        let mut r = Rig::new(
            weapon(),
            vec![],
            boss_with(attack(true, 40), parts),
            rules_counter(),
            Target::Boss,
        );
        r.until_wind_up();
        r.hit(true);
        assert_eq!(r.countered.len(), 1, "parts {parts}: {:?}", r.countered);
        assert_eq!(r.countered[0].1, r.id);
        assert!(!r.attack_running(), "parts {parts}");
        assert!(r.named("stun").is_empty(), "the boss is not stunned");
        assert_eq!(r.gauge(), Some(60), "parts {parts}");
    }
}

// --- C9 ---------------------------------------------------------------------------------------

#[test]
fn c9_the_view_marks_a_counterable_wind_up_only() {
    let mut r = Rig::enemy(weapon(), attack(true, 30), rules_counter());
    r.until_wind_up();
    let seen = r.view().unwrap().attack.unwrap();
    assert!(seen.counterable);
    assert!(
        serde_json::to_string(&seen)
            .unwrap()
            .contains("counterable"),
        "present when true"
    );
    // The active phase: not counterable, and the key is missing.
    for _ in 0..40 {
        r.idle(1);
        if let Some(a) = r.view().and_then(|v| v.attack) {
            if a.kind == stella_rain_core::attack::PhaseKind::Active {
                assert!(!a.counterable);
                assert!(!serde_json::to_string(&a).unwrap().contains("counterable"));
                return;
            }
        }
    }
    panic!("no active phase");
}

// --- C10 --------------------------------------------------------------------------------------

#[test]
fn c10_the_keys_move_no_hash_while_no_phase_is_flagged() {
    let hashes = |patch: Value| {
        let mut r = Rig::enemy(weapon(), attack(false, 30), patch);
        let mut out = vec![];
        for i in 0..600u32 {
            r.step(if i % 7 == 0 {
                Input {
                    smash: true,
                    ..idle()
                }
            } else {
                idle()
            });
            out.push(r.e.state_hash());
        }
        out
    };
    assert_eq!(hashes(json!({})), hashes(rules_counter()));
}

// --- A: super armour --------------------------------------------------------------------------

/// A friendly zone `ahead` along the aim, there for 200 ticks.
fn orb(ahead: i32) -> Value {
    json!({ "type": "zone", "shape": { "type": "circle", "radius": 512 },
            "offset": { "forward": ahead }, "direction": { "type": "aimed", "offset": 0 },
            "wind_up_ticks": 0, "active_ticks": 200 })
}

fn one_orb(ahead: i32) -> Value {
    json!({ "inline": [{ "kind": "active", "ticks": 1, "spawn": [orb(ahead)] }] })
}

/// A step that charges to level 1 at 30 ticks and level 2 at 60, released by itself at 90.
fn charged() -> Value {
    json!({
        "attack": one_orb(1024),
        "charge": { "levels": [
            { "at_ticks": 30, "perfect_ticks": 8, "perfect_pct": 50, "attack": one_orb(2048) },
            { "at_ticks": 60, "perfect_ticks": 8, "perfect_pct": 100, "attack": one_orb(3072) },
        ], "max_ticks": 90, "move_pct": 50 }
    })
}

/// A step held up to 180 ticks: its loop is one active phase.
fn held() -> Value {
    json!({
        "attack": { "inline": [
            { "kind": "active", "ticks": 2, "spawn": [orb(0)] },
            { "kind": "recovery", "ticks": 4 }
        ] },
        "hold": {
            "loop": { "inline": [{ "kind": "active", "ticks": 3, "spawn": [orb(0)] }] },
            "release": { "inline": [{ "kind": "active", "ticks": 1, "spawn": [orb(0)] }] },
            "max_ticks": 180,
        }
    })
}

/// A stage where an enemy 40 pixels above places a zone on the main character on tick 1 that
/// hurts it on tick `wind_up + 1`; `breaks` marks the attack as one that breaks charges.
fn shot_at(combo: Value, wind_up: u32, breaks: bool) -> Stage {
    let attack = json!({ "inline": [{ "kind": "active", "ticks": 1, "breaks_charge": breaks,
        "spawn": [{ "type": "zone", "shape": { "type": "circle", "radius": 3_072 },
                    "anchor": "target", "wind_up_ticks": wind_up, "active_ticks": 2 }] }] });
    let shooter = enemy(
        1000,
        hold(START_X, START_Y - 10_240),
        json!([rule(
            always(),
            json!("player"),
            json!({ "type": "attack", "attack": attack }),
            600
        )]),
    );
    let mut stage = armed(
        stage_full(
            18_000,
            Some(far_boss()),
            vec![wave(1, 1, 0, shooter, START_X, START_Y - 10_240)],
            vec![],
        ),
        weapon_combo(combo),
    );
    stage.sim_version = 2;
    stage.player.hp = 99;
    stage
}

/// What a run of `ticks` ticks under `input(tick)` showed: the ticks of each break, of each hit,
/// and the armour flag of each snapshot (index `t - 1` is tick `t`).
struct Armour {
    charge_broken: Vec<u32>,
    hold_broken: Vec<u32>,
    hit: Vec<u32>,
    armour: Vec<bool>,
    charging: Vec<bool>,
    holding: Vec<bool>,
}

fn armour_run(stage: &Stage, ticks: u32, input: impl Fn(u32) -> Input) -> Armour {
    let mut e = engine(stage);
    let mut a = Armour {
        charge_broken: vec![],
        hold_broken: vec![],
        hit: vec![],
        armour: vec![],
        charging: vec![],
        holding: vec![],
    };
    for t in 1..=ticks {
        e.step(&[input(t), Default::default(), Default::default()]);
        let now = e.tick();
        for ev in e.events() {
            match ev {
                DomainEvent::ChargeBroken => a.charge_broken.push(now),
                DomainEvent::HoldBroken => a.hold_broken.push(now),
                DomainEvent::PlayerHit { .. } => a.hit.push(now),
                _ => {}
            }
        }
        let p = &e.snapshot().players[0];
        a.armour.push(p.super_armour);
        a.charging.push(p.charge.is_some());
        a.holding.push(p.hold.is_some());
    }
    a
}

fn pressed(main: bool, smash: bool) -> impl Fn(u32) -> Input {
    move |_| Input {
        main,
        smash,
        ..idle()
    }
}

#[test]
fn a1_a_charge_at_level_2_is_not_broken_by_a_breaking_hit() {
    let combo = json!({ "main": charged(), "smash": charged() });
    // Level 2 at 60 held ticks: a 69-tick wind-up hurts on tick 70, past it.
    let a = armour_run(&shot_at(combo.clone(), 69, true), 85, pressed(true, false));
    assert_eq!(a.hit, [70]);
    assert!(a.charge_broken.is_empty(), "{:?}", a.charge_broken);
    assert!(a.charging[69], "the charge goes on");
    for t in 1..=90usize {
        if t > 85 {
            break;
        }
        assert_eq!(a.armour[t - 1], t >= 61, "tick {t}");
    }
    // Held 59 (level 1) on tick 60: it breaks as built, and there is no armour.
    let a = armour_run(&shot_at(combo.clone(), 59, true), 70, pressed(true, false));
    assert_eq!(a.hit, [60]);
    assert_eq!(a.charge_broken, [60]);
    assert!(!a.armour[59]);
    // Tick 61 (level 2): it does not break.
    let a = armour_run(&shot_at(combo.clone(), 60, true), 70, pressed(true, false));
    assert_eq!(a.hit, [61]);
    assert!(a.charge_broken.is_empty());
    // A hit that does not break charges never breaks, with or without armour.
    let a = armour_run(&shot_at(combo, 69, false), 85, pressed(true, false));
    assert!(a.charge_broken.is_empty());
}

#[test]
fn a2_a_smash_step_in_an_active_phase_is_not_held_broken() {
    let combo = json!({ "main": held(), "smash": held() });
    // A smash step's hold loops in active phases: the breaking hit on tick 20 does not break it.
    let a = armour_run(&shot_at(combo.clone(), 19, true), 30, pressed(false, true));
    assert_eq!(a.hit, [20]);
    assert!(a.hold_broken.is_empty(), "{:?}", a.hold_broken);
    assert!(a.holding[19], "the hold goes on");
    assert!(a.armour[19]);
    // An A step with the same hold breaks as built, and has no armour.
    let a = armour_run(&shot_at(combo, 19, true), 30, pressed(true, false));
    assert_eq!(a.hold_broken, [20]);
    assert!(!a.armour[19]);
    // A charge at level 1 on a smash step breaks, with no armour.
    let combo = json!({ "main": charged(), "smash": charged() });
    let a = armour_run(&shot_at(combo, 40, true), 50, pressed(false, true));
    assert_eq!(a.charge_broken, [41]);
    assert!(!a.armour[40]);
}

#[test]
fn a3_what_stays_the_evade_and_the_damage() {
    let combo = json!({ "main": charged(), "smash": charged() });
    // The evade at level 2 still breaks the charge on the evade tick.
    let stage = shot_at(combo.clone(), 400, true);
    let a = armour_run(&stage, 80, |t| Input {
        main: true,
        evade: t == 70,
        ..idle()
    });
    assert_eq!(a.charge_broken, [70]);
    // The damage and the hit are the same with and without armour.
    let with = armour_run(&shot_at(combo.clone(), 69, true), 75, pressed(true, false));
    let without = armour_run(&shot_at(combo, 69, true), 75, pressed(false, false));
    assert_eq!(with.hit, without.hit);
}

// --- C6 ---------------------------------------------------------------------------------------

/// A weapon whose smash knocks back by nothing: the enemy stays in the swing, and the knockback
/// (with the rules' `down_ticks`) knocks it down.
fn knockback_weapon() -> Value {
    let mut w = weapon();
    w["inline"]["smash"] = json!({ "type": "knockback", "distance": 0, "ticks": 1 });
    w
}

#[test]
fn c6_a_counter_lands_after_the_smash_s_own_knockdown_on_the_control_list() {
    let mut rules = rules_counter();
    rules["down_ticks"] = json!(30);
    rules["control_repeat_pcts"] = json!([100, 50]);
    rules["control_reset_ticks"] = json!(300);
    rules["control_immune_ticks"] = json!(120);
    let mut r = Rig::enemy(knockback_weapon(), attack(true, 30), rules);
    r.until_wind_up();
    r.hit(true);
    // The knockdown is the first control (all of 30 ticks), the counter's stun the second (half
    // of 40): the counter lands after the smash's own arm.
    let down: Vec<_> = r.named("knocked_down");
    assert_eq!(down.len(), 1, "{:?}", r.applied);
    assert_eq!(down[0].2, 30);
    let stun = r.named("stun");
    assert_eq!(stun.len(), 1, "{:?}", r.applied);
    assert_eq!(stun[0].2, 20);
    assert_eq!(r.countered.len(), 1);
    assert!(!r.attack_running());

    // With one entry in the list the knockdown uses it up: the counter's stun is refused, but the
    // cancel and the report still happen.
    let mut rules = rules_counter();
    rules["down_ticks"] = json!(30);
    rules["control_repeat_pcts"] = json!([100]);
    rules["control_reset_ticks"] = json!(300);
    rules["control_immune_ticks"] = json!(120);
    let mut r = Rig::enemy(knockback_weapon(), attack(true, 30), rules);
    r.until_wind_up();
    r.hit(true);
    assert!(r.named("stun").is_empty(), "{:?}", r.applied);
    assert_eq!(r.countered.len(), 1);
    assert!(!r.attack_running());
}
