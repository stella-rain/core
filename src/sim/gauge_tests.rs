//! The weapon gauge a style grants (ADR-SIM-5, ADR-SIM-7, ADR-SIM-9 points 2 to 7): the fills from
//! a normal step's hits and from hit points lost, the full-gauge smash, the orb clear around it,
//! and a run that names none. The tests build a bundled pack patched through `serde_json`, with a
//! gauge on the Sword's first style, and stack the enemies on the player so a swing hits them.

use super::state::{OrbState, PlayerIdx, State};
use super::{Blame, Culprit, Shot, hurt_player};
use crate::bench_fixtures::worst_case;
use crate::engine::{Engine, Run};
use crate::event::{DomainEvent, HitKind};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use crate::pack::Pack;
use crate::run_setup::RunSetup;
use serde_json::{Value, json};

const ME: PlayerIdx = PlayerIdx::SOLO;
const BIG: u32 = 1_000_000_000;

/// The test gauge: 100 long; every number is a test value, not content.
#[derive(Clone, Copy)]
struct Cfg {
    per_step: u32,
    per_hit: u32,
    pct: u32,
    cap: u32,
}

const STEP: Cfg = Cfg {
    per_step: 10,
    per_hit: 0,
    pct: 0,
    cap: 14,
};

/// The full-gauge attack: one wind-up tick, two active ticks and a recovery, a circle that is
/// unlike every attack of the Sword's tree.
fn full_attack() -> Value {
    json!({ "inline": [
        { "kind": "wind_up", "ticks": 1 },
        { "kind": "active", "ticks": 2, "type": "slash", "damage": 9000,
          "hit_box": { "type": "circle", "radius": 12000, "offset": { "forward": 0, "right": 0 } } },
        { "kind": "recovery", "ticks": 6 }
    ] })
}

