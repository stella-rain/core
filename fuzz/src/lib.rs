//! The bodies of the fuzz targets (ADR-020), as plain functions: the targets in
//! `fuzz_targets/` only call them, and `tests/smoke.rs` runs them on stable.
//!
//! Each function takes any bytes, must not panic, and returns whether the bytes got past the
//! first gate (a stage that validates, a replay that verifies, a share code that decodes), so
//! the tests can tell a target that reaches the code behind the gate from one that does not.
//! A panic is a finding: the assertions below say what must hold, not what happens to.

use miniz_oxide::deflate::compress_to_vec_zlib;
use stella_rain_core::engine::Engine;
use stella_rain_core::input::Input;
use stella_rain_core::replay::{self, Replay};
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_replay::{self, Command, CommandKind, RunReplay};
use stella_rain_core::run_setup;
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::stage::Stage;
use stella_rain_core::validate::limits::MAX_TICKS;
use stella_rain_core::validate::{self, parse_and_validate};
use stella_rain_core::{base64, share};

/// Ticks `stage_run` simulates: long enough for waves, attacks and boss phases to start, short
/// enough that the fuzzer keeps its speed.
const RUN_TICKS: usize = 120;

/// A stage the replay target verifies against (the corpus case that is cleared).
const CLEAR_STAGE: &[u8] = include_bytes!("../../tests/corpus/v0/clear/stage.json");

/// The stage parser and validator (ADR-020 items 1 to 4). A stage that loads can be saved
/// again, which is what the editor does, and loads again as the same stage. The same bytes are
/// also read as run stats and as rewards (ADR-067), content that may come from outside the app:
/// they parse without a panic, and files that load hash the same twice.
pub fn stage_parse(data: &[u8]) -> bool {
    if let Ok(stats) = RunStats::parse(data) {
        let again = RunStats::parse(data).expect("run stats that loaded load again");
        assert_eq!(again, stats, "the same bytes loaded as other run stats");
    }
    if let Ok(rewards) = Rewards::parse(data) {
        let again = Rewards::parse(data).expect("rewards that loaded load again");
        assert_eq!(again, rewards, "the same bytes loaded as other rewards");
    }
    let Ok(stage) = parse_and_validate(data) else {
        return false;
    };
    let saved = serde_json::to_vec(&stage).expect("a stage serialises");
    let again = parse_and_validate(&saved).expect("a stage that loaded loads again once saved");
    assert_eq!(again, stage, "a saved stage loads as another stage");
    true
}

/// The validator, then the simulation: a stage that validates must start a run and survive
/// `RUN_TICKS` ticks of any input without a panic (ADR-019, ADR-020). The bytes are the stage
/// file and, read over and over, the inputs.
pub fn stage_run(data: &[u8]) -> bool {
    // Deserialised without the validator's own parse, so the validator sees what the strict
    // types accept, not only what the size and version checks let through.
    let Ok(stage) = serde_json::from_slice::<Stage>(data) else {
        return false;
    };
    if validate::validate(&stage).is_err() {
        return false;
    }
    let mut engine = Engine::new(&stage).expect("a stage that validates starts a run");
    let ticks = usize::try_from(stage.length_ticks).map_or(RUN_TICKS, |n| n.min(RUN_TICKS));
    for (t, input) in inputs_from(data, ticks).into_iter().enumerate() {
        step_spending(&mut engine, input, pick(data, t));
        let _ = engine.snapshot();
    }
    engine.tick() > 0
}

/// The run setup (ADR-051) and the run it starts: a setup that validates must save and load as
/// the same setup, start a run, and survive `RUN_TICKS` ticks of any input without a panic
/// (ADR-019, ADR-020). The bytes are the setup file and, read over and over, the inputs.
pub fn run_setup(data: &[u8]) -> bool {
    let Ok(setup) = run_setup::parse_and_validate(data) else {
        return false;
    };
    let saved = serde_json::to_vec(&setup).expect("a setup serialises");
    let again = run_setup::parse_and_validate(&saved).expect("a setup that loaded loads again");
    assert_eq!(again, setup, "a saved setup loads as another setup");
    let mut engine = Engine::start(&setup).expect("a setup that validates starts a run");
    for (t, input) in inputs_from(data, RUN_TICKS).into_iter().enumerate() {
        step_spending(&mut engine, input, pick(data, t));
        let _ = engine.snapshot();
    }
    engine.tick() > 0
}

/// One tick of `input`, after the commands between ticks (ADR-066): a choice with `pick` as it
/// is, then a spend of `pick` as it is and one of `pick` inside the stats, each of which may be
/// refused and must then change nothing; then, while an offer is open, a choice of `pick`
/// inside its options, which is never refused, so the world goes on.
fn step_spending(engine: &mut Engine, input: Input, pick: u8) {
    let stats = u32::try_from(engine.run_stats().stats().len()).expect("few run stats");
    for (kind, index) in [
        (CommandKind::Choose, u32::from(pick)),
        (CommandKind::Spend, u32::from(pick)),
        (CommandKind::Spend, u32::from(pick) % stats),
    ] {
        let hash = engine.state_hash();
        let refused = match kind {
            CommandKind::Choose => engine.choose(index).is_err(),
            CommandKind::Spend => engine.spend(index).is_err(),
        };
        if refused {
            assert_eq!(
                engine.state_hash(),
                hash,
                "a refused command changed the run"
            );
        }
    }
    // Each choice spends a reroll or a banish, or closes the offer; a run has few of either.
    for _ in 0..64 {
        let Some(offer) = engine.snapshot().progress.and_then(|p| p.offer) else {
            break;
        };
        let options = u32::try_from(offer.options.len()).expect("few options");
        engine
            .choose(u32::from(pick) % options)
            .expect("an option of the open offer is taken");
    }
    assert!(
        !engine.offer_open(),
        "an offer stayed open after 64 choices"
    );
    engine.step(input);
}

