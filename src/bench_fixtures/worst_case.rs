//! The worst-case tick (ADR-020, ADR-021). Shared by `benches/worst_tick.rs`, which counts its
//! instructions, and `tests/worst_case.rs`, which checks that it really reaches every dynamic
//! cap: a benchmark of a tick that does not would pass without measuring the worst case.

use super::common::on_hit::{self, NEAR_Y};
use super::common::{self, START_X};
use crate::engine::Engine;
use crate::input::Input;
use crate::pack::ContentFiles;
use crate::run_content::RunContent;
use crate::runes::{MAX_DOT_STACKS, Runes};
use crate::stage::Stage;
use crate::validate::limits::{MAX_ASSIST_RANGE, MAX_ELITES_ALIVE};
use crate::version::Versions;
use serde_json::{Value, json};

/// Ticks to run before the tick that is measured, so that the caps are reached and held, the
/// statuses are at their stack cap, and the main character is charged.
pub const WARMUP_TICKS: u32 = 420;

/// The tick the crowd start to arrive: after the main character has taken every rune and
/// Rune Haste from the first eight enemies' stars (`take_runes`).
pub const WAVES_AT: u32 = 150;

/// The tick the shooter that lays a warning line arrives: after the boss and the companions have
/// placed their zones, before the companions place again into the one place the boss leaves free.
const LINE_SHOOTER_AT: u32 = 2;

/// How far right of the main character's start the line shooter arrives: 160 pixels, inside the
/// 640 x 360 view (a placeholder; any point in view of the start will do).
const LINE_SHOOTER_DX: i32 = 160 * 256;

/// The tick the main character sets off for the enemies, and the ticks it walks left and up
/// before it stands about 36,000 units from them with every enemy inside its step.
const WALK_FROM: u32 = 100;
const WALK_LEFT_TICKS: u32 = 40;
const WALK_UP_TICKS: u32 = 140;

/// From this tick the main character taps its Main weapon every other tick: its step lands on
/// every enemy, and each hit puts a stack of each status on it.
const TAP_FROM: u32 = WALK_FROM + WALK_LEFT_TICKS + WALK_UP_TICKS + 4;

/// Ticks the main character charges before it lets go on the measured tick.
const CHARGE_TICKS: u32 = 24;

/// The Rune Haste reward: the most one gives (`DEFAULT_MAX_RUNE_HASTE_PCT`), so that the picks reach
/// the cap.
const HASTE: &str = "haste";

/// Where the enemies stand.
pub const ENEMIES_AT: (i32, i32) = (60_000, 20_480);

