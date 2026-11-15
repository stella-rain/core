//! The `charge_sooner` kind of a style level (ADR-SIM-9 points 2 to 7, ADR-SIM-5): the weapon
//! built for a path that has taken the kind reaches each level of the step's charge sooner and
//! changes nothing else. Every number is a test value or the bundled Sword's own.

use crate::expertise::{Expertise, sooner_at, weapon_with_style};
use crate::pack::Pack;
use crate::version::Versions;
use crate::weapon::Weapon;
use serde_json::{Value, json};

const AAA: [&str; 3] = ["main", "main", "main"];

/// The bundled Sword's expertise with `upgrade_2` naming the kind on `step` at `pct`.
fn sword_with(step: &[&str], pct: u32) -> (Weapon, Expertise) {
    let pack = Pack::from_bytes(Pack::BUNDLED).expect("the bundled pack");
    let base = pack
        .content()
        .weapons()
        .tree("giant_sword", None)
        .expect("the Sword's base tree")
        .clone();
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_sword")
        .unwrap();
    let up = &mut w["styles"][0]["upgrade_2"][0];
    up["effect"] = json!("none");
    up.as_object_mut().unwrap().remove("amount");
    up["kinds"] = json!([{ "type": "charge_sooner", "step": step, "pct": pct }]);
    let file = v["expertise"].to_string();
    let expertise = Expertise::parse(file.as_bytes(), Versions::CURRENT).expect("the expertise");
    (base, expertise)
}

fn built(base: &Weapon, expertise: &Expertise, taken: usize) -> Weapon {
    let style = &expertise
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_sword")
        .unwrap()
        .styles[0];
    weapon_with_style(base, style, taken)
}

fn charge_of(weapon: &Weapon) -> &crate::weapon::Charge {
    weapon
        .combo
        .main
        .as_deref()
        .and_then(|a| a.next.main.as_deref())
        .and_then(|a| a.next.main.as_deref())
        .and_then(|s| s.charge.as_ref())
        .expect("AAA's charge")
}

/// W1: places 0 and 1 keep the charge's ticks, places 2 and 3 take 25% off each level; the rest
/// of the charge equals the place-0 weapon's.
#[test]
fn the_levels_come_sooner_from_the_second_place_and_nothing_else_moves() {
    let (base, expertise) = sword_with(&AAA, 25);
    let first = built(&base, &expertise, 0);
    for (taken, at) in [(0, [20, 50]), (1, [20, 50]), (2, [15, 37]), (3, [15, 37])] {
        let weapon = built(&base, &expertise, taken);
        let charge = charge_of(&weapon);
        let ticks: Vec<u32> = charge.levels.iter().map(|l| l.at_ticks).collect();
        assert_eq!(ticks, at, "taken {taken}");
        let mut expected = charge_of(&first).clone();
        for (level, now) in expected.levels.iter_mut().zip(at) {
            level.at_ticks = now;
        }
        assert_eq!(charge, &expected, "taken {taken}: only at_ticks differ");
        assert_eq!(charge.max_ticks, 90);
    }
}

/// W2: the one formula, floored, at least 1, with no overflow.
#[test]
fn the_formula_floors_and_never_overflows() {
    assert_eq!(sooner_at(20, 25), 15);
    assert_eq!(sooner_at(50, 25), 37);
    assert_eq!(sooner_at(1, 90), 1);
    assert_eq!(sooner_at(10, 25), 7);
    assert_eq!(sooner_at(u32::MAX, 90), 429_496_729);
}

/// W3: a step with no charge in the base (AAAAA) is left alone by the build.
#[test]
fn a_step_with_no_charge_leaves_the_weapon_unchanged() {
    let (base, expertise) = sword_with(&["main"; 5], 25);
    assert_eq!(built(&base, &expertise, 2), built(&base, &expertise, 0));
}

// --- Behaviour: the Sword in a run (R1 to R4) -----------------------------------------------

use crate::engine::{Engine, Run};
use crate::event::DomainEvent;
use crate::input::Input;
use crate::run_setup::RunSetup;
use crate::sim::state::PlayerIdx;

