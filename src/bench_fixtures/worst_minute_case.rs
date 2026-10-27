//! The dense minute of the `time_limit` mode (BR1e-2, core#368, DS49, DS50): the mode's own
//! rules at their caps, not the light minute of the corpus run. A one-tick bench misses wave
//! spawns, elite minutes, the boss's arrival and death bursts, so `benches/worst_minute.rs`
//! counts the 3,600 ticks of `MINUTE_START..MINUTE_START + MINUTE_TICKS` and
//! `benches/worst_minute_peak.rs` the heaviest tick of them.
//!
//! The run is the bundled mode with a schedule that keeps the crowd at its target (200; the
//! spawn director holds the waves there, and the elites come in the room up to the cap of 260,
//! DIR1): every wave starts
//! at tick 1 and spawns at the rate that spends its enemies over `SPAWN_SPAN_TICKS` (the
//! mode's 4,143 spawns, 1.4 times each wave's count, in four minutes, not twenty), a mid-boss comes inside the
//! minute (`BOSS_TICK`) and the elite cap is `MAX_ELITES_ALIVE`. The elite minutes (the mode's own: every
//! whole minute that is no boss time, so ticks 3,600 and 7,200 before the minute) bring its elites. The main character taps its Main weapon every other tick
//! and the choices of the level-ups it opens take the first option. The seed is `SEED`. The
//! `enemy_attack_damage` is the 100 of the other cases, which the 50,000 hit points of the
//! trial's main character survive. Reaching the minute is not counted; where the heaviest tick
//! is (`worst_minute_window`, by the unweighted sum of the five fields of `Load`, the first
//! tick winning a tie) is pinned in `tests/worst_minute.rs`, and the state hashes in
//! `tests/bench_fixture_pins.rs`.

use crate::engine::{Engine, Load};
use crate::input::Input;
use crate::pack::ContentFiles;
use crate::run_content::RunContent;
use crate::run_setup::RunSetup;
use crate::run_stats::RunStats;
use crate::time_limit::TimeLimit;
use crate::validate::limits::MAX_ELITES_ALIVE;

/// A minute: 60 s at 60 ticks a second.
pub const MINUTE_TICKS: u32 = 3_600;

/// The ticks played before the minute: the crowd is at its target and two elite minutes have come.
pub const MINUTE_START: u32 = 7_300;

/// The tick the mid-boss arrives at, inside the minute.
const BOSS_TICK: u32 = 9_000;

/// The ticks over which each wave spends its enemies.
const SPAWN_SPAN_TICKS: u32 = 14_000;

/// The run's seed (a placeholder: any that reaches the caps).
const SEED: u64 = 1;

/// The bundled mode with the dense schedule (see the module).
fn mode() -> TimeLimit {
    let mut file: serde_json::Value =
        serde_json::from_slice(ContentFiles::bundled_file("time_limit")).expect("the bundled mode");
    for wave in file["waves"].as_array_mut().expect("waves") {
        let count = wave["count"].as_u64().expect("a count") * 14 / 10;
        wave["count"] = count.into();
        wave["at_tick"] = 1.into();
        wave["every_ticks"] = (u64::from(SPAWN_SPAN_TICKS) / count).max(1).into();
    }
    file["boss_ticks"] = serde_json::json!([BOSS_TICK, file["length_ticks"]]);
    file["no_call_ticks"] = serde_json::json!([]);
    file["elite_cap"] = MAX_ELITES_ALIVE.into();
    TimeLimit::parse(file.to_string().as_bytes()).expect("the dense mode")
}

fn content() -> RunContent {
    let mut v: serde_json::Value =
        serde_json::from_slice(ContentFiles::bundled_file("run_stats")).unwrap();
    v["enemy_attack_damage"] = serde_json::json!(100);
    let stats = RunStats::parse(v.to_string().as_bytes()).unwrap();
    RunContent::new(stats, RunContent::bundled().rewards().clone()).with_time_limit(mode())
}

/// The option an open offer takes between ticks.
pub const OFFER_CHOICE: u32 = 0;

/// The input stepped at `tick` (the tick the engine is at before the step): the main weapon is
/// tapped every other tick. `Run::play` reads it, and so does the digest of
/// `tests/bench_fixture_pins.rs`, so the two cannot drift.
pub fn input_at(tick: u32) -> Input {
    Input {
        main: tick % 2 == 1,
        ..Input::default()
    }
}

/// The run, part way through: `play` takes the next ticks.
pub struct Run {
    pub engine: Engine,
}

impl Run {
    /// The run, `ticks` ticks in.
    pub fn fast_forwarded(ticks: u32) -> Run {
        let setup = RunSetup {
            mode: "time_limit".into(),
            seed: SEED,
            ..RunSetup::default()
        };
        let content = content();
        let engine = Engine::start_with(&setup, &content).expect("the run starts");
        let mut run = Run { engine };
        run.play(ticks);
        run
    }

    /// Plays the next `ticks` ticks: an open offer takes its first option between them.
    pub fn play(&mut self, ticks: u32) {
        for _ in 0..ticks {
            while self.engine.offer_open() {
                self.engine.choose(OFFER_CHOICE).expect("the first option");
            }
            self.engine.step(input_at(self.engine.tick()));
        }
    }
}

fn weight(load: Load) -> u64 {
    [
        load.enemies,
        load.tasks,
        load.zones,
        load.orbs,
        load.emitter_steps,
    ]
    .iter()
    .map(|&n| u64::from(n))
    .sum()
}

/// Scans the minute for its heaviest tick (`tests/worst_minute.rs` pins it): the minute and that tick, as numbers of ticks played before them: the minute is the
/// ticks `start..start + MINUTE_TICKS`, and the next `step` of a run `peak_tick` ticks in is the
/// heaviest tick of it.
pub fn worst_minute_window() -> (u32, u32) {
    let mut run = Run::fast_forwarded(MINUTE_START);
    let mut heaviest = (0, MINUTE_START);
    for i in 0..MINUTE_TICKS {
        run.play(1);
        let w = weight(run.engine.load());
        if w > heaviest.0 {
            heaviest = (w, MINUTE_START + i);
        }
    }
    (MINUTE_START, heaviest.1)
}

/// The run at the start of the dense minute (`benches/worst_minute.rs` plays `MINUTE_TICKS`).
pub fn worst_minute() -> Run {
    Run::fast_forwarded(MINUTE_START)
}

/// The run one tick before the heaviest tick of that minute (`benches/worst_minute_peak.rs`; the
/// tick is the one `worst_minute_window` finds).
pub fn worst_minute_peak() -> Run {
    Run::fast_forwarded(worst_minute_window().1)
}
