//! A steady tick neither allocates nor frees (T1b): the per-tick buffers are reused and the
//! per-enemy lists are inline. An allocator call inside a measured tick would make a bench's
//! instruction count depend on the heap's layout, so both are counted. The counting allocator is
//! installed in `integration.rs` (a global allocator must be in the crate root); its counters are
//! per thread, so parallel tests do not disturb each other.

#![expect(unsafe_code, reason = "the counting allocator forwards to System")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use stella_rain_core::version::Versions;

use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{CommandKind, RunReplay};

thread_local! {
    static COUNT: Cell<u64> = const { Cell::new(0) };
    static REALLOCS: Cell<u64> = const { Cell::new(0) };
    static FREES: Cell<u64> = const { Cell::new(0) };
    static LIVE: Cell<i64> = const { Cell::new(0) };
    static PEAK: Cell<i64> = const { Cell::new(0) };
}

/// The system allocator, counting the allocations, the frees and the bytes held by the current
/// thread.
pub struct Counting;

#[expect(
    clippy::cast_possible_wrap,
    reason = "a Layout size is at most isize::MAX"
)]
fn grew(bytes: usize) {
    COUNT.with(|c| c.set(c.get() + 1));
    let live = LIVE.with(|l| {
        l.set(l.get() + bytes as i64);
        l.get()
    });
    PEAK.with(|p| p.set(p.get().max(live)));
}

// SAFETY: every call is forwarded to `System` unchanged; the counters are const-initialised
// thread locals without destructors, so touching them never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    #[expect(
        clippy::cast_possible_wrap,
        reason = "a Layout size is at most isize::MAX"
    )]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        FREES.with(|f| f.set(f.get() + 1));
        LIVE.with(|l| l.set(l.get() - layout.size() as i64));
        unsafe { System.dealloc(ptr, layout) }
    }
    #[expect(
        clippy::cast_possible_wrap,
        reason = "a Layout size is at most isize::MAX"
    )]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // A realloc is an allocation for the count (and is counted on its own too); the bytes
        // change by the difference.
        COUNT.with(|c| c.set(c.get() + 1));
        REALLOCS.with(|r| r.set(r.get() + 1));
        let live = LIVE.with(|l| {
            l.set(l.get() + new_size as i64 - layout.size() as i64);
            l.get()
        });
        PEAK.with(|p| p.set(p.get().max(live)));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// The allocations this thread has made so far.
fn allocations() -> u64 {
    COUNT.with(Cell::get)
}

/// The frees this thread has made so far (`realloc` counts as one allocation, not a free).
fn frees() -> u64 {
    FREES.with(Cell::get)
}

/// The allocations and the frees `f` makes.
fn counted<R>(f: impl FnOnce() -> R) -> (u64, u64) {
    let (allocs, freed) = (allocations(), frees());
    let result = f();
    let counts = (allocations() - allocs, frees() - freed);
    // Dropped after the counts are read: whatever `f` returns is not part of the measured tick.
    drop(result);
    counts
}

/// What a measured step called in the allocator: `allocs` counts `alloc` and `alloc_zeroed`,
/// `reallocs` counts `realloc`, `frees` counts `dealloc`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Allocs {
    allocs: u64,
    reallocs: u64,
    frees: u64,
}

/// The `alloc` calls, the `realloc` calls and the frees `f` makes.
fn counted3<R>(f: impl FnOnce() -> R) -> Allocs {
    let (total, reallocs, freed) = (allocations(), REALLOCS.with(Cell::get), frees());
    let result = f();
    let counts = Allocs {
        allocs: allocations() - total - (REALLOCS.with(Cell::get) - reallocs),
        reallocs: REALLOCS.with(Cell::get) - reallocs,
        frees: frees() - freed,
    };
    drop(result);
    counts
}

/// The ticks that allocated or freed, as `(tick, allocs, frees)`.
fn nonzero(per_tick: &[(u64, u64)], first: u32) -> Vec<(u32, u64, u64)> {
    per_tick
        .iter()
        .enumerate()
        .filter(|(_, (a, f))| *a > 0 || *f > 0)
        .map(|(i, (a, f))| (first + u32::try_from(i).unwrap(), *a, *f))
        .collect()
}

