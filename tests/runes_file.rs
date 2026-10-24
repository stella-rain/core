//! The runes file (ADR-099 points 2 and 7, ADR-067): each on-hit rune's numbers, read from
//! JSON as untrusted bytes (ADR-020). Every on-hit rune of the closed list has its entry, once:
//! a cooldown of 1 to `MAX_RUNE_COOLDOWN_TICKS` ticks, a base damage of 1 to `MAX_DAMAGE`, and
//! its attack, at most `MAX_RUNE_PHASES` phases whose hit boxes land around the hit point,
//! over before the cooldown is, spawning no orbs. Explosion, thunder, boomerang and homing
//! missiles are the four; boomerang's attack is its throw, out and back, two active phases;
//! homing missiles' is one missile's flight, one active phase, and only its entry has the
//! missiles it launches, 1 to `MAX_HOMING_MISSILES`, and the range it picks targets within, 1
//! to `MAX_ASSIST_RANGE`. Every damage-over-time rune has its entry too, once: bleed, poison and
//! burn, with a tick interval of 1 to `MAX_DOT_INTERVAL_TICKS` and no longer than its duration,
//! a duration of 1 to `MAX_DOT_DURATION_TICKS`, a damage a stack of 1 to `MAX_DAMAGE`, a stack
//! maximum of 1 to `MAX_DOT_STACKS`, and a damage type or none. The bundled file is the
//! default; other bytes change the run's content hash.

use serde_json::{Value, json};
use stella_rain_core::attack::{AttackPhase, DamageType, HitBox, PhaseKind};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    RunRecorder, RunReplayError, content_sha256, content_sha256_with, verify_run, verify_run_with,
};
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::{
    MAX_DOT_DURATION_TICKS, MAX_DOT_INTERVAL_TICKS, MAX_DOT_STACKS, MAX_HOMING_MISSILES,
    MAX_RUNE_COOLDOWN_TICKS, MAX_RUNE_PHASES, MAX_RUNES_BYTES, Runes,
};
use stella_rain_core::validate::Error;
use stella_rain_core::validate::limits::{MAX_ASSIST_RANGE, MAX_DAMAGE};

use crate::common;

use common::run::{run_script, trial_setup};

fn active(ticks: u32) -> Value {
    json!({ "kind": "active", "ticks": ticks, "type": "pyro",
            "hit_box": { "type": "circle", "radius": 4_096 } })
}

fn explosion(cooldown: Value, damage: Value, attack: Value) -> Value {
    json!({ "rune": "explosion", "cooldown_ticks": cooldown, "damage": damage,
            "attack": attack })
}

/// A valid thunder entry, after explosion's in every file whose explosion is under test.
fn thunder() -> Value {
    json!({ "rune": "thunder", "cooldown_ticks": 60, "damage": 5,
            "attack": [{ "kind": "wind_up", "ticks": 4 }, active(2)] })
}

/// A valid boomerang entry: its throw out and back, after thunder's.
fn boomerang() -> Value {
    boomerang_of(json!([active(4), active(4)]))
}

fn boomerang_of(attack: Value) -> Value {
    json!({ "rune": "boomerang", "cooldown_ticks": 60, "damage": 5, "attack": attack })
}

/// A valid homing missiles entry: two missiles of one active phase, after boomerang's.
fn homing() -> Value {
    homing_of(json!([active(4)]))
}

fn homing_of(attack: Value) -> Value {
    json!({ "rune": "homing_missiles", "cooldown_ticks": 60, "damage": 5, "missiles": 2,
            "range": 40_960, "attack": attack })
}

/// A file of explosion's entry `explosion`, and a valid thunder, boomerang and homing missiles.
fn with_thunder(explosion: Value) -> Value {
    file(vec![explosion, thunder(), boomerang(), homing()])
}

/// A valid bleed entry.
fn bleed() -> Value {
    bleed_of(json!(30), json!(180), json!(3), json!(5))
}

fn bleed_of(interval: Value, duration: Value, damage: Value, max_stacks: Value) -> Value {
    json!({ "rune": "bleed", "interval_ticks": interval, "duration_ticks": duration,
            "damage": damage, "max_stacks": max_stacks, "type": "slash" })
}

