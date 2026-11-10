//! Slots 1 and 2 act like slot 0, in slot order (ADR-SIM-7 point 6, ADR-SIM-8): they move,
//! evade, attack, regenerate and keep their orbits and runes going; a downed player's timers keep
//! running but it cannot act, regenerate or keep a dash or combo going.

use super::state::{OrbitState, PlayerIdx, State};
use super::{Blame, Culprit, Shot, World, hurt_player};
use crate::engine::{Engine, Run};
use crate::event::{DomainEvent, HitKind};
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::input::Input;
use crate::run_setup::RunSetup;
use crate::stage::Map;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

fn px(x: i32, y: i32) -> Point {
    Point {
        x: Fx::px(x),
        y: Fx::px(y),
    }
}

/// A run of `n` players on a 4000 by 4000 px map with no enemies, every player up at (2000, 2000).
fn start(n: u8) -> Engine {
    let setup = RunSetup {
        seed: 20_261_104,
        ..RunSetup::default()
    };
    let mut e = Engine::start_party(&setup, n).expect("a party");
    let (world, s) = live_mut(&mut e);
    world.stage.map = Some(Map {
        width: Fx::px(4000),
        height: Fx::px(4000),
        start: None,
        facing: None,
    });
    s.enemies.clear();
    s.companions.clear();
    for i in s.players.indices().collect::<Vec<_>>() {
        s.players[i].at = px(2000, 2000);
    }
    e
}

