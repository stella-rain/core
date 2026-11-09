//! One replayer for a run case, offers-era or equipment (ADR-SIM-7, ADR-REPL-5). The solo views
//! (`unpack_inputs`, `unpack_commands`) refuse a log that holds an equipment action or a join or
//! leave; this reads the step log through `RunReplay::step_range`, which carries all of them, and
//! hands a test the steps in order, one per world tick and one per pick, action, join or leave.

use stella_rain_core::RangeStep;
use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{Command, RunRecorder, RunReplay};

/// One step of a run. Entries sit at their slots; an absent slot is `Default` / `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// One world tick. The inputs hold no `pick` and no `action` (the log holds none there).
    Tick([Input; 3]),
    /// One step that does not advance the world: the pick of each slot.
    Picks([Option<u32>; 3]),
    /// One step that does not advance the world: the equipment action of each slot.
    Actions([Option<EquipAction>; 3]),
    Join(PlayerIdx),
    Leave(PlayerIdx),
}

/// The steps of a run, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSteps {
    /// The party the log starts with (the slots set in its first byte).
    pub party: u8,
    /// World ticks in the run.
    pub ticks: u32,
    steps: Vec<Step>,
    /// `first[t]`: the index of the first step made while the world is at tick `t`.
    first: Vec<usize>,
}

fn spread<T: Copy + Default>(mask: u8, dense: &[T]) -> [T; 3] {
    let mut out = [T::default(); 3];
    for (slot, value) in (0..3).filter(|s| mask >> s & 1 == 1).zip(dense) {
        out[slot] = *value;
    }
    out
}

fn player(slot: u8) -> PlayerIdx {
    PlayerIdx::new(slot).expect("a slot of the log is 0 to 2")
}

impl RunSteps {
    /// Reads `replay`'s step log, either schema. Panics with the error on a damaged log: a test
    /// helper.
    pub fn of(replay: &RunReplay) -> RunSteps {
        let range = replay
            .step_range(0, replay.ticks, u32::MAX)
            .unwrap_or_else(|e| panic!("a recorded step log: {e:?}"));
        let party = u8::try_from(range.present.count_ones()).unwrap();
        let mut mask = range.present;
        let (mut steps, mut first) = (Vec::new(), vec![0]);
        for step in &range.steps {
            match step {
                RangeStep::Ticks { run, inputs } => {
                    let all = spread(mask, inputs);
                    for _ in 0..*run {
                        steps.push(Step::Tick(all));
                        first.push(steps.len());
                    }
                }
                RangeStep::Picks { picks } => steps.push(Step::Picks(spread(mask, picks))),
                RangeStep::Actions { actions } => steps.push(Step::Actions(spread(mask, actions))),
                RangeStep::Join { slot } => {
                    mask |= 1 << slot;
                    steps.push(Step::Join(player(*slot)));
                }
                RangeStep::Leave { slot } => {
                    mask &= !(1 << slot);
                    steps.push(Step::Leave(player(*slot)));
                }
            }
        }
        assert_eq!(first.len(), replay.ticks as usize + 1, "ticks add up");
        RunSteps {
            party,
            ticks: replay.ticks,
            steps,
            first,
        }
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The index of the first step made while the world stands at `tick` ticks: the picks,
    /// actions, joins and leaves made then are at and after it, and the tick `tick` itself
    /// follows them. `tick == ticks` gives the index of the steps after the last tick.
    pub fn at(&self, tick: u32) -> usize {
        self.first[tick as usize]
    }

    pub fn has_actions(&self) -> bool {
        self.steps.iter().any(|s| matches!(s, Step::Actions(_)))
    }
}

/// The engine `replay` starts: the party its log names, `content` as given.
pub fn start(replay: &RunReplay, content: &RunContent) -> Engine {
    let steps = RunSteps::of(replay);
    Engine::start_party_with(&replay.setup, content, steps.party).expect("a recorded setup starts")
}

/// Applies one step to a bare engine. A pick is the pick step of `common::pick` (it does not
/// advance the world) and panics with the engine's refusal; an action is `check_action` then
/// `act`, slot by slot.
pub fn apply(engine: &mut Engine, step: &Step) {
    match step {
        Step::Tick(inputs) => engine.step(inputs),
        Step::Picks(picks) => {
            let mut inputs = [Input::default(); 3];
            for (slot, pick) in picks.iter().enumerate() {
                if let Some(index) = *pick {
                    engine
                        .check_pick(player(u8::try_from(slot).unwrap()), index)
                        .unwrap_or_else(|e| panic!("pick {index} of slot {slot}: {e:?}"));
                    inputs[slot] = Input::picking(index);
                }
            }
            engine.step(&inputs);
        }
        Step::Actions(actions) => {
            for (slot, action) in actions.iter().enumerate() {
                if let Some(action) = *action {
                    let who = player(u8::try_from(slot).unwrap());
                    engine
                        .check_action(who, action)
                        .unwrap_or_else(|e| panic!("action {action:?} of slot {slot}: {e:?}"));
                    engine
                        .act(who, action)
                        .unwrap_or_else(|e| panic!("action {action:?} of slot {slot}: {e:?}"));
                }
            }
        }
        Step::Join(slot) => engine.join(*slot).expect("a recorded join"),
        Step::Leave(slot) => engine.leave(*slot).expect("a recorded leave"),
    }
}

/// Applies one step through a recorder, which records it as it was.
pub fn apply_to(rec: &mut RunRecorder, step: &Step) {
    match step {
        Step::Tick(inputs) => rec.step(inputs).expect("a recorded tick"),
        Step::Picks(picks) => rec.picks(picks).expect("a recorded pick"),
        Step::Actions(actions) => rec.actions(actions).expect("a recorded action"),
        Step::Join(slot) => rec.join(*slot).expect("a recorded join"),
        Step::Leave(slot) => rec.leave(*slot).expect("a recorded leave"),
    }
}

/// The solo commands `record_run_with` takes to write `steps` again: a PICKS step is a
/// `Choose`, an ACTIONS step goes into the `input.action` of the tick that follows it. Panics on
/// an ACTIONS step that is not directly followed by a tick, and on a party.
pub fn commands_of(party: u8, steps: &[Step]) -> Vec<Command> {
    assert_eq!(party, 1, "a party has no commands");
    let mut out = Vec::new();
    let mut action: Option<EquipAction> = None;
    for step in steps {
        if action.is_some() && !matches!(step, Step::Tick(_)) {
            panic!("an ACTIONS step that does not directly precede its tick");
        }
        match step {
            Step::Tick(inputs) => {
                let mut input = inputs[0];
                input.action = action.take();
                out.push(Command::Step(input));
            }
            Step::Picks(picks) => {
                out.push(Command::Choose(picks[0].expect("a solo pick is slot 0's")))
            }
            Step::Actions(actions) => action = Some(actions[0].expect("a solo action is slot 0's")),
            Step::Join(_) | Step::Leave(_) => panic!("a party has no commands"),
        }
    }
    assert!(
        action.is_none(),
        "an ACTIONS step that does not directly precede its tick"
    );
    out
}

/// `commands_of` for the whole run `replay` holds.
pub fn commands(replay: &RunReplay) -> Vec<Command> {
    let steps = RunSteps::of(replay);
    commands_of(steps.party, steps.steps())
}
