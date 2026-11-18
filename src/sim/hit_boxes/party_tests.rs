
//! Hits on each player of a party (ADR-SIM-7): an enemy's swing or orb reaches every player
//! it covers, once each and in slot order, and the companions once.

use super::*;
use crate::bench_fixtures::worst_case;
use crate::engine::Run;
use crate::event::DomainEvent;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

fn party() -> (World, State) {
    let e = worst_case::warmed_up().into_party(3);
    let Run::V2 { world, state } = e.run() else {
        panic!("the live rules");
    };
    let mut s = state.clone();
    assert!(!s.runs.is_empty(), "a run for the swing to belong to");
    s.runs[0].hits.clear();
    for i in 0..3 {
        let p = &mut s.players[slot(i)];
        p.hp = p.max_hp;
        p.invulnerable = 0;
        p.at.x.0 = i32::from(i) * 4_000_000;
        p.at.y.0 = 0;
    }
    s.companions.clear();
    (World::clone(world), s)
}

fn swing(at: Point, radius: i32, friendly: bool, owner: EntityId) -> Swing {
    Swing {
        source: Source::Run(0),
        owner,
        slot: 0,
        friendly,
        area: Area {
            centre: at,
            facing: Dir::UP,
            radius,
            arc: None,
            line: None,
        },
        damage: 10,
        damage_type: None,
        rehit_ticks: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        poise_damage: None,
        stagger: None,
        rune: None,
        once: false,
    }
}

fn hp(s: &State) -> [u32; 3] {
    [0, 1, 2].map(|i| s.players[slot(i)].hp)
}

fn hurt_in(ev: &[DomainEvent]) -> usize {
    ev.iter()
        .filter(|e| matches!(e, DomainEvent::PlayerHit { .. }))
        .count()
}

#[test]
fn a_swing_hurts_the_player_it_covers_and_no_other() {
    let (w, mut s) = party();
    let before = hp(&s);
    let mut ev = Vec::new();
    let sw = swing(
        s.players[slot(2)].at,
        w.tuning.player_radius,
        false,
        EntityId(900),
    );
    hit_players_side(&w, &mut s, &sw, &mut ev);
    let after = hp(&s);
    assert_eq!((after[0], after[1]), (before[0], before[1]));
    assert!(after[2] < before[2]);
    assert_eq!(hurt_in(&ev), 1);
}

#[test]
fn a_swing_over_all_three_hurts_each_once_in_slot_order() {
    let (w, mut s) = party();
    for i in 1..3 {
        s.players[slot(i)].at = s.players[PlayerIdx::SOLO].at;
    }
    let before = hp(&s);
    let mut ev = Vec::new();
    let at = s.players[PlayerIdx::SOLO].at;
    let sw = swing(at, w.tuning.player_radius, false, EntityId(900));
    hit_players_side(&w, &mut s, &sw, &mut ev);
    assert!(hp(&s).iter().zip(before).all(|(a, b)| *a < b));
    assert_eq!(hurt_in(&ev), 3);
    let order: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            DomainEvent::PlayerHit { hp_left, .. } => Some(*hp_left),
            _ => None,
        })
        .collect();
    assert_eq!(order, hp(&s).to_vec(), "in slot order");
}

#[test]
fn an_invulnerable_player_is_skipped() {
    let (w, mut s) = party();
    for i in 1..3 {
        s.players[slot(i)].at = s.players[PlayerIdx::SOLO].at;
    }
    s.players[slot(1)].invulnerable = 30;
    let before = hp(&s);
    let mut ev = Vec::new();
    let at = s.players[PlayerIdx::SOLO].at;
    let sw = swing(at, w.tuning.player_radius, false, EntityId(900));
    hit_players_side(&w, &mut s, &sw, &mut ev);
    let after = hp(&s);
    assert!(after[0] < before[0] && after[2] < before[2]);
    assert_eq!(after[1], before[1]);
}

