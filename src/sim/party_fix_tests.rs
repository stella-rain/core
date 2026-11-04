//! A downed or out player in a party (ADR-SIM-7 points 4 and 5, ADR-SIM-8): its offers, its hit
//! points, the boss offer and the outcome.

use super::state::{Due, PlayerIdx, State};
use super::{
    Blame, Culprit, Shot, World, decide_outcome, hurt_player, offer_open_for, tick_down_and_revive,
};
use crate::engine::{Engine, Outcome, Run};
use crate::event::{DomainEvent, HitKind};
use crate::id::EntityId;
use crate::input::Input;
use crate::rewards::Rarity;
use crate::run_setup::{OfferMode, RunSetup};
use crate::run_stats::StatEffect;
use crate::snapshot::OfferKind;
use crate::stage::{RarityWeights, Tier};

pub(super) fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("a slot")
}

/// A party of `n` at the start: stat offers are picked, nothing has been stepped.
pub(super) fn party(n: u8) -> (World, State) {
    let setup = RunSetup {
        seed: 20_261_104,
        pick_stats: true,
        ..RunSetup::default()
    };
    let e = Engine::start_party(&setup, n).expect("a party");
    let Run::V2 { world, state } = e.run() else {
        panic!("the live rules")
    };
    (world.clone(), state.clone())
}

pub(super) fn down(s: &mut State, i: u8) {
    s.players[slot(i)].hp = 0;
    s.players[slot(i)].down_ticks = 0;
}

pub(super) fn out(s: &mut State, i: u8) {
    s.players[slot(i)].hp = 0;
    s.players[slot(i)].out = true;
}

pub(super) fn star_due() -> Due {
    Due {
        kind: OfferKind::Star,
        rarity: Rarity::Common,
    }
}

fn idle() -> [Input; 3] {
    [Input::default(); 3]
}

/// Out, then down (ADR-SIM-7 point 5): with slot 1 out, a party fails when slot 0 is down too,
/// and runs on while it is up.
#[test]
fn one_out_and_the_other_goes_down_fails_the_run() {
    let (w, mut s) = party(2);
    out(&mut s, 1);
    let mut ev = Vec::new();
    decide_outcome(&w, &mut s, &mut ev);
    assert_eq!(s.outcome, Outcome::Running);
    assert!(ev.is_empty());

    down(&mut s, 0);
    decide_outcome(&w, &mut s, &mut ev);
    assert_eq!(s.outcome, Outcome::Failed);
    assert_eq!(ev, vec![DomainEvent::StageFailed]);
}

/// The queued offers of a player are dropped on the tick it goes out (ADR-SIM-7 point 4).
#[test]
fn queued_offers_are_dropped_when_a_player_goes_out() {
    let (w, mut s) = party(2);
    down(&mut s, 1);
    s.players[slot(1)].down_ticks = 1199;
    let career = s.players.career_mut(slot(1));
    career.progress.as_mut().expect("levels").offers_due = 2;
    career
        .offers
        .as_mut()
        .expect("stars")
        .due
        .extend([star_due(), star_due()]);
    let gauge = career.expertise.as_mut().expect("expertise");
    gauge.level = 3;
    gauge.offered = 1;

    let mut ev = Vec::new();
    tick_down_and_revive(&w, &mut s, &idle(), &mut ev);
    assert!(ev.contains(&DomainEvent::PlayerOut { player: slot(1) }));
    let career = s.players.career(slot(1));
    assert_eq!(career.progress.as_ref().unwrap().offers_due, 0);
    assert!(career.offers.as_ref().unwrap().due.is_empty());
    let gauge = career.expertise.as_ref().unwrap();
    assert_eq!(gauge.offered, gauge.level);
}

// --- offers of a downed and an out player (ADR-SIM-7 point 4) ----------------------------------

/// A party engine, `timed` or not, to edit and to open offers on.
fn engine(n: u8, timed: bool) -> Engine {
    let setup = RunSetup {
        seed: 20_261_104,
        pick_stats: true,
        offer_mode: if timed {
            OfferMode::Timed
        } else {
            OfferMode::Stop
        },
        ..RunSetup::default()
    };
    Engine::start_party(&setup, n).expect("a party")
}

fn st(e: &mut Engine) -> &mut State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

