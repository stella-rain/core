//! The counter bonus a style grants (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SIM-7, ADR-SIM-8): a
//! near miss of the evade opens a window, the first dash-branch step started inside it takes the
//! bonus and the step's hits carry it. Every number is a test value, not content: window 30,
//! damage +100%, stagger x3, a 45-tick stun, a 48 px slide.

use super::state::{PlayerIdx, State};
use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use crate::pack::Pack;
use crate::rules::PerfectEvade;
use crate::run_setup::RunSetup;
use serde_json::{Value, json};
use std::sync::Arc;

const ME: PlayerIdx = PlayerIdx::SOLO;
const BIG: u32 = 1_000_000_000;
const RULE: PerfectEvade = PerfectEvade {
    window_ticks: 10,
    slow_ticks: 60,
    cooldown_ticks: 300,
    reach: 200 * 256,
    slow_every: 2,
};

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

fn counter_kind(effects: Value) -> Value {
    json!({ "type": "counter_bonus", "window_ticks": 30, "effects": effects })
}

fn all_effects() -> Value {
    json!([
        { "type": "damage_pct", "pct": 100 },
        { "type": "stagger_pct", "pct": 300 },
        { "type": "stun_ticks", "ticks": 45 },
        { "type": "advance", "distance": 48 * 256 }
    ])
}

/// The bundled pack with `kinds` on the Hammer's first style.
fn pack(kinds: Vec<Value>) -> Pack {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["styles"][0]["kinds"] = json!(kinds);
    // The tests set their own stun: the bundled counter stun, its gauge value and the
    // `vulnerable` a landing stun adds (a second status of the stun's ticks) stay out.
    let rules = v["rules"].as_object_mut().expect("the rules");
    rules.remove("counter_stun_ticks");
    rules.remove("vulnerable_pcts");
    if let Some(values) = rules.get_mut("stagger_values").and_then(Value::as_object_mut) {
        values.remove("counter");
    }
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack")
}

