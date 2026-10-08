//! Bosses (ADR-036 tier 2): phases that only move forward, transitions, timelines, breakable
//! parts and the boss's rules. Small stages whose every tick can be worked out by hand; the
//! boss stands at (46080, 60000), straight above the main character, with a hurt circle of
//! radius 8192, and the main character is at (46080, 147456).

mod common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::event::DomainEvent;
use stella_rain_core::id::EntityId;
use stella_rain_core::input::Input;
use stella_rain_core::snapshot::EntityKind;
use stella_rain_core::stage::Stage;

// --- Builders --------------------------------------------------------------------------------

const BOSS_X: i32 = 46_080;
const BOSS_Y: i32 = 60_000;
/// The boss's entity ID; the main character is 0.
const BOSS: EntityId = EntityId(1);

fn boss(hp: u32, parts: Vec<Value>, phases: Vec<Value>) -> Value {
    json!({ "base": "golem", "hp": hp, "radius": 8192, "spawn": { "x": BOSS_X, "y": BOSS_Y },
            "parts": parts, "phases": phases })
}

/// A part `ox`, `oy` from the boss's centre.
fn part(id: &str, hp: u32, ox: i32, oy: i32, radius: i32) -> Value {
    json!({ "id": id, "asset": "horns_2", "hp": hp, "offset": { "x": ox, "y": oy },
            "radius": radius })
}

/// A phase that ends when `until` holds (absent for the last phase).
fn phase(until: Option<Value>, transition: Value, timeline: Value, rules: Value) -> Value {
    let mut p = json!({ "transition": transition, "timeline": timeline, "rules": rules });
    if let Some(u) = until {
        p["until"] = u;
    }
    p
}

fn time_above(ticks: u32) -> Value {
    json!({ "type": "time_above", "ticks": ticks })
}

fn part_broken(part: &str) -> Value {
    json!({ "type": "part_broken", "part": part })
}

fn step_attack(preset: &str, arg: i32) -> Value {
    json!({ "type": "attack", "attack": { "preset": { "id": preset, "args": [arg] } } })
}

fn step_wait(ticks: u32) -> Value {
    json!({ "type": "wait", "ticks": ticks })
}

fn step_telegraph(ticks: u32) -> Value {
    json!({ "type": "telegraph", "ticks": ticks })
}

fn step_move(x: i32, y: i32, ticks: u32) -> Value {
    json!({ "type": "move_to", "to": { "x": x, "y": y }, "ticks": ticks })
}

fn step_loop() -> Value {
    json!({ "type": "loop" })
}

fn stage_of(boss: Value) -> Stage {
    stage_full(18_000, Some(boss), vec![], vec![], vec![])
}

fn play(e: &mut Engine, ticks: u32, input: Input) -> Vec<(u32, DomainEvent)> {
    run_until(e, input, ticks, |_| false)
}

fn phase_changes(events: &[(u32, DomainEvent)]) -> Vec<(u32, u8)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::PhaseChanged { phase } => Some((*t, *phase)),
            _ => None,
        })
        .collect()
}

fn telegraphs(events: &[(u32, DomainEvent)]) -> Vec<(u32, u32)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::Telegraph { source, ticks } if *source == BOSS => Some((*t, *ticks)),
            _ => None,
        })
        .collect()
}

/// `(tick, target, damage)` of every hit by the main character's shots.
fn hits(events: &[(u32, DomainEvent)]) -> Vec<(u32, EntityId, u32)> {
    events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::Hit {
                source,
                target,
                damage,
            } if *source == EntityId(0) => Some((*t, *target, *damage)),
            _ => None,
        })
        .collect()
}

fn hostile_bullets(e: &Engine) -> usize {
    e.snapshot().bullets.iter().filter(|b| !b.friendly).count()
}

fn friendly_bullets(e: &Engine) -> usize {
    e.snapshot().bullets.iter().filter(|b| b.friendly).count()
}

fn boss_at(e: &Engine) -> (i32, i32) {
    let s = e.snapshot();
    let b = s
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Boss)
        .expect("the boss is there");
    (b.at.x.0, b.at.y.0)
}

