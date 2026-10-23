//! The Main weapon's combos and the Sub weapon's pull (ADR-070, ADR-076), made by hand. Small
//! stages whose every tick can be worked out by hand: the main character stands at (46080,
//! 147456), facing up the field.
//!
//! The test weapon's steps are all the same swing: a wind-up of 2 ticks, an active phase of 2
//! that fires one bullet up the field on its first tick, and a recovery of 6, 10 ticks in all;
//! each step's bullet has its own speed, so the bullets say which steps ran. A step pressed on
//! tick t winds up on t and t + 1, fires on t + 2 and recovers from t + 4 to t + 9.

use crate::common;

use common::*;
use serde_json::{Value, json};
use stella_rain_core::attack::{AttackRef, DamageType, HitBox, PhaseKind};
use stella_rain_core::engine::Engine;
use stella_rain_core::event::{DomainEvent, Press};
use stella_rain_core::input::Input;
use stella_rain_core::presets;
use stella_rain_core::registry::{self, Kind};
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::validate_weapon_preset;
use stella_rain_core::weapon::{Branches, Step, WeaponRef};
use stella_rain_core::weapons::Weapons;

// --- Builders --------------------------------------------------------------------------------

/// The steps, by the speed of their bullet: A, then AA, then AAA (a leaf); AB after A; B from
/// the root; D, the dash-strike.
const A: i32 = 256;
const AA: i32 = 512;
const AAA: i32 = 768;
const AB: i32 = 1024;
const B: i32 = 1280;
const D: i32 = 1536;

const WINDOW: u32 = 10;
const DASH_STRIKE: u32 = 5;

fn swing(speed: i32) -> Value {
    json!({ "inline": [
        { "kind": "wind_up", "ticks": 2 },
        { "kind": "active", "ticks": 2, "spawn": [{ "type": "fire", "speed": speed,
            "direction": { "type": "aimed", "offset": 0 } }] },
        { "kind": "recovery", "ticks": 6 }
    ] })
}

/// The test weapon with a window of `window` ticks.
fn test_weapon(window: u32) -> Value {
    json!({ "inline": {
        "combo": {
            "main": { "attack": swing(A), "next": {
                "main": { "attack": swing(AA), "next": { "main": step(swing(AAA)) } },
                "smash": step(swing(AB)) } },
            "smash": step(swing(B))
        },
        "dash": { "main": step(swing(D)) },
        "window_ticks": window, "dash_strike_ticks": DASH_STRIKE,
        "assist_range": 0
    } })
}

fn far() -> Stage {
    stage_with(600, Some(far_boss()), vec![])
}

fn combo_stage(window: u32) -> Stage {
    armed(far(), test_weapon(window))
}

/// The inputs of ticks 1 to `ticks`: on each tick of `presses` the buttons its letters name
/// are held, for that tick only: `a` (`main`), `b` (`smash`), `x` (`evade`).
fn presses(presses: &[(u32, &str)], ticks: u32) -> Vec<Input> {
    (1..=ticks)
        .map(|t| {
            let held = presses
                .iter()
                .find(|(at, _)| *at == t)
                .map_or("", |(_, keys)| *keys);
            Input {
                main: held.contains('a'),
                smash: held.contains('b'),
                evade: held.contains('x'),
                ..idle()
            }
        })
        .collect()
}

/// What a run of `inputs` did: every event with its tick, and the speed of every friendly
/// bullet with the tick it was made.
struct Played {
    events: Vec<(u32, DomainEvent)>,
    shots: Vec<(u32, i32)>,
}

fn play(e: &mut Engine, inputs: &[Input]) -> Played {
    let mut played = Played {
        events: Vec::new(),
        shots: Vec::new(),
    };
    let mut seen = 0;
    for input in inputs {
        e.step(*input);
        played
            .events
            .extend(e.events().iter().map(|ev| (e.tick(), ev.clone())));
        let bullets: Vec<i32> = e
            .snapshot()
            .bullets
            .iter()
            .filter(|b| b.friendly)
            .map(|b| -b.vy.0)
            .collect();
        played
            .shots
            .extend(bullets[seen..].iter().map(|v| (e.tick(), *v)));
        seen = bullets.len();
    }
    played
}

