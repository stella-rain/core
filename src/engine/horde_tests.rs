use super::*;
use crate::run_content::RunContent;
use crate::time_limit::TimeLimit;

fn engine(boss_ticks: &[u32]) -> Engine {
    engine_with(boss_ticks, &[])
}

fn engine_with(boss_ticks: &[u32], no_call_ticks: &[u32]) -> Engine {
    engine_edited(boss_ticks, no_call_ticks, |_| {})
}

/// As `engine_with`, the mode file edited by `edit` before it is parsed.
fn engine_edited(
    boss_ticks: &[u32],
    no_call_ticks: &[u32],
    edit: impl FnOnce(&mut serde_json::Value),
) -> Engine {
    let mut v: serde_json::Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    edit(&mut v);
    v["no_call_ticks"] = serde_json::json!(no_call_ticks);
    v["length_ticks"] = serde_json::json!(boss_ticks.last().unwrap());
    v["boss_ticks"] = serde_json::json!(boss_ticks);
    v["waves"] = serde_json::json!([]);
    v["elite_table"] =
        serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
    let mode = TimeLimit::parse(v.to_string().as_bytes()).unwrap();
    let setup = RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 1,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        difficulty: "normal".into(),
    };
    Engine::start_time_limit(&setup, RunContent::bundled(), &mode).unwrap()
}

fn run_to(e: &mut Engine, tick: u32) {
    while e.tick() < tick {
        e.step(Input::default());
    }
}

/// The tick each boss came on and the tick it called its elites on, the main character
/// kept alive.
fn arrivals_and_calls(e: &mut Engine, until: u32) -> (Vec<u32>, Vec<u32>) {
    let (mut arrived, mut called) = (Vec::new(), Vec::new());
    let mut last = None;
    while e.tick() < until {
        e.state.player.hp = e.state.player.max_hp;
        e.step(Input::default());
        let id = e.state.boss.as_ref().map(|b| b.id);
        if id.is_some() && id != last {
            arrived.push(e.tick());
        }
        last = id;
        if e.events()
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
        {
            called.push(e.tick());
        }
    }
    (arrived, called)
}

#[test]
fn a_boss_calls_its_elites_after_the_same_time_whenever_it_arrives() {
    // Mid-bosses at 60 and 3000, the final boss at 6000; the mode's call is 1800 ticks.
    let mut e = engine(&[60, 3000, 6000]);
    let (arrived, called) = arrivals_and_calls(&mut e, 6000);
    assert_eq!(arrived, [60, 3000, 6000]);
    let delays: Vec<u32> = arrived.iter().zip(&called).map(|(a, c)| c - a).collect();
    assert_eq!(delays, [1800, 1800], "{arrived:?} {called:?}");
}

/// Drops the boss on the field below 70% of its hit points and steps once: whether it
/// called its elites on that tick.
fn calls_below_70_pct(e: &mut Engine) -> bool {
    let boss = e.state.boss.as_mut().unwrap();
    boss.hp = boss.max_hp * 69 / 100;
    e.step(Input::default());
    e.events()
        .iter()
        .any(|ev| matches!(ev, DomainEvent::ElitesCalled { .. }))
}

#[test]
fn a_boss_calls_below_70_pct_unless_its_time_is_a_no_call_time() {
    // The first time stands for the 5-minute boss, the second for the 10-minute one.
    let mut e = engine_with(&[50, 200, 400], &[50]);
    run_to(&mut e, 60);
    assert!(!calls_below_70_pct(&mut e), "a no-call boss never calls");
    run_to(&mut e, 210);
    assert!(calls_below_70_pct(&mut e), "well before its 30 seconds");
}

/// The first summon rule's agent of a mid-boss gets this many hit points, plus the boss's
/// place in the mode file's list, so each boss's summon tells which boss made it.
const SUMMON_HP: u32 = 50_000;