#[test]
fn an_orb_is_used_up_by_the_first_player_it_overlaps() {
    let (w, mut s) = party();
    s.players[slot(1)].at = s.players[PlayerIdx::SOLO].at;
    let before = hp(&s);
    let shot = Shot {
        at: s.players[PlayerIdx::SOLO].at,
        owner: EntityId(900),
        friendly: false,
        damage: 10,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        rune: None,
        chain: false,
        dot: false,
    };
    let by = Blame::of_program(HitKind::Attack, crate::sim::world::ProgramIndex(0));
    let mut ev = Vec::new();
    assert!(super::super::hit_players_side(
        &w, &mut s, shot, by, &mut ev
    ));
    let after = hp(&s);
    assert!(after[0] < before[0]);
    assert_eq!((after[1], after[2]), (before[1], before[2]));
    assert_eq!(hurt_in(&ev), 1);
}

fn orb(
    at: Point,
    owner: EntityId,
    link: Option<crate::sim::state::OrbLink>,
) -> crate::sim::state::OrbState {
    crate::sim::state::OrbState {
        at,
        vx: Fx(0),
        vy: Fx(0),
        dir: Dir::UP,
        style: 0,
        owner,
        damage: 1,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        hook: None,
        breakable: true,
        life: 100,
        stop: None,
        link,
        program: crate::sim::world::ProgramIndex(0),
    }
}

#[test]
fn a_players_swing_breaks_an_orb_and_a_companions_does_not() {
    let (w, mut s) = party();
    for (owner, broken) in [(slot(1).id(), true), (EntityId(900), false)] {
        s.orbs = vec![orb(Point::default(), EntityId(900), None)];
        let sw = swing(Point::default(), 1_000_000, true, owner);
        let mut ev = Vec::new();
        for me in s.players.indices() {
            break_orbs(&w, &mut s, me, std::slice::from_ref(&sw), &mut ev);
        }
        assert_eq!(s.orbs.is_empty(), broken, "owner {owner:?}");
        assert_eq!(
            ev.iter()
                .filter(|e| matches!(e, DomainEvent::OrbBroken { .. }))
                .count(),
            usize::from(broken)
        );
    }
}

#[test]
fn a_beam_hurts_one_player_per_rehit_by_the_shared_last_hit() {
    let (w, mut s) = party();
    let link = |slot| crate::sim::state::OrbLink {
        group: 1,
        slot,
        of: 2,
        width: Fx::px(40),
        damage: 10,
        rehit_ticks: 30,
        last_hit: None,
    };
    let (a, b) = (
        Point::default(),
        Point {
            x: Fx::px(200),
            y: Fx(0),
        },
    );
    s.orbs = vec![
        orb(a, EntityId(900), Some(link(0))),
        orb(b, EntityId(900), Some(link(1))),
    ];
    s.tick = 100;
    let mid = Point {
        x: Fx::px(100),
        y: Fx(0),
    };
    for i in 0..2 {
        s.players[slot(i)].at = mid;
    }
    let before = hp(&s);
    let mut ev = Vec::new();
    for me in s.players.indices() {
        cage_beams(&w, &mut s, me, &mut ev);
    }
    let after = hp(&s);
    assert!(after[0] < before[0], "the lowest slot is hit");
    assert_eq!(
        after[1], before[1],
        "the shared last hit spares slot 1 this tick"
    );
    assert_eq!(hurt_in(&ev), 1);
}

// The near miss (the perfect evade): the evade's own invulnerability stopped a hit inside the
// window. Test values: window 10, slow 60, cooldown 300, an evade of 10 ticks.

const EVADE_TICKS: u32 = 10;

fn tuned() -> (World, State) {
    let (mut w, s) = party();
    w.tuning.perfect_evade = Some(crate::rules::PerfectEvade {
        window_ticks: 10,
        slow_ticks: 60,
        cooldown_ticks: 300,
        reach: Fx::px(200).0,
        slow_every: 2,
    });
    w.stage.player.evade.invulnerable_ticks = EVADE_TICKS;
    (w, s)
}

