//! The bodies of the fuzz targets (ADR-SVC-1), as plain functions: the targets in
//! `fuzz_targets/` only call them, and `tests/smoke.rs` runs them on stable.
//!
//! Each function takes any bytes, must not panic, and returns whether the bytes got past the
//! first gate (a stage that validates, a replay that verifies, a share code that decodes), so
//! the tests can tell a target that reaches the code behind the gate from one that does not.
//! A panic is a finding: the assertions below say what must hold, not what happens to.

use std::sync::OnceLock;

use miniz_oxide::deflate::compress_to_vec_zlib;
use stella_rain_core::attack_presets::AttackPresets;
use stella_rain_core::content_hash::sha256;
use stella_rain_core::engine::{ChoiceError, Engine, Outcome};
use stella_rain_core::expertise::Expertise;
use stella_rain_core::id::{ContentId, PlayerIdx};
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::match_ups::MatchUps;
use stella_rain_core::pack::{Pack, PackFile};
use stella_rain_core::packs::{Packs, PacksError};
use stella_rain_core::progression::{Bank, RunOutcome, RunResult, settle};
use stella_rain_core::replay::{self, Replay};
use stella_rain_core::rewards::Rewards;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{self, Command, RunRecorder, RunReplay};
use stella_rain_core::run_setup::{self, RunSetup};
use stella_rain_core::run_stats::RunStats;
use stella_rain_core::runes::Runes;
use stella_rain_core::savepoint::{HEADER_LEN, SavepointError};
use stella_rain_core::stage::Stage;
use stella_rain_core::stage_skill::StageSkills;
use stella_rain_core::text::TextTable;
use stella_rain_core::time_limit::TimeLimit;
use stella_rain_core::validate::limits::{MAX_RUN_STARDUST, MAX_TICKS};
use stella_rain_core::validate::{self, parse_and_validate};
use stella_rain_core::version::{
    FIRST_SCHEMA_VERSION, FIRST_SIM_VERSION, NEWEST_READ_SCHEMA_VERSION, SCHEMA_VERSION, Versions,
};
use stella_rain_core::{RangeStep, StepRange, base64, read_step_range, share};

/// Ticks `stage_run` simulates: long enough for waves, attacks and boss phases to start, short
/// enough that the fuzzer keeps its speed.
const RUN_TICKS: usize = 120;

/// A stage the replay target verifies against (the corpus case that is cleared).
const CLEAR_STAGE: &[u8] = include_bytes!("../../tests/corpus/current/stages/clear/stage.json");