fn summoning_mode(v: &mut serde_json::Value) {
    let agent = serde_json::json!({
        "base": "fairy_a",
        "hp": SUMMON_HP,
        "radius": 2048,
        "movement": {"type": "follow", "distance": 12288, "speed": 1},
        "rules": []
    });
    for (i, boss) in v["bosses"].as_array_mut().unwrap().iter_mut().enumerate() {
        let mut agent = agent.clone();
        agent["hp"] = serde_json::json!(SUMMON_HP + i as u32);
        boss["phases"][0]["rules"] = serde_json::json!([{
            "when": {"type": "always"},
            "target": "self",
            "do": {"type": "summon", "count": 1, "agent": agent},
            "once": true
        }]);
    }
}

#[test]
fn a_boss_summon_outlives_its_boss_and_keeps_its_definition() {
    let mut e = engine_edited(&[50, 100, 150], &[], summoning_mode);
    run_to(&mut e, 60);
    let first = e.state.boss.as_ref().unwrap().id;
    let summon_hp = |e: &Engine| -> Vec<u32> {
        e.state
            .enemies
            .iter()
            .map(|en| e.stage.defs.agent(en.def).hp)
            .collect()
    };
    let made = summon_hp(&e);
    assert_eq!(made.len(), 1, "the first boss has summoned");
    e.state.boss.as_mut().unwrap().hp = 0;
    for _ in 0..60 {
        e.state.player.hp = e.state.player.max_hp;
        e.step(Input::default());
    }
    assert_ne!(
        e.state.boss.as_ref().unwrap().id,
        first,
        "the next boss has come"
    );
    let all = summon_hp(&e);
    assert_eq!(
        all[0], made[0],
        "the first summon still resolves to the first boss's summon agent"
    );
    assert_eq!(all.len(), 2, "the second boss summoned its own");
    assert_ne!(all[1], all[0], "from its own definition");
}

/// The definition of the boss on the field.
fn boss_def(e: &Engine) -> &crate::stage::Boss {
    &e.stage.boss(e.state.boss.as_ref().unwrap().boss).boss
}

/// The slot of the boss on the field in the run's list of boss definitions.
fn boss_slot(e: &Engine) -> u32 {
    u32::from(e.state.boss.as_ref().unwrap().boss.0)
}

#[test]
fn the_world_compiles_each_horde_boss_once() {
    // Two sprites with no definition of their own are made from the template, and
    // each is its own boss; the same sprite coming twice is one.
    let mut e = engine_edited(&[50, 100, 150], &[], |v| {
        let pool = ["horn_charger", "weaver"];
        v["mid_boss_pool"] = serde_json::json!(pool);
        v["final_boss"] = serde_json::json!("horn_charger");
        v["bosses"]
            .as_array_mut()
            .unwrap()
            .retain(|b| !pool.contains(&b["base"].as_str().unwrap()));
    });
    // Three definitions of the file, and the template once for each of the two sprites.
    let bases: Vec<&str> = e
        .stage
        .bosses
        .iter()
        .map(|b| b.boss.base.0.as_str())
        .collect();
    assert_eq!(bases.len(), 5, "{bases:?}");
    let mut distinct = bases.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 5, "one definition per sprite: {bases:?}");
    let mut slots = Vec::new();
    for until in [60, 110, 160] {
        run_to(&mut e, until);
        slots.push(boss_slot(&e));
    }
    assert_ne!(
        slots[0], slots[1],
        "two sprites, two definitions: {slots:?}"
    );
    assert!(
        slots[2] == slots[0] || slots[2] == slots[1],
        "the final boss shares the slot of its sprite's first arrival: {slots:?}"
    );
}

#[test]
fn a_boss_coming_leaves_the_stage_as_it_was() {
    let mut e = engine(&[50, 100]);
    let before = e.stage.stage.clone();
    run_to(&mut e, 100);
    assert!(e.state.boss.is_some(), "a boss has come");
    assert_eq!(
        e.stage.stage, before,
        "the stage is read-only for a run (core#271)"
    );
}

#[test]
fn the_final_boss_is_the_core_pair_and_its_death_clears_the_run() {
    let mut e = engine(&[50, 100]);
    run_to(&mut e, 100);
    assert_eq!(boss_def(&e).base.0, "core_pair");
    assert_eq!(e.state.boss.as_ref().unwrap().parts.len(), 2);
    e.state.boss.as_mut().unwrap().hp = 0;
    e.step(Input::default());
    assert_eq!(e.outcome(), Outcome::Cleared);
}