/// The bundled pack with the Sword's `upgrade_2` naming the kind.
fn pack_with_kind() -> Pack {
    let mut v: Value = serde_json::from_slice(Pack::BUNDLED).unwrap();
    let w = v["expertise"]["weapons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|w| w["weapon"] == "giant_sword")
        .unwrap();
    let up = &mut w["styles"][0]["upgrade_2"][0];
    up["effect"] = json!("none");
    up.as_object_mut().unwrap().remove("amount");
    up["kinds"] = json!([{ "type": "charge_sooner", "step": AAA, "pct": 25 }]);
    Pack::from_bytes(&serde_json::to_vec(&v).unwrap()).expect("the test pack")
}

fn state_of(e: &mut Engine) -> &mut crate::sim::state::State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// A run of `places.len()` players on the Sword's first style, player `i` having taken
/// `places[i]` places of its path, standing alone (no enemy) and hardy.
fn rig(places: &[usize]) -> Engine {
    rig_on(&pack_with_kind(), places)
}

fn rig_on(pack: &Pack, places: &[usize]) -> Engine {
    let setup = RunSetup {
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        seed: 20_261_113,
        ..RunSetup::default()
    };
    let pack = pack_with_kind();
    let mut e =
        Engine::start_party_with(&setup, pack.content(), u8::try_from(places.len()).unwrap())
            .expect("a run");
    let s = state_of(&mut e);
    s.enemies.clear();
    for (i, taken) in places.iter().enumerate() {
        let slot = PlayerIdx::new(u8::try_from(i).unwrap()).expect("a slot");
        s.players[slot].hp = 100_000;
        s.players[slot].max_hp = 100_000;
        let g = s
            .players
            .career_mut(slot)
            .expertise
            .as_mut()
            .expect("expertise");
        g.style = Some(0);
        g.path = vec![1; *taken].into_iter().collect();
    }
    e
}

fn main_held(on: bool) -> Input {
    Input {
        main: on,
        ..Input::default()
    }
}

/// Two taps of A (a gap of 8 ticks each), then the third press held: the third step charges.
fn two_taps_then_hold(e: &mut Engine, players: usize) {
    let step = |e: &mut Engine, on: bool| {
        let mut inputs = [Input::default(); 3];
        for input in inputs.iter_mut().take(players) {
            *input = main_held(on);
        }
        e.step(&inputs);
    };
    for _ in 0..2 {
        step(e, true);
        for _ in 0..8 {
            step(e, false);
        }
    }
}

/// Holds A, every player in `held_by`, until player 0 shows `held` ticks held; every player's
/// `(level, held)` at that moment.
fn hold_to(e: &mut Engine, players: usize, held: u32) -> Vec<Option<(u8, u32)>> {
    for _ in 0..200 {
        let mut inputs = [Input::default(); 3];
        for input in inputs.iter_mut().take(players) {
            *input = main_held(true);
        }
        e.step(&inputs);
        let seen: Vec<_> = e
            .snapshot()
            .players
            .iter()
            .take(players)
            .map(|p| p.charge.map(|c| (c.level, c.held)))
            .collect();
        if seen[0].is_some_and(|(_, h)| h == held) {
            return seen;
        }
    }
    panic!("no charge reached {held} held");
}

/// R1: the level the charge shows at each held count, at place 1 and place 2.
#[test]
fn the_charge_levels_come_at_the_new_ticks() {
    for (place, expected) in [
        (1, [(19, 0), (20, 1), (49, 1), (50, 2)]),
        (2, [(14, 0), (15, 1), (36, 1), (37, 2)]),
    ] {
        for (held, level) in expected {
            let mut e = rig(&[place]);
            two_taps_then_hold(&mut e, 1);
            let seen = hold_to(&mut e, 1, held);
            assert_eq!(seen[0], Some((level, held)), "place {place}, held {held}");
        }
    }
}

/// The `(level, perfect)` of the first `ChargeReleased` after player 0 lets go at `held`.
fn release_at(place: usize, held: u32) -> Option<(u8, bool)> {
    let mut e = rig(&[place]);
    two_taps_then_hold(&mut e, 1);
    hold_to(&mut e, 1, held);
    for _ in 0..4 {
        e.step(&[Input::default(); 3]);
        for ev in e.events() {
            if let DomainEvent::ChargeReleased { level, perfect } = ev {
                return Some((*level, *perfect));
            }
        }
    }
    None
}

/// R2: at place 2 a tap let go at held 15 is perfect at level 1 (the window runs 15 to 20), at
/// held 21 it is not; at place 1 the same tap at held 15 plays level 0, which says nothing.
#[test]
fn the_perfect_window_moves_with_the_level() {
    assert_eq!(release_at(2, 15), Some((1, true)));
    assert_eq!(release_at(2, 21), Some((1, false)));
    assert_eq!(release_at(1, 15), None);
}

/// R3: a charge held to level 2 has the super armour from held 37 at place 2, from 50 at place 1.
#[test]
fn the_level_two_super_armour_comes_with_the_level() {
    for (place, first) in [(2, 37), (1, 50)] {
        for held in [first - 1, first] {
            let mut e = rig(&[place]);
            two_taps_then_hold(&mut e, 1);
            hold_to(&mut e, 1, held);
            let Run::V2 { world, state } = e.run_mut() else {
                panic!("the live rules")
            };
            assert_eq!(
                crate::sim::player::super_armour(world, state, PlayerIdx::SOLO),
                held >= first,
                "place {place}, held {held}"
            );
        }
    }
}

/// R4: two players on one pack read their own charge; the same inputs give the same hash; and
/// with no place taken the charge at held 40 is level 1 where place 2 reads level 2.
#[test]
fn each_player_reads_their_own_charge_and_the_run_replays() {
    let mut e = rig(&[2, 1]);
    two_taps_then_hold(&mut e, 2);
    let seen = hold_to(&mut e, 2, 37);
    assert_eq!(seen[0], Some((2, 37)));
    assert_eq!(seen[1], Some((1, 37)));
    let seen = hold_to(&mut e, 2, 50);
    assert_eq!(seen[1], Some((2, 50)));

    let hash = |places: &[usize]| {
        let mut e = rig(places);
        two_taps_then_hold(&mut e, places.len());
        for _ in 0..600 {
            e.step(&[main_held(true), main_held(true), Input::default()]);
        }
        e.state_hash()
    };
    assert_eq!(hash(&[2, 1]), hash(&[2, 1]));

    let mut e = rig(&[0, 2]);
    two_taps_then_hold(&mut e, 2);
    let seen = hold_to(&mut e, 2, 40);
    assert_eq!(seen[0], Some((1, 40)), "no place taken: level 1");
    assert_eq!(seen[1], Some((2, 40)), "place 2: level 2");
}

/// C3: the bundled Sword at each place of Spirit Blade: the charge at 20 and 50 ticks at places
/// 0 and 1 and at 15 and 37 at 2 and 3; the spirit lasts 1200 ticks at place 0 and 1800 from
/// place 1; the capstone's repeat is there at place 3 only.
#[test]
fn the_bundled_sword_at_each_place_of_spirit_blade() {
    let pack = Pack::from_bytes(Pack::BUNDLED).expect("the bundled pack");
    let base = pack
        .content()
        .weapons()
        .tree("giant_sword", None)
        .expect("the Sword's base tree")
        .clone();
    let style = &pack
        .content()
        .expertise()
        .weapons()
        .iter()
        .find(|w| w.weapon.as_str() == "giant_sword")
        .unwrap()
        .styles[0];
    assert_eq!(style.id.as_str(), "spirit_blade");
    fn count(b: &crate::weapon::Branches) -> usize {
        [b.main.as_deref(), b.smash.as_deref()]
            .into_iter()
            .flatten()
            .map(|s| s.repeats.len() + count(&s.next))
            .sum()
    }
    let repeats = |w: &Weapon| count(&w.combo);
    for (taken, ticks, repeating) in [
        (0, [20, 50], 0),
        (1, [20, 50], 0),
        (2, [15, 37], 0),
        (3, [15, 37], 1),
    ] {
        let weapon = weapon_with_style(&base, style, taken);
        let at: Vec<u32> = charge_of(&weapon)
            .levels
            .iter()
            .map(|l| l.at_ticks)
            .collect();
        assert_eq!(at, ticks, "taken {taken}");
        assert_eq!(
            repeats(&weapon),
            repeating,
            "taken {taken}: the capstone's repeat"
        );
    }
    for (taken, decay) in [(0, 1200), (1, 1800), (2, 1800), (3, 1800)] {
        let mut e = rig_on(&pack, &[taken]);
        let Run::V2 { world, state } = e.run_mut() else {
            panic!("the live rules")
        };
        let grant = world.spirit_of(state, PlayerIdx::SOLO).expect("the spirit");
        assert_eq!(grant.decay_ticks, decay, "taken {taken}");
    }
}
