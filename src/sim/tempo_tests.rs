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
use crate::run_stats::StatEffect;
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
    let mut v: Value = serde_json::from_slice(super::before::pack_bytes()).unwrap();
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
        // No attack speed of its own: the tempo's is the only one.
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
    st(&mut plain.e).max_attack_speed_pct = 0;
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
        let (mut hashes, mut most) = (Vec::new(), 0);
        for k in 0..6 {
            r.run_step(k % 3 == 2);
            hashes.push(r.e.state_hash());
            most = most.max(r.tempo(0));
        }
        (hashes, most)
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

/// B9: under `try_styles` a style pick onto another style zeroes the stacks; the same style
/// again keeps them.
#[test]
fn leaving_the_style_empties_the_stacks() {
    use crate::input::EquipAction;
    let pick = |option: u32| {
        let mut r = rig(1, vec![tempo_kind(5, 4, false)], 0);
        let Run::V2 { world, state } = r.e.run_mut() else {
            panic!("the live rules");
        };
        std::sync::Arc::make_mut(world).stage.try_styles = true;
        let g = state
            .players
            .career_mut(ME)
            .expertise
            .as_mut()
            .expect("a gauge");
        g.level = 99;
        state.players[ME].combat.combo_tempo = 3;
        r.e.act(ME, EquipAction::TreePick { option })
            .expect("a pick");
        r.tempo(0)
    };
    assert_eq!(pick(0), 3, "the same style again");
    assert_eq!(pick(1), 0, "another style");
}

/// B12: a run's stacks are inside its state hash, and a held stack changes it.
#[test]
fn a_held_stack_is_in_the_state_hash() {
    let mut a = rig(1, vec![tempo_kind(5, 4, false)], 0);
    let mut b = rig(1, vec![tempo_kind(5, 4, false)], 0);
    b.set_tempo(0, 3);
    a.idle(1);
    b.idle(1);
    assert_ne!(a.e.state_hash(), b.e.state_hash());
}

/// B3: a held step's loop re-hits and each loop hit adds a stack (the AAA hold).
#[test]
fn a_hold_adds_a_stack_per_loop_hit() {
    let mut r = rig(1, vec![tempo_kind(10, 4, false)], 1);
    r.run_step(false);
    r.run_step(false);
    let before = r.tempo(0);
    // The third A held: the loop re-hits while the button stays down.
    let hold = Input {
        main: true,
        ..Input::default()
    };
    for _ in 0..80 {
        r.step([hold, Input::default(), Input::default()]);
    }
    assert!(
        r.tempo(0) >= before + 3,
        "the loop's hits count: {} from {before}",
        r.tempo(0)
    );
}

/// A tempo kind of 5 stacks of 4 percent that is `pct` percent wider at full stacks.
fn range_kind(pct: u32) -> Value {
    let mut k = tempo_kind(5, 4, false);
    k["effects"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "type": "range_pct_at_full", "pct": pct }));
    k
}

impl Rig {
    /// The range of player 0's running combo attack, None if none runs.
    fn width(&mut self) -> Option<u32> {
        let s = st(&mut self.e);
        s.runs
            .iter()
            .find(|r| super::attacks::is_combo(s, r))
            .map(|r| r.width_pct)
    }

    /// A press of A at `stacks` stacks: the range the started attack has.
    fn width_at(&mut self, stacks: u8) -> u32 {
        self.idle(120);
        self.set_tempo(0, stacks);
        self.press(false);
        self.width().expect("a combo attack runs")
    }
}

/// R1: at full stacks the attack starts `pct` wider; below full, nothing.
#[test]
fn full_stacks_widen_the_attack() {
    let mut r = rig(1, vec![range_kind(25)], 1);
    let base = r.width_at(0);
    for stacks in [3, 4] {
        assert_eq!(r.width_at(stacks), base, "{stacks} stacks");
    }
    assert_eq!(r.width_at(5), base + 25);
}

