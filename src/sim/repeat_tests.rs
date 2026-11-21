//! The step repeat and the proc factor over a state (ADR-SIM-9 point 3, ADR-SIM-7): the deal of a
//! queued repeat through the damage pipeline, what a repeat is not, the drop of an entry whose
//! owner is gone, the factor's draw on the attack stream and the queue's memory. The queue's
//! own timing, over whole runs, is in `tests/repeat.rs`.

use super::AttackIndex;
use super::stacking_tests::take;
use super::state::{OrbState, PlayerIdx, Repeat, RepeatAnchor, State};
use super::trigger_tests::rig_levelled;
use super::{World, combat, runes};
use crate::attack::{HitBox, PhaseKind};
use crate::bench_fixtures::{combat_case, worst_case};
use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::fixed::{Fx, Point};
use crate::id::EntityId;
use crate::inline::InlineVec;
use crate::limits::MAX_REPEATS_QUEUED;
use serde_json::{Value, json};

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
        bleed: 0,
        stack_given: 0,
        queued: 0,
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
    assert!(!world.has_repeat && !world.has_proc && !world.has_repeat_break);
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

// ---- The orb-breaking kind: `combat::deal_repeat` breaks the orbs round what a repeat strikes ----

fn repeat_on_main() -> Value {
    json!({ "type": "repeat", "step": ["main"], "after_ticks": 10, "repeat_pct": 50,
        "anchor": "hit_list" })
}

/// A three-player Hammer run whose style 0 names a repeat on `[main]` and, at its first upgrade,
/// `repeat_breaks_orbs {chance, radius}`; player 0 has taken `places` places. Two foes: foe 0 at
/// (0, 0) and foe 1 far away; no run, orb, zone, echo or queued repeat.
fn broken(chance: u32, radius: u32, places: usize) -> (World, State) {
    let kind = json!({ "type": "repeat_breaks_orbs", "chance_pct": chance, "radius_px": radius });
    let mut r = rig_levelled(3, vec![repeat_on_main()], [vec![kind], vec![], vec![]], 2);
    take(&mut r.e, 0, places);
    let Run::V2 { world, state, .. } = r.e.run() else {
        panic!("the live rules");
    };
    let mut s = state.clone();
    s.runs.clear();
    s.orbs.clear();
    s.zones.clear();
    s.echoes.clear();
    s.repeats.clear();
    s.boss = None;
    s.enemies[0].at = Point::default();
    s.enemies[1].at = Point {
        x: Fx::px(10_000),
        y: Fx::px(10_000),
    };
    (World::clone(world), s)
}

/// A breakable orb `dist` fixed-point units east of the origin, fired by `owner`.
fn orb(dist: i32, owner: u32, breakable: bool) -> OrbState {
    OrbState {
        at: Point {
            x: Fx(dist),
            y: Fx(0),
        },
        vx: Fx(0),
        vy: Fx(0),
        dir: super::state::Dir::UP,
        style: 0,
        owner: EntityId(owner),
        damage: 1,
        damage_type: None,
        breaks_charge: false,
        smash: None,
        combo: false,
        hook: None,
        breakable,
        life: 100,
        stop: None,
        link: None,
        program: super::world::ProgramIndex(0),
    }
}

/// `dist_px` pixels plus the orb's own radius: an orb touching a circle of `dist_px`.
fn touching(w: &World, dist_px: i32) -> i32 {
    Fx::px(dist_px).0 + w.tuning.orb_radius
}

fn orbs_broken(ev: &[DomainEvent]) -> Vec<(EntityId, Point, Option<PlayerIdx>)> {
    ev.iter()
        .filter_map(|e| match e {
            DomainEvent::OrbBroken { owner, at, player } => Some((*owner, *at, *player)),
            _ => None,
        })
        .collect()
}

fn owners(s: &State) -> Vec<u32> {
    s.orbs.iter().map(|o| o.owner.0).collect()
}