/// A valid poison entry: Dendro.
fn poison() -> Value {
    json!({ "rune": "poison", "interval_ticks": 30, "duration_ticks": 180, "damage": 3,
            "max_stacks": 5, "type": "dendro" })
}

/// A valid burn entry: Pyro.
fn burn() -> Value {
    json!({ "rune": "burn", "interval_ticks": 30, "duration_ticks": 180, "damage": 3,
            "max_stacks": 5, "type": "pyro" })
}

/// A file of the on-hit entries `on_hit` and a valid bleed, poison and burn.
fn file(on_hit: Vec<Value>) -> Value {
    file_with(on_hit, vec![bleed(), poison(), burn()])
}

fn file_with(on_hit: Vec<Value>, damage_over_time: Vec<Value>) -> Value {
    json!({ "schema_version": 0, "sim_version": 0, "on_hit": on_hit,
            "damage_over_time": damage_over_time })
}

/// Valid on-hit entries, and `damage_over_time` followed by a valid poison and burn.
fn with_bleed(mut damage_over_time: Vec<Value>) -> Value {
    damage_over_time.extend([poison(), burn()]);
    let one = explosion(json!(30), json!(5), json!([active(4)]));
    file_with(
        vec![one, thunder(), boomerang(), homing()],
        damage_over_time,
    )
}

fn plain() -> Value {
    with_thunder(explosion(json!(30), json!(5), json!([active(4)])))
}

fn parse(v: &Value) -> Result<Runes, Error> {
    Runes::parse(v.to_string().as_bytes())
}

fn paths(e: Error) -> Vec<String> {
    match e {
        Error::Invalid(issues) => issues.into_iter().map(|i| i.path).collect(),
        other => panic!("not a validation error: {other:?}"),
    }
}

#[test]
fn the_bundled_explosion_is_a_pyro_circle_on_its_own_cooldown() {
    // The numbers of app#298 ("C spiky").
    let explosion = Runes::bundled()
        .on_hit(Rune::Explosion)
        .expect("explosion's entry");
    assert_eq!(explosion.cooldown_ticks, 90);
    assert_eq!(explosion.damage, 2_500);
    assert_eq!(explosion.attack.len(), 1);
    assert_eq!(explosion.attack[0].damage_type, Some(DamageType::Pyro));
    assert!(explosion.attack[0].hit_box.is_some());
}

#[test]
fn the_bundled_thunder_is_an_electro_strike_after_a_wind_up_on_its_own_cooldown() {
    // PLACEHOLDER shape and numbers, Kade's to set: the game design document gives thunder no
    // shape beyond on-hit and Electro.
    let thunder = Runes::bundled()
        .on_hit(Rune::Thunder)
        .expect("thunder's entry");
    assert_eq!(thunder.cooldown_ticks, 60);
    assert_eq!(thunder.damage, 3_000);
    let kinds: Vec<PhaseKind> = thunder.attack.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, [PhaseKind::WindUp, PhaseKind::Active]);
    assert_eq!(thunder.attack[0].ticks, 12);
    assert_eq!(thunder.attack[1].damage_type, Some(DamageType::Electro));
    let explosion = Runes::bundled().on_hit(Rune::Explosion).unwrap();
    let radius = |p: &AttackPhase| match p.hit_box {
        Some(HitBox::Circle { radius, .. }) => radius,
        ref other => panic!("{other:?}"),
    };
    assert!(
        radius(&thunder.attack[1]) < radius(&explosion.attack[0]),
        "narrower than explosion"
    );
}

#[test]
fn the_bundled_boomerang_is_a_neutral_throw_out_and_back_on_its_own_cooldown() {
    // PLACEHOLDER numbers, Kade's to set: the game design document gives boomerang no shape
    // beyond on-hit and neutral.
    let boomerang = Runes::bundled()
        .on_hit(Rune::Boomerang)
        .expect("boomerang's entry");
    assert_eq!(boomerang.cooldown_ticks, 120);
    assert_eq!(boomerang.damage, 1_800);
    let kinds: Vec<PhaseKind> = boomerang.attack.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [PhaseKind::Active, PhaseKind::Active],
        "out, then back"
    );
    assert_eq!(
        boomerang.attack.iter().map(|p| p.ticks).collect::<Vec<_>>(),
        [15, 15]
    );
    for p in &boomerang.attack {
        assert_eq!(p.damage_type, None, "neutral");
        assert!(p.hit_box.is_some());
    }
}