fn st(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// One Hammer run of `players` on style 0, a rule for the perfect evade, `foes` enemies of
/// defence 0 stacked in front of the first player each tick.
struct Rig {
    e: Engine,
    foes: Vec<EntityId>,
}

fn rig(players: u8, kinds: Vec<Value>, foes: u32) -> Rig {
    let pack = pack(kinds);
    let setup = RunSetup {
        mode: "time_limit".into(),
        weapon: "giant_hammer".into(),
        seed: 20_261_114,
        ..RunSetup::default()
    };
    let mut e = Engine::start_party_with(&setup, pack.content(), players).expect("a run");
    while !st(&mut e).enemies.iter().any(|x| x.hp > 0 && !x.hidden()) {
        e.step(&[Input::default(); 3]);
    }
    if let Run::V2 { world, .. } = e.run_mut() {
        Arc::make_mut(world).tuning.perfect_evade = Some(RULE);
    }
    let s = st(&mut e);
    let model = s
        .enemies
        .iter()
        .find(|x| x.hp > 0 && !x.hidden())
        .expect("one")
        .clone();
    s.enemies.clear();
    let mut ids = Vec::new();
    for i in 0..foes {
        let mut f = model.clone();
        f.id = EntityId(s.next_id + i);
        f.hp = BIG;
        f.max_hp = BIG;
        f.defence = 0;
        ids.push(f.id);
        s.enemies.push(f);
    }
    s.next_id += foes;
    for i in 0..players {
        let p = &mut s.players[slot(i)];
        p.hp = 1_000_000;
        p.max_hp = 1_000_000;
        p.invulnerable = 0;
        if let Some(g) = s.players.career_mut(slot(i)).expertise.as_mut() {
            g.style = Some(0);
        }
    }
    Rig { e, foes: ids }
}

impl Rig {
    fn stack(&mut self) {
        let s = st(&mut self.e);
        let (at, facing) = (s.players[ME].at, s.players[ME].facing);
        s.enemies.retain(|x| self.foes.contains(&x.id));
        let scale = i64::from(crate::trig::SCALE);
        let along = i64::from(Fx::px(24).0);
        for f in &mut s.enemies {
            f.at = Point {
                x: Fx(at.x.0 + i32::try_from(i64::from(facing.x) * along / scale).unwrap_or(0)),
                y: Fx(at.y.0 + i32::try_from(i64::from(facing.y) * along / scale).unwrap_or(0)),
            };
            f.hp = BIG;
        }
    }

    /// One tick; the enemies stand where the step needs them and the hit points they lost.
    fn step(&mut self, inputs: [Input; 3]) -> (u32, Vec<Value>) {
        self.stack();
        self.e.step(&inputs);
        let lost = st(&mut self.e)
            .enemies
            .iter()
            .filter(|x| self.foes.contains(&x.id))
            .map(|x| BIG - x.hp.min(BIG))
            .sum();
        (lost, as_json(self.e.events()))
    }

    fn idle(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step([Input::default(); 3]);
        }
    }

    /// Player 0's evade press, which starts the dash-strike.
    fn evade(&mut self) {
        let press = Input {
            evade: true,
            ..Input::default()
        };
        self.step([press, Input::default(), Input::default()]);
    }

    /// Player 0's press of the main button: the dash branch while a dash-strike may start.
    fn press(&mut self) -> (u32, Vec<Value>) {
        let press = Input {
            main: true,
            ..Input::default()
        };
        self.step([press, Input::default(), Input::default()])
    }

    /// Runs on until the step ends, summing what it dealt and listing every event.
    fn finish(&mut self, mut dealt: u32, mut events: Vec<Value>) -> (u32, Vec<Value>) {
        for _ in 0..60 {
            let (lost, ev) = self.step([Input::default(); 3]);
            dealt += lost;
            events.extend(ev);
        }
        (dealt, events)
    }

    fn combat(&mut self, i: u8) -> (u16, u8) {
        let c = &st(&mut self.e).players[slot(i)].combat;
        (c.counter_window, c.counter_bonus_step)
    }

    fn set_window(&mut self, i: u8, window: u16) {
        st(&mut self.e).players[slot(i)].combat.counter_window = window;
    }

    fn keep_dash_strike(&mut self, i: u8) {
        st(&mut self.e).players[slot(i)].combo.dash_strike_left = 1_000;
    }

    fn view(&self, i: usize) -> Value {
        serde_json::to_value(&self.e.snapshot().players[i]).expect("a view")
    }

    /// A near miss of player `i`, as the hit pass reports it: the evade's own invulnerability
    /// stopped the hit `since` ticks after the press.
    fn near_miss(&mut self, i: u8, cooldown: u16) -> Vec<Value> {
        let mut events = Vec::new();
        let Run::V2 { world, state } = self.e.run_mut() else {
            panic!("the live rules");
        };
        let p = &mut state.players[slot(i)];
        p.combat.perfect_window = RULE.window_ticks;
        p.combat.perfect_cooldown = cooldown;
        p.invulnerable = world.player.evade.invulnerable_ticks;
        super::player::near_miss(world, state, slot(i), &mut events);
        as_json(&events)
    }
}

fn as_json(events: &[DomainEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|e| serde_json::to_value(e).expect("an event"))
        .collect()
}

fn count(events: &[Value], ty: &str) -> usize {
    events.iter().filter(|e| e["type"] == ty).count()
}

fn named(events: &[Value], ty: &str) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e["type"] == ty)
        .map(|e| e["data"].clone())
        .collect()
}

/// B1: a run whose styles name no counter bonus never opens a window, shows no view and emits no
/// counter event, through evades, dash-branch presses and hits.
#[test]
fn a_run_that_names_no_counter_bonus_stays_dormant() {
    let mut r = rig(1, vec![], 1);
    for _ in 0..4 {
        r.evade();
        r.keep_dash_strike(0);
        let (_, mut events) = r.press();
        events.extend(r.finish(0, vec![]).1);
        r.near_miss(0, 0);
        assert_eq!(r.combat(0), (0, 0));
        assert_eq!(count(&events, "CounterWindowOpened"), 0);
        assert_eq!(count(&events, "CounterBonusTaken"), 0);
    }
    assert!(r.view(0).get("counter").is_none());
}