/// B1 (chance 100, radius 96): the orbs inside the circle of the struck foe, the touching one
/// included, break, reported in orb order before the entry's hits; the outside one and the
/// unbreakable one stay; one draw per candidate, taken at 100 too.
#[test]
fn a_closing_cut_breaks_the_orbs_round_its_target() {
    let (w, mut s) = broken(100, 96, 1);
    s.tick = 50;
    let cases = [
        (Fx::px(50).0, 1, true),
        (touching(&w, 96), 2, true),
        (Fx::px(100).0 + 2 * w.tuning.orb_radius, 3, true),
        (Fx::px(10).0, 4, false),
    ];
    for (d, o, b) in cases {
        s.orbs.push(orb(d, o, b));
    }
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 0, 50, 50, a));
    let before = s.rng.attack.clone();
    let ev = deal(&w, &mut s);
    assert_eq!(owners(&s), vec![3, 4]);
    let me = Some(slot(0));
    assert_eq!(
        orbs_broken(&ev),
        vec![
            (
                EntityId(1),
                Point {
                    x: Fx::px(50),
                    y: Fx(0)
                },
                me
            ),
            (
                EntityId(2),
                Point {
                    x: Fx(touching(&w, 96)),
                    y: Fx(0)
                },
                me
            ),
        ]
    );
    let first_hit = ev.iter().position(|e| matches!(e, DomainEvent::Hit { .. }));
    let last_broken = ev
        .iter()
        .rposition(|e| matches!(e, DomainEvent::OrbBroken { .. }));
    assert!(last_broken < first_hit, "the orbs break before the hits");
    assert_eq!(draws_since(&before, &s), 2);
}

/// B2: before the kind's place is taken (0 places) the grant is none: nothing breaks, the target
/// still takes its hit; at 1, 2 and 3 places the orb breaks.
#[test]
fn the_cut_breaks_nothing_until_the_place_is_taken() {
    for places in 0..=3 {
        let (w, mut s) = broken(100, 96, places);
        s.tick = 50;
        s.orbs.push(orb(Fx::px(50).0, 1, true));
        let a = list(&s, &[0]);
        s.repeats.push(entry(&w, 0, 50, 50, a));
        let before = s.rng.attack.clone();
        let ev = deal(&w, &mut s);
        assert_eq!(hits(&ev).len(), 1, "{places} places");
        if places == 0 {
            assert_eq!(s.orbs.len(), 1);
            assert!(orbs_broken(&ev).is_empty());
            assert_eq!(draws_since(&before, &s), 0);
        } else {
            assert!(s.orbs.is_empty(), "{places} places");
            assert_eq!(orbs_broken(&ev).len(), 1);
        }
    }
}

/// B3 (60 percent): the orbs left are exactly those whose own draw, in orb order from a clone of
/// the stream, is 60 or more; one draw per candidate, none for the outside or unbreakable orb.
#[test]
fn each_candidate_orb_takes_one_draw_against_the_chance() {
    let (w, mut s) = broken(60, 96, 1);
    s.tick = 50;
    for i in 0..5u32 {
        s.orbs.push(orb(
            Fx::px(10 + 10 * i32::try_from(i).unwrap()).0,
            10 + i,
            true,
        ));
    }
    s.orbs.push(orb(Fx::px(200).0, 20, true));
    s.orbs.push(orb(Fx::px(20).0, 21, false));
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 0, 50, 50, a));
    let before = s.rng.attack.clone();
    let mut probe = before.clone();
    let mut left: Vec<u32> = Vec::new();
    for o in 10..15 {
        if probe.below(100) >= 60 {
            left.push(o);
        }
    }
    left.extend([20, 21]);
    deal(&w, &mut s);
    assert_eq!(owners(&s), left);
    assert_eq!(draws_since(&before, &s), 5);
}

/// B4: an orb inside two circles takes one draw; a foe that fell breaks nothing round it.
#[test]
fn an_orb_in_two_circles_takes_one_draw_and_a_fallen_foe_breaks_nothing() {
    let (w, mut s) = broken(60, 96, 1);
    s.tick = 50;
    s.enemies[1].at = Point {
        x: Fx::px(150),
        y: Fx(0),
    };
    s.orbs.push(orb(Fx::px(75).0, 1, true));
    let a = list(&s, &[0, 1]);
    s.repeats.push(entry(&w, 0, 50, 50, a));
    let before = s.rng.attack.clone();
    deal(&w, &mut s);
    assert_eq!(draws_since(&before, &s), 1);
    // The second foe has fallen: only the first circle counts.
    let (w, mut s) = broken(100, 96, 1);
    s.tick = 50;
    s.enemies[0].hp = 0;
    s.orbs.push(orb(Fx::px(50).0, 1, true));
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 0, 50, 50, a));
    let before = s.rng.attack.clone();
    deal(&w, &mut s);
    assert_eq!(owners(&s), vec![1]);
    assert_eq!(draws_since(&before, &s), 0);
}

/// B5: a `place` anchor breaks the orbs round the place.
#[test]
fn a_place_repeat_breaks_the_orbs_round_the_place() {
    let (w, mut s) = broken(100, 96, 1);
    s.tick = 50;
    s.orbs.push(orb(Fx::px(50).0, 1, true));
    s.orbs
        .push(orb(Fx::px(100).0 + 2 * w.tuning.orb_radius, 2, true));
    let a = RepeatAnchor::Place {
        at: Point::default(),
        facing: super::state::Dir::UP,
    };
    s.repeats.push(entry(&w, 0, 50, 50, a));
    deal(&w, &mut s);
    assert_eq!(owners(&s), vec![2]);
}