#[test]
fn a_mid_boss_s_death_does_not_clear_the_run_and_the_final_boss_s_does() {
    let mut e = engine(&[50, 100]);
    run_to(&mut e, 50);
    assert!(e.state.boss.as_ref().is_some_and(|b| b.hp > 0));
    // The mid-boss falls; the stage's own rule would clear the run here.
    e.state.boss.as_mut().unwrap().hp = 0;
    run_to(&mut e, 99);
    assert_eq!(e.outcome(), Outcome::Running);
    run_to(&mut e, 100);
    let final_boss = e.state.boss.as_ref().unwrap();
    assert!(final_boss.hp > 0, "a new boss at the last time");
    e.state.boss.as_mut().unwrap().hp = 0;
    e.step(Input::default());
    assert_eq!(e.outcome(), Outcome::Cleared);
    assert!(e.events().contains(&DomainEvent::StageCleared));
}

#[test]
fn the_main_character_s_death_fails_the_run_and_wins_over_the_final_boss_s() {
    let mut e = engine(&[50, 100]);
    run_to(&mut e, 100);
    e.state.boss.as_mut().unwrap().hp = 0;
    e.state.player.hp = 0;
    e.step(Input::default());
    assert_eq!(e.outcome(), Outcome::Failed);
    assert!(e.events().contains(&DomainEvent::StageFailed));
    assert!(!e.events().contains(&DomainEvent::StageCleared));
}

#[test]
fn the_run_lasts_its_length_and_its_overtime_and_an_empty_field_does_not_clear_it() {
    let mut e = engine(&[50, 100]);
    run_to(&mut e, 99);
    assert_eq!(e.outcome(), Outcome::Running);
    assert_eq!(e.stage.length_ticks, 100 + 18_000, "the run's overtime");
}

#[test]
fn the_run_fails_when_the_overtime_runs_out_with_the_final_boss_alive() {
    let mut e = engine(&[50, 100]);
    e.state.player.hp = u32::MAX;
    e.state.player.max_hp = u32::MAX;
    run_to(&mut e, 100 + 18_000 - 1);
    assert_eq!(e.outcome(), Outcome::Running);
    e.step(Input::default());
    assert_eq!(e.outcome(), Outcome::Failed);
    assert!(e.events().contains(&DomainEvent::StageFailed));
}

/// A `time_limit` run of `edit`ed bundled mode, no waves, that the main character cannot die in.
fn run_of(seed: u64, edit: impl FnOnce(&mut serde_json::Value)) -> Engine {
    let mut v: serde_json::Value = serde_json::from_slice(TimeLimit::BUNDLED).unwrap();
    v["waves"] = serde_json::json!([]);
    edit(&mut v);
    let mode = TimeLimit::parse(v.to_string().as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    let setup = RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        difficulty: "normal".into(),
    };
    let mut e = Engine::start_time_limit(&setup, RunContent::bundled(), &mode).unwrap();
    e.state.player.hp = u32::MAX;
    e.state.player.max_hp = u32::MAX;
    e
}

/// Steps to `until` and lists each elite spawned: the tick, the sprite.
fn elites_spawned(e: &mut Engine, until: u32) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    while e.tick() < until {
        e.step(Input::default());
        for ev in e.events() {
            if let DomainEvent::EliteSpawned { asset, .. } = ev {
                out.push((e.tick(), asset.0.clone()));
            }
        }
    }
    out
}

#[test]
fn elite_minutes_land_on_each_whole_minute_but_boss_ticks_and_overtime() {
    // Room for all sixteen: the cap is not what is tested here.
    let mut e = run_of(1, |v| v["elite_cap"] = serde_json::json!(32));
    let ticks: Vec<u32> = elites_spawned(&mut e, 72_000 + 3_600 * 2)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let expected: Vec<u32> = (1..=19)
        .map(|k| k * 3_600)
        .filter(|t| ![18_000, 36_000, 54_000].contains(t))
        .collect();
    assert_eq!(expected.len(), 16);
    assert_eq!(ticks, expected);
}