#[test]
fn boomerang_s_attack_is_its_throw_two_active_phases() {
    let with = |attack: Value| {
        file(vec![
            explosion(json!(30), json!(5), json!([active(4)])),
            thunder(),
            boomerang_of(attack),
            homing(),
        ])
    };
    assert!(parse(&with(json!([active(2), active(1)]))).is_ok());
    let at = |v: Value| paths(parse(&v).unwrap_err());
    assert_eq!(
        at(with(json!([active(4)]))),
        ["$.on_hit[2].attack"],
        "no way back"
    );
    assert_eq!(
        at(with(json!([active(4), active(4), active(4)]))),
        ["$.on_hit[2].attack"],
        "a third leg"
    );
    let wind_up = json!({ "kind": "wind_up", "ticks": 4,
                          "hit_box": { "type": "circle", "radius": 4_096 } });
    assert_eq!(
        at(with(json!([wind_up, active(4)]))),
        ["$.on_hit[2].attack[0].kind"]
    );
    let recovery = json!({ "kind": "recovery", "ticks": 4 });
    assert_eq!(
        at(with(json!([active(4), recovery]))),
        ["$.on_hit[2].attack[1].kind"]
    );
    // Each leg has its hit box.
    assert_eq!(
        at(with(json!([active(4), { "kind": "active", "ticks": 4 }]))),
        ["$.on_hit[2].attack[1].hit_box"]
    );
    // It is checked as the others are: orbs, its first leg's ticks, the cooldown.
    let mut spawning = active(4);
    spawning["spawn"] = json!([
        { "type": "zone", "shape": { "type": "circle", "radius": 3_072 }, "anchor": "target", "wind_up_ticks": 0, "active_ticks": 2 }
    ]);
    assert_eq!(
        at(with(json!([active(4), spawning]))),
        ["$.on_hit[2].attack[1].spawn"]
    );
    assert_eq!(
        at(with(json!([active(1), active(4)]))),
        ["$.on_hit[2].attack[0].ticks"]
    );
    let mut slow = boomerang_of(json!([active(40), active(40)]));
    slow["cooldown_ticks"] = json!(79);
    let v = file(vec![
        explosion(json!(30), json!(5), json!([active(4)])),
        thunder(),
        slow,
        homing(),
    ]);
    assert_eq!(
        at(v),
        ["$.on_hit[2].attack"],
        "80 ticks outlast the cooldown"
    );
}

#[test]
fn the_bundled_homing_missiles_are_neutral_missiles_on_their_own_cooldown() {
    // PLACEHOLDER numbers, Kade's to set: the game design document gives homing missiles no
    // shape beyond on-hit and neutral.
    let homing = Runes::bundled()
        .on_hit(Rune::HomingMissiles)
        .expect("homing missiles' entry");
    assert_eq!(homing.cooldown_ticks, 150);
    assert_eq!(homing.damage, 1_000);
    assert_eq!(homing.missiles, Some(4));
    assert_eq!(
        homing.range.map(|r| r.0),
        Some(40_960),
        "aim assist's Sword range"
    );
    assert_eq!(homing.attack.len(), 1, "one flight");
    let flight = &homing.attack[0];
    assert_eq!((flight.kind, flight.ticks), (PhaseKind::Active, 30));
    assert_eq!(flight.damage_type, None, "neutral");
    assert!(matches!(flight.hit_box, Some(HitBox::Circle { .. })));
    for other in [Rune::Explosion, Rune::Thunder, Rune::Boomerang] {
        let entry = Runes::bundled().on_hit(other).unwrap();
        assert_eq!((entry.missiles, entry.range), (None, None), "{other:?}");
    }
}