/// R2: a foe just past the first step's circle is missed at 4 stacks and hit at 5.
#[test]
fn full_stacks_reach_a_foe_the_plain_step_misses() {
    let hit = |stacks: u8, along: i32| {
        let mut r = rig(1, vec![range_kind(25)], 1);
        r.idle(120);
        r.set_tempo(0, stacks);
        let s = st(&mut r.e);
        let (at, facing) = (s.players[ME].at, s.players[ME].facing);
        let scale = i64::from(crate::trig::SCALE);
        let d = i64::from(Fx::px(along).0);
        let x = at.x.0 + i32::try_from(i64::from(facing.x) * d / scale).unwrap();
        let y = at.y.0 + i32::try_from(i64::from(facing.y) * d / scale).unwrap();
        s.enemies.retain(|f| r.foes.contains(&f.id));
        let mut hits = 0;
        let press = Input {
            main: true,
            ..Input::default()
        };
        for t in 0..60 {
            let s = st(&mut r.e);
            for f in &mut s.enemies {
                f.at = Point { x: Fx(x), y: Fx(y) };
                f.hp = BIG;
            }
            r.e.step(&[if t == 0 { press } else { Input::default() }; 3]);
            hits += count(&as_json(r.e.events()), "Hit");
        }
        hits
    };
    // Found by the sweep: the plain circle reaches `along` px at most, the widened one 25% more.
    let mut found = false;
    for along in 20..120 {
        if hit(4, along) == 0 && hit(5, along) > 0 {
            found = true;
            break;
        }
    }
    assert!(found, "no distance is hit only at full stacks");
}

/// R3: each loop of the hold and its release start wider at full stacks; below full the first
/// loop is plain.
#[test]
fn the_hold_and_its_release_are_wider_at_full_stacks() {
    let hold = Input {
        main: true,
        ..Input::default()
    };
    let widths = |stacks: u8, foes: u32| {
        let mut r = rig(1, vec![range_kind(25)], foes);
        r.run_step(false);
        r.run_step(false);
        r.idle(60);
        r.set_tempo(0, stacks);
        let mut seen = Vec::new();
        for _ in 0..40 {
            r.step([hold, Input::default(), Input::default()]);
            seen.extend(r.width());
        }
        for _ in 0..40 {
            r.step([Input::default(); 3]);
            seen.extend(r.width());
        }
        seen
    };
    let base = widths(0, 0);
    assert!(!base.is_empty(), "the hold ran");
    let plain = base[0];
    let full = widths(5, 0);
    assert!(!full.is_empty());
    assert!(
        full.iter().all(|w| *w == plain + 25),
        "{full:?} from {plain}"
    );
    let four = widths(4, 0);
    assert_eq!(four[0], plain);
}

/// R4: the swing that adds the fifth stack is not wider, the next one is.
#[test]
fn the_stack_is_read_when_the_attack_starts() {
    let mut r = rig(1, vec![range_kind(25)], 1);
    r.idle(120);
    r.set_tempo(0, 4);
    r.press(false);
    let first = r.width().expect("a run");
    while r.busy() {
        r.step([Input::default(); 3]);
    }
    assert_eq!(r.tempo(0), 5, "the swing hit and filled the stacks");
    r.press(false);
    assert_eq!(r.width(), Some(first + 25));
}

/// R5: in a party only the player with full stacks is wider, and two runs agree.
#[test]
fn only_the_player_at_full_stacks_is_wider() {
    let run = || {
        let mut r = rig(2, vec![range_kind(25)], 1);
        r.idle(120);
        r.set_tempo(0, 5);
        let press = Input {
            main: true,
            ..Input::default()
        };
        r.step([press, press, Input::default()]);
        let s = st(&mut r.e);
        let width = |i: u8| {
            let id = s.players.id(slot(i));
            s.runs
                .iter()
                .find(|x| x.owner == id && x.slot == super::player::COMBO_SLOT)
                .map(|x| x.width_pct)
                .expect("a run")
        };
        let (a, b) = (width(0), width(1));
        for _ in 0..300 {
            r.step([Input::default(); 3]);
        }
        (a, b, r.e.state_hash())
    };
    let (a, b, h1) = run();
    assert_eq!(a, b + 25);
    assert_eq!(run().2, h1);
}

