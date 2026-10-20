//! The pack leader (ADR-090 point 5, app#259): an elite that comes with an escort on a ring
//! around it and carries an aura. While it is alive, every non-elite enemy within the aura's
//! radius of its centre deals more damage; the leader and the other elites are not helped, and
//! auras do not stack, the highest bonus in range counts.

use serde_json::{Value, json};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::trig;
use stella_rain_core::validate::{self, Error, limits};

use crate::common;

use common::*;

/// 100 pixels to the side of the main character: out of its line of fire.
const SIDE: (i32, i32) = (START_X + 25_600, START_Y);
/// 40 pixels: the escort's ring.
const RING: i32 = 10_240;
/// The attacker's shot: 7 hurts for 8 with a quarter more (8.75 rounded down), 10 with half more.
const SHOT: u32 = 7;
/// The attacker stands 20 pixels above the main character.
const ATTACKER: (i32, i32) = (START_X, START_Y - 5_120);
/// Ticks between its shots: longer than the main character's invulnerability after a hit.
const COOLDOWN: u32 = 70;

fn pack_aura(radius: i32, percent: u32) -> Value {
    json!({ "radius": radius, "damage_percent": percent })
}

fn escort_of(count: u8, radius: i32) -> Value {
    json!({ "enemy": enemy(5, json!({ "type": "straight", "vx": 0, "vy": 0 }), json!([])),
            "count": count, "radius": radius })
}

/// An enemy that shoots the main character once in `COOLDOWN` ticks for `SHOT`.
fn shooter() -> Value {
    let attack = json!({ "inline": [{ "kind": "active", "ticks": 1, "damage": SHOT,
        "spawn": [{ "type": "fire", "speed": 640, "direction": { "type": "aimed", "offset": 0 } }] }] });
    enemy(
        100_000,
        hold(ATTACKER.0, ATTACKER.1),
        json!([{ "when": always(), "target": "player", "do": { "type": "attack", "attack": attack },
                 "cooldown_ticks": COOLDOWN }]),
    )
}

/// The same, as an elite, at the same place.
fn elite_shooter() -> Value {
    let mut e = shooter();
    e["elite"] = json!({ "range": 262_144 });
    e
}

/// A leader that holds `at` and does nothing, with `aura` and, if any, `escort`.
fn leader(at: (i32, i32), aura: Value, escort: Option<Value>) -> Value {
    let mut e = enemy(100_000, hold(at.0, at.1), json!([]));
    e["radius"] = json!(3072);
    e["elite"] = json!({ "range": 256, "aura": aura });
    if let Some(escort) = escort {
        e["elite"]["escort"] = escort;
    }
    e
}

fn stage(waves: Vec<Value>) -> Stage {
    quiet(stage_full(18_000, Some(far_boss()), waves, vec![]))
}

/// Ticks and damage of each time the main character was hurt, in 99 hit points.
fn hurts(events: &[(u32, DomainEvent)]) -> Vec<(u32, u32)> {
    let mut hp = 99;
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::PlayerHit { hp_left } => {
                let dealt = hp - hp_left;
                hp = *hp_left;
                Some((*t, dealt))
            }
            _ => None,
        })
        .collect()
}

/// The damage of the first shot of `shooter_def` that lands, with `others` in the stage.
fn first_hurt(shooter_def: Value, others: Vec<Value>) -> u32 {
    let mut waves = vec![wave(1, 1, 0, shooter_def, ATTACKER.0, ATTACKER.1)];
    waves.extend(others);
    let mut e = engine(&stage(waves));
    let events = run_until(&mut e, idle(), 120, |_| false);
    hurts(&events).first().expect("the shot landed").1
}

fn at_leader(at: (i32, i32), aura: Value) -> Value {
    wave(1, 1, 0, leader(at, aura, None), at.0, at.1)
}

/// 10 pixels to the side of the attacker.
const NEAR: (i32, i32) = (ATTACKER.0 + 2_560, ATTACKER.1);

fn positions(e: &Engine, elite: bool) -> Vec<(i32, i32)> {
    e.snapshot()
        .entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy && v.elite == elite)
        .map(|v| (v.at.x.0, v.at.y.0))
        .collect()
}