impl Played {
    /// The steps taken: the tick, the press and the depth reached.
    fn steps(&self) -> Vec<(u32, Press, u8)> {
        self.events
            .iter()
            .filter_map(|(t, ev)| match ev {
                DomainEvent::ComboStep { press, depth } => Some((*t, *press, *depth)),
                _ => None,
            })
            .collect()
    }

    fn resets(&self) -> Vec<u32> {
        ticks_of(&self.events, |ev| *ev == DomainEvent::ComboReset)
    }
}

fn played(stage: &Stage, inputs: &[Input]) -> Played {
    play(&mut engine(stage), inputs)
}

// --- Steps and presses -----------------------------------------------------------------------

#[test]
fn a_press_takes_a_step_and_a_held_button_presses_once() {
    let held = vec![pressing_main(); 40];
    let p = played(&combo_stage(WINDOW), &held);
    assert_eq!(p.steps(), [(1, Press::Main, 1)]);
    assert_eq!(p.shots, [(3, A)]);
}

#[test]
fn a_buffered_press_starts_the_next_step_where_the_recovery_would_begin() {
    // Pressed again while A winds up: AA starts on tick 5 instead of A's recovery.
    let p = played(&combo_stage(WINDOW), &presses(&[(1, "a"), (3, "a")], 30));
    assert_eq!(p.steps(), [(1, Press::Main, 1), (5, Press::Main, 2)]);
    assert_eq!(p.shots, [(3, A), (7, AA)]);
}

#[test]
fn with_nothing_buffered_the_recovery_plays_and_a_press_during_it_is_taken_at_once() {
    let mut e = engine(&combo_stage(WINDOW));
    play(&mut e, &presses(&[(1, "a")], 10));
    assert_eq!(
        e.snapshot().player.attacks.len(),
        1,
        "A recovers to tick 10"
    );
    e.step(idle());
    assert!(e.snapshot().player.attacks.is_empty());

    let p = played(&combo_stage(WINDOW), &presses(&[(1, "a"), (8, "a")], 20));
    assert_eq!(p.steps(), [(1, Press::Main, 1), (8, Press::Main, 2)]);
}

#[test]
fn one_press_is_buffered_and_the_latest_wins() {
    let p = played(
        &combo_stage(WINDOW),
        &presses(&[(1, "a"), (3, "a"), (4, "b")], 20),
    );
    assert_eq!(p.steps(), [(1, Press::Main, 1), (5, Press::Smash, 2)]);
    assert_eq!(p.shots, [(3, A), (7, AB)]);
}

#[test]
fn a_and_b_on_one_tick_count_as_a() {
    let p = played(&combo_stage(WINDOW), &presses(&[(1, "a"), (3, "ab")], 20));
    assert_eq!(p.steps(), [(1, Press::Main, 1), (5, Press::Main, 2)]);
    let p = played(&combo_stage(WINDOW), &presses(&[(1, "ab")], 20));
    assert_eq!(p.steps(), [(1, Press::Main, 1)]);
}

#[test]
fn b_takes_its_own_branch_from_the_root() {
    let p = played(&combo_stage(WINDOW), &presses(&[(1, "b")], 20));
    assert_eq!(p.steps(), [(1, Press::Smash, 1)]);
    assert_eq!(p.shots, [(3, B)]);
}

#[test]
fn a_leaf_runs_to_its_end_before_a_press_starts_again_from_the_root() {
    // AAA starts on tick 9 and recovers to tick 18; the press on tick 11 waits for it.
    let p = played(
        &combo_stage(WINDOW),
        &presses(&[(1, "a"), (3, "a"), (7, "a"), (11, "a")], 30),
    );
    assert_eq!(
        p.steps(),
        [
            (1, Press::Main, 1),
            (5, Press::Main, 2),
            (9, Press::Main, 3),
            (19, Press::Main, 1)
        ]
    );
    assert_eq!(p.resets(), [19]);
    assert_eq!(p.shots, [(3, A), (7, AA), (11, AAA), (21, A)]);
}

