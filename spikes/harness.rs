//! A rollback measurement harness (throwaway branch, never merged): the wall-clock of cloning
//! and restoring the engine plus W re-simulated ticks, for 1 to 3 players. Three players are a
//! proxy: N independent engines stepped in lockstep, because `Engine::step` takes one player's
//! input. Included by `src/bin/spike_rollback.rs` and by `tests/spike_rollback.rs`.
#![allow(dead_code, clippy::disallowed_methods)]

use std::hint::black_box;
use std::io::Write;
use std::time::{Duration, Instant};

use stella_rain_core::bench_fixtures::worst_minute_case::{
    OFFER_CHOICE, input_at, worst_minute_peak,
};
use stella_rain_core::bench_fixtures::{time_limit_tick, worst_tick};
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::pack::{ContentFiles, Pack};
use stella_rain_core::run_replay::RunRecorder;
use stella_rain_core::run_setup::RunSetup;

/// The re-simulated windows, in ticks.
pub const WINDOWS: [u32; 5] = [1, 2, 4, 6, 8];
/// The player counts of the proxy: that many engines in one measured interval.
pub const PLAYERS: [u32; 3] = [1, 2, 3];
/// The scenarios, in the order of the CSV.
pub const SCENARIOS: [&str; 3] = ["worst_tick", "worst_minute_peak", "time_limit_tick"];
/// The scenario the savepoint is measured on, and the tick it is taken at.
pub const SAVEPOINT_SCENARIO: &str = "time_limit_run_10800";
pub const SAVEPOINT_TICK: u32 = 10_800;
pub const HEADER: &str = "device,scenario,players_proxy,window,op,p50,p95,p99,max,samples";

/// The rows per scenario and player count: clone, restore, step and one resim per window.
pub const OPS_PER_CELL: usize = 3 + WINDOWS.len();
/// Every row of a full run: the engine ops, then the two savepoint ops.
pub const EXPECTED_ROWS: usize = SCENARIOS.len() * PLAYERS.len() * OPS_PER_CELL + 2;

/// The state hashes of one rollback cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cycle {
    pub at_clone: u64,
    pub straight: u64,
    pub restored: u64,
    pub resimulated: u64,
}

/// Clones `engine`, steps it `w` ticks with `step`, restores the clone and steps the same `w`
/// ticks again. `engine` ends at the re-simulated state.
pub fn rollback_cycle(engine: &mut Engine, step: &mut dyn FnMut(&mut Engine), w: u32) -> Cycle {
    let saved = engine.clone();
    let at_clone = engine.state_hash();
    for _ in 0..w {
        step(engine);
    }
    let straight = engine.state_hash();
    *engine = saved;
    let restored = engine.state_hash();
    for _ in 0..w {
        step(engine);
    }
    Cycle {
        at_clone,
        straight,
        restored,
        resimulated: engine.state_hash(),
    }
}

/// How a scenario steps a tick: the fixture's own input every tick, or the dense minute's.
#[derive(Clone, Copy)]
enum Drive {
    Fixed(Input),
    Minute,
}

impl Drive {
    fn step(self, engine: &mut Engine) {
        match self {
            Drive::Fixed(input) => engine.step(input),
            Drive::Minute => {
                while engine.offer_open() {
                    engine.choose(OFFER_CHOICE).expect("the first option");
                }
                engine.step(input_at(engine.tick()));
            }
        }
    }
}

fn scenario(name: &str) -> (Engine, Drive) {
    match name {
        "worst_tick" => {
            let case = worst_tick();
            (case.engine, Drive::Fixed(case.input))
        }
        "time_limit_tick" => {
            let case = time_limit_tick();
            (case.engine, Drive::Fixed(case.input))
        }
        _ => (worst_minute_peak().engine, Drive::Minute),
    }
}

/// How many samples a cell takes and how many of them are dropped as warm-up.
#[derive(Clone, Copy)]
pub struct Config {
    pub samples: usize,
    pub warm_up: usize,
}

fn micros(d: Duration) -> u64 {
    u64::try_from((d.as_nanos() + 500) / 1000).expect("a short interval")
}

fn percentile(sorted: &[u64], q_percent: usize) -> u64 {
    let rank = (q_percent * sorted.len()).div_ceil(100).max(1);
    sorted[rank - 1]
}

/// Runs `sample` `warm_up + samples` times, drops the first `warm_up` and writes the row.
fn measure(
    out: &mut dyn Write,
    config: Config,
    device: &str,
    cell: (&str, u32, u32, &str),
    mut sample: impl FnMut() -> Duration,
) -> std::io::Result<()> {
    let mut times: Vec<u64> = (0..config.warm_up + config.samples)
        .map(|_| micros(sample()))
        .skip(config.warm_up)
        .collect();
    times.sort_unstable();
    let (name, players, window, op) = cell;
    writeln!(
        out,
        "{device},{name},{players},{window},{op},{},{},{},{},{}",
        percentile(&times, 50),
        percentile(&times, 95),
        percentile(&times, 99),
        times[times.len() - 1],
        times.len()
    )
}

