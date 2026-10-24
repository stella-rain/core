//! Damage types and match-ups (ADR-100): an attack phase may have one damage type, which its
//! hit box and its orbs carry; enemies, elites, bosses and boss parts may have a match-up
//! profile. Each hit gets exactly one match-up, weak (+20), strong (−20), resisted (−20) or
//! none, applied as a target's multiplier after vulnerability and before defence (ADR-056).

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::MatchUp;
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::replay::{record, verify};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate;

const CX: i32 = 20_000;
const CY: i32 = 30_000;
/// The companion is entity 2 (the far boss is 1); the first enemy is 3.
const COMPANION: EntityId = EntityId(2);

/// A one-tick strike of `damage`, typed `ty` (or neutral), over the target next to the striker.
fn strike(damage: u32, ty: Option<&str>) -> Value {
    let mut phase = json!({ "kind": "active", "ticks": 1, "damage": damage,
                            "hit_box": { "type": "circle", "radius": 4_096 } });
    if let Some(ty) = ty {
        phase["type"] = json!(ty);
    }
    json!({ "type": "attack", "attack": { "inline": [phase] } })
}

/// One zone of `damage`, typed `ty`, on the target.
fn shoot(damage: u32, ty: &str) -> Value {
    json!({ "type": "attack", "attack": { "inline": [
        { "kind": "active", "ticks": 1, "damage": damage, "type": ty, "spawn": [
            { "type": "zone", "shape": { "type": "circle", "radius": 3_072 }, "anchor": "target", "wind_up_ticks": 0, "active_ticks": 2 }
        ] }] } })
}

fn once(action: Value) -> Value {
    json!([{ "when": always(), "target": "nearest_enemy", "once": true, "do": action }])
}

/// An enemy with `profile` (and `defence`), holding still next to the corner.
fn target(profile: Option<Value>, defence: u32) -> Value {
    let mut e = enemy(100_000, hold(CX + 2_560, CY), json!([]));
    if let Some(p) = profile {
        e["profile"] = p;
    }
    if defence > 0 {
        e["defence"] = json!(defence);
    }
    wave(1, 1, 0, e, CX + 2_560, CY)
}

/// A stage where a companion at the corner runs `rules` against `foe`.
fn striker(rules: Value, foe: Value) -> Stage {
    stage_full(
        18_000,
        Some(far_boss()),
        vec![foe],
        vec![companion(50, hold(CX, CY), rules)],
    )
}

/// The damage and match-up of every hit `source` deals over `ticks` ticks.
fn hits(stage: &Stage, source: EntityId, ticks: u32) -> Vec<(u32, MatchUp)> {
    validate::validate(stage).expect("the test stage is valid");
    run_until(&mut engine(stage), idle(), ticks, |_| false)
        .into_iter()
        .filter_map(|(_, ev)| match ev {
            DomainEvent::Hit {
                source: s,
                damage,
                match_up,
                ..
            } if s == source => Some((damage, match_up)),
            _ => None,
        })
        .collect()
}

fn against(profile: Value, ty: Option<&str>) -> Vec<(u32, MatchUp)> {
    let st = striker(once(strike(10, ty)), target(Some(profile), 0));
    hits(&st, COMPANION, 3)
}

#[test]
fn a_listed_weakness_adds_20_percent() {
    let profile = json!({ "weak": ["pyro", "slash"] });
    assert_eq!(
        against(profile.clone(), Some("pyro")),
        [(12, MatchUp::Weak)]
    );
    assert_eq!(against(profile, Some("slash")), [(12, MatchUp::Weak)]);
}

#[test]
fn a_listed_strength_takes_20_percent() {
    let profile = json!({ "strong": ["hydro"] });
    assert_eq!(against(profile, Some("hydro")), [(8, MatchUp::Strong)]);
}

#[test]
fn a_blanket_resist_covers_its_group_only() {
    let physical = json!({ "resist": "physical" });
    for ty in ["slash", "pierce", "strike"] {
        assert_eq!(
            against(physical.clone(), Some(ty)),
            [(8, MatchUp::Resisted)]
        );
    }
    assert_eq!(against(physical, Some("cryo")), [(10, MatchUp::None)]);
    let elements = json!({ "resist": "elements" });
    for ty in ["pyro", "hydro", "electro", "cryo", "anemo", "geo", "dendro"] {
        assert_eq!(
            against(elements.clone(), Some(ty)),
            [(8, MatchUp::Resisted)]
        );
    }
    assert_eq!(against(elements, Some("pierce")), [(10, MatchUp::None)]);
}