/// R6: stacks that never reach full change nothing; the range sum is stored whole and the box
/// is held to twice the plain radius.
#[test]
fn below_full_nothing_moves_and_the_sum_is_capped_when_used() {
    let play = |kind: Value| {
        let mut r = rig(1, vec![kind], 1);
        r.idle(60);
        for t in 0..600 {
            r.set_tempo(0, 4);
            let press = Input {
                main: t % 20 == 0,
                move_x: if t % 7 < 3 { 100 } else { 0 },
                ..Input::default()
            };
            r.step([press, Input::default(), Input::default()]);
        }
        r.e.state_hash()
    };
    assert_eq!(play(range_kind(25)), play(tempo_kind(5, 4, false)));

    let mut r = rig(1, vec![range_kind(25)], 1);
    r.idle(120);
    st(&mut r.e).players[ME].range_pct = 90;
    r.set_tempo(0, 5);
    r.press(false);
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let run = state
        .runs
        .iter()
        .find(|x| super::attacks::is_combo(state, x))
        .expect("a run");
    assert!(
        run.width_pct > 100,
        "the sum is stored whole: {}",
        run.width_pct
    );
    let phases = super::attacks::phase_count(world, run);
    let capped = (0..phases)
        .find_map(|i| super::attacks::phase(world, run, i).hit_box().cloned())
        .expect("a widened box");
    let plain = (0..phases)
        .find_map(|i| super::attacks::widened_box(world, run.attack, i, 0))
        .expect("a box");
    use crate::attack::HitBox;
    match (capped, plain) {
        (HitBox::Circle { radius: a, .. }, HitBox::Circle { radius: b, .. }) => {
            assert_eq!(i64::from(a.0), i64::from(b.0) * 2)
        }
        other => panic!("{other:?}"),
    }
}

/// A tempo kind of 5 stacks of 4 percent, 25 percent wider at full stacks, with the flags asked.
fn flags_kind(keep: bool, carry: bool) -> Value {
    let mut k = range_kind(25);
    let effects = k["effects"].as_array_mut().unwrap();
    if keep {
        effects.push(json!({ "type": "keep_on_smash" }));
    }
    if carry {
        effects.push(json!({ "type": "carry_at_full" }));
    }
    k
}

/// The kind of the carry tests: keeps on a full-stack smash and carries them.
fn carry_kind() -> Value {
    flags_kind(true, true)
}

impl Rig {
    /// A smash pressed at full stacks: an A, the stacks set to 5, then the smash, run out.
    fn full_smash(&mut self) -> Vec<Value> {
        self.run_step(false);
        self.set_tempo(0, 5);
        self.run_step(true).1
    }

    /// A step pressed and run out: its length, its events and the range it started with.
    fn run_width(&mut self, smash: bool) -> (u32, Vec<Value>, u32) {
        let mut events = self.press(smash);
        let width = self.width().expect("a combo attack runs");
        let mut ticks = 1;
        while self.busy() && ticks < 300 {
            events.extend(self.step([Input::default(); 3]));
            ticks += 1;
        }
        (ticks, events, width)
    }

    fn evade(&mut self) -> Vec<Value> {
        let evade = Input {
            evade: true,
            ..Input::default()
        };
        self.step([evade, Input::default(), Input::default()])
    }
}