/// Slot `i` pressed its evade `since` ticks ago.
fn pressed(s: &mut State, i: u8, since: u16) {
    let p = &mut s.players[slot(i)];
    p.combat.perfect_window = 10 - since;
    p.invulnerable = EVADE_TICKS - u32::from(since);
}

fn timers(s: &State, i: u8) -> (u16, u16, u16) {
    let c = &s.players[slot(i)].combat;
    (c.perfect_window, c.perfect_slow, c.perfect_cooldown)
}

fn hit_solo(w: &World, s: &mut State, ev: &mut Vec<DomainEvent>) {
    let sw = swing(
        s.players[PlayerIdx::SOLO].at,
        w.tuning.player_radius,
        false,
        FOE,
    );
    hit_players_side(w, s, &sw, ev);
}

#[test]
fn a_hit_box_the_evade_stops_inside_the_window_is_a_perfect_near_miss() {
    for since in [0, 3, 9] {
        let (w, mut s) = tuned();
        pressed(&mut s, 0, since);
        let before = hp(&s);
        let mut ev = Vec::new();
        hit_solo(&w, &mut s, &mut ev);
        assert_eq!(hp(&s), before, "since {since}");
        assert_eq!(hurt_in(&ev), 0);
        assert_eq!(timers(&s, 0), (0, 60, 300), "since {since}");
    }
    let (w, mut s) = tuned();
    pressed(&mut s, 0, 9);
    s.players[slot(0)].invulnerable = 0;
    s.players[slot(0)].combat.perfect_window = 0;
    let before = hp(&s);
    hit_solo(&w, &mut s, &mut Vec::new());
    assert!(hp(&s)[0] < before[0], "nothing left to stop it hits");
}

/// The `PerfectEvade` events of `ev`, as JSON.
fn perfect_events(ev: &[DomainEvent]) -> Vec<serde_json::Value> {
    ev.iter()
        .map(|e| serde_json::to_value(e).expect("an event"))
        .filter(|e| e["type"] == "PerfectEvade")
        .collect()
}

#[test]
fn a_perfect_near_miss_says_so_once_and_the_rest_say_nothing() {
    for since in [0, 3, 9] {
        let (w, mut s) = tuned();
        pressed(&mut s, 0, since);
        let mut ev = Vec::new();
        hit_solo(&w, &mut s, &mut ev);
        assert_eq!(
            perfect_events(&ev),
            vec![serde_json::json!({ "type": "PerfectEvade",
                    "data": { "player": PlayerIdx::SOLO } })],
            "since {since}"
        );
        // A second near miss during the cooldown: nothing.
        pressed(&mut s, 0, since);
        let mut again = Vec::new();
        hit_solo(&w, &mut s, &mut again);
        assert!(perfect_events(&again).is_empty(), "in the cooldown");
    }
    // Outside the window: no event.
    let (w, mut s) = tuned();
    s.players[slot(0)].invulnerable = 4;
    let mut ev = Vec::new();
    hit_solo(&w, &mut s, &mut ev);
    assert!(perfect_events(&ev).is_empty());
    // No rule: no event.
    let (mut w, mut s) = tuned();
    w.tuning.perfect_evade = None;
    pressed(&mut s, 0, 0);
    let mut ev = Vec::new();
    hit_solo(&w, &mut s, &mut ev);
    assert!(perfect_events(&ev).is_empty());
}