fn boss_hp(e: &Engine) -> u32 {
    let s = e.snapshot();
    s.entities
        .iter()
        .find(|v| v.kind == EntityKind::Boss)
        .map_or(0, |b| b.hp)
}

// --- Phases ----------------------------------------------------------------------------------

#[test]
fn a_phase_ends_when_its_condition_holds_and_the_boss_goes_on_to_the_next() {
    // Phase time is 0 on the first tick, so `time_above 9` first holds on tick 10.
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![
            phase(Some(time_above(9)), json!([]), json!([]), json!([])),
            phase(None, json!([]), json!([]), json!([])),
        ],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 40, idle());
    // Phase 0 is where the boss starts: it is not announced.
    assert_eq!(phase_changes(&events), vec![(10, 1)]);
}

#[test]
fn phases_only_move_forward_one_a_tick_and_the_last_has_no_end() {
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![
            phase(
                Some(json!({ "type": "always" })),
                json!([]),
                json!([]),
                json!([]),
            ),
            phase(
                Some(json!({ "type": "always" })),
                json!([]),
                json!([]),
                json!([]),
            ),
            phase(None, json!([]), json!([]), json!([])),
        ],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 30, idle());
    assert_eq!(phase_changes(&events), vec![(1, 1), (2, 2)]);
}

#[test]
fn a_phase_can_end_on_the_boss_hp() {
    // The boss has 100 hp; the main character's shots hit it for 10. The phase ends on the tick
    // after the hit that leaves it below half.
    let stage = stage_of(boss(
        100,
        vec![],
        vec![
            phase(
                Some(json!({ "type": "hp_below", "who": "self", "pct": 50 })),
                json!([]),
                json!([]),
                json!([]),
            ),
            phase(None, json!([]), json!([]), json!([])),
        ],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 200, firing());
    let hit_ticks: Vec<(u32, u32)> = hits(&events)
        .iter()
        .map(|(t, _, damage)| (*t, *damage))
        .collect();
    let mut hp = 100;
    let mut below = None;
    for (t, damage) in &hit_ticks {
        hp -= damage;
        if hp < 50 && below.is_none() {
            below = Some(*t);
        }
    }
    let below = below.expect("the shots take the boss below half");
    assert_eq!(phase_changes(&events), vec![(below + 1, 1)]);
}

#[test]
fn a_phase_can_end_when_a_part_breaks() {
    let stage = stage_of(boss(
        1_000_000,
        vec![part("horn", 20, 0, 16_000, 6_000)],
        vec![
            phase(Some(part_broken("horn")), json!([]), json!([]), json!([])),
            phase(None, json!([]), json!([]), json!([])),
        ],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 200, firing());
    let broken: Vec<u32> = ticks_of(
        &events,
        |ev| matches!(ev, DomainEvent::PartBroken { part } if part.0 == "horn"),
    );
    assert_eq!(broken.len(), 1, "the part breaks once");
    assert_eq!(phase_changes(&events), vec![(broken[0] + 1, 1)]);
}

#[test]
fn a_phase_has_its_own_clock_and_its_own_rule_states() {
    // The rule fires once, when the phase is 5 ticks old. The same rule in phase 1 fires again,
    // 5 ticks after that phase began (phase 1 begins on tick 20).
    let once = |_n: u32| {
        json!([{ "when": time_above(4), "target": "self", "once": true,
                 "do": { "type": "telegraph", "ticks": 1 }, "cooldown_ticks": 0 }])
    };
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![
            phase(Some(time_above(19)), json!([]), json!([]), once(0)),
            phase(None, json!([]), json!([]), once(1)),
        ],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 40, idle());
    assert_eq!(phase_changes(&events), vec![(20, 1)]);
    // Phase age 5 is tick 5 in phase 0 and tick 24 in phase 1 (which began on tick 20).
    let fired: Vec<u32> = ticks_of(
        &events,
        |ev| matches!(ev, DomainEvent::RuleFired { agent, .. } if *agent == BOSS),
    );
    assert_eq!(fired, vec![5, 24]);
}

// --- Transitions -----------------------------------------------------------------------------

#[test]
fn clear_bullets_removes_the_hostile_bullets_and_keeps_the_players() {
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![
            phase(
                Some(time_above(29)),
                json!([]),
                json!([step_attack("aimed_single", 768), step_wait(5), step_loop()]),
                json!([]),
            ),
            phase(
                None,
                json!([{ "type": "clear_bullets" }]),
                json!([]),
                json!([]),
            ),
        ],
    ));
    let mut e = engine(&stage);
    play(&mut e, 29, firing());
    assert!(hostile_bullets(&e) > 0, "the boss has been shooting");
    let friendly = friendly_bullets(&e);
    assert!(friendly > 0, "so has the main character");
    // Tick 30 begins phase 1.
    e.step(firing());
    assert_eq!(e.tick(), 30);
    assert_eq!(hostile_bullets(&e), 0);
    assert!(friendly_bullets(&e) > 0);
    assert!(friendly_bullets(&e) <= friendly + 2);
}

