//! The bundled modes fire no bullet (BR1a, DS10, DS11): the crowd, the elites and the bosses
//! attack with zones, orbs and melee. These tests read the bundled files, so they check the
//! content; the rules they use are covered by `zones.rs`, `orbs.rs` and `ranged_cap.rs`.

use serde_json::Value;
use stella_rain_core::run_setup::mode_stage;
use stella_rain_core::time_limit::TimeLimit;

/// The presets that fire bullets.
const BULLET_PRESETS: [&str; 5] = [
    "aimed_single",
    "aimed_spread",
    "spread_5",
    "twin_shot",
    "spiral",
];

const MID_BOSSES: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];

fn time_limit() -> Value {
    serde_json::from_slice(TimeLimit::BUNDLED).expect("the bundled file is JSON")
}

/// Every bundled mode file, by name.
fn modes() -> Vec<(&'static str, Value)> {
    let file = |id| serde_json::from_slice(mode_stage(id).expect("a stage file")).expect("JSON");
    vec![
        ("time_limit", time_limit()),
        ("trial", file("trial")),
        ("tutorial", file("tutorial")),
    ]
}

/// Every object and the path it is at, depth first.
fn walk<'a>(v: &'a Value, path: String, out: &mut Vec<(String, &'a Value)>) {
    match v {
        Value::Object(map) => {
            out.push((path.clone(), v));
            for (k, child) in map {
                walk(child, format!("{path}.{k}"), out);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                walk(child, format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

fn objects(v: &Value) -> Vec<(String, &Value)> {
    let mut out = Vec::new();
    walk(v, "$".into(), &mut out);
    out
}

fn places_zones_or_orbs(v: &Value) -> bool {
    objects(v)
        .iter()
        .any(|(_, o)| matches!(o["type"].as_str(), Some("zone" | "orb")))
}

/// The bosses of a mode file: the list of `time_limit`, the one boss of a stage.
fn bosses_of(v: &Value) -> Vec<&Value> {
    match v["bosses"].as_array() {
        Some(list) => list.iter().collect(),
        None => vec![&v["boss"]],
    }
}

#[test]
fn no_bundled_mode_fires_a_bullet() {
    for (mode, v) in modes() {
        for (path, o) in objects(&v) {
            assert_ne!(o["type"], "fire", "{mode} {path}: a fire node");
            assert_ne!(o["type"], "clear_bullets", "{mode} {path}: clear_bullets");
            if let Some(id) = o["id"].as_str()
                && o.get("args").is_some()
            {
                assert!(
                    !BULLET_PRESETS.contains(&id),
                    "{mode} {path}: the bullet preset {id}"
                );
            }
        }
    }
}

#[test]
fn every_phase_change_of_a_boss_clears_the_zones() {
    for (mode, v) in modes() {
        for boss in bosses_of(&v) {
            let phases = boss["phases"].as_array().expect("phases");
            for (i, phase) in phases.iter().enumerate().skip(1) {
                let transition = phase["transition"].as_array().expect("a transition");
                assert!(
                    transition.iter().any(|t| t["type"] == "clear_zones"),
                    "{mode} {} phase {i}: {transition:?}",
                    boss["base"]
                );
            }
        }
    }
}

fn wave<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["waves"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == id)
        .unwrap_or_else(|| panic!("no wave {id}"))
}

/// The `attack` actions of a wave's rules.
fn attacks_of(wave: &Value) -> Vec<&Value> {
    wave["enemy"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["do"])
        .filter(|d| d["type"] == "attack")
        .collect()
}

#[test]
fn the_ranged_roles_attack_under_the_ranged_cap() {
    let v = time_limit();
    for id in ["ranged_shooter", "line_caster"] {
        let attacks = attacks_of(wave(&v, id));
        assert!(!attacks.is_empty(), "{id} attacks");
        for a in attacks {
            assert_eq!(a["ranged"], true, "{id}");
            assert!(places_zones_or_orbs(a), "{id} places a zone");
        }
    }
}

#[test]
fn the_exploder_bursts_as_a_zone_round_itself() {
    let v = time_limit();
    let rule = &wave(&v, "exploder")["enemy"]["rules"][0];
    assert_eq!(rule["self_destruct"], true);
    let zones: Vec<_> = objects(&rule["do"])
        .into_iter()
        .filter(|(_, o)| o["type"] == "zone")
        .collect();
    assert_eq!(zones.len(), 1);
    assert!(zones[0].1.get("anchor").is_none(), "anchored at itself");
}

#[test]
fn the_line_caster_is_the_wave_ds11_names() {
    let v = time_limit();
    let w = wave(&v, "line_caster");
    assert_eq!(w["at_tick"], 7 * 3600);
    assert_eq!(w["every_ticks"], 600);
    assert_eq!(w["enemy"]["base"], "fairy_a");
    assert_eq!(w["enemy"]["movement"]["type"], "follow");
    assert_eq!(w["enemy"]["movement"]["distance"], 71_680);
    let rule = &w["enemy"]["rules"][0];
    assert_eq!(rule["cooldown_ticks"], 240);
    let lanes: Vec<_> = objects(&rule["do"])
        .into_iter()
        .filter(|(_, o)| o["type"] == "zone")
        .collect();
    assert_eq!(lanes.len(), 1);
    let lane = lanes[0].1;
    // 480 x 20 pixels, from itself, warned for 90 ticks and hurting for 6.
    assert_eq!(lane["shape"]["type"], "line");
    assert_eq!(lane["shape"]["length"], 480 * 256);
    assert_eq!(lane["shape"]["width"], 20 * 256);
    assert_eq!(lane["wind_up_ticks"], 90);
    assert_eq!(lane["active_ticks"], 6);
    assert!(lane.get("anchor").is_none());
}

#[test]
fn the_melee_roles_swing_and_do_not_shoot() {
    let v = time_limit();
    let melee = |attack: &Value| attack["attack"]["preset"]["id"] == "slash";
    // The two bat waves have no ID.
    for w in v["waves"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["id"].is_null())
    {
        assert!(attacks_of(w).into_iter().all(melee));
    }
    let elites = v["elite_types"].as_array().unwrap();
    let elite = |id: &str| elites.iter().find(|e| e["id"] == id).unwrap();
    let guardians = &elite("elite_shrine")["enemy"]["movement"]["guardians"];
    let escorts = &elite("elite_leader")["enemy"]["elite"]["escort"];
    for (id, group) in [("elite_shrine", guardians), ("elite_leader", escorts)] {
        let rules = group["enemy"]["rules"].as_array().unwrap();
        assert!(!rules.is_empty(), "{id}");
        assert!(rules.iter().all(|r| melee(&r["do"])), "{id}");
    }
}

#[test]
fn every_elite_with_an_attack_places_zones() {
    let v = time_limit();
    for id in [
        "elite_low",
        "elite_mid",
        "elite_high",
        "elite_leader",
        "elite_mimic",
    ] {
        let e = v["elite_types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap();
        let rules = e["enemy"]["rules"].as_array().unwrap();
        assert!(
            rules.iter().any(|r| places_zones_or_orbs(&r["do"])),
            "{id} places a zone"
        );
        assert!(
            !rules.iter().any(|r| r["do"]["ranged"] == true),
            "{id} is outside the cap"
        );
    }
}

#[test]
fn every_phase_of_every_boss_places_zones_or_orbs() {
    for (mode, v) in modes() {
        for boss in bosses_of(&v) {
            for (i, phase) in boss["phases"].as_array().unwrap().iter().enumerate() {
                assert!(
                    places_zones_or_orbs(&phase["timeline"]),
                    "{mode} {} phase {i} places nothing",
                    boss["base"]
                );
            }
        }
    }
}

#[test]
fn the_golems_take_the_stone_warden_pick() {
    // The trial's golem has the warden's two phases, the second with orbs; the tutorial's one.
    let modes = modes();
    let orbs = |v: &Value| objects(v).iter().any(|(_, o)| o["type"] == "orb");
    assert!(orbs(&modes[1].1["boss"]["phases"][1]), "trial phase 2");
    assert_eq!(modes[2].1["boss"]["phases"].as_array().unwrap().len(), 1);
}

#[test]
fn the_mid_bosses_are_still_the_four() {
    let v = time_limit();
    for id in MID_BOSSES {
        assert!(
            v["bosses"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["base"] == id)
        );
    }
}