#[test]
fn homing_missiles_launch_1_to_the_hard_cap_within_a_bounded_range() {
    let with = |field: &str, value: Value| {
        let mut v = plain();
        v["on_hit"][3][field] = value;
        v
    };
    let at = |v: Value| paths(parse(&v).unwrap_err());
    assert_eq!(MAX_HOMING_MISSILES, 4);
    assert!(parse(&with("missiles", json!(1))).is_ok());
    assert!(parse(&with("missiles", json!(MAX_HOMING_MISSILES))).is_ok());
    assert_eq!(at(with("missiles", json!(0))), ["$.on_hit[3].missiles"]);
    assert_eq!(
        at(with("missiles", json!(MAX_HOMING_MISSILES + 1))),
        ["$.on_hit[3].missiles"]
    );
    assert!(parse(&with("range", json!(1))).is_ok());
    assert!(parse(&with("range", json!(MAX_ASSIST_RANGE))).is_ok());
    assert_eq!(at(with("range", json!(0))), ["$.on_hit[3].range"]);
    assert_eq!(
        at(with("range", json!(MAX_ASSIST_RANGE + 1))),
        ["$.on_hit[3].range"]
    );
    // Both are homing missiles' own: required on it, refused on any other rune.
    for field in ["missiles", "range"] {
        let mut v = plain();
        v["on_hit"][3].as_object_mut().unwrap().remove(field);
        assert_eq!(at(v), [format!("$.on_hit[3].{field}")]);
    }
    let mut v = plain();
    v["on_hit"][0]["missiles"] = json!(2);
    v["on_hit"][2]["range"] = json!(4_096);
    assert_eq!(at(v), ["$.on_hit[0].missiles", "$.on_hit[2].range"]);
}

#[test]
fn homing_missiles_attack_is_one_flight_an_active_phase_with_a_hit_box() {
    let with = |attack: Value| {
        file(vec![
            explosion(json!(30), json!(5), json!([active(4)])),
            thunder(),
            boomerang(),
            homing_of(attack),
        ])
    };
    let at = |v: Value| paths(parse(&v).unwrap_err());
    assert_eq!(
        at(with(json!([active(4), active(4)]))),
        ["$.on_hit[3].attack"],
        "one flight"
    );
    let wind_up = json!({ "kind": "wind_up", "ticks": 4,
                          "hit_box": { "type": "circle", "radius": 4_096 } });
    assert_eq!(at(with(json!([wind_up]))), ["$.on_hit[3].attack[0].kind"]);
    assert_eq!(
        at(with(json!([{ "kind": "active", "ticks": 4 }]))),
        ["$.on_hit[3].attack[0].hit_box"]
    );
    // It is checked as the others are: orbs, its first phase's ticks, the cooldown.
    let mut spawning = active(4);
    spawning["spawn"] = json!([
        { "type": "zone", "shape": { "type": "circle", "radius": 3_072 }, "anchor": "target", "wind_up_ticks": 0, "active_ticks": 2 }
    ]);
    assert_eq!(at(with(json!([spawning]))), ["$.on_hit[3].attack[0].spawn"]);
    assert_eq!(
        at(with(json!([active(1)]))),
        ["$.on_hit[3].attack[0].ticks"]
    );
    let mut slow = homing_of(json!([active(61)]));
    slow["cooldown_ticks"] = json!(60);
    let v = file(vec![
        explosion(json!(30), json!(5), json!([active(4)])),
        thunder(),
        boomerang(),
        slow,
    ]);
    assert_eq!(
        at(v),
        ["$.on_hit[3].attack"],
        "61 ticks outlast the cooldown"
    );
}

#[test]
fn the_bundled_bleed_is_slash_damage_over_time_in_stacks() {
    // The numbers of app#298 ("C spiky"): 3 damage a stack (300 after the scale of ADR-103)
    // every 20 ticks for 180 ticks, up to 6 stacks, Slash.
    let bleed = Runes::bundled()
        .damage_over_time(Rune::Bleed)
        .expect("bleed's entry");
    assert_eq!(
        (
            bleed.interval_ticks,
            bleed.duration_ticks,
            bleed.damage,
            bleed.max_stacks
        ),
        (20, 180, 300, 6)
    );
    assert_eq!(bleed.damage_type, Some(DamageType::Slash));
    assert!(Runes::bundled().on_hit(Rune::Bleed).is_none());
    assert!(Runes::bundled().damage_over_time(Rune::Explosion).is_none());
}