/// B2: a near miss opens the window for `window_ticks`, with the perfect evade's cooldown running
/// too (no second `PerfectEvade`), and again while a window is open.
#[test]
fn a_near_miss_opens_the_window_even_in_the_cooldown() {
    let mut r = rig(1, vec![counter_kind(all_effects())], 0);
    let events = r.near_miss(0, 0);
    assert_eq!(r.combat(0), (30, 0));
    assert_eq!(count(&events, "CounterWindowOpened"), 1);
    assert_eq!(count(&events, "PerfectEvade"), 1);
    r.idle(5);
    assert_eq!(r.combat(0).0, 25);
    let events = r.near_miss(0, 300);
    assert_eq!(r.combat(0), (30, 0));
    assert_eq!(count(&events, "CounterWindowOpened"), 1);
    assert_eq!(count(&events, "PerfectEvade"), 0);
    // A hit's residue (more invulnerability than the evade has left) opens nothing.
    r.set_window(0, 0);
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    state.players[ME].combat.perfect_window = RULE.window_ticks;
    state.players[ME].invulnerable = world.player.evade.invulnerable_ticks + 20;
    let mut events = Vec::new();
    super::player::near_miss(world, state, ME, &mut events);
    assert_eq!(state.players[ME].combat.counter_window, 0);
    assert!(events.is_empty());
}

/// B2: a run whose rules name no perfect evade has a counter that never opens.
#[test]
fn without_a_perfect_evade_the_window_never_opens() {
    let mut r = rig(1, vec![counter_kind(all_effects())], 0);
    if let Run::V2 { world, .. } = r.e.run_mut() {
        Arc::make_mut(world).tuning.perfect_evade = None;
    }
    let events = r.near_miss(0, 0);
    assert_eq!(r.combat(0), (0, 0));
    assert!(events.is_empty());
}

/// B3: the window counts down one a tick and a dash-branch press takes it on its last tick and
/// not after; B4: the take closes the window and marks the step.
#[test]
fn the_window_lasts_its_ticks_and_the_take_closes_it() {
    let take_after = |idle: u32| {
        let mut r = rig(1, vec![counter_kind(all_effects())], 1);
        r.set_window(0, 30);
        for k in 0..idle {
            r.keep_dash_strike(0);
            r.step([Input::default(); 3]);
            assert_eq!(u32::from(r.combat(0).0), 30 - k - 1);
        }
        r.keep_dash_strike(0);
        let (_, events) = r.press();
        (r.combat(0), count(&events, "CounterBonusTaken"))
    };
    assert_eq!(take_after(0), ((0, 1), 1));
    assert_eq!(take_after(29), ((0, 1), 1));
    // Thirty ticks on, the window is gone: no take, and it stays shut.
    assert_eq!(take_after(30), ((0, 0), 0));
}

/// B4: an ordinary combo step, a later step of the dash branch and a press with no window take
/// nothing, and an ordinary step leaves the open window running.
#[test]
fn only_the_first_dash_branch_step_takes_the_bonus() {
    let mut r = rig(1, vec![counter_kind(all_effects())], 1);
    r.set_window(0, 30);
    // No dash-strike left: the ordinary combo.
    st(&mut r.e).players[ME].combo.dash_strike_left = 0;
    let (_, events) = r.press();
    assert_eq!(count(&events, "CounterBonusTaken"), 0);
    assert_eq!(r.combat(0), (29, 0));
    r.finish(0, vec![]);
    // The dash branch with no window.
    let mut r = rig(1, vec![counter_kind(all_effects())], 1);
    r.keep_dash_strike(0);
    let (_, events) = r.press();
    assert_eq!(count(&events, "CounterBonusTaken"), 0);
    assert_eq!(r.combat(0), (0, 0));
}

/// B5: the mark lasts as long as the step and no longer; a down empties the window and the mark.
#[test]
fn the_mark_ends_with_the_step_and_a_down_empties_both() {
    let mut r = rig(1, vec![counter_kind(all_effects())], 1);
    r.set_window(0, 30);
    r.keep_dash_strike(0);
    r.press();
    assert_eq!(r.combat(0).1, 1);
    r.step([Input::default(); 3]);
    assert_eq!(r.combat(0).1, 1, "mid-step");
    r.finish(0, vec![]);
    assert_eq!(r.combat(0), (0, 0), "the step is over");
    // An evade pressed mid-step ends the mark.
    r.set_window(0, 30);
    r.keep_dash_strike(0);
    r.press();
    assert_eq!(r.combat(0).1, 1);
    r.evade();
    assert_eq!(r.combat(0).1, 0);
    // A near miss during the taken step reopens the window and the mark stays.
    r.set_window(0, 30);
    r.keep_dash_strike(0);
    st(&mut r.e).players[ME].combo.dash = false;
    r.press();
    r.near_miss(0, 300);
    assert_eq!(r.combat(0), (30, 1));
    // A down.
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    super::player::stand_down(world, state, ME, &mut Vec::new());
    assert_eq!(state.players[ME].combat.counter_window, 0);
    assert_eq!(state.players[ME].combat.counter_bonus_step, 0);
}

