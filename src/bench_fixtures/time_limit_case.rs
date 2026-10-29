//! The heaviest tick of a `time_limit` run (core#212 step d, ADR-021, ADR-058). Shared by
//! `benches/time_limit_tick.rs`, which counts its instructions, and `tests/time_limit_case.rs`,
//! which checks that the measured tick has the load it claims.

use crate::engine::Engine;
use crate::input::Input;
use crate::pack::ContentFiles;
use crate::run_content::RunContent;
use crate::run_setup::{self, RunSetup};
use crate::run_stats::RunStats;
use crate::stage::Stage;
use crate::time_limit::TimeLimit;
use crate::validate;
use crate::validate::limits::{MAX_AGENT_HP, MAX_PLAYER_HP};
use crate::version::Versions;

/// Ticks before the measured tick: the horde has all arrived (the cap of 260 is reached on tick
/// 76) and the measured tick is one on which the main character's Main weapon hits
/// (`tests/time_limit_case.rs`) and the most attacks are in progress while the horde is at the
/// cap. Since BR1a nothing fires a projectile, so the load is zones, the crowd's rules and the
/// melee: of the ticks from 76 to 2,500 (a placeholder for how long the horde keeps its cap) on
/// which the Main weapon hits and 260 enemies stand, tick 1,027 has the most attacks running (3)
/// and then the most zones (124), under the 840 px view (tick 1,999 before it). Chosen
/// by the engine's `Load`, not by an instruction count: valgrind was not at hand (BR1a). The
/// Sword's chain is five A's deep (ADR-117), so its tap-every-other-tick cycle is long.
pub const WARMUP_TICKS: u32 = 1_026;

/// The longest stage there is: 5 minutes. The mode's run is longer, which only the mode may be.
const STAGE_LENGTH_TICKS: u32 = 18_000;

/// The most enemies a wave of a stage spawns (`MAX_WAVE_COUNT`); the mode file's own waves are
/// larger than a stage allows, so the bench splits them.
const WAVE_COUNT: u16 = 100;

/// The setup of the run: the mode's weapons, as the app's first run uses them.
pub fn setup() -> RunSetup {
    RunSetup {
        mode: "time_limit".into(),
        seed: 1,
        ..RunSetup::default()
    }
}

/// The `time_limit` run at its heaviest, as a stage (`Engine::new_with`).
///
/// The map, the main character, its weapons and the enemies are the bundled mode's
/// (`run_setup::stage_with`): every kind of enemy the schedule spawns (the two bats and the six
/// roles, swarmer to splitter), each in the share of the cap that its spawns are of the
/// schedule's (`shares`, 4,065 spawns in all), spawning round the main character at the mode's
/// distances. The tank follows at its `speed`, the exploder checks `near_target` and the
/// splitter checks half its hit points, as they do in the mode. Four differences, all so that
/// the run reaches the load and stays in it:
///
/// - The enemies arrive at once, in waves of at most `MAX_WAVE_COUNT` (100) that spawn one enemy
///   each a tick, so that the cap on enemies alive (260) is reached in about 100 ticks, not in the
///   20 minutes the schedule takes. After that the schedule's spawns are refused, as they are in
///   a run that holds the cap, so they are left out.
/// - Each enemy has the most hit points, so that the main character's attacks hit and none
///   dies: the crowd stays alive and every hit is carried out. So the splitter never drops below
///   half its hit points and never splits; its check still runs.
/// - The bench clears the `self_destruct` flag of every rule: the exploders burst and stay, so
///   the measured tick holds the cap with the `near_target` check and the burst running. In
///   a real run an exploder that bursts goes at once and gives no credit; kept alive here, the
///   cap stays full. (The bench's rule, not a change of behaviour: app#336 Q11.)
/// - The main character has the most hit points a stage allows, and a boss stands where the
///   trial's golem does from the start, in place of a boss that comes at a boss time
///   (`Engine::start_time_limit` skips the stage's caps, which the schedule's 1,199 enemies
///   exceed, and gives the main character the placeholder's 5 hit points, which the cap of enemies
///   shoot down in a few hundred ticks).
pub fn stage() -> Stage {
    let mode = TimeLimit::bundled();
    let mut stage = run_setup::stage_with(&setup(), mode).expect("the run's stage");
    let trial = validate::parse_and_validate(run_setup::mode_stage("trial").expect("the trial"))
        .expect("the trial stage");
    stage.boss = trial.boss;
    stage.player.hp = MAX_PLAYER_HP;
    stage.length_ticks = STAGE_LENGTH_TICKS;
    let rules = RunContent::bundled().rules();
    let cap = rules.enemy_cap_for(stage.crowd_target.unwrap_or_else(|| rules.crowd_target()));
    let kinds = mode.waves();
    let spawns: Vec<u32> = kinds.iter().map(|k| u32::from(k.count)).collect();
    stage.waves = kinds
        .iter()
        .zip(shares(&spawns, cap))
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

/// The bundled content with an enemy's attack at 100 stored, as the case was measured: the
/// scripted input does not dodge, and the bundled 10,000 would end the run before the tick.
fn content() -> RunContent {
    let mut v: serde_json::Value =
        serde_json::from_slice(ContentFiles::bundled_file("run_stats")).unwrap();
    v["enemy_attack_damage"] = serde_json::json!(100);
    let stats = RunStats::parse(v.to_string().as_bytes(), Versions::CURRENT).unwrap();
    RunContent::new(stats, RunContent::bundled().rewards().clone())
}

/// An engine `WARMUP_TICKS` ticks into the run: the next `step` is the measured tick.
pub fn warmed_up() -> Engine {
    let mut engine = Engine::new_with(&stage(), &content()).expect("the run starts");
    for tick in 1..=WARMUP_TICKS {
        engine.step(input(tick));
    }
    engine
}