fn pack(cfg: Option<Cfg>, radius_px: Option<i32>) -> Pack {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    if let Some(cfg) = cfg {
        let w = v["expertise"]["weapons"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|w| w["weapon"] == "giant_sword")
            .expect("the Sword's expertise");
        w["styles"][0]["kinds"] = json!([{
            "type": "gauge", "max": 100, "per_step": cfg.per_step, "per_hit": cfg.per_hit,
            "from_damage_pct": cfg.pct, "step_cap": cfg.cap,
            "spend": { "type": "full_smash", "attack": full_attack() }
        }]);
    }
    if let Some(px) = radius_px {
        v["rules"]["orb_clear_radius"] = json!(px * 256);
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

/// A run of `players` on the Sword that names `cfg` on style 0, every player on that style and
/// hardy, with `foes` enemies of ids 9000.. kept stacked on the first player.
struct Rig {
    e: Engine,
    foes: Vec<EntityId>,
    players: u8,
}

fn rig(players: u8, cfg: Option<Cfg>, radius_px: Option<i32>, foes: u32) -> Rig {
    let pack = pack(cfg, radius_px);
    let setup = RunSetup {
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        seed: 20_261_112,
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
        f.id = EntityId(9000 + i);
        f.hp = BIG;
        f.max_hp = BIG;
        ids.push(f.id);
        s.enemies.push(f);
    }
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
        players,
    }
}

impl Rig {
    fn stack(&mut self) {
        let s = st(&mut self.e);
        let (at, facing) = (s.players[ME].at, s.players[ME].facing);
        s.enemies.retain(|x| self.foes.contains(&x.id));
        // A column across the facing, 24 px ahead and a few px apart, inside every sword swing.
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

    fn step(&mut self, inputs: [Input; 3]) {
        self.stack();
        self.e.step(&inputs);
    }

    fn idle(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step([Input::default(); 3]);
        }
    }

    /// One press of `a` or `b` by player 0, then `ticks` idle ticks.
    fn press(&mut self, b: bool, ticks: u32) {
        let press = Input {
            main: !b,
            smash: b,
            ..Input::default()
        };
        self.step([press, Input::default(), Input::default()]);
        self.idle(ticks);
    }

    fn gauge(&mut self, i: u8) -> (u32, u32) {
        let c = &st(&mut self.e).players[slot(i)].combat;
        (c.weapon_gauge, c.gauge_step_fill)
    }

    fn set_gauge(&mut self, i: u8, gauge: u32) {
        st(&mut self.e).players[slot(i)].combat.weapon_gauge = gauge;
    }

    /// The compiled attack of player `i`'s combo run, if it has one.
    fn attack_of(&mut self, i: u8) -> Option<u16> {
        let s = st(&mut self.e);
        let owner = s.players.id(slot(i));
        s.runs
            .iter()
            .find(|r| r.owner == owner && r.slot == 0)
            .map(|r| r.attack.0)
    }
}

/// B1: a run that names no gauge keeps both fields 0 through presses, hits and hits taken.
#[test]
fn a_run_with_no_gauge_never_moves_either_field() {
    let mut r = rig(1, None, None, 3);
    let Run::V2 { world, .. } = r.e.run() else {
        panic!("the live rules");
    };
    assert_eq!(world.tuning.orb_clear_radius, 0);
    assert!(!world.has_gauge);
    for t in 0..600 {
        let b = t % 90 == 60;
        let press = Input {
            main: !b && t % 30 == 0,
            smash: b,
            ..Input::default()
        };
        if t % 100 == 0 {
            st(&mut r.e).players[ME].hp = 500;
        }
        r.step([press, Input::default(), Input::default()]);
        assert_eq!(r.gauge(0), (0, 0), "tick {t}");
    }
}

/// B2: `per_step` is paid once to a step, however many of its active phases hit.
#[test]
fn a_step_that_hits_pays_per_step_once() {
    let mut r = rig(1, Some(STEP), None, 1);
    r.press(false, 40);
    assert_eq!(r.gauge(0), (10, 10));
    let mut none = rig(1, Some(STEP), None, 0);
    none.press(false, 40);
    assert_eq!(none.gauge(0), (0, 0), "a step that hits nothing");
}

/// B3: `per_hit` counts each enemy a swing hits, to the step's cap; the next step starts again.
#[test]
fn per_hit_is_held_to_the_step_cap_and_a_new_step_resets_it() {
    let cfg = Cfg {
        per_step: 0,
        per_hit: 2,
        ..STEP
    };
    let mut three = rig(1, Some(cfg), None, 3);
    three.press(false, 40);
    assert_eq!(three.gauge(0), (6, 6));
    let mut ten = rig(1, Some(cfg), None, 10);
    ten.press(false, 40);
    assert_eq!(ten.gauge(0), (14, 14), "the cap");
    ten.press(false, 40);
    assert_eq!(
        ten.gauge(0).1,
        14,
        "the next step fills again, to its own cap"
    );
    assert_eq!(ten.gauge(0).0, 28);
}

/// B4: the gauge never passes its max, and `gauge_step_fill` is what was really added.
#[test]
fn the_gauge_stops_at_its_max() {
    let mut r = rig(1, Some(STEP), None, 1);
    r.set_gauge(0, 95);
    r.press(false, 40);
    assert_eq!(r.gauge(0), (100, 5));
    let mut full = rig(1, Some(STEP), None, 1);
    full.set_gauge(0, 100);
    full.press(false, 40);
    assert_eq!(full.gauge(0), (100, 0));
}

/// B5: a smash, the full-gauge attack, a downed player and another player's hits fill nothing.
#[test]
fn a_smash_and_another_players_hits_fill_nothing() {
    let mut r = rig(1, Some(STEP), None, 3);
    r.press(true, 60);
    assert_eq!(r.gauge(0), (0, 0), "a smash");
    let mut party = rig(3, Some(STEP), None, 3);
    party.press(false, 40);
    assert_eq!(party.gauge(0).0, 10);
    assert_eq!(party.gauge(1), (0, 0));
    assert_eq!(party.gauge(2), (0, 0));
    assert_eq!(party.players, 3);
}

/// A hit of `damage` on player 0; the hit points it really took.
fn hurt(r: &mut Rig, damage: u32) -> u32 {
    let Run::V2 { world, state } = r.e.run_mut() else {
        panic!("the live rules");
    };
    let shot = Shot {
        at: state.players[ME].at,
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
    let before = state.players[ME].hp;
    hurt_player(world, state, ME, shot, by, &mut Vec::new());
    before - state.players[ME].hp
}

/// B6: hit points lost fill the bar by `max * lost * pct / (max_hp * 100)`, and not to the cap.
#[test]
fn hit_points_lost_fill_the_gauge() {
    let cfg = Cfg { pct: 100, ..STEP };
    let mut r = rig(1, Some(cfg), None, 0);
    let first = hurt(&mut r, 400);
    assert!(first >= 20, "a hit that takes a real share: {first}");
    assert_eq!(
        r.gauge(0).0,
        first / 10,
        "a tenth of the loss in units of a 100 bar"
    );
    assert!(
        first / 10 > cfg.cap,
        "the damage fill is not held to the step cap"
    );
    assert_eq!(r.gauge(0).1, 0, "and is no step's fill");
    let before = r.gauge(0).0;
    st(&mut r.e).players[ME].hp = 1;
    hurt(&mut r, 400);
    assert_eq!(
        r.gauge(0).0,
        before,
        "a hit that downs the player adds nothing"
    );

    let mut off = rig(1, Some(STEP), None, 0);
    assert!(hurt(&mut off, 400) > 0);
    assert_eq!(off.gauge(0).0, 0, "no `from_damage_pct`");
}

/// B7: a B press at a full gauge plays the gauge's attack and empties the gauge.
#[test]
fn a_full_gauge_changes_the_smash() {
    let mut full = rig(1, Some(STEP), None, 1);
    let ordinary = {
        let mut o = rig(1, Some(STEP), None, 1);
        let b = Input {
            smash: true,
            ..Input::default()
        };
        o.step([b, Input::default(), Input::default()]);
        o.attack_of(0).expect("a run")
    };
    full.set_gauge(0, 100);
    let b = Input {
        smash: true,
        ..Input::default()
    };
    full.step([b, Input::default(), Input::default()]);
    let played = full.attack_of(0).expect("a run");
    assert_ne!(played, ordinary, "the gauge's own attack");
    assert_eq!(full.gauge(0), (0, 0), "spent on the tick");
    full.idle(60);
    assert_eq!(full.gauge(0), (0, 0), "its hits fill nothing");

    let mut short = rig(1, Some(STEP), None, 1);
    short.set_gauge(0, 99);
    short.step([b, Input::default(), Input::default()]);
    assert_eq!(short.attack_of(0), Some(ordinary));
    assert_eq!(short.gauge(0).0, 99);

    let mut a = rig(1, Some(STEP), None, 0);
    a.set_gauge(0, 100);
    a.press(false, 40);
    assert_eq!(a.gauge(0).0, 100, "an A press keeps it");
}

/// B8: a gauge granted by the style goes with it: the player without the style has none.
#[test]
fn a_player_off_the_style_has_no_gauge() {
    let mut r = rig(1, Some(STEP), None, 1);
    let Run::V2 { world, state } = r.e.run() else {
        panic!("the live rules");
    };
    assert!(world.has_gauge);
    assert!(world.gauge_of(state, ME).is_some(), "on the style");
    if let Some(g) = st(&mut r.e).players.career_mut(ME).expertise.as_mut() {
        g.style = Some(1);
    }
    r.press(false, 40);
    assert_eq!(r.gauge(0), (0, 0));
}

fn orb(at: Point, breakable: bool) -> OrbState {
    OrbState {
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

fn px(me: Point, dx: i32, plus: i32) -> Point {
    Point {
        x: Fx(me.x.0 + Fx::px(dx).0 + plus),
        y: me.y,
    }
}

fn broken(events: &[DomainEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, DomainEvent::OrbBroken { .. }))
        .count()
}

/// B9: the full-gauge smash's first active tick clears the breakable orbs within the radius.
#[test]
fn the_full_gauge_smash_clears_the_orbs_around_the_player() {
    let mut r = rig(1, Some(STEP), Some(100), 0);
    let me = st(&mut r.e).players[ME].at;
    {
        let s = st(&mut r.e);
        s.orbs.push(orb(px(me, 50, 0), true));
        s.orbs.push(orb(px(me, 100, 0), true));
        s.orbs.push(orb(px(me, 100, 1), true));
        s.orbs.push(orb(px(me, 50, 0), false));
    }
    r.set_gauge(0, 100);
    let b = Input {
        smash: true,
        ..Input::default()
    };
    r.step([b, Input::default(), Input::default()]);
    assert_eq!(st(&mut r.e).orbs.len(), 4, "nothing in the wind-up");
    let mut seen = 0;
    for _ in 0..3 {
        r.step([Input::default(); 3]);
        seen += broken(r.e.events());
    }
    assert_eq!(seen, 2, "one event each, the edge included");
    assert_eq!(
        st(&mut r.e).orbs.len(),
        2,
        "past the radius and unbreakable"
    );

    let mut plain = rig(1, Some(STEP), Some(100), 0);
    let me = st(&mut plain.e).players[ME].at;
    st(&mut plain.e).orbs.push(orb(px(me, 50, 0), true));
    plain.press(true, 40);
    assert_eq!(
        st(&mut plain.e).orbs.len(),
        1,
        "an ordinary smash clears nothing"
    );
}

/// B10: three full players pressing B on one tick each spend their own gauge.
#[test]
fn each_player_spends_its_own_gauge() {
    let mut r = rig(3, Some(STEP), None, 1);
    for i in 0..3 {
        r.set_gauge(i, 100);
    }
    r.set_gauge(1, 40);
    let b = Input {
        smash: true,
        ..Input::default()
    };
    r.step([b, b, b]);
    assert_eq!(r.gauge(0), (0, 0));
    assert_eq!(r.gauge(1).0, 40, "not full: the ordinary smash");
    assert_eq!(r.gauge(2), (0, 0));
}

/// B11: the same scenario twice gives the same hash every tick.
#[test]
fn the_gauge_is_deterministic() {
    let run = || {
        let mut r = rig(1, Some(STEP), Some(100), 4);
        let mut hashes = Vec::new();
        for t in 0..300 {
            let press = Input {
                main: t % 25 == 0,
                smash: t % 97 == 96,
                ..Input::default()
            };
            r.step([press, Input::default(), Input::default()]);
            hashes.push(r.e.state_hash());
        }
        hashes
    };
    assert_eq!(run(), run());
}

/// B12: the bundled trial run, with presses and hits taken, never moves either field.
#[test]
fn the_bundled_run_keeps_the_gauge_at_zero() {
    let mut e = worst_case::warmed_up();
    for t in 0..600 {
        let press = Input {
            main: t % 20 == 0,
            smash: t % 70 == 69,
            ..Input::default()
        };
        e.step(&[press, Input::default(), Input::default()]);
        let c = &st(&mut e).players[ME].combat;
        assert_eq!((c.weapon_gauge, c.gauge_step_fill), (0, 0), "tick {t}");
    }
}
