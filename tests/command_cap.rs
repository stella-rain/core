//! The most commands (`choose`, `spend`) the longest possible run takes, which `MAX_COMMANDS`
//! is about twice (app#336 D5b): `time_limit` at a difficulty that keeps its full length, played
//! through the final boss's time and the whole overtime by a bot that cannot die, takes every
//! offer and spends every point as soon as it has one.

use crate::common;
use crate::common::run::run_input_for;
use serde_json::json;
use stella_rain_core::engine::Outcome;
use stella_rain_core::perk_sets::PerkSets;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{
    CommandKind, MAX_COMMANDS, RunRecorder, RunReplay, RunReplayError, TickCommand, pack_commands,
    verify_run_with,
};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::snapshot::{EntityKind, OfferOption};
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::version::Versions;

/// The perk set the bot plays with: the most hit points and regeneration a set may hold (the
/// engine has no invulnerability switch), the most extra rerolls and banishes, and the
/// strongest attack, so that it kills, levels and takes stars as fast as a run can.
const PERK_SET: &str = "undying";

/// The run content with the `undying` perk set added to the bundled ones.
pub fn undying_content() -> RunContent {
    let mut file: serde_json::Value =
        serde_json::from_slice(common::bundled_file("perk_sets")).unwrap();
    file["perk_sets"].as_array_mut().unwrap().push(json!({
        "id": PERK_SET, "title": "Undying", "description": "",
        "effects": [
            { "effect": "max_hp", "amount": 50_000 },
            { "effect": "regen", "amount": 1_000 },
            { "effect": "attack_pct", "amount": 100 },
            { "effect": "attack_speed_pct", "amount": 100 },
            { "effect": "extra_rerolls", "amount": 20 },
            { "effect": "extra_banishes", "amount": 20 }
        ]
    }));
    RunContent::bundled()
        .clone()
        .with_perk_sets(PerkSets::parse(file.to_string().as_bytes(), Versions::CURRENT).unwrap())
}

pub fn setup(seed: u64, difficulty: &str, extra_rerolls: u8) -> RunSetup {
    RunSetup {
        schema_version: 0,
        sim_version: 0,
        mode: "time_limit".into(),
        weapon: "giant_sword".into(),
        weapon_tree: None,
        seed,
        star_pool: None,
        extra_rerolls,
        upgrades: Vec::new(),
        perk_set: PERK_SET.into(),
        rune_pool: Vec::new(),
        carried: Vec::new(),
        lumine: 0,
        starting_rune: None,
        stat_priority: Vec::new(),
        // Every level-up opens a stat offer: the most commands a run can cost.
        pick_stats: true,
        difficulty: difficulty.into(),
    }
}

/// How the bot answers an offer.
#[derive(Clone, Copy, Debug)]
pub enum Bot {
    /// Takes candidate 0.
    Takes,
    /// Rerolls and banishes as long as it may, then takes candidate 0: the most commands an
    /// offer can cost.
    Spends,
}

/// What a bot's run came to.
pub struct Played {
    pub replay: RunReplay,
    pub commands: usize,
    pub outcome: Outcome,
}

/// Plays `setup` to its end (the length and the overtime, or the final boss's kill).
pub fn play(content: &RunContent, setup: &RunSetup, bot: Bot) -> Played {
    let mut run = RunRecorder::start_with(setup, content).unwrap();
    let mut tick = 0;
    while run.engine().outcome() == Outcome::Running {
        answer_offers(&mut run, bot);
        let input = run_input_for(run.engine(), tick, &[EntityKind::Enemy], u32::MAX);
        run.step(input).unwrap();
        tick += 1;
    }
    answer_offers(&mut run, bot);
    let replay = run.replay();
    let commands = replay.unpack_commands().unwrap().len();
    Played {
        replay,
        commands,
        outcome: run.engine().outcome(),
    }
}

fn answer_offers(run: &mut RunRecorder, bot: Bot) {
    while run.engine().offer_open() {
        let snapshot = run.engine().snapshot();
        if snapshot.expertise.and_then(|g| g.offer).is_some()
            || snapshot
                .progress
                .as_ref()
                .is_some_and(|p| p.stat_offer.is_some())
        {
            run.choose(0).unwrap();
            continue;
        }
        let options = snapshot.progress.and_then(|p| p.offer).unwrap().options;
        let find = |wanted: fn(&OfferOption) -> bool| options.iter().position(wanted);
        let pick = match bot {
            Bot::Takes => None,
            Bot::Spends => find(|o| matches!(o, OfferOption::Reroll { .. }))
                .or_else(|| find(|o| matches!(o, OfferOption::Banish { .. }))),
        };
        let pick = pick
            .or_else(|| find(|o| *o == OfferOption::Take { candidate: 0 }))
            .unwrap_or(0);
        run.choose(u32::try_from(pick).unwrap()).unwrap();
    }
}