/// B6: each owner reads its own grant; an entry whose owner is out is dropped and breaks nothing.
#[test]
fn each_owner_reads_its_own_grant() {
    let (w, mut s) = broken(100, 96, 1);
    s.tick = 50;
    // Player 1 has taken no place of its path.
    s.players
        .career_mut(slot(1))
        .expertise
        .as_mut()
        .expect("a gauge")
        .path
        .clear();
    s.orbs.push(orb(Fx::px(50).0, 1, true));
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 1, 50, 50, a));
    deal(&w, &mut s);
    assert_eq!(s.orbs.len(), 1, "player 1 breaks nothing");
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 0, 50, 50, a));
    deal(&w, &mut s);
    assert!(s.orbs.is_empty(), "player 0 breaks");
}

/// B7: the hits of the deal and the queue after it are those of the same deal without the kind.
#[test]
fn the_kind_changes_no_hit_and_no_entry() {
    let run = |places: usize| {
        let (w, mut s) = broken(100, 96, places);
        s.tick = 50;
        s.orbs.push(orb(Fx::px(50).0, 1, true));
        let a = list(&s, &[0]);
        s.repeats.push(entry(&w, 0, 50, 50, a));
        let ev = deal(&w, &mut s);
        (hits(&ev), s.repeats.len(), s.enemies[0].hp)
    };
    assert_eq!(run(0), run(1));
}

/// C3: on the bundled `giant_sword` combat run (style `judgement_cut`) the world holds the grant:
/// none at 0 places, 60 percent and 96 px at 1, 2 and 3; the other two styles read none; and a
/// repeat dealt while `s.orbs` is empty takes no draw.
#[test]
fn the_bundled_judgement_cut_grants_the_break_from_its_first_upgrade() {
    let mut e = combat_case::warmed_up();
    let read = |e: &Engine, i: u8| {
        let Run::V2 { world, state, .. } = e.run() else {
            panic!("the live rules");
        };
        world.repeat_break_of(state, slot(i)).copied()
    };
    {
        let Run::V2 { world, .. } = e.run() else {
            panic!("the live rules");
        };
        assert!(world.has_repeat_break);
    }
    let own = super::trigger_tests::st(&mut e)
        .players
        .career_mut(slot(0))
        .expertise
        .as_ref()
        .and_then(|g| g.style)
        .expect("the style is taken");
    for places in 0..=3 {
        take(&mut e, 0, places);
        let want = (places > 0).then_some((60, Fx::px(96)));
        let got = read(&e, 0).map(|g| (g.chance_pct, g.radius));
        assert_eq!(got, want, "{places} places");
    }
    for other in (0..3u8).filter(|&i| i != own) {
        super::trigger_tests::st(&mut e)
            .players
            .career_mut(slot(0))
            .expertise
            .as_mut()
            .expect("a gauge")
            .style = Some(other);
        assert_eq!(read(&e, 0), None, "style {other}");
    }
    // With no orb on the field the deal takes no draw.
    super::trigger_tests::st(&mut e)
        .players
        .career_mut(slot(0))
        .expertise
        .as_mut()
        .expect("a gauge")
        .style = Some(own);
    take(&mut e, 0, 1);
    let Run::V2 { world, state, .. } = e.run() else {
        panic!("the live rules");
    };
    let (w, mut s) = (World::clone(world), state.clone());
    s.repeats.clear();
    s.orbs.clear();
    s.tick = 50;
    let a = list(&s, &[0]);
    s.repeats.push(entry(&w, 0, 50, 50, a));
    let before = s.rng.attack.clone();
    deal(&w, &mut s);
    assert_eq!(draws_since(&before, &s), 0);
}

/// The Sword's style 0 with one repeat on its own level, the repeat bleed at `upgrade_2` and a
/// second repeat at the capstone.
fn bleeding_style() -> (crate::weapon::Weapon, crate::expertise::Expertise) {
    use crate::expertise::Expertise;
    use crate::version::Versions;
    let bundled: Value = serde_json::from_slice(super::before::pack_bytes()).unwrap();
    let mut file = bundled["expertise"].clone();
    let w = file["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_sword")
        .unwrap();
    let repeat = |after: u32| {
        json!({ "type": "repeat", "step": ["main"], "after_ticks": after, "repeat_pct": 50,
            "anchor": "hit_list" })
    };
    let s = &mut w["styles"][0];
    s["kinds"] = json!([repeat(30)]);
    for (field, kinds) in [
        (
            "upgrade_2",
            json!([{ "type": "repeat_bleed", "stacks": 1, "duration_ticks": 180,
                "damage": 300, "max_stacks": 6 }]),
        ),
        ("capstone", json!([repeat(60)])),
    ] {
        s[field][0]["effect"] = json!("none");
        s[field][0].as_object_mut().unwrap().remove("amount");
        s[field][0]["kinds"] = kinds;
    }
    let content = super::before::pack().content();
    let base = content.weapons().tree("giant_sword", None).unwrap().clone();
    let e = Expertise::parse(file.to_string().as_bytes(), Versions::CURRENT).expect("the file");
    (base, e)
}

