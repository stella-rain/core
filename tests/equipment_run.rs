//! A solo run in equipment mode played by `play_equipment_run*` and the helper `common::equip_run`
//! (ADR-SIM-7): the bot's actions ride in `Input.action`, the recorder applies and records them.

use crate::common::equip_run::{equipment_setup, play};
use crate::common::run::{time_limit_setup, trial_setup};
use crate::party_bot::{PartyBot, RigParty, RigRun};
use crate::run_play::{
    EQUIP_RIG_SEEDS, equipment_rig, party_inputs_with, play_equipment_run, play_equipment_run_from,
    play_equipment_run_in, play_equipment_run_with,
};
use crate::zone_bot::run_input;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{Command, RunRecorder, record_run, verify_run};
use stella_rain_core::run_setup::RunSetup;
use stella_rain_core::validate::limits::MAX_TICKS;

/// The world seed of `a_run_plays_to_its_end_with_the_zone_bot`: the first seed of 1..=20 whose
/// `time_limit` equipment run, played by the zone bot with a building `PartyBot` (bot seed 1),
/// ends before `MAX_TICKS` (`search_equipment_end_seed`).
const END_SEED: u64 = 1;
const END_BOT_SEED: u64 = 1;

fn rig_1() -> RigParty {
    let (seed, bot_seed) = EQUIP_RIG_SEEDS[0];
    RigParty::new(1)
        .equipment()
        .builds()
        .seed(seed)
        .bot_seed(bot_seed)
}

fn rig_1_commands() -> (RunSetup, PartyBot, Vec<Command>) {
    let case = rig_1().case("x");
    let bot = PartyBot::building(EQUIP_RIG_SEEDS[0].1);
    let commands = play_equipment_run_with(
        &case.setup,
        RunContent::bundled(),
        3000,
        |e, t| bot.input(e, 0, t),
        &bot,
    );
    (case.setup, bot, commands)
}

fn end_setup(seed: u64) -> RunSetup {
    RunSetup {
        seed,
        ..equipment_setup(time_limit_setup())
    }
}

#[test]
fn the_twin_plays_the_party_of_one_rig_run() {
    let (setup, _, commands) = rig_1_commands();
    let replay = record_run(&setup, &commands).unwrap();
    let party = equipment_rig(1).play().replay();
    let (twin, party_run) = (RigRun::of(&replay), RigRun::of(&party));
    assert_eq!(twin.ticks, party_run.ticks);
    assert_eq!(twin.picks, party_run.picks);
    assert_eq!(twin.actions, party_run.actions);
    let counts = twin.actions_per_slot()[0];
    assert_eq!(counts, party_run.actions_per_slot()[0]);
    // The corpus pin of `equipment_rig_1` (party_run.rs) holds every kind at least once.
    assert!(
        counts.equip > 0
            && counts.replace > 0
            && counts.extract > 0
            && counts.craft > 0
            && counts.tree_pick > 0,
        "{counts:?}"
    );
}

#[test]
fn the_commands_carry_every_action_and_no_choose() {
    let (setup, _, commands) = rig_1_commands();
    assert!(commands.iter().all(|c| matches!(c, Command::Step(_))));
    assert_eq!(commands.len(), 3000);
    let with_action = commands
        .iter()
        .filter(|c| matches!(c, Command::Step(i) if i.action.is_some()))
        .count();
    let recorded = RigRun::of(&record_run(&setup, &commands).unwrap()).actions_per_slot()[0];
    let total =
        recorded.equip + recorded.replace + recorded.extract + recorded.craft + recorded.tree_pick;
    assert!(total > 0);
    assert_eq!(with_action, usize::try_from(total).unwrap());
}