fn stat_offers_due(e: &mut Engine, i: u8, n: u32) {
    let p = st(e).players.career_mut(slot(i)).progress.as_mut().unwrap();
    p.offers_due = n;
}

fn count<F: Fn(&DomainEvent) -> bool>(e: &Engine, f: F) -> usize {
    e.events().iter().filter(|ev| f(ev)).count()
}

fn lethal(e: &mut Engine, i: u8) {
    let Run::V2 { world, state: s } = e.run_mut() else {
        panic!("the live rules")
    };
    let shot = Shot {
        at: s.players[slot(i)].at,
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
}

#[test]
fn a_downed_player_takes_a_due_stat_offer_on_the_tick_it_opens() {
    let mut e = engine(2, true);
    down(st(&mut e), 1);
    stat_offers_due(&mut e, 1, 1);
    e.open_offers_for_test();
    let s = st(&mut e);
    assert!(!offer_open_for(s, slot(1)));
    assert_eq!(
        s.players
            .career(slot(1))
            .progress
            .as_ref()
            .unwrap()
            .offers_due,
        0
    );
    assert_eq!(s.offer_timer.as_ref().unwrap().due[1], None);
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::StatRaised { .. })),
        1
    );
}

#[test]
fn a_downed_player_takes_its_whole_queue_on_one_tick() {
    let mut e = engine(2, true);
    down(st(&mut e), 1);
    st(&mut e)
        .players
        .career_mut(slot(1))
        .offers
        .as_mut()
        .unwrap()
        .due
        .extend([star_due(), star_due(), star_due()]);
    e.open_offers_for_test();
    let s = st(&mut e);
    let offers = s.players.career(slot(1)).offers.as_ref().unwrap();
    assert!(offers.due.is_empty() && offers.offer.is_none());
    assert!(!offer_open_for(s, slot(1)));
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::RewardTaken { .. })),
        3
    );
}

#[test]
fn going_down_takes_the_open_offer_and_the_one_queued_behind_it() {
    let mut e = engine(2, true);
    stat_offers_due(&mut e, 1, 2);
    e.open_offers_for_test();
    assert!(offer_open_for(st(&mut e), slot(1)));
    lethal(&mut e, 1);
    assert!(st(&mut e).players[slot(1)].is_downed());
    e.open_offers_for_test();
    let s = st(&mut e);
    assert!(!offer_open_for(s, slot(1)));
    assert_eq!(
        s.players
            .career(slot(1))
            .progress
            .as_ref()
            .unwrap()
            .offers_due,
        0
    );
    assert_eq!(s.offer_timer.as_ref().unwrap().due[1], None);
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::StatRaised { .. })),
        2
    );
}

#[test]
fn a_player_that_is_up_opens_one_offer_with_the_full_timer() {
    let mut e = engine(2, true);
    stat_offers_due(&mut e, 0, 2);
    e.open_offers_for_test();
    let s = st(&mut e);
    assert!(offer_open_for(s, slot(0)));
    assert_eq!(
        s.players
            .career(slot(0))
            .progress
            .as_ref()
            .unwrap()
            .offers_due,
        1
    );
    let want = s.tick + 300;
    assert_eq!(s.offer_timer.as_ref().unwrap().due[0], Some(want));
}

#[test]
fn an_out_player_opens_nothing_and_loses_what_is_queued() {
    let mut e = engine(2, true);
    out(st(&mut e), 1);
    stat_offers_due(&mut e, 1, 2);
    st(&mut e)
        .players
        .career_mut(slot(1))
        .offers
        .as_mut()
        .unwrap()
        .due
        .push(star_due());
    e.open_offers_for_test();
    let s = st(&mut e);
    assert!(!offer_open_for(s, slot(1)));
    let career = s.players.career(slot(1));
    assert_eq!(career.progress.as_ref().unwrap().offers_due, 0);
    assert!(career.offers.as_ref().unwrap().due.is_empty());
    assert_eq!(s.offer_timer.as_ref().unwrap().due[1], None);
    assert_eq!(
        count(&e, |ev| matches!(
            ev,
            DomainEvent::StatOfferOpened { .. } | DomainEvent::OfferOpened { .. }
        )),
        0
    );
}

