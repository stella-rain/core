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
pub(super) fn full_attack_value() -> Value {
    full_attack()
}

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
    levelled_pack(extra, vec![], vec![], vec![])
}

/// `pack(extra)` with the kinds of the style's first upgrade, second upgrade and capstone set too.
pub(super) fn levelled_pack(
    extra: Vec<Value>,
    upgrade_1: Vec<Value>,
    upgrade_2: Vec<Value>,
    capstone: Vec<Value>,
) -> Pack {
    let v = levelled_value(extra, upgrade_1, upgrade_2, capstone);
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack")
}

/// The JSON of `levelled_pack`, before the pack check reads it.
pub(super) fn levelled_value(
    extra: Vec<Value>,
    upgrade_1: Vec<Value>,
    upgrade_2: Vec<Value>,
    capstone: Vec<Value>,
) -> Value {
    let mut v: Value = serde_json::from_slice(super::before::pack_bytes()).unwrap();
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
    for (field, kinds) in [
        ("upgrade_1", upgrade_1),
        ("upgrade_2", upgrade_2),
        ("capstone", capstone),
    ] {
        if !kinds.is_empty() {
            w["styles"][0][field][0]["effect"] = json!("none");
            w["styles"][0][field][0]
                .as_object_mut()
                .unwrap()
                .remove("amount");
            w["styles"][0][field][0]["kinds"] = json!(kinds);
        }
    }
    v
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
    /// How far in front of the first player the enemies are stacked each tick, in stored units.
    pub along: i32,
}

pub(super) fn rig(players: u8, extra: Vec<Value>, foes: u32) -> Rig {
    rig_levelled(players, extra, [vec![], vec![], vec![]], foes)
}

/// `rig` whose style also names `levels` kinds at its first upgrade, second upgrade and capstone.
pub(super) fn rig_levelled(
    players: u8,
    extra: Vec<Value>,
    levels: [Vec<Value>; 3],
    foes: u32,
) -> Rig {
    let [up_1, up_2, cap] = levels;
    rig_from(players, levelled_pack(extra, up_1, up_2, cap), foes)
}

/// `rig_levelled` over a pack already built.
pub(super) fn rig_from(players: u8, pack: Pack, foes: u32) -> Rig {
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
        along: Fx::px(24).0,
    }
}

