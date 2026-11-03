//! Pins for where a player's damage meets a foe's hit points: the numbers, the events and the
//! credit as the rules stand before the landing is one function (ADR-SIM-14).

use super::entity::Who;
use super::state::{PlayerIdx, State};
use super::{Shot, World};
use crate::bench_fixtures::worst_case;
use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::id::EntityId;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// A party's world and state with the tick's other work taken away.
fn bare(n: u8) -> (World, State) {
    let e: Engine = worst_case::warmed_up().into_party(n);
    let (w, s) = match e.run() {
        Run::V2 { world, state, .. } => (world, state),
        Run::V1 { .. } => panic!("the live rules"),
    };
    let mut s = s.clone();
    s.runs.clear();
    s.orbs.clear();
    s.zones.clear();
    s.echoes.clear();
    (w.clone(), s)
}

fn shot(s: &State, owner: EntityId, at: usize, damage: u32) -> Shot {
    Shot {
        at: s.enemies[at].at,
        owner,
        friendly: true,
        damage,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        rune: None,
        chain: false,
    }
}

fn hit(w: &World, s: &mut State, me: u8, at: usize, damage: u32) -> Vec<DomainEvent> {
    let mut ev = Vec::new();
    let b = shot(s, slot(me).id(), at, damage);
    super::damage(w, s, slot(me), b, Who::Enemy(at), &mut ev);
    ev
}

fn hits(ev: &[DomainEvent]) -> Vec<(u32, bool, bool, Option<EntityId>, Option<EntityId>)> {
    ev.iter()
        .filter_map(|e| match e {
            DomainEvent::Hit {
                damage,
                crit,
                echo,
                player,
                buffed_by,
                ..
            } => Some((*damage, *crit, *echo, *player, *buffed_by)),
            _ => None,
        })
        .collect()
}

fn deaths(ev: &[DomainEvent]) -> Vec<Option<EntityId>> {
    ev.iter()
        .filter_map(|e| match e {
            DomainEvent::Died { killer, .. } => Some(*killer),
            _ => None,
        })
        .collect()
}

#[test]
fn a_hit_lowers_hit_points_reports_and_raises_aggro() {
    let (w, mut s) = bare(1);
    s.enemies[0].hp = 1000;
    s.enemies[0].defence = 0;
    let aggro = s.aggro(Who::Player(slot(0)));
    let ev = hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.enemies[0].hp, 900);
    assert_eq!(
        hits(&ev),
        vec![(100, false, false, Some(slot(0).id()), None)]
    );
    assert!(deaths(&ev).is_empty());
    let gained = 100 * w.tuning.aggro_per_damage;
    assert_eq!(s.aggro(Who::Player(slot(0))), aggro + gained);
}

#[test]
fn a_killing_hit_reports_its_full_damage_and_credits_the_hitter() {
    let (w, mut s) = bare(3);
    s.players[slot(0)].echo_hit_pct = 50;
    s.enemies[0].hp = 900;
    s.enemies[0].defence = 0;
    let ev = hit(&w, &mut s, 1, 0, 1000);
    assert_eq!(s.enemies[0].hp, 0);
    assert_eq!(hits(&ev)[0].0, 1000, "not clamped to the hit points left");
    assert_eq!(deaths(&ev), vec![Some(slot(1).id())]);
    assert_eq!(super::killer_of(&s, 0), slot(1));
    assert!(s.echoes.is_empty(), "a kill queues no echo");
}

#[test]
fn a_hit_that_leaves_the_foe_standing_queues_one_echo() {
    let (w, mut s) = bare(1);
    s.players[slot(0)].echo_hit_pct = 50;
    s.enemies[0].hp = 1000;
    s.enemies[0].defence = 0;
    hit(&w, &mut s, 0, 0, 100);
    assert_eq!(s.echoes.len(), 1);
    assert_eq!(s.echoes[0].damage, 50);
}

#[test]
fn defence_reduces_the_damage_that_lands() {
    let (w, mut s) = bare(1);
    s.enemies[0].hp = 1000;
    s.enemies[0].defence = w.tuning.defence_k as u32;
    let ev = hit(&w, &mut s, 0, 0, 100);
    assert_eq!(hits(&ev)[0].0, 50);
    assert_eq!(s.enemies[0].hp, 950);
}

#[test]
fn a_hit_on_the_boss_lowers_its_hit_points() {
    let (w, mut s) = bare(1);
    let Some(b) = s.boss.as_mut() else {
        return; // the fixture has no boss at this tick: the enemy cases above cover the write
    };
    b.hp = 10_000;
    b.defence = 0;
    let mut ev = Vec::new();
    let sh = shot(&s, slot(0).id(), 0, 100);
    super::damage(&w, &mut s, slot(0), sh, Who::Boss, &mut ev);
    assert_eq!(s.boss.as_ref().map(|b| b.hp), Some(9_900));
}

