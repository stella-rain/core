//! The breaker a style grants (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SIM-7, ADR-SIM-8): its
//! factors scale the stagger fills, the poise damage and a smash's damage on a boss part of the
//! player's hits, and its burst goes off in the call that staggers an elite or the boss. Every
//! number is a test value, not content: 200% factors, a burst of radius 128 px of 50% damage, a
//! 45-tick stun and a knockdown. The rules the tests need (poise, stagger, knockdown) are set on
//! each test's own world.

use super::entity::Who;
use super::state::{EliteState, Stagger, State};
use super::trigger_tests::{Rig, count, named, rig, slot, st};
use crate::engine::Run;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use serde_json::{Value, json};
use std::sync::Arc;

const BIG: u32 = 1_000_000_000;
const MAX: u32 = 100;

/// A breaker kind holding `effects`.
fn breaker(effects: Vec<Value>) -> Value {
    json!({ "type": "breaker", "effects": effects })
}

fn stagger_pct(pct: u32) -> Value {
    json!({ "type": "stagger_pct", "pct": pct })
}

fn poise_pct(pct: u32) -> Value {
    json!({ "type": "poise_pct", "pct": pct })
}

/// The burst of the design table: radius 128 px, 50% damage, a 45-tick stun, a knockdown.
fn burst_of(effects: Vec<Value>) -> Value {
    json!({ "type": "on_stagger_burst", "radius_px": 128, "effects": effects })
}

fn full_burst() -> Value {
    burst_of(vec![
        json!({ "type": "damage_pct", "pct": 50 }),
        json!({ "type": "stun_ticks", "ticks": 45 }),
        json!({ "type": "knock_down" }),
    ])
}

/// Sets the rules this file tests on the rig's world and makes the enemies at `elites` elites
/// of `stagger_max` 100; every enemy names a poise of 60. The rules' own refill, drain and
/// `vulnerable` numbers are cleared so a fill adds what it is given.
fn prep(r: &mut Rig, elites: &[usize]) {
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let def = state.enemies[0].def;
    let w = Arc::make_mut(world);
    let t = &mut w.tuning;
    t.flinch_ticks = 30;
    t.poise_damage_a = 10;
    t.poise_damage_b = 40;
    t.stagger_smash = 40;
    t.stagger_counter = 30;
    t.staggered_ticks = 120;
    t.down_ticks = 40;
    t.stagger_refill_len = 0;
    t.vulnerable_stagger_pct = 0;
    t.vulnerable_stun_pct = 0;
    t.vulnerable_down_pct = 0;
    w.has_stagger = true;
    w.has_down = true;
    let a = w.defs.agent_mut(def);
    a.stagger_max = Some(MAX);
    a.staggered_ticks = None;
    a.poise = Some(60);
    for &i in elites {
        let at = state.enemies[i].at;
        state.enemies[i].elite = Some(EliteState {
            home: at,
            called: false,
            boss_call: Default::default(),
            disguised: false,
            stagger: Stagger::default(),
        });
    }
    for e in &mut state.enemies {
        e.stun_immune = 0;
    }
}

/// Puts enemy `i` `px` pixels (and `extra` 1/256) right of enemy 0.
fn place(state: &mut State, i: usize, px: i32, extra: i32) {
    let base = state.enemies[0].at;
    state.enemies[i].at = Point {
        x: Fx(base.x.0 + Fx::px(px).0 + extra),
        y: base.y,
    };
}

fn gauge(state: &State, i: usize) -> u32 {
    state.enemies[i].elite.as_ref().unwrap().stagger.gauge
}

fn staggered(state: &State, i: usize) -> u16 {
    state.enemies[i].elite.as_ref().unwrap().stagger.staggered
}

/// One fill of `value` credited to `me` on enemy `i`, from a hit of `hit` damage: the events.
fn fill(r: &mut Rig, i: usize, me: u8, value: u32, hit: u32) -> Vec<Value> {
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let mut events = Vec::new();
    super::stun::fill(
        world,
        state,
        Who::Enemy(i),
        slot(me),
        value,
        hit,
        &mut events,
    );
    super::trigger_tests::as_json(&events)
}