#[test]
fn the_window_keeps_the_place_for_its_ticks_after_the_step() {
    // A ends after tick 10: a press on ticks 11 to 20 takes AA; from tick 21 the combo is back
    // at the root.
    let p = played(&combo_stage(WINDOW), &presses(&[(1, "a"), (20, "a")], 30));
    assert_eq!(p.steps(), [(1, Press::Main, 1), (20, Press::Main, 2)]);
    assert!(p.resets().is_empty());

    let p = played(&combo_stage(WINDOW), &presses(&[(1, "a"), (21, "a")], 30));
    assert_eq!(p.steps(), [(1, Press::Main, 1), (21, Press::Main, 1)]);
    assert_eq!(p.resets(), [21]);

    // The snapshot counts the window down: the ticks left after this one.
    let mut e = engine(&combo_stage(WINDOW));
    play(&mut e, &presses(&[(1, "a")], 11));
    assert_eq!(e.snapshot().player.combo.window_left, WINDOW - 1);
    play(&mut e, &presses(&[], 9));
    assert_eq!(e.snapshot().player.combo.window_left, 0);
    assert_eq!(e.snapshot().player.combo.presses, [Press::Main]);
    let p = play(&mut e, &presses(&[], 1));
    assert_eq!(p.resets(), [21]);
    assert!(e.snapshot().player.combo.presses.is_empty());
}

#[test]
fn with_a_window_of_0_the_next_press_after_the_step_starts_from_the_root() {
    // A ends after tick 10, so the press on tick 11 starts A again; that one ends after tick
    // 20, and the combo is back at the root on tick 21.
    let p = played(&combo_stage(0), &presses(&[(1, "a"), (11, "a")], 30));
    assert_eq!(p.steps(), [(1, Press::Main, 1), (11, Press::Main, 1)]);
    assert_eq!(p.resets(), [11, 21]);
}

#[test]
fn the_snapshot_shows_the_combo_and_the_buffered_press() {
    let mut e = engine(&combo_stage(WINDOW));
    play(&mut e, &presses(&[(1, "a"), (3, "b")], 3));
    let combo = e.snapshot().player.combo;
    assert_eq!(
        (combo.presses, combo.dash, combo.buffered),
        (vec![Press::Main], false, Some(Press::Smash))
    );
}

// --- The evade and the dash-strike -----------------------------------------------------------

#[test]
fn an_evade_cancels_the_step_and_clears_the_buffer() {
    // A fires on tick 3; B is buffered on tick 3; the evade on tick 4 ends A and drops B.
    let p = played(
        &combo_stage(WINDOW),
        &presses(&[(1, "a"), (3, "b"), (4, "x")], 30),
    );
    assert_eq!(p.steps(), [(1, Press::Main, 1)]);
    assert_eq!(p.resets(), [4]);
    assert_eq!(p.shots, [(3, A)]);
}

#[test]
fn a_press_during_the_dash_or_its_dash_strike_ticks_takes_the_dash_branch() {
    // The dash moves on ticks 1 to 8 (its 8 ticks); the dash-strike may start until tick 13.
    for (at, speed, dash) in [(1, D, true), (4, D, true), (13, D, true), (14, A, false)] {
        let keys: &[(u32, &str)] = if at == 1 {
            &[(1, "xa")]
        } else {
            &[(1, "x"), (at, "a")]
        };
        let mut e = engine(&combo_stage(WINDOW));
        let p = play(&mut e, &presses(keys, at + 2));
        assert_eq!(p.shots, [(at + 2, speed)], "a press on tick {at}");
        assert_eq!(e.snapshot().player.combo.dash, dash, "a press on tick {at}");
    }
    // A press with no dash step starts from the root.
    let p = played(&combo_stage(WINDOW), &presses(&[(1, "x"), (3, "b")], 10));
    assert_eq!(p.shots, [(5, B)]);
}

// --- The Sub weapon's pull -------------------------------------------------------------------

/// A blow all round, 48 pixels across, for one tick.
fn blow() -> Value {
    json!({ "inline": [{ "kind": "active", "ticks": 1,
                         "hit_box": { "type": "circle", "radius": 12_288 } }] })
}