#[test]
fn a_hit_residue_longer_than_the_evade_is_not_a_near_miss_and_a_tie_is() {
    // A hit 5 ticks before the press left 55 ticks; two ticks after the press the evade has 8.
    let (w, mut s) = tuned();
    pressed(&mut s, 0, 2);
    s.players[slot(0)].invulnerable = 53;
    let before = hp(&s);
    hit_solo(&w, &mut s, &mut Vec::new());
    assert_eq!(hp(&s), before, "skipped");
    assert_eq!(timers(&s, 0), (8, 0, 0), "the window stays open, no slow");
    // Residue equal to what the evade has left: a tie goes to the evade.
    s.players[slot(0)].invulnerable = 8;
    hit_solo(&w, &mut s, &mut Vec::new());
    assert_eq!(timers(&s, 0), (0, 60, 300));
}

#[test]
fn a_skip_outside_the_window_or_of_a_downed_player_opens_nothing() {
    let (w, mut s) = tuned();
    // The window ran out (4 ticks) while the evade's 10 go on: skipped, not a near miss.
    s.players[slot(0)].invulnerable = 4;
    hit_solo(&w, &mut s, &mut Vec::new());
    assert_eq!(timers(&s, 0), (0, 0, 0));
    // A downed player records nothing.
    pressed(&mut s, 0, 0);
    s.players[slot(0)].hp = 0;
    let mut ev = Vec::new();
    hit_solo(&w, &mut s, &mut ev);
    assert_eq!(timers(&s, 0), (10, 0, 0));
    assert_eq!(hurt_in(&ev), 0);
}

#[test]
fn a_near_miss_in_the_cooldown_closes_the_window_and_slows_nothing() {
    let (w, mut s) = tuned();
    pressed(&mut s, 0, 1);
    s.players[slot(0)].combat.perfect_cooldown = 5;
    let before = hp(&s);
    hit_solo(&w, &mut s, &mut Vec::new());
    assert_eq!(hp(&s), before);
    assert_eq!(timers(&s, 0), (0, 0, 5));
    // A second near miss 59 ticks into a slow cannot restart it.
    pressed(&mut s, 0, 0);
    s.players[slot(0)].combat.perfect_slow = 1;
    hit_solo(&w, &mut s, &mut Vec::new());
    assert_eq!(timers(&s, 0), (0, 1, 5));
}

#[test]
fn an_orb_and_a_cage_beam_the_evade_stops_are_near_misses() {
    let (w, mut s) = tuned();
    pressed(&mut s, 0, 0);
    let shot = Shot {
        at: s.players[PlayerIdx::SOLO].at,
        owner: FOE,
        friendly: false,
        damage: 10,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        rune: None,
        chain: false,
        dot: false,
    };
    let by = Blame::of_program(HitKind::Attack, crate::sim::world::ProgramIndex(0));
    let before = hp(&s);
    let used = super::super::hit_players_side(&w, &mut s, shot, by, &mut Vec::new());
    assert!(!used, "the orb flies on");
    assert_eq!(hp(&s), before);
    assert_eq!(timers(&s, 0), (0, 60, 300));

    let (w, mut s) = tuned();
    pressed(&mut s, 0, 0);
    let link = |slot| crate::sim::state::OrbLink {
        group: 1,
        slot,
        of: 2,
        width: Fx::px(40),
        damage: 10,
        rehit_ticks: 30,
        last_hit: None,
    };
    let far = Point {
        x: Fx::px(200),
        y: Fx(0),
    };
    s.orbs = vec![
        orb(Point::default(), FOE, Some(link(0))),
        orb(far, FOE, Some(link(1))),
    ];
    s.players[slot(0)].at = Point {
        x: Fx::px(100),
        y: Fx(0),
    };
    let before = hp(&s);
    let mut ev = Vec::new();
    cage_beams(&w, &mut s, slot(0), &mut ev);
    assert_eq!(hp(&s), before);
    assert_eq!(hurt_in(&ev), 0);
    assert_eq!(timers(&s, 0), (0, 60, 300));
}

// A run end made during the pass (a player downed, a hold broken, a stun) must not shift the
// places the swings hold into `s.runs` (ADR-SIM-7).