#[test]
fn invulnerable_ticks_use_up_the_shots_that_land_for_exactly_that_long() {
    // Phase 1 begins on tick `from` and the boss cannot be hurt for 10 ticks, `from` to
    // `from + 9`. Phase 0 ends on `time_above(from - 1)`. The same shots land on the same ticks
    // as without the shield, except those in the window; windows that begin and end on a tick
    // where a shot lands test both edges.
    let build = |from: u32, transition: Value| {
        stage_of(boss(
            1_000_000,
            vec![],
            vec![
                phase(Some(time_above(from - 1)), json!([]), json!([]), json!([])),
                phase(None, transition, json!([]), json!([])),
            ],
        ))
    };
    let run = |stage: &Stage| hits(&play(&mut engine(stage), 140, firing()));
    let open = run(&build(20, json!([])));
    let landing: Vec<u32> = open
        .iter()
        .map(|(t, ..)| *t)
        .filter(|t| *t >= 15)
        .take(3)
        .collect();
    assert_eq!(landing.len(), 3, "shots land on the boss");
    let shield = json!([{ "type": "invulnerable_ticks", "ticks": 10 }]);
    for h in landing {
        // The window starts on the landing tick, and ends on it.
        for from in [h, h - 9] {
            let shielded = run(&build(from, shield.clone()));
            let expected: Vec<_> = run(&build(from, json!([])))
                .into_iter()
                .filter(|(t, ..)| !(from..from + 10).contains(t))
                .collect();
            assert!(
                expected.len() < open.len(),
                "the window takes a shot (from {from})"
            );
            assert_eq!(shielded, expected, "window {from} to {}", from + 9);
        }
    }
}

#[test]
fn an_invulnerable_boss_also_shields_its_parts() {
    let build = |transition: Value| {
        stage_of(boss(
            1_000_000,
            vec![part("horn", 100_000, 0, 16_000, 6_000)],
            vec![phase(None, transition, json!([]), json!([]))],
        ))
    };
    let run = |stage: &Stage| hits(&play(&mut engine(stage), 100, firing()));
    let open = run(&build(json!([])));
    let shielded = run(&build(
        json!([{ "type": "invulnerable_ticks", "ticks": 40 }]),
    ));
    assert!(open.iter().any(|(t, ..)| *t <= 40));
    // From tick 1 to 40 nothing lands; afterwards the same shots land on the same ticks.
    let expected: Vec<_> = open.iter().filter(|(t, ..)| *t > 40).copied().collect();
    assert_eq!(shielded, expected);
}

// --- Timelines -------------------------------------------------------------------------------

#[test]
fn a_timeline_waits_telegraphs_and_moves_on_the_ticks_it_says() {
    // Tick 1: wait 5, so the telegraph is on tick 6; it lasts 3, so the move begins on tick 9
    // and takes 4 ticks, 10 to 13, landing exactly on the point (a step of 100, 50 a tick).
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![phase(
            None,
            json!([]),
            json!([
                step_wait(5),
                step_telegraph(3),
                step_move(BOSS_X + 400, BOSS_Y + 200, 4)
            ]),
            json!([]),
        )],
    ));
    let mut e = engine(&stage);
    let mut at = Vec::new();
    let mut events = Vec::new();
    for _ in 0..16 {
        events.extend(run_until(&mut e, idle(), 1, |_| false));
        at.push(boss_at(&e));
    }
    assert_eq!(telegraphs(&events), vec![(6, 3)]);
    for (i, p) in at.iter().enumerate() {
        let tick = i as i32 + 1;
        let moved = (tick - 9).clamp(0, 4);
        assert_eq!(
            *p,
            (BOSS_X + 100 * moved, BOSS_Y + 50 * moved),
            "tick {tick}"
        );
    }
}