#[test]
fn the_bundled_poison_is_dendro_and_burn_pyro_damage_over_time_in_stacks() {
    // Bleed's numbers: 300 damage a stack every 20 ticks for 180 ticks, up to 6 stacks; only
    // the types differ.
    for (rune, ty) in [
        (Rune::Poison, DamageType::Dendro),
        (Rune::Burn, DamageType::Pyro),
    ] {
        let entry = Runes::bundled()
            .damage_over_time(rune)
            .unwrap_or_else(|| panic!("{rune:?}'s entry"));
        assert_eq!(
            (
                entry.interval_ticks,
                entry.duration_ticks,
                entry.damage,
                entry.max_stacks
            ),
            (20, 180, 300, 6),
            "{rune:?}"
        );
        assert_eq!(entry.damage_type, Some(ty), "{rune:?}");
        assert!(Runes::bundled().on_hit(rune).is_none());
    }
}

#[test]
fn poison_and_burn_numbers_are_held_to_bleed_s_caps() {
    for name in ["poison", "burn"] {
        let entry = |interval: u32, duration: u32, damage: u32, max: u32| {
            let mut v = if name == "poison" { poison() } else { burn() };
            v["interval_ticks"] = json!(interval);
            v["duration_ticks"] = json!(duration);
            v["damage"] = json!(damage);
            v["max_stacks"] = json!(max);
            let mut file = plain();
            let other = if name == "poison" { burn() } else { poison() };
            file["damage_over_time"] = json!(if name == "poison" {
                [bleed(), v, other]
            } else {
                [bleed(), other, v]
            });
            parse(&file).map(|_| ()).map_err(paths)
        };
        let at = |field: &str| {
            let i = if name == "poison" { 1 } else { 2 };
            vec![format!("$.damage_over_time[{i}].{field}")]
        };
        assert_eq!(entry(1, 1, 1, 1), Ok(()), "{name}");
        assert_eq!(
            entry(
                MAX_DOT_INTERVAL_TICKS,
                MAX_DOT_DURATION_TICKS,
                MAX_DAMAGE,
                MAX_DOT_STACKS
            ),
            Ok(()),
            "{name}"
        );
        assert_eq!(entry(0, 180, 3, 5), Err(at("interval_ticks")), "{name}");
        assert_eq!(entry(181, 180, 3, 5), Err(at("interval_ticks")), "{name}");
        assert_eq!(entry(30, 0, 3, 5), Err(at("duration_ticks")), "{name}");
        assert_eq!(
            entry(30, MAX_DOT_DURATION_TICKS + 1, 3, 5),
            Err(at("duration_ticks")),
            "{name}"
        );
        assert_eq!(entry(30, 180, 0, 5), Err(at("damage")), "{name}");
        assert_eq!(
            entry(30, 180, MAX_DAMAGE + 1, 5),
            Err(at("damage")),
            "{name}"
        );
        assert_eq!(entry(30, 180, 3, 0), Err(at("max_stacks")), "{name}");
        assert_eq!(
            entry(30, 180, 3, MAX_DOT_STACKS + 1),
            Err(at("max_stacks")),
            "{name}"
        );
    }
}

#[test]
fn poison_and_burn_each_need_their_entry_once_and_are_not_on_hit_runes() {
    let mut missing = plain();
    missing["damage_over_time"] = json!([bleed(), burn()]);
    assert_eq!(paths(parse(&missing).unwrap_err()), ["$.damage_over_time"]);
    let mut twice = plain();
    twice["damage_over_time"] = json!([bleed(), poison(), burn(), burn()]);
    assert_eq!(
        paths(parse(&twice).unwrap_err()),
        ["$.damage_over_time[3].rune"]
    );
    for name in ["poison", "burn"] {
        let mut v = plain();
        let mut as_on_hit = thunder();
        as_on_hit["rune"] = json!(name);
        v["on_hit"].as_array_mut().unwrap().push(as_on_hit);
        assert_eq!(
            paths(parse(&v).unwrap_err()),
            ["$.on_hit[4].rune"],
            "{name}"
        );
    }
}