const FOE: EntityId = EntityId(900);

/// A run of `owner` in `slot`, cloned from a real one, pushed; its place.
fn add_run(
    s: &mut State,
    template: &crate::sim::state::AttackRun,
    owner: EntityId,
    slot: u8,
) -> usize {
    let mut run = template.clone();
    run.owner = owner;
    run.slot = slot;
    run.hits = Vec::new();
    s.runs.push(run);
    s.runs.len() - 1
}

fn at_place(mut w: Swing, place: usize, slot: u8) -> Swing {
    w.source = Source::Run(place);
    w.slot = slot;
    w
}

fn pass(w: &World, s: &mut State, swings: &[Swing]) -> Vec<DomainEvent> {
    let mut ev = Vec::new();
    let (mut bits, mut found) = (Vec::new(), Vec::new());
    land(w, s, &mut None, swings, (&mut bits, &mut found), &mut ev);
    ev
}

fn hit_ids(s: &State, place: usize) -> Vec<EntityId> {
    s.runs[place].hits.iter().map(|h| h.0).collect()
}

fn downed(ev: &[DomainEvent]) -> usize {
    ev.iter()
        .filter(|e| matches!(e, DomainEvent::PlayerDowned { .. }))
        .count()
}

/// A party where players 1 and 2 stand together far from player 0, and player 1 goes down
/// on one hit. The runs: slot 1's combo run, then `foes` enemy runs.
fn downing(foes: usize) -> (World, State) {
    let (w, mut s) = party();
    let template = s.runs[0].clone();
    s.runs.clear();
    s.players[slot(2)].at = s.players[slot(1)].at;
    s.players[slot(1)].hp = 1;
    add_run(
        &mut s,
        &template,
        slot(1).id(),
        crate::sim::player::COMBO_SLOT,
    );
    for id in (900..).take(foes) {
        add_run(&mut s, &template, EntityId(id), 0);
    }
    (w, s)
}

fn over(s: &State, i: u8, w: &World, owner: EntityId, place: usize) -> Swing {
    at_place(
        swing(s.players[slot(i)].at, w.tuning.player_radius, false, owner),
        place,
        0,
    )
}

#[test]
fn a_player_downed_mid_swing_leaves_the_rest_of_the_swing_its_run() {
    let (w, mut s) = downing(1);
    let one = over(&s, 1, &w, FOE, 1);
    let ev = pass(&w, &mut s, &[one]);
    assert_eq!(s.players[slot(1)].hp, 0);
    assert_eq!(downed(&ev), 1);
    assert_eq!(hurt_in(&ev), 2, "players 1 and 2 are each hurt once");
    assert_eq!(s.runs.len(), 1, "the downed player's combo run is gone");
    assert_eq!(hit_ids(&s, 0), vec![slot(1).id(), slot(2).id()]);
}

#[test]
fn a_later_swing_writes_to_its_own_run_after_a_down() {
    let (w, mut s) = downing(2);
    let before = s.players[slot(0)].hp;
    let swings = [over(&s, 1, &w, FOE, 1), over(&s, 0, &w, FOE, 2)];
    let ev = pass(&w, &mut s, &swings);
    assert_eq!(downed(&ev), 1);
    assert!(
        s.players[slot(0)].hp < before,
        "the second swing hurts player 0"
    );
    assert_eq!(s.runs.len(), 2);
    assert_eq!(hit_ids(&s, 0), vec![slot(1).id(), slot(2).id()]);
    assert_eq!(hit_ids(&s, 1), vec![slot(0).id()]);
}

