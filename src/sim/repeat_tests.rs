//! The step repeat and the proc factor over a state (ADR-SIM-9 point 3, ADR-SIM-7): the deal of a
//! queued repeat through the damage pipeline, what a repeat is not, the drop of an entry whose
//! owner is gone, the factor's draw on the attack stream and the queue's memory. The queue's
//! own timing, over whole runs, is in `tests/repeat.rs`.

use super::state::{AttackIndex, PlayerIdx, Repeat, RepeatAnchor, State};
use super::{World, combat, runes};
use crate::attack::{HitBox, PhaseKind};
use crate::bench_fixtures::worst_case;
use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::inline::InlineVec;
use crate::limits::MAX_REPEATS_QUEUED;
use crate::rune::Rune;

fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// A party's world and state with the tick's other work taken away and the foes parked far apart
/// except the first `near`, which stand at the origin of the test (0, 0) in a row 8 pixels apart.
fn bare(n: u8, near: usize) -> (World, State) {
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
    s.repeats.clear();
    s.boss = None;
    for (i, e) in s.enemies.iter_mut().enumerate() {
        e.hp = 100_000;
        e.defence = 0;
        e.at = if i < near {
            Point {
                x: Fx::px(8 * i32::try_from(i).unwrap()),
                y: Fx::px(0),
            }
        } else {
            Point {
                x: Fx::px(10_000),
                y: Fx::px(10_000 + 100 * i32::try_from(i).unwrap()),
            }
        };
    }
    (World::clone(w), s)
}

/// An attack of the run's table with an active circle phase: its index, the phase and the radius.
fn circle_phase(w: &World) -> (AttackIndex, u8, i32) {
    for (i, a) in w.attacks.iter().enumerate() {
        for (j, p) in a.phases.iter().enumerate() {
            if let (PhaseKind::Active, Some(HitBox::Circle { radius, .. })) = (p.kind, &p.hit_box) {
                return (
                    AttackIndex(u16::try_from(i).unwrap()),
                    u8::try_from(j).unwrap(),
                    radius.0,
                );
            }
        }
    }
    panic!("an attack with an active circle");
}

fn entry(w: &World, owner: u8, due: u32, percent: u16, anchor: RepeatAnchor) -> Repeat {
    let (attack, phase, _) = circle_phase(w);
    Repeat {
        owner,
        due,
        percent,
        proc_pct: None,
        anchor,
        attack,
        phase,
        damage: 1_000,
        width_pct: 0,
    }
}

fn list(s: &State, enemies: &[usize]) -> RepeatAnchor {
    RepeatAnchor::HitList(
        enemies
            .iter()
            .map(|&i| s.enemies[i].id)
            .collect::<InlineVec<_, _>>(),
    )
}

fn deal(w: &World, s: &mut State) -> Vec<DomainEvent> {
    let mut ev = Vec::new();
    combat::deal_repeats(w, s, &mut ev);
    ev
}

/// `(target, damage, crit, echo, repeat, player)` of every `Hit`.
fn hits(ev: &[DomainEvent]) -> Vec<(EntityId, u32, bool, bool, bool, Option<EntityId>)> {
    ev.iter()
        .filter_map(|e| match e {
            DomainEvent::Hit {
                target,
                damage,
                crit,
                echo,
                repeat,
                player,
                ..
            } => Some((*target, *damage, *crit, *echo, *repeat, *player)),
            _ => None,
        })
        .collect()
}

/// R1 (the deal): a due entry hits each target on its list, in the order stored, for the percent
/// of its damage, as a repeat that is no crit and no echo; the entry is gone.
#[test]
fn a_due_hit_list_entry_hits_each_target_for_its_percent() {
    let (w, mut s) = bare(1, 3);
    s.tick = 50;
    let a = list(&s, &[0, 2]);
    s.repeats.push(entry(&w, 0, 50, 50, a));
    let ev = deal(&w, &mut s);
    let ids = [s.enemies[0].id, s.enemies[2].id];
    assert_eq!(
        hits(&ev),
        vec![
            (ids[0], 500, false, false, true, Some(slot(0).id())),
            (ids[1], 500, false, false, true, Some(slot(0).id())),
        ]
    );
    assert_eq!((s.enemies[0].hp, s.enemies[1].hp), (99_500, 100_000));
    assert!(s.repeats.is_empty());
}