/// B6: the plain dash step hits for D, the one that took the bonus for twice that.
#[test]
fn the_taken_step_hits_for_double() {
    let dealt = |window: u16| {
        let mut r = rig(1, vec![counter_kind(all_effects())], 1);
        r.set_window(0, window);
        r.keep_dash_strike(0);
        let (lost, events) = r.press();
        r.finish(lost, events).0
    };
    let plain = dealt(0);
    assert!(plain > 0, "the step hits");
    assert_eq!(dealt(30), plain * 2);
}

/// B8: a regular enemy the bonus step hits is stunned for the bonus's ticks, once; the plain step
/// stuns nothing.
#[test]
fn the_taken_step_stuns_the_enemy_it_hits() {
    let stuns = |window: u16, kinds: Vec<Value>| {
        let mut r = rig(1, kinds, 1);
        r.set_window(0, window);
        r.keep_dash_strike(0);
        let (lost, events) = r.press();
        let (_, events) = r.finish(lost, events);
        named(&events, "StatusApplied")
            .iter()
            .filter(|d| d["duration_ticks"] == 45)
            .count()
    };
    assert_eq!(stuns(30, vec![counter_kind(all_effects())]), 1);
    assert_eq!(stuns(0, vec![counter_kind(all_effects())]), 0);
    let only_damage = counter_kind(json!([{ "type": "damage_pct", "pct": 100 }]));
    assert_eq!(stuns(30, vec![only_damage]), 0);
}

/// B13: the view carries the window, the style's window and the mark, closed included.
#[test]
fn the_view_reads_the_two_fields() {
    let mut r = rig(1, vec![counter_kind(all_effects())], 1);
    let want =
        |left: u32, taken: bool| json!({ "window_left": left, "window_ticks": 30, "taken": taken });
    assert_eq!(r.view(0)["counter"], want(0, false));
    r.set_window(0, 12);
    assert_eq!(r.view(0)["counter"], want(12, false));
    r.set_window(0, 30);
    r.keep_dash_strike(0);
    r.press();
    assert_eq!(r.view(0)["counter"], want(0, true));
    r.near_miss(0, 300);
    assert_eq!(r.view(0)["counter"], want(30, true));
}

/// B11: three players each follow their own window and mark.
#[test]
fn each_player_keeps_its_own_window() {
    let mut r = rig(3, vec![counter_kind(all_effects())], 1);
    for i in 0..3 {
        r.near_miss(i, 0);
    }
    for i in 0..3 {
        assert_eq!(r.combat(i), (30, 0));
    }
    r.idle(3);
    st(&mut r.e).players[slot(1)].combo.dash_strike_left = 1_000;
    let press = Input {
        main: true,
        ..Input::default()
    };
    let (_, events) = r.step([Input::default(), press, Input::default()]);
    assert_eq!(r.combat(0), (26, 0));
    assert_eq!(r.combat(1), (0, 1));
    assert_eq!(r.combat(2), (26, 0));
    assert_eq!(
        named(&events, "CounterBonusTaken"),
        vec![json!({ "player": "second" })]
    );
}

/// B12: the same scenario twice gives the same hashes.
#[test]
fn the_scenario_is_deterministic() {
    let run = || {
        let mut r = rig(1, vec![counter_kind(all_effects())], 1);
        let mut hashes = Vec::new();
        r.near_miss(0, 0);
        r.keep_dash_strike(0);
        r.press();
        for _ in 0..120 {
            r.step([Input::default(); 3]);
            hashes.push(r.e.state_hash());
        }
        hashes
    };
    assert_eq!(run(), run());
}