/// The option to pick on tick `t`: a byte of `data`, round again at its end.
fn pick(data: &[u8], t: usize) -> u8 {
    data.get(t % data.len().max(1)).copied().unwrap_or(0)
}

/// Inputs read out of `data`, three bytes a tick (the stick, then the buttons' and switches'
/// bits: `evade`, `main`, `sub`, `skill`, `assist`, `auto` from bit 0) and round again at its
/// end. The stick takes
/// −128 too, which no replay can hold, because the engine is also called directly.
fn inputs_from(data: &[u8], ticks: usize) -> Vec<Input> {
    let byte = |t: usize, i: usize| {
        data.get((t * 3 + i) % data.len().max(1))
            .copied()
            .unwrap_or(0)
    };
    (0..ticks)
        .map(|t| Input {
            move_x: i8::from_le_bytes([byte(t, 0)]),
            move_y: i8::from_le_bytes([byte(t, 1)]),
            evade: byte(t, 2) & 1 != 0,
            main: byte(t, 2) & 2 != 0,
            sub: byte(t, 2) & 4 != 0,
            skill: byte(t, 2) & 8 != 0,
            assist: byte(t, 2) & 16 != 0,
            auto: byte(t, 2) & 32 != 0,
        })
        .collect()
}

/// The replay loader (ADR-020 item 5, ADR-051 §3), for stage replays and run replays: a replay
/// that loads unpacks without a panic and saves to bytes that load as the same replay; any
/// bytes verified against a stage that is cleared, or as a run replay, never panic, whatever
/// they claim; and a run replay that verifies is exactly what its own commands record.
pub fn replay_load(data: &[u8]) -> bool {
    if let Ok(replay) = Replay::from_bytes(data) {
        let _ = replay.unpack_inputs(MAX_TICKS);
        let again = Replay::from_bytes(&replay.to_bytes())
            .expect("a replay that loaded loads again once saved");
        assert_eq!(again, replay, "a saved replay loads as another replay");
    }
    if let Ok(run) = RunReplay::from_bytes(data) {
        let _ = run.unpack_inputs(MAX_TICKS);
        let _ = run.unpack_commands();
        let again = RunReplay::from_bytes(&run.to_bytes())
            .expect("a run replay that loaded loads again once saved");
        assert_eq!(again, run, "a saved run replay loads as another run replay");
    }
    let stage_replay = replay::verify(CLEAR_STAGE, data).is_ok();
    let run_replay = run_replay::verify_run(data).is_ok();
    if run_replay {
        let run = RunReplay::from_bytes(data).expect("a run replay that verified loads");
        let recorded = run_replay::record_run(&run.setup, &commands(&run))
            .expect("the commands of a run replay that verified record");
        assert_eq!(recorded, run, "a verified run replay records as another");
    }
    stage_replay || run_replay
}

/// A run replay's commands in the order they were made: before each tick the commands that
/// carry the tick before it, then its input; at the end the commands of the last tick.
fn commands(run: &RunReplay) -> Vec<Command> {
    let inputs = run
        .unpack_inputs(MAX_TICKS)
        .expect("verified inputs unpack");
    let between = run.unpack_commands().expect("verified commands unpack");
    let mut next = between.iter().peekable();
    let command = |c: &run_replay::TickCommand| match c.kind {
        CommandKind::Choose => Command::Choose(c.index),
        CommandKind::Spend => Command::Spend(c.index),
    };
    let mut out = Vec::new();
    for (tick, input) in (0u32..).zip(inputs) {
        while let Some(c) = next.next_if(|c| c.tick == tick) {
            out.push(command(c));
        }
        out.push(Command::Step(input));
    }
    out.extend(next.map(command));
    out
}

/// The share-code decoder (ADR-020 item 7, ADR-009). The bytes are tried as a code, and wrapped
/// as the zlib stream of a code so that they reach the code behind the decompression. A code
/// that decodes holds files that encode to a code that decodes to them; base64 round-trips
/// any bytes.
pub fn share_decode(data: &[u8]) -> bool {
    let bytes_back = base64::decode(&base64::encode(data)).expect("base64 decodes what it encoded");
    assert_eq!(bytes_back, data, "base64 changed the bytes");

    let mut decoded = false;
    if let Ok(code) = std::str::from_utf8(data) {
        decoded |= check_code(code);
    }
    decoded | check_code(&base64::encode(&compress_to_vec_zlib(data, 1)))
}

fn check_code(code: &str) -> bool {
    let Ok((stage, replay)) = share::decode(code) else {
        return false;
    };
    // The code may be longer than `encode` allows when another compressor made it.
    if let Ok(again) = share::encode(&stage, &replay) {
        let back = share::decode(&again).expect("a code that was encoded decodes");
        assert_eq!(back, (stage, replay), "a share code changed its files");
    }
    true
}