fn ring(at: (i32, i32), count: i32, radius: i32) -> Vec<(i32, i32)> {
    (0..count)
        .map(|i| {
            let angle = i * trig::PER_TURN / count;
            let (sin, cos) = trig::sin_cos(angle);
            let off = |unit: i32| {
                i32::try_from(i64::from(radius) * i64::from(unit) / i64::from(trig::SCALE)).unwrap()
            };
            (at.0 + off(cos), at.1 + off(sin))
        })
        .collect()
}

fn sorted(mut v: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    v.sort_unstable();
    v
}

// --- The escort --------------------------------------------------------------------------------

#[test]
fn the_escort_spawns_with_the_leader_on_a_ring_around_it() {
    let lead = leader(SIDE, pack_aura(20_480, 25), Some(escort_of(4, RING)));
    let mut e = engine(&stage(vec![wave(5, 1, 0, lead, SIDE.0, SIDE.1)]));
    run_until(&mut e, idle(), 4, |_| false);
    assert_eq!(positions(&e, false).len(), 0, "nothing before its tick");
    run_until(&mut e, idle(), 1, |_| false);
    assert_eq!(positions(&e, true), [SIDE]);
    assert_eq!(
        sorted(positions(&e, false)),
        sorted(ring(SIDE, 4, RING)),
        "four escorts on a ring of 40 pixels, evenly spaced from the east, on its tick"
    );
}

#[test]
fn the_escort_is_ordinary_enemies_after_the_leader_in_id_order() {
    let lead = leader(SIDE, pack_aura(20_480, 25), Some(escort_of(3, RING)));
    let mut e = engine(&stage(vec![wave(1, 1, 0, lead, SIDE.0, SIDE.1)]));
    run_until(&mut e, idle(), 1, |_| false);
    let snap = e.snapshot();
    let enemies: Vec<_> = snap
        .entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .collect();
    assert_eq!(enemies.len(), 4);
    assert!(enemies[0].elite && enemies[1..].iter().all(|v| !v.elite));
    assert!(enemies.windows(2).all(|w| w[0].id < w[1].id));
}

#[test]
fn the_enemy_cap_trims_the_escort() {
    // 497 enemies and the leader make 498 of the 500 alive at most: two escorts fit.
    let filler = |count: u16, x: i32| {
        wave(
            1,
            count,
            1,
            enemy(10, hold(x, 40_000), json!([])),
            x,
            40_000,
        )
    };
    let mut waves = vec![
        filler(100, 1_000),
        filler(100, 2_000),
        filler(100, 3_000),
        filler(100, 4_000),
        filler(97, 5_000),
    ];
    let lead = leader(SIDE, pack_aura(20_480, 25), Some(escort_of(4, RING)));
    waves.push(wave(150, 1, 0, lead, SIDE.0, SIDE.1));
    let mut e = engine(&stage(waves));
    run_until(&mut e, idle(), 149, |_| false);
    assert_eq!(e.snapshot().entities.len() - 1, 497);
    run_until(&mut e, idle(), 1, |_| false);
    let ring = ring(SIDE, 4, RING);
    let near: Vec<_> = positions(&e, false)
        .into_iter()
        .filter(|p| ring.contains(p))
        .collect();
    assert_eq!(near, ring[..2], "the first two on the ring spawned");
    assert_eq!(e.snapshot().entities.len() - 1, 500);
}

// --- The aura ----------------------------------------------------------------------------------

#[test]
fn an_enemy_within_the_radius_deals_a_quarter_more_rounded_down() {
    assert_eq!(first_hurt(shooter(), vec![]), SHOT, "no leader: base");
    let near = at_leader(NEAR, pack_aura(5_120, 25));
    assert_eq!(first_hurt(shooter(), vec![near]), 8, "7 * 125 / 100 = 8");
}

#[test]
fn an_enemy_outside_the_radius_deals_the_base() {
    // 10 pixels away, radius 9 pixels.
    let far = at_leader(NEAR, pack_aura(2_304, 25));
    assert_eq!(first_hurt(shooter(), vec![far]), SHOT);
}