/// B7: every gauge fill credited to a player whose step took the bonus is scaled by the factor
/// (20 becomes 60), a fill of another player and a fill after the step are not.
#[test]
fn the_taken_step_scales_the_fills_it_credits() {
    use super::state::{EliteState, Stagger};
    let mut r = rig(2, vec![counter_kind(all_effects())], 1);
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let def = state.enemies[0].def;
    let w = Arc::make_mut(world);
    w.defs.agent_mut(def).stagger_max = Some(1_000);
    w.has_stagger = true;
    state.enemies[0].elite = Some(EliteState {
        home: state.enemies[0].at,
        called: false,
        boss_call: Default::default(),
        disguised: false,
        stagger: Stagger::default(),
    });
    let gauge = |s: &State| s.enemies[0].elite.as_ref().unwrap().stagger.gauge;
    let fill = |state: &mut State, me: u8| {
        state.enemies[0].elite.as_mut().unwrap().stagger.gauge = 0;
        super::stun::fill(
            world,
            state,
            super::entity::Who::Enemy(0),
            slot(me),
            20,
            &mut Vec::new(),
        );
        gauge(state)
    };
    assert_eq!(fill(state, 0), 20, "no mark");
    state.players[slot(0)].combat.counter_bonus_step = 1;
    assert_eq!(fill(state, 0), 60, "the mark");
    assert_eq!(fill(state, 1), 20, "another player's fill");
    state.players[slot(0)].combat.counter_bonus_step = 0;
    assert_eq!(fill(state, 0), 20, "after the step");
}

/// B9: the first phase of the taken step slides the player toward its target by the grant's
/// distance; the plain step slides nothing.
#[test]
fn the_taken_step_slides_toward_its_target() {
    let slid = |window: u16| {
        let mut r = rig(1, vec![counter_kind(all_effects())], 1);
        r.set_window(0, window);
        r.keep_dash_strike(0);
        let start = st(&mut r.e).players[ME].at;
        let facing = st(&mut r.e).players[ME].facing;
        let press = Input {
            main: true,
            assist: true,
            ..Input::default()
        };
        // The target stands 100 px ahead along the facing.
        let at = |s: &State| s.players[ME].at;
        let place = |e: &mut Engine| {
            let s = st(e);
            let (p, scale) = (s.players[ME].at, i64::from(crate::trig::SCALE));
            let ahead = i64::from(Fx::px(100).0);
            s.enemies[0].at = Point {
                x: Fx(p.x.0 + i32::try_from(i64::from(facing.x) * ahead / scale).unwrap()),
                y: Fx(p.y.0 + i32::try_from(i64::from(facing.y) * ahead / scale).unwrap()),
            };
            s.enemies[0].hp = BIG;
        };
        place(&mut r.e);
        r.e.step(&[press, Input::default(), Input::default()]);
        for _ in 0..12 {
            place(&mut r.e);
            r.e.step(&[Input::default(); 3]);
        }
        let end = at(st(&mut r.e));
        let (dx, dy) = (
            i64::from(end.x.0 - start.x.0),
            i64::from(end.y.0 - start.y.0),
        );
        dx * dx + dy * dy
    };
    assert_eq!(slid(0), 0, "the plain step slides nothing");
    let moved = slid(30);
    assert!(moved > 0, "the taken step slides");
    let px48 = i64::from(Fx::px(48).0);
    assert!(moved <= px48 * px48, "no further than the distance");
}

/// B10: under `try_styles` a style pick onto another style zeroes the window and the mark; the
/// same style again keeps them.
#[test]
fn leaving_the_style_empties_the_window_and_the_mark() {
    use crate::input::EquipAction;
    let pick = |option: u32| {
        let mut r = rig(1, vec![counter_kind(all_effects())], 0);
        let Run::V2 { world, state } = r.e.run_mut() else {
            panic!("the live rules");
        };
        Arc::make_mut(world).stage.try_styles = true;
        let g = state
            .players
            .career_mut(ME)
            .expertise
            .as_mut()
            .expect("a gauge");
        g.level = 99;
        let c = &mut state.players[ME].combat;
        c.counter_window = 12;
        c.counter_bonus_step = 1;
        r.e.act(ME, EquipAction::TreePick { option })
            .expect("a pick");
        r.combat(0)
    };
    assert_eq!(pick(0), (12, 1), "the same style again");
    assert_eq!(pick(1), (0, 0), "another style");
}
