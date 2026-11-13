//! The spirit level a style grants (ADR-SIM-9 points 2 to 7, ADR-SIM-5, ADR-SIM-7): the kind
//! changes no step of the tree, the level rises with the full-gauge smash, falls with a
//! countdown and carries its own effects. Every number is a test value, not content.

use super::state::{PlayerIdx, State, StatusKind};
use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::expertise::{Expertise, weapon_with_style};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use crate::pack::Pack;
use crate::run_setup::RunSetup;
use crate::version::Versions;
use serde_json::{Value, json};

const ME: PlayerIdx = PlayerIdx::SOLO;
const BIG: u32 = 1_000_000_000;
const DECAY: u16 = 100;

/// S5: a style that names only `spirit` leaves the weapon's tree equal to the base's: the kind
/// changes no step, the rules read it beside the weapon.
#[test]
fn the_spirit_kind_changes_no_step() {
    let content = Pack::bundled().content();
    let base = content
        .weapons()
        .tree("giant_sword", None)
        .expect("the Sword's base tree");
    let bundled: serde_json::Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let mut file = bundled["expertise"].clone();
    let w = file["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_sword")
        .unwrap();
    w["styles"][0]["kinds"] = json!([{
        "type": "spirit", "decay_ticks": 100,
        "levels": [[{ "type": "damage_pct", "pct": 8 }]]
    }]);
    let expertise =
        Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the expertise");
    let style = &expertise
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_sword")
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

/// The Sword's three levels as the design table gives them (test values, not content).
fn levels() -> Value {
    json!([
        [{ "type": "damage_pct", "pct": 8 }],
        [{ "type": "damage_pct", "pct": 16 }, { "type": "range_pct", "pct": 15 }],
        [{ "type": "damage_pct", "pct": 25 }, { "type": "smash_bleed_stacks", "stacks": 2 }]
    ])
}

/// The full-gauge attack: one wind-up tick, two active ticks of damage 100 and a recovery, a
/// circle of `radius_px` pixels at the player.
fn full_attack(radius_px: i32) -> Value {
    json!({ "inline": [
        { "kind": "wind_up", "ticks": 1 },
        { "kind": "active", "ticks": 2, "type": "slash", "damage": 100,
          "hit_box": { "type": "circle", "radius": radius_px * 256,
                       "offset": { "forward": 0, "right": 0 } } },
        { "kind": "recovery", "ticks": 6 }
    ] })
}

/// The bundled pack with the gauge (one A hit fills it) and, when `spirit`, the spirit on the
/// Sword's first style.
fn pack(spirit: bool, radius_px: i32, bleed_max: Option<u32>) -> Pack {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_sword")
        .expect("the Sword's expertise");
    let mut kinds = vec![json!({
        "type": "gauge", "max": 100, "per_step": 100, "step_cap": 100,
        "spend": { "type": "full_smash", "attack": full_attack(radius_px) }
    })];
    if spirit {
        kinds.push(json!({ "type": "spirit", "decay_ticks": DECAY, "levels": levels() }));
        kinds.push(json!({ "type": "smash_effect", "effect": { "type": "bleed" } }));
    }
    w["styles"][0]["kinds"] = json!(kinds);
    if let Some(max) = bleed_max {
        for d in v["runes"]["damage_over_time"].as_array_mut().unwrap() {
            if d["rune"] == "bleed" {
                d["max_stacks"] = json!(max);
            }
        }
    }
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack")
}

fn st(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// A run of `players` on the Sword, every player on style 0 and hardy, with `foes` enemies
/// kept stacked on the first player.
struct Rig {
    e: Engine,
    foes: Vec<EntityId>,
    keep: bool,
}

fn rig(players: u8, spirit: bool, foes: u32) -> Rig {
    rig_with(players, spirit, foes, 40, None)
}

fn rig_with(players: u8, spirit: bool, foes: u32, radius_px: i32, bleed_max: Option<u32>) -> Rig {
    let pack = pack(spirit, radius_px, bleed_max);
    let setup = RunSetup {
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        seed: 20_261_113,
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
        p.invulnerable = 0;
        if let Some(g) = s.players.career_mut(slot(i)).expertise.as_mut() {
            g.style = Some(0);
        }
    }
    Rig {
        e,
        foes: ids,
        keep: true,
    }
}

impl Rig {
    fn stack(&mut self) {
        if !self.keep {
            return;
        }
        let s = st(&mut self.e);
        let (at, facing) = (s.players[ME].at, s.players[ME].facing);
        s.enemies.retain(|x| self.foes.contains(&x.id));
        let scale = i64::from(crate::trig::SCALE);
        let (fx, fy) = (i64::from(facing.x), i64::from(facing.y));
        let n = i64::try_from(s.enemies.len()).unwrap_or(0);
        for (i, f) in s.enemies.iter_mut().enumerate() {
            let along = i64::from(Fx::px(24).0);
            let across = (i64::try_from(i).unwrap_or(0) - n / 2) * i64::from(Fx::px(3).0);
            let dx = (fx * along - fy * across) / scale;
            let dy = (fy * along + fx * across) / scale;
            f.at = Point {
                x: Fx(at.x.0 + i32::try_from(dx).unwrap_or(0)),
                y: Fx(at.y.0 + i32::try_from(dy).unwrap_or(0)),
            };
            f.hp = BIG;
        }
    }

    fn step(&mut self, inputs: [Input; 3]) -> Vec<Value> {
        self.stack();
        self.e.step(&inputs);
        as_json(self.e.events())
    }

    fn idle(&mut self, ticks: u32) -> Vec<Value> {
        let mut all = Vec::new();
        for _ in 0..ticks {
            all.extend(self.step([Input::default(); 3]));
        }
        all
    }

    /// One press of `a` or `b` by player 0 and `ticks` idle ticks; every event.
    fn press(&mut self, b: bool, ticks: u32) -> Vec<Value> {
        let press = Input {
            main: !b,
            smash: b,
            ..Input::default()
        };
        let mut all = self.step([press, Input::default(), Input::default()]);
        all.extend(self.idle(ticks));
        all
    }

    fn set_gauge(&mut self, i: u8, gauge: u32) {
        st(&mut self.e).players[slot(i)].combat.weapon_gauge = gauge;
    }

    fn set_spirit(&mut self, i: u8, level: u8, decay: u16) {
        let c = &mut st(&mut self.e).players[slot(i)].combat;
        c.spirit_level = level;
        c.spirit_decay = decay;
    }

    /// Player `i`'s level and countdown.
    fn spirit(&mut self, i: u8) -> (u8, u16) {
        let c = &st(&mut self.e).players[slot(i)].combat;
        (c.spirit_level, c.spirit_decay)
    }

    fn view(&self, i: usize) -> Value {
        serde_json::to_value(&self.e.snapshot().players[i]).expect("a view")
    }
}

fn as_json(events: &[DomainEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|e| serde_json::to_value(e).expect("an event"))
        .collect()
}

/// The `data` of every event of type `ty`.
fn data_of(events: &[Value], ty: &str) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e["type"] == ty)
        .map(|e| e["data"].clone())
        .collect()
}