/// A timed engine: the offer open for slot 1 is taken on the tick it goes down.
#[test]
fn an_offer_open_when_a_player_goes_down_closes_on_that_tick() {
    let mut e = engine(2, true);
    st(&mut e)
        .players
        .career_mut(slot(1))
        .offers
        .as_mut()
        .unwrap()
        .due
        .push(star_due());
    e.step(&idle());
    assert!(offer_open_for(st(&mut e), slot(1)));
    assert!(!e.offer_ratings(slot(1)).is_empty());
    lethal(&mut e, 1);
    e.step(&idle());
    assert!(!offer_open_for(st(&mut e), slot(1)));
    assert!(e.offer_ratings(slot(1)).is_empty());
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::RewardTaken { .. })),
        1
    );
}

// --- no hit points while downed (ADR-SIM-7) -----------------------------------------------------

/// The place of the MaxHp stat among the run stats.
fn max_hp_stat(s: &State, me: PlayerIdx) -> usize {
    let p = s.players.career(me).progress.as_ref().unwrap();
    p.stats
        .iter()
        .position(|r| r.effect == StatEffect::MaxHp)
        .expect("a MaxHp stat")
}

fn add_max_hp(s: &mut State, me: PlayerIdx, amount: u32) {
    let (player, career) = s.players.both_mut(me);
    super::levels::add_effect(
        career.progress.as_mut().unwrap(),
        player,
        StatEffect::MaxHp,
        amount,
    );
}

#[test]
fn max_hp_raises_the_most_and_heals_only_a_player_that_is_up() {
    let (_, mut s) = party(3);
    s.players[slot(0)].max_hp = 100;
    s.players[slot(0)].hp = 50;
    add_max_hp(&mut s, slot(0), 20);
    assert_eq!(
        (s.players[slot(0)].hp, s.players[slot(0)].max_hp),
        (70, 120)
    );

    for (i, is_out) in [(1, false), (2, true)] {
        s.players[slot(i)].max_hp = 100;
        s.players[slot(i)].hp = 0;
        s.players[slot(i)].out = is_out;
        add_max_hp(&mut s, slot(i), 20);
        assert_eq!((s.players[slot(i)].hp, s.players[slot(i)].max_hp), (0, 120));
    }
    assert!(s.players[slot(1)].is_downed());
}

#[test]
fn a_stat_offer_of_max_hp_taken_while_downed_leaves_the_player_down() {
    let mut e = engine(2, false);
    let s = st(&mut e);
    down(s, 1);
    s.players[slot(1)].max_hp = 100;
    let at = max_hp_stat(s, slot(1));
    s.players
        .career_mut(slot(1))
        .progress
        .as_mut()
        .unwrap()
        .offer_open = true;
    let option = crate::snapshot::OfferOption::Take {
        candidate: u8::try_from(at).unwrap(),
    };
    let mut ev = Vec::new();
    super::levels::choose(s, slot(1), option, &mut ev);
    assert!(
        ev.iter()
            .any(|e| matches!(e, DomainEvent::StatRaised { .. }))
    );
    assert_eq!(s.players[slot(1)].hp, 0);
    assert!(s.players[slot(1)].max_hp > 100);
    assert!(s.players[slot(1)].is_downed());
    // Nobody got up: with slot 0 down too the run fails.
    down(s, 0);
    let (w, _) = party(2);
    decide_outcome(&w, s, &mut ev);
    assert_eq!(s.outcome, Outcome::Failed);
}

/// A state with one enemy at 0 hit points that gives a great deal of experience, and the run's
/// stats spent by priority with MaxHp first.
fn level_up_state(n: u8) -> (World, State) {
    let mut e = engine(n, false);
    for _ in 0..1200 {
        if !st(&mut e).enemies.is_empty() {
            break;
        }
        e.step(&idle());
    }
    let Run::V2 { world, state } = e.run() else {
        panic!("the live rules")
    };
    let (mut world, mut s) = (world.clone(), state.clone());
    assert!(!s.enemies.is_empty(), "an enemy to defeat");
    s.enemies[0].hp = 0;
    s.enemies[0].burst = false;
    world.tuning.default_xp = 1_000_000;
    for w in &mut world.stage.waves {
        w.xp = Some(1_000_000);
    }
    for i in s.players.indices().collect::<Vec<_>>() {
        let at = max_hp_stat(&s, i);
        let p = s.players.career_mut(i).progress.as_mut().unwrap();
        p.pick_stats = false;
        p.priority = vec![u8::try_from(at).unwrap()];
    }
    (world, s)
}

