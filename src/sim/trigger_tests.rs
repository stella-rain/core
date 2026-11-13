//! The giant trigger a style grants (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SIM-7, ADR-SIM-8): the
//! full-gauge smash starts a timed state, it counts down, widens the combo's hit boxes,
//! strengthens its smashes, gives super armour through the one `player::super_armour`, and ends
//! when the player is downed. Every number is a test value, not content: 480 ticks, 40% range, 20%
//! smash.

use super::state::{PlayerIdx, State};
use super::world::World;
use super::{Blame, Culprit, Shot, hurt_player};
use crate::engine::{Engine, Run};
use crate::event::{DomainEvent, HitKind};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use crate::pack::Pack;
use crate::run_setup::RunSetup;
use crate::run_stats::StatEffect;
use serde_json::{Value, json};

pub(super) const ME: PlayerIdx = PlayerIdx::SOLO;
const BIG: u32 = 1_000_000_000;
const TICKS: u16 = 480;

pub(super) fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// The trigger kind of the design table's Hammer (test values).
pub(super) fn trigger_kind(effects: Vec<Value>) -> Value {
    json!({ "type": "giant_trigger", "ticks": TICKS, "effects": effects })
}

pub(super) fn full_effects() -> Vec<Value> {
    vec![
        json!({ "type": "range_pct", "pct": 40 }),
        json!({ "type": "smash_pct", "pct": 20 }),
        json!({ "type": "super_armour" }),
    ]
}

/// The full-gauge attack: one wind-up tick, two active ticks and a recovery.
fn full_attack() -> Value {
    json!({ "inline": [
        { "kind": "wind_up", "ticks": 1 },
        { "kind": "active", "ticks": 2, "type": "slash", "damage": 100,
          "hit_box": { "type": "circle", "radius": 40 * 256,
                       "offset": { "forward": 0, "right": 0 } } },
        { "kind": "recovery", "ticks": 6 }
    ] })
}

/// The bundled pack with a gauge (one A hit fills it, spend `full_smash`) and `extra` kinds on the
/// Hammer's first style.
fn pack(extra: Vec<Value>) -> Pack {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_hammer")
        .expect("the Hammer's expertise");
    w["base"] = json!([]);
    for style in w["styles"].as_array_mut().unwrap() {
        style["stats"] = json!([]);
    }
    let mut kinds = vec![json!({
        "type": "gauge", "max": 100, "per_step": 100, "step_cap": 100,
        "spend": { "type": "full_smash", "attack": full_attack() }
    })];
    kinds.extend(extra);
    w["styles"][0]["kinds"] = json!(kinds);
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack")
}