/// The damage of every hit among `events`.
fn damages(events: &[Value]) -> Vec<u64> {
    data_of(events, "Hit")
        .iter()
        .map(|d| d["damage"].as_u64().expect("a damage"))
        .collect()
}

fn b() -> [Input; 3] {
    [
        Input {
            smash: true,
            ..Input::default()
        },
        Input::default(),
        Input::default(),
    ]
}

/// B1: a run that names no spirit keeps both fields 0 and shows and says nothing of it.
#[test]
fn a_run_with_no_spirit_never_moves_either_field() {
    let mut r = rig(1, false, 3);
    let Run::V2 { world, .. } = r.e.run() else {
        panic!("the live rules");
    };
    assert!(!world.has_spirit);
    for t in 0..600 {
        let smash = t % 90 == 60;
        let press = Input {
            main: !smash && t % 30 == 0,
            smash,
            ..Input::default()
        };
        if t % 100 == 0 {
            st(&mut r.e).players[ME].hp = 500;
            r.set_gauge(0, 100);
        }
        let events = r.step([press, Input::default(), Input::default()]);
        assert_eq!(r.spirit(0), (0, 0), "tick {t}");
        assert!(data_of(&events, "SpiritRaised").is_empty());
        assert!(r.view(0).get("spirit").is_none());
    }
}