#[test]
fn a_level_up_with_max_hp_first_does_not_lift_a_downed_player() {
    let (w, mut s) = level_up_state(2);
    s.players[slot(1)].hp = 0;
    s.players[slot(1)].max_hp = 100;
    let up = s.players[slot(0)].max_hp;
    let mut ev = Vec::new();
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    assert!(
        ev.iter()
            .any(|e| matches!(e, DomainEvent::LeveledUp { .. }))
    );
    assert_eq!(s.players[slot(1)].hp, 0);
    assert!(s.players[slot(1)].max_hp > 100, "the most still rises");
    assert!(s.players[slot(0)].max_hp > up);
    assert_eq!(s.players[slot(0)].hp, s.players[slot(0)].max_hp);
}

/// DS-0386: a solo run, or a party of one, follows the same rule at 0 hit points.
#[test]
fn a_solo_player_at_zero_hit_points_is_not_lifted_by_a_level_up() {
    for timed in [false, true] {
        let (w, mut s) = level_up_state(1);
        s.players[slot(0)].hp = 0;
        s.players[slot(0)].max_hp = 100;
        let mut ev = Vec::new();
        super::levels::credit_defeated(&w, &mut s, &mut ev);
        assert!(
            ev.iter()
                .any(|e| matches!(e, DomainEvent::LeveledUp { .. }))
        );
        assert_eq!(s.players[slot(0)].hp, 0, "timed {timed}");
        assert!(s.players[slot(0)].max_hp > 100);
        decide_outcome(&w, &mut s, &mut ev);
        assert_eq!(s.outcome, Outcome::Failed);
    }
}

#[test]
fn a_revive_after_a_max_hp_raise_while_downed_uses_the_raised_most() {
    let (w, mut s) = party(2);
    s.players[slot(1)].max_hp = 100;
    down(&mut s, 1);
    add_max_hp(&mut s, slot(1), 20);
    s.players[slot(1)].revive_hold = 179;
    s.players[slot(0)].at = s.players[slot(1)].at;
    let mut inputs = idle();
    inputs[0].interact = true;
    let mut ev = Vec::new();
    tick_down_and_revive(&w, &mut s, &inputs, &mut ev);
    assert_eq!(s.players[slot(1)].hp, 120 * 30 / 100);
}

/// Guards: nothing else lifts a player from 0 hit points.
#[test]
fn a_downed_player_gets_no_boss_heal_pickup_or_regeneration() {
    let (w, mut s) = party(2);
    down(&mut s, 1);
    assert!(w.heal.is_some(), "the bundled stage heals");
    let mut ev = Vec::new();
    super::heal::boss_killed(&w, &mut s, slot(1), &mut ev);
    let at = s.players[slot(1)].at;
    let id = EntityId(s.next_id);
    s.next_id += 1;
    s.heal_pickups
        .as_mut()
        .expect("pickups")
        .push(super::state::HealPickup { id, at });
    super::heal::pick_up(&w, &mut s, slot(1), &mut ev);
    s.players.career_mut(slot(1)).regen = 5;
    s.tick = 600;
    super::loadout::regen(&mut s, slot(1));
    assert_eq!(s.players[slot(1)].hp, 0);
    assert!(!ev.iter().any(|e| matches!(e, DomainEvent::Healed { .. })));
}

// --- the boss kill (ADR-SIM-7 point 5, ADR-SIM-8) ----------------------------------------------

fn weights(top: Rarity) -> RarityWeights {
    let mut w = RarityWeights::default();
    match top {
        Rarity::Uncommon => w.uncommon = 1,
        Rarity::Rare => w.rare = 1,
        Rarity::Epic => w.epic = 1,
        _ => w.common = 1,
    }
    w
}