#[test]
fn an_equipment_solo_run_records_verifies_and_repeats() {
    let (setup, _, commands) = rig_1_commands();
    let a = record_run(&setup, &commands).unwrap();
    let b = record_run(&setup, &commands).unwrap();
    assert_eq!(a.to_bytes(), b.to_bytes());
    let verified = verify_run(&a.to_bytes()).unwrap();
    assert_eq!(verified.outcome, Outcome::Running);
    let mut rec = RunRecorder::start_with(&setup, RunContent::bundled()).unwrap();
    for command in &commands {
        let Command::Step(input) = command else {
            panic!("a choose in an equipment run")
        };
        rec.step(&[*input, Default::default(), Default::default()])
            .unwrap();
    }
    assert_eq!(
        verified.final_hash,
        rec.engine().state_hash(),
        "the verifier's hash is the recorder's"
    );
}

#[test]
fn a_run_plays_to_its_end_with_the_zone_bot() {
    let setup = end_setup(END_SEED);
    let bot = PartyBot::building(END_BOT_SEED);
    let play_it = || play_equipment_run_in(&setup, RunContent::bundled(), MAX_TICKS, &bot);
    let commands = play_it();
    let mut rec = RunRecorder::start_with(&setup, RunContent::bundled()).unwrap();
    for command in &commands {
        let Command::Step(input) = command else {
            panic!("a choose in an equipment run")
        };
        rec.step(&[*input, Default::default(), Default::default()])
            .unwrap();
    }
    assert_ne!(rec.engine().outcome(), Outcome::Running);
    assert_eq!(
        commands.len(),
        usize::try_from(rec.engine().tick()).unwrap()
    );
    let replay = record_run(&setup, &commands).unwrap();
    let verified = verify_run(&replay.to_bytes()).unwrap();
    assert_eq!(verified.outcome, rec.engine().outcome());
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::Step(i) if i.action.is_some())),
        "an action was recorded"
    );
    assert_eq!(commands, play_it());
}

#[test]
fn a_bot_without_builds_records_no_action() {
    let case = rig_1().case("x");
    let bot = PartyBot {
        seed: EQUIP_RIG_SEEDS[0].1,
        builds: false,
    };
    let commands = play_equipment_run_with(
        &case.setup,
        RunContent::bundled(),
        600,
        |e, t| bot.input(e, 0, t),
        &bot,
    );
    assert!(
        commands
            .iter()
            .all(|c| matches!(c, Command::Step(i) if i.action.is_none()))
    );
    let mut rec = RunRecorder::start_with(&case.setup, RunContent::bundled()).unwrap();
    for t in 0..600 {
        let input = bot.input(rec.engine(), 0, t);
        rec.step(&[input, Default::default(), Default::default()])
            .unwrap();
    }
    let plain: Vec<Command> = (0..600)
        .map(|t| (t, ()))
        .scan(Engine::start(&case.setup).unwrap(), |engine, (t, ())| {
            let input = bot.input(engine, 0, t);
            engine.step(&[input, Default::default(), Default::default()]);
            Some(Command::Step(input))
        })
        .collect();
    assert_eq!(commands, plain);
    assert_eq!(
        record_run(&case.setup, &commands).unwrap().final_hash,
        format!("{:016x}", rec.engine().state_hash())
    );
}

#[test]
#[should_panic(expected = "needs sim_version 2")]
fn the_twin_refuses_a_version_1_setup() {
    let bot = PartyBot::building(1);
    let frozen = RunSetup {
        sim_version: 1,
        ..trial_setup()
    };
    play_equipment_run_from(&frozen, 10, &bot);
}

#[test]
fn play_stops_at_the_end_and_at_the_ticks() {
    let setup = equipment_setup(trial_setup());
    let mut rec = RunRecorder::start_with(&setup, RunContent::bundled()).unwrap();
    play(&mut rec, 100, |rec, t| {
        [
            run_input(rec.engine(), t),
            Default::default(),
            Default::default(),
        ]
    });
    assert_eq!(rec.engine().tick(), 100);

    let setup = end_setup(END_SEED);
    let bot = PartyBot::building(END_BOT_SEED);
    let mut rec = RunRecorder::start_with(&setup, RunContent::bundled()).unwrap();
    let inputs = |rec: &RunRecorder, t: u32| {
        let mut input = run_input(rec.engine(), t);
        input.action = bot.action(rec.engine(), 0, t);
        [input, Default::default(), Default::default()]
    };
    play(&mut rec, MAX_TICKS, inputs);
    assert_ne!(rec.engine().outcome(), Outcome::Running);
    let ended = rec.engine().tick();
    play(&mut rec, 10, inputs);
    assert_eq!(rec.engine().tick(), ended);
}