#[test]
fn a_loop_starts_the_timeline_again() {
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![phase(
            None,
            json!([]),
            json!([step_telegraph(10), step_loop()]),
            json!([]),
        )],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 35, idle());
    assert_eq!(
        telegraphs(&events),
        vec![(1, 10), (11, 10), (21, 10), (31, 10)]
    );
}

#[test]
fn a_timeline_that_ends_stops_and_the_boss_waits_for_its_rules() {
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![phase(
            None,
            json!([]),
            json!([step_telegraph(2)]),
            json!([]),
        )],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 20, idle());
    assert_eq!(telegraphs(&events), vec![(1, 2)]);
}

#[test]
fn a_timeline_takes_a_bounded_number_of_steps_a_tick() {
    // Nothing in `[attack, loop]` ever waits: the boss takes 16 steps, 8 attacks, a tick.
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![phase(
            None,
            json!([]),
            json!([step_attack("aimed_single", 256), step_loop()]),
            json!([]),
        )],
    ));
    let mut e = engine(&stage);
    e.step(idle());
    assert_eq!(hostile_bullets(&e), 8);
    e.step(idle());
    assert_eq!(hostile_bullets(&e), 16);
}

#[test]
fn a_timeline_attack_is_aimed_at_the_main_character() {
    // The boss is straight above the main character: an aimed bullet only moves down.
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![phase(
            None,
            json!([]),
            json!([step_attack("aimed_single", 768)]),
            json!([]),
        )],
    ));
    let mut e = engine(&stage);
    e.step(idle());
    let first = e.snapshot().bullets[0].at;
    e.step(idle());
    let second = e.snapshot().bullets[0].at;
    assert_eq!(second.x, first.x);
    assert_eq!(second.y.0 - first.y.0, 768);
    assert_eq!(first.x.0, BOSS_X);
}

#[test]
fn the_end_of_a_phase_ends_the_attacks_the_boss_was_making() {
    // A spiral shoots every 3 ticks for 120 ticks. Phase 1 begins on tick 21; no bullet is
    // fired after that, though the ones in the air fly on.
    let build = |until: Option<Value>| {
        stage_of(boss(
            1_000_000,
            vec![],
            vec![
                phase(
                    until,
                    json!([]),
                    json!([step_attack("spiral", 256)]),
                    json!([]),
                ),
                phase(None, json!([]), json!([]), json!([])),
            ],
        ))
    };
    let mut cut = engine(&build(Some(time_above(19))));
    let mut open = engine(&build(Some(
        json!({ "type": "hp_below", "who": "self", "pct": 0 }),
    )));
    let mut counts = Vec::new();
    for _ in 0..60 {
        cut.step(idle());
        open.step(idle());
        counts.push((hostile_bullets(&cut), hostile_bullets(&open)));
    }
    // Phase 0 ran for ticks 1 to 20: a bullet on ticks 1, 4, ..., 19, so seven.
    assert_eq!(counts[19].0, 7);
    assert_eq!(counts[59].0, 7);
    assert_eq!(counts[59].1, 20);
}

// --- Parts -----------------------------------------------------------------------------------

#[test]
fn parts_have_ids_after_the_companions_and_come_before_the_enemies_in_the_snapshot() {
    let stage = stage_full(
        600,
        Some(boss(
            1_000_000,
            vec![
                part("left", 50, -6_000, 12_000, 2_048),
                part("right", 60, 6_000, 12_000, 2_048),
            ],
            vec![phase(None, json!([]), json!([]), json!([]))],
        )),
        vec![wave(
            1,
            1,
            0,
            enemy(10, hold(10_000, 10_000), json!([])),
            10_000,
            10_000,
        )],
        vec![companion(10, hold(30_000, 140_000), json!([]))],
        vec![],
    );
    let mut e = engine(&stage);
    e.step(idle());
    let s = e.snapshot();
    let layout: Vec<(u32, EntityKind, u32)> =
        s.entities.iter().map(|v| (v.id.0, v.kind, v.hp)).collect();
    assert_eq!(
        layout,
        vec![
            (1, EntityKind::Boss, 1_000_000),
            (2, EntityKind::Companion, 10),
            (3, EntityKind::BossPart, 50),
            (4, EntityKind::BossPart, 60),
            (5, EntityKind::Enemy, 10),
        ]
    );
    let left = &s.entities[2];
    assert_eq!(
        (left.at.x.0, left.at.y.0),
        (BOSS_X - 6_000, BOSS_Y + 12_000)
    );
}