/// V6: the weapon copy stamps the style's repeat bleed on every repeat it holds, those a later
/// level grants included, and on none before the cell is taken.
#[test]
fn the_weapon_copy_stamps_the_repeat_bleed_on_every_repeat() {
    use crate::weapon::RepeatBleed;
    let (base, e) = bleeding_style();
    let style = &e
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_sword")
        .unwrap()
        .styles[0];
    let row = RepeatBleed {
        stacks: 1,
        duration_ticks: 180,
        damage: 300,
        max_stacks: 6,
    };
    let bleeds = |taken| -> Vec<Option<RepeatBleed>> {
        let w = crate::expertise::weapon_with_style(&base, style, taken);
        let mut out = Vec::new();
        for_each_repeat(&w, &mut |r| out.push(r.bleed));
        out
    };
    assert_eq!(bleeds(0), [None]);
    assert_eq!(bleeds(1), [None]);
    assert_eq!(bleeds(2), [Some(row)]);
    assert_eq!(bleeds(3), [Some(row), Some(row)]);
}

fn for_each_repeat(w: &crate::weapon::Weapon, f: &mut dyn FnMut(&crate::weapon::StepRepeat)) {
    use crate::event::Press;
    let mut w = w.clone();
    if let Some(step) = crate::expertise::step_mut(&mut w.combo, &[Press::Main]) {
        step.repeats.iter().for_each(&mut *f);
    }
}

// ---- The repeat bleed (a status of its own, put by a repeat that carries a bleed row). ----

use super::state::{StatusKind, StatusState};
use crate::weapon::RepeatBleed;

fn row(stacks: u8, duration_ticks: u16, damage: u32, max_stacks: u8) -> RepeatBleed {
    RepeatBleed {
        stacks,
        duration_ticks,
        damage,
        max_stacks,
    }
}

/// `bare`, with no crit and no echo on the players and one table row, and a due entry over the
/// foes `foes` that carries that row.
fn bleeding(foes: &[usize], near: usize, bleed_row: RepeatBleed) -> (World, State) {
    let (mut w, mut s) = bare(1, near);
    for p in 0..s.players.len() {
        let p = slot(u8::try_from(p).unwrap());
        s.players[p].crit_chance_pct = 0;
        s.players[p].echo_hit_pct = 0;
        // No rune: the runes' bleed would land beside the repeat's own.
        s.players[p].runes.clear();
    }
    // The warmed-up foes already carry statuses of the runes: start from none.
    for e in &mut s.enemies {
        e.statuses = Default::default();
    }
    w.repeat_bleeds = vec![bleed_row];
    s.tick = 50;
    let a = list(&s, foes);
    let mut e = entry(&w, 0, 50, 50, a);
    e.bleed = 1;
    s.repeats.push(e);
    (w, s)
}

fn status_of(s: &State, foe: usize, kind: StatusKind) -> Option<StatusState> {
    s.enemies[foe]
        .statuses
        .iter()
        .find(|st| st.kind == kind)
        .copied()
}

/// S1: each foe the repeat hits holds one repeat-bleed status with the row's own numbers, and
/// none of the runes' bleed; one `StatusApplied` each.
#[test]
fn a_repeat_with_a_bleed_row_puts_its_own_status_on_each_foe_it_hits() {
    let (w, mut s) = bleeding(&[0, 1, 2], 3, row(1, 90, 100, 3));
    let ev = deal(&w, &mut s);
    for i in 0..3 {
        let st = status_of(&s, i, StatusKind::RepeatBleed).expect("the status");
        assert_eq!((st.stacks, st.remaining, st.total), (1, 90, 90));
        let dot = st.dot.expect("a dot");
        assert_eq!((dot.damage, dot.interval, dot.next_in), (100, 20, 20));
        assert!(
            status_of(&s, i, StatusKind::Bleed).is_none(),
            "not the runes' bleed"
        );
    }
    let applied: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            DomainEvent::StatusApplied {
                target,
                status,
                stacks,
                duration_ticks,
                ..
            } => Some((
                *target,
                status.as_str().to_owned(),
                *stacks,
                *duration_ticks,
            )),
            _ => None,
        })
        .collect();
    let want: Vec<_> = (0..3)
        .map(|i| (s.enemies[i].id, "repeat_bleed".to_owned(), 1, 90))
        .collect();
    assert_eq!(applied, want);
}