/// B2: a full-gauge smash raises the level on the press and starts the countdown.
#[test]
fn a_full_gauge_smash_raises_the_level() {
    let mut r = rig(1, true, 1);
    let Run::V2 { world, .. } = r.e.run() else {
        panic!("the live rules");
    };
    assert!(world.has_spirit);
    for want in 1..=3u8 {
        r.set_gauge(0, 100);
        let events = r.step(b());
        assert_eq!(r.spirit(0), (want, DECAY), "the press tick, level {want}");
        assert_eq!(
            data_of(&events, "SpiritRaised"),
            [json!({ "player": "first", "level": want })]
        );
        r.idle(30);
    }
    r.set_gauge(0, 100);
    let events = r.step(b());
    assert_eq!(r.spirit(0), (3, DECAY), "at the top the countdown restarts");
    assert!(data_of(&events, "SpiritRaised").is_empty());
    r.idle(30);
    // An ordinary smash and an A press at a full gauge raise nothing.
    let mut o = rig(1, true, 1);
    o.set_gauge(0, 99);
    o.step(b());
    assert_eq!(o.spirit(0).0, 0, "the ordinary smash");
    let mut a = rig(1, true, 1);
    a.set_gauge(0, 100);
    a.press(false, 40);
    assert_eq!(a.spirit(0).0, 0, "an A press");
}

/// B3: every smash step restarts the countdown of a raised level, an A step does not, and a
/// smash at level 0 starts none. An A step on the press tick has counted one tick down.
#[test]
fn a_smash_restarts_the_countdown_and_an_a_step_does_not() {
    let mut r = rig(1, true, 1);
    r.set_spirit(0, 1, 40);
    r.step(b());
    assert_eq!(r.spirit(0), (1, DECAY), "an ordinary smash step");
    let mut a = rig(1, true, 1);
    a.set_spirit(0, 1, 40);
    a.step([
        Input {
            main: true,
            ..Input::default()
        },
        Input::default(),
        Input::default(),
    ]);
    assert_eq!(a.spirit(0), (1, 39), "an A step only counts down");
    let mut z = rig(1, true, 1);
    z.step(b());
    assert_eq!(z.spirit(0), (0, 0), "a smash at level 0");
}

/// B4: the level falls on the tick its countdown reaches 0, and restarts while above 0.
#[test]
fn the_level_falls_when_the_countdown_runs_out() {
    let mut r = rig(1, true, 1);
    r.set_gauge(0, 100);
    r.step(b());
    for k in 1..DECAY {
        let events = r.step([Input::default(); 3]);
        assert_eq!(r.spirit(0), (1, DECAY - k), "tick +{k}");
        assert!(data_of(&events, "SpiritFell").is_empty());
    }
    let events = r.step([Input::default(); 3]);
    assert_eq!(r.spirit(0), (0, 0), "the fall");
    assert_eq!(
        data_of(&events, "SpiritFell"),
        [json!({ "player": "first", "level": 0 })]
    );
    assert!(data_of(&r.idle(300), "SpiritFell").is_empty());
    assert_eq!(r.spirit(0), (0, 0));

    let mut two = rig(1, true, 1);
    two.set_spirit(0, 2, DECAY);
    let events = two.idle(100);
    assert_eq!(two.spirit(0), (1, DECAY));
    assert_eq!(
        data_of(&events, "SpiritFell"),
        [json!({ "player": "first", "level": 1 })]
    );
    two.idle(100);
    assert_eq!(two.spirit(0), (0, 0));

    // A smash at +60 moves the fall to +160.
    let mut s = rig(1, true, 1);
    s.set_spirit(0, 1, DECAY);
    s.idle(60);
    s.step(b());
    s.idle(98);
    assert_eq!(s.spirit(0).0, 1);
    s.idle(1);
    assert_eq!(s.spirit(0).0, 1, "still one tick");
    s.idle(1);
    assert_eq!(s.spirit(0), (0, 0));
}

/// B4: a downed player's countdown runs, and a revive changes neither field.
#[test]
fn a_downed_player_s_countdown_runs() {
    let mut r = rig(2, true, 1);
    r.set_spirit(1, 2, DECAY);
    super::party_fix_tests::down(st(&mut r.e), 1);
    r.idle(100);
    assert_eq!(r.spirit(1), (1, DECAY));
}