fn live_mut(e: &mut Engine) -> (&mut World, &mut State) {
    match e.run_mut() {
        Run::V2 { world, state } => (world, state),
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn state(e: &Engine) -> &State {
    match e.run() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn idle() -> [Input; 3] {
    [Input::default(); 3]
}

fn step(e: &mut Engine, n: u32, inputs: &[Input; 3]) {
    for _ in 0..n {
        e.step(inputs);
        let s = live_mut(e).1;
        s.enemies.clear();
        s.companions.clear();
    }
}

fn down(e: &mut Engine, i: u8) {
    let p = &mut live_mut(e).1.players[slot(i)];
    p.hp = 0;
    p.down_ticks = 100;
}

#[test]
fn slots_one_and_two_walk_like_slot_zero() {
    let mut e = start(3);
    let mut inputs = idle();
    inputs[1].move_x = 127;
    inputs[2].move_y = -127;
    step(&mut e, 10, &inputs);
    let s = state(&e);
    assert_eq!(s.players[slot(0)].at, px(2000, 2000));
    assert_eq!(s.players[slot(1)].at, px(2030, 2000));
    assert_eq!(s.players[slot(2)].at, px(2000, 1970));

    let mut solo = start(1);
    let mut inputs = idle();
    inputs[0].move_x = 127;
    step(&mut solo, 10, &inputs);
    assert_eq!(state(&solo).players[slot(0)].at, px(2030, 2000));
}

#[test]
fn slot_one_evades_and_slot_two_attacks() {
    let mut e = start(3);
    let mut inputs = idle();
    inputs[1].evade = true;
    inputs[2].main = true;
    step(&mut e, 1, &inputs);
    let s = state(&e);
    assert!(s.players[slot(1)].dash.is_some_and(|d| d.left > 0));
    assert_eq!(
        e.events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::Evaded))
            .count(),
        1
    );
    let owned = |i: u8| {
        s.runs
            .iter()
            .filter(|r| r.owner == s.players.id(slot(i)))
            .count()
    };
    assert_eq!((owned(0), owned(1)), (0, 0));
    assert!(owned(2) > 0, "slot 2's combo run");
}

#[test]
fn runs_start_in_slot_order_and_the_replay_is_deterministic() {
    let run = || {
        let mut e = start(3);
        let mut inputs = idle();
        for i in &mut inputs {
            i.main = true;
        }
        e.step(&inputs);
        let owners: Vec<_> = state(&e).runs.iter().map(|r| r.owner).collect();
        let mut hashes = vec![e.state_hash()];
        for t in 0..600u32 {
            let mut inputs = inputs;
            inputs[1].move_x = if t % 50 < 25 { 100 } else { -100 };
            inputs[2].evade = t % 90 < 5;
            step(&mut e, 1, &inputs);
            hashes.push(e.state_hash());
        }
        (owners, hashes)
    };
    let (owners, hashes) = run();
    let e = start(3);
    let ids: Vec<_> = (0..3).map(|i| state(&e).players.id(slot(i))).collect();
    let mut seen = owners
        .iter()
        .filter_map(|o| ids.iter().position(|i| i == o));
    let mut last = 0;
    for p in seen.by_ref() {
        assert!(p >= last, "runs in slot order: {owners:?}");
        last = p;
    }
    assert!(last == 2, "all three slots started a run");
    assert_eq!(run().1, hashes);
}

#[test]
fn regen_reaches_the_living_and_not_the_downed() {
    let mut e = start(3);
    {
        let s = live_mut(&mut e).1;
        for i in 0..3 {
            s.players.career_mut(slot(i)).regen = 5;
            s.players[slot(i)].max_hp = 100;
            s.players[slot(i)].hp = 10;
        }
        s.tick = super::loadout::REGEN_TICKS - 1;
    }
    down(&mut e, 2);
    step(&mut e, 1, &idle());
    let s = state(&e);
    assert_eq!(s.players[slot(0)].hp, 15);
    assert_eq!(s.players[slot(1)].hp, 15);
    assert_eq!(s.players[slot(2)].hp, 0);
    assert!(s.players[slot(2)].is_downed());
}

#[test]
fn slot_ones_orbit_attacks_while_it_is_up_and_not_when_downed() {
    for downed in [false, true] {
        let mut e = start(2);
        let key = e
            .content()
            .rewards()
            .rewards()
            .iter()
            .position(|r| r.orbit().is_some())
            .expect("an orbit reward");
        let key = u16::try_from(key).unwrap();
        super::orbit::take(&mut live_mut(&mut e).1.players[slot(1)], key, 1);
        if downed {
            down(&mut e, 1);
        }
        step(&mut e, 2, &idle());
        let s = state(&e);
        let id = s.players.id(slot(1));
        let started = s
            .runs
            .iter()
            .any(|r| r.owner == id && r.slot >= super::orbit::FIRST_SLOT);
        assert_eq!(started, !downed, "downed={downed}");
        let _: &OrbitState = &s.players[slot(1)].orbits[0];
    }
}

#[test]
fn slot_ones_thrown_rune_flies_when_slot_zero_holds_none() {
    let mut e = start(2);
    step(&mut e, 1, &idle());
    let mut inputs = idle();
    inputs[1].main = true;
    step(&mut e, 1, &inputs);
    let id = state(&e).players.id(slot(1));
    let (world, s) = live_mut(&mut e);
    assert!(s.players[slot(0)].runes.is_empty());
    let Some(i) = s.runs.iter().position(|r| r.owner == id) else {
        panic!("slot 1 has a run");
    };
    s.runs[i].at = Some(px(2000, 2000));
    s.runs[i].throw = Some(px(2100, 2000));
    s.players[slot(1)].runes.push(super::state::HeldRune {
        rune: crate::rune::Rune::Boomerang,
        cooldown_left: 0,
    });
    let _ = world;
    // slot 1 holds a rune, so the step runs (the guard reads every present player).
    step(&mut e, 1, &idle());
    let moved = state(&e)
        .runs
        .iter()
        .find(|r| r.owner == id && r.throw.is_some());
    assert!(moved.is_some_and(|r| r.at != Some(px(2000, 2000))));
}

#[test]
fn a_downed_players_timers_run_and_it_cannot_act() {
    let mut e = start(2);
    down(&mut e, 1);
    {
        let p = &mut live_mut(&mut e).1.players[slot(1)];
        p.evade_recovering = vec![30];
        p.invulnerable = 20;
        p.combo.dash_strike_left = 10;
    }
    let before = state(&e).players[slot(1)].clone();
    let mut inputs = idle();
    inputs[1].main = true;
    inputs[1].evade = true;
    inputs[1].move_x = 127;
    step(&mut e, 5, &inputs);
    let s = state(&e);
    let p = &s.players[slot(1)];
    assert_eq!(p.evade_recovering, vec![25]);
    assert_eq!(p.invulnerable, 15);
    assert_eq!(p.combo.dash_strike_left, 5);
    assert_eq!((p.at, p.facing), (before.at, before.facing));
    assert!(p.dash.is_none());
    assert!(!s.runs.iter().any(|r| r.owner == s.players.id(slot(1))));
}

#[test]
fn a_downed_players_skill_cooldown_runs_down() {
    let mut e = start(2);
    down(&mut e, 1);
    live_mut(&mut e).1.players[slot(1)].skill = Some(super::state::SkillSlot {
        index: 0,
        level: 1,
        cooldown_left: 40,
    });
    step(&mut e, 5, &idle());
    assert_eq!(
        state(&e).players[slot(1)].skill.map(|k| k.cooldown_left),
        Some(35)
    );
}

#[test]
fn held_follows_the_input_while_downed_so_a_revive_is_no_press() {
    let mut e = start(2);
    down(&mut e, 1);
    let mut inputs = idle();
    inputs[1].main = true;
    step(&mut e, 10, &inputs);
    {
        let p = &mut live_mut(&mut e).1.players[slot(1)];
        p.hp = 50;
        p.max_hp = 100;
    }
    step(&mut e, 1, &inputs);
    let s = state(&e);
    assert!(!s.runs.iter().any(|r| r.owner == s.players.id(slot(1))));
}

#[test]
fn the_revive_precedes_the_timers() {
    let mut e = start(2);
    down(&mut e, 1);
    {
        let s = live_mut(&mut e).1;
        s.players[slot(1)].revive_hold = 179;
        s.players[slot(0)].at = px(2010, 2000);
    }
    let mut inputs = idle();
    inputs[0].interact = true;
    step(&mut e, 1, &inputs);
    let hit = live_mut(&mut e).0.tuning.hit_invulnerable_ticks;
    let p = &state(&e).players[slot(1)];
    assert!(p.is_up());
    assert_eq!(p.invulnerable, hit - 1);
}

fn lethal(e: &mut Engine, i: u8) -> Vec<DomainEvent> {
    let (world, s) = live_mut(e);
    let at = s.players[slot(i)].at;
    let shot = Shot {
        at,
        owner: EntityId(99),
        friendly: false,
        damage: 1_000_000,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        rune: None,
        chain: false,
    };
    let by = Blame {
        kind: HitKind::Attack,
        by: Culprit::Run(super::world::AttackIndex(0)),
    };
    let mut events = Vec::new();
    hurt_player(world, s, slot(i), shot, by, &mut events);
    assert!(s.players[slot(i)].is_downed() || s.players.len() == 1);
    events
}

#[test]
fn going_down_ends_a_dash_and_a_combo() {
    let mut e = start(2);
    let mut inputs = idle();
    inputs[1].main = true;
    inputs[1].evade = true;
    step(&mut e, 2, &inputs);
    {
        let s = state(&e);
        assert!(s.players[slot(1)].dash.is_some() || s.players[slot(1)].combo.running);
    }
    live_mut(&mut e).1.players[slot(1)].dash = Some(super::state::DashState {
        dir: super::state::Dir::DOWN,
        done: 0,
        left: 6,
    });
    let events = lethal(&mut e, 1);
    let s = state(&e);
    let p = &s.players[slot(1)];
    assert!(p.dash.is_none(), "the dash ends");
    assert!(!p.combo.running);
    let id = s.players.id(slot(1));
    assert!(!s.runs.iter().any(|r| r.owner == id && r.slot == 0));
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, DomainEvent::ComboReset))
    );
    let at = p.at;
    step(&mut e, 30, &idle());
    assert_eq!(state(&e).players[slot(1)].at, at);
}