pub(super) fn st(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

pub(super) fn world(e: &mut Engine) -> &mut World {
    match e.run_mut() {
        Run::V2 { world, .. } => std::sync::Arc::make_mut(world),
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// One Hammer run of `players` on style 0, `foes` enemies of defence 0 stacked in front of the
/// first player each tick, every player of 1,000 hit points and no defence.
pub(super) struct Rig {
    pub e: Engine,
    foes: Vec<EntityId>,
    pub events: Vec<Value>,
}

pub(super) fn rig(players: u8, extra: Vec<Value>, foes: u32) -> Rig {
    let pack = pack(extra);
    let setup = RunSetup {
        mode: "time_limit".into(),
        weapon: "giant_hammer".into(),
        seed: 20_261_115,
        ..RunSetup::default()
    };
    let mut e = Engine::start_party_with(&setup, pack.content(), players).expect("a run");
    while !st(&mut e).enemies.iter().any(|x| x.hp > 0 && !x.hidden()) {
        e.step(&[Input::default(); 3]);
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
        p.hp = 1_000;
        p.max_hp = 1_000;
        p.defence = 0;
        p.invulnerable = 0;
        if let Some(g) = s.players.career_mut(slot(i)).expertise.as_mut() {
            g.style = Some(0);
        }
        if let Some(p) = s.players.career_mut(slot(i)).progress.as_mut() {
            p.totals[StatEffect::AttackSpeedPct as usize] = 0;
            p.set_tenths[StatEffect::AttackSpeedPct as usize] = 0;
        }
    }
    Rig {
        e,
        foes: ids,
        events: Vec::new(),
    }
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

    /// One tick of `inputs`; its events join the log.
    pub fn step(&mut self, inputs: [Input; 3]) -> Vec<Value> {
        self.stack();
        self.e.step(&inputs);
        let ev = as_json(self.e.events());
        self.events.extend(ev.iter().cloned());
        ev
    }

    pub fn idle(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step([Input::default(); 3]);
        }
    }

    /// Player 0's press of A (`main`) or of the smash button.
    pub fn press(&mut self, smash: bool) -> Vec<Value> {
        self.press_by(0, smash)
    }

    /// Player `i`'s press of A or of the smash button, the others idle.
    pub fn press_by(&mut self, i: usize, smash: bool) -> Vec<Value> {
        let mut inputs = [Input::default(); 3];
        inputs[i] = Input {
            main: !smash,
            smash,
            ..Input::default()
        };
        self.step(inputs)
    }

    pub fn busy(&mut self) -> bool {
        let s = st(&mut self.e);
        s.runs.iter().any(|r| super::attacks::is_combo(s, r))
    }

    /// Presses and runs on until the step has ended: the events of those ticks.
    pub fn run_step(&mut self, smash: bool) -> Vec<Value> {
        let mut events = self.press(smash);
        let mut ticks = 1;
        while self.busy() && ticks < 300 {
            events.extend(self.step([Input::default(); 3]));
            ticks += 1;
        }
        events
    }

    /// Presses, then runs on until the combo's attack is running: its `(width_pct, atk_up_pct)`.
    pub fn started(&mut self, smash: bool) -> (u32, u32) {
        self.press(smash);
        for _ in 0..120 {
            let s = st(&mut self.e);
            if let Some(r) = s.runs.iter().find(|r| super::attacks::is_combo(s, r)) {
                return (r.width_pct, r.atk_up_pct);
            }
            self.step([Input::default(); 3]);
        }
        panic!("no combo attack started");
    }

    pub fn trigger(&mut self, i: u8) -> u16 {
        st(&mut self.e).players[slot(i)].combat.giant_trigger
    }

    pub fn set_trigger(&mut self, i: u8, ticks: u16) {
        st(&mut self.e).players[slot(i)].combat.giant_trigger = ticks;
    }

    pub fn fill_gauge(&mut self, i: u8) {
        st(&mut self.e).players[slot(i)].combat.weapon_gauge = 100;
    }

    pub fn view(&self, i: usize) -> Value {
        serde_json::to_value(&self.e.snapshot().players[i]).expect("a view")
    }

    pub fn armoured(&mut self, i: u8) -> bool {
        let Run::V2 { world, state } = self.e.run_mut() else {
            panic!("the live rules");
        };
        super::player::super_armour(world, state, slot(i))
    }

    /// A hit of `damage` on player `i`, as the hit pass reports it.
    pub fn hurt(&mut self, i: u8, damage: u32, breaks_charge: bool) -> Vec<Value> {
        let Run::V2 { world, state } = self.e.run_mut() else {
            panic!("the live rules");
        };
        let shot = Shot {
            at: state.players[slot(i)].at,
            owner: EntityId(99),
            friendly: false,
            damage,
            damage_type: None,
            breaks_charge,
            smash: None,
            combo: false,
            rune: None,
            chain: false,
        };
        let by = Blame {
            kind: HitKind::Attack,
            by: Culprit::Run(crate::sim::world::AttackIndex(0)),
        };
        let mut events = Vec::new();
        hurt_player(world, state, slot(i), shot, by, &mut events);
        let ev = as_json(&events);
        self.events.extend(ev.iter().cloned());
        ev
    }
}

pub(super) fn as_json(events: &[DomainEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|e| serde_json::to_value(e).expect("an event"))
        .collect()
}

pub(super) fn count(events: &[Value], ty: &str) -> usize {
    events.iter().filter(|e| e["type"] == ty).count()
}

pub(super) fn named(events: &[Value], ty: &str) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e["type"] == ty)
        .map(|e| e["data"].clone())
        .collect()
}

fn trigger_events(events: &[Value]) -> usize {
    [
        "TriggerStarted",
        "TriggerEnded",
        "RallyGained",
        "RallyWon",
        "RallyLapsed",
    ]
    .iter()
    .map(|t| count(events, t))
    .sum()
}

/// S5: the kind changes no step: the style's tree equals the base's.
#[test]
fn the_kind_leaves_the_tree_equal_to_the_bases() {
    use crate::expertise::weapon_with_style;
    let pack = pack(vec![trigger_kind(full_effects())]);
    let content = pack.content();
    let base = content
        .weapons()
        .tree("giant_hammer", None)
        .expect("the Hammer's base tree");
    let style = &content
        .expertise()
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_hammer")
        .expect("the Hammer")
        .styles[0];
    for taken in 0..=3 {
        assert_eq!(
            &weapon_with_style(base, style, taken),
            base,
            "taken {taken}"
        );
    }
}