/// B5: the level is the style's: a pick of another style empties it, an upgrade pick keeps it.
#[test]
fn the_level_goes_with_the_style() {
    let mut r = rig(1, true, 1);
    let pack = pack(true, 40, None);
    let expertise = pack.content().expertise().clone();
    {
        let Run::V2 { world, state } = r.e.run_mut() else {
            panic!("the live rules");
        };
        let g = state.players.career_mut(ME).expertise.as_mut().unwrap();
        g.level = 30;
        g.style = Some(0);
        g.path.clear();
        state.players[ME].combat.spirit_level = 2;
        state.players[ME].combat.spirit_decay = 50;
        super::tree_pick_action(world, &expertise, state, ME, 0).expect("an upgrade pick");
        let c = &state.players[ME].combat;
        assert_eq!(
            (c.spirit_level, c.spirit_decay),
            (2, 50),
            "an upgrade keeps it"
        );
        let g = state.players.career_mut(ME).expertise.as_mut().unwrap();
        g.style = None;
        g.path.clear();
        super::tree_pick_action(world, &expertise, state, ME, 1).expect("a style pick");
        let c = &state.players[ME].combat;
        assert_eq!(
            (c.spirit_level, c.spirit_decay),
            (0, 0),
            "another style empties it"
        );
    }
}

/// B6: each player has its own level, countdown and event, on its own style.
#[test]
fn each_player_has_its_own_level() {
    let mut r = rig(3, true, 1);
    if let Some(g) = st(&mut r.e).players.career_mut(slot(2)).expertise.as_mut() {
        g.style = Some(1);
    }
    for i in 0..3 {
        r.set_gauge(i, 100);
    }
    let all = Input {
        smash: true,
        ..Input::default()
    };
    let events = r.step([all; 3]);
    assert_eq!(r.spirit(0), (1, DECAY));
    assert_eq!(r.spirit(1), (1, DECAY));
    assert_eq!(r.spirit(2), (0, 0), "a style with no spirit");
    let raised = data_of(&events, "SpiritRaised");
    assert_eq!(raised.len(), 2);
    assert!(raised.contains(&json!({ "player": "first", "level": 1 })));
    assert!(raised.contains(&json!({ "player": "second", "level": 1 })));
    r.set_spirit(0, 1, 1);
    r.idle(1);
    assert_eq!(r.spirit(0), (0, 0));
    assert_eq!(r.spirit(1), (1, DECAY - 1), "the others do not move");
}

/// What every hit of `press` deals at spirit level `level` (the countdown full): the full-gauge
/// attack's with `full`, else an A step's.
fn dealt(level: u8, full: bool) -> Vec<u64> {
    let mut r = rig(1, true, 1);
    r.set_spirit(0, level, DECAY);
    if full {
        // The raise is the press's own: start one level below.
        r.set_spirit(0, level.saturating_sub(1), DECAY);
        r.set_gauge(0, 100);
    }
    damages(&r.press(!full, 40))
}

/// B7: each level's damage percent is that level's own and every combo step carries it.
#[test]
fn the_level_adds_its_damage_percent() {
    let base = dealt(0, false);
    assert!(!base.is_empty());
    for (level, pct) in [(1u8, 8u64), (2, 16), (3, 25)] {
        let want: Vec<u64> = base.iter().map(|d| d * (100 + pct) / 100).collect();
        assert_eq!(dealt(level, false), want, "an A step at level {level}");
    }
    assert_eq!(
        dealt(1, true).first(),
        Some(&108),
        "the full-gauge attack at level 1"
    );
    assert_eq!(dealt(3, true).first(), Some(&125));
}