/// CB1: the combo window running out ends the carry; a press after it is a press at the root.
#[test]
fn the_window_ends_the_carry() {
    let mut r = rig(1, vec![carry_kind()], 1);
    r.full_smash();
    assert_eq!(r.tempo(0), 5);
    r.idle(200);
    assert_eq!(r.tempo(0), 0);
    assert_eq!(count(&r.events, "TempoLost"), 1);
    assert_eq!(count(&r.events, "ComboReset"), 1);

    let mut plain = rig(1, vec![flags_kind(true, false)], 1);
    plain.full_smash();
    plain.idle(200);
    assert_eq!(plain.tempo(0), 0);
    assert_eq!(count(&plain.events, "TempoLost"), 1);

    let base = rig(1, vec![carry_kind()], 1).width_at(0);
    let mut late = rig(1, vec![carry_kind()], 1);
    late.full_smash();
    late.idle(30);
    late.press(false);
    assert_eq!(late.tempo(0), 0);
    assert_eq!(late.width(), Some(base));
}

/// CB2: a plain combo started inside the window keeps the 5 stacks, with their range and speed
/// from its first step; a smash below full leaves nothing to carry.
#[test]
fn a_press_after_the_smash_keeps_the_stacks() {
    let mut p = rig(1, vec![], 1);
    let (plain_a, _) = p.run_step(false);
    let base = rig(1, vec![carry_kind()], 1).width_at(0);

    let mut r = rig(1, vec![carry_kind()], 1);
    r.full_smash();
    let lost = count(&r.events, "TempoLost");
    let (len, ev, width) = r.run_width(false);
    assert_eq!(r.tempo(0), 5);
    assert_eq!(count(&ev, "TempoLost"), 0);
    assert_eq!(count(&r.events, "TempoLost"), lost);
    assert_eq!(width, base + 25);
    assert_eq!(len, faster(plain_a, 20));
    let (_, ev, width) = r.run_width(false);
    assert_eq!((r.tempo(0), width), (5, base + 25));
    assert_eq!(count(&ev, "TempoLost"), 0);

    let mut below = rig(1, vec![carry_kind()], 1);
    below.run_step(false);
    below.set_tempo(0, 4);
    below.run_step(true);
    let (len, _, width) = below.run_width(false);
    assert_eq!(width, base);
    assert_eq!(len, plain_a);
}

/// CB3: what ends the carried stacks.
#[test]
fn what_ends_the_carried_stacks() {
    // (a) the combo that follows ends on an A.
    let mut a = rig(1, vec![carry_kind()], 1);
    a.full_smash();
    a.run_step(false);
    assert_eq!(a.tempo(0), 5);
    a.idle(200);
    assert_eq!(a.tempo(0), 0);
    assert_eq!(count(&a.events, "TempoLost"), 1);

    // (b) it ends on a smash: a full-stack smash again, and a further A inside the window.
    let mut b = rig(1, vec![carry_kind()], 1);
    b.full_smash();
    b.run_step(false);
    b.run_step(true);
    b.run_step(false);
    assert_eq!(b.tempo(0), 5);
    assert_eq!(count(&b.events, "TempoLost"), 0);

    // (c) a hit taken, with the stacks carried by an evade.
    let mut c = rig(1, vec![carry_kind()], 1);
    c.full_smash();
    c.evade();
    c.idle(120);
    assert_eq!(c.tempo(0), 5);
    let ev = c.hurt(0, 1);
    assert_eq!((c.tempo(0), count(&ev, "TempoLost")), (0, 1));

    // (d) a smash below full ends them even with the carry.
    let mut d = rig(1, vec![carry_kind()], 1);
    d.run_step(false);
    d.set_tempo(0, 4);
    d.run_step(true);
    assert_eq!(d.tempo(0), 0);

    // (e) a down.
    let mut e = rig(1, vec![carry_kind()], 1);
    e.full_smash();
    e.evade();
    assert_eq!(e.tempo(0), 5);
    let Run::V2 { world, state } = e.e.run_mut() else {
        panic!("the live rules");
    };
    super::player::stand_down(world, state, ME, &mut Vec::new());
    assert_eq!(state.players[ME].combat.combo_tempo, 0);
}