/// A stage whose weapon has the blow on both buttons, whose smashes pull 24 pixels over 6
/// ticks, with `target` (an enemy that stands still) `dy` above the main character.
fn pulling(target: Value, dy: i32, levels: bool) -> Stage {
    let mut stage = armed(
        stage_full(
            600,
            Some(far_boss()),
            vec![wave(1, 1, 0, target, START_X, START_Y - dy)],
            vec![],
        ),
        with_smash(
            weapon_combo(json!({ "main": step(blow()), "smash": step(blow()) })),
            6_144,
            6,
        ),
    );
    if levels {
        stage.levels = Some(serde_json::from_value(json!({ "xp": [1000] })).unwrap());
    }
    stage
}

fn still(hp: u32) -> Value {
    enemy(
        hp,
        json!({ "type": "straight", "vx": 0, "vy": 0 }),
        json!([]),
    )
}

/// How far above the main character the enemy stands 30 ticks after `keys` on tick 2.
fn after_blow(stage: &Stage, keys: &str) -> i32 {
    let mut e = engine(stage);
    play(&mut e, &presses(&[(2, keys)], 32));
    let s = e.snapshot();
    let target = s.entities.iter().find(|v| v.hp > 0 && v.id.0 > 1).unwrap();
    START_Y - target.at.y.0
}

#[test]
fn a_smash_pulls_what_it_hits_by_its_distance() {
    // 40 pixels above, pulled 24: it stops 16 pixels above. A step reached by `main` pulls
    // nothing.
    let stage = pulling(still(1000), 10_240, false);
    assert_eq!(after_blow(&stage, "b"), 4_096);
    assert_eq!(after_blow(&stage, "a"), 10_240);
}

#[test]
fn the_pull_takes_its_ticks_and_stops_where_the_circles_touch() {
    // 20 pixels above: the pull would end 4 pixels past it, so it stops where the circles
    // touch, the enemy's 8 pixels and the main character's 2.
    let stage = pulling(still(1000), 5_120, false);
    assert_eq!(after_blow(&stage, "b"), 2_048 + 512);
    // Halfway through its 6 ticks it is on its way.
    let mut e = engine(&pulling(still(1000), 10_240, false));
    play(&mut e, &presses(&[(2, "b")], 5));
    let y = START_Y - e.snapshot().entities[1].at.y.0;
    assert!(4_096 < y && y < 10_240, "{y}");
}

#[test]
fn an_elite_and_the_boss_resist_the_pull() {
    let mut elite = still(1000);
    elite["elite"] = json!({ "range": 65_536 });
    assert_eq!(after_blow(&pulling(elite, 10_240, true), "b"), 10_240);
    let mut stage = armed(
        stage_full(600, Some(boss_at(1000, START_Y - 10_240)), vec![], vec![]),
        with_smash(weapon_combo(json!({ "smash": step(blow()) })), 6_144, 6),
    );
    stage.player.hp = 99;
    let mut e = engine(&stage);
    play(&mut e, &presses(&[(2, "b")], 12));
    let boss = &e.snapshot().entities[0];
    assert!(boss.hp < 1000, "the blow lands");
    assert_eq!(START_Y - boss.at.y.0, 10_240);
}

// --- The knockback smash ---------------------------------------------------------------------

/// `pulling`'s stage with a weapon whose smashes knock back 24 pixels over 6 ticks instead.
fn knocking(target: Value, dy: i32, levels: bool) -> Stage {
    let mut stage = pulling(target, dy, levels);
    stage.player.weapon = serde_json::from_value(with_knockback(
        weapon_combo(json!({ "main": step(blow()), "smash": step(blow()) })),
        6_144,
        6,
    ))
    .unwrap();
    stage
}

fn with_knockback(mut weapon: Value, distance: i32, ticks: u32) -> Value {
    weapon["inline"]["smash"] =
        json!({ "type": "knockback", "distance": distance, "ticks": ticks });
    weapon
}

#[test]
fn a_knockback_smash_pushes_what_it_hits_away_by_its_distance() {
    // 10 pixels above, pushed 24: it ends 34 pixels above. A step reached by `main` pushes
    // nothing.
    let stage = knocking(still(1000), 2_560, false);
    assert_eq!(after_blow(&stage, "b"), 2_560 + 6_144);
    assert_eq!(after_blow(&stage, "a"), 2_560);
}

#[test]
fn the_knockback_takes_its_ticks() {
    let mut e = engine(&knocking(still(1000), 2_560, false));
    play(&mut e, &presses(&[(2, "b")], 5));
    let y = START_Y - e.snapshot().entities[1].at.y.0;
    assert!(2_560 < y && y < 2_560 + 6_144, "{y}");
}