/// The stage parser and validator (ADR-SVC-1). A stage that loads can be saved
/// again, which is what the editor does, and loads again as the same stage. The same bytes are
/// also read as run stats, as rewards, as stage skills, as expertise, as the match-up table, as
/// runes and as attack presets (ADR-CONT-1, CS1b-1), content
/// that may come from outside the app: they parse without a panic, and files that load hash the
/// same twice.
pub fn stage_parse(data: &[u8]) -> bool {
    if let Ok(stats) = RunStats::parse(data, Versions::CURRENT) {
        let again =
            RunStats::parse(data, Versions::CURRENT).expect("run stats that loaded load again");
        assert_eq!(again, stats, "the same bytes loaded as other run stats");
    }
    if let Ok(rewards) = Rewards::parse(data, Versions::CURRENT) {
        let again =
            Rewards::parse(data, Versions::CURRENT).expect("rewards that loaded load again");
        assert_eq!(again, rewards, "the same bytes loaded as other rewards");
    }
    if let Ok(skills) = StageSkills::parse(data, Versions::CURRENT) {
        let again = StageSkills::parse(data, Versions::CURRENT)
            .expect("stage skills that loaded load again");
        assert_eq!(again, skills, "the same bytes loaded as other stage skills");
    }
    if let Ok(expertise) = Expertise::parse(data, Versions::CURRENT) {
        let again =
            Expertise::parse(data, Versions::CURRENT).expect("expertise that loaded loads again");
        assert_eq!(again, expertise, "the same bytes loaded as other expertise");
    }
    if let Ok(match_ups) = MatchUps::parse(data, Versions::CURRENT) {
        let again =
            MatchUps::parse(data, Versions::CURRENT).expect("match-ups that loaded load again");
        assert_eq!(again, match_ups, "the same bytes loaded as other match-ups");
    }
    if let Ok(mode) = TimeLimit::parse(data, Versions::CURRENT) {
        let again =
            TimeLimit::parse(data, Versions::CURRENT).expect("a mode that loaded loads again");
        assert_eq!(again, mode, "the same bytes loaded as another mode");
    }
    if let Ok(runes) = Runes::parse(data, Versions::CURRENT) {
        let again = Runes::parse(data, Versions::CURRENT).expect("runes that loaded load again");
        assert_eq!(again, runes, "the same bytes loaded as other runes");
    }
    if let Ok(presets) = AttackPresets::parse(data, Versions::CURRENT) {
        let again = AttackPresets::parse(data, Versions::CURRENT)
            .expect("attack presets that loaded load again");
        assert_eq!(
            again, presets,
            "the same bytes loaded as other attack presets"
        );
    }
    if let Ok(text) = TextTable::from_bytes(data) {
        let again = TextTable::from_bytes(data).expect("a text table that loaded loads again");
        assert_eq!(
            again.entries(),
            text.entries(),
            "the same bytes loaded as another text table"
        );
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
/// `RUN_TICKS` ticks of any input without a panic (ADR-SIM-3, ADR-SVC-1). The bytes are the stage
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

/// The run setup (ADR-REPL-5) and the run it starts: a setup that validates must save and load as
/// the same setup, start a run, and survive `RUN_TICKS` ticks of any input without a panic
/// (ADR-SIM-3, ADR-SVC-1). The bytes are the setup file and, read over and over, the inputs.
pub fn run_setup(data: &[u8]) -> bool {
    let Ok(setup) = run_setup::parse_and_validate(data) else {
        return false;
    };
    let saved = serde_json::to_vec(&setup).expect("a setup serialises");
    let again = run_setup::parse_and_validate(&saved).expect("a setup that loaded loads again");
    assert_eq!(again, setup, "a saved setup loads as another setup");
    // The perk set and the carried stars are checked against the run's content when it starts:
    // a setup that names what the content lacks fails closed there, and only there.
    let Ok(mut engine) = Engine::start(&setup) else {
        assert!(
            run_setup::validate_in(&setup, RunContent::bundled()).is_err(),
            "a setup that validates against the content starts a run"
        );
        return false;
    };
    for (t, input) in inputs_from(data, RUN_TICKS).into_iter().enumerate() {
        step_spending(&mut engine, input, pick(data, t));
        let _ = engine.snapshot();
    }
    engine.tick() > 0
}

/// One tick of `input`, after the commands between ticks (ADR-REPL-5): a choice with `pick` as it
/// is, which may be refused and must then change nothing; then, while an offer is open, a choice
/// of `pick` inside its options, which is never refused, so the world goes on.
fn step_spending(engine: &mut Engine, input: Input, pick: u8) {
    let hash = engine.state_hash();
    if take(engine, u32::from(pick)).is_err() {
        assert_eq!(
            engine.state_hash(),
            hash,
            "a refused command changed the run"
        );
    }
    // Each choice spends a reroll or a banish, or closes the offer; a run has few of either.
    for _ in 0..64 {
        let Some(options) = engine.snapshot().progress.and_then(|p| {
            p.offer
                .map(|o| o.options.len())
                .or(p.stat_offer.map(|o| o.options.len()))
        }) else {
            break;
        };
        let options = u32::try_from(options).expect("few options");
        take(engine, u32::from(pick) % options).expect("an option of the open offer is taken");
    }
    assert!(
        !engine.offer_open(),
        "an offer stayed open after 64 choices"
    );
    engine.step(&[input, Default::default(), Default::default()]);
}

/// Takes option `index` of the open offer through a step, as `check_pick` allows it.
fn take(engine: &mut Engine, index: u32) -> Result<(), ChoiceError> {
    engine.check_pick(PlayerIdx::SOLO, index)?;
    engine.step(&[
        Input::picking(index),
        Default::default(),
        Default::default(),
    ]);
    Ok(())
}

/// The option to pick on tick `t`: a byte of `data`, round again at its end.
fn pick(data: &[u8], t: usize) -> u8 {
    data.get(t % data.len().max(1)).copied().unwrap_or(0)
}

/// Inputs read out of `data`, three bytes a tick (the stick, then the buttons' and switches'
/// bits: `evade`, `main`, `smash`, `skill`, `assist`, `auto` from bit 0) and round again at its
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
            smash: byte(t, 2) & 4 != 0,
            skill: byte(t, 2) & 8 != 0,
            assist: byte(t, 2) & 16 != 0,
            auto: byte(t, 2) & 32 != 0,
            pick: None,
            action: None,
            interact: false,
        })
        .collect()
}

/// The replay loader (ADR-SVC-1, ADR-REPL-5), for stage replays and run replays: a replay
/// that loads unpacks without a panic and saves to bytes that load as the same replay; any
/// bytes verified against a stage that is cleared, or as a run replay, never panic, whatever
/// they claim; a run replay that verifies is exactly what its own commands record; and a replay
/// that loads is at this core's schema version and a sim version from `FIRST_SIM_VERSION` to the current (any other pair is `Older`, `Newer` or `Damaged`).
pub fn replay_load(data: &[u8]) -> bool {
    // The newest versions this build reads; writers still write `Versions::CURRENT`.
    let current = Versions {
        schema_version: NEWEST_READ_SCHEMA_VERSION,
        sim_version: Versions::CURRENT.sim_version,
    };
    if let Ok(replay) = Replay::from_bytes(data) {
        assert!(
            (FIRST_SCHEMA_VERSION..=current.schema_version).contains(&replay.schema_version)
                && (FIRST_SIM_VERSION..=current.sim_version).contains(&replay.sim_version),
            "a replay that loads is at another version"
        );
        let _ = replay.unpack_inputs(MAX_TICKS);
        let again = Replay::from_bytes(&replay.to_bytes())
            .expect("a replay that loaded loads again once saved");
        assert_eq!(again, replay, "a saved replay loads as another replay");
    }
    if let Ok(run) = RunReplay::from_bytes(data) {
        assert!(
            (FIRST_SCHEMA_VERSION..=current.schema_version).contains(&run.setup.schema_version)
                && (FIRST_SIM_VERSION..=current.sim_version).contains(&run.setup.sim_version),
            "a run replay that loads is at another version"
        );
        let _ = run.unpack_inputs(MAX_TICKS);
        let _ = run.unpack_commands();
        let again = RunReplay::from_bytes(&run.to_bytes())
            .expect("a run replay that loaded loads again once saved");
        assert_eq!(again, run, "a saved run replay loads as another run replay");
        // A prefix that resumes is a replay of itself, whatever the bytes claimed.
        if let Ok(resumed) = RunRecorder::resume_with_pack(&run, Pack::bundled()) {
            // The recorder writes schema 2: a schema 1 prefix resumes as its schema 2 form.
            if run.setup.schema_version == SCHEMA_VERSION {
                assert_eq!(resumed.replay(), run, "a resumed prefix replays as another");
            } else {
                let again = resumed.replay();
                assert_eq!(
                    (&again.final_hash, again.ticks),
                    (&run.final_hash, run.ticks),
                    "a resumed schema 1 prefix replays as another run"
                );
                assert!(
                    run_replay::verify_run(&again.to_bytes()).is_ok(),
                    "a resumed schema 1 prefix does not verify as schema 2"
                );
            }
        }
    }
    let stage_replay = replay::verify(CLEAR_STAGE, data).is_ok();
    let verified = run_replay::verify_run(data);
    let run_replay = verified.is_ok();
    if run_replay {
        let run = RunReplay::from_bytes(data).expect("a run replay that verified loads");
        // The recorder writes schema 2, so only a schema 2 run re-records to itself (a schema 1
        // run re-records as its schema 2 form).
        if run.setup.schema_version == SCHEMA_VERSION {
            let recorded = recorder_until(&run, None).replay();
            assert_eq!(recorded, run, "a verified run replay records as another");
        }
        assert_eq!(
            run.checkpoints.len(),
            16 * usize::try_from(run.ticks / run_replay::CHECKPOINT_TICKS).unwrap(),
            "a verified run replay holds one checkpoint per 60 ticks"
        );
        if verified.is_ok_and(|v| v.outcome == Outcome::Running) {
            assert!(
                RunRecorder::resume_with_pack(&run, Pack::bundled()).is_ok(),
                "a verified run still running does not resume"
            );
        }
    }
    stage_replay || run_replay
}