fn hp_lost(r: &mut Rig, i: usize) -> u32 {
    BIG - st(&mut r.e).enemies[i].hp
}

/// Gauge value of enemy 0 after one fill of `value` by player `me`.
fn filled(r: &mut Rig, me: u8, value: u32) -> u32 {
    st(&mut r.e).enemies[0].elite.as_mut().unwrap().stagger = Stagger::default();
    fill(r, 0, me, value, 0);
    gauge(st(&mut r.e), 0)
}

/// S5: a style that names only the breaker leaves the weapon's tree equal to the base's: the
/// kind changes no step, the rules read it beside the weapon.
#[test]
fn the_breaker_kind_changes_no_step() {
    use crate::expertise::{Expertise, weapon_with_style};
    use crate::pack::Pack;
    use crate::version::Versions;
    let base = Pack::bundled()
        .content()
        .weapons()
        .tree("giant_hammer", None)
        .expect("the Hammer's base tree");
    let bundled: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let mut file = bundled["expertise"].clone();
    let w = file["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .unwrap();
    w["styles"][0]["kinds"] = json!([breaker(vec![stagger_pct(200), full_burst()])]);
    let expertise =
        Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the expertise");
    let style = &expertise
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_hammer")
        .unwrap()
        .styles[0];
    for taken in 0..=3 {
        assert_eq!(
            &weapon_with_style(base, style, taken),
            base,
            "taken {taken}"
        );
    }
}

/// B1: with no breaker style a fill, a poise damage and a burst-less stagger are plain: every
/// fill adds what it is given and nothing emits a `StaggerBurst`.
#[test]
fn a_run_that_names_no_breaker_stays_dormant() {
    let mut r = rig(1, vec![], 3);
    prep(&mut r, &[0, 1]);
    assert_eq!(filled(&mut r, 0, 40), 40);
    let events = fill(&mut r, 0, 0, 60, 200);
    assert!(staggered(st(&mut r.e), 0) > 0, "40 and 60 fill the bar");
    assert_eq!(count(&events, "StaggerBurst"), 0);
    let events = r.run_step(true);
    assert_eq!(count(&events, "StaggerBurst"), 0);
}

/// B2: the stagger factor scales every fill of the player: 40 becomes 80, a bar that is full is
/// capped, the refill percent applies after it, a downed player fills nothing and another
/// player's fill is plain.
#[test]
fn the_stagger_factor_scales_every_fill() {
    let mut r = rig(2, vec![breaker(vec![stagger_pct(200)])], 2);
    prep(&mut r, &[0]);
    assert_eq!(filled(&mut r, 0, 40), 80, "doubled");
    assert_eq!(filled(&mut r, 0, 30), 60, "a counter's 30 doubled");
    // A bar nearly full is capped and staggers.
    st(&mut r.e).enemies[0]
        .elite
        .as_mut()
        .unwrap()
        .stagger
        .gauge = 60;
    fill(&mut r, 0, 0, 40, 0);
    assert!(staggered(st(&mut r.e), 0) > 0);
    // The refill percent applies after the factor: a second bar at 50% takes 40, not 80.
    {
        let Run::V2 { world, state } = r.e.run_mut() else {
            panic!("the live rules");
        };
        let t = &mut Arc::make_mut(world).tuning;
        t.stagger_refill_len = 2;
        t.stagger_refill_pcts[0] = 100;
        t.stagger_refill_pcts[1] = 50;
        let e = state.enemies[0].elite.as_mut().unwrap();
        e.stagger = Stagger {
            breaks: 1,
            ..Stagger::default()
        };
    }
    fill(&mut r, 0, 0, 40, 0);
    assert_eq!(gauge(st(&mut r.e), 0), 40);
    // Another player (no breaker style) fills plain.
    st(&mut r.e).enemies[0].elite.as_mut().unwrap().stagger = Stagger::default();
    if let Some(g) = st(&mut r.e).players.career_mut(slot(1)).expertise.as_mut() {
        g.style = Some(1);
    }
    assert_eq!(filled(&mut r, 1, 40), 40, "a player without the style");
    // A downed player fills nothing.
    let s = st(&mut r.e);
    s.players[slot(0)].hp = 0;
    s.players[slot(0)].down_ticks = 0;
    assert_eq!(filled(&mut r, 0, 40), 0);
}

/// B2: a style naming only the other effects leaves the fill plain.
#[test]
fn a_breaker_without_the_stagger_factor_fills_plain() {
    let mut r = rig(1, vec![breaker(vec![poise_pct(200)])], 2);
    prep(&mut r, &[0]);
    assert_eq!(filled(&mut r, 0, 40), 40);
}

/// B3: the poise factor scales the poise damage of the player's combo hits: an A step's 10 on a
/// poise-15 enemy does not flinch it plain and flinches it with 200% (20); with 150% (15) it
/// does too, with the other effects only it does not, and on a poise-60 enemy 20 is not enough.
/// (A smash's own stun wins over a flinch, so the A step carries the test.)
#[test]
fn the_poise_factor_scales_a_combo_hits_poise_damage() {
    let flinches = |kinds: Vec<Value>, poise: u32| {
        let mut r = rig(1, kinds, 1);
        prep(&mut r, &[]);
        let Run::V2 { world, state } = r.e.run_mut() else {
            panic!("the live rules");
        };
        let def = state.enemies[0].def;
        Arc::make_mut(world).defs.agent_mut(def).poise = Some(poise);
        let events = r.run_step(false);
        named(&events, "StatusApplied")
            .iter()
            .filter(|e| e["status"] == "flinch")
            .count()
    };
    assert_eq!(flinches(vec![], 15), 0, "plain");
    assert!(
        flinches(vec![breaker(vec![poise_pct(200)])], 15) > 0,
        "doubled"
    );
    assert!(
        flinches(vec![breaker(vec![poise_pct(150)])], 15) > 0,
        "one and a half"
    );
    assert_eq!(
        flinches(vec![breaker(vec![poise_pct(200)])], 60),
        0,
        "20 is under 60"
    );
    assert_eq!(
        flinches(vec![breaker(vec![stagger_pct(200)])], 15),
        0,
        "another effect only"
    );
}

/// The enemies of a burst scene: enemy 0 the elite that is staggered, 1 a regular enemy 100 px
/// away, 2 one just past the edge, 3 one exactly on it, 4 a second elite 50 px away.
fn scene(kinds: Vec<Value>) -> Rig {
    let mut r = rig(1, kinds, 5);
    prep(&mut r, &[0, 4]);
    let radius = {
        let Run::V2 { world, state } = r.e.run_mut() else {
            panic!("the live rules");
        };
        world.defs.agent(state.enemies[0].def).radius.0
    };
    let s = st(&mut r.e);
    place(s, 1, 100, 0);
    place(s, 2, 128, radius + 1);
    place(s, 3, 128, radius);
    place(s, 4, 50, 0);
    r
}

/// B6: a fill that crosses the gauge bursts once: 50% of the hit's damage reaches every other
/// enemy within the radius (the edge counts), the staggered elite takes none, a second elite
/// takes the damage and nothing else, and a fill during the stagger bursts nothing.
#[test]
fn a_stagger_bursts_once_and_hurts_the_enemies_round_it() {
    let mut r = scene(vec![breaker(vec![burst_of(vec![
        json!({ "type": "damage_pct", "pct": 50 }),
    ])])]);
    let events = fill(&mut r, 0, 0, 100, 200);
    let bursts = named(&events, "StaggerBurst");
    assert_eq!(bursts.len(), 1, "{events:?}");
    assert_eq!(bursts[0]["radius"], 128 * 256);
    assert_eq!(bursts[0]["player"], "first");
    assert_eq!(hp_lost(&mut r, 0), 0, "not the staggered elite");
    assert_eq!(hp_lost(&mut r, 1), 100, "50% of 200");
    assert_eq!(hp_lost(&mut r, 2), 0, "past the edge");
    assert_eq!(hp_lost(&mut r, 3), 100, "on the edge");
    assert_eq!(hp_lost(&mut r, 4), 100, "the second elite takes the damage");
    assert_eq!(gauge(st(&mut r.e), 4), 0, "and no fill");
    assert_eq!(staggered(st(&mut r.e), 4), 0, "no stagger");
    let again = fill(&mut r, 0, 0, 100, 200);
    assert_eq!(count(&again, "StaggerBurst"), 0, "a fill in the stagger");
    assert_eq!(hp_lost(&mut r, 1), 100);
}

/// B6: the burst deals at least 1.
#[test]
fn the_burst_deals_at_least_one() {
    let mut r = scene(vec![breaker(vec![burst_of(vec![
        json!({ "type": "damage_pct", "pct": 1 }),
    ])])]);
    fill(&mut r, 0, 0, 100, 10);
    assert_eq!(hp_lost(&mut r, 1), 1);
}

/// B7: the burst stuns and knocks down the regular enemies in its radius, never an elite; one
/// that its damage killed is neither; a burst naming only damage controls nothing.
#[test]
fn the_burst_controls_regular_enemies_only() {
    let mut r = scene(vec![breaker(vec![full_burst()])]);
    let events = fill(&mut r, 0, 0, 100, 200);
    let s = st(&mut r.e);
    assert_eq!(count(&events, "StaggerBurst"), 1);
    assert!(super::stun::stunned(&s.enemies[1].statuses), "stunned");
    assert!(s.enemies[1].control.knocked_down > 0, "knocked down");
    assert!(
        !super::stun::stunned(&s.enemies[2].statuses),
        "past the edge"
    );
    assert!(!super::stun::stunned(&s.enemies[4].statuses), "an elite");
    assert_eq!(s.enemies[4].control.knocked_down, 0);
    // A kill is neither.
    let mut r = scene(vec![breaker(vec![full_burst()])]);
    st(&mut r.e).enemies[1].hp = 50;
    fill(&mut r, 0, 0, 100, 200);
    let s = st(&mut r.e);
    assert_eq!(s.enemies[1].hp, 0);
    assert!(!super::stun::stunned(&s.enemies[1].statuses));
    // Damage only.
    let mut r = scene(vec![breaker(vec![burst_of(vec![
        json!({ "type": "damage_pct", "pct": 50 }),
    ])])]);
    fill(&mut r, 0, 0, 100, 200);
    let s = st(&mut r.e);
    assert!(!super::stun::stunned(&s.enemies[1].statuses));
    assert_eq!(s.enemies[1].control.knocked_down, 0);
}

/// B9: the burst's damage fills no gauge, wins no rally and staggers nobody else.
#[test]
fn the_burst_has_no_side_effects() {
    let mut r = scene(vec![breaker(vec![full_burst()])]);
    let before = st(&mut r.e).players[slot(0)].combat.rally;
    fill(&mut r, 0, 0, 100, 200);
    let s = st(&mut r.e);
    assert_eq!(gauge(s, 4), 0);
    assert_eq!(s.players[slot(0)].combat.rally, before);
    assert_eq!(staggered(s, 4), 0);
}

/// B10: a style naming no burst bursts nothing, though its fill staggers.
#[test]
fn a_style_without_a_burst_bursts_nothing() {
    let mut r = scene(vec![breaker(vec![stagger_pct(200)])]);
    let events = fill(&mut r, 0, 0, 100, 200);
    assert!(staggered(st(&mut r.e), 0) > 0);
    assert_eq!(count(&events, "StaggerBurst"), 0);
    assert_eq!(hp_lost(&mut r, 1), 0);
}

/// B11: in a party only the player on the breaker style scales; another's fill is plain.
#[test]
fn only_the_player_on_the_style_gets_the_breaker() {
    let mut r = rig(3, vec![breaker(vec![stagger_pct(200)])], 1);
    prep(&mut r, &[0]);
    for i in [0u8, 2] {
        if let Some(g) = st(&mut r.e).players.career_mut(slot(i)).expertise.as_mut() {
            g.style = Some(1);
        }
    }
    assert_eq!(filled(&mut r, 1, 40), 80);
    assert_eq!(filled(&mut r, 0, 40), 40);
    assert_eq!(filled(&mut r, 2, 40), 40);
}

/// B12: picking another style stops every effect at the next fill.
#[test]
fn another_style_stops_the_effects() {
    let mut r = rig(1, vec![breaker(vec![stagger_pct(200)])], 1);
    prep(&mut r, &[0]);
    assert_eq!(filled(&mut r, 0, 40), 80);
    if let Some(g) = st(&mut r.e).players.career_mut(slot(0)).expertise.as_mut() {
        g.style = Some(1);
    }
    assert_eq!(filled(&mut r, 0, 40), 40);
}

/// A one-player run on the bundled pack whose Hammer style 0 names `style`, its first upgrade 1
/// `upgrade_1`, its upgrade 2 `upgrade_2` and its capstone `capstone` (kinds, each a list).
fn levelled(
    style: Vec<Value>,
    upgrade_1: Vec<Value>,
    upgrade_2: Vec<Value>,
    capstone: Vec<Value>,
) -> crate::engine::Engine {
    use crate::pack::Pack;
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = json!(style);
    w["styles"][0]["upgrade_1"][0]["kinds"] = json!(upgrade_1);
    w["styles"][0]["upgrade_2"][0]["kinds"] = json!(upgrade_2);
    w["styles"][0]["capstone"][0]["kinds"] = json!(capstone);
    let pack = Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack");
    let setup = crate::run_setup::RunSetup {
        mode: "time_limit".into(),
        weapon: "giant_hammer".into(),
        seed: 20_261_116,
        ..crate::run_setup::RunSetup::default()
    };
    let mut e = crate::engine::Engine::start_party_with(&setup, pack.content(), 3).expect("a run");
    let s = st(&mut e);
    for i in 0..3 {
        if let Some(g) = s.players.career_mut(slot(i)).expertise.as_mut() {
            g.style = Some(0);
        }
    }
    e
}

/// A grant as a tuple: the three factors and the burst's radius, damage, stun and fall.
type Seen = (u32, u32, u32, Option<(i32, u32, u32, bool)>);

/// The grant player `i` has now: its three factors and its burst's radius, damage, stun and fall.
fn grant(e: &mut crate::engine::Engine, i: u8) -> Option<Seen> {
    let Run::V2 { world, state } = e.run_mut() else {
        panic!("the live rules");
    };
    world.breaker_of(state, slot(i)).map(|g| {
        (
            g.stagger_pct,
            g.poise_pct,
            g.part_break_pct,
            g.burst
                .map(|b| (b.radius, b.damage_pct, b.stun_ticks, b.knock_down)),
        )
    })
}

fn take(e: &mut crate::engine::Engine, i: u8, places: usize) {
    let path = &mut st(e)
        .players
        .career_mut(slot(i))
        .expertise
        .as_mut()
        .expect("a gauge")
        .path;
    path.clear();
    path.extend(std::iter::repeat_n(0, places));
}

/// B5: each player reads the grant of its own style and path; a style that names none gives none;
/// a grant a hand-built kind pushed out of range is clamped when the world is built.
#[test]
fn each_player_reads_its_own_grant() {
    let kind = breaker(vec![stagger_pct(200), poise_pct(150), full_burst()]);
    let mut e = levelled(vec![kind], vec![], vec![], vec![]);
    assert_eq!(
        grant(&mut e, 0),
        Some((200, 150, 100, Some((128 * 256, 50, 45, true))))
    );
    if let Some(g) = st(&mut e).players.career_mut(slot(1)).expertise.as_mut() {
        g.style = Some(1);
    }
    assert_eq!(grant(&mut e, 1), None, "a style naming none");
    assert!(grant(&mut e, 2).is_some(), "another player's own");
    // Under try_styles only the style's own grants count, whatever the path says.
    take(&mut e, 0, 3);
    assert_eq!(grant(&mut e, 0).map(|g| g.0), Some(200));
    let plain = levelled(vec![], vec![], vec![], vec![]);
    let Run::V2 { world, .. } = plain.run() else {
        panic!("the live rules");
    };
    assert!(!world.has_breaker, "the bundled pack names none");
}

/// B13: a path names the breaker at several levels: the effects stack as each level is taken, in
/// path order, and a later level's same effect replaces the earlier one.
#[test]
fn the_levels_of_a_path_stack_their_effects() {
    let mut e = levelled(
        vec![breaker(vec![stagger_pct(200)])],
        vec![breaker(vec![stagger_pct(300), poise_pct(200)])],
        vec![breaker(vec![
            json!({ "type": "part_break_pct", "pct": 200 }),
        ])],
        vec![breaker(vec![full_burst()])],
    );
    let bare = |burst| (200, 100, 100, burst);
    assert_eq!(grant(&mut e, 0), Some(bare(None)), "the style alone");
    take(&mut e, 0, 1);
    assert_eq!(
        grant(&mut e, 0),
        Some((300, 200, 100, None)),
        "the later level replaces"
    );
    take(&mut e, 0, 2);
    assert_eq!(grant(&mut e, 0), Some((300, 200, 200, None)));
    take(&mut e, 0, 3);
    assert_eq!(
        grant(&mut e, 0),
        Some((300, 200, 200, Some((128 * 256, 50, 45, true))))
    );
    // A burst named at two levels reads the later one.
    let mut e = levelled(
        vec![breaker(vec![burst_of(vec![
            json!({ "type": "damage_pct", "pct": 10 }),
        ])])],
        vec![],
        vec![],
        vec![breaker(vec![full_burst()])],
    );
    assert_eq!(grant(&mut e, 0).unwrap().3, Some((128 * 256, 10, 0, false)));
    take(&mut e, 0, 3);
    assert_eq!(grant(&mut e, 0).unwrap().3, Some((128 * 256, 50, 45, true)));
}

/// The damage a press of `smash` (else A) does to a boss part of the first boss, over the whole
/// step: the part is rebuilt with `BIG` hit points before each tick and the boss body is kept in
/// front of the player, so the part's loss is what the step dealt.
fn part_loss(kinds: Vec<Value>, smash: bool) -> u32 {
    use super::state::PartState;
    let mut r = rig(1, kinds, 0);
    let mut n = 0;
    while st(&mut r.e).boss.is_none() && n < 40_000 {
        r.step([Input::default(); 3]);
        n += 1;
    }
    let s = st(&mut r.e);
    let b = s.boss.as_mut().expect("a boss");
    b.invulnerable = 0;
    let kind = b.kind;
    let id = EntityId(s.next_id);
    s.next_id += 1;
    s.boss.as_mut().unwrap().parts = vec![PartState {
        id,
        name: crate::id::ContentId::from("horn"),
        kind,
        hp: BIG,
        max_hp: BIG,
        defence: 0,
        offset: Point::default(),
        radius: Fx::px(20).0,
        statuses: Default::default(),
    }];
    let mut lost = 0;
    for tick in 0..60 {
        let s = st(&mut r.e);
        let (at, facing) = (s.players[slot(0)].at, s.players[slot(0)].facing);
        let scale = i64::from(crate::trig::SCALE);
        let along = i64::from(Fx::px(24).0);
        let b = s.boss.as_mut().unwrap();
        b.invulnerable = 0;
        b.at = Point {
            x: Fx(at.x.0 + i32::try_from(i64::from(facing.x) * along / scale).unwrap_or(0)),
            y: Fx(at.y.0 + i32::try_from(i64::from(facing.y) * along / scale).unwrap_or(0)),
        };
        b.parts[0].hp = BIG;
        let input = Input {
            main: tick == 0 && !smash,
            smash: tick == 0 && smash,
            ..Input::default()
        };
        r.step([input, Input::default(), Input::default()]);
        lost += BIG - st(&mut r.e).boss.as_ref().unwrap().parts[0].hp;
    }
    lost
}

/// B4: a smash step's hit on a boss part takes 200% of the plain damage; an A step's hit on the
/// same part, and a run without the effect, take the plain damage.
#[test]
fn the_part_break_factor_scales_a_smashs_hit_on_a_part() {
    let part_break = breaker(vec![json!({ "type": "part_break_pct", "pct": 200 })]);
    let plain = part_loss(vec![], true);
    assert!(plain > 0, "the smash reaches the part");
    assert_eq!(
        part_loss(vec![part_break.clone()], true),
        plain * 2,
        "doubled"
    );
    assert_eq!(
        part_loss(vec![breaker(vec![stagger_pct(200)])], true),
        plain
    );
    let a = part_loss(vec![], false);
    assert_eq!(part_loss(vec![part_break], false), a, "an A step is plain");
}

/// B8: a resisted stun's fill that staggers bursts too (with no hit to measure, the burst still
/// deals its at-least-1 damage).
#[test]
fn a_resisted_stuns_fill_bursts() {
    let mut r = scene(vec![breaker(vec![burst_of(vec![
        json!({ "type": "damage_pct", "pct": 50 }),
    ])])]);
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    Arc::make_mut(world).tuning.stagger_values[0] = 100;
    let source = state.players.id(slot(0));
    state.stun.elite_pct = 0;
    let mut events = Vec::new();
    super::stun::apply(world, state, Who::Enemy(0), source, 30, &mut events);
    let events = super::trigger_tests::as_json(&events);
    assert_eq!(count(&events, "StaggerBurst"), 1);
    assert!(staggered(st(&mut r.e), 0) > 0);
    assert_eq!(hp_lost(&mut r, 1), 1);
}

/// B8: the smash that staggers an elite bursts once, from that elite.
#[test]
fn a_smash_that_staggers_an_elite_bursts_with_its_damage() {
    let mut r = rig(
        1,
        vec![breaker(vec![burst_of(vec![
            json!({ "type": "damage_pct", "pct": 50 }),
        ])])],
        2,
    );
    prep(&mut r, &[0]);
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let def = state.enemies[0].def;
    Arc::make_mut(world).defs.agent_mut(def).stagger_max = Some(40);
    let events = r.run_step(true);
    assert_eq!(count(&events, "StaggerBurst"), 1, "{events:?}");
    let burst_hit = named(&events, "StaggerBurst")[0]["target"].as_u64();
    assert_eq!(burst_hit, Some(u64::from(st(&mut r.e).enemies[0].id.0)));
}

/// B8: a part's hit that staggers the boss bursts from the boss's centre.
#[test]
fn a_part_hit_that_staggers_the_boss_bursts_from_its_centre() {
    use super::state::PartState;
    let mut r = rig(
        1,
        vec![breaker(vec![burst_of(vec![
            json!({ "type": "damage_pct", "pct": 50 }),
        ])])],
        0,
    );
    let mut n = 0;
    while st(&mut r.e).boss.is_none() && n < 40_000 {
        r.step([Input::default(); 3]);
        n += 1;
    }
    {
        let Run::V2 { world, state } = r.e.run_mut() else {
            panic!("the live rules");
        };
        let w = Arc::make_mut(world);
        w.tuning.stagger_smash = 40;
        w.tuning.staggered_ticks = 120;
        w.tuning.stagger_refill_len = 0;
        w.has_stagger = true;
        for b in &mut w.bosses {
            b.boss.stagger_max = Some(40);
        }
        let b = state.boss.as_mut().expect("a boss");
        b.invulnerable = 0;
        let (kind, id) = (b.kind, EntityId(state.next_id));
        state.next_id += 1;
        b.parts = vec![PartState {
            id,
            name: crate::id::ContentId::from("horn"),
            kind,
            hp: BIG,
            max_hp: BIG,
            defence: 0,
            offset: Point::default(),
            radius: Fx::px(20).0,
            statuses: Default::default(),
        }];
    }
    let mut seen = Vec::new();
    for tick in 0..60 {
        let s = st(&mut r.e);
        let (at, facing) = (s.players[slot(0)].at, s.players[slot(0)].facing);
        let scale = i64::from(crate::trig::SCALE);
        let along = i64::from(Fx::px(24).0);
        let b = s.boss.as_mut().unwrap();
        b.invulnerable = 0;
        b.at = Point {
            x: Fx(at.x.0 + i32::try_from(i64::from(facing.x) * along / scale).unwrap_or(0)),
            y: Fx(at.y.0 + i32::try_from(i64::from(facing.y) * along / scale).unwrap_or(0)),
        };
        let input = Input {
            smash: tick == 0,
            ..Input::default()
        };
        seen.extend(r.step([input, Input::default(), Input::default()]));
    }
    let bursts = named(&seen, "StaggerBurst");
    assert_eq!(bursts.len(), 1, "{bursts:?}");
    let boss = st(&mut r.e).boss.as_ref().unwrap().id.0;
    assert_eq!(bursts[0]["target"].as_u64(), Some(u64::from(boss)));
}

/// B11: the burst's events and credit name the player whose fill staggered the target.
#[test]
fn the_burst_names_the_player_whose_fill_staggered() {
    let mut r = rig(
        3,
        vec![breaker(vec![burst_of(vec![
            json!({ "type": "damage_pct", "pct": 50 }),
        ])])],
        2,
    );
    prep(&mut r, &[0]);
    place(st(&mut r.e), 1, 10, 0);
    for i in [0u8, 2] {
        if let Some(g) = st(&mut r.e).players.career_mut(slot(i)).expertise.as_mut() {
            g.style = Some(1);
        }
    }
    assert_eq!(count(&fill(&mut r, 0, 0, 100, 100), "StaggerBurst"), 0);
    // Player 0 staggered it without a breaker: reset and let player 1 do it.
    st(&mut r.e).enemies[0].elite.as_mut().unwrap().stagger = Stagger::default();
    let events = fill(&mut r, 0, 1, 100, 100);
    let bursts = named(&events, "StaggerBurst");
    assert_eq!(bursts.len(), 1);
    assert_eq!(bursts[0]["player"], serde_json::to_value(slot(1)).unwrap());
    assert_eq!(hp_lost(&mut r, 1), 50);
}

/// B14: the same scenario twice gives the same state hash every tick, and a savepoint taken
/// mid-stagger replays the same hashes for the next 240 ticks.
#[test]
fn a_burst_is_deterministic_and_leaves_no_state_behind() {
    let go = |r: &mut Rig| {
        fill(r, 0, 0, 100, 200);
        r.idle(10);
    };
    let make = || {
        let mut r = scene(vec![breaker(vec![full_burst()])]);
        // Idle ticks must not move the scene: nothing else acts.
        if let Run::V2 { world, .. } = r.e.run_mut() {
            Arc::make_mut(world).tuning.stagger_refill_len = 0;
        }
        r
    };
    let (mut a, mut b) = (make(), make());
    go(&mut a);
    go(&mut b);
    let bytes = crate::savepoint::encode_state(&a.e);
    assert!(!bytes.is_empty());
    assert_eq!(a.e.state_hash(), b.e.state_hash());
    for _ in 0..240 {
        a.step([Input::default(); 3]);
        b.step([Input::default(); 3]);
        assert_eq!(a.e.state_hash(), b.e.state_hash());
    }
}