/// The most commands the measurement found (`measure_the_longest_run`), pinned: `MAX_COMMANDS`
/// is at least twice it.
const MEASURED_MOST: usize = 244;

/// The bundled mode cut to 20 seconds with a boss at 10 and at 20 and 5 of overtime, a run
/// that a debug build plays in seconds.
fn short_content() -> RunContent {
    let mut v: serde_json::Value =
        serde_json::from_slice(common::bundled_file("time_limit")).unwrap();
    v["length_ticks"] = json!(1200);
    v["boss_ticks"] = json!([600, 1200]);
    v["no_call_ticks"] = json!([]);
    v["overtime_ticks"] = json!(300);
    v["waves"]
        .as_array_mut()
        .unwrap()
        .retain(|w| w["at_tick"].as_u64().unwrap() < 1200);
    v["elite_table"] =
        json!([{ "start_tick": 0, "weights": [{ "elite": "elite_low", "weight": 1 }] }]);
    let mode = TimeLimit::parse(v.to_string().as_bytes(), Versions::CURRENT)
        .unwrap_or_else(|e| panic!("{e}"));
    undying_content().with_time_limit(mode)
}

#[test]
fn the_cap_is_twice_the_longest_run_or_more() {
    assert!(
        2 * MEASURED_MOST <= MAX_COMMANDS as usize,
        "MAX_COMMANDS {MAX_COMMANDS} is under twice {MEASURED_MOST}"
    );
}

#[test]
fn a_bot_s_short_run_records_and_verifies_within_the_cap() {
    let content = short_content();
    for bot in [Bot::Takes, Bot::Spends] {
        let played = play(&content, &setup(1, "normal", 20), bot);
        assert!(
            played.commands > 0,
            "{bot:?} answers an offer or spends a point"
        );
        assert!(played.commands <= MAX_COMMANDS as usize);
        let verified = verify_run_with(&played.replay.to_bytes(), &content)
            .unwrap_or_else(|e| panic!("{bot:?}: {e}"));
        assert_eq!(verified.commands as usize, played.commands);
        assert_eq!(verified.outcome, played.outcome);
    }
}

#[test]
fn a_crafted_replay_with_one_command_over_the_cap_is_refused() {
    let content = short_content();
    let mut replay = play(&content, &setup(1, "normal", 20), Bot::Takes).replay;
    let choices = |n: u32| -> String {
        let commands: Vec<TickCommand> = (0..n)
            .map(|_| TickCommand {
                tick: 0,
                kind: CommandKind::Choose,
                index: 0,
            })
            .collect();
        pack_commands(&commands)
    };
    // At the cap the file loads (the choices are then refused by the engine for lack of an
    // offer, not for their number); one over it never gets that far.
    replay.commands = choices(MAX_COMMANDS);
    assert_eq!(
        RunReplay::from_bytes(&replay.to_bytes())
            .unwrap()
            .unpack_commands()
            .unwrap()
            .len(),
        MAX_COMMANDS as usize
    );
    replay.commands = choices(MAX_COMMANDS + 1);
    assert_eq!(
        verify_run_with(&replay.to_bytes(), &content),
        Err(RunReplayError::TooManyCommands {
            limit: MAX_COMMANDS
        })
    );
}

/// The difficulties that keep the mode's full length (`easy` halves it).
const LONG: [&str; 3] = ["normal", "hard", "rainy"];

/// The most commands of the longest runs measured, over `SEEDS` seeds and every long difficulty
/// and both bots, with the run setup's extra rerolls at their most. Slow (minutes in release),
/// so `#[ignore]`d: `cargo test --release --test integration command_cap:: -- --ignored --nocapture`.
#[test]
#[ignore = "the measurement of MAX_COMMANDS: minutes in release"]
fn measure_the_longest_run() {
    let content = undying_content();
    let mut most = 0;
    let mut longest = None;
    for difficulty in LONG {
        for seed in 1..=5 {
            for bot in [Bot::Takes, Bot::Spends] {
                let played = play(&content, &setup(seed, difficulty, 20), bot);
                println!(
                    "difficulty {difficulty} seed {seed} {bot:?}: {} commands, {} ticks, {:?}",
                    played.commands, played.replay.ticks, played.outcome
                );
                if played.commands > most {
                    most = played.commands;
                    longest = Some(played.replay);
                }
            }
        }
    }
    println!("most commands: {most}");
    assert!(
        most <= MEASURED_MOST,
        "a run takes {most} commands: re-pin MEASURED_MOST"
    );
    assert!(
        2 * most <= MAX_COMMANDS as usize,
        "MAX_COMMANDS is under twice {most}"
    );
    // The longest run records within the cap and verifies from its file.
    let verified = verify_run_with(&longest.unwrap().to_bytes(), &content).unwrap();
    assert_eq!(verified.commands as usize, most);
}