/// The enemies of the run's cap (`bench_fixtures::enemy_cap`, 260 for the bundled pack) stand in
/// one place: four shooters, then the rest in five streams of waves that spawn one enemy a tick
/// each, 32 of them elites (`MAX_ELITES_ALIVE`)
/// (core BR1b: nothing here fires a projectile).
///
/// The push-apart (ADR-058) parts a fresh stack on every tick: on each tick it does not start
/// its attack, every enemy orders itself back to the point in one tick (`move_to`, which a
/// crowded enemy still carries out). So when the pass runs, all stand on the point but the 15
/// or 20 that started their attack the tick before, which are within 8 pixels of it. Every
/// enemy reads 32 others from the grid and counts 6. It also means every enemy fires a rule on
/// every tick.
///
/// Every crowd enemy starts the same attack every 30 ticks: an active phase of 30 ticks with a
/// hit box that spawns a wait of 29 ticks, which holds an attack task. Each start is refused
/// once 256 attacks are going (`MAX_ATTACK_TASKS`).
///
/// The four shooters (`MAX_RANGED_ATTACKERS`) start first, with a `ranged` run each: one
/// wind-up tick that sends a ring of eight orbs (`MAX_ORBS_PER_OWNER`), then an active phase
/// longer than the case, so each holds its token. The 32 orbs (`MAX_ORBS_ALIVE`) fly slowly
/// for 600 ticks and burst into nothing. No other projectile is alive or made.
///
/// The main character is a run that holds every rune (ADR-099): before the enemies arrive it
/// kills eight small ones in front of it and takes the seven runes and Rune Haste of 300 percent
/// (`DEFAULT_MAX_RUNE_HASTE_PCT`) from their stars, as a run does, through `Engine::new_with`. It then
/// walks to the left of the enemies, about 36,000 units from them (out of the 128-pixel arcs of their
/// attacks), where its Main weapon's step, a circle of 256
/// pixels, lands on all of them every tick: each hit puts a stack of bleed, poison and burn
/// on the enemy, to the stack cap (`MAX_DOT_STACKS`), and fires every on-hit rune that is ready.
/// The statuses last 600 ticks and tick every 4, which full Rune Haste makes every tick; so on
/// every tick three statuses on each enemy (780 at the cap of 260) deal damage, and each on-hit rune, whose cooldown the haste cuts to
/// a quarter, fires again as soon as its attack is over. The step charges for the 24 ticks
/// before the measured tick and lets go on it. From the measured tick aim assist and auto mode are on, with
/// the longest assist range, so every tick looks through all the enemies for a target
/// (ADR-072, ADR-073).
///
/// Hit boxes (ADR-055): the enemies' attack is one active phase of 30 ticks, so every enemy is
/// always in it, with a 128-pixel arc facing the main character, out of its reach. The four
/// companions (`MAX_COMPANIONS`) stand 78 pixels to the right of the enemies, behind every
/// arc, so each arc tests each of them against its angle and both edges and misses. They cleave
/// all the enemies every tick with an arc of their own (`rehit_ticks` 1, 1 damage a hit).
///
/// Zones (core Z1, `MAX_ZONES_ALIVE`): on the first tick the boss places 111 and each companion 4
/// (16 friendly), all active for 600 ticks, and the fourth shooter's wind-up lays one warn-only
/// line of 120 by 10 pixels for 600 ticks (RA1a, DS83), so 128 are alive on every measured tick,
/// 127 of them active.
/// The boss's are enemy zones at the map's corner, out of reach of everyone: each is tested
/// against the main character and the four companions and misses. The companions' are circles of
/// 100 pixels round the companion, which cover the whole crowd: each of the 16 is looked up
/// through the enemy grid and tests, on every tick, the enemies it has hit already (each
/// once, long before the measured tick). Later attempts to place more are dropped at the cap.
///
/// The fourth shooter's line is the one `line` zone, so under the split of ranged tokens into zone and
/// line pools (RA1) all four shooters hold a token. It arrives on tick 2, 160 pixels to the right
/// of the main character's start and so in its view, where RA1's off-view rule lets its attack
/// start at once (RA1b, DS82); it then walks to the stack as the others do, which arrive there.
pub fn stage() -> Stage {
    let swing = json!([{ "kind": "active", "ticks": 30,
                         "spawn": [{ "type": "wait", "ticks": 29 }],
                         "hit_box": { "type": "arc", "radius": 32_768,
                                      "half_angle": 60 * 256 } }]);
    let attack = common::rule(
        common::always(),
        json!("player"),
        json!({ "type": "attack", "attack": { "inline": swing } }),
        30,
    );
    // A shooter's run: one wind-up tick that sends a ring of eight orbs, then an active phase
    // that outlasts the measured ticks, so it holds a ranged token (`MAX_RANGED_ATTACKERS`) and
    // an attack task for all of them. The orbs fly 32 units a tick, aimed at the main character
    // 36,000 units away, and last 600 ticks: they stay alive, short of it.
    let orb = json!({ "type": "orb", "speed": 32, "ticks": 600, "breakable": false,
                      "direction": { "type": "aimed", "offset": 0 }, "burst": [] });
    let shot = json!([
        { "kind": "wind_up", "ticks": 1,
          "spawn": [{ "type": "ring", "count": 8, "body": [orb] }] },
        { "kind": "active", "ticks": 600, "hit_box": { "type": "arc", "radius": 32_768,
                                                        "half_angle": 60 * 256 } },
    ]);
    // The fourth shooter also lays a warning line in its wind-up (RA1a, DS83): a warn-only zone
    // of shape `line` (120 x 10 pixels) that outlasts the measured ticks and hurts nobody, so
    // that it holds the line token when ranged attacks are split into zone and line pools.
    let mut line_shot = shot.clone();
    line_shot[0]["spawn"].as_array_mut().unwrap().push(json!({
        "type": "zone", "shape": { "type": "line", "length": 30_720, "width": 2_560 },
        "warn_only": true, "wind_up_ticks": 600 }));
    let volley = |shot: Value| {
        let mut volley = common::rule(
            common::always(),
            json!("player"),
            json!({ "type": "attack", "attack": { "inline": shot } }),
            0,
        );
        volley["do"]["ranged"] = json!(true);
        volley
    };
    let (x, y) = ENEMIES_AT;
    let back = common::rule(
        common::always(),
        json!("self"),
        json!({ "type": "move_to", "to": { "x": x, "y": y }, "ticks": 1 }),
        0,
    );
    let enemy = common::enemy(100_000, common::hold(x, y), json!([attack, back.clone()]));
    let shooter = common::enemy(
        100_000,
        common::hold(x, y),
        json!([volley(shot), back.clone()]),
    );
    let line_shooter = common::enemy(
        100_000,
        common::hold(x, y),
        json!([volley(line_shot), back]),
    );
    let mut waves = vec![common::wave(
        1,
        PICKS,
        6,
        common::enemy(1, common::hold(START_X, NEAR_Y), json!([])),
        START_X,
        NEAR_Y,
    )];
    // The shooters arrive first, so that their runs take their tokens and attack tasks before
    // the crowd's swings fill the 256; then 496 enemies in five streams of waves.
    // The one that lays the line arrives on tick 2, before the companions' cleaves fill the free
    // zone place (they place again every 30 ticks); the other three arrive on `WAVES_AT`, at the
    // stack.
    // It spawns in view of the main character's start (RA1b, DS82), so that RA1's off-view rule
    // lets its ranged attack start on arrival; its rules send it back to the stack as the others.
    let (line_x, line_y) = (START_X + LINE_SHOOTER_DX, common::START_Y);
    waves.push(common::wave(
        LINE_SHOOTER_AT,
        1,
        1,
        line_shooter,
        line_x,
        line_y,
    ));
    waves.push(common::wave(WAVES_AT, SHOOTERS - 1, 1, shooter, x, y));
    // 32 of the crowd are elites (`MAX_ELITES_ALIVE`): the crowd's own enemy with an elite's
    // mark, in three kinds, so that their rules and every other cap are as they were. Each of
    // the first three streams of 100 starts with its share of elites and goes on with the
    // crowd, so five enemies still arrive a tick and the stack is parted as before.
    let streams = even_split(u32::from(super::enemy_cap()) - u32::from(SHOOTERS), 5);
    for (i, base) in ELITE_BASES.iter().enumerate() {
        let mut elite = enemy.clone();
        elite["base"] = json!(base);
        elite["elite"] = json!({ "range": ELITE_RANGE });
        let share = MAX_ELITES_ALIVE / 3 + u16::from(i < usize::from(MAX_ELITES_ALIVE % 3));
        waves.push(common::wave(WAVES_AT + 4, share, 1, elite, x, y));
        let after = WAVES_AT + 4 + u32::from(share);
        waves.push(common::wave(
            after,
            streams[i] - share,
            1,
            enemy.clone(),
            x,
            y,
        ));
    }
    waves.push(common::wave(
        WAVES_AT + 4,
        streams[3],
        1,
        enemy.clone(),
        x,
        y,
    ));
    waves.push(common::wave(WAVES_AT + 4, streams[4], 1, enemy, x, y));
    let zone = |radius: i32| {
        json!({ "type": "zone", "shape": { "type": "circle", "radius": radius },
                "wind_up_ticks": 0, "active_ticks": 600 })
    };
    let cleave = json!([{ "kind": "active", "ticks": 30, "damage": 1, "rehit_ticks": 1,
                          "hit_box": { "type": "arc", "offset": { "forward": 10_000 },
                                       "radius": 65_536, "half_angle": 90 * 256 },
                          "spawn": [{ "type": "ring", "count": 4, "body": [zone(25_600)] }] }]);
    // 111 zones: the shooter's line takes the 128th place (111 + 16 friendly + 1).
    let corner = json!([{ "kind": "active", "ticks": 1, "spawn": [
        { "type": "ring", "count": 56, "body": [zone(2_048)] },
        { "type": "ring", "count": 55, "body": [zone(2_048)] },
    ] }]);
    let mut boss = common::far_boss();
    boss["phases"] = json!([{ "timeline": [
        { "type": "attack", "attack": { "inline": corner } },
    ] }]);
    let guard = common::companion(
        50,
        common::hold(x + 20_000, y),
        json!([common::rule(
            common::always(),
            json!("nearest_enemy"),
            json!({ "type": "attack", "attack": { "inline": cleave } }),
            0
        )]),
    );
    // The Main weapon's step: a circle of 256 pixels round the main character, active for two
    // ticks and hitting every tick, so that a tap every other tick lands on every enemy in it,
    // every tick. Held, the charge lets go with the same step.
    let step = json!([{ "kind": "active", "ticks": 2, "damage": 1, "rehit_ticks": 1,
                        "hit_box": { "type": "circle", "radius": 65_536 } }]);
    let released = json!({ "inline": [{ "kind": "active", "ticks": 2, "damage": 1,
        "rehit_ticks": 1, "hit_box": { "type": "circle", "radius": 65_536 } }] });
    let charged = json!({ "attack": { "inline": step }, "charge": {
        "levels": [{ "at_ticks": CHARGE_TICKS, "attack": released }], "max_ticks": 600 } });
    let mut main = common::weapon_combo(json!({ "main": charged }));
    main["inline"]["assist_range"] = json!(MAX_ASSIST_RANGE);
    let mut stage = common::armed(
        common::stage_full(18_000, Some(boss), waves, vec![guard; 4]),
        main,
    );
    stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    stage.stars = Some(
        serde_json::from_value(json!({
            "drop_per_mille": 1000, "pity": 1, "cap": 8, "pickup_radius": 12_288,
            "rerolls": 0, "banishes": 0,
            "tiers": [{ "from_level": 1, "weights": { "common": 1 } }],
        }))
        .unwrap(),
    );
    stage
}