/// S2 and S3: a second repeat adds stacks up to the row's cap and restarts the duration; a tick
/// deals the row's damage a stack.
#[test]
fn the_stacks_grow_to_the_cap_and_tick_for_the_rows_damage() {
    let (w, mut s) = bleeding(&[0], 1, row(2, 90, 100, 3));
    deal(&w, &mut s);
    assert_eq!(status_of(&s, 0, StatusKind::RepeatBleed).unwrap().stacks, 2);
    let mut again = s.repeats.clone();
    again.clear();
    let a = list(&s, &[0]);
    let mut e = entry(&w, 0, 50, 50, a);
    e.bleed = 1;
    s.repeats.push(e);
    deal(&w, &mut s);
    assert_eq!(
        status_of(&s, 0, StatusKind::RepeatBleed).unwrap().stacks,
        3,
        "the cap"
    );
    let before = s.enemies[0].hp;
    let mut ev = Vec::new();
    for _ in 0..20 {
        super::entity::tick_dots(&w, &mut s, &mut ev);
    }
    assert_eq!(before - s.enemies[0].hp, 100 * 3);
    for _ in 0..90 {
        super::entity::tick_dots(&w, &mut s, &mut ev);
    }
    assert!(
        status_of(&s, 0, StatusKind::RepeatBleed).is_none(),
        "it ran out"
    );
}

/// S4: a foe the repeat kills holds no status.
#[test]
fn a_foe_the_repeat_kills_bleeds_nothing() {
    let (w, mut s) = bleeding(&[0], 1, row(1, 90, 100, 3));
    s.enemies[0].hp = 1;
    let ev = deal(&w, &mut s);
    assert!(status_of(&s, 0, StatusKind::RepeatBleed).is_none());
    assert!(
        !ev.iter()
            .any(|e| matches!(e, DomainEvent::StatusApplied { .. }))
    );
}

/// S5: an entry with `bleed` 0 changes nothing.
#[test]
fn a_repeat_with_no_bleed_row_is_as_before() {
    let (w, mut s) = bleeding(&[0], 1, row(1, 90, 100, 3));
    s.repeats[0].bleed = 0;
    let ev = deal(&w, &mut s);
    assert!(status_of(&s, 0, StatusKind::RepeatBleed).is_none());
    assert!(
        !ev.iter()
            .any(|e| matches!(e, DomainEvent::StatusApplied { .. }))
    );
}

/// S6: beside the runes' bleed the status keeps its own numbers.
#[test]
fn the_repeat_bleed_lives_beside_the_runes_bleed() {
    let (w, mut s) = bleeding(&[0], 1, row(1, 90, 100, 3));
    let source = s.players.id(slot(0));
    let dot = super::state::DotState {
        damage: 300,
        damage_type: None,
        atk_up_pct: 0,
        type_bonus_pct: 0,
        interval: 20,
        next_in: 20,
    };
    let mut ev = Vec::new();
    for _ in 0..3 {
        s.add_dot(
            super::entity::Who::Enemy(0),
            StatusKind::Bleed,
            source,
            dot,
            180,
            6,
            &mut ev,
        );
    }
    deal(&w, &mut s);
    let runes = status_of(&s, 0, StatusKind::Bleed).unwrap();
    assert_eq!((runes.stacks, runes.remaining), (3, 180));
    assert_eq!(status_of(&s, 0, StatusKind::RepeatBleed).unwrap().stacks, 1);
    let before = s.enemies[0].hp;
    for _ in 0..20 {
        super::entity::tick_dots(&w, &mut s, &mut ev);
    }
    assert_eq!(before - s.enemies[0].hp, 300 * 3 + 100);
}

/// S9: a repeat-bleed tick neither crits nor echoes, with the owner at 100 percent of both.
#[test]
fn a_repeat_bleed_tick_neither_crits_nor_echoes() {
    let (w, mut s) = bleeding(&[0], 1, row(1, 90, 100, 3));
    deal(&w, &mut s);
    s.players[slot(0)].crit_chance_pct = 100;
    s.players[slot(0)].echo_hit_pct = 100;
    let before = s.enemies[0].hp;
    let mut ev = Vec::new();
    for _ in 0..20 {
        super::entity::tick_dots(&w, &mut s, &mut ev);
    }
    assert_eq!(before - s.enemies[0].hp, 100);
    assert!(s.echoes.is_empty());
}