#[test]
fn bleed_s_numbers_are_1_to_their_hard_caps() {
    let at = |field: &str| format!("$.damage_over_time[0].{field}");
    let check = |interval: u32, duration: u32, damage: u32, max: u32| {
        parse(&with_bleed(vec![bleed_of(
            json!(interval),
            json!(duration),
            json!(damage),
            json!(max),
        )]))
        .map(|_| ())
        .map_err(paths)
    };
    assert_eq!(check(1, 1, 1, 1), Ok(()));
    assert_eq!(
        check(
            MAX_DOT_INTERVAL_TICKS,
            MAX_DOT_DURATION_TICKS,
            MAX_DAMAGE,
            MAX_DOT_STACKS
        ),
        Ok(())
    );
    assert_eq!(check(0, 180, 3, 5), Err(vec![at("interval_ticks")]));
    assert_eq!(
        check(MAX_DOT_INTERVAL_TICKS + 1, MAX_DOT_DURATION_TICKS, 3, 5),
        Err(vec![at("interval_ticks")])
    );
    assert_eq!(check(30, 0, 3, 5), Err(vec![at("duration_ticks")]));
    assert_eq!(
        check(30, MAX_DOT_DURATION_TICKS + 1, 3, 5),
        Err(vec![at("duration_ticks")])
    );
    assert_eq!(check(30, 180, 0, 5), Err(vec![at("damage")]));
    assert_eq!(check(30, 180, MAX_DAMAGE + 1, 5), Err(vec![at("damage")]));
    assert_eq!(check(30, 180, 3, 0), Err(vec![at("max_stacks")]));
    assert_eq!(
        check(30, 180, 3, MAX_DOT_STACKS + 1),
        Err(vec![at("max_stacks")])
    );
}

#[test]
fn bleed_ticks_at_least_once_in_its_duration() {
    let check = |interval: u32, duration: u32| {
        parse(&with_bleed(vec![bleed_of(
            json!(interval),
            json!(duration),
            json!(3),
            json!(5),
        )]))
        .map(|_| ())
        .map_err(paths)
    };
    assert_eq!(check(30, 30), Ok(()));
    assert_eq!(
        check(31, 30),
        Err(vec!["$.damage_over_time[0].interval_ticks".to_owned()])
    );
}

#[test]
fn bleed_may_be_neutral() {
    let mut neutral = bleed();
    neutral.as_object_mut().unwrap().remove("type");
    let runes = parse(&with_bleed(vec![neutral])).unwrap();
    assert_eq!(
        runes.damage_over_time(Rune::Bleed).unwrap().damage_type,
        None
    );
}

#[test]
fn every_damage_over_time_rune_has_one_entry_and_only_those() {
    assert_eq!(
        paths(parse(&with_bleed(vec![])).unwrap_err()),
        ["$.damage_over_time"]
    );
    assert_eq!(
        paths(parse(&with_bleed(vec![bleed(), bleed()])).unwrap_err()),
        ["$.damage_over_time[1].rune"]
    );
    let mut on_hit = bleed();
    on_hit["rune"] = json!("explosion");
    assert_eq!(
        paths(parse(&with_bleed(vec![bleed(), on_hit])).unwrap_err()),
        ["$.damage_over_time[1].rune"]
    );
    // Bleed among the on-hit runes: a valid entry but for its rune.
    let mut v = plain();
    let mut as_on_hit = thunder();
    as_on_hit["rune"] = json!("bleed");
    v["on_hit"].as_array_mut().unwrap().push(as_on_hit);
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.on_hit[4].rune"]);
    let mut missing = plain();
    missing.as_object_mut().unwrap().remove("damage_over_time");
    assert!(matches!(parse(&missing), Err(Error::Parse { .. })));
    let mut extra = plain();
    extra["damage_over_time"][0]["extra"] = json!(1);
    assert!(matches!(parse(&extra), Err(Error::Parse { .. })));
}

#[test]
fn a_file_parses_with_its_numbers() {
    let runes = parse(&plain()).unwrap();
    let explosion = runes.on_hit(Rune::Explosion).unwrap();
    assert_eq!((explosion.cooldown_ticks, explosion.damage), (30, 5));
    let thunder = runes.on_hit(Rune::Thunder).unwrap();
    assert_eq!((thunder.cooldown_ticks, thunder.damage), (60, 5));
}

