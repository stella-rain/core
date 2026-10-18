//! The runes file (ADR-099 points 2 and 7, ADR-067): each on-hit rune's numbers, read from
//! JSON as untrusted bytes (ADR-020). Every on-hit rune of the closed list has its entry, once:
//! a cooldown of 1 to `MAX_RUNE_COOLDOWN_TICKS` ticks, a base damage of 1 to `MAX_DAMAGE`, and
//! its attack, at most `MAX_RUNE_PHASES` phases whose hit boxes land around the hit point,
//! over before the cooldown is, spawning no bullets. Explosion and thunder are the two. The bundled file is the default; other
//! bytes change the run's content hash.

use serde_json::{Value, json};
use stella_rain_core::attack::{AttackPhase, DamageType, HitBox, PhaseKind};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    RunRecorder, RunReplayError, content_sha256, content_sha256_with, verify_run, verify_run_with,
};
use stella_rain_core::rune::Rune;
use stella_rain_core::runes::{MAX_RUNE_COOLDOWN_TICKS, MAX_RUNE_PHASES, MAX_RUNES_BYTES, Runes};
use stella_rain_core::validate::Error;
use stella_rain_core::validate::limits::MAX_DAMAGE;

mod common;

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

/// A file of explosion's entry `explosion` and a valid thunder.
fn with_thunder(explosion: Value) -> Value {
    file(vec![explosion, thunder()])
}

fn file(on_hit: Vec<Value>) -> Value {
    json!({ "schema_version": 0, "sim_version": 0, "on_hit": on_hit })
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
    // Placeholder numbers: Kade tunes them.
    let explosion = Runes::bundled()
        .on_hit(Rune::Explosion)
        .expect("explosion's entry");
    assert_eq!(explosion.cooldown_ticks, 120);
    assert_eq!(explosion.damage, 15);
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
    assert_eq!(thunder.cooldown_ticks, 90);
    assert_eq!(thunder.damage, 20);
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
fn the_attack_fires_no_bullets_sets_no_damage_and_never_slows_the_main_character() {
    let mut spawning = active(4);
    spawning["spawn"] = json!([
        { "type": "fire", "speed": 512, "direction": { "type": "aimed", "offset": 0 } }
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
        paths(parse(&file(vec![thunder()])).unwrap_err()),
        ["$.on_hit"],
        "explosion missing"
    );
    assert_eq!(
        paths(parse(&file(vec![one.clone(), thunder(), one])).unwrap_err()),
        ["$.on_hit[2].rune"]
    );
    assert_eq!(
        paths(parse(&file(vec![thunder(), one, thunder()])).unwrap_err()),
        ["$.on_hit[2].rune"]
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