use crate::common;
use stella_rain_core::bench_fixtures::{self, Case, time_limit_case};

/// The steady window: the horde of the benchmark's case is at its cap of 260 from tick 76 on
/// (`WARMUP_TICKS`), no wave is left to spawn, and the main character taps its weapon every other
/// tick, so combo steps, hit boxes, pushes and orbs all run. Enemy attacks and the boss's
/// start in it too; none of them may allocate. 60 ticks, a second.
const STEADY_TICKS: u32 = 60;

#[test]
fn a_steady_tick_allocates_and_frees_nothing() {
    let mut engine = time_limit_case::warmed_up();
    // One more second of warm-up: the per-tick buffers keep their high-water capacity, and with
    // the per-purpose streams (R1) the busiest tick so far (many rules firing at once, tick
    // 2,004) is the first to grow one, so it now falls in the warm-up.
    let from = time_limit_case::WARMUP_TICKS + 1;
    for tick in from..from + STEADY_TICKS {
        engine.step(time_limit_case::input(tick));
    }
    let from = from + STEADY_TICKS;
    let mut per_tick = Vec::with_capacity(STEADY_TICKS as usize);
    for tick in from..from + STEADY_TICKS {
        per_tick.push(counted(|| engine.step(time_limit_case::input(tick))));
    }
    let at = nonzero(&per_tick, from);
    assert!(
        at.is_empty(),
        "allocations and frees in {STEADY_TICKS} steady ticks, (tick, allocs, frees): {at:?}"
    );
}

/// The tick the bench counts (`benches/time_limit_tick.rs`), built by the bench's own fixture:
/// it must not call the allocator, or its instruction count depends on the heap's layout.
#[test]
fn the_bench_tick_allocates_and_frees_nothing() {
    let Case { mut engine, input } = bench_fixtures::time_limit_tick();
    let (allocs, freed) = counted(|| engine.step(input));
    assert_eq!(
        (allocs, freed),
        (0, 0),
        "the bench's measured tick: (allocs, frees)"
    );
}

const fn pin(allocs: u64, reallocs: u64, frees: u64) -> Allocs {
    Allocs {
        allocs,
        reallocs,
        frees,
    }
}

/// The allocator calls of the one tick each bench measures (`benches/`), by case. These pin
/// today's steady-state allocations at the caps; T1b drives them to 0. A count that falls is
/// re-pinned lower in the same PR; a count that rises is a regression to fix, never a re-pin
/// without the PR saying why. DIR1 (the cases at the cap of 260): `worst_tick` 1506 -> 786
/// allocs; `worst_minute_peak` 3, 3, 3 -> 3, 2, 4, where the extra free drops an event with a
/// string that the earlier tick made (the run differs; no buffer grows). The cases are built by `bench_fixtures`, outside the
/// count.
const BENCH_TICK_ALLOCS: [(&str, Allocs); 5] = [
    ("worst_tick", pin(786, 0, 3)),
    ("hit_tick", pin(0, 0, 0)),
    ("hit_tick_spread", pin(0, 0, 0)),
    ("time_limit_tick", pin(0, 0, 0)),
    ("worst_minute_peak", pin(0, 0, 0)),
];

fn pinned(case: &str) -> Allocs {
    BENCH_TICK_ALLOCS
        .iter()
        .find(|(name, _)| *name == case)
        .unwrap()
        .1
}

fn assert_tick_pinned(case: &str, measured: Allocs) {
    assert_eq!(measured, pinned(case), "{case}: (allocs, reallocs, frees)");
}

fn measure_case(case: &str, build: fn() -> Case) {
    let Case { mut engine, input } = build();
    assert_tick_pinned(case, counted3(|| engine.step(input)));
}

#[test]
fn the_worst_tick_allocates_as_pinned() {
    measure_case("worst_tick", bench_fixtures::worst_tick);
}

#[test]
fn the_hit_tick_allocates_as_pinned() {
    measure_case("hit_tick", bench_fixtures::hit_tick);
}