#[test]
fn an_out_player_takes_the_timers_only_path() {
    let mut e = start(2);
    {
        let p = &mut live_mut(&mut e).1.players[slot(1)];
        p.hp = 0;
        p.out = true;
        p.invulnerable = 9;
    }
    let mut inputs = idle();
    inputs[1].main = true;
    inputs[1].move_x = 127;
    step(&mut e, 4, &inputs);
    let s = state(&e);
    assert_eq!(s.players[slot(1)].at, px(2000, 2000));
    assert_eq!(s.players[slot(1)].invulnerable, 5);
    assert!(!s.runs.iter().any(|r| r.owner == s.players.id(slot(1))));
}

// The rooted reviver (ADR-SIM-7): a player holding interact within `REVIVE_RANGE` of a downed
// teammate cannot move, attack or evade.

/// Slot 1 downed 30 px to the right of slot 0, which holds interact and every action.
fn reviver(extra: u8) -> (Engine, [Input; 3]) {
    let mut e = start(extra);
    live_mut(&mut e).1.players[slot(1)].at = px(2030, 2000);
    down(&mut e, 1);
    let mut inputs = idle();
    inputs[0] = Input {
        interact: true,
        move_x: 127,
        main: true,
        evade: true,
        skill: true,
        ..Input::default()
    };
    (e, inputs)
}