#[test]
fn a_type_the_profile_does_not_name_is_none() {
    let profile = json!({ "weak": ["slash"], "strong": ["geo"] });
    assert_eq!(against(profile, Some("anemo")), [(10, MatchUp::None)]);
}

#[test]
fn a_listed_type_wins_over_the_blanket() {
    let profile = json!({ "weak": ["pyro"], "strong": ["slash"], "resist": "elements" });
    assert_eq!(
        against(profile.clone(), Some("pyro")),
        [(12, MatchUp::Weak)]
    );
    assert_eq!(
        against(profile.clone(), Some("cryo")),
        [(8, MatchUp::Resisted)]
    );
    let physical = json!({ "weak": ["strike"], "resist": "physical" });
    assert_eq!(
        against(physical.clone(), Some("strike")),
        [(12, MatchUp::Weak)]
    );
    assert_eq!(against(physical, Some("pierce")), [(8, MatchUp::Resisted)]);
}

#[test]
fn a_neutral_attack_never_gets_a_match_up() {
    let profile = json!({ "weak": ["slash", "pierce", "strike"], "resist": "elements" });
    assert_eq!(against(profile, None), [(10, MatchUp::None)]);
}

#[test]
fn no_profile_is_neutral_to_every_type() {
    let st = striker(once(strike(10, Some("electro"))), target(None, 0));
    assert_eq!(hits(&st, COMPANION, 3), [(10, MatchUp::None)]);
}

#[test]
fn the_match_up_comes_after_vulnerability_and_before_defence() {
    // 10 × 150 / 100 = 15, × 120 / 100 = 18, × 10,000 / 20,000 = 9 (K is 10,000, ADR-103).
    let rules = json!([
        { "when": always(), "target": "nearest_enemy", "once": true,
          "do": cast(json!({ "type": "vulnerable", "value_pct": 50, "duration_ticks": 60 })) },
        { "when": always(), "target": "nearest_enemy", "once": true,
          "do": strike(10, Some("pyro")) }
    ]);
    let st = striker(rules, target(Some(json!({ "weak": ["pyro"] })), 10_000));
    assert_eq!(hits(&st, COMPANION, 3), [(9, MatchUp::Weak)]);
    // Truncated toward zero: 7 × 80 / 100 = 5.6, then at least 1 from 1 × 80 / 100.
    let st = striker(
        once(strike(7, Some("geo"))),
        target(Some(json!({ "strong": ["geo"] })), 0),
    );
    assert_eq!(hits(&st, COMPANION, 3), [(5, MatchUp::Strong)]);
    let st = striker(
        once(strike(1, Some("geo"))),
        target(Some(json!({ "strong": ["geo"] })), 0),
    );
    assert_eq!(hits(&st, COMPANION, 3), [(1, MatchUp::Strong)]);
}

#[test]
fn an_orb_carries_its_phases_type() {
    let st = striker(
        once(shoot(10, "pyro")),
        target(Some(json!({ "weak": ["pyro"] })), 0),
    );
    assert_eq!(hits(&st, COMPANION, 20), [(12, MatchUp::Weak)]);
    let st = striker(
        once(shoot(10, "dendro")),
        target(Some(json!({ "resist": "elements" })), 0),
    );
    assert_eq!(hits(&st, COMPANION, 20), [(8, MatchUp::Resisted)]);
}