/// The run the savepoint target resumes: a corpus run case that pins the bundled pack, cleared
/// in 297 ticks, with its one open offer at tick 230.
const SAVEPOINT_RUN: &[u8] = include_bytes!("../../tests/corpus/current/runs/tutorial/run.replay");

/// The ticks the savepoint target's prefixes are cut at: running, and with the offer open.
const SAVEPOINT_CUTS: [u32; 2] = [150, 230];

/// Ticks the target steps a loaded run on.
const SAVEPOINT_EXTRA_STEPS: usize = 20;

/// A record's dense entries, one per slot of `mask`, placed at their slots.
fn slots<T: Copy + Default>(mask: u8, dense: &[T]) -> [T; 3] {
    let mut all = [T::default(); 3];
    let mut from = dense.iter();
    for (slot, to) in all.iter_mut().enumerate() {
        if mask >> slot & 1 == 1 {
            *to = *from.next().expect("one entry per present slot");
        }
    }
    all
}

fn slot_of(n: u8) -> PlayerIdx {
    PlayerIdx::new(n).expect("a slot of a verified run")
}

/// The recorder of `run` (a verified run of any party size, from slot 0) driven by the run's own
/// step log up to world tick `cut`, before the picks made at that tick, or to its end for `None`:
/// the party is read from the log's `present` byte, so a party replay re-records as it is.
///
/// # Panics
///
/// If `run` does not start, or its log does not replay (callers pass a verified run).
pub fn recorder_until(run: &RunReplay, cut: Option<u32>) -> RunRecorder {
    let end = cut.unwrap_or(run.ticks);
    let range = run
        .step_range(0, end, MAX_TICKS)
        .expect("a verified run's step log reads");
    let party = u8::try_from(range.present.count_ones()).expect("a party of at most 3");
    let mut recorder = RunRecorder::start_party_with_pack(&run.setup, Pack::bundled(), party)
        .expect("a verified run starts");
    for step in &range.steps {
        match step {
            RangeStep::Actions { actions } => {
                let all = slots(recorder.engine().present(), actions);
                recorder
                    .actions(&all)
                    .expect("a verified run's actions are accepted");
            }
            RangeStep::Ticks { run, inputs } => {
                for _ in 0..*run {
                    let all = slots(recorder.engine().present(), inputs);
                    recorder.step(&all).expect("a verified run steps");
                }
            }
            RangeStep::Picks { picks } => {
                let all = slots(recorder.engine().present(), picks);
                recorder
                    .picks(&all)
                    .expect("a verified run's picks are accepted");
            }
            RangeStep::Join { slot } => recorder
                .join(slot_of(*slot))
                .expect("a verified run's joins are accepted"),
            RangeStep::Leave { slot } => recorder
                .leave(slot_of(*slot))
                .expect("a verified run's leaves are accepted"),
        }
    }
    recorder
}

/// The fixed run of the savepoint target, played by its own steps up to `cut` ticks (before the
/// picks that follow that tick), or to its end for `None`.
pub fn savepoint_recorder(cut: Option<u32>) -> RunRecorder {
    let run = RunReplay::from_bytes(SAVEPOINT_RUN).expect("the savepoint run is a run replay");
    recorder_until(&run, cut)
}

/// The party run of the savepoint target: a corpus party case (3 players, the time_limit mode),
/// in which slots 1 and 2 answer an offer of their own in the middle of the run.
const PARTY_RUN: &[u8] =
    include_bytes!("../../tests/corpus/current/parties/three_time_limit/run.replay");

