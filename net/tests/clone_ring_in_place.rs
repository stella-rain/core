//! A rollback through the in-place `CloneRing` ends on the straight run's hash, and the ring
//! reuses its slots and restores exactly.

mod support;

use stella_rain_core::input::Input;
use stella_rain_net::save::{CloneRing, Save};
use stella_rain_net::session::{Config, Session};
use support::{CountingSave, engine, frame_of, script, slot, straight};

/// Delivers the remote frames due before call `k`: every one arrives `w` ticks late, so each
/// window is mispredicted.
fn deliver<S: Save>(session: &mut Session<S>, s: &support::Script, w: u32, k: u32) {
    if k >= w {
        for who in [1u8, 2] {
            session
                .receive_inputs(slot(who), &frame_of(s, who, k - w, k - w))
                .unwrap();
        }
    }
    for who in [1u8, 2] {
        session.ack(slot(who), session.tick() + 2).unwrap();
    }
}

#[test]
fn rollback_through_the_ring_ends_on_the_straight_hash_at_windows_4_and_16() {
    for w in [4u32, 16] {
        let s = script(3, 160, 2, None);
        let oracle = straight(&s, 3);
        let config = Config {
            window: w,
            ..Config::new(slot(0))
        };
        let mut ring = Session::new(engine(3), config).unwrap();
        let mut deep = Session::with_save(engine(3), config, CountingSave::default()).unwrap();
        let mut rolled = 0u64;
        for k in 0..120u32 {
            deliver(&mut ring, &s, w, k);
            deliver(&mut deep, &s, w, k);
            let local = &s.eff[k as usize + 2][0];
            let a = ring.advance(local).unwrap();
            let b = deep.advance(local).unwrap();
            assert_eq!(a, b, "window {w}, call {k}");
            rolled += u64::from(a.rolled_back);
            assert_eq!(ring.tick(), deep.tick(), "window {w}, call {k}");
            for t in 0..=ring.tick() {
                let (x, y) = (ring.hash_after(t), deep.hash_after(t));
                assert_eq!(x, y, "window {w}, call {k}, tick {t}");
                if let Some(h) = x {
                    assert_eq!(h, oracle[t as usize], "window {w}, call {k}, tick {t}");
                }
            }
        }
        assert!(rolled > 0, "window {w}: the script mispredicts");
    }
}

fn step(e: &mut stella_rain_core::engine::Engine) {
    e.step(&[Input::default(); 3]);
}

#[test]
fn a_ring_reuses_its_slots_and_restores_exactly() {
    let mut e = engine(1);
    let mut ring = CloneRing::new(5);
    let mut hashes = Vec::new();
    for t in 0..60u32 {
        ring.save(t, &e);
        hashes.push(e.state_hash());
        step(&mut e);
        if t % 11 == 0 {
            ring.forget_before(t.saturating_sub(3));
        }
        assert!(ring.held() <= 5);
    }
    let mut probe = engine(1);
    for _ in 0..9 {
        step(&mut probe);
    }
    for t in (55..60u32).rev() {
        assert!(ring.load(t, &mut probe), "tick {t} is held");
        assert_eq!(probe.state_hash(), hashes[t as usize], "tick {t}");
        assert_eq!(probe.tick(), t);
    }
    let before = probe.state_hash();
    assert!(!ring.load(54, &mut probe), "tick 54 was dropped");
    assert!(!ring.load(60, &mut probe), "tick 60 was never saved");
    assert_eq!(
        probe.state_hash(),
        before,
        "a refused load leaves the engine"
    );

    // Saving a held tick again replaces it.
    let mut other = engine(1);
    for _ in 0..3 {
        step(&mut other);
    }
    ring.save(58, &other);
    assert!(ring.load(58, &mut probe));
    assert_eq!(probe.state_hash(), other.state_hash());
    assert_eq!(ring.held(), 5);

    // Dropping below a tick, then filling again, still restores exactly.
    ring.forget_before(59);
    assert_eq!(ring.held(), 1);
    for t in 60..70u32 {
        ring.save(t, &e);
        hashes.push(e.state_hash());
        step(&mut e);
    }
    for t in 65..70u32 {
        assert!(ring.load(t, &mut probe), "tick {t} is held");
        assert_eq!(probe.state_hash(), hashes[t as usize], "tick {t}");
    }
    assert!(!ring.load(59, &mut probe));

    // A capacity of 0 keeps 1.
    let mut one = CloneRing::new(0);
    one.save(1, &e);
    one.save(2, &e);
    assert_eq!(one.held(), 1);
    assert!(one.load(2, &mut probe) && !one.load(1, &mut probe));
}
