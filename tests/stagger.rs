//! The stagger gauge of elites and the boss: the keys, their validators and (below) the rules.
//! Every number is worked out by hand from the placeholders of each test.

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

/// A hit phase of the main character's: one active tick with a circle in front.
fn hit_phase(extra: Value) -> Value {
    let mut p = json!({ "kind": "active", "ticks": 1, "damage": 10,
        "hit_box": { "type": "circle", "offset": { "forward": 1_024 }, "radius": 8_192 } });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

/// A smash step of one circle, with `phases` as its attack.
fn smashing(phases: Value) -> Value {
    json!({ "inline": {
        "combo": { "smash": { "attack": { "inline": phases } } },
        "window_ticks": 30,
        "assist_range": 0,
    } })
}

/// A stage at `sim_version` 2 with `waves`, a `boss` and the main character's weapon `weapon`.
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

fn paths(stage: &Stage) -> Vec<String> {
    invalid(stage).into_iter().map(|(p, _)| p).collect()
}

/// An elite of 1,000 hit points with `extra` keys.
fn elite(extra: Value) -> Value {
    let mut e = enemy(1000, hold(START_X, NEAR_Y), json!([]));
    e["elite"] = json!({ "range": 256 });
    for (k, v) in extra.as_object().unwrap() {
        e[k] = v.clone();
    }
    e
}

fn one(e: Value) -> Vec<Value> {
    vec![wave(1, 1, 0, e, START_X, NEAR_Y)]
}

fn quiet_stage(waves: Vec<Value>, boss: Value) -> Stage {
    stage_of(waves, boss, quiet_weapon())
}

/// The boss of the far-boss fixture with `extra` keys.
fn boss_with(extra: Value) -> Value {
    let mut b = far_boss();
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

#[test]
fn v1_a_stagger_max_is_an_elites_within_its_bound() {
    for ok in [1, 10_000] {
        let stage = quiet_stage(one(elite(json!({ "stagger_max": ok }))), far_boss());
        validate::validate(&stage).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    for bad in [0, 10_001] {
        let stage = quiet_stage(one(elite(json!({ "stagger_max": bad }))), far_boss());
        assert_eq!(paths(&stage), ["$.waves[0].enemy.stagger_max"], "{bad}");
    }
    // A regular enemy (no `elite`) and a companion name none.
    let mut plain = enemy(1000, hold(START_X, NEAR_Y), json!([]));
    plain["stagger_max"] = json!(100);
    let stage = quiet_stage(one(plain), far_boss());
    assert_eq!(paths(&stage), ["$.waves[0].enemy.stagger_max"]);
    let mut companion = companion(10, hold(START_X, START_Y), json!([]));
    companion["stagger_max"] = json!(100);
    let mut stage = quiet_stage(vec![], far_boss());
    stage.companions = vec![serde_json::from_value(companion).unwrap()];
    assert_eq!(paths(&stage), ["$.companions[0].stagger_max"]);
    // At `sim_version` 1.
    let mut stage = quiet_stage(one(elite(json!({ "stagger_max": 100 }))), far_boss());
    stage.sim_version = 1;
    let found = invalid(&stage);
    assert_eq!(found[0].0, "$.waves[0].enemy.stagger_max");
    assert!(found[0].1.contains("needs sim_version 2"), "{found:?}");
}

#[test]
fn v2_an_agents_staggered_ticks_needs_a_stagger_max_and_its_bound() {
    for ok in [1, 600] {
        let e = elite(json!({ "stagger_max": 100, "staggered_ticks": ok }));
        validate::validate(&quiet_stage(one(e), far_boss()))
            .unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    for bad in [0, 601] {
        let e = elite(json!({ "stagger_max": 100, "staggered_ticks": bad }));
        assert_eq!(
            paths(&quiet_stage(one(e), far_boss())),
            ["$.waves[0].enemy.staggered_ticks"],
            "{bad}"
        );
    }
    let e = elite(json!({ "staggered_ticks": 20 }));
    assert_eq!(
        paths(&quiet_stage(one(e), far_boss())),
        ["$.waves[0].enemy.staggered_ticks"]
    );
}

#[test]
fn v3_the_boss_names_its_stagger_max_and_staggered_ticks_the_same_way() {
    for ok in [1, 10_000] {
        let boss = boss_with(json!({ "stagger_max": ok }));
        validate::validate(&quiet_stage(vec![], boss)).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
    }
    for bad in [0, 10_001] {
        let boss = boss_with(json!({ "stagger_max": bad }));
        assert_eq!(
            paths(&quiet_stage(vec![], boss)),
            ["$.boss.stagger_max"],
            "{bad}"
        );
    }
    for bad in [0, 601] {
        let boss = boss_with(json!({ "stagger_max": 100, "staggered_ticks": bad }));
        assert_eq!(
            paths(&quiet_stage(vec![], boss)),
            ["$.boss.staggered_ticks"],
            "{bad}"
        );
    }
    let boss = boss_with(json!({ "staggered_ticks": 20 }));
    assert_eq!(
        paths(&quiet_stage(vec![], boss)),
        ["$.boss.staggered_ticks"]
    );
    let mut stage = quiet_stage(vec![], boss_with(json!({ "stagger_max": 100 })));
    stage.sim_version = 1;
    let found = invalid(&stage);
    assert_eq!(found[0].0, "$.boss.stagger_max");
    assert!(found[0].1.contains("needs sim_version 2"), "{found:?}");
}

#[test]
fn v4_a_phases_stagger_is_a_hit_phase_of_the_main_characters() {
    let ok = smashing(json!([hit_phase(json!({ "stagger": 10_000 }))]));
    validate::validate(&stage_of(vec![], far_boss(), ok)).expect("valid");
    let found = |weapon: Value| paths(&stage_of(vec![], far_boss(), weapon));
    let too_big = found(smashing(json!([hit_phase(json!({ "stagger": 10_001 }))])));
    let wind_up = found(smashing(json!([
        { "kind": "wind_up", "ticks": 3, "damage": 0, "stagger": 1 }
    ])));
    let no_box = found(smashing(json!([
        { "kind": "active", "ticks": 1, "damage": 0, "stagger": 1 }
    ])));
    for list in [too_big, wind_up, no_box] {
        assert!(list.iter().any(|p| p.ends_with(".stagger")), "{list:?}");
    }
    // An enemy's attack is refused.
    let rules = json!([{ "when": always(), "target": "player", "cooldown_ticks": 0,
        "do": { "type": "attack", "attack": { "inline": [
            hit_phase(json!({ "stagger": 1 }))] } } }]);
    let e = enemy(1000, hold(START_X, NEAR_Y), rules);
    let list = paths(&quiet_stage(one(e), far_boss()));
    assert!(list.iter().any(|p| p.ends_with(".stagger")), "{list:?}");
    // At `sim_version` 1.
    let mut stage = stage_of(
        vec![],
        far_boss(),
        smashing(json!([hit_phase(json!({ "stagger": 5 }))])),
    );
    stage.sim_version = 1;
    assert!(paths(&stage).iter().any(|p| p.ends_with(".stagger")));
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
fn v5_a_gauge_needs_staggered_ticks_from_the_rules_or_the_agent() {
    let refused = |stage: &Stage, patch: Value, path: &str| {
        let Err(Error::Invalid(list)) = Engine::new_with(stage, &content_naming(patch)) else {
            panic!("the run started without staggered ticks");
        };
        assert_eq!(list[0].path, path);
    };
    let elite_stage = quiet_stage(one(elite(json!({ "stagger_max": 100 }))), far_boss());
    refused(&elite_stage, json!({}), "$.waves[0].enemy.stagger_max");
    Engine::new_with(
        &elite_stage,
        &content_naming(json!({ "staggered_ticks": 20 })),
    )
    .expect("the rules' ticks");
    let own = quiet_stage(
        one(elite(json!({ "stagger_max": 100, "staggered_ticks": 20 }))),
        far_boss(),
    );
    Engine::new_with(&own, &content_naming(json!({}))).expect("the agent's ticks");

    let boss_stage = quiet_stage(vec![], boss_with(json!({ "stagger_max": 100 })));
    refused(&boss_stage, json!({}), "$.boss.stagger_max");
    Engine::new_with(
        &boss_stage,
        &content_naming(json!({ "staggered_ticks": 20 })),
    )
    .expect("the rules' ticks");
    let own = quiet_stage(
        vec![],
        boss_with(json!({ "stagger_max": 100, "staggered_ticks": 20 })),
    );
    Engine::new_with(&own, &content_naming(json!({}))).expect("the boss's ticks");
}

// --- The rules ------------------------------------------------------------------------------

use stella_rain_core::event::DomainEvent;
use stella_rain_core::fixed::Point;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::{EntityKind, EntityView};

const HP: u32 = 100_000;

/// The harness rules: a smash adds 40, a resisted stun 30, pull 20 and knockback 10; the first
/// bar fills in full and later ones at 50 percent; an idle gauge holds 10 ticks and then loses 5
/// a tick; a stagger lasts 20 ticks, raises the damage taken by 50 percent and puts `vulnerable`
/// of 20 percent.
fn rules_all() -> Value {
    json!({
        "stagger_smash": 40,
        "stagger_values": { "stun": 30, "pull": 20, "knockback": 10 },
        "stagger_refill_pcts": [100, 50],
        "stagger_drain_delay_ticks": 10,
        "stagger_drain_per_tick": 5,
        "stagger_damage_pct": 50,
        "staggered_ticks": 20,
        "vulnerable_pcts": { "stagger": 20 },
    })
}

/// A step: one active tick of 10 damage with `extra`, then 4 ticks of recovery.
fn step_attack(extra: Value) -> Value {
    json!({ "attack": { "inline": [hit_phase(extra), { "kind": "recovery", "ticks": 4 }] } })
}

/// The weapon: an A step and a smash step, with `smash` as the smash effect when given.
fn weapon(a: Value, b: Value, smash: Option<Value>) -> Value {
    let mut w = json!({ "inline": {
        "combo": { "main": step_attack(a), "smash": step_attack(b) },
        "window_ticks": 1,
        "assist_range": 0,
    } });
    if let Some(effect) = smash {
        w["inline"]["smash"] = effect;
    }
    w
}

fn plain_weapon() -> Value {
    weapon(json!({}), json!({}), None)
}

/// An elite of `HP` hit points at the target's place, naming a gauge of 100 unless `extra` says.
fn elite_wave(extra: Value) -> Value {
    let mut e = elite(json!({ "stagger_max": 100 }));
    e["hp"] = json!(HP);
    for (k, v) in extra.as_object().unwrap() {
        e[k] = v.clone();
    }
    wave(TARGET_TICK, 1, 0, e, START_X, NEAR_Y)
}

/// A regular enemy of `HP` hit points at the target's place.
fn plain_wave() -> Value {
    wave(
        TARGET_TICK,
        1,
        0,
        enemy(HP, hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )
}

/// A boss of 900,000 hit points whose body the swing reaches, naming a gauge of 100.
fn boss_body(extra: Value) -> Value {
    let mut b = json!({ "base": "golem", "hp": 900_000, "radius": 8_192,
        "spawn": { "x": START_X, "y": NEAR_Y }, "phases": [{}], "stagger_max": 100 });
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

/// A boss whose body is out of the swing's reach and whose part `horn`, of `part_hp` hit points,
/// is in it.
fn boss_part(part_hp: u32, extra: Value) -> Value {
    let part = json!({ "id": "horn", "asset": "horns_2", "hp": part_hp,
        "offset": { "x": 0, "y": 30_000 }, "radius": 2_048 });
    let mut b = boss_body(extra);
    b["spawn"]["y"] = json!(START_Y - 40_000);
    b["parts"] = json!([part]);
    b
}

/// What a run showed of one target, and the engine.
struct Rig {
    e: Engine,
    id: EntityId,
    hits: Vec<(u32, u32)>,
    ended: Vec<u32>,
    fired: Vec<u32>,
    applied: Vec<(u32, String, Option<EntityId>)>,
    part_broken: Vec<u32>,
    at: Vec<(u32, Point)>,
}

fn content_with(patch: Value) -> RunContent {
    content_naming(patch)
}

impl Rig {
    /// A run of `waves` and `boss` under the weapon, at the tick the waves arrive. The target
    /// is the boss when `boss_is_target`, else the enemy of `HP` hit points.
    fn new(
        weapon: Value,
        waves: Vec<Value>,
        boss: Value,
        patch: Value,
        boss_is_target: bool,
    ) -> Rig {
        let mut stage = stage_of(waves, boss, weapon);
        stage.player.hp = 99;
        let mut e = Engine::new_with(&stage, &content_with(patch)).expect("the run starts");
        while e.tick() < TARGET_TICK {
            e.step(&[idle(), Default::default(), Default::default()]);
        }
        let found: Vec<EntityView> = e.snapshot().entities;
        let id = found
            .iter()
            .find(|en| {
                if boss_is_target {
                    en.kind == EntityKind::Boss
                } else {
                    en.kind == EntityKind::Enemy && en.hp == HP
                }
            })
            .map(|en| en.id)
            .expect("the target");
        Rig {
            e,
            id,
            hits: vec![],
            ended: vec![],
            fired: vec![],
            applied: vec![],
            part_broken: vec![],
            at: vec![],
        }
    }

    fn elite(patch: Value, weapon: Value) -> Rig {
        Rig::new(
            weapon,
            vec![elite_wave(json!({}))],
            far_boss(),
            patch,
            false,
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
                DomainEvent::StaggerEnded { target } if *target == self.id => self.ended.push(now),
                DomainEvent::RuleFired { agent, .. } if *agent == self.id => self.fired.push(now),
                DomainEvent::PartBroken { .. } => self.part_broken.push(now),
                DomainEvent::StatusApplied {
                    target,
                    status,
                    source,
                    ..
                } if *target == self.id => self.applied.push((now, status.to_string(), *source)),
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
        self.step(Input {
            main: !smash,
            smash,
            ..idle()
        });
        self.idle(5);
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

    fn gauge(&self) -> Option<u32> {
        self.view().and_then(|v| v.stagger)
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

    /// Three smashes: the third staggers; then waits the stagger out.
    fn break_once(&mut self) {
        for _ in 0..3 {
            self.smash();
        }
        self.idle(25);
    }
}

#[test]
fn s1_a_smash_fills_the_gauge_and_a_full_gauge_staggers() {
    let mut r = Rig::elite(rules_all(), plain_weapon());
    assert_eq!(r.view().unwrap().stagger_max, Some(100));
    assert_eq!(r.gauge(), Some(0));
    r.smash();
    assert_eq!(r.gauge(), Some(40));
    r.smash();
    assert_eq!(r.gauge(), Some(80));
    r.step(Input {
        smash: true,
        ..idle()
    });
    // 80 + 40 reaches 100: staggered, gauge 0, one status of 20 ticks, one event of its own.
    assert_eq!(r.gauge(), Some(0));
    assert_eq!(r.status("staggered"), [(20, 20)]);
    let landed = r.hits.last().unwrap().0;
    let events: Vec<_> = r
        .applied
        .iter()
        .filter(|(_, s, _)| s == "staggered")
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, landed);
    assert!(events[0].2.is_some(), "the hitting player is its source");
    // An A step adds nothing; a phase of its own `stagger` adds that much.
    let mut a = Rig::elite(rules_all(), plain_weapon());
    a.press(false);
    assert_eq!(a.gauge(), Some(0));
    let w = weapon(json!({}), json!({ "stagger": 70 }), None);
    let mut b = Rig::elite(rules_all(), w);
    b.smash();
    assert_eq!(b.gauge(), Some(70));
}

#[test]
fn s1_the_boss_body_and_a_part_fill_the_bosss_gauge() {
    let mut body = Rig::new(
        plain_weapon(),
        vec![],
        boss_body(json!({})),
        rules_all(),
        true,
    );
    body.smash();
    assert_eq!(body.gauge(), Some(40));
    body.smash();
    body.step(Input {
        smash: true,
        ..idle()
    });
    assert_eq!(body.status("staggered"), [(20, 20)]);
    let mut part = Rig::new(
        plain_weapon(),
        vec![],
        boss_part(1_000, json!({})),
        rules_all(),
        true,
    );
    part.smash();
    assert_eq!(
        part.gauge(),
        Some(40),
        "a hit on the part fills the boss's gauge"
    );
    let views = part.e.snapshot().entities;
    let horn = views
        .iter()
        .find(|v| v.kind == EntityKind::BossPart)
        .unwrap();
    assert_eq!(horn.stagger, None);
    part.smash();
    part.smash();
    assert_eq!(part.status("staggered").len(), 1);
}

#[test]
fn s2_nothing_fills_a_gauge_that_should_not_fill() {
    // An A step, a regular enemy, an elite without a gauge, a staggered target.
    let mut plain = Rig::new(
        plain_weapon(),
        vec![plain_wave()],
        far_boss(),
        rules_all(),
        false,
    );
    plain.smash();
    assert_eq!(plain.gauge(), None);
    let mut none = Rig::new(
        plain_weapon(),
        vec![elite_wave(json!({ "stagger_max": null }))],
        far_boss(),
        rules_all(),
        false,
    );
    none.smash();
    assert_eq!(none.gauge(), None);
    assert!(none.status("staggered").is_empty());
    // No fill while staggered: a fourth smash 6 ticks after the stagger adds nothing.
    let mut r = Rig::elite(rules_all(), plain_weapon());
    for _ in 0..3 {
        r.smash();
    }
    assert_eq!(r.status("staggered").len(), 1);
    r.smash();
    assert_eq!(r.gauge(), Some(0));
    // The rules name nothing: no gauge, no fill, even with `stagger_max` on the elite
    // naming its own ticks.
    let e = elite_wave(json!({ "staggered_ticks": 20 }));
    let mut off = Rig::new(plain_weapon(), vec![e], far_boss(), json!({}), false);
    off.smash();
    assert_eq!(off.gauge(), Some(0));
    // A hit on an invulnerable boss: a boss of a phase with a transition has none to test
    // cheaply; the unit test in `stun.rs` covers the `invulnerable` gate.
}

#[test]
fn s3_later_bars_fill_slower_and_an_idle_gauge_drains() {
    let mut r = Rig::elite(rules_all(), plain_weapon());
    r.break_once();
    r.smash();
    assert_eq!(r.gauge(), Some(20), "the second bar fills at 50 percent");
    for _ in 0..4 {
        r.smash();
    }
    r.idle(25);
    r.smash();
    assert_eq!(
        r.gauge(),
        Some(20),
        "the third bar fills at the last entry too"
    );
    // Drain: after the fill at tick L the gauge holds through L + 10 and loses 5 from L + 11.
    let mut d = Rig::elite(rules_all(), plain_weapon());
    d.step(Input {
        smash: true,
        ..idle()
    });
    let landed = d.hits.last().unwrap().0;
    assert_eq!(d.gauge(), Some(40));
    let mut seen = vec![];
    for _ in 0..12 {
        d.step(idle());
        seen.push((d.e.tick() - landed, d.gauge().unwrap()));
    }
    assert!(
        seen.iter().filter(|(t, _)| *t <= 10).all(|(_, g)| *g == 40),
        "{seen:?}"
    );
    assert_eq!(seen[10], (11, 35), "{seen:?}");
    assert_eq!(seen[11], (12, 30), "{seen:?}");
    d.idle(20);
    assert_eq!(d.gauge(), Some(0));
    // A fill while it drains restarts the delay.
    let mut f = Rig::elite(rules_all(), plain_weapon());
    f.smash();
    f.idle(8);
    f.smash();
    assert_eq!(f.gauge(), Some(80));
    // Without the drain keys it never drops.
    let mut keys = rules_all();
    let m = keys.as_object_mut().unwrap();
    m.remove("stagger_drain_delay_ticks");
    m.remove("stagger_drain_per_tick");
    let mut n = Rig::elite(keys, plain_weapon());
    n.smash();
    n.idle(200);
    assert_eq!(n.gauge(), Some(40));
}

#[test]
fn s4_a_staggered_elite_holds_still_and_loses_its_attack() {
    let mut shooter = elite(json!({ "stagger_max": 100 }));
    shooter["hp"] = json!(HP);
    shooter["rules"] = json!([attack_rule(always(), 1000)]);
    shooter["elite"] = json!({ "range": 100_000 });
    let waves = vec![wave(TARGET_TICK, 1, 0, shooter, START_X, NEAR_Y)];
    let mut a = Rig::new(plain_weapon(), waves, far_boss(), rules_all(), false);
    for _ in 0..40 {
        if a.view().unwrap().attack.is_some() {
            break;
        }
        a.idle(1);
    }
    assert!(a.view().unwrap().attack.is_some(), "it attacks");
    a.smash();
    a.smash();
    a.step(Input {
        smash: true,
        ..idle()
    });
    assert!(
        a.view().unwrap().attack.is_none(),
        "its attack is gone on the hit tick"
    );
    // Its rule that is ready 10 ticks after it appears fires 20 ticks after the stagger instead.
    let mut ready = elite(json!({ "stagger_max": 100 }));
    ready["hp"] = json!(HP);
    ready["elite"] = json!({ "range": 100_000 });
    ready["rules"] = json!([attack_rule(
        json!({ "type": "time_above", "ticks": 10 }),
        1000
    )]);
    let waves = vec![wave(TARGET_TICK, 1, 0, ready, START_X, NEAR_Y)];
    let mut r = Rig::new(plain_weapon(), waves, far_boss(), rules_all(), false);
    r.step(Input {
        smash: true,
        ..idle()
    });
    r.smash();
    r.smash();
    let landed = r.hits.last().unwrap().0;
    r.idle(40);
    assert_eq!(r.fired.first(), Some(&(landed + 20)), "{:?}", r.fired);
    assert_eq!(r.ended, [landed + 20]);
    // The gauge fills again after.
    r.smash();
    assert_eq!(r.gauge(), Some(20));
}

#[test]
fn s4_a_staggered_boss_holds_its_timeline_and_is_not_stunned() {
    let walk =
        json!([{ "type": "move_to", "to": { "x": START_X + 2_000, "y": NEAR_Y }, "ticks": 600 }]);
    let boss = boss_body(json!({ "phases": [{ "timeline": walk, "rules": [] }] }));
    let mut r = Rig::new(plain_weapon(), vec![], boss, rules_all(), true);
    let before = r.view().unwrap().at;
    r.idle(1);
    assert_ne!(r.view().unwrap().at, before, "it walks");
    r.smash();
    r.smash();
    r.step(Input {
        smash: true,
        ..idle()
    });
    let landed = r.hits.last().unwrap().0;
    r.idle(30);
    let held: Vec<_> =
        r.at.iter()
            .filter(|(t, _)| *t >= landed && *t < landed + 20)
            .map(|(_, a)| *a)
            .collect();
    assert_eq!(held.len(), 20);
    assert!(held.iter().all(|a| *a == held[0]), "it holds still");
    let after = r.at.iter().find(|(t, _)| *t >= landed + 20).unwrap().1;
    assert_ne!(after, held[0], "it walks on at the end");
    assert_eq!(r.ended, [landed + 20]);
}

#[test]
fn s5_a_staggered_target_takes_more_damage_and_is_vulnerable() {
    let mut r = Rig::elite(rules_all(), plain_weapon());
    r.smash();
    r.smash();
    r.step(Input {
        smash: true,
        ..idle()
    });
    let hits: Vec<u32> = r.hits.iter().map(|(_, d)| *d).collect();
    assert_eq!(
        hits,
        [10, 10, 10],
        "the hit that fills the gauge is not raised"
    );
    assert_eq!(r.status("vulnerable"), [(20, 20)]);
    r.idle(5);
    // The next hit: 10 raised by (100 + 20) * (100 + 50) / 100 - 100 = 80 percent: 18.
    r.press(false);
    assert_eq!(r.hits.last().unwrap().1, 18);
    // Not a hard control: no stun status.
    assert!(r.status("stun").is_empty());
    // The bonus ends with the stagger.
    r.idle(25);
    r.press(false);
    assert_eq!(r.hits.last().unwrap().1, 10);
    // Without `vulnerable_pcts.stagger` the factor is 50 percent alone: 15.
    let mut keys = rules_all();
    keys["vulnerable_pcts"] = json!({});
    let mut k = Rig::elite(keys, plain_weapon());
    for _ in 0..3 {
        k.smash();
    }
    k.press(false);
    assert_eq!(k.hits.last().unwrap().1, 15);
    assert!(k.status("vulnerable").is_empty());
    // The boss and a part take it too.
    let mut b = Rig::new(
        plain_weapon(),
        vec![],
        boss_body(json!({})),
        rules_all(),
        true,
    );
    for _ in 0..3 {
        b.smash();
    }
    b.press(false);
    assert_eq!(b.hits.last().unwrap().1, 18);
    let mut p = Rig::new(
        plain_weapon(),
        vec![],
        boss_part(1_000, json!({})),
        rules_all(),
        true,
    );
    for _ in 0..3 {
        p.smash();
    }
    let before =
        p.e.snapshot()
            .entities
            .iter()
            .find(|v| v.kind == EntityKind::BossPart)
            .unwrap()
            .hp;
    p.press(false);
    let after =
        p.e.snapshot()
            .entities
            .iter()
            .find(|v| v.kind == EntityKind::BossPart)
            .unwrap()
            .hp;
    assert_eq!(before - after, 18);
}

/// A weapon whose smash step adds no stagger of its own and carries `effect`.
fn controlling(effect: Value) -> Value {
    weapon(json!({}), json!({ "stagger": 0 }), Some(effect))
}

#[test]
fn s6_a_resisted_control_fills_the_gauge() {
    let stun = json!({ "type": "stun", "ticks": 30 });
    let pull = json!({ "type": "pull", "distance": 4_096, "ticks": 10 });
    let knock = json!({ "type": "knockback", "distance": 4_096, "ticks": 10 });
    // The stun on an elite: its half share lands, and 30 * 50 percent fills.
    let mut r = Rig::elite(rules_all(), controlling(stun.clone()));
    r.smash();
    assert_eq!(r.gauge(), Some(15));
    assert_eq!(r.status("stun").len(), 1);
    // The pull and knockback fill an elite that is not moved.
    for (effect, fill) in [(pull.clone(), 20), (knock.clone(), 10)] {
        let mut r = Rig::elite(rules_all(), controlling(effect));
        let at = r.view().unwrap().at;
        r.smash();
        assert_eq!(r.gauge(), Some(fill));
        assert_eq!(r.view().unwrap().at, at, "an elite resists the move");
    }
    // The boss and a part resist the stun whole.
    let mut b = Rig::new(
        controlling(stun.clone()),
        vec![],
        boss_body(json!({})),
        rules_all(),
        true,
    );
    b.smash();
    assert_eq!(b.gauge(), Some(30));
    assert!(b.status("stun").is_empty());
    let mut p = Rig::new(
        controlling(stun),
        vec![],
        boss_part(1_000, json!({})),
        rules_all(),
        true,
    );
    p.smash();
    assert_eq!(p.gauge(), Some(30));
    for (effect, fill) in [(pull.clone(), 20), (knock.clone(), 10)] {
        let mut b = Rig::new(
            controlling(effect),
            vec![],
            boss_body(json!({})),
            rules_all(),
            true,
        );
        b.smash();
        assert_eq!(b.gauge(), Some(fill));
    }
    // A regular enemy is moved as built and has no gauge.
    let mut n = Rig::new(
        controlling(pull),
        vec![plain_wave()],
        far_boss(),
        rules_all(),
        false,
    );
    let at = n.view().unwrap().at;
    n.smash();
    assert_eq!(n.gauge(), None);
    assert_ne!(n.view().unwrap().at, at);
    // A kind the rules leave out adds nothing.
    let mut keys = rules_all();
    keys["stagger_values"] = json!({ "pull": 20 });
    let mut k = Rig::elite(keys, controlling(knock));
    k.smash();
    assert_eq!(k.gauge(), Some(0));
    // The elite that is stun-immune (`stun.elite_pct` of 0 leaves it no ticks) still fills the
    // whole share it resisted: the stun's value scaled by 100 - elite_pct.
    // The base fill and the control fill of one hit run through the refill list one by one:
    // at 50 percent, 40 gives 20 and the scaled 15 gives 7.
    let both = weapon(
        json!({}),
        json!({}),
        Some(json!({ "type": "stun", "ticks": 30 })),
    );
    let mut c = Rig::elite(rules_all(), both);
    c.break_once();
    c.smash();
    assert_eq!(c.gauge(), Some(27));
}

#[test]
fn s7_a_part_breaks_at_its_hit_points_and_each_smash_fills_the_boss() {
    let boss = boss_part(10, json!({}));
    let mut r = Rig::new(plain_weapon(), vec![], boss, rules_all(), true);
    let swings = weapon(json!({ "damage": 5 }), json!({ "damage": 5 }), None);
    let mut r2 = Rig::new(swings, vec![], boss_part(10, json!({})), rules_all(), true);
    r2.smash();
    assert!(r2.part_broken.is_empty());
    assert_eq!(r2.gauge(), Some(40));
    r2.smash();
    assert_eq!(
        r2.part_broken.len(),
        1,
        "PartBroken fires once, on the second hit"
    );
    assert_eq!(r2.gauge(), Some(80));
    r2.press(false);
    assert_eq!(r2.part_broken.len(), 1);
    drop(r);
}

#[test]
fn s9_rules_naming_every_key_move_nothing_while_no_gauge_is_named() {
    let plain = || {
        let mut stage = stage_of(one(elite(json!({}))), far_boss(), plain_weapon());
        stage.player.hp = 99;
        stage
    };
    let hashes = |patch: Value| {
        let mut e = Engine::new_with(&plain(), &content_naming(patch)).unwrap();
        let mut out = vec![];
        for t in 0..600 {
            e.step(&[
                Input {
                    smash: t % 6 == 0,
                    ..idle()
                },
                Default::default(),
                Default::default(),
            ]);
            out.push(e.state_hash());
        }
        out
    };
    assert_eq!(hashes(json!({})), hashes(rules_all()));
}

#[test]
fn s8_a_long_run_of_fills_drains_and_breaks_stays_inside_the_caps() {
    // Smash every 7th tick (past the delay's end in half the gaps) for 2,000 ticks on an elite
    // of a gauge of 100 and 10,000, and read the gauge each tick.
    for max in [100, 10_000] {
        let e = elite_wave(json!({ "stagger_max": max }));
        let mut r = Rig::new(plain_weapon(), vec![e], far_boss(), rules_all(), false);
        let id_hp = |r: &Rig| r.view().map(|v| v.hp).unwrap_or(0);
        let mut staggered_once = false;
        for t in 0..2_000u32 {
            let smash = t % 7 == 0 || (t / 300) % 2 == 1 && t % 25 == 0;
            r.step(Input { smash, ..idle() });
            let Some(v) = r.view() else { break };
            let gauge = v.stagger.expect("the gauge is in the snapshot");
            assert!(gauge <= max, "tick {t}: {gauge} > {max}");
            assert_eq!(v.stagger_max, Some(max));
            let st = r.status("staggered");
            assert!(st.iter().all(|(left, total)| left <= total && *total <= 20));
            staggered_once |= !st.is_empty();
            assert!(id_hp(&r) > 0 || t > 0);
        }
        if max == 100 {
            assert!(staggered_once, "a gauge of 100 breaks in 2,000 ticks");
        }
    }
}