fn time<T>(f: impl FnOnce() -> T) -> Duration {
    let start = Instant::now();
    black_box(f());
    start.elapsed()
}

fn copies(base: &Engine, players: u32) -> Vec<Engine> {
    (0..players).map(|_| base.clone()).collect()
}

fn run_scenario(
    out: &mut dyn Write,
    config: Config,
    device: &str,
    name: &str,
) -> std::io::Result<()> {
    let (base, drive) = scenario(name);
    for players in PLAYERS {
        // Cloning the engines and dropping the copies: the cost of a saved state per frame.
        let cell = |window, op| (name, players, window, op);
        measure(out, config, device, cell(0, "clone"), || {
            time(|| copies(&base, players))
        })?;
        // The dirty engines are replaced by the saved ones; their drop is in the interval.
        let mut restore = || {
            let saved = copies(&base, players);
            let mut live = copies(&base, players);
            live.iter_mut().for_each(|e| drive.step(e));
            time(move || {
                drop(black_box(live));
                saved
            })
        };
        measure(out, config, device, cell(0, "restore"), &mut restore)?;
        measure(out, config, device, cell(1, "step"), || {
            let mut live = copies(&base, players);
            time(move || {
                live.iter_mut().for_each(|e| drive.step(e));
                live
            })
        })?;
        for window in WINDOWS {
            measure(out, config, device, cell(window, "resim"), || {
                let saved = copies(&base, players);
                let mut live = copies(&base, players);
                for _ in 0..window {
                    live.iter_mut().for_each(|e| drive.step(e));
                }
                time(move || {
                    let mut live = saved;
                    for _ in 0..window {
                        live.iter_mut().for_each(|e| drive.step(e));
                    }
                    live
                })
            })?;
        }
    }
    Ok(())
}

/// The bundled pack with the fixtures' enemy damage of 100, which the trial's main character
/// survives to tick 10,800 (the bundled damage ends the run at tick 724).
fn survivable_pack() -> Pack {
    let mut files = ContentFiles::bundled().clone();
    let mut stats: serde_json::Value =
        serde_json::from_slice(ContentFiles::bundled_file("run_stats")).expect("run_stats");
    stats["enemy_attack_damage"] = serde_json::json!(100);
    assert!(files.insert("run_stats", stats.to_string().into_bytes()));
    Pack::from_bytes(&files.export().expect("exports")).expect("a pack")
}

/// The `time_limit` run of `pack` `ticks` in, as a recorder.
fn pack_run(pack: &Pack, ticks: u32) -> RunRecorder {
    let setup = RunSetup {
        mode: "time_limit".into(),
        seed: 1,
        ..RunSetup::default()
    };
    let mut rec = RunRecorder::start_with_pack(&setup, pack).expect("the run starts");
    while rec.engine().tick() < ticks {
        while rec.engine().offer_open() {
            rec.choose(OFFER_CHOICE).expect("the first option");
        }
        rec.step(input_at(rec.engine().tick())).expect("a tick");
    }
    rec
}

/// A run `ticks` in, saved as a savepoint and restored: the state hash before saving and of
/// the restored engine, and the savepoint's size in bytes.
pub fn savepoint_round_trip(ticks: u32) -> (u64, u64, usize) {
    let pack = survivable_pack();
    let rec = pack_run(&pack, ticks);
    let bytes = rec.save();
    let restored =
        RunRecorder::resume_from_savepoint(&bytes, &rec.replay(), &pack).expect("restores");
    (
        rec.engine().state_hash(),
        restored.engine().state_hash(),
        bytes.len(),
    )
}

fn run_savepoint(out: &mut dyn Write, config: Config, device: &str) -> std::io::Result<()> {
    let pack = survivable_pack();
    let rec = pack_run(&pack, SAVEPOINT_TICK);
    let prefix = rec.replay();
    let bytes = rec.save();
    eprintln!(
        "# savepoint at tick {SAVEPOINT_TICK}: {} bytes",
        bytes.len()
    );
    let cell = |op| (SAVEPOINT_SCENARIO, 1, 0, op);
    measure(out, config, device, cell("savepoint_save"), || {
        time(|| rec.save())
    })?;
    measure(out, config, device, cell("savepoint_restore"), || {
        time(|| RunRecorder::resume_from_savepoint(&bytes, &prefix, &pack))
    })
}

/// Writes the CSV of every cell of the grid.
pub fn run(out: &mut dyn Write, config: Config, device: &str) -> std::io::Result<()> {
    writeln!(out, "{HEADER}")?;
    for name in SCENARIOS {
        eprintln!("# scenario {name}");
        run_scenario(out, config, device, name)?;
    }
    eprintln!("# scenario {SAVEPOINT_SCENARIO}");
    run_savepoint(out, config, device)
}