#[test]
fn the_knockback_stays_inside_the_map() {
    // Well above the top edge's margin: the push ends at the map's clamp, not past it.
    let stage = knocking(still(1000), START_Y, false);
    let y = after_blow(&stage, "b");
    assert!(START_Y - y >= 0, "{y}");
}

#[test]
fn an_elite_and_the_boss_resist_the_knockback() {
    let mut elite = still(1000);
    elite["elite"] = json!({ "range": 65_536 });
    assert_eq!(after_blow(&knocking(elite, 2_560, true), "b"), 2_560);
    let mut stage = armed(
        stage_full(600, Some(boss_at(1000, START_Y - 10_240)), vec![], vec![]),
        with_knockback(weapon_combo(json!({ "smash": step(blow()) })), 6_144, 6),
    );
    stage.player.hp = 99;
    let mut e = engine(&stage);
    play(&mut e, &presses(&[(2, "b")], 12));
    let boss = &e.snapshot().entities[0];
    assert!(boss.hp < 1000, "the blow lands");
    assert_eq!(START_Y - boss.at.y.0, 10_240);
}

#[test]
fn a_knockback_is_deterministic_and_in_the_state_hash() {
    let run = |stage: &Stage| {
        let mut e = engine(stage);
        play(&mut e, &presses(&[(2, "b")], 12));
        e.state_hash()
    };
    let knock = knocking(still(1000), 2_560, false);
    assert_eq!(run(&knock), run(&knocking(still(1000), 2_560, false)));
    assert_ne!(run(&knock), run(&pulling(still(1000), 2_560, false)));
}

// --- Presets ---------------------------------------------------------------------------------

fn preset(id: &str) -> Value {
    json!({ "preset": { "id": id } })
}

#[test]
fn a_preset_is_the_one_weapon_both_buttons_play() {
    // The giant_sword's A slashes; the giant_hammer's A is its own spin, no preset.
    let first = |weapon: &str| {
        let mut e = engine(&armed(far(), preset(weapon)));
        e.step(pressing_main());
        e.snapshot().player.attacks[0].preset.clone().map(|p| p.0)
    };
    assert_eq!(first("giant_hammer"), None);
    assert_eq!(first("giant_sword").as_deref(), Some("slash"));
}

#[test]
fn a_preset_and_its_inline_copy_run_the_same() {
    let preset_of = |id: &str| Weapons::bundled().tree(id, None).expect("a weapon preset");
    let copy = |w: Value| json!({ "inline": w });
    let waves = || vec![wave(1, 3, 20, still(40), START_X, START_Y - 9_216)];
    let by_id = armed(
        stage_with(600, Some(far_boss()), waves()),
        preset("giant_sword"),
    );
    let inline = armed(
        stage_with(600, Some(far_boss()), waves()),
        copy(serde_json::to_value(preset_of("giant_sword")).unwrap()),
    );
    let (mut a, mut b) = (engine(&by_id), engine(&inline));
    // Slash, slash and the thrust charged to level 2; B, B; an evade and its dash-strike.
    for t in 0..160 {
        let input = Input {
            main: matches!(t, 0 | 8 | 120) || (16..70).contains(&t),
            smash: t == 90 || t == 98,
            evade: t == 118,
            ..if t < 130 { idle() } else { moving(40, -20) }
        };
        a.step(input);
        b.step(input);
        assert_eq!(a.state_hash(), b.state_hash(), "tick {}", a.tick());
    }
}

#[test]
fn every_weapon_preset_is_registered_and_both_roles_are_valid() {
    let registered: Vec<_> = registry::ENTRIES
        .iter()
        .filter(|e| e.kind == Kind::Weapon)
        .map(|e| e.id)
        .collect();
    let defined: Vec<_> = Weapons::bundled().ids().map(|i| i.0.as_str()).collect();
    assert_eq!(registered, defined);
    for id in defined {
        let w = Weapons::bundled().tree(id, None).expect("a definition");
        assert_eq!(validate_weapon_preset(w, registry::ENTRIES), Ok(()), "{id}");
        assert_eq!(
            Weapons::bundled().resolve(&WeaponRef::Preset {
                id: id.into(),
                tree: None
            }),
            Some(w)
        );
    }
}