#[test]
fn a_tier_boundary_on_an_elite_minute_picks_the_later_tier() {
    let mut e = run_of(1, |v| {
        v["length_ticks"] = serde_json::json!(18_000);
        v["boss_ticks"] = serde_json::json!([18_000]);
        v["mid_boss_pool"] = serde_json::json!([]);
        v["elite_cap"] = serde_json::json!(32);
        v["elite_table"] = serde_json::json!([
            { "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] },
            { "start_tick": 10_800, "weights": [{ "elite": "elite_mid", "weight": 1 }] }
        ]);
    });
    let spawned = elites_spawned(&mut e, 14_400);
    let sprites: Vec<(u32, &str)> = spawned.iter().map(|(t, a)| (*t, a.as_str())).collect();
    assert_eq!(
        sprites,
        [
            (3_600, "bat"),
            (7_200, "bat"),
            (10_800, "fairy_a"),
            (14_400, "fairy_a")
        ]
    );
}

#[test]
fn the_draws_are_those_of_a_table_of_the_same_shape_before_types_were_content() {
    // Recorded before elite types became content: same weights, same seed. The positions are
    // those after the boss's call counts from its arrival (ADR-103); the draws are the same.
    let table = serde_json::json!([{ "start_tick": 0, "weights": [
        { "elite": "elite_low", "weight": 1 }, { "elite": "elite_mid", "weight": 2 } ] },
        { "start_tick": 10_800, "weights": [
        { "elite": "elite_low", "weight": 1 }, { "elite": "elite_high", "weight": 1 }] }]);
    let mut e = run_of(7, |v| {
        v["elite_cap"] = serde_json::json!(32);
        v["elite_table"] = table.clone();
    });
    let spawned = elites_spawned(&mut e, 5 * 3_600 + 10);
    let sprites: Vec<(u32, &str)> = spawned.iter().map(|(t, a)| (*t, a.as_str())).collect();
    assert_eq!(
        sprites,
        [
            (3_600, "bat"),
            (7_200, "bat"),
            (10_800, "bat"),
            (14_400, "bat")
        ]
    );
    assert_eq!(e.state.rng.state(), 10_372_713_005_361_028_292);
    let at: Vec<(i32, i32)> = e
        .state
        .enemies
        .iter()
        .map(|a| (a.at.x.0, a.at.y.0))
        .collect();
    assert_eq!(
        at,
        [
            (430_742, 482_638),
            (468_518, 610_167),
            (623_198, 497_785),
            (502_998, 624_450)
        ]
    );
}

#[test]
fn each_grade_is_spawned_from_its_own_definition() {
    let mode = TimeLimit::bundled();
    for (id, sprite) in [
        ("elite_low", "bat"),
        ("elite_mid", "fairy_a"),
        ("elite_high", "horns_2"),
    ] {
        let def = mode.elite_types().iter().find(|t| t.id.0 == id).unwrap();
        let mut e = run_of(1, |v| {
            v["elite_table"] =
                serde_json::json!([{"start_tick": 0, "weights": [{"elite": id, "weight": 1}]}]);
        });
        let spawned = elites_spawned(&mut e, 3_600);
        assert_eq!(spawned, [(3_600, sprite.to_string())], "{id}");
        let elites: Vec<_> = e
            .state
            .enemies
            .iter()
            .filter(|a| a.elite.is_some())
            .collect();
        assert_eq!(elites.len(), 1);
        assert_eq!(
            elites[0].hp, def.enemy.hp,
            "{id}'s health is its definition's"
        );
    }
    let hp = |id: &str| {
        mode.elite_types()
            .iter()
            .find(|t| t.id.0 == id)
            .unwrap()
            .enemy
            .hp
    };
    assert!(hp("elite_low") < hp("elite_mid") && hp("elite_mid") < hp("elite_high"));
}

#[test]
fn the_cap_holds_an_elite_back_but_the_generator_is_drawn_the_same() {
    let table = serde_json::json!([{ "start_tick": 0, "weights": [
        { "elite": "elite_low", "weight": 1 }, { "elite": "elite_mid", "weight": 1 } ] }]);
    let run = |cap: u32| {
        run_of(7, |v| {
            v["elite_cap"] = serde_json::json!(cap);
            v["elite_table"] = table.clone();
        })
    };
    let (mut tight, mut roomy) = (run(1), run(32));
    let (mut few, mut many) = (0, 0);
    for minute in 1..=4 {
        let at = minute * 3_600;
        few += elites_spawned(&mut tight, at).len();
        many += elites_spawned(&mut roomy, at).len();
        assert_eq!(
            tight.state.rng.state(),
            roomy.state.rng.state(),
            "the generator after minute {minute}"
        );
    }
    assert_eq!((few, many), (1, 4), "the cap held three back");
    let alive = |e: &Engine| e.state.enemies.iter().filter(|a| a.elite.is_some()).count();
    assert_eq!((alive(&tight), alive(&roomy)), (1, 4));
}

#[test]
fn the_boss_calls_the_elites_of_the_minutes_before_and_after_it_comes() {
    let mut e = run_of(3, |v| {
        v["length_ticks"] = serde_json::json!(20_000);
        v["boss_ticks"] = serde_json::json!([4_000, 20_000]);
        v["mid_boss_pool"] = serde_json::json!(["golem"]);
        v["elite_cap"] = serde_json::json!(32);
        v["elite_table"] = serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
    });
    let mut called = Vec::new();
    while e.tick() < 7_200 {
        e.step(Input::default());
        for ev in e.events() {
            if let DomainEvent::ElitesCalled { elites } = ev {
                called.push((e.tick(), *elites));
            }
        }
    }
    // The elite of minute 1 is alive when the boss comes at 4000, and is called when the
    // boss's 1800 ticks from its arrival are up (ADR-103).
    assert_eq!(called, [(5_800, 1)]);
    // The elite of minute 2 spawned after the call: called at once.
    let elites: Vec<bool> = e
        .state
        .enemies
        .iter()
        .filter_map(|a| a.elite.map(|el| el.called))
        .collect();
    assert_eq!(elites, [true, true]);
}

/// A bundled `time_limit` run on `difficulty`, the main character alive (`arrivals_and_calls`
/// keeps it so; `boss_sprite_at` does too).
fn bundled_run(difficulty: &str, seed: u64) -> Engine {
    let setup = RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        difficulty: difficulty.into(),
    };
    let mode = TimeLimit::bundled();
    Engine::start_time_limit(&setup, RunContent::bundled(), mode).unwrap()
}