#[test]
fn the_helper_plays_a_party_rig_like_play_party_with() {
    let (seed, bot_seed) = EQUIP_RIG_SEEDS[1];
    let case = RigParty::new(2)
        .equipment()
        .builds()
        .seed(seed)
        .bot_seed(bot_seed)
        .case("x");
    let mut rec = RunRecorder::start_party(&case.setup, 2).unwrap();
    play(&mut rec, 3000, |rec, t| {
        party_inputs_with(rec, t, case.bot.as_ref())
    });
    assert_eq!(rec.replay().to_bytes(), case.play().replay().to_bytes());
}

/// `cargo test --release --locked --test integration search_equipment_end_seed -- --ignored
/// --nocapture`: world seeds 1..=20, the first whose run ends before `MAX_TICKS`.
#[test]
#[ignore = "a seed search, run by hand"]
fn search_equipment_end_seed() {
    for seed in 1..=20u64 {
        let setup = end_setup(seed);
        let bot = PartyBot::building(END_BOT_SEED);
        let commands = play_equipment_run_in(&setup, RunContent::bundled(), MAX_TICKS, &bot);
        let ended = commands.len() < usize::try_from(MAX_TICKS).unwrap();
        println!("seed={seed} ticks={} ended={ended}", commands.len());
    }
}

#[test]
fn the_trial_wrapper_plays_the_trial_in_equipment_mode() {
    let bot = PartyBot::building(1);
    let commands = play_equipment_run(300, &bot);
    assert_eq!(commands.len(), 300);
    let setup = equipment_setup(trial_setup());
    assert_eq!(commands, play_equipment_run_from(&setup, 300, &bot));
}

#[test]
fn a_default_setup_runs_in_equipment_mode() {
    let setup = RunSetup {
        seed: 20_261_010,
        ..RunSetup::default()
    };
    let mut engine = Engine::start(&setup).unwrap();
    let snap = engine.snapshot();
    assert!(!snap.players[0].slots.is_empty(), "no slots");
    assert!(snap.starlight.is_some(), "no Starlight");
    for _ in 0..600 {
        engine.step(&Default::default());
    }
    // The state hash of this run at tick 600; re-pinned when the combat fields joined the hash, then for the counter-bonus mark, then for the shipped ranged grace.
    assert_eq!(format!("{:016x}", engine.state_hash()), "b77d4117b61d7847");

    let frozen = RunSetup {
        schema_version: 1,
        sim_version: 1,
        ..setup
    };
    let v1 = crate::frozen_v1::pack().content();
    let snap = Engine::start_with(&frozen, v1).unwrap().snapshot();
    assert!(snap.players[0].slots.is_empty());
    assert!(snap.starlight.is_none());
}

/// No run of the live rules opens a star offer, and the public offer calls answer as they did
/// when the offer logic was still in the build (ADR-SIM-3, ADR-REPL-5).
#[test]
fn a_live_run_never_opens_a_star_offer() {
    use stella_rain_core::input::Input;

    let input = Input {
        main: true,
        move_x: 60,
        ..Input::default()
    };
    let cases = [
        (equipment_setup(trial_setup()), 1u8),
        (equipment_setup(time_limit_setup()), 1),
        (equipment_setup(trial_setup()), 3),
    ];
    for (setup, party) in cases {
        let mut engine = Engine::start_party(&setup, party).unwrap();
        for _ in 0..1_500 {
            if engine.outcome() != Outcome::Running {
                break;
            }
            assert!(!engine.offer_open());
            let before = engine.tick();
            engine.step(&[input; 3]);
            assert_eq!(engine.tick(), before + 1);
        }
    }
}