#[test]
fn a_down_does_not_move_a_hit_into_another_runs_list() {
    let (w, mut s) = downing(3);
    // The second foe has hit player 0 already; the third has hit nobody.
    s.runs[2].hits.push((slot(0).id(), 0));
    let swings = [over(&s, 1, &w, FOE, 1), over(&s, 0, &w, FOE, 2)];
    pass(&w, &mut s, &swings);
    assert_eq!(s.runs.len(), 3);
    assert_eq!(hit_ids(&s, 0), vec![slot(1).id(), slot(2).id()]);
    assert_eq!(
        hit_ids(&s, 1),
        vec![slot(0).id()],
        "its own list, from before"
    );
    assert!(
        hit_ids(&s, 2).is_empty(),
        "the third foe's list is untouched"
    );
}

#[test]
fn a_beam_that_downs_a_player_leaves_the_swings_their_runs() {
    let (w, mut s) = downing(1);
    let mid = s.players[slot(1)].at;
    s.players[slot(2)].at = Point {
        x: Fx(mid.x.0 + 4_000_000),
        y: mid.y,
    };
    let link = |slot| crate::sim::state::OrbLink {
        group: 1,
        slot,
        of: 2,
        width: Fx::px(40),
        damage: 10,
        rehit_ticks: 30,
        last_hit: None,
    };
    let end = |dx, k| {
        let at = Point {
            x: Fx(mid.x.0 + dx),
            y: mid.y,
        };
        orb(at, FOE, Some(link(k)))
    };
    s.orbs = vec![end(-Fx::px(100).0, 0), end(Fx::px(100).0, 1)];
    s.next_orb_group = 2;
    s.tick = 100;
    let one = over(&s, 2, &w, FOE, 1);
    let ev = pass(&w, &mut s, &[one]);
    assert_eq!(s.players[slot(1)].hp, 0, "the beam downs player 1");
    assert_eq!(downed(&ev), 1);
    assert_eq!(s.runs.len(), 1);
    assert_eq!(hit_ids(&s, 0), vec![slot(2).id()]);
}

/// A single player, with a run of its own at place 0 and one foe run after it.
fn solo() -> (World, State) {
    let e = worst_case::warmed_up();
    let Run::V2 { world, state } = e.run() else {
        panic!("the live rules");
    };
    let mut s = state.clone();
    assert_eq!(s.players.len(), 1);
    let template = s.runs[0].clone();
    s.runs.clear();
    let p = &mut s.players[PlayerIdx::SOLO];
    p.hp = p.max_hp;
    p.invulnerable = 0;
    s.companions.clear();
    let me = slot(0).id();
    add_run(&mut s, &template, me, crate::sim::player::COMBO_SLOT);
    (World::clone(world), s)
}

#[test]
fn a_hold_broken_by_the_swing_leaves_the_swings_their_runs() {
    let (mut w, mut s) = solo();
    // A second hit on the same player in the pass: the swing after the break is judged
    // against its own run.
    w.tuning.hit_invulnerable_ticks = 0;
    let template = s.runs[0].clone();
    add_run(&mut s, &template, FOE, 0);
    add_run(&mut s, &template, EntityId(901), 0);
    s.players[PlayerIdx::SOLO].combo.hold = Some(3);
    let mut first = over(&s, 0, &w, FOE, 1);
    first.breaks_charge = true;
    let second = over(&s, 0, &w, EntityId(901), 2);
    let ev = pass(&w, &mut s, &[first, second]);
    assert_eq!(s.players[PlayerIdx::SOLO].combo.hold, None);
    assert!(ev.iter().any(|e| matches!(e, DomainEvent::HoldBroken)));
    assert_eq!(s.runs.len(), 2, "the held step's run ended after the pass");
    assert!(s.runs.iter().all(|r| r.owner != slot(0).id()));
    assert_eq!(hit_ids(&s, 0), vec![slot(0).id()]);
    assert_eq!(hit_ids(&s, 1), vec![slot(0).id()]);
}