#[test]
fn thunder_s_entry_is_checked_as_explosion_s() {
    let with = |field: &str, value: Value| {
        let mut v = plain();
        v["on_hit"][1][field] = value;
        v
    };
    assert_eq!(
        paths(parse(&with("cooldown_ticks", json!(0))).unwrap_err()),
        ["$.on_hit[1].cooldown_ticks"]
    );
    assert_eq!(
        paths(parse(&with("cooldown_ticks", json!(5))).unwrap_err()),
        ["$.on_hit[1].attack"],
        "its 6 ticks of attack outlast the cooldown"
    );
    assert_eq!(
        paths(parse(&with("damage", json!(MAX_DAMAGE + 1))).unwrap_err()),
        ["$.on_hit[1].damage"]
    );
    // A wind-up alone never lands.
    let wind_up = json!([{ "kind": "wind_up", "ticks": 4,
                           "hit_box": { "type": "circle", "radius": 4_096 } }]);
    assert_eq!(
        paths(parse(&with("attack", wind_up)).unwrap_err()),
        ["$.on_hit[1].attack"]
    );
}

#[test]
fn the_cooldown_is_1_to_the_hard_cap() {
    let with =
        |cooldown: u32| with_thunder(explosion(json!(cooldown), json!(5), json!([active(2)])));
    assert!(parse(&with(2)).is_ok());
    assert!(parse(&with(MAX_RUNE_COOLDOWN_TICKS)).is_ok());
    assert_eq!(
        paths(parse(&with(0)).unwrap_err()),
        ["$.on_hit[0].cooldown_ticks"]
    );
    assert_eq!(
        paths(parse(&with(MAX_RUNE_COOLDOWN_TICKS + 1)).unwrap_err()),
        ["$.on_hit[0].cooldown_ticks"]
    );
}

#[test]
fn the_damage_is_1_to_the_hard_cap() {
    let with = |damage: u32| with_thunder(explosion(json!(30), json!(damage), json!([active(4)])));
    assert!(parse(&with(1)).is_ok());
    assert!(parse(&with(MAX_DAMAGE)).is_ok());
    assert_eq!(paths(parse(&with(0)).unwrap_err()), ["$.on_hit[0].damage"]);
    assert_eq!(
        paths(parse(&with(MAX_DAMAGE + 1)).unwrap_err()),
        ["$.on_hit[0].damage"]
    );
}

#[test]
fn the_attack_is_over_before_the_cooldown_is() {
    // Two phases of 4 and 6 ticks: 10 in all.
    let attack = json!([active(4), { "kind": "recovery", "ticks": 6 }]);
    let with = |cooldown: u32| with_thunder(explosion(json!(cooldown), json!(5), attack.clone()));
    assert!(parse(&with(10)).is_ok());
    assert_eq!(paths(parse(&with(9)).unwrap_err()), ["$.on_hit[0].attack"]);
}

#[test]
fn the_attack_has_at_most_the_phase_cap() {
    let with = |n: usize| {
        let attack: Vec<Value> = (0..n).map(|_| active(2)).collect();
        with_thunder(explosion(json!(1_000), json!(5), json!(attack)))
    };
    assert!(parse(&with(MAX_RUNE_PHASES)).is_ok());
    assert_eq!(
        paths(parse(&with(MAX_RUNE_PHASES + 1)).unwrap_err()),
        ["$.on_hit[0].attack"]
    );
    assert_eq!(paths(parse(&with(0)).unwrap_err()), ["$.on_hit[0].attack"]);
}

#[test]
fn the_attack_lands_somewhere_and_from_the_tick_after_it_fires() {
    // No hit box at all.
    let none = json!([{ "kind": "active", "ticks": 4 }]);
    let v = with_thunder(explosion(json!(30), json!(5), none));
    assert_eq!(paths(parse(&v).unwrap_err()), ["$.on_hit[0].attack"]);
    // Its first phase counts the tick it fires on, after that tick's hits: one tick of it
    // would never land.
    let v = with_thunder(explosion(json!(30), json!(5), json!([active(1)])));
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.on_hit[0].attack[0].ticks"]
    );
}

#[test]
fn the_attack_fires_no_orbs_sets_no_damage_and_never_slows_the_main_character() {
    let mut spawning = active(4);
    spawning["spawn"] = json!([
        { "type": "zone", "shape": { "type": "circle", "radius": 3_072 }, "anchor": "target", "wind_up_ticks": 0, "active_ticks": 2 }
    ]);
    let mut own_damage = active(4);
    own_damage["damage"] = json!(9);
    let mut rooting = active(4);
    rooting["move_pct"] = json!(0);
    for (phase, field) in [
        (spawning, "spawn"),
        (own_damage, "damage"),
        (rooting, "move_pct"),
    ] {
        let v = with_thunder(explosion(json!(30), json!(5), json!([phase])));
        assert_eq!(
            paths(parse(&v).unwrap_err()),
            [format!("$.on_hit[0].attack[0].{field}")]
        );
    }
}