#[test]
fn the_hit_tick_spread_allocates_as_pinned() {
    measure_case("hit_tick_spread", bench_fixtures::hit_tick_spread);
}

#[test]
fn the_time_limit_tick_allocates_as_pinned() {
    measure_case("time_limit_tick", bench_fixtures::time_limit_tick);
}

/// `benches/worst_minute_peak.rs` measures `run.play(1)`.
#[test]
fn the_worst_minute_peak_tick_allocates_as_pinned() {
    let mut run = bench_fixtures::worst_minute_peak();
    assert_tick_pinned("worst_minute_peak", counted3(|| run.play(1)));
}

/// A crowd that climbs to the cap and then stays there allocates nothing after load (DIR1,
/// ADR-SIM-7 decision 9): every per-enemy buffer is reserved to the run's cap when the engine
/// is built, so no tick of the climb grows one.
#[test]
fn a_run_climbing_to_the_cap_allocates_nothing_after_load() {
    use crate::common::*;
    use serde_json::json;
    // Three waves of 100 that spawn one enemy a tick from tick 1 against a cap of 260, standing
    // in the main character's line, which hits them on every other tick.
    let waves: Vec<_> = (0..3)
        .map(|_| {
            wave(
                1,
                100,
                1,
                enemy(5_000, hold(START_X, START_Y - 40_000), json!([])),
                START_X,
                START_Y - 40_000,
            )
        })
        .collect();
    let mut e = engine(&stage_full(18_000, Some(far_boss()), waves, vec![]));
    let mut ticks = Vec::new();
    let mut most = 0;
    for tick in 1..=600 {
        let input = Input {
            main: tick % 2 == 0,
            ..Input::default()
        };
        ticks.push(counted3(|| e.step(input)));
        most = most.max(e.load().enemies);
    }
    let grew: Vec<_> = ticks
        .iter()
        .enumerate()
        .filter(|(_, c)| **c != pin(0, 0, 0))
        .map(|(i, c)| (i + 1, *c))
        .collect();
    assert!(
        grew.is_empty(),
        "ticks that called the allocator, (tick, counts): {grew:?}"
    );
    assert_eq!(most, e.caps().enemies, "the crowd climbed to the cap");
}

/// The `time_limit` bench case from load to a second past its warm-up: the horde climbs to its
/// cap and holds it, with every kind of enemy, the boss and the Main weapon, and no tick calls
/// the allocator (DIR1).
#[test]
fn a_horde_at_the_cap_allocates_nothing_after_load() {
    let mut e = Engine::new_with(&time_limit_case::stage(), &crate::common::design_content())
        .expect("the run starts");
    let mut grew = Vec::new();
    for tick in 1..=time_limit_case::WARMUP_TICKS + STEADY_TICKS {
        let counts = counted3(|| e.step(time_limit_case::input(tick)));
        if counts != pin(0, 0, 0) {
            grew.push((tick, counts));
        }
    }
    assert!(
        grew.is_empty(),
        "ticks that called the allocator, (tick, counts): {grew:?}"
    );
}

/// A clone keeps the room the run reserved at load (DIR1): taken while the director is holding
/// waves, it follows the original and its first ticks call the allocator no more than the
/// original's.
#[test]
fn a_clone_of_a_run_with_waves_held_allocates_nothing() {
    use serde_json::Value;
    use stella_rain_core::engine::Outcome;
    use stella_rain_core::run_setup::RunSetup;
    use stella_rain_core::time_limit::TimeLimit;
    let mut file: Value = serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    file["crowd_target"] = 20.into();
    for wave in file["waves"].as_array_mut().unwrap() {
        wave["at_tick"] = 1.into();
        wave["every_ticks"] = 1.into();
    }
    let mode = TimeLimit::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap();
    let content = crate::common::design_content().with_time_limit(mode);
    let setup = RunSetup {
        mode: "time_limit".into(),
        seed: 1,
        ..RunSetup::default()
    };
    let mut e = Engine::start_with(&setup, &content).unwrap();
    let mut held = 0;
    for _ in 0..300 {
        e.step(Input::default());
        held += e.load().held_spawns;
    }
    assert!(held > 0, "no wave was held");
    assert_eq!(e.outcome(), Outcome::Running);
    let mut copy = e.clone();
    for tick in 0..60 {
        let (original, clone) = (
            counted3(|| e.step(Input::default())),
            counted3(|| copy.step(Input::default())),
        );
        assert_eq!(clone, pin(0, 0, 0), "the clone's tick {tick}: {original:?}");
        assert_eq!(copy.state_hash(), e.state_hash());
    }
}

