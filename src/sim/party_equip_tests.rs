//! The party rules of equipment mode that the drops and the slots already build (ADR-SIM-7,
//! ADR-SAVE-7, ADR-REPL-5): every member that is not out rolls on a pickup, a downed member keeps
//! its rolls, an out member gets none, a leaver's items go with it and a joiner takes the start
//! loadout.

use super::drops::DropTable;
use super::drops_tests::{after, drop_star, expected, inventory, rewards, tiered};
use super::equipment::Held;
use super::party_fix_tests::{down, out, slot};
use super::runes::FULL_POOL;
use super::starlight::bundled_rules;
use super::stars;
use crate::engine::{Engine, Run};
use crate::fixed::Fx;
use crate::rewards::Rarity;
use crate::run_setup::RunSetup;
use crate::rune::Rune;
use crate::snapshot::{HeldView, SlotKindView};

/// A party of `n` whose rare stars are `a`, `b`, `c` (weights 10, 20, 30), in equipment mode.
fn rare_party(n: u8) -> (super::World, super::State) {
    let (mut w, mut s) = tiered(n);
    let rw = rewards(
        &[
            ("a", "rare", false, 10),
            ("b", "rare", false, 20),
            ("c", "rare", false, 30),
        ],
        false,
    );
    w.drops = DropTable::of(&rw, bundled_rules());
    w.equipment_rules = bundled_rules().clone();
    s.stars.as_mut().unwrap().pool = vec![0, 1, 2];
    s.stars.as_mut().unwrap().rune_pool = FULL_POOL;
    (w, s)
}

const PLACES: [(u16, u32); 3] = [(0, 10), (1, 20), (2, 30)];

/// E1: the downed member rolls into its own inventory and keeps what it got, the out member
/// rolls nothing and its stream does not move.
#[test]
fn a_downed_member_keeps_the_rolls_it_gets() {
    let (w, mut s) = rare_party(3);
    down(&mut s, 1);
    out(&mut s, 2);
    drop_star(&mut s, 0, Rarity::Rare);
    let before = s.clone();
    stars::pick_up(&w, &mut s, slot(0), &mut Vec::new());
    for i in [0, 1] {
        let was = before.players.career(slot(i));
        let want = expected(&was.offer, &Rune::ALL, &PLACES);
        assert_eq!(inventory(&s, i), vec![want], "slot {i}");
        assert_eq!(
            s.players.career(slot(i)).offer.state(),
            after(&was.offer, 2).state(),
            "slot {i} drew twice"
        );
    }
    assert!(inventory(&s, 2).is_empty());
    assert_eq!(
        s.players.career(slot(2)).offer.state(),
        before.players.career(slot(2)).offer.state(),
        "the out member's stream did not move"
    );
    // A revive does not take the item away.
    s.players[slot(1)].hp = 1;
    assert_eq!(inventory(&s, 1).len(), 1);
}

/// E2: a downed member collects nothing itself.
#[test]
fn a_downed_member_collects_nothing() {
    let (w, mut s) = rare_party(2);
    s.players[slot(0)].at.x = Fx::px(3_000);
    down(&mut s, 1);
    drop_star(&mut s, 1, Rarity::Rare);
    let before = s.clone();
    stars::pick_up(&w, &mut s, slot(1), &mut Vec::new());
    assert_eq!(s.stars.as_ref().unwrap().on_map.len(), 1);
    for i in 0..2 {
        assert_eq!(inventory(&s, i), inventory(&before, i), "slot {i}");
    }
}

/// E3: a boss kill gives the downed and the out nothing; the up member gets the rules' `boss_drops`.
#[test]
fn a_boss_kill_gives_the_downed_and_the_out_nothing() {
    let (w, mut s) = rare_party(3);
    down(&mut s, 1);
    out(&mut s, 2);
    stars::boss_killed(&w, &mut s);
    assert_eq!(
        inventory(&s, 0).len(),
        usize::from(bundled_rules().boss_drops)
    );
    assert!(inventory(&s, 1).is_empty() && inventory(&s, 2).is_empty());
}

fn equipment_setup() -> RunSetup {
    RunSetup {
        seed: 20_261_106,
        ..RunSetup::default()
    }
}

fn state_mut(e: &mut Engine) -> &mut super::State {
    match e.run_mut() {
        Run::V2 { state, .. } => state,
        Run::V1 { .. } => panic!("the live rules"),
    }
}

/// A party of 3 in which slot 2 holds `items` (or nothing), 5 Starlight and a tree point.
fn holder(items: bool) -> Engine {
    let mut e = Engine::start_party(&equipment_setup(), 3).expect("a party");
    if items {
        let eq = &mut state_mut(&mut e).players.career_mut(slot(2)).equipment;
        eq.inventory = vec![Held::Star(1), Held::Rune(Rune::Thunder)];
        eq.starlight = 5;
        eq.tree_points = 1;
    }
    e
}

/// E4: a leaver's inventory, Starlight and tree point go with it, in the hash too.
#[test]
fn a_leaver_takes_the_inventory_with_it() {
    let (mut rich, mut poor) = (holder(true), holder(false));
    assert_ne!(rich.state_hash(), poor.state_hash(), "the items are hashed");
    rich.leave(slot(2)).expect("leaves");
    poor.leave(slot(2)).expect("leaves");
    assert_eq!(rich.present(), 0b011);
    assert_eq!(state_mut(&mut rich).players.generation(slot(2)), 1);
    assert_eq!(rich.state_hash(), poor.state_hash());
}