#[test]
fn a_zones_type_is_part_of_the_state_hash() {
    // The same shot, typed or not, at an enemy with no profile: the hit deals the same, but
    // while the zone warns the states differ by its type.
    let hash_after = |ty: Option<&str>, ticks: u32| {
        let mut shot = shoot(10, "pyro");
        // The zone warns for 20 ticks, in which the states differ by its type.
        shot["attack"]["inline"][0]["spawn"][0]["wind_up_ticks"] = json!(20);
        match ty {
            Some(ty) => shot["attack"]["inline"][0]["type"] = json!(ty),
            None => {
                shot["attack"]["inline"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("type");
            }
        }
        let far = enemy(100_000, hold(CX + 25_600, CY), json!([]));
        let mut e = engine(&striker(once(shot), wave(1, 1, 0, far, CX + 25_600, CY)));
        for _ in 0..ticks {
            e.step(idle());
        }
        e.state_hash()
    };
    assert_ne!(hash_after(Some("pyro"), 5), hash_after(None, 5));
    assert_ne!(hash_after(Some("pyro"), 5), hash_after(Some("cryo"), 5));
    // Once the zone has hurt, nothing typed is left.
    assert_eq!(hash_after(Some("pyro"), 100), hash_after(None, 100));
}

#[test]
fn damage_stays_bounded() {
    // The largest base damage a stage may give, weak: 12 000 dealt, in 64 bits. Past the
    // validator's bounds the arithmetic saturates (`rules_v0::entity`'s own tests).
    let max = validate::limits::MAX_DAMAGE;
    let st = striker(
        once(strike(max, Some("pyro"))),
        target(Some(json!({ "weak": ["pyro"] })), 0),
    );
    assert_eq!(hits(&st, COMPANION, 3), [(max / 100 * 120, MatchUp::Weak)]);
}

#[test]
fn bosses_their_parts_and_elites_have_their_own_profiles() {
    // The boss is weak to pyro, its horn strong against it, under one hit box.
    let boss = json!({ "base": "golem", "hp": 1_000, "radius": 8_192,
                       "profile": { "weak": ["pyro"] },
                       "spawn": { "x": CX, "y": CY + 10_240 },
                       "parts": [{ "id": "horn", "asset": "horns_2", "hp": 100,
                                   "profile": { "strong": ["pyro"] },
                                   "offset": { "x": 0, "y": -4_096 }, "radius": 2_048 }],
                       "phases": [{}] });
    let sweep = json!([{ "when": always(), "target": "boss_first", "once": true,
        "do": { "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10, "type": "pyro",
              "hit_box": { "type": "circle", "radius": 20_480 } }] } } }]);
    let st = stage_full(
        18_000,
        Some(boss),
        vec![],
        vec![companion(50, hold(CX, CY), sweep)],
    );
    assert_eq!(
        hits(&st, COMPANION, 3),
        [(12, MatchUp::Weak), (8, MatchUp::Strong)]
    );

    // An elite uses its own profile.
    let mut elite = enemy(100_000, hold(CX + 2_560, CY), json!([]));
    elite["elite"] = json!({ "range": 25_600 });
    elite["profile"] = json!({ "resist": "physical" });
    let st = striker(
        once(strike(10, Some("slash"))),
        wave(1, 1, 0, elite, CX + 2_560, CY),
    );
    assert_eq!(hits(&st, COMPANION, 3), [(8, MatchUp::Resisted)]);
}

#[test]
fn the_main_character_has_no_profile_so_typed_hits_on_it_are_neutral() {
    let swing = json!([{ "when": always(), "target": "player", "once": true,
        "do": { "type": "attack", "attack": { "inline": [
            { "kind": "active", "ticks": 1, "damage": 10, "type": "pyro",
              "hit_box": { "type": "circle", "offset": { "forward": 5_120 }, "radius": 1_024 } }
        ] } } }]);
    let foe = wave(
        1,
        1,
        0,
        enemy(50, hold(START_X, START_Y - 5_120), swing),
        START_X,
        START_Y - 5_120,
    );
    let st = stage_full(18_000, Some(far_boss()), vec![foe], vec![]);
    let hp_left = run_until(&mut engine(&st), idle(), 3, |_| false)
        .into_iter()
        .find_map(|(_, ev)| match ev {
            DomainEvent::PlayerHit { hp_left } => Some(hp_left),
            _ => None,
        });
    assert_eq!(hp_left, Some(89));
}

// --- The validator -----------------------------------------------------------------------------

fn issues(profile: Value) -> Vec<String> {
    let st = striker(json!([]), target(Some(profile), 0));
    match validate::validate(&st) {
        Ok(()) => vec![],
        Err(validate::Error::Invalid(issues)) => issues.into_iter().map(|i| i.path).collect(),
        Err(e) => panic!("unexpected error {e:?}"),
    }
}

#[test]
fn the_validator_refuses_a_type_twice_in_a_list() {
    assert_eq!(
        issues(json!({ "weak": ["pyro", "geo", "pyro"] })),
        ["$.waves[0].enemy.profile.weak[2]"]
    );
    assert_eq!(
        issues(json!({ "strong": ["slash", "slash"] })),
        ["$.waves[0].enemy.profile.strong[1]"]
    );
}