/// The most memory the engine may hold at once in the worst minute of a run (DS22 item 3).
const PEAK_HEAP_LIMIT: i64 = 16 * 1024 * 1024;
const MINUTE_TICKS: usize = 3600;

/// The bytes this thread holds now, and the most it has held since `reset_peak`.
pub(crate) fn live_bytes() -> i64 {
    LIVE.with(Cell::get)
}

pub(crate) fn reset_peak() {
    PEAK.with(|p| p.set(LIVE.with(Cell::get)));
}

pub(crate) fn peak_bytes() -> i64 {
    PEAK.with(Cell::get)
}

#[test]
fn peak_heap_of_the_worst_minute_is_under_16_mib() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/current/runs/time_limit");
    let replay = RunReplay::from_bytes(&std::fs::read(dir.join("run.replay")).unwrap()).unwrap();
    let commands = replay.unpack_commands().unwrap();
    let inputs = replay.unpack_inputs(u32::MAX).unwrap();
    let start = live_bytes();
    let mut engine = Engine::start_with(&replay.setup, RunContent::bundled()).unwrap();
    let mut next = commands.iter().peekable();
    // The peak of each minute of the run, in bytes held beyond what the test held before.
    let mut minutes = Vec::new();
    reset_peak();
    for (i, input) in inputs.into_iter().enumerate() {
        while let Some(c) = next.next_if(|c| c.tick == engine.tick()) {
            match c.kind {
                CommandKind::Choose => engine.choose(c.index).unwrap(),
            }
        }
        engine.step(input);
        if (i + 1) % MINUTE_TICKS == 0 {
            minutes.push(peak_bytes() - start);
            reset_peak();
        }
    }
    minutes.push(peak_bytes() - start);
    let worst = minutes.iter().copied().max().unwrap();
    eprintln!("peak heap per minute of the time_limit run: {minutes:?} bytes; worst {worst}");
    assert!(
        worst < PEAK_HEAP_LIMIT,
        "peak heap {worst} bytes in the worst minute, limit {PEAK_HEAP_LIMIT}: {minutes:?}"
    );
}

#[test]
fn peak_heap_of_a_minute_of_the_saturated_horde_is_under_16_mib() {
    let start = live_bytes();
    let mut engine = time_limit_case::warmed_up();
    let from = time_limit_case::WARMUP_TICKS + 1;
    reset_peak();
    for tick in from..from + u32::try_from(MINUTE_TICKS).unwrap() {
        engine.step(time_limit_case::input(tick));
    }
    let peak = peak_bytes() - start;
    eprintln!("peak heap of a minute of the saturated horde: {peak} bytes");
    assert!(
        peak < PEAK_HEAP_LIMIT,
        "peak heap {peak} bytes, limit {PEAK_HEAP_LIMIT}"
    );
}