/// The sprite of the boss on the field once the run has reached `tick`.
fn boss_sprite_at(e: &mut Engine, tick: u32) -> Option<String> {
    while e.tick() < tick {
        e.state.player.hp = e.state.player.max_hp;
        e.step(Input::default());
    }
    e.snapshot()
        .entities
        .iter()
        .find(|v| v.kind == crate::snapshot::EntityKind::Boss)
        .map(|v| v.asset.0.clone())
}

/// The mid-bosses drawn from the pool come without repeating whatever the minute marks are
/// (`spawn_boss` marks each pick used), so the schedule is shortened to 60-tick steps.
fn assert_three_distinct_mid_bosses(seed: u64) {
    const POOL: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];
    let times = [60, 120, 180];
    let mut e = run_of(seed, |v| {
        v["length_ticks"] = serde_json::json!(240);
        v["boss_ticks"] = serde_json::json!([60, 120, 180, 240]);
        v["no_call_ticks"] = serde_json::json!([]);
        v["elite_table"] = serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
    });
    let drawn: Vec<String> = times
        .iter()
        .map(|&t| boss_sprite_at(&mut e, t).expect("a boss"))
        .collect();
    let mut unique = drawn.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3, "seed {seed}: {drawn:?}");
    assert!(
        drawn.iter().all(|d| POOL.contains(&d.as_str())),
        "{drawn:?}"
    );
}

#[test]
fn the_run_draws_three_distinct_mid_bosses_from_the_four_seed_0() {
    assert_three_distinct_mid_bosses(0);
}

#[test]
fn the_run_draws_three_distinct_mid_bosses_from_the_four_seed_1() {
    assert_three_distinct_mid_bosses(1);
}