/// `total` in `parts` nearly equal shares, the larger ones first (the shares sum to `total`).
fn even_split(total: u32, parts: u32) -> Vec<u16> {
    (0..parts)
        .map(|i| u16::try_from(total / parts + u32::from(i < total % parts)).expect("a wave"))
        .collect()
}

/// The enemies that send orbs: `MAX_RANGED_ATTACKERS`, each with its eight (`MAX_ORBS_PER_OWNER`),
/// 32 in all (`MAX_ORBS_ALIVE`).
const SHOOTERS: u16 = 4;

/// The sprites the 32 elites are drawn as, a share each.
const ELITE_BASES: [&str; 3] = ["bat", "fairy_a", "horns_2"];

/// How near the main character must come for an elite's rules to fire: the bundled elites'
/// (a placeholder; the main character stands 36,000 units away, so they fire on every tick).
const ELITE_RANGE: i32 = 51_200;

/// The first enemies the main character kills, for the stars that give it every rune and Rune
/// Haste: seven runes, then the Rune Haste reward.
const PICKS: u16 = 8;

/// The run's content: the random rune and Rune Haste of 300 percent (`DEFAULT_MAX_RUNE_HASTE_PCT`) to
/// pick from, the bundled on-hit runes, and damage-over-time runes that stack to the cap
/// (`MAX_DOT_STACKS`), last for 600 ticks and tick every 4, which full Rune Haste makes every
/// tick.
pub fn content() -> RunContent {
    let haste = json!({ "id": HASTE, "rarity": "common", "effect": "rune_haste",
                        "amount": 300, "limit": 2, "title": "Haste", "description": "" });
    let mut file: Value =
        serde_json::from_slice(ContentFiles::bundled_file("runes")).expect("the bundled runes");
    let dots: Vec<Value> = ["bleed", "poison", "burn"]
        .into_iter()
        .map(|r| on_hit::dot(r, 4, 600, 1, MAX_DOT_STACKS))
        .collect();
    file["damage_over_time"] = json!(dots);
    let runes =
        Runes::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the worst-case runes");
    on_hit::content(vec![on_hit::random_rune(), haste], None, runes)
}