/// The ticks the party target's prefixes are cut at: running, and with the first offer of
/// slot 1 or 2 open (found in the run's step log).
fn party_cuts() -> [u32; 2] {
    let run = RunReplay::from_bytes(PARTY_RUN).expect("the party run is a run replay");
    let range = run
        .step_range(0, run.ticks, MAX_TICKS)
        .expect("the party run's step log reads");
    let mut world = 0;
    for step in &range.steps {
        match step {
            RangeStep::Ticks { run, .. } => world += run,
            RangeStep::Picks { picks } if picks[1..].iter().any(Option::is_some) => {
                return [150, world];
            }
            RangeStep::Picks { .. }
            | RangeStep::Actions { .. }
            | RangeStep::Join { .. }
            | RangeStep::Leave { .. } => {}
        }
    }
    panic!("a pick by slot 1 or 2");
}

/// The party run of the savepoint target played to `cut` ticks (before the picks that follow
/// that tick), or to its end for `None`.
pub fn party_recorder(cut: Option<u32>) -> RunRecorder {
    let run = RunReplay::from_bytes(PARTY_RUN).expect("the party run is a run replay");
    recorder_until(&run, cut)
}

/// The party run with joins and leaves of the savepoint target: the corpus case `leave_hole`
/// (slot 1 leaves at 900, slot 2 at 1500 and comes back at 2100).
const CHANGE_RUN: &[u8] =
    include_bytes!("../../tests/corpus/current/parties/leave_hole/run.replay");

/// The ticks the change target's prefixes are cut at: inside the first hole (`0b101`), alone
/// (`0b001`) and in the second hole.
const CHANGE_CUTS: [u32; 3] = [1000, 1600, 2200];

/// The change run played to `cut` ticks (before the picks that follow that tick), or to its end.
pub fn change_recorder(cut: Option<u32>) -> RunRecorder {
    let run = RunReplay::from_bytes(CHANGE_RUN).expect("the change run is a run replay");
    recorder_until(&run, cut)
}

/// The timed party run of the savepoint target: the corpus case `timed_two` (two players, offers
/// that stay open while the world runs, the timer closing the ones nobody answers).
const TIMED_RUN: &[u8] = include_bytes!("../../tests/corpus/current/parties/timed_two/run.replay");

/// The ticks the timed target's prefixes are cut at: offers are open and running toward their
/// deadlines in the middle of the run.
pub const TIMED_CUTS: [u32; 2] = [700, 2000];

/// The timed run played to `cut` ticks (before the picks that follow that tick), or to its end.
pub fn timed_recorder(cut: Option<u32>) -> RunRecorder {
    let run = RunReplay::from_bytes(TIMED_RUN).expect("the timed run is a run replay");
    recorder_until(&run, cut)
}

/// The replays a savepoint is loaded against: the solo run at each of `SAVEPOINT_CUTS` and whole
/// (a finished run), then the party run at its two cuts and whole.
pub fn savepoint_prefixes() -> &'static [RunReplay] {
    static PREFIXES: OnceLock<Vec<RunReplay>> = OnceLock::new();
    PREFIXES.get_or_init(|| {
        SAVEPOINT_CUTS
            .iter()
            .map(|cut| Some(*cut))
            .chain([None])
            .map(|cut| savepoint_recorder(cut).replay())
            .chain(
                party_cuts()
                    .iter()
                    .map(|cut| Some(*cut))
                    .chain([None])
                    .map(|cut| party_recorder(cut).replay()),
            )
            .chain(
                CHANGE_CUTS
                    .iter()
                    .map(|cut| change_recorder(Some(*cut)).replay()),
            )
            .chain(
                TIMED_CUTS
                    .iter()
                    .map(|cut| timed_recorder(Some(*cut)).replay()),
            )
            .collect()
    })
}

/// The savepoint loader (ADR-SAVE-4, ADR-SVC-1). The bytes are a savepoint file, tried against
/// each prefix of `savepoint_prefixes()` with the bundled pack. The refusal is one of the seven
/// typed errors (the `match` is exhaustive); a savepoint that loads is the canonical bytes of
/// the run it holds: its replay is the prefix, saving it again gives the same bytes, its state
/// hashes as the prefix says, and it steps on without a panic, answering an open offer with
/// choice 0. Returns whether some prefix accepted the bytes.
pub fn savepoint_load(data: &[u8]) -> bool {
    let mut loaded = false;
    for prefix in savepoint_prefixes() {
        match RunRecorder::resume_from_savepoint(data, prefix, Pack::bundled()) {
            Ok(mut recorder) => {
                loaded = true;
                assert_eq!(
                    recorder.replay(),
                    *prefix,
                    "a loaded run has another prefix"
                );
                let saved = recorder.save();
                assert!(saved == data, "a loaded savepoint is not canonical");
                assert_eq!(
                    replay::hash_to_hex(recorder.engine().state_hash()),
                    prefix.final_hash,
                    "a loaded state hashes otherwise than the prefix says"
                );
                for _ in 0..SAVEPOINT_EXTRA_STEPS {
                    if recorder.engine().outcome() != Outcome::Running {
                        break;
                    }
                    let moved = if recorder.engine().offer_open() {
                        // The slot that holds the offer answers it with choice 0 (an absent
                        // slot, in a hole, is refused by `check_pick`).
                        let holder = (0..3u8)
                            .filter_map(PlayerIdx::new)
                            .find(|p| recorder.engine().check_pick(*p, 0).is_ok())
                            .unwrap_or(PlayerIdx::SOLO);
                        recorder.choose(holder, 0)
                    } else {
                        recorder.step(&[Input::default(); 3])
                    };
                    if moved.is_err() {
                        break;
                    }
                }
            }
            Err(
                SavepointError::Format
                | SavepointError::SimVersion
                | SavepointError::Pack
                | SavepointError::Damaged
                | SavepointError::HashMismatch
                | SavepointError::PrefixMismatch
                | SavepointError::Ended,
            ) => {}
        }
    }
    loaded
}