#[test]
fn the_validator_refuses_a_type_in_both_lists() {
    assert_eq!(
        issues(json!({ "weak": ["pyro", "cryo"], "strong": ["cryo"] })),
        ["$.waves[0].enemy.profile.strong[0]"]
    );
}

#[test]
fn a_profile_has_at_most_one_blanket_resist_and_only_known_types() {
    let parse = |profile: Value| {
        let mut v = serde_json::to_value(striker(json!([]), target(None, 0))).unwrap();
        v["waves"][0]["enemy"]["profile"] = profile;
        serde_json::from_value::<Stage>(v).is_ok()
    };
    assert!(parse(json!({ "resist": "elements" })));
    assert!(!parse(json!({ "resist": ["physical", "elements"] })));
    assert!(!parse(json!({ "resist": "everything" })));
    assert!(!parse(json!({ "weak": ["fire"] })));
    assert!(!parse(json!({ "weak": [], "extra": 1 })));
}

#[test]
fn a_full_profile_is_valid() {
    assert!(
        issues(json!({ "weak": ["pyro", "hydro", "electro"],
                       "strong": ["slash", "pierce", "strike", "cryo"],
                       "resist": "elements" }))
        .is_empty()
    );
}

#[test]
fn a_companion_has_no_profile() {
    let mut ally = companion(50, hold(CX, CY), json!([]));
    ally["profile"] = json!({ "weak": ["pyro"] });
    let st = stage_full(18_000, Some(far_boss()), vec![], vec![ally]);
    let Err(validate::Error::Invalid(found)) = validate::validate(&st) else {
        panic!("the profile is refused");
    };
    let paths: Vec<_> = found.into_iter().map(|i| i.path).collect();
    assert_eq!(paths, ["$.companions[0].profile"]);
}

#[test]
fn the_validator_checks_boss_and_part_profiles() {
    let boss = json!({ "base": "golem", "hp": 1_000, "radius": 8_192,
                       "profile": { "weak": ["geo", "geo"] },
                       "spawn": { "x": CX, "y": CY + 10_240 },
                       "parts": [{ "id": "horn", "asset": "horns_2", "hp": 100,
                                   "profile": { "weak": ["anemo"], "strong": ["anemo"] },
                                   "offset": { "x": 0, "y": -4_096 }, "radius": 2_048 }],
                       "phases": [{}] });
    let st = stage_full(18_000, Some(boss), vec![], vec![]);
    let Err(validate::Error::Invalid(found)) = validate::validate(&st) else {
        panic!("the profiles are refused");
    };
    let paths: Vec<_> = found.into_iter().map(|i| i.path).collect();
    assert_eq!(
        paths,
        [
            "$.boss.profile.weak[1]",
            "$.boss.parts[0].profile.strong[0]"
        ]
    );
}

// --- Replays -----------------------------------------------------------------------------------

/// A stage whose boss, 44 hit points and weak to pyro, waits within reach of the Main weapon's
/// one pyro step (10 damage).
fn typed_stage(weak: bool) -> Vec<u8> {
    let mut v = clearing_stage_json();
    v["boss"]["hp"] = json!(44);
    v["boss"]["spawn"]["y"] = json!(START_Y - 5_120);
    if weak {
        v["boss"]["profile"] = json!({ "weak": ["pyro"], "resist": "physical" });
    }
    v["player"]["weapon"] = weapon_ref(json!({ "inline": [
        { "kind": "active", "ticks": 2, "type": "pyro",
          "hit_box": { "type": "circle", "radius": 8_192 } },
        { "kind": "recovery", "ticks": 2 }] }));
    serde_json::to_vec(&v).unwrap()
}

#[test]
fn a_replay_with_typed_attacks_and_profiles_verifies() {
    let pressing = |t: u32| Input {
        main: t % 2 == 0,
        ..Input::default()
    };
    let inputs: Vec<Input> = (0..200).map(pressing).collect();
    let weak = typed_stage(true);
    let replay = record(&weak, &inputs).unwrap();
    let verified = verify(&weak, &replay.to_bytes()).unwrap();
    // Four hits of 12 clear 44 hit points; without the weakness it takes five of 10, later.
    let neutral = typed_stage(false);
    let slower = record(&neutral, &inputs).unwrap();
    assert!(verify(&neutral, &slower.to_bytes()).is_ok());
    assert!(verified.ticks < slower.ticks);
    // The profile is content: a replay recorded with it does not verify without it.
    assert!(verify(&neutral, &replay.to_bytes()).is_err());
}