/// The input of tick `tick` (from 1) once the runes are taken: the main character walks left
/// and up to stand about 36,000 units from the enemies, taps its Main weapon every other tick, and
/// from 24 ticks before the measured tick holds it, charging, to let go on the measured tick;
/// from then on it taps again, with aim assist and auto mode on.
pub fn input(tick: u32) -> Input {
    let after_warmup = tick > WARMUP_TICKS;
    let walked = tick - WALK_FROM.min(tick);
    let charging = tick > WARMUP_TICKS - CHARGE_TICKS && !after_warmup;
    let tapping = tick >= TAP_FROM && !charging && tick % 2 == 0;
    Input {
        main: charging || tapping,
        assist: after_warmup,
        auto: after_warmup,
        ..if walked == 0 {
            common::idle()
        } else if walked <= WALK_LEFT_TICKS {
            common::moving(-127, -127)
        } else if walked <= WALK_LEFT_TICKS + WALK_UP_TICKS {
            common::moving(0, -127)
        } else {
            common::idle()
        }
    }
}

/// An engine `WARMUP_TICKS` ticks into the run: the next `step` is the worst-case tick. The
/// main character takes every rune and Rune Haste first, from the stars of the eight enemies
/// that stand in front of it, as a run does (`on_hit::take_all`).
pub fn warmed_up() -> Engine {
    let mut engine = on_hit::with(&stage(), &content());
    on_hit::take_all(&mut engine);
    on_hit::take(&mut engine, HASTE);
    assert!(
        engine.tick() < WALK_FROM,
        "the runes were taken by tick {}",
        engine.tick()
    );
    for tick in engine.tick() + 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}
