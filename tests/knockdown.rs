//! The knockdown: a smash's knockback that lands on a regular enemy knocks it down once the rules
//! name `down_ticks`; a smash hit on a downed enemy deals a bonus; a knockback an elite, the boss or
//! a part resists fills its gauge. Every number is worked out by hand from the placeholders of each
//! test (30 ticks, 50 and 20 percent, 5 and 10 of gauge, a push of 4,096 over 10 ticks).

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::Point;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::pack::ContentFiles;
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

/// A step: one active tick of 10 damage with `extra`, then 4 ticks of recovery.
fn step_attack(extra: Value) -> Value {
    json!({ "attack": { "inline": [hit_phase(extra), { "kind": "recovery", "ticks": 4 }] } })
}

/// The weapon: an A step and a smash step, with `smash` as the smash effect when given.
fn weapon(smash: Option<Value>) -> Value {
    let mut w = json!({ "inline": {
        "combo": { "main": step_attack(json!({})), "smash": step_attack(json!({})) },
        "window_ticks": 1,
        "assist_range": 0,
    } });
    if let Some(effect) = smash {
        w["inline"]["smash"] = effect;
    }
    w
}

fn knockback(distance: i32, ticks: u32) -> Value {
    json!({ "type": "knockback", "distance": distance, "ticks": ticks })
}

/// A knockback weapon whose push is short enough that the enemy stays inside the swing.
fn stay_put() -> Value {
    weapon(Some(knockback(0, 1)))
}

/// The harness rules: a knockdown of 30 ticks, a bonus of 50 percent, `vulnerable` of 20 percent,
/// a resisted knockback adding 10 and its knockdown 5; a stagger lasts 20 ticks.
fn rules_down() -> Value {
    json!({
        "down_ticks": 30,
        "down_bonus_pct": 50,
        "vulnerable_pcts": { "knockdown": 20 },
        "stagger_values": { "knockback": 10, "knockdown": 5 },
        "staggered_ticks": 20,
    })
}

fn content_with(patch: Value) -> RunContent {
    let mut v: Value = serde_json::from_slice(ContentFiles::bundled_file("rules")).unwrap();
    for (k, x) in patch.as_object().unwrap() {
        v[k] = x.clone();
    }
    let rules = Rules::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();
    design_content().with_rules(rules)
}

/// An enemy of `hp` hit points at the target's place, with `rules` and `extra` keys.
fn plain(hp: u32, rules: Value) -> Value {
    enemy(hp, hold(START_X, NEAR_Y), rules)
}

fn wave_of(e: Value) -> Vec<Value> {
    vec![wave(TARGET_TICK, 1, 0, e, START_X, NEAR_Y)]
}

fn elite_wave() -> Vec<Value> {
    let mut e = plain(HP, json!([]));
    e["elite"] = json!({ "range": 256 });
    e["stagger_max"] = json!(100);
    wave_of(e)
}

/// A boss of 900,000 hit points whose body the swing reaches, naming a gauge of 100.
fn boss_body() -> Value {
    json!({ "base": "golem", "hp": 900_000, "radius": 8_192,
        "spawn": { "x": START_X, "y": NEAR_Y }, "phases": [{}], "stagger_max": 100 })
}

/// A boss whose body is out of the swing's reach and whose part `horn` is in it.
fn boss_part() -> Value {
    let part = json!({ "id": "horn", "asset": "horns_2", "hp": 1_000,
        "offset": { "x": 0, "y": 30_000 }, "radius": 2_048 });
    let mut b = boss_body();
    b["spawn"]["y"] = json!(START_Y - 40_000);
    b["parts"] = json!([part]);
    b
}

fn stage_of(waves: Vec<Value>, boss: Value, weapon: Value) -> Stage {
    let mut stage = armed(stage_full(900, Some(boss), waves, vec![]), weapon);
    stage.sim_version = 2;
    stage.player.hp = 99;
    stage
}

