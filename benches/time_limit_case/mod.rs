//! The heaviest tick of a `time_limit` run (core#212 step d, ADR-021, ADR-058). Shared by
//! `benches/time_limit_tick.rs`, which counts its instructions, and `tests/time_limit_case.rs`,
//! which checks that the measured tick has the load it claims.
#![allow(dead_code)]

use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::stage::{Stage, Wave};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate;
use stella_rain_core::validate::limits::{MAX_AGENT_HP, MAX_PLAYER_HP};

/// Ticks before the measured tick: the 500 enemies have all arrived and shot for long enough
/// that the bullets alive are about 3,950 (`MAX_BULLETS_ALIVE` is 4,000), and the measured tick
/// is one on which the main character's Main weapon hits (`tests/time_limit_case.rs`).
pub const WARMUP_TICKS: u32 = 1_517;

/// The longest stage there is: 5 minutes. The mode's run is longer, which only the mode may be.
const STAGE_LENGTH_TICKS: u32 = 18_000;

/// The most enemies a wave of a stage spawns (`MAX_WAVE_COUNT`); the mode file's own waves are
/// larger than a stage allows, so the bench splits them.
const WAVE_COUNT: u16 = 100;

/// The setup of the run: the mode's weapons, as the app's first run uses them.
pub fn setup() -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        main: "blade".into(),
        sub: "maul".into(),
        seed: 1,
        star_pool: None,
        extra_rerolls: 0,
    }
}

/// The `time_limit` run at its heaviest, as a stage (`Engine::new_with`).
///
/// The map, the main character, its weapons and the enemies are the bundled mode's
/// (`run_setup::stage_with`): the same two kinds of enemy in the mode's ratio of 3 to 2 (its
/// schedule spawns 60 and 40 a minute, 100 in all), spawning round the main character at the
/// mode's distances. Three differences, all so that the run reaches the load and stays in it:
///
/// - The enemies arrive at once, five waves of 100 that spawn one enemy each a tick, so that
///   the cap of `MAX_ENEMIES_ALIVE` is reached in 100 ticks, not in the 5 minutes the schedule
///   takes. After that the schedule's spawns are refused, as they are in a run that holds the
///   cap, so they are left out.
/// - Each enemy has the most hit points, so that the main character's attacks hit and none
///   dies: the 500 stay alive and every hit is carried out.
/// - The main character has the most hit points a stage allows, and a boss stands where the
///   trial's golem does from the start, in place of a boss that comes at a boss time
///   (`Engine::start_time_limit` skips the stage's caps, which the schedule's 1,199 enemies
///   exceed, and gives the main character the placeholder's 5 hit points, which 500 enemies
///   shoot down in a few hundred ticks).
pub fn stage() -> Stage {
    let mode = TimeLimit::bundled();
    let mut stage = run_setup::stage_with(&setup(), mode).expect("the run's stage");
    let trial = validate::parse_and_validate(run_setup::mode_stage("trial").expect("the trial"))
        .expect("the trial stage");
    stage.boss = trial.boss;
    stage.player.hp = MAX_PLAYER_HP;
    stage.length_ticks = STAGE_LENGTH_TICKS;
    let kinds: Vec<Wave> = mode.waves().to_vec();
    let share = [3usize, 2];
    stage.waves = kinds
        .iter()
        .zip(share)
        .flat_map(|(kind, waves)| {
            (0..waves).map(move |_| {
                let mut wave = kind.clone();
                wave.at_tick = 1;
                wave.every_ticks = 1;
                wave.count = WAVE_COUNT;
                wave.enemy.hp = MAX_AGENT_HP;
                wave.expertise = None;
                wave
            })
        })
        .collect();
    stage
}

/// The input of tick `tick` (from 1): the main character stays where it starts and taps its
/// Main weapon every other tick, so its combo steps land on the enemies round it all the time.
pub fn input(tick: u32) -> Input {
    Input {
        main: tick % 2 == 0,
        ..Input::default()
    }
}

/// An engine `WARMUP_TICKS` ticks into the run: the next `step` is the measured tick.
pub fn warmed_up() -> Engine {
    let mut engine = Engine::new_with(&stage(), RunContent::bundled()).expect("the run starts");
    for tick in 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}