/// Q2: the table lists each distinct row once, in the order first met; an entry's index is its
/// row's place counted from 1, and a missing row or no row is 0.
#[test]
fn the_world_indexes_the_bleed_rows() {
    let (mut w, _) = bare(1, 1);
    w.repeat_bleeds = vec![row(1, 180, 300, 6), row(2, 90, 100, 3)];
    assert_eq!(w.repeat_bleed_index(None), 0);
    assert_eq!(w.repeat_bleed_index(Some(row(1, 180, 300, 6))), 1);
    assert_eq!(w.repeat_bleed_index(Some(row(2, 90, 100, 3))), 2);
    assert_eq!(
        w.repeat_bleed_index(Some(row(3, 90, 100, 3))),
        0,
        "fails closed"
    );
}

// ---- A group `repeat` through the world (ADR-SIM-9 points 3 and 4) ----

use super::trigger_tests::{Rig, rig_from, st};

/// The bundled pack's JSON with the Hammer's style 0 naming `own` and then, at each level that
/// has kinds, those kinds; a level with none names a stat effect instead.
fn bundled_levelled(own: Vec<Value>, levels: [Vec<Value>; 3]) -> Value {
    let mut v: Value = serde_json::from_slice(crate::pack::Pack::BUNDLED).unwrap();
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
    w["styles"][0]["kinds"] = json!(own);
    for (field, kinds) in ["upgrade_1", "upgrade_2", "capstone"].iter().zip(levels) {
        let level = &mut w["styles"][0][field][0];
        if kinds.is_empty() {
            level["effect"] = json!("attack_pct");
            level["amount"] = json!(10);
            level["kinds"] = json!([]);
        } else {
            level["effect"] = json!("none");
            level.as_object_mut().unwrap().remove("amount");
            level["kinds"] = json!(kinds);
        }
    }
    v
}

fn bundled_rig(own: Vec<Value>, levels: [Vec<Value>; 3]) -> Rig {
    let bytes = serde_json::to_vec(&bundled_levelled(own, levels)).unwrap();
    rig_from(
        1,
        crate::pack::Pack::from_bytes(&bytes).expect("the pack"),
        1,
    )
}

fn place_repeat_kind(step: Value, after: u32, pct: u32, proc_pct: Option<u32>) -> Value {
    let mut k = json!({ "type": "repeat", "step": step, "after_ticks": after,
        "repeat_pct": pct, "anchor": "place" });
    if let Some(p) = proc_pct {
        k["proc_pct"] = json!(p);
    }
    k
}

/// Two `repeat`s on `normals` of the style's own level, two on AAAAA at the capstone.
fn group_levels() -> (Vec<Value>, [Vec<Value>; 3]) {
    (
        vec![
            place_repeat_kind(json!("normals"), 8, 50, Some(30)),
            place_repeat_kind(json!("normals"), 16, 40, None),
        ],
        [
            vec![],
            vec![],
            vec![
                place_repeat_kind(
                    json!(["main", "main", "main", "main", "main"]),
                    24,
                    30,
                    None,
                ),
                place_repeat_kind(
                    json!(["main", "main", "main", "main", "main"]),
                    32,
                    20,
                    None,
                ),
            ],
        ],
    )
}

/// What each press of a combo of `n` A presses queues: `(ticks until due, percent, anchor is a
/// place)` of the entries on the tick they are queued.
fn queued_by(r: &mut Rig, n: usize) -> Vec<(u32, u16, bool)> {
    for _ in 0..n - 1 {
        r.run_step(false);
    }
    st(&mut r.e).repeats.clear();
    r.press(false);
    for _ in 0..300 {
        let s = st(&mut r.e);
        if !s.repeats.is_empty() {
            let tick = s.tick;
            return s
                .repeats
                .iter()
                .map(|x| {
                    (
                        x.due - tick,
                        x.percent,
                        matches!(x.anchor, RepeatAnchor::Place { .. }),
                    )
                })
                .collect();
        }
        r.step([Default::default(); 3]);
    }
    Vec::new()
}

