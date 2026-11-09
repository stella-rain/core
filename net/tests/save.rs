//! `CloneRing`: it keeps the newest `capacity` saves, restores a held tick exactly and refuses
//! one it does not hold without touching the engine.

mod support;

use stella_rain_core::input::Input;
use stella_rain_net::save::{CloneRing, Save};
use support::engine;

fn step(e: &mut stella_rain_core::engine::Engine) {
    e.step(&[Input::default(); 3]);
}

#[test]
fn a_ring_of_five_keeps_the_newest_five_and_restores_them_exactly() {
    let mut e = engine(1);
    let mut ring = CloneRing::new(5);
    let mut hashes = Vec::new();
    for t in 0..10 {
        ring.save(t, &e);
        hashes.push(e.state_hash());
        step(&mut e);
    }
    assert_eq!(ring.held(), 5);
    let mut probe = engine(1);
    for _ in 0..3 {
        step(&mut probe);
    }
    let before = probe.state_hash();
    assert!(!ring.load(4, &mut probe), "tick 4 was dropped");
    assert_eq!(
        probe.state_hash(),
        before,
        "a refused load leaves the engine"
    );
    for t in 5..=9u32 {
        assert!(ring.load(t, &mut probe), "tick {t} is held");
        assert_eq!(probe.state_hash(), hashes[usize::try_from(t).unwrap()]);
        assert_eq!(probe.tick(), t);
    }
}

#[test]
fn saving_a_tick_again_replaces_it_and_keeps_the_count() {
    let mut e = engine(1);
    let mut ring = CloneRing::new(5);
    for t in 0..10 {
        ring.save(t, &e);
        step(&mut e);
    }
    // `e` is now one tick past what was saved as tick 9.
    let newer = e.state_hash();
    ring.save(9, &e);
    assert_eq!(ring.held(), 5);
    let mut probe = engine(1);
    assert!(ring.load(9, &mut probe));
    assert_eq!(probe.state_hash(), newer);
    assert!(ring.load(5, &mut probe), "the older saves stay");
}

#[test]
fn forget_before_drops_the_older_saves() {
    let mut e = engine(1);
    let mut ring = CloneRing::new(5);
    for t in 0..10 {
        ring.save(t, &e);
        step(&mut e);
    }
    ring.forget_before(8);
    assert_eq!(ring.held(), 2);
    let mut probe = engine(1);
    assert!(ring.load(8, &mut probe));
    assert!(ring.load(9, &mut probe));
    assert!(!ring.load(7, &mut probe));
}

#[test]
fn a_ring_of_zero_keeps_one() {
    let e = engine(1);
    let mut ring = CloneRing::new(0);
    ring.save(0, &e);
    ring.save(1, &e);
    assert_eq!(ring.held(), 1);
}