#[test]
fn a_part_has_its_own_hp_and_a_shot_that_hits_it_hurts_only_it() {
    // The horn is in front of the body and takes the shots until it breaks (20 hp, 10 a hit).
    let stage = stage_of(boss(
        1_000_000,
        vec![part("horn", 20, 0, 16_000, 6_000)],
        vec![phase(None, json!([]), json!([]), json!([]))],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 200, firing());
    let hit = hits(&events);
    let horn = EntityId(2);
    let on_horn: Vec<_> = hit.iter().filter(|(_, t, _)| *t == horn).collect();
    assert_eq!(on_horn.len(), 2, "two hits break the horn");
    let broken = ticks_of(&events, |ev| matches!(ev, DomainEvent::PartBroken { .. }));
    assert_eq!(broken, vec![on_horn[1].0]);
    // Everything before that landed on the horn; afterwards on the body.
    assert!(hit.iter().take(2).all(|(_, t, _)| *t == horn));
    assert!(hit.iter().skip(2).all(|(_, t, _)| *t == BOSS));
    assert!(hit.len() > 2, "the body is hit once the horn is gone");
    // A broken part is not a death, and is gone from the snapshot.
    assert!(!events.iter().any(|(_, ev)| matches!(
        ev,
        DomainEvent::Died { entity } if *entity == horn
    )));
    assert!(
        e.snapshot()
            .entities
            .iter()
            .all(|v| v.kind != EntityKind::BossPart)
    );
}

#[test]
fn a_part_that_is_missed_leaves_the_body_to_be_hit_and_the_part_whole() {
    // The horn is to the side of the shots: the body is hit, the horn is not.
    let stage = stage_of(boss(
        1_000_000,
        vec![part("horn", 20, 30_000, 0, 2_048)],
        vec![phase(None, json!([]), json!([]), json!([]))],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 100, firing());
    let hit = hits(&events);
    assert!(!hit.is_empty());
    assert!(hit.iter().all(|(_, t, _)| *t == BOSS));
    let s = e.snapshot();
    assert!(
        s.entities
            .iter()
            .any(|v| v.kind == EntityKind::BossPart && v.hp == 20)
    );
}

#[test]
fn parts_move_with_the_boss() {
    let stage = stage_of(boss(
        1_000_000,
        vec![part("horn", 20, 5_000, -3_000, 2_048)],
        vec![phase(
            None,
            json!([]),
            json!([step_move(BOSS_X - 2_000, BOSS_Y + 4_000, 2)]),
            json!([]),
        )],
    ));
    let mut e = engine(&stage);
    for _ in 0..4 {
        e.step(idle());
    }
    let s = e.snapshot();
    let b = s
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::Boss)
        .unwrap();
    let p = s
        .entities
        .iter()
        .find(|v| v.kind == EntityKind::BossPart)
        .unwrap();
    assert_eq!((b.at.x.0, b.at.y.0), (BOSS_X - 2_000, BOSS_Y + 4_000));
    assert_eq!(
        (p.at.x.0 - b.at.x.0, p.at.y.0 - b.at.y.0),
        (5_000, -3_000),
        "the part keeps its offset"
    );
}

#[test]
fn the_part_selector_picks_a_part_that_is_still_whole() {
    // A rule of the boss aims at the horn every tick until it breaks.
    let rule = json!([{ "when": { "type": "always" }, "target": { "part": "horn" },
                        "do": { "type": "telegraph", "ticks": 1 }, "cooldown_ticks": 0 }]);
    let stage = stage_of(boss(
        1_000_000,
        vec![part("horn", 20, 0, 16_000, 6_000)],
        vec![phase(None, json!([]), json!([]), rule)],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 120, firing());
    let broken = ticks_of(&events, |ev| matches!(ev, DomainEvent::PartBroken { .. }))[0];
    let fired: Vec<(u32, Option<EntityId>)> = events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::RuleFired { agent, target, .. } if *agent == BOSS => Some((*t, *target)),
            _ => None,
        })
        .collect();
    // It fires on every tick, at the horn, up to and including the tick it breaks: the boss
    // acts before the shots land.
    assert_eq!(fired.len() as u32, broken);
    assert!(fired.iter().all(|(_, t)| *t == Some(EntityId(2))));
}