#[test]
fn the_radius_includes_its_edge() {
    let edge = at_leader(NEAR, pack_aura(2_560, 25));
    assert_eq!(first_hurt(shooter(), vec![edge]), 8);
}

#[test]
fn two_leaders_do_not_stack_the_highest_bonus_wins() {
    let a = at_leader(NEAR, pack_aura(5_120, 25));
    let b = at_leader((NEAR.0, NEAR.1 - 1_024), pack_aura(5_120, 50));
    assert_eq!(first_hurt(shooter(), vec![a.clone(), b.clone()]), 10);
    // Out of range, the higher one does not count.
    let b_far = at_leader((NEAR.0, NEAR.1 - 60_000), pack_aura(5_120, 50));
    assert_eq!(first_hurt(shooter(), vec![a, b_far]), 8);
}

#[test]
fn an_elite_is_not_helped_and_neither_is_the_leader() {
    let aura = at_leader(NEAR, pack_aura(5_120, 25));
    assert_eq!(first_hurt(elite_shooter(), vec![aura]), SHOT);
    // A leader that shoots, standing in its own aura.
    let mut own = elite_shooter();
    own["elite"]["aura"] = pack_aura(5_120, 25);
    assert_eq!(first_hurt(own, vec![]), SHOT);
}

#[test]
fn the_bonus_ends_with_the_leader_s_death_tick() {
    // The leader stands 107 pixels above the main character, in the line of its shots, and
    // dies to the first one; the shooter, 100 pixels to its side, is in its aura. `from` is the
    // tick the main character starts firing. Its shots land on every tick from there.
    let lead_at = (START_X, 120_000);
    let side = (START_X + 25_600, START_Y);
    let mut found_same_tick = false;
    for from in 0..COOLDOWN {
        let mut shooter = shooter();
        shooter["movement"] = hold(side.0, side.1);
        let mut waves = vec![wave(1, 1, 0, shooter, side.0, side.1)];
        let mut lead = leader(lead_at, pack_aura(40_960, 25), None);
        lead["hp"] = json!(10);
        waves.push(wave(1, 1, 0, lead, lead_at.0, lead_at.1));
        let mut e = engine(&stage(waves));
        let mut events = Vec::new();
        for _ in 0..400 {
            let input = Input {
                main: e.tick() >= from && e.tick() % 6 == 0,
                ..idle()
            };
            e.step(input);
            events.extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        }
        let Some(&died) = ticks_of(&events, |ev| matches!(ev, DomainEvent::Died { .. })).first()
        else {
            continue;
        };
        let hurt = hurts(&events);
        for (tick, dealt) in &hurt {
            // Its own tick still has the aura; the next has none.
            let expected = if *tick <= died { 8 } else { SHOT };
            assert_eq!(
                *dealt, expected,
                "from {from}: hurt on {tick}, died on {died}"
            );
            found_same_tick |= *tick == died;
        }
    }
    assert!(found_same_tick, "a shot landed on the leader's death tick");
}

// --- Data and validation -----------------------------------------------------------------------

#[test]
fn the_bundled_mode_has_the_leader_in_the_middle_and_late_tiers_only() {
    let mode = TimeLimit::bundled();
    let t = mode
        .elite_types()
        .iter()
        .find(|t| t.id.0 == "elite_leader")
        .expect("elite_leader is defined");
    let elite = serde_json::to_value(&t.enemy.elite).unwrap();
    assert_eq!(elite["aura"], pack_aura(40_960, 25), "160 pixels, +25%");
    assert_eq!(elite["escort"]["count"], 4);
    assert_eq!(elite["escort"]["radius"], 15_360, "60 pixels");
    assert_eq!(elite["escort"]["enemy"]["base"], "bat");
    for (i, tier) in mode.elite_table().iter().enumerate() {
        let weight = tier
            .weights
            .iter()
            .find(|w| w.elite.0 == "elite_leader")
            .map(|w| w.weight);
        assert_eq!(weight, (i > 0).then_some(1), "tier {i}");
    }
}