/// Tiers from levels 1, 5 and 7 whose highest rarities are Uncommon, Rare and Epic; the
/// difficulty's range is every rarity; the players are at `levels`.
fn tiered(n: u8, levels: [u32; 3]) -> (World, State) {
    let (mut w, mut s) = party(n);
    let stars = w.stage.stars.as_mut().expect("stars");
    stars.tiers = [(1, Rarity::Uncommon), (5, Rarity::Rare), (7, Rarity::Epic)]
        .into_iter()
        .map(|(from_level, top)| Tier {
            from_level,
            weights: weights(top),
            random_rune_weight: None,
        })
        .collect();
    s.stars.as_mut().expect("stars").rarity = (Rarity::Common, Rarity::Supernova);
    for i in s.players.indices().collect::<Vec<_>>() {
        s.players.career_mut(i).progress.as_mut().unwrap().level = levels[i.get()];
    }
    (w, s)
}

/// What a boss kill does to the offers due, as `collide` calls it.
fn kill_boss(w: &World, s: &mut State) {
    super::stars::boss_killed(w, s);
}

fn boss_dues(s: &State, i: u8) -> Vec<Due> {
    let due = &s.players.career(slot(i)).offers.as_ref().unwrap().due;
    due.iter()
        .copied()
        .filter(|d| d.kind == OfferKind::Boss)
        .collect()
}

fn boss(rarity: Rarity) -> Vec<Due> {
    vec![Due {
        kind: OfferKind::Boss,
        rarity,
    }]
}

#[test]
fn a_boss_kill_offers_every_up_player_the_top_of_the_highest_level() {
    let (w, mut s) = tiered(3, [3, 7, 5]);
    kill_boss(&w, &mut s);
    for i in 0..3 {
        assert_eq!(boss_dues(&s, i), boss(Rarity::Epic), "slot {i}");
    }
}

#[test]
fn a_downed_or_out_player_gets_no_boss_offer_and_does_not_set_the_rarity() {
    for is_out in [false, true] {
        let (w, mut s) = tiered(3, [3, 7, 5]);
        if is_out {
            out(&mut s, 1);
        } else {
            down(&mut s, 1);
        }
        kill_boss(&w, &mut s);
        assert_eq!(boss_dues(&s, 0), boss(Rarity::Rare), "out {is_out}");
        assert_eq!(boss_dues(&s, 2), boss(Rarity::Rare), "out {is_out}");
        assert!(boss_dues(&s, 1).is_empty(), "out {is_out}");
    }
}

#[test]
fn one_player_up_gets_its_own_tier_and_nobody_up_gets_nothing() {
    let (w, mut s) = tiered(3, [3, 7, 5]);
    down(&mut s, 1);
    out(&mut s, 2);
    kill_boss(&w, &mut s);
    assert_eq!(boss_dues(&s, 0), boss(Rarity::Uncommon));
    assert!(boss_dues(&s, 1).is_empty() && boss_dues(&s, 2).is_empty());

    let (w, mut s) = tiered(3, [3, 7, 5]);
    for i in 0..3 {
        down(&mut s, i);
    }
    kill_boss(&w, &mut s);
    assert!((0..3).all(|i| boss_dues(&s, i).is_empty()));
}

/// DS-0386: a solo player at 0 hit points on the kill tick gets no boss offer.
#[test]
fn a_solo_player_at_zero_hit_points_gets_no_boss_offer() {
    let (w, mut s) = tiered(1, [7, 1, 1]);
    kill_boss(&w, &mut s);
    assert_eq!(boss_dues(&s, 0), boss(Rarity::Epic));
    let (w, mut s) = tiered(1, [7, 1, 1]);
    down(&mut s, 0);
    kill_boss(&w, &mut s);
    assert!(boss_dues(&s, 0).is_empty());
}

#[test]
fn the_boss_rarity_is_held_inside_the_difficulty_range() {
    let (w, mut s) = tiered(2, [7, 7, 1]);
    s.stars.as_mut().unwrap().rarity = (Rarity::Common, Rarity::Rare);
    kill_boss(&w, &mut s);
    assert_eq!(boss_dues(&s, 0), boss(Rarity::Rare));
    assert_eq!(boss_dues(&s, 1), boss(Rarity::Rare));
}

#[test]
fn a_players_boss_candidates_come_from_its_own_stream() {
    let candidates = |n: u8| {
        let mut e = engine(n, false);
        let Run::V2 { world, state } = e.run_mut() else {
            panic!("the live rules")
        };
        kill_boss(world, state);
        e.open_offers_for_test();
        let offer = st(&mut e)
            .players
            .career(slot(0))
            .offers
            .as_ref()
            .unwrap()
            .offer
            .clone();
        offer.expect("an offer").candidates
    };
    assert_eq!(candidates(3), candidates(2));
}