#[test]
fn an_echo_lands_as_an_echo_without_a_buffer_and_credits_its_kill() {
    let (w, mut s) = bare(3);
    s.enemies[0].hp = 7;
    let id = s.enemies[0].id;
    let aggro = s.aggro(Who::Player(slot(1)));
    s.echoes.push(super::state::Echo {
        source: slot(1).id(),
        target: id,
        damage: 7,
        damage_type: None,
    });
    let mut ev = Vec::new();
    super::combat::deal_echoes(&w, &mut s, &mut ev);
    assert_eq!(s.enemies[0].hp, 0);
    assert_eq!(hits(&ev), vec![(7, false, true, Some(slot(1).id()), None)]);
    assert_eq!(deaths(&ev), vec![Some(slot(1).id())]);
    assert_eq!(super::killer_of(&s, 0), slot(1));
    assert!(s.echoes.is_empty());
    assert!(s.aggro(Who::Player(slot(1))) >= aggro);
}

#[test]
fn an_echo_on_a_standing_target_lowers_it_and_one_at_zero_is_skipped() {
    let (w, mut s) = bare(1);
    s.enemies[0].hp = 100;
    s.enemies[1].hp = 0;
    let (a, b) = (s.enemies[0].id, s.enemies[1].id);
    for (target, damage) in [(a, 7), (b, 9)] {
        s.echoes.push(super::state::Echo {
            source: slot(0).id(),
            target,
            damage,
            damage_type: None,
        });
    }
    let mut ev = Vec::new();
    super::combat::deal_echoes(&w, &mut s, &mut ev);
    assert_eq!(s.enemies[0].hp, 93);
    assert_eq!(hits(&ev).len(), 1);
    assert!(s.echoes.is_empty());
}

/// A poison stack on enemy 0 that is due on the next tick: 3 stacks of 10.
fn poisoned(s: &mut State, by: u8) {
    use super::state::{DotState, StatusKind, StatusState};
    while !s.enemies[0].statuses.is_empty() {
        s.enemies[0].statuses.pop();
    }
    s.enemies[0].statuses.push(StatusState {
        kind: StatusKind::Poison,
        source: slot(by).id(),
        remaining: 10,
        total: 10,
        stacks: 3,
        value_pct: 0,
        dot: Some(DotState {
            damage: 10,
            damage_type: None,
            atk_up_pct: 0,
            type_bonus_pct: 0,
            interval: 1,
            next_in: 1,
        }),
    });
}

#[test]
fn a_damage_over_time_tick_lands_as_a_runed_hit_of_its_source() {
    let (w, mut s) = bare(3);
    s.enemies[0].hp = 1000;
    s.enemies[0].defence = 0;
    poisoned(&mut s, 2);
    let mut ev = Vec::new();
    super::entity::tick_dots(&w, &mut s, &mut ev);
    let ev = only(ev, s.enemies[0].id);
    assert_eq!(s.enemies[0].hp, 970);
    assert_eq!(
        hits(&ev),
        vec![(30, false, false, Some(slot(2).id()), None)]
    );
    assert!(ev.iter().any(|e| matches!(
        e,
        DomainEvent::Hit {
            rune: Some(crate::rune::Rune::Poison),
            ..
        }
    )));
}

#[test]
fn a_damage_over_time_kill_is_credited_to_the_stack_owner() {
    let (w, mut s) = bare(3);
    s.enemies[0].hp = 20;
    s.enemies[0].defence = 0;
    poisoned(&mut s, 1);
    let mut ev = Vec::new();
    super::entity::tick_dots(&w, &mut s, &mut ev);
    let ev = only(ev, s.enemies[0].id);
    assert_eq!(s.enemies[0].hp, 0);
    assert_eq!(deaths(&ev), vec![Some(slot(1).id())]);
    assert_eq!(super::killer_of(&s, 0), slot(1));
}

#[test]
fn a_hit_by_a_companion_or_an_unknown_source_is_slot_zeros() {
    let (w, mut s) = bare(2);
    s.enemies[0].hp = 1000;
    let mut ev = Vec::new();
    let b = shot(&s, EntityId(999), 0, 5);
    super::damage(&w, &mut s, slot(0), b, Who::Enemy(0), &mut ev);
    assert_eq!(hits(&ev)[0].3, Some(slot(0).id()));
}

#[test]
fn a_kill_burst_lands_a_plain_hit_without_an_echo() {
    let (w, mut s) = bare(1);
    s.players[slot(0)].burst_pct = 5;
    s.players[slot(0)].burst_radius = 100_000;
    s.players[slot(0)].echo_hit_pct = 50;
    let (a, b) = (s.enemies[0].at, s.enemies[1].at);
    s.enemies[1].at = a;
    let _ = b;
    s.enemies[0].hp = 0;
    s.enemies[0].max_hp = 1000;
    s.enemies[0].defence = 0;
    s.enemies[1].hp = 500;
    s.enemies[1].defence = 0;
    let mut ev = Vec::new();
    let mut n = 0;
    super::combat::bursts(&w, &mut s, slot(0), &mut n, &mut ev);
    let landed = hits(&only(ev, s.enemies[1].id));
    assert!(!landed.is_empty(), "the burst hurt a neighbour");
    assert!(landed.iter().all(|h| !h.1 && !h.2));
    assert_eq!(landed[0].0, 50);
    assert!(s.echoes.is_empty());
}

/// The events that name `id` as their target.
fn only(ev: Vec<DomainEvent>, id: EntityId) -> Vec<DomainEvent> {
    ev.into_iter()
        .filter(|e| match e {
            DomainEvent::Hit { target, .. } | DomainEvent::Died { entity: target, .. } => {
                *target == id
            }
            _ => false,
        })
        .collect()
}
