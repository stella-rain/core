//! ADR-014 spike: a tiny simulation whose per-tick hashes must be identical on every platform.
//! CI runs these on x86_64 and on an arm64 runner against the same golden value (ADR-019).

use stella_rain_core::spike::{Input, Spike};

/// Hash after `GOLDEN_TICKS` ticks of `script` from seed `GOLDEN_SEED`. Every platform must
/// produce exactly this; a change to it is a change to the spike's rules.
const GOLDEN_SEED: u64 = 0x5354_454c_4c41_5241; // "STELLARA"
const GOLDEN_TICKS: u32 = 600;
const GOLDEN_HASH: u64 = 0x30387edf3607e244;

fn script(tick: u32) -> Input {
    // A fixed, varied input: drift right, then up-left, focus every third second.
    let phase = (tick / 60) % 4;
    let (dx, dy) = match phase {
        0 => (1, 0),
        1 => (-1, -1),
        2 => (0, 1),
        _ => (1, 1),
    };
    Input {
        dx,
        dy,
        focus: (tick / 60) % 3 == 2,
    }
}

fn run(seed: u64, ticks: u32, input: impl Fn(u32) -> Input) -> Vec<u64> {
    let mut sim = Spike::new(seed);
    (0..ticks)
        .map(|t| {
            sim.step(input(t));
            sim.state_hash()
        })
        .collect()
}

#[test]
fn the_same_seed_and_inputs_give_the_same_hash_every_tick() {
    assert_eq!(
        run(GOLDEN_SEED, GOLDEN_TICKS, script),
        run(GOLDEN_SEED, GOLDEN_TICKS, script)
    );
}

#[test]
fn the_seed_changes_the_hashes() {
    assert_ne!(run(1, 120, script), run(2, 120, script));
}

#[test]
fn the_input_changes_the_hashes() {
    let still = |_| Input {
        dx: 0,
        dy: 0,
        focus: false,
    };
    assert_ne!(run(GOLDEN_SEED, 120, script), run(GOLDEN_SEED, 120, still));
}

#[test]
fn bullets_spawn_and_leave_the_field() {
    let mut sim = Spike::new(GOLDEN_SEED);
    let mut most = 0;
    for t in 0..GOLDEN_TICKS {
        sim.step(script(t));
        most = most.max(sim.bullet_count());
    }
    assert!(most > 0, "no bullet ever spawned");
    assert!(most < 200, "bullets never leave the field: {most}");
}

#[test]
fn the_golden_hash_is_the_same_on_every_platform() {
    let hashes = run(GOLDEN_SEED, GOLDEN_TICKS, script);
    assert_eq!(
        *hashes.last().unwrap(),
        GOLDEN_HASH,
        "golden hash differs on {}",
        std::env::consts::ARCH
    );
}