fn issues(stage: &Stage) -> Vec<String> {
    match validate::validate(stage) {
        Ok(()) => Vec::new(),
        Err(Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

fn with_elite(aura: Value, escort: Option<Value>) -> Stage {
    stage(vec![wave(1, 1, 0, leader(SIDE, aura, escort), 1000, 1000)])
}

#[test]
fn the_aura_is_bounded() {
    // The cap on the bonus: 200 percent.
    let (max, pct) = (limits::MAX_LEN, 200u16);
    let e = "$.waves[0].enemy.elite.aura";
    let cases: [(Value, Vec<String>); 7] = [
        (pack_aura(1, 1), vec![]),
        (pack_aura(max, u32::from(pct)), vec![]),
        (pack_aura(0, 25), vec![format!("{e}.radius")]),
        (pack_aura(max + 1, 25), vec![format!("{e}.radius")]),
        (pack_aura(1, 0), vec![format!("{e}.damage_percent")]),
        (
            pack_aura(1, u32::from(pct) + 1),
            vec![format!("{e}.damage_percent")],
        ),
        (
            pack_aura(0, 0),
            vec![format!("{e}.radius"), format!("{e}.damage_percent")],
        ),
    ];
    for (aura, expected) in cases {
        assert_eq!(issues(&with_elite(aura.clone(), None)), expected, "{aura}");
    }
}

#[test]
fn the_escort_is_bounded() {
    let (max, most) = (limits::MAX_LEN, limits::MAX_SHRINE_GUARDIANS);
    let e = "$.waves[0].enemy.elite.escort";
    let cases: [(Value, Vec<String>); 6] = [
        (escort_of(1, 1), vec![]),
        (escort_of(most, max), vec![]),
        (escort_of(0, 1), vec![format!("{e}.count")]),
        (escort_of(most + 1, 1), vec![format!("{e}.count")]),
        (escort_of(1, 0), vec![format!("{e}.radius")]),
        (escort_of(1, max + 1), vec![format!("{e}.radius")]),
    ];
    for (escort, expected) in cases {
        assert_eq!(
            issues(&with_elite(pack_aura(1, 1), Some(escort.clone()))),
            expected,
            "{escort}"
        );
    }
}

#[test]
fn an_escort_member_is_an_ordinary_enemy() {
    let e = "$.waves[0].enemy.elite.escort.enemy";
    let escort = |member: Value| json!({ "enemy": member, "count": 1, "radius": 1 });
    let plain = || enemy(5, json!({ "type": "follow", "distance": 0 }), json!([]));
    let mut elite = plain();
    elite["elite"] = json!({ "range": 256 });
    assert_eq!(
        issues(&with_elite(pack_aura(1, 1), Some(escort(elite)))),
        [format!("{e}.elite")]
    );
    let nested = enemy(
        5,
        json!({ "type": "shrine", "fight_ticks": 1, "guardians": escort_of(1, 1) }),
        json!([]),
    );
    assert_eq!(
        issues(&with_elite(pack_aura(1, 1), Some(escort(nested)))),
        [format!("{e}.movement")]
    );
    let summoner = enemy(
        5,
        json!({ "type": "follow", "distance": 0 }),
        json!([rule(always(), json!("player"), summon(2), 0)]),
    );
    assert_eq!(
        issues(&with_elite(pack_aura(1, 1), Some(escort(summoner)))),
        [format!("{e}.rules[0].do")]
    );
    let mut bad = plain();
    bad["hp"] = json!(0);
    assert_eq!(
        issues(&with_elite(pack_aura(1, 1), Some(escort(bad)))),
        [format!("{e}.hp")]
    );
}

#[test]
fn an_elite_without_an_aura_or_an_escort_is_unchanged() {
    let mut plain = enemy(5, hold(1000, 1000), json!([]));
    plain["elite"] = json!({ "range": 256 });
    let st = stage(vec![wave(1, 1, 0, plain, 1000, 1000)]);
    assert_eq!(issues(&st), Vec::<String>::new());
    let back = serde_json::to_value(&st.waves[0].enemy.elite).unwrap();
    assert_eq!(back, json!({ "range": 256 }), "nothing extra is written");
}