/// R5: an entry not yet due waits in its place; two due ones deal in the order queued.
#[test]
fn entries_deal_in_the_order_queued_and_a_later_one_waits() {
    let (w, mut s) = bare(1, 3);
    s.tick = 50;
    let on = |s: &State, i| list(s, &[i]);
    let (a, b, c) = (on(&s, 2), on(&s, 1), on(&s, 0));
    s.repeats.push(entry(&w, 0, 50, 100, a));
    s.repeats.push(entry(&w, 0, 80, 100, b));
    s.repeats.push(entry(&w, 0, 40, 100, c));
    let ev = deal(&w, &mut s);
    let order: Vec<_> = hits(&ev).iter().map(|h| h.0).collect();
    assert_eq!(order, vec![s.enemies[2].id, s.enemies[0].id]);
    assert_eq!(s.repeats.len(), 1);
    assert_eq!(s.repeats[0].due, 80, "the waiting entry keeps its place");
}

/// R2: a target that no longer stands takes nothing; the others still do.
#[test]
fn a_target_that_fell_takes_no_repeat() {
    let (w, mut s) = bare(1, 2);
    s.tick = 10;
    let a = list(&s, &[0, 1]);
    s.repeats.push(entry(&w, 0, 10, 100, a));
    s.enemies[0].hp = 0;
    let ev = deal(&w, &mut s);
    assert_eq!(hits(&ev).len(), 1);
    assert_eq!(hits(&ev)[0].0, s.enemies[1].id);
}

/// R3: a `place` entry hits the foes its hit box covers where it was placed, and only them; the
/// range it carries widens the box.
#[test]
fn a_place_entry_hits_what_the_box_covers_and_the_range_widens_it() {
    let (w, mut s) = bare(1, 3);
    s.tick = 10;
    let (_, _, radius) = circle_phase(&w);
    let reach = Fx(radius + 4 * Fx::px(1).0);
    // The enemies stand 0, 8 and 16 px from the origin; the box is centred there. Move the third
    // to just past the box's edge: 4 px beyond its radius plus the foe's own.
    let foe_r = |s: &State, i: usize| w.defs.agent(s.enemies[i].def).radius.0;
    s.enemies[2].at = Point {
        x: Fx(reach.0 + foe_r(&s, 2)),
        y: Fx::px(0),
    };
    let at = Point::default();
    let place = RepeatAnchor::Place {
        at,
        facing: crate::sim::state::Dir::UP,
    };
    s.repeats.push(entry(&w, 0, 10, 100, place));
    let ev = deal(&w, &mut s);
    let got: Vec<_> = hits(&ev).iter().map(|h| h.0).collect();
    assert_eq!(got, vec![s.enemies[0].id, s.enemies[1].id]);
    // At 50 percent more range the third is inside.
    for e in &mut s.enemies[..3] {
        e.hp = 100_000;
    }
    let mut wide = entry(&w, 0, 10, 100, place);
    wide.width_pct = 50;
    s.repeats.push(wide);
    let ev = deal(&w, &mut s);
    assert_eq!(hits(&ev).len(), 3);
}

/// R4: a repeat is no crit and queues no echo even with both held at 100 percent, and it queues
/// no repeat.
#[test]
fn a_repeat_does_not_crit_echo_or_queue() {
    let (w, mut s) = bare(1, 1);
    s.tick = 10;
    s.players[slot(0)].crit_chance_pct = 100;
    s.players[slot(0)].echo_hit_pct = 100;
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 0, 10, 100, a));
    let ev = deal(&w, &mut s);
    assert_eq!(hits(&ev)[0].1, 1_000);
    assert!(!hits(&ev)[0].2 && s.echoes.is_empty() && s.repeats.is_empty());
}

/// R6: the slot's damage credit; an owner who is out has the entry dropped and nothing lands.
#[test]
fn the_owner_gets_the_credit_and_one_who_is_out_loses_the_entry() {
    let (w, mut s) = bare(2, 2);
    s.tick = 10;
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 1, 10, 100, a));
    let ev = deal(&w, &mut s);
    assert_eq!(hits(&ev)[0].5, Some(slot(1).id()));
    // A downed owner still deals what was queued while it was up.
    s.players[slot(1)].hp = 0;
    let a = list(&s, &[1]);
    s.repeats.push(entry(&w, 1, 10, 100, a));
    assert_eq!(hits(&deal(&w, &mut s)).len(), 1);
    s.players[slot(1)].out = true;
    let a = list(&s, &[1]);
    s.repeats.push(entry(&w, 1, 10, 100, a));
    assert!(hits(&deal(&w, &mut s)).is_empty());
    assert!(s.repeats.is_empty(), "the entry is dropped");
}