fn hold_of(e: &Engine) -> u32 {
    state(e).players[slot(1)].revive_hold
}

#[test]
fn a_reviver_in_range_holding_interact_is_rooted() {
    let (mut e, inputs) = reviver(2);
    live_mut(&mut e).1.players[slot(0)].skill = Some(super::state::SkillSlot {
        index: 0,
        level: 1,
        cooldown_left: 0,
    });
    step(&mut e, 10, &inputs);
    let s = state(&e);
    let p = &s.players[slot(0)];
    assert_eq!(p.at, px(2000, 2000));
    assert!(p.dash.is_none());
    assert!(!s.runs.iter().any(|r| r.owner == s.players.id(slot(0))));
    assert_eq!(p.skill.map(|k| k.cooldown_left), Some(0), "no skill used");
    assert_eq!(hold_of(&e), 10);
}

#[test]
fn letting_go_of_interact_frees_the_player_that_tick() {
    let (mut e, mut inputs) = reviver(2);
    step(&mut e, 10, &inputs);
    inputs[0].interact = false;
    inputs[0].main = false;
    inputs[0].evade = false;
    inputs[0].skill = false;
    step(&mut e, 1, &inputs);
    assert_eq!(state(&e).players[slot(0)].at, px(2003, 2000));
    assert_eq!(hold_of(&e), 0);
}

#[test]
fn out_of_range_is_not_rooted() {
    let (mut e, mut inputs) = reviver(2);
    live_mut(&mut e).1.players[slot(1)].at = px(2100, 2000);
    inputs[0].main = false;
    inputs[0].evade = false;
    inputs[0].skill = false;
    step(&mut e, 10, &inputs);
    assert_eq!(state(&e).players[slot(0)].at, px(2030, 2000));
    assert_eq!(hold_of(&e), 0);
}

#[test]
fn a_charge_in_progress_breaks_once_and_releases_nothing() {
    let (mut e, mut inputs) = reviver(2);
    inputs[0].evade = false;
    inputs[0].skill = false;
    live_mut(&mut e).1.players[slot(0)].combo.charge = Some(5);
    let mut broken = 0;
    for _ in 0..5 {
        step(&mut e, 1, &inputs);
        broken += e
            .events()
            .iter()
            .filter(|ev| matches!(ev, DomainEvent::ChargeBroken))
            .count();
    }
    let s = state(&e);
    assert_eq!(broken, 1);
    assert!(s.players[slot(0)].combo.charge.is_none());
    assert!(!s.runs.iter().any(|r| r.owner == s.players.id(slot(0))));
}

#[test]
fn every_reviver_in_range_is_rooted_and_one_hold_counts() {
    let (mut e, inputs) = reviver(3);
    let mut inputs = inputs;
    inputs[2] = inputs[0];
    live_mut(&mut e).1.players[slot(2)].at = px(2030, 2020);
    step(&mut e, 5, &inputs);
    let s = state(&e);
    assert_eq!(s.players[slot(0)].at, px(2000, 2000));
    assert_eq!(s.players[slot(2)].at, px(2030, 2020));
    assert_eq!(hold_of(&e), 5);
}

#[test]
fn the_reviver_is_free_on_the_tick_the_revive_completes() {
    let (mut e, mut inputs) = reviver(2);
    inputs[0].main = false;
    inputs[0].evade = false;
    inputs[0].skill = false;
    live_mut(&mut e).1.players[slot(1)].revive_hold = 179;
    step(&mut e, 1, &inputs);
    assert!(state(&e).players[slot(1)].is_up());
    assert_eq!(state(&e).players[slot(0)].at, px(2003, 2000));
}

#[test]
fn solo_and_out_teammates_do_not_root() {
    let mut solo = start(1);
    let mut inputs = idle();
    inputs[0].interact = true;
    inputs[0].move_x = 127;
    step(&mut solo, 10, &inputs);
    assert_eq!(state(&solo).players[slot(0)].at, px(2030, 2000));

    let (mut e, mut inputs) = reviver(2);
    inputs[0].main = false;
    inputs[0].evade = false;
    inputs[0].skill = false;
    live_mut(&mut e).1.players[slot(1)].out = true;
    step(&mut e, 10, &inputs);
    assert_eq!(state(&e).players[slot(0)].at, px(2030, 2000));
}
