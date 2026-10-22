//! The heaviest tick of a `time_limit` run (core#212 step d, ADR-021, ADR-058). Shared by
//! `benches/time_limit_tick.rs`, which counts its instructions, and `tests/time_limit_case.rs`,
//! which checks that the measured tick has the load it claims.
#![allow(dead_code)]

use stella_rain_core::engine::{Engine, Load};
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::stage::Stage;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate;
use stella_rain_core::validate::limits::{MAX_AGENT_HP, MAX_PLAYER_HP};

/// Ticks before the measured tick: the 500 enemies have all arrived and shot for long enough
/// that the bullets alive are about 3,950 (`MAX_BULLETS_ALIVE` is 4,000: 3,954 on the measured
/// tick), and the measured tick is one on which the main character's Main weapon hits
/// (`tests/time_limit_case.rs`). The six roles shoot less than the bats alone did, so the
/// bullets take longer to build up than they did (tick 1,518 then, 1,957 now); the enemies
/// start to fall to the Main weapon from about tick 2,300. The Sword's chain is five A's deep (ADR-117),
/// so its tap-every-other-tick cycle is longer: the first tick from 1,957 on where a step lands a
/// hit is 1,963, so the measured tick is that one.
pub const WARMUP_TICKS: u32 = 1_962;

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
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed: 1,
        star_pool: None,
        extra_rerolls: 0,
        upgrades: Vec::new(),
        perk_set: "duelist".into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        difficulty: "normal".into(),
    }
}

/// The `time_limit` run at its heaviest, as a stage (`Engine::new_with`).
///
/// The map, the main character, its weapons and the enemies are the bundled mode's
/// (`run_setup::stage_with`): every kind of enemy the schedule spawns (the two bats and the six
/// roles, swarmer to splitter), each in the share of the 500 that its spawns are of the
/// schedule's (`shares`, 4,065 spawns in all), spawning round the main character at the mode's
/// distances. The tank follows at its `speed`, the exploder checks `near_target` and the
/// splitter checks half its hit points, as they do in the mode. Four differences, all so that
/// the run reaches the load and stays in it:
///
/// - The enemies arrive at once, in waves of at most `MAX_WAVE_COUNT` (100) that spawn one enemy
///   each a tick, so that the cap of 500 enemies alive is reached in about 100 ticks, not in the
///   20 minutes the schedule takes. After that the schedule's spawns are refused, as they are in
///   a run that holds the cap, so they are left out.
/// - Each enemy has the most hit points, so that the main character's attacks hit and none
///   dies: the 500 stay alive and every hit is carried out. So the splitter never drops below
///   half its hit points and never splits; its check still runs.
/// - The bench clears the `self_destruct` flag of every rule: the exploders burst and stay, so
///   the measured tick holds 500 enemies with the `near_target` check and the burst running. In
///   a real run an exploder that bursts goes at once and gives no credit; kept alive here, the
///   cap stays full. (The bench's rule, not a change of behaviour: app#336 Q11.)
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
    let kinds = mode.waves();
    let spawns: Vec<u32> = kinds.iter().map(|k| u32::from(k.count)).collect();
    stage.waves = kinds
        .iter()
        .zip(shares(&spawns, Load::CAPS.enemies))
        .flat_map(|(kind, share)| {
            let mut left = share;
            std::iter::from_fn(move || {
                (left > 0).then(|| {
                    let count = left.min(u32::from(WAVE_COUNT));
                    left -= count;
                    let mut wave = kind.clone();
                    wave.at_tick = 1;
                    wave.every_ticks = 1;
                    wave.count = u16::try_from(count).expect("a wave of at most WAVE_COUNT");
                    wave.enemy.hp = MAX_AGENT_HP;
                    wave.expertise = None;
                    for rule in &mut wave.enemy.rules {
                        rule.self_destruct = false;
                    }
                    wave
                })
            })
        })
        .collect();
    stage
}

/// `total` split in proportion to `weights`, by the largest remainder (the shares sum to `total`;
/// a tie goes to the earlier kind).
pub fn shares(weights: &[u32], total: u32) -> Vec<u32> {
    let sum: u64 = weights.iter().map(|&w| u64::from(w)).sum();
    let mut shares: Vec<u32> = weights
        .iter()
        .map(|&w| u32::try_from(u64::from(total) * u64::from(w) / sum).expect("at most total"))
        .collect();
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(u64::from(total) * u64::from(weights[i]) % sum));
    let missing = total - shares.iter().sum::<u32>();
    for &i in order.iter().take(missing as usize) {
        shares[i] += 1;
    }
    shares
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