#[test]
fn a_stun_leaves_the_later_swing_its_run() {
    let (w, mut s) = solo();
    let template = s.runs[0].clone();
    let x = (0..s.enemies.len())
        .find(|&i| w.defs.agent(s.enemies[i].def).elite.is_none())
        .expect("a regular enemy");
    s.enemies[x].hp = u32::MAX / 2;
    s.enemies[x].stun_immune = 0;
    let xid = s.enemies[x].id;
    add_run(&mut s, &template, xid, 0);
    add_run(&mut s, &template, FOE, 0);
    let me = slot(0).id();
    let mut stun = at_place(swing(s.enemies[x].at, 1_000, true, me), 0, 0);
    stun.combo = true;
    stun.smash = Some(Smash::Stun { ticks: 30 });
    let foe = over(&s, 0, &w, FOE, 2);
    pass(&w, &mut s, &[stun, foe]);
    assert!(
        s.runs.iter().all(|r| r.owner != xid),
        "the stunned enemy's run ended after the pass"
    );
    let foe_run = s.runs.iter().find(|r| r.owner == FOE).expect("its run");
    assert_eq!(
        foe_run.hits.iter().map(|h| h.0).collect::<Vec<_>>(),
        vec![me]
    );
}

#[test]
fn a_pass_with_no_run_end_changes_no_run() {
    let (w, mut s) = downing(1);
    s.players[slot(1)].hp = s.players[slot(1)].max_hp;
    let one = over(&s, 1, &w, FOE, 1);
    let ev = pass(&w, &mut s, &[one]);
    assert_eq!(downed(&ev), 0);
    assert_eq!(s.runs.len(), 2);
    assert_eq!(hit_ids(&s, 1), vec![slot(1).id(), slot(2).id()]);
    assert!(hit_ids(&s, 0).is_empty());
}

#[test]
fn the_ends_are_applied_once_and_leave_the_scratch_clean() {
    let (w, mut s) = downing(1);
    let owner = slot(1).id();
    s.move_index.lower(owner, 50);
    // A list with a room only this run's hit list has, so the pool shows it came back.
    s.runs[0].hits = Vec::with_capacity(5);
    s.runs[0].hits.push((slot(2).id(), 0));
    let one = over(&s, 1, &w, FOE, 1);
    pass(&w, &mut s, &[one]);
    assert_eq!(s.runs.len(), 1);
    assert_eq!(
        s.move_index.get(owner),
        100,
        "the index is rebuilt without it"
    );
    assert_eq!(s.scratch.ends_pinned, None);
    assert!(s.scratch.ends_pending.is_empty());
    assert!(
        s.scratch.hit_lists.iter().any(|l| l.capacity() == 5),
        "its hit list is kept"
    );
}

#[test]
fn a_run_started_in_the_pass_survives_an_end_for_its_owner_and_slot() {
    let (w, mut s) = downing(1);
    let template = s.runs[1].clone();
    let owner = slot(1).id();
    s.scratch.ends_pinned = Some(s.runs.len());
    // The run that existed ends; the one the pass started afterwards, same owner and slot,
    // stays.
    s.scratch
        .ends_pending
        .push(RunEnd::Slot(owner, crate::sim::player::COMBO_SLOT));
    add_run(&mut s, &template, owner, crate::sim::player::COMBO_SLOT);
    apply_ends(&w, &mut s);
    assert_eq!(s.runs.len(), 2);
    assert_eq!(s.runs[0].owner, FOE);
    assert_eq!(s.runs[1].owner, owner);
}

#[test]
fn two_pending_ends_both_apply() {
    let (w, mut s) = downing(2);
    s.scratch.ends_pinned = Some(s.runs.len());
    s.scratch
        .ends_pending
        .push(RunEnd::Slot(slot(1).id(), crate::sim::player::COMBO_SLOT));
    s.scratch.ends_pending.push(RunEnd::All(EntityId(900)));
    apply_ends(&w, &mut s);
    assert_eq!(s.runs.len(), 1);
    assert_eq!(s.runs[0].owner, EntityId(901));
    assert_eq!(s.scratch.ends_pinned, None);
}