/// CB4: an evade at once after a full-stack smash keeps the stacks, with no timer at the root;
/// after an A it ends them as before.
#[test]
fn an_evade_after_the_smash_keeps_the_stacks() {
    let mut r = rig(1, vec![carry_kind()], 1);
    r.full_smash();
    let ev = r.evade();
    assert_eq!((r.tempo(0), count(&ev, "TempoLost")), (5, 0));
    r.idle(200);
    assert_eq!(r.tempo(0), 5);
    assert_eq!(count(&r.events, "TempoLost"), 0);

    let mut a = rig(1, vec![carry_kind()], 1);
    a.run_step(false);
    let ev = a.evade();
    assert_eq!((a.tempo(0), count(&ev, "TempoLost")), (0, 1));
}

/// CB5: the auto combo's restart keeps full stacks a full-stack smash left; without the carry
/// the restart ends them.
#[test]
fn the_auto_restart_keeps_the_stacks() {
    let auto = Input {
        auto: true,
        ..Input::default()
    };
    let run = |kind: Value| {
        let mut r = rig(1, vec![kind], 1);
        for _ in 0..5 {
            r.run_step(false);
        }
        r.set_tempo(0, 5);
        r.run_step(true);
        r.idle(200);
        assert_eq!(r.tempo(0), 0, "the window ended the hand combo's stacks");
        r.events.clear();
        for _ in 0..600 {
            r.step([auto, Input::default(), Input::default()]);
        }
        assert!(st(&mut r.e).players[ME].target.is_some());
        (
            r.tempo(0),
            count(&r.events, "ComboReset"),
            count(&r.events, "TempoLost"),
        )
    };
    let (stacks, resets, lost) = run(carry_kind());
    assert!(resets >= 2, "the auto combo restarted: {resets}");
    assert_eq!((stacks, lost), (5, 0));
    let (_, resets, lost) = run(flags_kind(true, false));
    assert!(resets >= 2 && lost >= 1, "{resets} {lost}");
}

/// CB6: without `keep_on_smash` the carry is inert: the smash ends the stacks at the press.
#[test]
fn the_carry_is_inert_without_keep_on_smash() {
    let mut r = rig(1, vec![flags_kind(false, true)], 1);
    let ev = r.full_smash();
    assert_eq!(r.tempo(0), 0);
    assert_eq!(count(&ev, "TempoLost"), 1);
}

/// CB7: the hold's armour applies from the start of the next combo's hold when the stacks were
/// carried, and not once they were ended.
#[test]
fn the_hold_after_a_carry_is_armoured_and_wide() {
    let kind = {
        let mut k = carry_kind();
        k["effects"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "type": "super_armour_at_full" }));
        k
    };
    let hold = Input {
        main: true,
        ..Input::default()
    };
    let held = |carry: bool| {
        let mut r = rig(1, vec![kind.clone()], 0);
        r.full_smash();
        if !carry {
            r.hurt(0, 1);
            assert_eq!(r.tempo(0), 0);
        }
        r.run_step(false);
        r.run_step(false);
        let mut widths = Vec::new();
        for _ in 0..30 {
            r.step([hold, Input::default(), Input::default()]);
            widths.extend(r.width());
        }
        assert!(st(&mut r.e).players[ME].combo.hold.is_some(), "a hold runs");
        let ev = r.hurt(0, 1);
        (widths, r.tempo(0), count(&ev, "TempoLost"))
    };
    let (plain, stacks, lost) = held(false);
    assert_eq!((stacks, lost), (0, 0), "the stacks ended before the hold");
    let (wide, stacks, lost) = held(true);
    assert_eq!(
        (stacks, lost),
        (5, 0),
        "the armour shelters the carried stacks"
    );
    assert!(!plain.is_empty());
    assert!(
        wide.iter().all(|w| *w == plain[0] + 25),
        "{wide:?} {plain:?}"
    );
}