// --- The Vindictus shape (ADR-084 points 2 and 11) --------------------------------------------

/// The weapon preset `id`.
fn weapon_of(id: &str) -> &'static stella_rain_core::weapon::Weapon {
    Weapons::bundled().tree(id, None).expect("a weapon preset")
}

/// The step that a press sequence reaches from `branches`: `A` for `main`, `B` for `smash`.
fn step_at<'a>(branches: &'a Branches, presses: &str) -> Option<&'a Step> {
    let mut at = branches;
    let mut found = None;
    for press in presses.chars() {
        let step = at.get(press == 'B')?;
        found = Some(step);
        at = &step.next;
    }
    found
}

/// Every step of `branches` with whether a `smash` press reached it (a smash), depth first.
fn steps(branches: &Branches) -> Vec<(bool, &Step)> {
    let mut out = Vec::new();
    for smash in [false, true] {
        if let Some(step) = branches.get(smash) {
            out.push((smash, step));
            out.extend(steps(&step.next));
        }
    }
    out
}

/// The damage types of an attack's active phases.
fn active_types(attack: &AttackRef) -> Vec<Option<DamageType>> {
    presets::phases(attack)
        .unwrap()
        .iter()
        .filter(|p| p.kind == PhaseKind::Active)
        .map(|p| p.damage_type)
        .collect()
}

/// The radius of an attack's circular hit box in its active phase.
fn active_radius(attack: &AttackRef) -> i32 {
    let phases = presets::phases(attack).unwrap();
    let hit_box = phases
        .iter()
        .find(|p| p.kind == PhaseKind::Active)
        .and_then(|p| p.hit_box.as_ref());
    match hit_box {
        Some(HitBox::Circle { radius, .. }) => radius.0,
        other => panic!("a circle hit box, found {other:?}"),
    }
}

#[test]
fn a_smash_ends_its_combo_in_every_weapon() {
    // ADR-084 point 2: after a B press the combo ends, the dash branch included.
    for id in Weapons::bundled().ids().map(|i| i.0.as_str()) {
        let w = weapon_of(id);
        for (smash, step) in steps(&w.combo).into_iter().chain(steps(&w.dash)) {
            if smash {
                assert!(step.next.is_empty(), "{id}: a smash has a next step");
            }
        }
    }
}

#[test]
fn the_giant_sword_s_smashes_bleed_and_the_giant_hammer_s_smashes_stun() {
    // ADR-084 point 8: the Sword's smashes put the bleed on what they hit, the Hammer's stun them
    // for 30 ticks (a placeholder, ADR-085 point 5).
    use stella_rain_core::weapon::SmashEffect;
    assert_eq!(weapon_of("giant_sword").smash, Some(SmashEffect::Bleed {}));
    assert_eq!(
        weapon_of("giant_hammer").smash,
        Some(SmashEffect::Stun { ticks: 30 })
    );
}

#[test]
fn the_giant_sword_takes_the_vindictus_shape() {
    // A to AAAAA, and the smashes B, AB, AAB, AAAB, AAAAB and AAAAAB, all open (ADR-117).
    let w = weapon_of("giant_sword");
    for presses in [
        "A", "AA", "AAA", "AAAA", "AAAAA", "B", "AB", "AAB", "AAAB", "AAAAB", "AAAAAB",
    ] {
        assert!(step_at(&w.combo, presses).is_some(), "{presses}");
    }
    // AAB and AAAB are smashes: Slash, then Pierce, ending the combo, at 100 times the default
    // damage.
    assert_eq!(
        active_types(&step_at(&w.combo, "AAB").unwrap().attack),
        [Some(DamageType::Slash)]
    );
    assert_eq!(
        active_types(&step_at(&w.combo, "AAAB").unwrap().attack),
        [Some(DamageType::Pierce)]
    );
    // Slash, slash, then a thrust that is Pierce; its smashes are a slash and a thrust.
    let types = |presses: &str| active_types(&step_at(&w.combo, presses).unwrap().attack);
    assert_eq!(types("A"), [Some(DamageType::Slash)]);
    assert_eq!(types("AA"), [Some(DamageType::Slash)]);
    assert_eq!(types("AAA"), [Some(DamageType::Pierce)]);
    assert_eq!(types("AAAA"), [Some(DamageType::Slash)]);
    assert_eq!(types("AAAAA"), [Some(DamageType::Pierce)]);
    assert_eq!(types("AAAAB"), [Some(DamageType::Slash)]);
    assert_eq!(types("AAAAAB"), [Some(DamageType::Pierce)]);
    assert_eq!(types("B"), [Some(DamageType::Slash)]);
    assert_eq!(types("AB"), [Some(DamageType::Pierce)]);
    // AAA charges: a wide slash at level 1, a spin at level 2, each with the +50% perfect bonus.
    let charge = step_at(&w.combo, "AAA").unwrap().charge.as_ref().unwrap();
    assert_eq!(charge.levels.len(), 2);
    assert!(charge.levels.iter().all(|l| l.perfect_pct == 50));
    // The dash branch: a lunge thrust, then a dash smash.
    let dash = |smash: bool| w.dash.get(smash).expect("a dash step");
    assert_eq!(
        active_types(&dash(false).attack),
        [Some(DamageType::Pierce)]
    );
    assert!(dash(true).next.is_empty());
}

