//! The combo tempo a style grants (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SIM-7, ADR-SIM-8): each
//! swing of a combo step that hits adds a stack, each stack makes the attacks the combo starts
//! afterwards faster, and the stacks end when the combo resets, a smash is pressed, the player
//! is hit or downed. Every number is a test value, not content: up to 5 stacks of 4%.

use super::state::{PlayerIdx, State};
use super::{Blame, Culprit, Shot, hurt_player};
use crate::engine::{Engine, Run};
use crate::event::{DomainEvent, HitKind};
use crate::expertise::{Expertise, weapon_with_style};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use crate::pack::Pack;
use crate::run_setup::RunSetup;
use serde_json::{Value, json};

const ME: PlayerIdx = PlayerIdx::SOLO;
const BIG: u32 = 1_000_000_000;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// A tempo kind of `max` stacks of `pct` percent, keeping them on a full-stack smash if `keep`.
fn tempo_kind(max: u32, pct: u32, keep: bool) -> Value {
    let mut effects = vec![json!({ "type": "speed_pct", "pct": pct })];
    if keep {
        effects.push(json!({ "type": "keep_on_smash" }));
    }
    json!({ "type": "combo_tempo_pct", "max_stacks": max, "effects": effects })
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
    // No set stat of its own: the attack speed is the tempo's alone.
    w["base"] = json!([]);
    for style in w["styles"].as_array_mut().unwrap() {
        style["stats"] = json!([]);
    }
    w["styles"][0]["kinds"] = json!(kinds);
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack")
}