/// G9: a group repeat is on each of A to AAAAA and no smash, and queues each of its entries;
/// the capstone's two on AAAAA make four, the cap, on that step only.
#[test]
fn a_group_repeat_queues_on_each_normal_step_and_the_cap_counts_per_member() {
    let (own, levels) = group_levels();
    for (places, aaaaa) in [(0, 2), (3, 4)] {
        let mut r = bundled_rig(own.clone(), levels.clone());
        take(&mut r.e, 0, places);
        let Run::V2 { world, state, .. } = r.e.run() else {
            panic!("the live rules");
        };
        let counts = |presses: &[crate::event::Press]| {
            let w = world.weapon_of(state, slot(0)).expect("a weapon");
            crate::expertise::step_ref(&w.combo, presses)
                .map(|s| s.repeats.len())
                .expect("a step")
        };
        use crate::event::Press::{Main, Smash};
        assert_eq!(counts(&[Main]), 2);
        assert_eq!(counts(&[Main, Main]), 2);
        assert_eq!(counts(&[Main; 5]), aaaaa);
        assert_eq!(counts(&[Smash]), 0);
        assert_eq!(counts(&[Main, Smash]), 0);
    }
    // Driven: A queues the two entries, due 8 and 16 ticks on.
    let mut r = bundled_rig(own.clone(), levels.clone());
    take(&mut r.e, 0, 0);
    let a = queued_by(&mut r, 1);
    assert_eq!(a, [(8, 50, true), (16, 40, true)]);
    // AAAAA at the capstone queues four.
    let mut r = bundled_rig(own, levels);
    take(&mut r.e, 0, 3);
    let five = queued_by(&mut r, 5);
    assert_eq!(
        five,
        [
            (8, 50, true),
            (16, 40, true),
            (24, 30, true),
            (32, 20, true)
        ]
    );
}

/// G9: a fifth repeat on AAAAA is refused at its kind, the group's two counted on that member.
#[test]
fn a_fifth_repeat_on_a_member_is_refused_though_the_group_is_one_kind() {
    use crate::pack::{Pack, PackError};
    let (own, levels) = group_levels();
    let five = ["main"; 5];
    let [_, _, cap] = levels;
    // Two from the group and the capstone's two are four: loaded.
    let v = bundled_levelled(own.clone(), [vec![], vec![], cap.clone()]);
    assert!(Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).is_ok());
    // One more, from the second upgrade, is the fifth.
    let up_2 = vec![place_repeat_kind(json!(five), 40, 10, None)];
    let v = bundled_levelled(own, [vec![], up_2, cap]);
    let Err(PackError::File { error, .. }) = Pack::from_bytes(&serde_json::to_vec(&v).unwrap())
    else {
        panic!("the fifth repeat is refused");
    };
    let text = format!("{error:?}");
    assert!(text.contains("at most 4 repeats on one step"), "{text}");
    assert!(text.contains("AAAAA"), "{text}");
}

// ---- The riders on a style's repeats (ADR-SIM-9 points 3 and 4) ----

fn proc_rider(pct: u32) -> Value {
    json!({ "type": "repeat_proc_pct", "proc_pct": pct })
}

fn range_rider(pct: u32) -> Value {
    json!({ "type": "repeat_range_pct", "pct": pct })
}

const FIVE: [&str; 5] = ["main"; 5];

/// The Hammer's style 0 over the bundled pack's base tree, built with `taken` places.
fn copy_of(own: Vec<Value>, levels: [Vec<Value>; 3], taken: usize) -> crate::weapon::Weapon {
    use crate::expertise::{Expertise, weapon_with_style};
    use crate::version::Versions;
    let v = bundled_levelled(own, levels);
    let e = Expertise::parse(v["expertise"].to_string().as_bytes(), Versions::CURRENT)
        .expect("the expertise");
    let style = &e
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_hammer")
        .unwrap()
        .styles[0];
    let base = crate::pack::Pack::from_bytes(crate::pack::Pack::BUNDLED)
        .unwrap()
        .content()
        .weapons()
        .tree("giant_hammer", None)
        .unwrap()
        .clone();
    weapon_with_style(&base, style, taken)
}

/// What `read` gives for each repeat of the step `presses`.
fn per_repeat<T>(
    w: &crate::weapon::Weapon,
    presses: &[crate::event::Press],
    read: impl Fn(&crate::weapon::StepRepeat) -> T,
) -> Vec<T> {
    crate::expertise::step_ref(&w.combo, presses)
        .expect("a step")
        .repeats
        .iter()
        .map(read)
        .collect()
}

/// The style of the group tests: its own level's two repeats on `normals` carry 30, the capstone's
/// two on AAAAA too; `rider` sits at the level `at` (1 or 2 for an upgrade).
fn rider_style(rider: Value, at: usize) -> (Vec<Value>, [Vec<Value>; 3]) {
    let own = vec![
        place_repeat_kind(json!("normals"), 8, 50, Some(30)),
        place_repeat_kind(json!("normals"), 16, 40, Some(30)),
    ];
    let mut levels = [
        vec![],
        vec![],
        vec![
            place_repeat_kind(json!(FIVE), 24, 30, Some(30)),
            place_repeat_kind(json!(FIVE), 32, 20, Some(30)),
        ],
    ];
    levels[at - 1] = vec![rider];
    (own, levels)
}