/// R10: defence lowers a repeat as it lowers any hit, a hit of at least 1 stays 1, a damage of 0
/// stays 0.
#[test]
fn the_pipeline_applies_defence_and_keeps_a_floor_of_one() {
    let (w, mut s) = bare(1, 1);
    s.tick = 10;
    s.enemies[0].defence = 3;
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 0, 10, 100, a));
    let plain = hits(&deal(&w, &mut s))[0].1;
    assert!(plain < 1_000, "defence lowers it: {plain}");
    s.enemies[0].defence = 0;
    for (damage, percent, expected) in [(1, 10, 1), (0, 100, 0)] {
        let a = list(&s, &[0]);
        let mut e = entry(&w, 0, 10, percent, a);
        e.damage = damage;
        s.repeats.push(e);
        let got = hits(&deal(&w, &mut s));
        assert_eq!(got.first().map(|h| h.1).unwrap_or(0), expected);
    }
}

/// A1: the queue keeps the memory it was given through any number of deals.
#[test]
fn dealing_keeps_the_queue_s_memory() {
    let (w, mut s) = bare(1, 1);
    s.tick = 10;
    s.repeats = Vec::with_capacity(MAX_REPEATS_QUEUED);
    for _ in 0..200 {
        let a = list(&s, &[0]);
        s.repeats.push(entry(&w, 0, 10, 1, a));
        let a = list(&s, &[0]);
        s.repeats.push(entry(&w, 0, 99, 1, a));
        deal(&w, &mut s);
        s.repeats.clear();
        assert_eq!(s.repeats.capacity(), MAX_REPEATS_QUEUED);
    }
}

/// D1: the bundled rules name no repeat and no factor, and a run holds no queue memory.
#[test]
fn the_bundled_run_is_dormant() {
    let e = worst_case::warmed_up();
    let Run::V2 { world, state, .. } = e.run() else {
        panic!("the live rules");
    };
    assert!(!world.has_repeat && !world.has_proc);
    assert_eq!(state.repeats.capacity(), 0);
}

// ---- The proc factor: `runes::on_hit(.., proc)` ----

/// A state whose first player holds every rune, all ready, and the foe it is to be fired at.
fn runes_ready() -> (World, State, EntityId, Point) {
    let (w, mut s) = bare(1, 1);
    for r in &mut s.players[slot(0)].runes {
        r.cooldown_left = 0;
    }
    let (id, at) = (s.enemies[0].id, s.enemies[0].at);
    (w, s, id, at)
}

/// The on-hit runes that would fire when ready: held, with a rule entry and an attack.
fn candidates(w: &World, s: &State) -> Vec<usize> {
    (0..s.players[slot(0)].runes.len())
        .filter(|&i| {
            let r = s.players[slot(0)].runes[i].rune;
            s.runes.on_hit(r).is_some() && w.rune_attacks[usize::from(r.index())].is_some()
        })
        .collect()
}

fn fired(s: &State) -> Vec<usize> {
    (0..s.players[slot(0)].runes.len())
        .filter(|&i| s.players[slot(0)].runes[i].cooldown_left > 0)
        .collect()
}

/// How many `below(100)` draws the attack stream has taken since `before`.
fn draws_since(before: &crate::rng::SplitMix64, s: &State) -> usize {
    (0..=64)
        .find(|&n| {
            let mut r = before.clone();
            for _ in 0..n {
                r.below(100);
            }
            r == s.rng.attack
        })
        .expect("a count of draws that matches the stream")
}

fn fire(w: &World, s: &mut State, id: EntityId, at: Point, proc: Option<u16>) {
    let mut ev = Vec::new();
    runes::on_hit(w, s, slot(0), id, at, &mut ev, proc);
}

/// P1: no factor, no draw: every ready rune fires and the stream is untouched.
#[test]
fn with_no_factor_every_ready_rune_fires_and_no_draw_is_taken() {
    let (w, mut s, id, at) = runes_ready();
    let before = s.rng.attack.clone();
    fire(&w, &mut s, id, at, None);
    assert!(fired(&s).len() >= 2, "{:?}", fired(&s));
    assert_eq!(s.rng.attack, before);
}