#[test]
fn the_run_draws_three_distinct_mid_bosses_from_the_four_seed_2() {
    assert_three_distinct_mid_bosses(2);
}

#[test]
fn the_run_draws_three_distinct_mid_bosses_from_the_four_seed_3() {
    assert_three_distinct_mid_bosses(3);
}

fn assert_easy_run_draws_one_mid_boss(seed: u64) {
    const POOL: [&str; 4] = ["stone_warden", "lantern_choir", "horn_charger", "weaver"];
    let mut e = bundled_run("easy", seed);
    let horde = e.state.horde.as_ref().unwrap();
    assert_eq!(horde.boss_ticks, [18_000, 36_000], "seed {seed}");
    let drawn = boss_sprite_at(&mut e, 18_000).expect("a boss");
    assert!(POOL.contains(&drawn.as_str()), "seed {seed}: {drawn}");
}

#[test]
fn the_easy_run_draws_one_mid_boss_from_the_four_and_then_the_final_boss_seed_0() {
    assert_easy_run_draws_one_mid_boss(0);
}

#[test]
fn the_easy_run_draws_one_mid_boss_from_the_four_and_then_the_final_boss_seed_1() {
    assert_easy_run_draws_one_mid_boss(1);
}

#[test]
fn each_mid_boss_s_hit_points_rise_by_its_slot_and_the_final_boss_s_do_not() {
    // Normal difficulty scales nothing; the slots are 100%, 130% and 160%.
    let mut e = engine(&[50, 100, 150, 200]);
    let mode = TimeLimit::bundled();
    let mut seen = Vec::new();
    for (t, pct) in [(50, 100), (100, 130), (150, 160), (200, 100)] {
        run_to(&mut e, t);
        let boss = e.state.boss.as_ref().unwrap();
        let def = mode
            .bosses()
            .iter()
            .find(|b| b.base == boss_def(&e).base)
            .unwrap();
        assert_eq!(boss.max_hp, def.hp * pct / 100, "at {t}");
        let parts: Vec<u32> = boss.parts.iter().map(|p| p.hp).collect();
        let want: Vec<u32> = def.parts.iter().map(|p| p.hp * pct / 100).collect();
        assert_eq!(parts, want, "at {t}");
        seen.push(def.base.0.clone());
    }
    assert_eq!(seen[3], "core_pair");
}

/// The first boss calls nothing and the later ones call 1800 ticks after they come, on a
/// schedule a quarter as long as the real one: bosses at 4000, 8000 and 12000.
fn assert_calls_after_30_seconds(seed: u64) {
    let mut e = run_of(seed, |v| {
        v["length_ticks"] = serde_json::json!(20_000);
        v["boss_ticks"] = serde_json::json!([4_000, 8_000, 12_000, 20_000]);
        v["no_call_ticks"] = serde_json::json!([4_000]);
        v["elite_cap"] = serde_json::json!(32);
        v["elite_table"] = serde_json::json!([{"start_tick": 0, "weights": [{"elite": "elite_low", "weight": 1}]}]);
    });
    let (arrived, called) = arrivals_and_calls(&mut e, 13_800);
    assert_eq!(arrived, [4_000, 8_000, 12_000], "seed {seed}");
    assert_eq!(called, [9_800, 13_800], "seed {seed}");
}

#[test]
fn the_first_boss_calls_no_elites_and_the_later_ones_call_after_30_seconds_seed_0() {
    assert_calls_after_30_seconds(0);
}

#[test]
fn the_first_boss_calls_no_elites_and_the_later_ones_call_after_30_seconds_seed_1() {
    assert_calls_after_30_seconds(1);
}

#[test]
fn the_five_minute_boss_calls_no_elites_and_the_later_ones_call_after_30_seconds() {
    // The only test of the real schedule: the 5-minute boss comes at 18000 and calls nothing;
    // the 10-minute one at 36000 calls 1800 ticks after it came, and so does the 15-minute
    // one at 54000.
    let mut e = bundled_run("normal", 0);
    let (arrived, called) = arrivals_and_calls(&mut e, 55_800);
    assert_eq!(arrived, [18_000, 36_000, 54_000]);
    assert_eq!(called, [37_800, 55_800]);
}