#[test]
fn hp_below_can_ask_about_a_part() {
    let stage = stage_of(boss(
        1_000_000,
        vec![part("horn", 30, 0, 16_000, 6_000)],
        vec![
            phase(
                Some(json!({ "type": "hp_below", "who": { "part": "horn" }, "pct": 50 })),
                json!([]),
                json!([]),
                json!([]),
            ),
            phase(None, json!([]), json!([]), json!([])),
        ],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 120, firing());
    // 30 hp, 10 a hit: below half (15) after the second hit.
    let second = hits(&events)[1].0;
    assert_eq!(phase_changes(&events), vec![(second + 1, 1)]);
}

// --- The boss's rules ------------------------------------------------------------------------

#[test]
fn a_boss_rule_shoots_summons_and_moves_like_an_agent_would() {
    let rules = json!([
        { "when": time_above(2), "target": "self", "once": true,
          "do": { "type": "move_to", "to": { "x": BOSS_X + 3_000, "y": BOSS_Y }, "ticks": 3 },
          "cooldown_ticks": 0 },
        { "when": time_above(20), "target": "player", "once": true,
          "do": { "type": "summon", "count": 2,
                  "agent": enemy(10, hold(10_000, 30_000), json!([])) },
          "cooldown_ticks": 0 },
        { "when": { "type": "always" }, "target": "player", "cooldown_ticks": 10,
          "do": { "type": "attack", "attack": { "preset": { "id": "aimed_single", "args": [256] } } } },
    ]);
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![phase(None, json!([]), json!([]), rules)],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 22, idle());
    // The first rule that can fire does. The shot (rule 2) is ready on ticks 1, 11 and 21, but
    // on tick 21 the summon (rule 1) comes first and the shot waits for tick 22; the move
    // (rule 0) is allowed once the phase is 3 ticks old.
    let fired: Vec<(u32, u8)> = events
        .iter()
        .filter_map(|(t, ev)| match ev {
            DomainEvent::RuleFired { agent, rule, .. } if *agent == BOSS => Some((*t, *rule)),
            _ => None,
        })
        .collect();
    assert_eq!(fired, vec![(1, 2), (3, 0), (11, 2), (21, 1), (22, 2)]);
    // The move took ticks 4 to 6, 1000 a tick.
    assert_eq!(boss_at(&e), (BOSS_X + 3_000, BOSS_Y));
    // Two summoned enemies, entities 2 and 3, went to their post.
    let s = e.snapshot();
    let helpers: Vec<(u32, i32, i32)> = s
        .entities
        .iter()
        .filter(|v| v.kind == EntityKind::Enemy)
        .map(|v| (v.id.0, v.at.x.0, v.at.y.0))
        .collect();
    assert_eq!(helpers, vec![(2, 10_000, 30_000), (3, 10_000, 30_000)]);
}

#[test]
fn the_boss_acts_before_the_companions_and_the_enemies() {
    let all = json!([{ "when": { "type": "always" }, "target": "player",
                       "do": { "type": "telegraph", "ticks": 1 }, "cooldown_ticks": 0 }]);
    let stage = stage_full(
        600,
        Some(boss(
            1_000_000,
            vec![],
            vec![phase(None, json!([]), json!([]), all.clone())],
        )),
        vec![wave(
            1,
            1,
            0,
            enemy(10, hold(10_000, 10_000), all.clone()),
            10_000,
            10_000,
        )],
        vec![companion(10, hold(30_000, 140_000), all)],
        vec![],
    );
    let mut e = engine(&stage);
    play(&mut e, 2, idle());
    e.step(idle());
    let order: Vec<u32> = e
        .events()
        .iter()
        .filter_map(|ev| match ev {
            DomainEvent::RuleFired { agent, .. } => Some(agent.0),
            _ => None,
        })
        .collect();
    // The boss is 1, the companion 2 and the enemy 3.
    assert_eq!(order, vec![1, 2, 3]);
}