/// The seeds of `savepoint_load`, named for what they are: valid savepoints of the target's run,
/// one file per refusal (`SavepointError`) made by the byte that causes it, truncations, single
/// byte flips, a trailing byte and a length field of 0xff. Generated, not committed: the bytes
/// move with `SAVEPOINT_FORMAT` and `SIM_VERSION`.
pub fn savepoint_seeds() -> Vec<(String, Vec<u8>)> {
    const PAYLOAD_LEN_AT: usize = 92;
    const CHECKSUM_AT: usize = 96;
    let valid = savepoint_recorder(Some(150)).save();
    let reseal = |mut bytes: Vec<u8>| {
        let mut sealed = bytes[..CHECKSUM_AT].to_vec();
        sealed.extend_from_slice(&bytes[HEADER_LEN..]);
        bytes[CHECKSUM_AT..CHECKSUM_AT + 32].copy_from_slice(&sha256(&sealed));
        bytes
    };
    let edited = |at: usize, f: &dyn Fn(u8) -> u8| {
        let mut bytes = valid.clone();
        bytes[at] = f(bytes[at]);
        bytes
    };
    let mut seeds = vec![
        (
            "valid-tick100".to_owned(),
            savepoint_recorder(Some(100)).save(),
        ),
        ("valid-tick150".to_owned(), valid.clone()),
        (
            "valid-tick230".to_owned(),
            savepoint_recorder(Some(230)).save(),
        ),
        ("valid-end".to_owned(), savepoint_recorder(None).save()),
        (
            "valid-party-tick150".to_owned(),
            party_recorder(Some(150)).save(),
        ),
        (
            "valid-party-open".to_owned(),
            party_recorder(Some(party_cuts()[1])).save(),
        ),
        (
            "valid-timed-tick700".to_owned(),
            timed_recorder(Some(700)).save(),
        ),
        (
            "valid-hole".to_owned(),
            change_recorder(Some(CHANGE_CUTS[0])).save(),
        ),
        (
            "valid-alone-after-leaves".to_owned(),
            change_recorder(Some(CHANGE_CUTS[1])).save(),
        ),
        (
            "valid-second-hole".to_owned(),
            change_recorder(Some(CHANGE_CUTS[2])).save(),
        ),
        ("format".to_owned(), edited(4, &|b| b.wrapping_add(1))),
        ("sim-version".to_owned(), edited(8, &|b| b.wrapping_add(1))),
        (
            "schema-version".to_owned(),
            // Schema 2 is readable; 3 is not.
            reseal(edited(12, &|b| b.wrapping_add(2))),
        ),
        ("pack".to_owned(), reseal(edited(16, &|b| b ^ 1))),
        ("damaged".to_owned(), edited(valid.len() - 1, &|b| b ^ 1)),
        ("hash-mismatch".to_owned(), reseal(edited(52, &|b| b ^ 1))),
    ];
    for (name, len) in [
        ("trunc-0", 0),
        ("trunc-1", 1),
        ("trunc-half", valid.len() / 2),
        ("trunc-last", valid.len() - 1),
    ] {
        seeds.push((name.to_owned(), valid[..len].to_vec()));
    }
    for k in 0..16 {
        let at = valid.len() * k / 16;
        seeds.push((format!("flip-{at:04}"), edited(at, &|b| b ^ 0xff)));
    }
    // The equipment block is the last thing the state hash covers: the ledger and the tallies
    // follow it. Its end is the last byte whose flip is a `HashMismatch` (the replay prefix
    // of the valid savepoint is its own run's). Each seed replaces its 17 empty bytes, moves
    // the payload length by the difference and reseals.
    let prefix = savepoint_recorder(Some(150)).replay();
    let equipment_end = (HEADER_LEN..valid.len())
        .rev()
        .find(|&at| {
            RunRecorder::resume_from_savepoint(
                &reseal(edited(at, &|b| b ^ 1)),
                &prefix,
                Pack::bundled(),
            )
            .err()
                == Some(SavepointError::HashMismatch)
        })
        .expect("a state byte")
        + 1;
    let with_tail = |tail: &[u8]| {
        let mut bytes = valid[..equipment_end - 17].to_vec();
        bytes.extend_from_slice(tail);
        bytes.extend_from_slice(&valid[equipment_end..]);
        let len = u32::from_le_bytes(
            valid[PAYLOAD_LEN_AT..PAYLOAD_LEN_AT + 4]
                .try_into()
                .unwrap(),
        ) - 17
            + u32::try_from(tail.len()).unwrap();
        bytes[PAYLOAD_LEN_AT..PAYLOAD_LEN_AT + 4].copy_from_slice(&len.to_le_bytes());
        reseal(bytes)
    };
    let rest = [0u8; 8]; // starlight and tree points
    let tail = |parts: &[&[u8]]| [parts.concat(), rest.to_vec()].concat();
    seeds.push((
        "equipment-inventory-over-bound".to_owned(),
        with_tail(&tail(&[&u32::MAX.to_le_bytes(), &[0; 4], &[0]])),
    ));
    seeds.push((
        "equipment-slots-over-bound".to_owned(),
        with_tail(&tail(&[&[0; 4], &u32::MAX.to_le_bytes(), &[0]])),
    ));
    // One rune slot (kind 1, empty, expertise 0) at level 11, over the cap of 10.
    seeds.push((
        "equipment-level-over-cap".to_owned(),
        with_tail(&tail(&[
            &[0; 4],
            &1u32.to_le_bytes(),
            &[1, 0, 11],
            &[0; 4],
            &[0],
        ])),
    ));
    // A Lumine slot (kind 2, empty, level 1) in a run whose setup names no Lumine star.
    seeds.push((
        "equipment-lumine-without-setup".to_owned(),
        with_tail(&tail(&[&[0; 8], &[1, 2, 0, 1], &[0; 4]])),
    ));
    let mut trailing = valid.clone();
    trailing.push(0);
    seeds.push(("trailing".to_owned(), trailing));
    let mut long = valid.clone();
    long[PAYLOAD_LEN_AT..PAYLOAD_LEN_AT + 4].fill(0xff);
    seeds.push(("len-ff".to_owned(), long));
    seeds
}