/// B1: a run that names no trigger and no rally never holds either, shows neither and says nothing.
#[test]
fn a_run_that_names_no_trigger_stays_dormant() {
    let mut r = rig(1, vec![], 1);
    assert!(!world(&mut r.e).has_trigger);
    assert!(world(&mut r.e).tuning.rally.is_none());
    for t in 0..600 {
        r.fill_gauge(0);
        r.press(t % 3 == 0);
        let c = st(&mut r.e).players[ME].combat;
        assert_eq!(
            (c.giant_trigger, c.rally, c.rally_ticks),
            (0, 0, 0),
            "tick {t}"
        );
        let v = r.view(0);
        assert!(v.get("trigger").is_none() && v.get("rally").is_none());
        r.hurt(0, 3, true);
    }
    assert_eq!(trigger_events(&r.events), 0);
}

/// B2: a full-gauge smash starts the state after its own attack began; a plain smash does not; a
/// second one restarts the count.
#[test]
fn the_full_gauge_smash_starts_the_state() {
    let mut r = rig(1, vec![trigger_kind(full_effects())], 1);
    assert!(world(&mut r.e).has_trigger);
    // Below full: nothing starts.
    r.fill_gauge(0);
    st(&mut r.e).players[ME].combat.weapon_gauge = 50;
    let ev = r.press(true);
    assert_eq!(count(&ev, "TriggerStarted"), 0);
    assert_eq!(r.trigger(0), 0);
    r.idle(60);
    // At full: it starts at the press.
    r.fill_gauge(0);
    let ev = r.press(true);
    assert_eq!(
        named(&ev, "TriggerStarted"),
        [json!({ "player": "first", "ticks": 480 })]
    );
    assert_eq!(r.trigger(0), 480);
    r.idle(1);
    assert_eq!(r.trigger(0), 479);
    // A second full-gauge smash restarts the count.
    r.idle(100);
    while r.busy() {
        r.idle(1);
    }
    r.fill_gauge(0);
    let ev = r.press(true);
    assert_eq!(count(&ev, "TriggerStarted"), 1);
    assert_eq!(r.trigger(0), 480);
}

/// B2: the slam's own attack carries none of the state's range or damage.
#[test]
fn the_slam_itself_is_plain() {
    let mut plain = rig(1, vec![], 1);
    plain.fill_gauge(0);
    let want = plain.started(true);
    let mut r = rig(1, vec![trigger_kind(full_effects())], 1);
    r.fill_gauge(0);
    assert_eq!(r.started(true), want);
    assert!(r.trigger(0) > 0);
}

/// B3: the state runs its ticks, ends once, and never exceeds them.
#[test]
fn the_state_counts_down_and_ends_once() {
    let mut r = rig(1, vec![trigger_kind(full_effects())], 0);
    r.set_trigger(0, 480);
    let mut last = 480;
    for t in 1..=480 {
        let ev = r.step([Input::default(); 3]);
        let now = r.trigger(0);
        assert!(now < last);
        last = now;
        assert_eq!(
            count(&ev, "TriggerEnded"),
            usize::from(t == 480),
            "tick {t}"
        );
    }
    assert_eq!(r.trigger(0), 0);
    r.idle(10);
    assert_eq!(count(&r.events, "TriggerEnded"), 1);
}

/// B4: while it runs the next steps start wider and a smash hits harder; a style with only
/// `super_armour` changes neither; after it ends the steps are plain.
#[test]
fn the_state_widens_steps_and_strengthens_smashes() {
    let (w0, up0) = rig(1, vec![trigger_kind(full_effects())], 1).started(false);
    let (sw0, sup0) = rig(1, vec![trigger_kind(full_effects())], 1).started(true);
    let mut r = rig(1, vec![trigger_kind(full_effects())], 1);
    r.set_trigger(0, 480);
    let (w, up) = r.started(false);
    assert_eq!((w, up), (w0 + 40, up0), "an A step: wider, same damage");
    let mut r = rig(1, vec![trigger_kind(full_effects())], 1);
    r.set_trigger(0, 480);
    let (sw, sup) = r.started(true);
    assert_eq!(
        (sw, sup),
        (sw0 + 40, sup0 + 20),
        "a smash: wider and stronger"
    );
    let mut r = rig(
        1,
        vec![trigger_kind(vec![json!({ "type": "super_armour" })])],
        1,
    );
    r.set_trigger(0, 480);
    assert_eq!(r.started(true), (sw0, sup0), "armour alone changes no step");
}