// --- The end ---------------------------------------------------------------------------------

#[test]
fn beating_the_boss_in_an_early_phase_clears_the_stage() {
    let stage = stage_of(boss(
        20,
        vec![],
        vec![
            phase(Some(time_above(1_000)), json!([]), json!([]), json!([])),
            phase(None, json!([]), json!([]), json!([])),
        ],
    ));
    let mut e = engine(&stage);
    let events = play(&mut e, 200, firing());
    assert_eq!(e.outcome(), Outcome::Cleared);
    assert!(
        events
            .iter()
            .any(|(_, ev)| matches!(ev, DomainEvent::StageCleared))
    );
    assert_eq!(boss_hp(&e), 0);
    assert!(phase_changes(&events).is_empty());
}

#[test]
fn a_boss_run_is_the_same_every_time() {
    let stage = stage_of(boss(
        300,
        vec![part("horn", 30, 0, 16_000, 6_000)],
        vec![
            phase(
                Some(part_broken("horn")),
                json!([]),
                json!([
                    step_telegraph(5),
                    step_attack("spiral", 256),
                    step_move(30_000, 50_000, 20),
                    step_loop()
                ]),
                json!([]),
            ),
            phase(
                None,
                json!([{ "type": "clear_bullets" }, { "type": "invulnerable_ticks", "ticks": 10 }]),
                json!([
                    json!({ "type": "attack", "attack": { "preset": { "id": "spread_5", "args": [] } } }),
                    step_wait(15),
                    step_loop()
                ]),
                json!([]),
            ),
        ],
    ));
    let run = || {
        let mut e = engine(&stage);
        (0..400)
            .map(|t| {
                e.step(if t % 7 < 5 { firing() } else { idle() });
                e.state_hash()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}

#[test]
fn the_end_of_a_phase_ends_a_move_that_was_under_way() {
    // Phase 0 sends the boss 20000 to the right over 40 ticks but lasts 10; the next phase has
    // no orders, so the boss stays where it was after tick 10 (a quarter of the way).
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![
            phase(
                Some(time_above(9)),
                json!([]),
                json!([step_move(BOSS_X + 20_000, BOSS_Y, 40)]),
                json!([]),
            ),
            phase(None, json!([]), json!([]), json!([])),
        ],
    ));
    let mut e = engine(&stage);
    for _ in 0..30 {
        e.step(idle());
    }
    // Moves on ticks 2 to 10: nine steps of 500.
    assert_eq!(boss_at(&e), (BOSS_X + 4_500, BOSS_Y));
}

#[test]
fn an_attack_started_by_a_rule_runs_on_the_phase_clock() {
    // `phase_time` is 0 on the first tick of a phase. Phase 1 begins on tick 11; its rule fires
    // when the phase is 6 ticks old, on tick 16, at phase time 5: the bullet's speed is
    // 100 + 5 * 10 (aimed straight down).
    let speed = json!({ "add": [100, { "mul": ["phase_time", 10] }] });
    let attack = json!({ "inline": [{ "type": "fire", "speed": speed,
                                      "direction": { "type": "absolute", "angle": 0 } }] });
    let rules = json!([{ "when": time_above(5), "target": "player", "once": true,
                         "do": { "type": "attack", "attack": attack }, "cooldown_ticks": 0 }]);
    let stage = stage_of(boss(
        1_000_000,
        vec![],
        vec![
            phase(Some(time_above(10)), json!([]), json!([]), json!([])),
            phase(None, json!([]), json!([]), rules),
        ],
    ));
    let mut e = engine(&stage);
    for _ in 0..16 {
        e.step(idle());
    }
    assert_eq!(hostile_bullets(&e), 1);
    let before = e.snapshot().bullets[0].at.y.0;
    e.step(idle());
    assert_eq!(e.snapshot().bullets[0].at.y.0 - before, 150);
}