/// CB8: nothing else moves: with the stacks never full, naming the carry changes no state.
#[test]
fn the_carry_moves_nothing_below_full() {
    let run = |kind: Value| {
        let mut r = rig(1, vec![kind], 0);
        let mut hashes = Vec::new();
        for t in 0..600u32 {
            r.set_tempo(0, 4);
            let input = Input {
                main: t % 97 == 0,
                smash: t % 131 == 7,
                evade: t % 211 == 5,
                ..Input::default()
            };
            r.step([input, Input::default(), Input::default()]);
            hashes.push(r.e.state_hash());
        }
        hashes
    };
    assert_eq!(run(carry_kind()), run(flags_kind(true, false)));
}

/// CB9: in a party each player keeps its own stacks, and two runs agree on every hash.
#[test]
fn each_player_carries_alone() {
    let run = || {
        let mut r = rig(2, vec![carry_kind()], 1);
        let both = Input {
            main: true,
            ..Input::default()
        };
        r.step([both, both, Input::default()]);
        r.set_tempo(1, 3);
        r.set_tempo(0, 5);
        let mut hashes = Vec::new();
        r.run_step(true);
        hashes.push(r.e.state_hash());
        r.run_step(false);
        let ev = r.events.clone();
        let held = (r.tempo(0), r.tempo(1));
        for t in 0..60 {
            r.step([Input::default(); 3]);
            if t % 10 == 0 {
                hashes.push(r.e.state_hash());
            }
        }
        (held.0, held.1, ev, hashes)
    };
    let (p0, p1, ev, hashes) = run();
    assert_eq!((p0, p1), (5, 0), "player 0 carried, player 1 lost its own");
    assert!(named(&ev, "TempoLost").contains(&json!({ "player": "second" })));
    assert!(!named(&ev, "TempoLost").contains(&json!({ "player": "first" })));
    assert_eq!(run().3, hashes);
}

/// CB10: the carry reads nothing a save does not hold: the combo and combat state saved between
/// the smash and the reset, and written back, carry the same way.
#[test]
fn a_save_between_the_smash_and_the_reset_carries() {
    use super::state::{ComboState, PlayerCombat};
    use crate::savepoint::testing::{from_bytes, to_bytes};
    let mut a = rig(1, vec![carry_kind()], 1);
    let mut b = rig(1, vec![carry_kind()], 1);
    a.full_smash();
    b.full_smash();
    {
        let c = &st(&mut b.e).players[ME].combo;
        assert!(c.depth > 0 && c.path >> (c.depth - 1) & 1 == 1, "a smash");
        assert_eq!(st(&mut b.e).players[ME].combat.combo_tempo, 5);
    }
    {
        let p = &mut st(&mut b.e).players[ME];
        p.combo = from_bytes::<ComboState>(&to_bytes(&p.combo)).expect("combo");
        p.combat = from_bytes::<PlayerCombat>(&to_bytes(&p.combat)).expect("combat");
    }
    assert_eq!(a.e.state_hash(), b.e.state_hash());
    let (_, _, wa) = a.run_width(false);
    let (_, _, wb) = b.run_width(false);
    assert_eq!((a.tempo(0), b.tempo(0), wa), (5, 5, wb));
    assert_eq!(count(&b.events, "TempoLost"), 0);
    for _ in 0..30 {
        a.step([Input::default(); 3]);
        b.step([Input::default(); 3]);
        assert_eq!(a.e.state_hash(), b.e.state_hash());
    }

    let mut a = rig(1, vec![carry_kind()], 1);
    a.full_smash();
    a.idle(200);
    assert_eq!((a.tempo(0), count(&a.events, "TempoLost")), (0, 1));
}