/// A solo run replay's commands in the order they were made, read from its step log, so a run
/// with equipment actions is covered: a pick is a `Choose`, an action rides in the `Input.action`
/// of the tick that follows it, as `record_run_with` takes it. A party has no solo commands and
/// panics here, as does an action that does not directly precede its tick.
pub fn commands(run: &RunReplay) -> Vec<Command> {
    let range = run
        .step_range(0, run.ticks, MAX_TICKS)
        .expect("a verified step log reads");
    assert_eq!(range.present, 1, "a party has no solo commands");
    let mut out = Vec::new();
    let mut action = None;
    for step in &range.steps {
        match step {
            RangeStep::Ticks { run, inputs } => {
                for _ in 0..*run {
                    let mut input = inputs[0];
                    input.action = action.take();
                    out.push(Command::Step(input));
                }
            }
            RangeStep::Picks { picks } => {
                assert!(action.is_none(), "an action must precede its tick");
                out.push(Command::Choose(picks[0].expect("a solo pick")));
            }
            RangeStep::Actions { actions } => {
                assert!(action.is_none(), "an action must precede its tick");
                action = Some(actions[0].expect("a solo action"));
            }
            RangeStep::Join { .. } | RangeStep::Leave { .. } => {
                panic!("a party has no solo commands")
            }
        }
    }
    assert!(action.is_none(), "an action must precede its tick");
    out
}

/// The share-code decoder (ADR-SVC-1, ADR-REPL-6). The bytes are tried as a code, and wrapped
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

/// The content pack loader (ADR-SVC-1, ADR-CONT-5): a pack that loads loads again as the same pack,
/// its hash is the SHA-256 of its bytes, it saves (compactly) to a pack of the same content,
/// and its content starts a run of each mode that survives `RUN_TICKS` ticks of any input
/// without a panic (ADR-SIM-3). The bytes are the pack and, read over and over, the inputs.
pub fn pack_load(data: &[u8]) -> bool {
    let Ok(pack) = Pack::from_bytes(data) else {
        return false;
    };
    let again = Pack::from_bytes(data).expect("a pack that loaded loads again");
    assert_eq!(again, pack, "the same bytes loaded as another pack");
    let file: PackFile = serde_json::from_slice(data).expect("a pack that loaded parses");
    let saved = serde_json::to_vec(&file).expect("a pack serialises");
    let resaved = Pack::from_bytes(&saved).expect("a saved pack loads");
    // The files are the same bytes in the new layout; only the pack's own hash differs.
    assert_eq!(pack.content().pack_sha256(), Some(pack.sha256()));
    assert_eq!(resaved.content().pack_sha256(), Some(resaved.sha256()));
    let runes = pack.content().runes().clone();
    assert_eq!(
        resaved.content().clone().with_runes(runes.clone()),
        pack.content().clone().with_runes(runes),
        "a saved pack loads as other content"
    );
    assert_eq!(
        resaved.trial(),
        pack.trial(),
        "a saved pack loads as another stage"
    );
    for mode in ["trial", "time_limit"] {
        let setup = RunSetup {
            schema_version: 1,
            sim_version: pack.sim_version(),
            mode: mode.into(),
            weapon: "giant_sword".into(),
            weapon_tree: None,
            seed: u64::from(pick(data, 0)),
            star_pool: None,
            extra_rerolls: 0,
            upgrades: Vec::new(),
            perk_set: "duelist".into(),
            rune_pool: Vec::new(),
            carried: Vec::new(),
            lumine: 0,
            starting_rune: None,
            stat_priority: Vec::new(),
            pick_stats: false,
            difficulty: "normal".into(),
            ..RunSetup::default()
        };
        let mut engine = Engine::start_with(&setup, pack.content())
            .expect("a pack that loads starts a run of each mode");
        for (t, input) in inputs_from(data, RUN_TICKS).into_iter().enumerate() {
            step_spending(&mut engine, input, pick(data, t));
            let _ = engine.snapshot();
        }
    }
    settles_everything(data, &pack);
    true
}