#[test]
fn the_giant_hammer_spins_wider_with_each_normal_attack() {
    // A to AAAAA, each a spin wider than the last; AAA holds (ADR-084 point 6); the smashes
    // B and AB, and AAB, AAAB, AAAAB and AAAAAB.
    let w = weapon_of("giant_hammer");
    for presses in [
        "A", "AA", "AAA", "AAAA", "AAAAA", "B", "AB", "AAB", "AAAB", "AAAAB", "AAAAAB",
    ] {
        assert!(step_at(&w.combo, presses).is_some(), "{presses}");
    }
    let radius = |presses: &str| active_radius(&step_at(&w.combo, presses).unwrap().attack);
    let widths: Vec<_> = ["A", "AA", "AAA", "AAAA", "AAAAA"].map(radius).into();
    assert!(widths.windows(2).all(|w| w[0] < w[1]), "{widths:?}");
    assert!(step_at(&w.combo, "AAA").unwrap().charge.is_none());
    // AAAAA, the last normal attack, has only its smash AAAAAB after it.
    let next = &step_at(&w.combo, "AAAAA").unwrap().next;
    assert!(next.main.is_none() && next.smash.is_some());
}

#[test]
fn the_long_smashes_end_the_combo_and_hit_harder_the_longer_the_chain() {
    use stella_rain_core::attack::AttackPhase;
    for (id, strike_or_not) in [("giant_sword", false), ("giant_hammer", true)] {
        let w = weapon_of(id);
        let mut last = 0;
        for presses in ["AAB", "AAAB", "AAAAB", "AAAAAB"] {
            let step = step_at(&w.combo, presses).unwrap();
            assert!(step.next.is_empty(), "{id} {presses}");
            let AttackRef::Inline(phases) = &step.attack else {
                panic!("{id} {presses}: an inline attack");
            };
            let damages: Vec<_> = phases
                .iter()
                .filter(|p: &&AttackPhase| p.kind == PhaseKind::Active)
                .map(|p| p.damage)
                .collect();
            // At least 100 times the default damage, and no smash weaker than the one before
            // it: AAAAAB, the six-attack smash, is the strongest (placeholders).
            let [Some(damage)] = damages[..] else {
                panic!("{id} {presses}: one active phase with a damage");
            };
            assert!(damage >= 10_000 && damage >= last, "{id} {presses}");
            if presses == "AAAAAB" {
                assert!(damage > last, "{id}: AAAAAB is the strongest");
            }
            last = damage;
            let types = active_types(&step.attack);
            assert_eq!(
                types == [Some(DamageType::Strike)],
                strike_or_not,
                "{id} {presses}"
            );
        }
    }
}

#[test]
fn every_step_of_the_giant_hammer_is_strike() {
    // ADR-100: every Hammer step is Strike, the dash branch included.
    let w = weapon_of("giant_hammer");
    for (_, step) in steps(&w.combo).into_iter().chain(steps(&w.dash)) {
        assert_eq!(active_types(&step.attack), [Some(DamageType::Strike)]);
    }
}