#[test]
fn the_attack_is_checked_as_any_attack() {
    let bad = json!([{ "kind": "active", "ticks": 4,
                       "hit_box": { "type": "circle", "radius": 0 } }]);
    let v = with_thunder(explosion(json!(30), json!(5), bad));
    assert_eq!(
        paths(parse(&v).unwrap_err()),
        ["$.on_hit[0].attack[0].hit_box.radius"]
    );
}

#[test]
fn every_on_hit_rune_has_one_entry() {
    assert_eq!(paths(parse(&file(vec![])).unwrap_err()), ["$.on_hit"]);
    let one = explosion(json!(30), json!(5), json!([active(4)]));
    assert_eq!(
        paths(parse(&file(vec![one.clone()])).unwrap_err()),
        ["$.on_hit"],
        "thunder missing"
    );
    assert_eq!(
        paths(parse(&file(vec![thunder(), boomerang(), homing()])).unwrap_err()),
        ["$.on_hit"],
        "explosion missing"
    );
    assert_eq!(
        paths(parse(&file(vec![one.clone(), thunder(), homing()])).unwrap_err()),
        ["$.on_hit"],
        "boomerang missing"
    );
    assert_eq!(
        paths(parse(&file(vec![one.clone(), thunder(), boomerang()])).unwrap_err()),
        ["$.on_hit"],
        "homing missiles missing"
    );
    assert_eq!(
        paths(
            parse(&file(vec![
                one.clone(),
                thunder(),
                one.clone(),
                boomerang(),
                homing()
            ]))
            .unwrap_err()
        ),
        ["$.on_hit[2].rune"]
    );
    assert_eq!(
        paths(
            parse(&file(vec![
                thunder(),
                one.clone(),
                homing(),
                boomerang(),
                homing()
            ]))
            .unwrap_err()
        ),
        ["$.on_hit[4].rune"]
    );
    assert_eq!(
        paths(parse(&file(vec![boomerang(), boomerang(), thunder(), homing()])).unwrap_err()),
        ["$.on_hit[1].rune", "$.on_hit"]
    );
    let mut unknown = plain();
    unknown["on_hit"][1]["rune"] = json!("lightning");
    assert!(matches!(parse(&unknown), Err(Error::Parse { .. })));
}

#[test]
fn the_file_is_strict_and_size_limited() {
    let mut v = plain();
    v["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut v = plain();
    v["on_hit"][0]["extra"] = json!(1);
    assert!(matches!(parse(&v), Err(Error::Parse { .. })));
    let mut newer = plain();
    newer["sim_version"] = json!(1);
    assert!(matches!(
        parse(&newer),
        Err(Error::UnsupportedVersion { .. })
    ));
    let mut big = Runes::BUNDLED.to_vec();
    big.resize(MAX_RUNES_BYTES + 1, b' ');
    assert!(matches!(Runes::parse(&big), Err(Error::TooLarge { .. })));
}

fn other() -> Runes {
    parse(&plain()).unwrap()
}

#[test]
fn the_content_hash_covers_the_runes() {
    let content = RunContent::bundled().clone().with_runes(other());
    assert_eq!(content.runes(), &other());
    assert_eq!(RunContent::bundled().runes(), Runes::bundled());
    assert_ne!(
        content_sha256_with("trial", &content),
        content_sha256("trial")
    );
}

#[test]
fn a_run_with_other_runes_verifies_only_with_them() {
    let content = RunContent::bundled().clone().with_runes(other());
    let mut recorder = RunRecorder::start_with(&trial_setup(), &content).unwrap();
    for t in 0..50 {
        recorder.step(run_script(t)).unwrap();
    }
    let bytes = recorder.replay().to_bytes();
    assert!(verify_run_with(&bytes, &content).is_ok());
    assert_eq!(verify_run(&bytes), Err(RunReplayError::ContentChanged));
}
