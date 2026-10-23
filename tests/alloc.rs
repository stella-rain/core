//! A steady tick allocates nothing (T1b): the per-tick buffers are reused and the per-enemy lists
//! are inline. The counting allocator is installed in `integration.rs` (a global allocator must
//! be in the crate root); its counters are per thread, so parallel tests do not disturb each
//! other.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use stella_rain_core::engine::Engine;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{CommandKind, RunReplay};

thread_local! {
    static COUNT: Cell<u64> = const { Cell::new(0) };
    static LIVE: Cell<i64> = const { Cell::new(0) };
    static PEAK: Cell<i64> = const { Cell::new(0) };
}

/// The system allocator, counting the allocations and the bytes held by the current thread.
pub struct Counting;

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
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.with(|l| l.set(l.get() - layout.size() as i64));
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // A realloc is an allocation for the count; the bytes change by the difference.
        COUNT.with(|c| c.set(c.get() + 1));
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

// The benchmark's own module, as in `tests/time_limit_case.rs`.
#[path = "../benches/time_limit_case/mod.rs"]
#[allow(clippy::duplicate_mod, clippy::module_inception)]
mod time_limit_case;

/// The steady window: the horde of the benchmark's case is at its cap of 500 from tick 1,962 on
/// (`WARMUP_TICKS`), no wave is left to spawn, and the main character taps its weapon every other
/// tick, so combo steps, hit boxes, pushes and bullets all run. Enemy attacks and the boss's
/// start in it too; none of them may allocate. 60 ticks, a second.
const STEADY_TICKS: u32 = 60;

#[test]
fn a_steady_tick_allocates_nothing() {
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
        let before = allocations();
        engine.step(time_limit_case::input(tick));
        per_tick.push(allocations() - before);
    }
    let total: u64 = per_tick.iter().sum();
    assert_eq!(
        total, 0,
        "allocations in {STEADY_TICKS} steady ticks, per tick: {per_tick:?}"
    );
}

/// The most memory the engine may hold at once in the worst minute of a run (DS22 item 3).
const PEAK_HEAP_LIMIT: i64 = 16 * 1024 * 1024;
const MINUTE_TICKS: usize = 3600;

/// The bytes this thread holds now, and the most it has held since `reset_peak`.
fn live_bytes() -> i64 {
    LIVE.with(Cell::get)
}

fn reset_peak() {
    PEAK.with(|p| p.set(LIVE.with(Cell::get)));
}

fn peak_bytes() -> i64 {
    PEAK.with(Cell::get)
}

#[test]
fn peak_heap_of_the_worst_minute_is_under_16_mib() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/runs/v0/time_limit");
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
                CommandKind::Spend => engine.spend(c.index).unwrap(),
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
    for tick in from..from + MINUTE_TICKS as u32 {
        engine.step(time_limit_case::input(tick));
    }
    let peak = peak_bytes() - start;
    eprintln!("peak heap of a minute of the saturated horde: {peak} bytes");
    assert!(
        peak < PEAK_HEAP_LIMIT,
        "peak heap {peak} bytes, limit {PEAK_HEAP_LIMIT}"
    );
}