/// Enemies whose swings and zones hit the main character all the time, on cooldowns that overlap
/// differently from tick to tick (BR1a-0): a run or a zone that hits for the first time must not
/// allocate once the pool of hit lists has settled.
#[test]
fn a_steady_tick_allocates_and_frees_nothing_while_melee_and_zones_hit_the_main_character() {
    use crate::common::*;
    use serde_json::json;
    let attack = json!({ "inline": [
        { "kind": "wind_up", "ticks": 1, "damage": 100, "spawn": [
            { "type": "zone", "shape": { "type": "circle", "radius": 1024 }, "anchor": "target",
              "wind_up_ticks": 3, "active_ticks": 3 }] },
        { "kind": "active", "ticks": 2, "damage": 100,
          "hit_box": { "type": "circle", "radius": 7680 } }] });
    let waves: Vec<_> = [7u32, 11, 13, 17, 19, 23, 29, 31]
        .iter()
        .enumerate()
        .map(|(i, cooldown)| {
            let (x, y) = (
                START_X + (i32::try_from(i).unwrap() - 4) * 600,
                START_Y - 1024,
            );
            let rules = json!([{ "when": always(), "target": "player", "cooldown_ticks": cooldown,
                "do": { "type": "attack", "attack": attack } }]);
            wave(1, 1, 0, enemy(1000, hold(x, y), rules), x, y)
        })
        .collect();
    let mut stage = quiet(stage_full(18_000, Some(far_boss()), waves, vec![]));
    stage.player.hp = 99_900;
    let mut e = engine(&stage);
    for _ in 0..600 {
        e.step(idle());
    }
    let mut per_tick = Vec::new();
    for _ in 0..600 {
        per_tick.push(counted(|| e.step(idle())));
    }
    let at = nonzero(&per_tick, 0);
    assert!(
        at.is_empty(),
        "allocations and frees in 600 steady ticks, (tick, allocs, frees): {at:?}"
    );
}

/// `Engine::places` on one reused buffer, beside `step`, in the steady window: no allocation
/// and no free once the buffer has grown in the warm-up.
#[test]
fn places_allocates_nothing_in_a_steady_tick() {
    let mut engine = time_limit_case::warmed_up();
    let mut places = stella_rain_core::engine::Places::default();
    let from = time_limit_case::WARMUP_TICKS + 1;
    for tick in from..from + 2 * STEADY_TICKS {
        engine.step(time_limit_case::input(tick));
        engine.places(&mut places);
    }
    let from = from + 2 * STEADY_TICKS;
    let mut per_tick = Vec::with_capacity(STEADY_TICKS as usize);
    for tick in from..from + STEADY_TICKS {
        per_tick.push(counted(|| {
            engine.step(time_limit_case::input(tick));
            engine.places(&mut places);
        }));
    }
    let at = nonzero(&per_tick, from);
    assert!(
        at.is_empty(),
        "allocations and frees in {STEADY_TICKS} steady ticks of step plus places, (tick, allocs, frees): {at:?}"
    );
    assert!(!places.entities.is_empty());
    assert_eq!(places.player, engine.snapshot().player.at);
}

/// At the cap of 260 enemies the first `places` may grow the buffer; the second may not.
#[test]
fn places_at_the_cap_allocates_nothing_the_second_time() {
    let engine = bench_fixtures::worst_case::warmed_up();
    let mut places = stella_rain_core::engine::Places::default();
    engine.places(&mut places);
    assert!(
        places.entities.len() >= 260,
        "{} entities",
        places.entities.len()
    );
    assert_eq!(counted(|| engine.places(&mut places)), (0, 0));
}

/// `Engine::tally` beside `step` in the steady window: the counts are kept by the observers and
/// the read is a borrowed view, so neither allocates nor frees.
#[test]
fn tally_allocates_nothing_in_a_steady_tick() {
    let mut engine = time_limit_case::warmed_up();
    let from = time_limit_case::WARMUP_TICKS + 1;
    for tick in from..from + 2 * STEADY_TICKS {
        engine.step(time_limit_case::input(tick));
    }
    let from = from + 2 * STEADY_TICKS;
    let mut per_tick = Vec::with_capacity(STEADY_TICKS as usize);
    for tick in from..from + STEADY_TICKS {
        per_tick.push(counted(|| {
            engine.step(time_limit_case::input(tick));
            let t = engine.tally();
            std::hint::black_box((
                t.kills(),
                t.taken().count(),
                t.named().len(),
                t.tiers().count(),
            ));
        }));
    }
    let at = nonzero(&per_tick, from);
    assert!(
        at.is_empty(),
        "allocations and frees in {STEADY_TICKS} steady ticks of step plus tally, (tick, allocs, frees): {at:?}"
    );
}