impl Rig {
    fn stack(&mut self) {
        let s = st(&mut self.e);
        let (at, facing) = (s.players[ME].at, s.players[ME].facing);
        s.enemies.retain(|x| self.foes.contains(&x.id));
        let scale = i64::from(crate::trig::SCALE);
        let along = i64::from(self.along);
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
            dot: false,
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

/// A tempo kind of 5 stacks of 4%, with the armour at full stacks when `armour`.
fn tempo_kind(armour: bool) -> Value {
    let mut effects = vec![json!({ "type": "speed_pct", "pct": 4 })];
    if armour {
        effects.push(json!({ "type": "super_armour_at_full" }));
    }
    json!({ "type": "combo_tempo_pct", "max_stacks": 5, "effects": effects })
}

fn set_stacks(r: &mut Rig, stacks: u8, held: bool) {
    let p = &mut st(&mut r.e).players[ME];
    p.combat.combo_tempo = stacks;
    p.combo.hold = held.then_some(3);
}

/// B15: full stacks in a held hold are super armour, and only that armour shelters the stacks.
#[test]
fn full_stacks_in_a_hold_are_armour() {
    let mut r = rig(1, vec![tempo_kind(true)], 0);
    set_stacks(&mut r, 5, true);
    assert!(r.armoured(0));
    set_stacks(&mut r, 4, true);
    assert!(!r.armoured(0), "4 of 5 stacks");
    set_stacks(&mut r, 5, false);
    assert!(!r.armoured(0), "off the hold");
    let mut plain = rig(1, vec![tempo_kind(false)], 0);
    set_stacks(&mut plain, 5, true);
    assert!(!plain.armoured(0), "without the effect");

    // A hit under it keeps the hold and the stacks.
    set_stacks(&mut r, 5, true);
    let ev = r.hurt(0, 10, true);
    assert_eq!((count(&ev, "PlayerHit"), count(&ev, "TempoLost")), (1, 0));
    assert_eq!(st(&mut r.e).players[ME].combat.combo_tempo, 5);
    assert!(
        st(&mut r.e).players[ME].combo.hold.is_some(),
        "the hold is unbroken"
    );
    // At 4 stacks the same hit ends both.
    set_stacks(&mut r, 4, true);
    let ev = r.hurt(0, 10, true);
    assert_eq!(count(&ev, "TempoLost"), 1);
    assert!(st(&mut r.e).players[ME].combo.hold.is_none());
}

/// B15: any other armour (the trigger's) still does not shelter the stacks.
#[test]
fn other_armour_does_not_shelter_the_stacks() {
    let mut r = rig(1, vec![tempo_kind(true), trigger_kind(full_effects())], 0);
    set_stacks(&mut r, 5, false);
    r.set_trigger(0, 100);
    assert!(r.armoured(0));
    let ev = r.hurt(0, 10, true);
    assert_eq!(count(&ev, "TempoLost"), 1);
}

/// A hold of `move_pct` on the style's `main` step, as Momentum grants one (test values).
fn hold_grant(move_pct: u32) -> Value {
    let strike = |ticks: u32| {
        json!({ "inline": [{
            "kind": "active", "ticks": ticks, "type": "strike", "rehit_ticks": ticks,
            "hit_box": { "type": "circle", "radius": 12_288,
                         "offset": { "forward": 0, "right": 0 } }
        }] })
    };
    json!({
        "type": "replace_step", "step": ["main"],
        "grant": { "hold": {
            "loop": strike(8), "release": strike(5), "max_ticks": 600, "move_pct": move_pct
        } }
    })
}

/// A rig whose style grants a hold of `hold` on its `main` step and names a trigger with
/// `effects`.
fn hold_rig(players: u8, hold: u32, effects: Vec<Value>) -> Rig {
    rig(players, vec![hold_grant(hold), trigger_kind(effects)], 0)
}

fn hold_effect(pct: u32) -> Value {
    json!({ "type": "hold_move_pct", "pct": pct })
}

/// The x position of player `i`.
fn x_of(r: &mut Rig, i: u8) -> i32 {
    st(&mut r.e).players[slot(i)].at.x.0
}

/// Main held and the stick right for `players` listed in `walking`; true when `i` is in a hold.
fn held(r: &mut Rig, i: u8) -> bool {
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    super::player::hold_view(world, state, slot(i)).is_some()
}

/// Walks every player in `walking` with Main held until all are held, then returns each one's
/// step on one held tick.
fn held_steps(r: &mut Rig, walking: &[u8]) -> Vec<i32> {
    let input = Input {
        main: true,
        move_x: 127,
        ..Input::default()
    };
    let mut inputs = [Input::default(); 3];
    for &i in walking {
        inputs[usize::from(i)] = input;
    }
    for _ in 0..200 {
        r.step(inputs);
        if walking.iter().all(|&i| held(r, i)) {
            let before: Vec<i32> = walking.iter().map(|&i| x_of(r, i)).collect();
            r.step(inputs);
            assert!(walking.iter().all(|&i| held(r, i)), "still held");
            return walking
                .iter()
                .zip(before)
                .map(|(&i, b)| x_of(r, i) - b)
                .collect();
        }
    }
    panic!("no hold started");
}

/// The step of a free walk (Main released) on the same rig.
fn free_step(r: &mut Rig) -> i32 {
    let input = Input {
        move_x: 127,
        ..Input::default()
    };
    r.step([input, Input::default(), Input::default()]);
    let before = x_of(r, 0);
    r.step([input, Input::default(), Input::default()]);
    x_of(r, 0) - before
}

/// R1: a held hold moves at its own percent, and at the trigger's while the trigger runs.
#[test]
fn the_trigger_raises_the_hold_s_movement() {
    let free = free_step(&mut hold_rig(1, 50, vec![hold_effect(80)]));
    assert!(free > 0);
    let mut r = hold_rig(1, 50, vec![hold_effect(80)]);
    assert_eq!(held_steps(&mut r, &[0])[0], free * 50 / 100);
    let mut r = hold_rig(1, 50, vec![hold_effect(80)]);
    r.set_trigger(0, 100);
    assert_eq!(held_steps(&mut r, &[0])[0], free * 80 / 100);
}

/// R2: with no hold nothing changes, and a trigger without the effect leaves the hold at its own.
#[test]
fn the_effect_needs_a_hold_and_a_trigger_that_names_it() {
    let mut r = hold_rig(1, 50, vec![hold_effect(80)]);
    let free = free_step(&mut r);
    r.set_trigger(0, 100);
    assert_eq!(free_step(&mut r), free, "no hold, no change");
    let mut r = hold_rig(1, 50, full_effects());
    r.set_trigger(0, 100);
    assert_eq!(held_steps(&mut r, &[0])[0], free * 50 / 100);
}

/// R3: when the trigger counts out the hold is back at its own percent.
#[test]
fn the_raise_ends_with_the_trigger() {
    let free = free_step(&mut hold_rig(1, 50, vec![hold_effect(80)]));
    let mut r = hold_rig(1, 50, vec![hold_effect(80)]);
    r.set_trigger(0, 1);
    let ev = r.step([Input::default(); 3]);
    assert_eq!(count(&ev, "TriggerEnded"), 1);
    assert_eq!(held_steps(&mut r, &[0])[0], free * 50 / 100);
}

/// R4: the trigger raises a hold, never lowers it.
#[test]
fn the_trigger_never_lowers_a_faster_hold() {
    let free = free_step(&mut hold_rig(1, 90, vec![hold_effect(80)]));
    let mut r = hold_rig(1, 90, vec![hold_effect(80)]);
    r.set_trigger(0, 100);
    assert_eq!(held_steps(&mut r, &[0])[0], free * 90 / 100);
}

/// R5: each player's own trigger decides; the run is deterministic.
#[test]
fn each_player_s_hold_follows_its_own_trigger() {
    let free = free_step(&mut hold_rig(2, 50, vec![hold_effect(80)]));
    let mut r = hold_rig(2, 50, vec![hold_effect(80)]);
    r.set_trigger(0, 600);
    assert_eq!(
        held_steps(&mut r, &[0, 1]),
        [free * 80 / 100, free * 50 / 100]
    );
    let run = || {
        let mut r = hold_rig(2, 50, vec![hold_effect(80)]);
        r.set_trigger(0, 300);
        let input = Input {
            main: true,
            move_x: 127,
            ..Input::default()
        };
        for _ in 0..300 {
            r.step([input, input, Input::default()]);
        }
        r.e.state_hash()
    };
    assert_eq!(run(), run());
}

/// R6: a trigger naming the effect in a weapon with no hold moves the player exactly as one
/// without it.
#[test]
fn the_effect_does_nothing_for_a_player_without_a_hold() {
    let run = |effects: Vec<Value>| {
        let mut r = rig(1, vec![trigger_kind(effects)], 0);
        r.set_trigger(0, 600);
        for t in 0..600u32 {
            let input = Input {
                main: t % 7 == 0,
                move_x: if t % 50 < 25 { 127 } else { -127 },
                move_y: if t % 90 < 45 { 60 } else { -60 },
                ..Input::default()
            };
            r.step([input, Input::default(), Input::default()]);
        }
        (x_of(&mut r, 0), r.e.state_hash())
    };
    let mut with_effect = full_effects();
    with_effect.push(hold_effect(80));
    assert_eq!(run(with_effect), run(full_effects()));
}

// The end ring (the Titan Trigger capstone): when the count runs out with the player up, the
// slam's first active phase plays once in `TRIGGER_RING_SLOT`, free.

use super::attacks::Strike;
use super::player::TRIGGER_RING_SLOT;
use crate::attack::{AttackRef, PhaseKind};

fn ring_effects() -> Vec<Value> {
    let mut e = full_effects();
    e.push(json!({ "type": "end_ring" }));
    e
}

/// How many runs player `i` has in the ring's slot.
fn ring_runs(r: &mut Rig, i: u8) -> usize {
    let s = st(&mut r.e);
    s.runs
        .iter()
        .filter(|x| {
            x.slot == TRIGGER_RING_SLOT
                && crate::id::player_of(x.owner).map(|(p, _)| p) == Some(slot(i))
        })
        .count()
}

/// The hit points the stacked enemies lost this tick (`stack` refills them each tick).
fn lost(r: &mut Rig) -> u64 {
    let foes = r.foes.clone();
    st(&mut r.e)
        .enemies
        .iter()
        .filter(|x| foes.contains(&x.id))
        .map(|x| u64::from(BIG - x.hp))
        .sum()
}

/// Idles `ticks` ticks: the total hit points lost, the ticks that started a ring run, and the
/// `TriggerEnded` events seen.
fn watch(r: &mut Rig, ticks: u32) -> (u64, u32, usize) {
    let (mut total, mut starts, mut ended, mut before) = (0, 0, 0, 0);
    for _ in 0..ticks {
        let ev = r.step([Input::default(); 3]);
        ended += count(&ev, "TriggerEnded");
        total += lost(r);
        let now = ring_runs(r, 0);
        if now > before {
            starts += 1;
        }
        before = now;
    }
    (total, starts, ended)
}

/// Plays player 0's end ring once by hand (the control: no trigger, no end), as the rule would.
fn play_ring(r: &mut Rig) {
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let ring = world.ring_of(state, ME).cloned().expect("a ring");
    let facing = state.players[ME].facing;
    let damage = super::levels::attack(state, ME, world.player.atk);
    let strike = Strike {
        bonus_pct: 0,
        speed_pct: 0,
        range_pct: super::levels::range_pct(state, ME),
        smash: None,
    };
    let mut events = Vec::new();
    assert!(super::attacks::start_player(
        world,
        state,
        ME,
        TRIGGER_RING_SLOT,
        facing,
        &ring,
        damage,
        strike,
        &mut events
    ));
}

/// What the ring does to one enemy in the standard rig, played by hand with the trigger off.
fn control_loss(pack: Pack, along: i32) -> u64 {
    let mut r = rig_from(1, pack, 1);
    r.along = along;
    play_ring(&mut r);
    watch(&mut r, 12).0
}

fn ring_pack() -> Pack {
    levelled_pack(vec![trigger_kind(ring_effects())], vec![], vec![], vec![])
}

/// R1: the end plays one ring run in the ring's slot that hurts the enemy by what the ring does
/// alone; without the effect nothing plays.
#[test]
fn the_end_of_the_state_plays_the_ring_once() {
    let d = control_loss(ring_pack(), Fx::px(24).0);
    assert!(d > 0, "the ring hurts");
    let mut r = rig(1, vec![trigger_kind(ring_effects())], 1);
    r.set_trigger(0, 3);
    let (total, starts, ended) = watch(&mut r, 12);
    assert_eq!((starts, ended), (1, 1), "one ring, one end");
    assert_eq!(total, d);
    let mut plain = rig(1, vec![trigger_kind(full_effects())], 1);
    plain.set_trigger(0, 3);
    assert_eq!(watch(&mut plain, 12), (0, 0, 1));
}

/// R2: the ring is free: the gauge, the step fill and the combo slot are as they were; it has no
/// wind-up; no gauge is spent.
#[test]
fn the_ring_costs_nothing() {
    let mut r = rig(1, vec![trigger_kind(ring_effects())], 1);
    {
        let c = &mut st(&mut r.e).players[ME].combat;
        c.weapon_gauge = 40;
        c.gauge_step_fill = 7;
    }
    r.set_trigger(0, 3);
    let mut ran = false;
    for _ in 0..12 {
        r.step([Input::default(); 3]);
        if ring_runs(&mut r, 0) > 0 {
            ran = true;
            break;
        }
    }
    assert!(ran, "the ring ran");
    let c = st(&mut r.e).players[ME].combat;
    assert_eq!((c.weapon_gauge, c.gauge_step_fill), (40, 7));
    assert_eq!(count(&r.events, "GaugeSpent"), 0);
    // The combo slot is free while the ring runs: a step pressed now starts.
    r.started(false);
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let Some(AttackRef::Inline(phases)) = world.ring_of(state, ME) else {
        panic!("an inline ring");
    };
    assert!(
        phases.len() == 1 && phases[0].kind == PhaseKind::Active,
        "no wind-up"
    );
}

/// R3: a down does not play it; a second slam restarts the count and the ring waits for the new
/// end; a player who is out gets none.
#[test]
fn the_ring_waits_for_the_end_not_a_down_or_a_restart() {
    let mut r = rig(2, vec![trigger_kind(ring_effects())], 1);
    r.set_trigger(0, 100);
    let ev = r.hurt(0, 1_000_000, false);
    assert_eq!(count(&ev, "TriggerEnded"), 1);
    assert_eq!(watch(&mut r, 12), (0, 0, 0), "no ring on a down");

    let mut r = rig(1, vec![trigger_kind(ring_effects())], 1);
    r.fill_gauge(0);
    r.run_step(true);
    r.set_trigger(0, 40);
    r.fill_gauge(0);
    r.run_step(true);
    assert!(r.trigger(0) > 40, "the second slam restarted the count");
    r.set_trigger(0, 3);
    let (_, starts, ended) = watch(&mut r, 12);
    assert_eq!((starts, ended), (1, 1), "exactly one ring at the new end");

    let mut r = rig(1, vec![trigger_kind(ring_effects())], 1);
    r.set_trigger(0, 3);
    st(&mut r.e).players[ME].out = true;
    assert_eq!(watch(&mut r, 12).1, 0, "no ring for a player who is out");
}

fn orb(at: Point, breakable: bool) -> super::state::OrbState {
    super::state::OrbState {
        at,
        vx: Fx(0),
        vy: Fx(0),
        dir: super::state::Dir::DOWN,
        style: 0,
        owner: EntityId(5000),
        damage: 0,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        hook: None,
        breakable,
        life: 600,
        stop: Some(0),
        link: None,
        program: crate::sim::ProgramIndex(0),
    }
}

fn orbs_left(r: &mut Rig) -> usize {
    st(&mut r.e).orbs.len()
}

/// R4: an orb inside the ring's circle breaks by the ordinary swing rule; one outside it and inside
/// the slam's clear radius stays (the slam's clear, the control, takes it).
#[test]
fn the_ring_breaks_the_orbs_it_covers_and_clears_none() {
    let place = |r: &mut Rig, dx: i32| {
        let me = st(&mut r.e).players[ME].at;
        st(&mut r.e).orbs.push(orb(
            Point {
                x: Fx(me.x.0 + dx),
                y: me.y,
            },
            true,
        ));
    };
    let mut r = rig(1, vec![trigger_kind(ring_effects())], 0);
    world(&mut r.e).tuning.orb_clear_radius = 40_000;
    place(&mut r, 5_000);
    place(&mut r, 30_000);
    r.set_trigger(0, 3);
    r.idle(12);
    assert_eq!(
        count(&r.events, "OrbBroken"),
        1,
        "the orb inside the circle"
    );
    assert_eq!(orbs_left(&mut r), 1, "the far orb stays");

    let mut control = rig(1, vec![trigger_kind(ring_effects())], 0);
    world(&mut control.e).tuning.orb_clear_radius = 40_000;
    place(&mut control, 30_000);
    control.fill_gauge(0);
    control.press(true);
    control.idle(6);
    assert_eq!(orbs_left(&mut control), 0, "the slam's clear takes it");
}

/// R5: the ring's slot is no rune's and no missile's.
#[test]
fn the_ring_slot_is_no_runes() {
    let mut r = rig(1, vec![trigger_kind(ring_effects())], 1);
    r.set_trigger(0, 3);
    let mut seen = false;
    for _ in 0..12 {
        r.step([Input::default(); 3]);
        let s = st(&mut r.e);
        for x in s.runs.iter().filter(|x| x.slot == TRIGGER_RING_SLOT) {
            seen = true;
            assert!(super::runes::of_run(x).is_none());
        }
    }
    assert!(seen);
}

/// R6: the player's range widens the ring as it does the slam.
#[test]
fn the_ring_is_scaled_by_the_players_range() {
    // The slam's radius is 40 px; an enemy at 56 px is out of it and in it widened by 50 percent.
    let far = Fx::px(56).0;
    let mut wide = rig_from(1, ring_pack(), 1);
    wide.along = far;
    if let Some(p) = st(&mut wide.e).players.career_mut(ME).progress.as_mut() {
        p.totals[StatEffect::RangePct as usize] = 50;
    }
    wide.set_trigger(0, 3);
    assert!(watch(&mut wide, 12).0 > 0, "hit with range 50");
    let mut plain = rig_from(1, ring_pack(), 1);
    plain.along = far;
    plain.set_trigger(0, 3);
    assert_eq!(watch(&mut plain, 12).0, 0, "not hit with range 0");
}

/// R7: the ring follows the slam: a retuned radius and damage move it; the ring is compiled after
/// every other attack; a party ends on one hash and only the player whose trigger ended has a ring.
#[test]
fn a_retuned_slam_retunes_the_ring() {
    let retuned = |radius: i64, damage: i64| {
        let mut v = levelled_value(vec![trigger_kind(ring_effects())], vec![], vec![], vec![]);
        let w = v["expertise"]["weapons"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|w| w["weapon"] == "giant_hammer")
            .unwrap();
        let active = &mut w["styles"][0]["kinds"][0]["spend"]["attack"]["inline"][1];
        active["hit_box"]["radius"] = json!(radius);
        active["damage"] = json!(damage);
        Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the pack")
    };
    let near = Fx::px(27).0;
    let unchanged = control_loss(retuned(40 * 256, 100), near);
    let narrow = control_loss(retuned(5 * 256, 100), near);
    assert!(unchanged > 0 && narrow == 0, "{unchanged} {narrow}");
    assert!(
        control_loss(retuned(40 * 256, 300), near) > unchanged,
        "damage follows"
    );

    // Compiled last: the slam's index is the same with and without the effect, one attack more.
    let slam = AttackRef::Inline(match serde_json::from_value(full_attack()).unwrap() {
        AttackRef::Inline(p) => p,
        AttackRef::Preset { .. } => unreachable!(),
    });
    let mut with = rig(1, vec![trigger_kind(ring_effects())], 0);
    let mut without = rig(1, vec![trigger_kind(full_effects())], 0);
    let (a, la) = {
        let w = world(&mut with.e);
        (w.attack_index(&slam, None), w.attacks.len())
    };
    let (b, lb) = {
        let w = world(&mut without.e);
        (w.attack_index(&slam, None), w.attacks.len())
    };
    assert_eq!((a, la), (b, lb + 1));

    let run = || {
        let mut r = rig(2, vec![trigger_kind(ring_effects())], 1);
        r.set_trigger(0, 3);
        let (mut one, mut two) = (0, 0);
        for _ in 0..300 {
            r.step([Input::default(); 3]);
            one += ring_runs(&mut r, 0);
            two += ring_runs(&mut r, 1);
        }
        (one > 0, two, r.e.state_hash())
    };
    let (rang, none, hash) = run();
    assert!(rang && none == 0);
    assert_eq!(run().2, hash, "the same run twice");
}

// The bundled Titan Trigger's second upgrade grants an AAA hold: Momentum's numbers, copied, so
// the trigger's `hold_move_pct` 80 has a hold to move.

/// One bundled Hammer run of `players` on Titan Trigger (style 0), player `i` at `places[i]`.
fn titan_rig(places: &[usize]) -> Rig {
    let pack = Pack::from_bytes(Pack::BUNDLED).expect("the bundled pack");
    let mut r = rig_from(u8::try_from(places.len()).unwrap(), pack, 0);
    for (i, &n) in places.iter().enumerate() {
        super::stacking_tests::take(&mut r.e, u8::try_from(i).unwrap(), n);
    }
    r
}

/// The AAA press script of `tests/hold.rs`: A on tick 1, buffered A on 3 and 13, Main held from
/// the AAA's recovery on, the stick right. `t` counts from 1.
fn aaa_input(t: u32) -> Input {
    Input {
        main: t == 1 || (t >= 3 && t != 12),
        move_x: 127,
        ..Input::default()
    }
}

/// Runs the AAA script for the players in `walking` until all are held, `limit` ticks at most;
/// the tick it stopped on, or none.
fn drive(r: &mut Rig, walking: &[u8], limit: u32) -> Option<u32> {
    for t in 1..=limit {
        let mut inputs = [Input::default(); 3];
        for &i in walking {
            inputs[usize::from(i)] = aaa_input(t);
        }
        r.step(inputs);
        if walking.iter().all(|&i| held(r, i)) {
            return Some(t);
        }
    }
    None
}

/// One more held tick of the same inputs: each walking player's step.
fn held_tick(r: &mut Rig, walking: &[u8]) -> Vec<i32> {
    let before: Vec<i32> = walking.iter().map(|&i| x_of(r, i)).collect();
    let mut inputs = [Input::default(); 3];
    for &i in walking {
        inputs[usize::from(i)] = aaa_input(100);
    }
    r.step(inputs);
    assert!(walking.iter().all(|&i| held(r, i)), "still held");
    walking
        .iter()
        .zip(before)
        .map(|(&i, b)| x_of(r, i) - b)
        .collect()
}

/// The AAA step of player `i`'s weapon, as a value.
fn aaa_of(r: &mut Rig, i: u8) -> crate::weapon::Step {
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let w = world.weapon_of(state, slot(i)).expect("a weapon");
    let a = w.combo.main.as_ref().expect("A");
    let aa = a.next.main.as_ref().expect("AA");
    (**aa.next.main.as_ref().expect("AAA")).clone()
}

/// The free step of a walk on a fresh rig at `places`.
fn free_at(places: &[usize]) -> i32 {
    free_step(&mut titan_rig(places))
}

/// B1: the hold is in the tree at places 2 and 3 only, Momentum's, and replaces no attack.
#[test]
fn the_second_titan_upgrade_puts_a_hold_on_the_aaa() {
    let base = aaa_of(&mut titan_rig(&[0]), 0);
    assert!(base.hold.is_none() && base.charge.is_none());
    for places in 0..4 {
        let mut r = titan_rig(&[places]);
        let step = aaa_of(&mut r, 0);
        assert_eq!(
            step.attack, base.attack,
            "place {places}: the tap is the same"
        );
        assert_eq!(
            step.next, base.next,
            "place {places}: the next steps are the same"
        );
        assert!(step.charge.is_none(), "place {places}");
        if places < 2 {
            assert!(step.hold.is_none(), "place {places}: no hold yet");
            continue;
        }
        let hold = step.hold.expect("a hold at places 2 and 3");
        assert_eq!((hold.max_ticks, hold.move_pct), (180, 50), "place {places}");
        let got = serde_json::to_value(&hold).unwrap();
        let strike = json!([{
            "kind": "active", "ticks": 8, "type": "strike", "rehit_ticks": 8,
            "hit_box": { "type": "circle", "radius": 12_288,
                         "offset": { "forward": 0, "right": 0 } }
        }]);
        assert_eq!(got["loop"], json!({ "inline": strike }), "place {places}");
        assert_eq!(
            got["release"],
            json!({ "inline": [
                { "kind": "wind_up", "ticks": 10 },
                { "kind": "active", "ticks": 5, "type": "strike", "damage": 10_000,
                  "hit_box": { "type": "circle", "radius": 16_384,
                               "offset": { "forward": 4096, "right": 0 } } },
                { "kind": "recovery", "ticks": 24 }
            ] }),
            "place {places}"
        );
    }
}

/// B2: a held hold moves at 50%, at 80% while the trigger runs; places 0 and 1 never hold.
#[test]
fn the_titan_hold_moves_at_eighty_under_the_trigger() {
    // The control: Momentum's hold (style 1, place 0) is reached by the same script.
    let mut control = titan_rig(&[0]);
    if let Some(g) = st(&mut control.e).players.career_mut(ME).expertise.as_mut() {
        g.style = Some(1);
    }
    assert!(drive(&mut control, &[0], 200).is_some(), "the script holds");
    for places in [2, 3] {
        let free = free_at(&[places]);
        assert!(free > 0);
        let mut r = titan_rig(&[places]);
        assert!(drive(&mut r, &[0], 200).is_some(), "place {places}");
        assert_eq!(
            held_tick(&mut r, &[0])[0],
            free * 50 / 100,
            "place {places}"
        );
        let mut r = titan_rig(&[places]);
        r.set_trigger(0, 600);
        assert!(drive(&mut r, &[0], 200).is_some(), "place {places}");
        assert_eq!(
            held_tick(&mut r, &[0])[0],
            free * 80 / 100,
            "place {places}"
        );
    }
    for places in [0, 1] {
        for ticks in [0, 600] {
            let mut r = titan_rig(&[places]);
            r.set_trigger(0, ticks);
            assert_eq!(drive(&mut r, &[0], 200), None, "place {places}: no hold");
        }
    }
}

/// B3: the trigger's end plays the ring (place 3) and leaves the hold held, at 50% after.
#[test]
fn the_end_of_the_trigger_leaves_the_titan_hold_held() {
    for (places, ring) in [(3, 1), (2, 0)] {
        let free = free_at(&[places]);
        let mut r = titan_rig(&[places]);
        r.set_trigger(0, 600);
        assert!(drive(&mut r, &[0], 200).is_some(), "place {places}");
        let before = r.trigger(0);
        assert!(before > 1);
        r.set_trigger(0, 1);
        let held_before = st(&mut r.e).players[ME].combo.hold.expect("held");
        let ev = r.step([aaa_input(100), Input::default(), Input::default()]);
        assert_eq!(count(&ev, "TriggerEnded"), 1, "place {places}");
        assert_eq!(ring_runs(&mut r, 0), ring, "place {places}: the ring");
        assert!(held(&mut r, 0), "place {places}: still held");
        assert_eq!(
            st(&mut r.e).players[ME].combo.hold,
            Some(held_before + 1),
            "place {places}"
        );
        assert_eq!(
            held_tick(&mut r, &[0])[0],
            free * 50 / 100,
            "place {places}"
        );
    }
}

/// B4: a slam after a hold: the B press waits while Main is held, then takes the smash.
#[test]
fn the_slam_after_a_titan_hold_starts_the_trigger() {
    let free = free_at(&[2]);
    let mut r = titan_rig(&[2]);
    assert!(drive(&mut r, &[0], 200).is_some());
    r.fill_gauge(0);
    r.step([
        Input {
            main: true,
            smash: true,
            move_x: 127,
            ..Input::default()
        },
        Input::default(),
        Input::default(),
    ]);
    assert_eq!(r.trigger(0), 0, "buffered, not played");
    let mut events = Vec::new();
    for _ in 0..200 {
        events.extend(r.step([Input::default(); 3]));
        if count(&events, "GaugeSpent") > 0 && !r.busy() {
            break;
        }
    }
    assert_eq!(count(&events, "GaugeSpent"), 1);
    assert_eq!(st(&mut r.e).players[ME].combat.weapon_gauge, 0);
    assert!(r.trigger(0) > 0 && r.trigger(0) <= 720);
    // The next AAA under the trigger holds and moves at 80%.
    assert!(drive(&mut r, &[0], 200).is_some());
    assert_eq!(held_tick(&mut r, &[0])[0], free * 80 / 100);
}

/// B5: only this cell moved; each player reads its own place; the run is deterministic.
#[test]
fn only_the_second_titan_upgrade_moved() {
    for style in [1u8, 2] {
        for places in 0..4 {
            let mut r = titan_rig(&[places]);
            if let Some(g) = st(&mut r.e).players.career_mut(ME).expertise.as_mut() {
                g.style = Some(style);
            }
            let step = aaa_of(&mut r, 0);
            if style == 1 {
                assert_eq!(step.hold.map(|h| h.move_pct), Some(50), "momentum {places}");
            } else {
                assert!(step.hold.is_none(), "breaker {places}");
            }
            let smash = r_weapon(&mut r);
            assert_eq!(
                smash.contains("\"charge\""),
                style == 2,
                "style {style} place {places}: charge"
            );
        }
    }
    let free = free_at(&[2]);
    let mut r = titan_rig(&[2, 0]);
    r.set_trigger(0, 600);
    assert!(drive(&mut r, &[0], 200).is_some());
    assert_eq!(held_tick(&mut r, &[0])[0], free * 80 / 100);
    let mut r = titan_rig(&[2, 0]);
    r.set_trigger(0, 600);
    for t in 1..=200 {
        r.step([aaa_input(t), aaa_input(t), Input::default()]);
        assert!(!held(&mut r, 1), "the place-0 player never holds");
    }
    let run = || {
        let mut r = titan_rig(&[2]);
        r.set_trigger(0, 600);
        for t in 1..=600 {
            r.step([aaa_input(t), Input::default(), Input::default()]);
        }
        r.e.state_hash()
    };
    assert_eq!(run(), run());
}

/// Player 0's weapon, serialised.
fn r_weapon(r: &mut Rig) -> String {
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    serde_json::to_string(world.weapon_of(state, ME).expect("a weapon")).unwrap()
}