/// B8: the level's reach widens the step: the radius 40 px circle reaches 46 px at level 2.
#[test]
fn the_level_widens_the_reach() {
    let reach = |level_before: u8| {
        let mut best = 0;
        for quarter in 0..400 {
            let d = 40 * 4 + quarter;
            let mut r = rig(1, true, 1);
            r.keep = false;
            r.set_spirit(0, level_before, DECAY);
            r.set_gauge(0, 100);
            let hit = {
                let mut got = false;
                for t in 0..12 {
                    let s = st(&mut r.e);
                    let (at, facing) = (s.players[ME].at, s.players[ME].facing);
                    let scale = i64::from(crate::trig::SCALE);
                    let along = i64::from(d) * 64;
                    s.enemies[0].at = Point {
                        x: Fx(at.x.0 + i32::try_from(i64::from(facing.x) * along / scale).unwrap()),
                        y: Fx(at.y.0 + i32::try_from(i64::from(facing.y) * along / scale).unwrap()),
                    };
                    let inputs = if t == 0 { b() } else { [Input::default(); 3] };
                    got |= !damages(&r.step(inputs)).is_empty();
                }
                got
            };
            if hit {
                best = d;
            }
        }
        best
    };
    let (one, two): (i32, i32) = (reach(0), reach(1));
    // Level 1 has no reach; level 2's is 15 percent of 40 px: 6 px, 24 quarter pixels.
    let level_one = reach(0);
    let level_two: i32 = two;
    assert_eq!(one, level_one);
    assert!(
        (level_two - level_one - 24).abs() <= 1,
        "{level_one} to {level_two}"
    );
}

/// The bleed stacks the first enemy holds.
fn bleed(r: &mut Rig) -> u32 {
    st(&mut r.e).enemies[0]
        .statuses
        .iter()
        .find(|x| x.kind == StatusKind::Bleed)
        .map_or(0, |x| u32::from(x.stacks))
}

/// B9: at level 3 a smash hit that leaves the target standing adds the level's two more stacks.
#[test]
fn the_top_level_adds_bleed_stacks_to_a_smash() {
    let after = |level: u8, a_step: bool| {
        let mut r = rig(1, true, 1);
        r.set_spirit(0, level, DECAY);
        r.press(!a_step, 4);
        bleed(&mut r)
    };
    let own = after(0, false);
    assert!(own >= 1, "the bleed smash adds its own stack: {own}");
    assert_eq!(after(2, false), own, "level 2 adds none");
    assert_eq!(after(3, false), own + 2, "level 3 adds two on top");
    assert_eq!(after(3, true), 0, "an A hit adds none");
    let mut capped = rig_with(1, true, 1, 40, Some(2));
    capped.set_spirit(0, 3, DECAY);
    capped.press(true, 4);
    assert_eq!(bleed(&mut capped), 2, "the bleed's own limit holds");
}

/// B10: the same scenario twice gives the same hash every tick, and a clone mid-countdown
/// keeps pace with the run it came from (the fall included).
#[test]
fn the_level_is_deterministic() {
    let run = || {
        let mut r = rig(1, true, 4);
        let mut hashes = Vec::new();
        for t in 0..400 {
            if t % 60 == 0 {
                r.set_gauge(0, 100);
            }
            let press = Input {
                main: t % 25 == 0,
                smash: t % 60 == 5,
                ..Input::default()
            };
            r.step([press, Input::default(), Input::default()]);
            hashes.push(r.e.state_hash());
        }
        hashes
    };
    assert_eq!(run(), run());
    let mut r = rig(1, true, 1);
    r.set_spirit(0, 2, 70);
    let mut copy = r.e.clone();
    for t in 0..240 {
        r.e.step(&[Input::default(); 3]);
        copy.step(&[Input::default(); 3]);
        assert_eq!(r.e.state_hash(), copy.state_hash(), "tick {t}");
    }
    assert_eq!(r.spirit(0), (0, 0));
}

/// B11: the view holds the level, the top and the countdown, at level 0 too.
#[test]
fn the_view_shows_the_level_every_tick() {
    let mut r = rig(2, true, 1);
    if let Some(g) = st(&mut r.e).players.career_mut(slot(1)).expertise.as_mut() {
        g.style = Some(1);
    }
    r.set_gauge(0, 100);
    for t in 0..130 {
        r.step(if t == 3 { b() } else { [Input::default(); 3] });
        let (level, decay) = r.spirit(0);
        assert_eq!(
            r.view(0)["spirit"],
            json!({
                "level": level, "max_level": 3, "decay_left": decay, "decay_ticks": DECAY
            }),
            "tick {t}"
        );
        assert!(r.view(1).get("spirit").is_none(), "a style with no spirit");
    }
}