/// P1: the proc rider stamps the repeats granted at or before its level and replaces their own
/// factor; a repeat a later level grants keeps its own.
#[test]
fn the_proc_rider_stamps_the_repeats_at_or_before_its_level() {
    use crate::event::Press::Main;
    let (own, levels) = rider_style(proc_rider(50), 2);
    let procs = |taken, presses: &[crate::event::Press]| {
        per_repeat(&copy_of(own.clone(), levels.clone(), taken), presses, |r| {
            r.proc_pct
        })
    };
    let (s30, s50) = (Some(30), Some(50));
    assert_eq!(procs(0, &[Main; 5]), [s30, s30]);
    assert_eq!(procs(1, &[Main; 5]), [s30, s30]);
    assert_eq!(procs(2, &[Main; 5]), [s50, s50]);
    assert_eq!(procs(3, &[Main; 5]), [s50, s50, s30, s30]);
    assert_eq!(procs(2, &[Main]), [s50, s50]);
    assert_eq!(procs(3, &[Main]), [s50, s50]);
}

/// P1: a repeat with no factor of its own takes the rider's, 100 and 0 included.
#[test]
fn the_proc_rider_gives_a_repeat_with_no_factor_its_own_value() {
    use crate::event::Press::Main;
    for pct in [100, 0] {
        let own = vec![
            place_repeat_kind(json!("normals"), 8, 50, None),
            proc_rider(pct),
        ];
        let w = copy_of(own, [vec![], vec![], vec![]], 0);
        let want = Some(u16::try_from(pct).unwrap());
        assert_eq!(per_repeat(&w, &[Main], |r| r.proc_pct), [want]);
    }
}

/// P2: the rider reaches the repeats of its own level whatever the order inside it.
#[test]
fn the_proc_rider_reaches_its_own_level_in_either_order() {
    use crate::event::Press::Main;
    let repeat = place_repeat_kind(json!("normals"), 8, 50, Some(30));
    for own in [
        vec![proc_rider(50), repeat.clone()],
        vec![repeat, proc_rider(50)],
    ] {
        let w = copy_of(own, [vec![], vec![], vec![]], 0);
        assert_eq!(per_repeat(&w, &[Main; 3], |r| r.proc_pct), [Some(50)]);
    }
}

/// P3: the range rider stamps every repeat of the path, those a later level grants included, and
/// none before its place is taken.
#[test]
fn the_range_rider_stamps_every_repeat_of_the_path() {
    use crate::event::Press::Main;
    let (own, levels) = rider_style(range_rider(20), 1);
    let ranges = |taken, presses: &[crate::event::Press]| {
        per_repeat(&copy_of(own.clone(), levels.clone(), taken), presses, |r| {
            r.range_pct
        })
    };
    assert_eq!(ranges(0, &[Main; 5]), [0, 0]);
    assert_eq!(ranges(1, &[Main; 5]), [20, 20]);
    assert_eq!(ranges(2, &[Main; 5]), [20, 20]);
    assert_eq!(ranges(3, &[Main; 5]), [20, 20, 20, 20]);
    assert_eq!(ranges(3, &[Main]), [20, 20]);
}

/// P4, P5: what A queues, as `(width_pct, proc_pct)`, with the run's width set to `run_width`
/// once it starts and `places` taken of a style whose first upgrade names the range rider.
fn queued_width(places: usize, run_width: u32) -> Vec<(u32, Option<u16>)> {
    let own = vec![place_repeat_kind(json!(["main"]), 10, 50, Some(30))];
    let mut r = bundled_rig(own, [vec![range_rider(20)], vec![], vec![]]);
    take(&mut r.e, 0, places);
    r.press(false);
    for _ in 0..120 {
        let s = st(&mut r.e);
        if let Some(i) = (0..s.runs.len()).find(|&i| super::attacks::is_combo(s, &s.runs[i])) {
            s.runs[i].width_pct = run_width;
            break;
        }
        r.step([Default::default(); 3]);
    }
    for _ in 0..300 {
        let s = st(&mut r.e);
        if !s.repeats.is_empty() {
            return s
                .repeats
                .iter()
                .map(|x| (x.width_pct, x.proc_pct))
                .collect();
        }
        r.step([Default::default(); 3]);
    }
    Vec::new()
}

/// P4: the stored width is the run's plus the rider's, held to `MAX_RANGE_PCT`.
#[test]
fn a_queued_repeat_adds_the_riders_range_to_the_runs_and_clamps_it() {
    assert_eq!(queued_width(1, 0), [(20, Some(30))]);
    assert_eq!(queued_width(1, 90), [(100, Some(30))]);
}

/// P5: with the rider's place not taken a repeat is queued as before: the run's own width and the
/// repeat's own factor.
#[test]
fn a_style_that_has_not_taken_a_rider_queues_as_before() {
    assert_eq!(queued_width(0, 0), [(0, Some(30))]);
    assert_eq!(queued_width(0, 90), [(90, Some(30))]);
}