/// P2 and P3: 0 never fires a rune and leaves it ready; 100 fires every one; each ready check
/// takes exactly one draw.
#[test]
fn a_factor_of_0_fires_nothing_and_of_100_fires_all_each_check_one_draw() {
    let (w, mut s, id, at) = runes_ready();
    let all = {
        let mut t = s.clone();
        fire(&w, &mut t, id, at, None);
        fired(&t)
    };
    let checks = candidates(&w, &s).len();
    let before = s.rng.attack.clone();
    let mut zero = s.clone();
    fire(&w, &mut zero, id, at, Some(0));
    assert!(fired(&zero).is_empty(), "the runes stay ready");
    assert_eq!(draws_since(&before, &zero), checks);
    fire(&w, &mut s, id, at, Some(100));
    assert_eq!(fired(&s), all);
    assert_eq!(draws_since(&before, &s), checks);
}

/// P4: the fire and no-fire sequence is the replay of the stream's draws against the factor; a
/// rune that did not fire stays ready for the next hit.
#[test]
fn a_factor_fires_by_the_draw_and_a_rune_that_did_not_stays_ready() {
    let (w, s0, id, at) = runes_ready();
    let all = {
        let mut t = s0.clone();
        fire(&w, &mut t, id, at, None);
        fired(&t)
    };
    for seed in 0..8u64 {
        let mut s = s0.clone();
        s.rng.attack = crate::rng::derive(seed, crate::rng::TAG_ATTACK);
        let mut replay = s.rng.attack.clone();
        let expected: Vec<usize> = candidates(&w, &s)
            .into_iter()
            .filter(|&i| replay.below(100) < 50 && all.contains(&i))
            .collect();
        // `replay` drew once per candidate, fired or not, as the rule does.
        fire(&w, &mut s, id, at, Some(50));
        assert_eq!(fired(&s), expected, "seed {seed}");
    }
}

/// P5: a repeat's own factor decides its runes: `Some(0)` fires none and draws; none fires every
/// ready rune and draws nothing.
#[test]
fn a_repeat_uses_its_own_factor() {
    let run = |proc_pct: Option<u16>| {
        let (w, mut s, ..) = runes_ready();
        s.tick = 10;
        let before = s.rng.attack.clone();
        let a = list(&s, &[0]);
        let mut e = entry(&w, 0, 10, 100, a);
        e.proc_pct = proc_pct;
        s.repeats.push(e);
        deal(&w, &mut s);
        (fired(&s), draws_since(&before, &s))
    };
    let (none, drawn) = run(None);
    assert!(none.len() >= 2 && drawn == 0);
    let (zero, drawn) = run(Some(0));
    assert!(zero.is_empty() && drawn >= 2);
}

/// P6: an echo draws nothing.
#[test]
fn an_echo_takes_no_draw() {
    let (w, mut s, ..) = runes_ready();
    let before = s.rng.attack.clone();
    s.echoes.push(crate::sim::state::Echo {
        source: slot(0).id(),
        target: s.enemies[0].id,
        damage: 10,
        damage_type: None,
    });
    let mut ev = Vec::new();
    combat::deal_echoes(&w, &mut s, &mut ev);
    assert_eq!(s.rng.attack, before);
}

/// P7: two players' repeats draw in the order of the queue, slot 0 then slot 1, and two runs from
/// the same state draw alike.
#[test]
fn two_owners_draw_in_the_queue_order() {
    let (w, mut s) = bare(2, 2);
    s.tick = 10;
    for p in 0..2 {
        for r in &mut s.players[slot(p)].runes {
            r.cooldown_left = 0;
        }
    }
    for (owner, foe) in [(0u8, 0usize), (1, 1)] {
        let a = list(&s, &[foe]);
        let mut e = entry(&w, owner, 10, 100, a);
        e.proc_pct = Some(50);
        s.repeats.push(e);
    }
    let before = s.rng.attack.clone();
    let mut twin = s.clone();
    deal(&w, &mut s);
    deal(&w, &mut twin);
    assert_eq!(s.rng.attack, twin.rng.attack);
    assert_eq!(super::state_hash(&s), super::state_hash(&twin));
    // One draw per ready check of each owner's runes, the two owners one after the other.
    assert_eq!(draws_since(&before, &s), 2 * candidates(&w, &s).len());
}