fn st(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// One Hammer run of `players` on style 0, `foes` enemies of defence 0 stacked in front of the
/// first player each tick.
struct Rig {
    e: Engine,
    foes: Vec<EntityId>,
    events: Vec<Value>,
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
    fn step(&mut self, inputs: [Input; 3]) -> Vec<Value> {
        self.stack();
        self.e.step(&inputs);
        let ev = as_json(self.e.events());
        self.events.extend(ev.iter().cloned());
        ev
    }

    fn idle(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step([Input::default(); 3]);
        }
    }

    /// Player 0's press of A (`main`) or of the smash button.
    fn press(&mut self, smash: bool) -> Vec<Value> {
        let press = Input {
            main: !smash,
            smash,
            ..Input::default()
        };
        self.step([press, Input::default(), Input::default()])
    }

    fn busy(&mut self) -> bool {
        let s = st(&mut self.e);
        s.runs.iter().any(|r| super::attacks::is_combo(s, r))
    }

    /// Presses and runs on until the step has ended: the ticks it ran (the press's included)
    /// and the events of those ticks.
    fn run_step(&mut self, smash: bool) -> (u32, Vec<Value>) {
        let mut events = self.press(smash);
        let mut ticks = 1;
        while self.busy() && ticks < 300 {
            events.extend(self.step([Input::default(); 3]));
            ticks += 1;
        }
        (ticks, events)
    }

    fn tempo(&mut self, i: u8) -> u8 {
        st(&mut self.e).players[slot(i)].combat.combo_tempo
    }

    fn set_tempo(&mut self, i: u8, stacks: u8) {
        st(&mut self.e).players[slot(i)].combat.combo_tempo = stacks;
    }

    fn view(&self, i: usize) -> Value {
        serde_json::to_value(&self.e.snapshot().players[i]).expect("a view")
    }

    /// A hit of `damage` on player `i`, as the hit pass reports it.
    fn hurt(&mut self, i: u8, damage: u32) -> Vec<Value> {
        let Run::V2 { world, state } = self.e.run_mut() else {
            panic!("the live rules");
        };
        let shot = Shot {
            at: state.players[slot(i)].at,
            owner: EntityId(99),
            friendly: false,
            damage,
            damage_type: None,
            breaks_charge: false,
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

/// The length `ticks` of an attack `pct` percent faster, as the phase arithmetic rounds a whole
/// attack (the Hammer's steps have three phases, so this holds only where the parts agree).
fn faster(ticks: u32, pct: u32) -> u32 {
    (ticks * 100 + (100 + pct) / 2) / (100 + pct)
}

/// S5: the kind changes no step: the style's tree equals the base's.
#[test]
fn the_kind_leaves_the_tree_equal_to_the_bases() {
    let pack = pack(vec![tempo_kind(5, 4, true)]);
    let content = pack.content();
    let base = content
        .weapons()
        .tree("giant_hammer", None)
        .expect("the Hammer's base tree");
    let expertise: &Expertise = content.expertise();
    let style = &expertise
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

/// B1: a run whose styles name no tempo keeps `combo_tempo` 0 through combos, smashes, evades and
/// hits, shows no view and emits no tempo event.
#[test]
fn a_run_that_names_no_tempo_stays_dormant() {
    let mut r = rig(1, vec![], 2);
    for _ in 0..6 {
        r.run_step(false);
        r.run_step(false);
        r.run_step(true);
        r.hurt(0, 400);
        r.idle(30);
        assert_eq!(r.tempo(0), 0);
        assert!(r.view(0).get("tempo").is_none());
    }
    assert_eq!(count(&r.events, "TempoRaised"), 0);
    assert_eq!(count(&r.events, "TempoLost"), 0);
}

/// B2: a step whose swing hits adds one stack on its hit tick, however many it hits; a step that
/// hits nothing adds none; the stacks stop at `max_stacks`.
#[test]
fn a_swing_that_hits_adds_one_stack() {
    let mut r = rig(1, vec![tempo_kind(2, 4, false)], 3);
    let (_, ev) = r.run_step(false);
    assert_eq!(
        named(&ev, "TempoRaised"),
        [json!({ "player": "first", "stacks": 1 })]
    );
    assert_eq!(r.tempo(0), 1);
    let (_, ev) = r.run_step(false);
    assert_eq!(
        named(&ev, "TempoRaised"),
        [json!({ "player": "first", "stacks": 2 })]
    );
    assert_eq!(r.tempo(0), 2);
    // The third swing hits at the cap: nothing changes and nothing is said.
    r.hurt(0, 0);
    r.set_tempo(0, 2);
    r.idle(1);
    let (_, ev) = r.run_step(false);
    assert_eq!(count(&ev, "TempoRaised"), 0, "at the cap");

    let mut none = rig(1, vec![tempo_kind(2, 4, false)], 0);
    let (_, ev) = none.run_step(false);
    assert_eq!(count(&ev, "TempoRaised"), 0);
    assert_eq!(none.tempo(0), 0);
}

/// B4: the stacks speed the attacks the combo starts afterwards: the first step runs its plain
/// length, the second at one stack is 4% faster, and a step in progress keeps its length.
#[test]
fn the_stacks_speed_the_next_step() {
    let mut plain = rig(1, vec![], 1);
    let (p1, _) = plain.run_step(false);
    let (p2, _) = plain.run_step(false);
    let mut r = rig(1, vec![tempo_kind(5, 4, false)], 1);
    let (t1, _) = r.run_step(false);
    assert_eq!(t1, p1, "no stacks at the first step");
    let (t2, _) = r.run_step(false);
    assert_eq!(r.tempo(0), 2, "the second swing hit");
    assert_eq!(t2, faster(p2, 4), "one stack at the start: plain {p2}");
    assert!(t2 < p2);
}

/// B5: the stacks' speed is held to the run's attack speed cap: with the cap at 0 they add nothing.
#[test]
fn the_runs_attack_speed_cap_holds_the_stacks() {
    let mut plain = rig(1, vec![], 1);
    plain.run_step(false);
    let (p2, _) = plain.run_step(false);
    let mut r = rig(1, vec![tempo_kind(5, 4, false)], 1);
    st(&mut r.e).max_attack_speed_pct = 0;
    r.run_step(false);
    let (t2, _) = r.run_step(false);
    assert_eq!(t2, p2);
    assert_eq!(r.tempo(0), 2, "the stacks still count");
}

/// B6: a smash pressed with stacks is itself as fast as they make it, then ends them; at full stacks
/// with `keep_on_smash` it keeps them; its own hits add none; an A step never ends them.
#[test]
fn a_smash_ends_the_stacks_unless_they_are_full_and_kept() {
    let mut plain = rig(1, vec![], 1);
    plain.run_step(false);
    let (smash_plain, _) = plain.run_step(true);

    let mut r = rig(1, vec![tempo_kind(3, 4, false)], 1);
    r.run_step(false);
    r.set_tempo(0, 3);
    let (len, ev) = r.run_step(true);
    assert_eq!(len, faster(smash_plain, 12), "the stacks held at the press");
    assert_eq!(r.tempo(0), 0);
    assert_eq!(count(&ev, "TempoLost"), 1);
    assert_eq!(count(&ev, "TempoRaised"), 0, "a smash's hits add none");

    let mut kept = rig(1, vec![tempo_kind(2, 4, true)], 1);
    kept.run_step(false);
    kept.set_tempo(0, 2);
    let (_, ev) = kept.run_step(true);
    assert_eq!(kept.tempo(0), 2, "full stacks and keep_on_smash");
    assert_eq!(count(&ev, "TempoLost"), 0);

    let mut below = rig(1, vec![tempo_kind(3, 4, true)], 1);
    below.run_step(false);
    below.set_tempo(0, 2);
    below.run_step(true);
    assert_eq!(below.tempo(0), 0, "below full they end as always");

    let mut zero = rig(1, vec![tempo_kind(3, 4, true)], 0);
    let (_, ev) = zero.run_step(true);
    assert_eq!(count(&ev, "TempoLost"), 0, "none to lose");
}

/// B7: the combo's window running out ends the stacks on the reset tick; a press inside the window
/// keeps them; a reset at the root says nothing.
#[test]
fn a_combo_reset_ends_the_stacks() {
    let mut r = rig(1, vec![tempo_kind(5, 4, false)], 1);
    r.run_step(false);
    assert_eq!(r.tempo(0), 1);
    r.idle(2);
    assert_eq!(r.tempo(0), 1, "inside the window");
    let before = count(&r.events, "TempoLost");
    r.idle(200);
    assert_eq!(r.tempo(0), 0);
    assert_eq!(count(&r.events, "TempoLost"), before + 1);
    r.idle(200);
    assert_eq!(
        count(&r.events, "TempoLost"),
        before + 1,
        "the root resets quietly"
    );

    // An evade ends them too.
    let mut e = rig(1, vec![tempo_kind(5, 4, false)], 1);
    e.run_step(false);
    let evade = Input {
        evade: true,
        ..Input::default()
    };
    let ev = e.step([evade, Input::default(), Input::default()]);
    assert_eq!(e.tempo(0), 0);
    assert_eq!(count(&ev, "TempoLost"), 1);
}

/// B8: a hit taken ends the stacks at once, with none held it says nothing.
#[test]
fn a_hit_taken_ends_the_stacks() {
    let mut r = rig(1, vec![tempo_kind(5, 4, false)], 1);
    let ev = r.hurt(0, 100);
    assert_eq!(count(&ev, "TempoLost"), 0);
    r.set_tempo(0, 3);
    let ev = r.hurt(0, 100);
    assert_eq!(r.tempo(0), 0);
    assert_eq!(count(&ev, "PlayerHit"), 1);
    assert_eq!(count(&ev, "TempoLost"), 1);
}

/// B9: a down ends the stacks; a style pick onto another style zeroes them.
#[test]
fn a_down_ends_the_stacks() {
    let mut r = rig(1, vec![tempo_kind(5, 4, false)], 1);
    r.set_tempo(0, 4);
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    super::player::stand_down(world, state, ME, &mut Vec::new());
    assert_eq!(state.players[ME].combat.combo_tempo, 0);
}

/// B10: three players each hold their own stacks and lose them alone.
#[test]
fn each_player_has_their_own_stacks() {
    let mut r = rig(3, vec![tempo_kind(5, 4, false)], 1);
    r.set_tempo(0, 2);
    r.set_tempo(1, 3);
    r.set_tempo(2, 4);
    let ev = r.hurt(1, 100);
    assert_eq!((r.tempo(0), r.tempo(1), r.tempo(2)), (2, 0, 4));
    assert_eq!(named(&ev, "TempoLost"), [json!({ "player": "second" })]);
}

/// B11: the kind alone changes nothing at 0 stacks: the first step's length and the run's
/// damage equal the plain run's.
#[test]
fn zero_stacks_equal_the_plain_run() {
    let mut plain = rig(1, vec![], 1);
    let mut r = rig(1, vec![tempo_kind(5, 4, true)], 1);
    let a = plain.run_step(false).0;
    let b = r.run_step(false).0;
    assert_eq!(a, b);
}

/// B12: the same scenario twice gives the same state hash every tick; a run resumed from its
/// saved state keeps its stacks.
#[test]
fn the_stacks_are_deterministic() {
    let run = || {
        let mut r = rig(1, vec![tempo_kind(5, 4, true)], 2);
        let mut hashes = Vec::new();
        for k in 0..6 {
            r.run_step(k % 3 == 2);
            hashes.push(r.e.state_hash());
        }
        (hashes, r.tempo(0))
    };
    assert_eq!(run(), run());
    assert!(run().1 > 0);
}

/// B13: the view carries the stacks, at 0 too, and is absent without a tempo style.
#[test]
fn the_view_follows_the_stacks() {
    let mut r = rig(1, vec![tempo_kind(5, 4, false)], 1);
    r.idle(1);
    assert_eq!(
        r.view(0)["tempo"],
        json!({ "stacks": 0, "max_stacks": 5, "speed_pct": 0 })
    );
    r.set_tempo(0, 3);
    assert_eq!(
        r.view(0)["tempo"],
        json!({ "stacks": 3, "max_stacks": 5, "speed_pct": 12 })
    );
}

/// B14: the bundled trial run, through combos, smashes, evades and hits, never holds a stack.
#[test]
fn the_bundled_run_never_holds_a_stack() {
    let mut r = rig(1, Vec::new(), 2);
    for k in 0..10 {
        r.run_step(k % 4 == 3);
        let evade = Input {
            evade: true,
            ..Input::default()
        };
        r.step([evade, Input::default(), Input::default()]);
        r.hurt(0, 50);
        assert_eq!(r.tempo(0), 0);
    }
}