/// Settles every mode the game holds at every difficulty of the pack, both outcomes, with a run
/// result and a bank read out of `data` (a bank up to `u64::MAX`, a save can be edited): the
/// books balance, no bank passes its cap, and what a clear opens is in the file.
fn settles_everything(data: &[u8], pack: &Pack) {
    let content = pack.content();
    let word = |at: usize, bytes: usize| {
        (0..bytes).fold(0u64, |acc, i| (acc << 8) | u64::from(pick(data, at + i)))
    };
    let earned = u32::try_from(word(1, 4) % (u64::from(MAX_RUN_STARDUST) + 1)).unwrap_or(0);
    let held = word(5, 8);
    let file: Vec<&ContentId> = content
        .difficulties()
        .list()
        .iter()
        .map(|d| &d.id)
        .collect();
    for mode in ["trial", "time_limit", "tutorial"] {
        let mode = ContentId::from(mode);
        let cap = content
            .stardust()
            .mode(mode.as_str())
            .map_or(u64::MAX, |m| u64::from(m.bank_cap) * 10);
        for difficulty in &file {
            let bank = Bank::new(vec![((mode.clone(), (*difficulty).clone()), held)]);
            for outcome in [RunOutcome::Cleared, RunOutcome::Failed] {
                let run = RunResult {
                    mode: mode.clone(),
                    difficulty: (*difficulty).clone(),
                    outcome,
                    stardust_tenths: earned,
                };
                let s = settle(content, &run, &bank);
                assert!(s.paid_tenths <= u64::from(earned), "paid more than earned");
                assert_eq!(s.stardust_tenths, s.paid_tenths + s.bank_paid_tenths);
                assert!(s.bank_paid_tenths <= cap, "paid a bank over its cap");
                if outcome == RunOutcome::Failed {
                    assert_eq!(
                        s.paid_tenths + s.banked_tenths + s.lost_tenths,
                        u64::from(earned),
                        "a fail's books do not balance"
                    );
                    assert!(s.opened.is_empty() && s.rune_shards == 0);
                }
                let after = s.bank_after.get(&mode, difficulty);
                let banking = content
                    .stardust()
                    .mode(mode.as_str())
                    .is_some_and(|m| m.bank);
                match (banking, outcome) {
                    (true, RunOutcome::Cleared) => {
                        assert_eq!(after, 0, "a clear left a bank");
                        assert_eq!(s.bank_paid_tenths, held.min(cap));
                    }
                    (true, RunOutcome::Failed) => {
                        assert_eq!(
                            after,
                            held.min(cap) + s.banked_tenths,
                            "a bank passed its cap"
                        );
                    }
                    (false, _) => assert_eq!(after, held, "a bank that is off moved"),
                }
                for opened in &s.opened {
                    assert!(
                        file.contains(&opened),
                        "a clear opened a difficulty outside the file"
                    );
                }
            }
        }
    }
}

/// The `packs.json` manifest loader (FT1a, ADR-SVC-1): a manifest that loads loads again as the
/// same manifest, every channel is found under its own key, and holds an `https://` url and a
/// tag the checks allow; bytes over the cap are refused without being parsed.
pub fn packs_load(data: &[u8]) -> bool {
    let result = Packs::from_bytes(data);
    if data.len() > Packs::MAX_BYTES {
        assert!(
            matches!(result, Err(PacksError::TooLarge { .. })),
            "bytes over the cap were parsed"
        );
    }
    let Ok(packs) = result else {
        return false;
    };
    let again = Packs::from_bytes(data).expect("a manifest that loaded loads again");
    assert_eq!(again, packs, "the same bytes loaded as another manifest");
    assert!(packs.channels().count() <= stella_rain_core::packs::MAX_CHANNELS);
    for (key, channel) in packs.channels() {
        assert_eq!(packs.channel(key), Some(channel));
        assert!(channel.url().starts_with("https://"));
        assert!(!channel.tag().is_empty());
    }
    true
}

/// The placeholder key id the `minisign_verify` seed was signed with; not core's trusted key.
pub const MINISIGN_TEST_KEY_ID: [u8; 8] = *b"STELLA01";

/// The placeholder public key the `minisign_verify` seed was signed with; not core's trusted key.
pub const MINISIGN_TEST_PUBLIC_KEY: [u8; 32] = [
    23, 56, 28, 152, 98, 105, 33, 97, 209, 110, 106, 198, 68, 166, 87, 110, 159, 114, 144, 255, 72,
    135, 166, 167, 4, 21, 106, 174, 117, 135, 18, 94,
];

/// The manifest the `minisign_verify` seed signature covers.
pub const MINISIGN_SEED_MANIFEST: &[u8] = br#"{"schema":1,"seq":1,"channels":{"0.1":{"tag":"v0.1.0","pack_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","url":"https://content.stella-rain.com/pack.json"}}}"#;

/// The `minisign_verify` fuzz target (SG1a, DS49): untrusted signature bytes through
/// `verify_packs_with_keys` against the placeholder key, so a signature reaches the check:
/// the shipped `verify_packs` would refuse every seed at the key-id gate. Never panics on
/// garbage, malformed base64, untrusted keys or invalid signatures.
pub fn minisign_verify(data: &[u8]) -> bool {
    let Ok(packs) = stella_rain_core::pack::trust::verify_packs_with_keys(
        MINISIGN_SEED_MANIFEST,
        data,
        &[MINISIGN_TEST_KEY_ID],
        &[MINISIGN_TEST_PUBLIC_KEY],
    ) else {
        return false;
    };
    assert_eq!(packs.seq(), 1);
    true
}

/// The bytes `step_log_range` reads: `ticks`, `a` and `b` as little-endian `u32`, then the step
/// log.
pub fn step_log_range_input(ticks: u32, a: u32, b: u32, log: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + log.len());
    for n in [ticks, a, b] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(log);
    out
}