#[test]
fn the_same_end_asked_twice_is_kept_once() {
    let (_, mut s) = downing(1);
    let end = RunEnd::All(FOE);
    s.scratch.push_end(end);
    s.scratch.push_end(end);
    s.scratch.push_end(RunEnd::All(EntityId(901)));
    assert_eq!(s.scratch.ends_pending.len(), 2);
}

mod sort_tests {
    use super::*;

    /// The order of the stable sort: tied swings stay in `active_swings` order.
    fn stable_order(swings: &[Swing]) -> Vec<(u32, u8, usize)> {
        let mut keys: Vec<_> = swings
            .iter()
            .map(|w| (w.owner.0, w.slot, w.place()))
            .collect();
        keys.sort_by_key(|k| (k.0, k.1));
        keys
    }

    fn keys(swings: &[Swing]) -> Vec<(u32, u8, usize)> {
        swings
            .iter()
            .map(|w| (w.owner.0, w.slot, w.place()))
            .collect()
    }

    fn made(owner: u32, slot: u8, place: usize) -> Swing {
        at_place(
            swing(Point::default(), 1, false, EntityId(owner)),
            place,
            slot,
        )
    }

    /// A fixed pseudo-random order (an LCG; the tests draw no `Rng`).
    fn shuffled(n: usize, owners: u32, slots: u8, unique: bool) -> Vec<Swing> {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move || {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (x >> 33) as u32
        };
        let mut all: Vec<(u32, u8)> = Vec::new();
        for i in 0..n {
            let i = u32::try_from(i).expect("a small count");
            all.push(if unique {
                (
                    900 + i / u32::from(slots),
                    u8::try_from(i % u32::from(slots)).expect("a slot"),
                )
            } else {
                let (o, sl) = (next() % owners, next() % u32::from(slots));
                (900 + o, u8::try_from(sl).expect("a slot"))
            });
        }
        for i in (1..all.len()).rev() {
            all.swap(i, usize::try_from(next()).expect("a usize") % (i + 1));
        }
        all.into_iter()
            .enumerate()
            .map(|(place, (o, sl))| made(o, sl, place))
            .collect()
    }

    #[test]
    fn unique_keys_sort_as_the_stable_sort_does() {
        for n in [0, 1, 20, 21, 260] {
            let mut w = shuffled(n, 0, 8, true);
            let want = stable_order(&w);
            sort_swings(&mut w);
            assert_eq!(keys(&w), want, "{n} swings");
        }
    }

    #[test]
    fn tied_keys_keep_their_places_in_order() {
        let mut w = shuffled(60, 5, 3, false);
        let mut pairs: Vec<_> = w.iter().map(|x| (x.owner.0, x.slot)).collect();
        pairs.sort();
        pairs.dedup();
        assert!(pairs.len() < w.len(), "the keys tie");
        let want = stable_order(&w);
        sort_swings(&mut w);
        assert_eq!(keys(&w), want);
    }

    #[test]
    fn tied_swings_in_any_input_order_come_out_in_place_order() {
        let mut w = shuffled(60, 5, 3, false);
        w.reverse();
        w.rotate_left(7);
        sort_swings(&mut w);
        let k = keys(&w);
        assert!(
            k.windows(2)
                .all(|p| (p[0].0, p[0].1, p[0].2) < (p[1].0, p[1].1, p[1].2))
        );
    }

    #[test]
    fn sorted_and_reversed_inputs_sort_as_the_stable_sort_does() {
        let mut w = shuffled(260, 0, 8, true);
        w.sort_by_key(|w| (w.owner.0, w.slot));
        let want = stable_order(&w);
        sort_swings(&mut w);
        assert_eq!(keys(&w), want);
        w.reverse();
        let want = stable_order(&w);
        sort_swings(&mut w);
        assert_eq!(keys(&w), want);
    }
}