/// B5: the state gives super armour through the one function: no charge, no smash, the view too,
/// and a `breaks_charge` hit leaves a charge unbroken (the damage still lands).
#[test]
fn the_state_gives_super_armour() {
    let mut r = rig(1, vec![trigger_kind(full_effects())], 0);
    assert!(!r.armoured(0));
    st(&mut r.e).players[ME].combo.charge = Some(0);
    let ev = r.hurt(0, 10, true);
    assert_eq!(count(&ev, "PlayerHit"), 1);
    assert!(
        st(&mut r.e).players[ME].combo.charge.is_none(),
        "plain: broken"
    );

    st(&mut r.e).players[ME].combo.charge = Some(0);
    r.set_trigger(0, 5);
    assert!(r.armoured(0));
    assert_eq!(r.view(0)["super_armour"], true);
    let before = st(&mut r.e).players[ME].hp;
    let ev = r.hurt(0, 10, true);
    assert_eq!(count(&ev, "PlayerHit"), 1);
    assert!(st(&mut r.e).players[ME].hp < before, "the damage lands");
    assert!(st(&mut r.e).players[ME].combo.charge.is_some(), "unbroken");

    r.set_trigger(0, 0);
    assert!(!r.armoured(0));
    r.hurt(0, 10, true);
    assert!(
        st(&mut r.e).players[ME].combo.charge.is_none(),
        "after: broken"
    );

    let mut none = rig(
        1,
        vec![trigger_kind(vec![json!({ "type": "range_pct", "pct": 5 })])],
        0,
    );
    none.set_trigger(0, 5);
    assert!(!none.armoured(0), "no super_armour effect");
}

/// B6: a down ends the state at once, with one event, and a player with 0 ticks left says nothing.
#[test]
fn a_down_ends_the_state() {
    let mut r = rig(2, vec![trigger_kind(full_effects())], 0);
    r.set_trigger(0, 100);
    let ev = r.hurt(0, 1_000_000, false);
    assert_eq!(count(&ev, "PlayerDowned"), 1);
    assert_eq!(count(&ev, "TriggerEnded"), 1);
    assert_eq!(r.trigger(0), 0);
    let ev = r.hurt(1, 1_000_000, false);
    assert_eq!(count(&ev, "PlayerDowned"), 1);
    assert_eq!(count(&ev, "TriggerEnded"), 0, "player 1 held none");
}

/// B8: the view follows the field, at 0 too; a style with no trigger shows no key.
#[test]
fn the_view_follows_the_state() {
    let mut r = rig(1, vec![trigger_kind(full_effects())], 0);
    assert_eq!(
        r.view(0)["trigger"],
        json!({ "ticks_left": 0, "ticks": 480 })
    );
    r.set_trigger(0, 200);
    r.idle(1);
    assert_eq!(
        r.view(0)["trigger"],
        json!({ "ticks_left": 199, "ticks": 480 })
    );
    let none = rig(1, vec![], 0);
    assert!(none.view(0).get("trigger").is_none());
}

/// B17: in a party each player starts and holds its own state.
#[test]
fn each_player_has_their_own_state() {
    let mut r = rig(3, vec![trigger_kind(full_effects())], 0);
    r.fill_gauge(1);
    let ev = r.press_by(1, true);
    assert_eq!(
        named(&ev, "TriggerStarted"),
        [json!({ "player": "second", "ticks": 480 })]
    );
    assert_eq!((r.trigger(0), r.trigger(1), r.trigger(2)), (0, 480, 0));
    assert!(r.armoured(1) && !r.armoured(0) && !r.armoured(2));
}

/// B18: the same scenario twice gives the same state hash every tick.
#[test]
fn the_state_is_deterministic() {
    let run = || {
        let mut r = rig(1, vec![trigger_kind(full_effects())], 1);
        let mut hashes = Vec::new();
        for t in 0..300 {
            if t % 40 == 0 {
                r.fill_gauge(0);
            }
            r.press(t % 40 == 0 || t % 7 == 0);
            hashes.push(r.e.state_hash());
        }
        hashes
    };
    assert_eq!(run(), run());
}