/// One entry per tick, and the picks where they fall: the oracle's form of a range.
#[derive(Debug, PartialEq)]
enum RangeEntry {
    Tick(Vec<Input>),
    Picks(Vec<Option<u32>>),
    Actions(Vec<Option<EquipAction>>),
    Join(u8),
    Leave(u8),
}

/// The whole log expanded to one entry per tick, cut to `a..b` by the written rule and not by the
/// reader's code: a tick `t` is in iff `a <= t < b`; a PICKS or ACTIONS after `p` ticks is in iff
/// `a <= p < b`, or `b == ticks` and `p == ticks` and the range is not empty; a JOIN or LEAVE
/// goes by the same rule as a PICKS.
fn range_oracle(full: &StepRange, ticks: u32, a: u32, b: u32) -> Vec<RangeEntry> {
    let mut out = Vec::new();
    let mut world = 0u32;
    for step in &full.steps {
        match step {
            RangeStep::Ticks { run, inputs } => {
                for t in world..world + run {
                    if a <= t && t < b {
                        out.push(RangeEntry::Tick(inputs.clone()));
                    }
                }
                world += run;
            }
            RangeStep::Picks { picks } => {
                let inside = (a <= world && world < b) || (a < b && b == ticks && world == ticks);
                if inside {
                    out.push(RangeEntry::Picks(picks.clone()));
                }
            }
            RangeStep::Actions { actions } => {
                let inside = (a <= world && world < b) || (a < b && b == ticks && world == ticks);
                if inside {
                    out.push(RangeEntry::Actions(actions.clone()));
                }
            }
            RangeStep::Join { slot } | RangeStep::Leave { slot } => {
                let inside = (a <= world && world < b) || (a < b && b == ticks && world == ticks);
                if inside {
                    out.push(if matches!(step, RangeStep::Join { .. }) {
                        RangeEntry::Join(*slot)
                    } else {
                        RangeEntry::Leave(*slot)
                    });
                }
            }
        }
    }
    out
}

fn expand_range(range: &StepRange) -> Vec<RangeEntry> {
    let mut out = Vec::new();
    for step in &range.steps {
        match step {
            RangeStep::Ticks { run, inputs } => {
                out.extend((0..*run).map(|_| RangeEntry::Tick(inputs.clone())));
            }
            RangeStep::Picks { picks } => out.push(RangeEntry::Picks(picks.clone())),
            RangeStep::Actions { actions } => out.push(RangeEntry::Actions(actions.clone())),
            RangeStep::Join { slot } => out.push(RangeEntry::Join(*slot)),
            RangeStep::Leave { slot } => out.push(RangeEntry::Leave(*slot)),
        }
    }
    out
}

/// The mask after every change before world tick `first`, by the written rule: the log's first
/// byte, then each JOIN or LEAVE after `p < first` world ticks.
fn mask_before(full: &StepRange, first: u32) -> u8 {
    let (mut mask, mut world) = (full.present, 0u32);
    for step in &full.steps {
        match step {
            RangeStep::Ticks { run, .. } => world += run,
            RangeStep::Join { slot } if world < first => mask |= 1 << slot,
            RangeStep::Leave { slot } if world < first => mask &= !(1 << slot),
            _ => {}
        }
    }
    mask
}

/// The step log range reader (ADR-REPL-5, ADR-SVC-1). The bytes are `ticks`, `a`, `b` (little-endian
/// `u32`) and a schema 2 step log. A range that is backwards or past `ticks` is refused; for a
/// range inside the log the verdict is the full read's, error for error, because the whole log is
/// validated; and when both succeed on a log of at most 65,536 ticks the range equals the full
/// read expanded to one entry per tick and cut by the written rule. Returns whether the log got
/// past the scanner.
pub fn step_log_range(data: &[u8]) -> bool {
    const EXPAND_LIMIT: u32 = 65_536;
    let Some((head, log)) = data.split_at_checked(12) else {
        return false;
    };
    let word = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
    let (ticks, a, b) = (word(0), word(4), word(8));
    if a > b || b > ticks {
        assert!(
            read_step_range(log, ticks, u32::MAX, a, b).is_err(),
            "a bad range is read"
        );
    }
    // A range inside the log: the raw one when it is, else the raw pair folded into it.
    let (x, y) = if a <= b && b <= ticks {
        (a, b)
    } else {
        let fold = |n: u32| u32::try_from(u64::from(n) % (u64::from(ticks) + 1)).unwrap_or(0);
        (fold(a).min(fold(b)), fold(a).max(fold(b)))
    };
    let full = read_step_range(log, ticks, u32::MAX, 0, ticks);
    let part = read_step_range(log, ticks, u32::MAX, x, y);
    match (&full, &part) {
        (Ok(full), Ok(part)) => {
            assert_eq!((part.first, part.end, part.present), (x, y, full.present));
            assert_eq!(
                part.present_at_first,
                mask_before(full, x),
                "the mask at {x}"
            );
            assert!(
                part.steps.len() <= full.steps.len(),
                "a range has more steps"
            );
            if ticks <= EXPAND_LIMIT {
                assert_eq!(
                    expand_range(part),
                    range_oracle(full, ticks, x, y),
                    "{x}..{y} of {ticks}"
                );
            }
        }
        (Err(full), Err(part)) => assert_eq!(full, part, "the range read refuses another way"),
        _ => panic!("the range read and the full read disagree on {x}..{y}"),
    }
    full.is_ok()
}
