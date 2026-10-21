//! What a run's difficulty does (ADR-110): the enemies, elites and bosses it spawns have its
//! percent of the content's hit points, their attacks its percent of the damage, its rarity
//! range bounds the stars that drop, and easy sets the Time Limit's length, boss times and level
//! table. A setup that names none plays on `normal`.

use serde_json::{Value, json};
use stella_rain_core::difficulties::scale_pct;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::input::Input;
use stella_rain_core::rewards::Rarity;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::snapshot::{EntityKind, Snapshot};
use stella_rain_core::stage::LevelTable;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::Error;

const DIFFICULTIES: [(&str, u32, u32); 4] = [
    ("easy", 70, 75),
    ("normal", 100, 100),
    ("hard", 150, 125),
    ("rainy", 200, 150),
];

fn setup(difficulty: &str) -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        seed: 5,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        difficulty: difficulty.into(),
        auto_combo: false,
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
    }
}

fn mode(edit: impl FnOnce(&mut Value)) -> TimeLimit {
    let mut v: Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    edit(&mut v);
    TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

fn start(difficulty: &str, m: &TimeLimit) -> Engine {
    Engine::start_time_limit(&setup(difficulty), RunContent::bundled(), m).unwrap()
}

fn step_to(e: &mut Engine, tick: u32) -> Snapshot {
    while e.tick() < tick {
        e.step(Input::default());
    }
    e.snapshot()
}

fn hp_of(s: &Snapshot, kind: EntityKind) -> Vec<u32> {
    s.entities
        .iter()
        .filter(|v| v.kind == kind)
        .map(|v| v.hp)
        .collect()
}

/// One enemy of `hp` hit points that stands still, far from the main character.
fn dummy_run(hp: u32) -> TimeLimit {
    mode(|v| {
        v["waves"] = json!([{
            "at_tick": 1,
            "enemy": { "base": "bat", "hp": hp, "radius": 2048,
                       "movement": { "type": "straight", "vx": 0, "vy": 0 } },
            "spawn": { "x": 0, "y": 0 }, "count": 1, "every_ticks": 0, "around": 200_000
        }]);
    })
}

#[test]
fn an_enemy_spawns_with_the_difficultys_percent_of_its_hit_points() {
    for (id, hp_pct, _) in DIFFICULTIES {
        let mut e = start(id, &dummy_run(3_001));
        let hp = hp_of(&step_to(&mut e, 2), EntityKind::Enemy);
        assert_eq!(hp, [scale_pct(3_001, hp_pct)], "{id}");
    }
}

#[test]
fn a_scaled_enemy_keeps_at_least_one_hit_point() {
    let mut e = start("easy", &dummy_run(1));
    assert_eq!(hp_of(&step_to(&mut e, 2), EntityKind::Enemy), [1]);
}

#[test]
fn a_boss_spawns_with_the_difficultys_percent_of_its_hit_points() {
    let m = mode(|v| {
        v["length_ticks"] = json!(200);
        v["boss_ticks"] = json!([50, 100, 150, 200]);
        v["mid_boss_pool"] = json!(["bat", "fairy_a", "pilot_a"]);
        v["waves"] = json!([]);
        v["elite_table"] =
            json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    });
    // Easy's own boss times put the first boss at 2.5 minutes.
    let first = |id: &str| if id == "easy" { 9_000 } else { 50 };
    let boss = |id: &str| {
        let s = step_to(&mut start(id, &m), first(id));
        (hp_of(&s, EntityKind::Boss), hp_of(&s, EntityKind::BossPart))
    };
    let (base, base_parts) = boss("normal");
    assert_eq!(base.len(), 1);
    for (id, hp_pct, _) in DIFFICULTIES {
        let (hp, parts) = boss(id);
        assert_eq!(hp, [scale_pct(base[0], hp_pct)], "{id}");
        let scaled: Vec<u32> = base_parts.iter().map(|p| scale_pct(*p, hp_pct)).collect();
        assert_eq!(parts, scaled, "{id}");
    }
}

/// An enemy beside the main character that shoots it and cannot be hurt: the hit points the
/// main character has lost after `ticks`.
fn damage_taken(difficulty: &str, ticks: u32) -> u32 {
    let m = mode(|v| {
        v["waves"] = json!([{
            "at_tick": 1,
            "enemy": {
                "base": "bat", "hp": 9_000_000, "radius": 2048,
                "movement": { "type": "straight", "vx": 0, "vy": 0 },
                "rules": [{ "when": { "type": "always" }, "target": "nearest_ally",
                    "do": { "type": "attack", "attack": { "preset": {
                        "id": "aimed_single", "args": [640] } } } }]
            },
            "spawn": { "x": 0, "y": 0 }, "count": 1, "every_ticks": 0, "around": 30_000
        }]);
    });
    let mut e = start(difficulty, &m);
    let full = e.snapshot().player.hp;
    full - step_to(&mut e, ticks).player.hp
}

#[test]
fn an_enemys_attack_does_the_difficultys_percent_of_the_damage() {
    // The first bullet lands at about tick 60, and the main character is then invulnerable
    // for a while: one hit, scaled before the damage pipeline (ADR-110).
    let normal = damage_taken("normal", 70);
    assert!(normal > 0, "the enemy hits");
    for (id, _, damage_pct) in DIFFICULTIES {
        assert_eq!(damage_taken(id, 70), scale_pct(normal, damage_pct), "{id}");
    }
}

#[test]
fn easy_runs_ten_minutes_with_its_boss_times_and_the_short_level_table() {
    let content = RunContent::bundled();
    let mode = content.time_limit();
    let normal = run_setup::stage(&setup("normal")).unwrap();
    assert_eq!(
        normal.length_ticks,
        mode.length_ticks() + mode.overtime_ticks()
    );
    assert_eq!(normal.level_table(), *mode.level_table());
    for id in ["hard", "rainy"] {
        let s = run_setup::stage(&setup(id)).unwrap();
        assert_eq!(
            (s.length_ticks, s.level_table()),
            (normal.length_ticks, normal.level_table())
        );
    }
    let easy = run_setup::stage(&setup("easy")).unwrap();
    assert_eq!(easy.length_ticks, 10 * 60 * 60 + mode.overtime_ticks());
    assert_eq!(easy.level_table(), LevelTable::SHORT);
    // A replay of an easy run may hold its own length, not the normal one's.
    assert_eq!(
        run_setup::max_replay_ticks(&setup("easy"), &easy, mode),
        easy.length_ticks + mode.star_hunter_extra_ticks()
    );
}

#[test]
fn easys_bosses_come_at_its_times() {
    let m = mode(|v| v["waves"] = json!([]));
    let mut e = start("easy", &m);
    for (tick, bosses) in [(8_999, 0), (9_000, 1)] {
        assert_eq!(
            hp_of(&step_to(&mut e, tick), EntityKind::Boss).len(),
            bosses,
            "tick {tick}"
        );
    }
    // Normal's first boss is at 5 minutes.
    let mut e = start("normal", &m);
    assert!(hp_of(&step_to(&mut e, 9_000), EntityKind::Boss).is_empty());
}

#[test]
fn an_override_with_more_mid_bosses_than_the_pool_has_is_refused() {
    let m = mode(|v| {
        v["length_ticks"] = json!(600);
        // The bundled schedule's waves start after a 600-tick run ends; this run has none.
        v["waves"] = json!([]);
        v["boss_ticks"] = json!([300, 600]);
        v["mid_boss_pool"] = json!(["golem"]);
        v["elite_table"] =
            json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    });
    assert!(
        matches!(Engine::start_time_limit(&setup("easy"), RunContent::bundled(), &m),
            Err(Error::Invalid(issues)) if issues[0].path == "$.difficulty")
    );
    // A difficulty that overrides nothing never asks for more.
    assert!(Engine::start_time_limit(&setup("hard"), RunContent::bundled(), &m).is_ok());
}

#[test]
fn a_setup_names_a_difficulty_of_the_file() {
    let content = RunContent::bundled();
    assert!(run_setup::validate_in(&setup("rainy"), content).is_ok());
    for (bad, path) in [("nightmare", "$.difficulty"), ("Bad Id", "$.difficulty")] {
        let e = run_setup::validate_in(&setup(bad), content).unwrap_err();
        match e {
            Error::Invalid(issues) => assert_eq!(issues[0].path, path, "{bad}"),
            other => panic!("{other:?}"),
        }
        assert!(Engine::start(&setup(bad)).is_err(), "{bad}");
    }
}

#[test]
fn a_setup_without_a_difficulty_reads_as_normal() {
    let mut v = serde_json::to_value(setup("hard")).unwrap();
    assert_eq!(v["difficulty"], "hard", "it is written out");
    v.as_object_mut().unwrap().remove("difficulty");
    let old = run_setup::parse_and_validate(v.to_string().as_bytes()).unwrap();
    assert_eq!(old.difficulty.0, "normal");
    assert_eq!(old, setup("normal"));
}

#[test]
fn the_difficulty_is_part_of_the_run() {
    let hash = |id: &str| {
        let mut e = start(id, &dummy_run(3_000));
        for _ in 0..30 {
            e.step(Input::default());
        }
        e.state_hash()
    };
    assert_eq!(hash("normal"), hash("normal"));
    assert_ne!(hash("normal"), hash("hard"));
    assert_ne!(hash("easy"), hash("normal"));
}

/// The rarities of the stars that drop in a scripted `time_limit` run on `difficulty`, offers
/// answered with the first option.
fn star_rarities(difficulty: &str, ticks: u32) -> Vec<Rarity> {
    let mut e = Engine::start(&setup(difficulty)).unwrap();
    let mut out = Vec::new();
    for t in 0..ticks {
        if e.outcome() != Outcome::Running {
            break;
        }
        while e.offer_open() {
            e.choose(0).unwrap();
        }
        let input = crate::common::run::run_input(&e, t);
        e.step(input);
        for ev in e.events() {
            if let DomainEvent::StarDropped { rarity, .. } = ev {
                out.push(*rarity);
            }
        }
    }
    out
}

#[test]
fn stars_drop_inside_the_difficultys_rarity_range() {
    // Level 1's drop table weighs only common stars; the rewards have every rarity, so rainy's
    // range (uncommon and up) is the rewards' own, and its level-1 stars fall to uncommon.
    let normal = star_rarities("normal", 3_000);
    assert!(normal.contains(&Rarity::Common), "{normal:?}");
    let rainy = star_rarities("rainy", 3_000);
    assert!(!rainy.is_empty());
    assert!(rainy.iter().all(|r| *r >= Rarity::Uncommon), "{rainy:?}");
    let easy = star_rarities("easy", 3_000);
    assert!(easy.iter().all(|r| *r <= Rarity::Rare), "{easy:?}");
}