/// The step itself reaches `take_downed`: a downed player's star offer never stays open past
/// the tick.
#[test]
fn a_step_takes_a_downed_players_offer_in_a_stop_run() {
    let mut e = engine(2, false);
    down(st(&mut e), 1);
    st(&mut e)
        .players
        .career_mut(slot(1))
        .offers
        .as_mut()
        .unwrap()
        .due
        .push(star_due());
    e.step(&idle());
    assert!(!offer_open_for(st(&mut e), slot(1)));
    assert_eq!(
        count(&e, |ev| matches!(ev, DomainEvent::RewardTaken { .. })),
        1
    );
}

// --- experience, Stardust and the star tier of the players that are up (ADR-SIM-7 point 5) -----

/// Slot 0 up, slot 1 downed, slot 2 out, with a kill present.
fn one_up_one_downed_one_out() -> (World, State) {
    let (w, mut s) = level_up_state(3);
    down(&mut s, 1);
    out(&mut s, 2);
    (w, s)
}

fn level(s: &State, i: u8) -> u32 {
    s.players.career(slot(i)).progress.as_ref().unwrap().level
}

fn xp(s: &State, i: u8) -> u32 {
    s.players.career(slot(i)).progress.as_ref().unwrap().xp
}

fn leveled(ev: &[DomainEvent]) -> usize {
    ev.iter()
        .filter(|e| matches!(e, DomainEvent::LeveledUp { .. }))
        .count()
}

#[test]
fn experience_goes_to_the_players_that_are_up_only() {
    let (w, mut s) = one_up_one_downed_one_out();
    let mut ev = Vec::new();
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    assert!(level(&s, 0) > 1);
    assert_eq!(leveled(&ev), (level(&s, 0) - 1) as usize);
    for i in [1, 2] {
        assert_eq!((level(&s, i), xp(&s, i)), (1, 0), "slot {i}");
        assert_eq!(
            s.players
                .career(slot(i))
                .progress
                .as_ref()
                .unwrap()
                .offers_due,
            0
        );
    }
}

#[test]
fn a_downed_player_queues_no_stat_offer_from_a_kill() {
    let (w, mut s) = one_up_one_downed_one_out();
    for i in s.players.indices().collect::<Vec<_>>() {
        s.players
            .career_mut(i)
            .progress
            .as_mut()
            .unwrap()
            .pick_stats = true;
    }
    let mut ev = Vec::new();
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    let due = |i| {
        s.players
            .career(slot(i))
            .progress
            .as_ref()
            .unwrap()
            .offers_due
    };
    assert!(due(0) > 0);
    assert_eq!((due(1), due(2)), (0, 0));
}

#[test]
fn every_player_that_is_up_gets_the_same_experience() {
    let (w, mut s) = level_up_state(3);
    let mut ev = Vec::new();
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    assert!(level(&s, 0) > 1);
    assert_eq!(level(&s, 0), level(&s, 1));
    assert_eq!(level(&s, 1), level(&s, 2));
}

#[test]
fn a_revived_player_earns_from_the_next_kill() {
    let (w, mut s) = one_up_one_downed_one_out();
    let mut ev = Vec::new();
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    assert_eq!(level(&s, 1), 1);
    s.players[slot(1)].hp = 10;
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    assert!(level(&s, 1) > 1);
}

#[test]
fn nobody_up_earns_no_experience_and_a_solo_player_at_zero_hit_points_neither() {
    let (w, mut s) = level_up_state(3);
    down(&mut s, 0);
    down(&mut s, 1);
    out(&mut s, 2);
    let mut ev = Vec::new();
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    assert_eq!(leveled(&ev), 0);
    assert!((0..3).all(|i| (level(&s, i), xp(&s, i)) == (1, 0)));

    let (w, mut s) = level_up_state(1);
    down(&mut s, 0);
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    assert_eq!((leveled(&ev), level(&s, 0), xp(&s, 0)), (0, 1, 0));

    let (w, mut s) = level_up_state(1);
    super::levels::credit_defeated(&w, &mut s, &mut ev);
    assert!(level(&s, 0) > 1);
}