/// What a run showed of one target, and the engine.
struct Rig {
    e: Engine,
    id: EntityId,
    hits: Vec<(u32, u32)>,
    down_ended: Vec<u32>,
    stun_ended: Vec<u32>,
    flinch_ended: Vec<u32>,
    fired: Vec<u32>,
    /// `(tick, status, duration, source)` of every `StatusApplied` on the target.
    applied: Vec<(u32, String, u32, Option<EntityId>)>,
    at: Vec<(u32, Point)>,
}

impl Rig {
    /// A run of `waves` and `boss` under the weapon, at the tick the waves arrive. The target is
    /// the boss when `boss_is_target`, else the enemy of `hp` hit points.
    fn new(weapon: Value, waves: Vec<Value>, boss: Value, patch: Value, target: Target) -> Rig {
        let stage = stage_of(waves, boss, weapon);
        let mut e = Engine::new_with(&stage, &content_with(patch)).expect("the run starts");
        while e.tick() < TARGET_TICK {
            e.step(&[idle(), Default::default(), Default::default()]);
        }
        let found: Vec<EntityView> = e.snapshot().entities;
        let id = found
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
            hits: vec![],
            down_ended: vec![],
            stun_ended: vec![],
            flinch_ended: vec![],
            fired: vec![],
            applied: vec![],
            at: vec![],
        }
    }

    /// A regular enemy of `HP` hit points with `rules`.
    fn enemy(weapon: Value, rules: Value, patch: Value) -> Rig {
        Rig::new(
            weapon,
            wave_of(plain(HP, rules)),
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
                DomainEvent::Hit { target, damage, .. } if *target == self.id => {
                    self.hits.push((now, *damage));
                }
                DomainEvent::KnockdownEnded { target } if *target == self.id => {
                    self.down_ended.push(now);
                }
                DomainEvent::StunEnded { target } if *target == self.id => {
                    self.stun_ended.push(now);
                }
                DomainEvent::FlinchEnded { target } if *target == self.id => {
                    self.flinch_ended.push(now);
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
        if let Some(v) = self.view() {
            self.at.push((now, v.at));
        }
    }

    fn idle(&mut self, n: u32) {
        for _ in 0..n {
            self.step(idle());
        }
    }

    /// A press of A or smash, then 5 idle ticks: one hit per press, 6 ticks apart.
    fn press(&mut self, smash: bool) {
        self.hit(smash);
        self.idle(5);
    }

    /// The press alone: the tick the hit lands.
    fn hit(&mut self, smash: bool) {
        self.step(Input {
            main: !smash,
            smash,
            ..idle()
        });
    }

    fn smash(&mut self) {
        self.press(true);
    }

    fn view(&self) -> Option<EntityView> {
        self.e
            .snapshot()
            .entities
            .into_iter()
            .find(|en| en.id == self.id)
    }

    fn status(&self, id: &str) -> Vec<(u32, u32)> {
        self.view()
            .map(|v| {
                v.statuses
                    .iter()
                    .filter(|s| s.id.as_str() == id)
                    .map(|s| (s.remaining, s.total))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn gauge(&self) -> Option<u32> {
        self.view().and_then(|v| v.stagger)
    }

    fn last_hit(&self) -> u32 {
        self.hits.last().expect("a hit").1
    }

    fn knocked_down_events(&self) -> Vec<&(u32, String, u32, Option<EntityId>)> {
        self.applied
            .iter()
            .filter(|(_, s, _, _)| s == "knocked_down")
            .collect()
    }
}

#[derive(Clone, Copy)]
enum Target {
    Boss,
    Enemy(u32),
}

fn ready_rule() -> Value {
    json!([attack_rule(
        json!({ "type": "time_above", "ticks": 10 }),
        1000
    )])
}

// --- K1 ---------------------------------------------------------------------------------------

#[test]
fn k1_a_knockback_knocks_a_regular_enemy_down_and_its_push_plays_out() {
    // Its running attack is gone on the hit tick, and the status shows its ticks.
    let mut a = Rig::enemy(
        weapon(Some(knockback(4_096, 10))),
        json!([attack_rule(always(), 1000)]),
        rules_down(),
    );
    for _ in 0..40 {
        if a.view().unwrap().attack.is_some() {
            break;
        }
        a.idle(1);
    }
    assert!(a.view().unwrap().attack.is_some(), "it attacks");
    a.hit(true);
    assert!(
        a.view().unwrap().attack.is_none(),
        "its attack is gone on the hit tick"
    );
    assert_eq!(a.status("knocked_down"), [(30, 30)]);
    let events = a.knocked_down_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].2, 30);
    assert!(events[0].3.is_some(), "the hitting player is its source");

    // Its rule that is ready 10 ticks after it appears fires 30 ticks after the hit instead,
    // and the end is reported once, on that tick.
    let mut r = Rig::enemy(
        weapon(Some(knockback(4_096, 10))),
        ready_rule(),
        rules_down(),
    );
    r.hit(true);
    let landed = r.hits.last().unwrap().0;
    r.idle(50);
    assert_eq!(r.fired.first(), Some(&(landed + 30)), "{:?}", r.fired);
    assert_eq!(r.down_ended, [landed + 30]);
    assert!(r.stun_ended.is_empty(), "a knockdown is no stun");
    assert!(r.status("stun").is_empty());

    // The push plays out while it lies: 4,096 away from the player, then it holds still.
    let goal = Point {
        x: stella_rain_core::fixed::Fx(START_X),
        y: stella_rain_core::fixed::Fx(NEAR_Y - 4_096),
    };
    let at = |tick: u32| r.at.iter().find(|(t, _)| *t == tick).unwrap().1;
    assert_eq!(at(landed + 12), goal, "{:?}", &r.at[..16]);
    assert_eq!(
        at(landed + 25),
        goal,
        "it does not move on its own while down"
    );
}

#[test]
fn k1_without_down_ticks_a_knockback_is_a_push_only() {
    let mut r = Rig::enemy(weapon(Some(knockback(4_096, 10))), ready_rule(), json!({}));
    r.hit(true);
    let landed = r.hits.last().unwrap().0;
    r.idle(30);
    assert!(r.status("knocked_down").is_empty());
    assert!(r.knocked_down_events().is_empty());
    assert!(r.down_ended.is_empty());
    assert!(
        r.fired.first().is_some_and(|t| *t <= landed + 12),
        "it acts at once"
    );
    let moved = r.at.iter().find(|(t, _)| *t == landed + 12).unwrap().1;
    assert_ne!(moved.y.0, NEAR_Y, "pushed as built");
}

// --- K2 ---------------------------------------------------------------------------------------

#[test]
fn k2_only_a_knockback_on_a_living_regular_enemy_knocks_down() {
    for effect in [
        json!({ "type": "pull", "distance": 4_096, "ticks": 10 }),
        json!({ "type": "stun", "ticks": 10 }),
        json!({ "type": "bleed" }),
    ] {
        let mut r = Rig::enemy(weapon(Some(effect.clone())), json!([]), rules_down());
        r.smash();
        assert!(r.status("knocked_down").is_empty(), "{effect}");
        assert!(r.knocked_down_events().is_empty(), "{effect}");
    }
    // An A step beside a knockback smash: no knockback, no knockdown.
    let mut a = Rig::enemy(stay_put(), json!([]), rules_down());
    a.press(false);
    assert!(a.status("knocked_down").is_empty());
    // A hit that kills it shows no status.
    let mut dead = Rig::new(
        stay_put(),
        wave_of(plain(10, json!([]))),
        far_boss(),
        rules_down(),
        Target::Enemy(10),
    );
    dead.smash();
    assert!(dead.view().is_none(), "it died");
    assert!(dead.knocked_down_events().is_empty());
    // An elite, the boss and a part are neither knocked down nor moved.
    let mut elite = Rig::new(
        weapon(Some(knockback(4_096, 10))),
        elite_wave(),
        far_boss(),
        rules_down(),
        Target::Enemy(HP),
    );
    elite.smash();
    elite.idle(12);
    assert!(elite.status("knocked_down").is_empty());
    assert_eq!(elite.view().unwrap().at.y.0, NEAR_Y);
    for boss in [boss_body(), boss_part()] {
        let mut b = Rig::new(
            weapon(Some(knockback(4_096, 10))),
            vec![],
            boss,
            rules_down(),
            Target::Boss,
        );
        let before = b.view().unwrap().at;
        b.smash();
        b.idle(12);
        assert!(b.status("knocked_down").is_empty());
        assert_eq!(b.view().unwrap().at, before);
    }
    // An enemy on the player's very centre has no direction: it is knocked down, moved nowhere.
    let centre = vec![wave(
        TARGET_TICK,
        1,
        0,
        enemy(HP, hold(START_X, START_Y), json!([])),
        START_X,
        START_Y,
    )];
    let mut c = Rig::new(
        weapon(Some(knockback(4_096, 10))),
        centre,
        far_boss(),
        rules_down(),
        Target::Enemy(HP),
    );
    c.smash();
    assert_eq!(c.status("knocked_down").len(), 1);
    assert_eq!(c.view().unwrap().at.y.0, START_Y);
}

// --- K4 ---------------------------------------------------------------------------------------

#[test]
fn k4_the_control_list_scales_and_refuses_knockdowns() {
    let mut keys = rules_down();
    keys["control_repeat_pcts"] = json!([100, 50]);
    keys["control_reset_ticks"] = json!(600);
    keys["control_immune_ticks"] = json!(100);
    let mut r = Rig::enemy(stay_put(), json!([]), keys.clone());
    r.smash();
    assert_eq!(r.status("knocked_down"), [(25, 30)], "the first keeps 30");
    r.idle(30);
    r.smash();
    let second = r.knocked_down_events();
    assert_eq!(second.len(), 2);
    assert_eq!(second[1].2, 15, "the second keeps 15");
    r.idle(30);
    r.smash();
    assert_eq!(r.knocked_down_events().len(), 2, "the third is refused");
    assert!(r.status("knocked_down").is_empty());

    // With no list the end of a knockdown starts the run stats' stun immunity.
    let mut plain_rules = rules_down();
    plain_rules["vulnerable_pcts"] = json!({});
    let mut n = Rig::enemy(stay_put(), json!([]), plain_rules);
    n.hit(true);
    n.idle(31);
    assert_eq!(n.down_ended.len(), 1);
    n.smash();
    assert_eq!(
        n.knocked_down_events().len(),
        1,
        "immune after the end: no second knockdown"
    );
}

// --- K6, K7 -----------------------------------------------------------------------------------

#[test]
fn k6_a_knockdown_puts_vulnerable_for_its_ticks() {
    let mut r = Rig::enemy(stay_put(), json!([]), rules_down());
    r.hit(true);
    assert_eq!(r.status("vulnerable"), [(30, 30)]);
    let mut keys = rules_down();
    keys["vulnerable_pcts"] = json!({});
    let mut off = Rig::enemy(stay_put(), json!([]), keys);
    off.hit(true);
    assert!(off.status("vulnerable").is_empty());
}

#[test]
fn k7_a_smash_hit_on_a_downed_enemy_deals_the_bonus() {
    let mut r = Rig::enemy(stay_put(), json!([]), rules_down());
    r.smash();
    assert_eq!(r.last_hit(), 10, "the knocking hit is not raised");
    // (100 + 20) * (100 + 50) / 100 = 180 percent: 18.
    r.smash();
    assert_eq!(r.last_hit(), 18);
    // An A step: vulnerable alone, 12.
    r.press(false);
    assert_eq!(r.last_hit(), 12);
    // The bonus ends with the knockdown.
    r.idle(30);
    assert_eq!(r.down_ended.len(), 1);
    r.press(false);
    assert_eq!(r.last_hit(), 10);

    // Without the `vulnerable` key the bonus is 50 percent alone: 15.
    let mut keys = rules_down();
    keys["vulnerable_pcts"] = json!({});
    let mut k = Rig::enemy(stay_put(), json!([]), keys);
    k.smash();
    k.smash();
    assert_eq!(k.last_hit(), 15);
    // Without the bonus key a downed enemy takes 10 from a smash.
    let mut keys = rules_down();
    keys.as_object_mut().unwrap().remove("down_bonus_pct");
    keys["vulnerable_pcts"] = json!({});
    let mut none = Rig::enemy(stay_put(), json!([]), keys);
    none.smash();
    none.smash();
    assert_eq!(none.last_hit(), 10);
    // A smash on a standing enemy is not raised.
    let mut standing = Rig::enemy(weapon(None), json!([]), rules_down());
    standing.smash();
    assert_eq!(standing.last_hit(), 10);
}

// --- K8 ----------------------------------------------------------------------------------------

#[test]
fn k8_a_resisted_knockback_fills_the_gauge_by_knockback_and_knockdown() {
    let kb = || weapon(Some(knockback(0, 1)));
    // 10 (knockback) + 5 (knockdown) in one fill.
    let mut e = Rig::new(
        kb(),
        elite_wave(),
        far_boss(),
        rules_down(),
        Target::Enemy(HP),
    );
    e.smash();
    assert_eq!(e.gauge(), Some(15));
    let mut b = Rig::new(kb(), vec![], boss_body(), rules_down(), Target::Boss);
    b.smash();
    assert_eq!(b.gauge(), Some(15));
    let mut p = Rig::new(kb(), vec![], boss_part(), rules_down(), Target::Boss);
    p.smash();
    assert_eq!(p.gauge(), Some(15), "the part's hit fills the boss's gauge");
    // Without the knockdown value only the knockback's.
    let mut keys = rules_down();
    keys["stagger_values"] = json!({ "knockback": 10 });
    let mut o = Rig::new(kb(), elite_wave(), far_boss(), keys, Target::Enemy(HP));
    o.smash();
    assert_eq!(o.gauge(), Some(10));
    // A regular enemy has no gauge.
    let mut r = Rig::enemy(kb(), json!([]), rules_down());
    r.smash();
    assert_eq!(r.gauge(), None);
    // With the rules naming no down_ticks nothing changes: 10.
    let mut keys = rules_down();
    let m = keys.as_object_mut().unwrap();
    m.remove("down_ticks");
    m.remove("down_bonus_pct");
    keys["vulnerable_pcts"] = json!({});
    keys["stagger_values"] = json!({ "knockback": 10 });
    let mut d = Rig::new(kb(), elite_wave(), far_boss(), keys, Target::Enemy(HP));
    d.smash();
    assert_eq!(d.gauge(), Some(10));
}

// --- K11 ---------------------------------------------------------------------------------------

#[test]
fn k11_naming_the_keys_without_a_knockback_changes_no_hash() {
    let run = |patch: Value| {
        let stage = stage_of(wave_of(plain(HP, ready_rule())), far_boss(), weapon(None));
        let mut e = Engine::new_with(&stage, &content_with(patch)).unwrap();
        let mut hashes = vec![];
        for t in 0..600u32 {
            let smash = t % 12 == 0;
            e.step(&[
                Input {
                    main: !smash && t % 6 == 0,
                    smash,
                    ..idle()
                },
                Default::default(),
                Default::default(),
            ]);
            hashes.push(e.state_hash());
        }
        hashes
    };
    assert_eq!(run(json!({})), run(rules_down()));
}