/// E5: a pickup after a leave rolls for the present members only.
#[test]
fn a_pickup_after_a_leave_rolls_for_the_present_only() {
    let (w, mut s) = rare_party(3);
    s.players.leave(slot(2));
    drop_star(&mut s, 0, Rarity::Rare);
    let before = s.clone();
    stars::pick_up(&w, &mut s, slot(0), &mut Vec::new());
    // An absent slot's career is not readable; the present ones each rolled once.
    assert!(!s.players.is_present(slot(2)));
    for i in [0, 1] {
        let was = before.players.career(slot(i));
        assert_eq!(
            inventory(&s, i),
            vec![expected(&was.offer, &Rune::ALL, &PLACES)]
        );
    }
    // Slot 2 rejoining starts from its own fresh stream, not one the pickup advanced.
    let template = before.players.template();
    s.players.join(slot(2), &template, s.crit_seed);
    assert!(inventory(&s, 2).is_empty());
}

/// E6: a joiner takes the start loadout, not slot 0's progress.
#[test]
fn a_joiner_takes_the_start_loadout() {
    let setup = RunSetup {
        lumine: 4,
        lumine_star: Some("star_mintaka".into()),
        starting_rune: Some("thunder".into()),
        ..equipment_setup()
    };
    let mut e = Engine::start(&setup).expect("starts");
    {
        let eq = &mut state_mut(&mut e).players.career_mut(slot(0)).equipment;
        let lumine = eq.lumine.as_mut().expect("a Lumine slot");
        lumine.level = 3;
        lumine.expertise = 20;
        eq.inventory = vec![Held::Star(0); 3];
    }
    e.join(slot(1)).expect("joins");
    let view = &e.snapshot().players[1];
    let lumine = view.slots.last().expect("slots");
    assert_eq!(lumine.kind, SlotKindView::Lumine);
    assert_eq!((lumine.level, lumine.expertise), (1, 0));
    assert!(matches!(
        &lumine.held,
        Some(HeldView::Star { id, .. }) if id.as_str() == "star_mintaka"
    ));
    assert_eq!(view.slots.len(), 13);
    assert!(view.inventory.is_empty());
    assert_eq!(view.starlight, 0);
    assert!(
        view.runes.iter().any(|r| r.rune == Rune::Thunder),
        "the starting rune is in a rune slot"
    );
    // Slot 0 keeps its own.
    let mine = e.snapshot().players[0].slots.last().cloned().unwrap();
    assert_eq!((mine.level, mine.expertise), (3, 20));
}

fn start_state(setup: &RunSetup, n: u8) -> Engine {
    Engine::start_party(setup, n).expect("a party")
}

fn regens(e: &mut Engine, n: u8) -> Vec<u32> {
    let s = state_mut(e);
    (0..n).map(|i| s.players.career(slot(i)).regen).collect()
}

/// R1: every member starts with the perk's regeneration.
#[test]
fn a_party_member_starts_with_the_perk_regen() {
    let setup = RunSetup {
        perk_set: "vanguard".into(),
        ..equipment_setup()
    };
    assert_eq!(regens(&mut start_state(&setup, 3), 3), vec![1_000; 3]);
}

/// R2: and with the regeneration of a regen Lumine star, with the same maximum hit points.
#[test]
fn a_party_member_starts_with_a_regen_lumine_star() {
    let setup = RunSetup {
        perk_set: "vanguard".into(),
        lumine: 4,
        lumine_star: Some("star_mintaka".into()),
        ..equipment_setup()
    };
    let mut e = start_state(&setup, 3);
    assert_eq!(regens(&mut e, 3), vec![4_000; 3]);
    let s = state_mut(&mut e);
    for i in 1..3 {
        assert_eq!(s.players[slot(i)].max_hp, s.players[slot(0)].max_hp);
    }
}

/// R3: a joiner takes the start regeneration, not what slot 0 has gained since; the second's
/// regeneration heals an up member and never a downed one.
#[test]
fn a_joiner_takes_the_start_regen_not_slot_0s_current() {
    let setup = RunSetup {
        perk_set: "vanguard".into(),
        ..equipment_setup()
    };
    let mut e = start_state(&setup, 1);
    state_mut(&mut e).players.career_mut(slot(0)).regen = 9_000;
    e.join(slot(1)).expect("joins");
    e.join(slot(2)).expect("joins");
    let s = state_mut(&mut e);
    assert_eq!(s.players.career(slot(1)).regen, 1_000);
    s.players[slot(1)].hp = s.players[slot(1)].max_hp - 500;
    down(s, 2);
    s.tick = 60;
    for i in 0..3 {
        super::loadout::regen(s, slot(i));
    }
    assert_eq!(s.players[slot(1)].hp, s.players[slot(1)].max_hp);
    assert_eq!(s.players[slot(2)].hp, 0);
}

/// T8: a boss kill pays each member who is up the pack's `boss_drops` rolls: 2 give two, 0 none.
#[test]
fn a_boss_kill_rolls_the_rules_boss_drops() {
    for n in [0u8, 2] {
        let (mut w, mut s) = rare_party(2);
        w.equipment_rules.boss_drops = n;
        stars::boss_killed(&w, &mut s);
        for i in 0..2 {
            assert_eq!(
                inventory(&s, i).len(),
                usize::from(n),
                "{n} drops, slot {i}"
            );
        }
    }
}

/// T8: the rune share is the pack's: 0 never gives a rune, 1000 only runes.
#[test]
fn a_drop_is_a_rune_by_the_rules_share() {
    for (share, runes) in [(0u32, false), (1000, true)] {
        let (mut w, mut s) = rare_party(1);
        w.equipment_rules.rune_share_permille = share;
        w.equipment_rules.boss_drops = 8;
        for _ in 0..50 {
            stars::boss_killed(&w, &mut s);
        }
        let items = inventory(&s, 0);
        assert!(!items.is_empty());
        assert!(
            items.iter().all(|h| matches!(h, Held::Rune(_)) == runes),
            "share {share}"
        );
    }
}
