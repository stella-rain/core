//! The scripted run of the trial and the other modes as a list of commands: `run_input`'s
//! ticks and every offer answered (`common::run::answer_all`); the setup's stat priority spends the
//! level-ups. Beside the common fixtures, not in them, so that the benches that
//! share `tests/common/` do not build it.

use crate::common;
use crate::common::run::{STEER_FROM, answer_all, trial_setup};
use crate::zone_bot::run_input;
use stella_rain_core::engine::{Engine, Outcome};
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_content::RunContent;
use stella_rain_core::run_replay::{Command, RunRecorder};
use stella_rain_core::run_setup::{OfferMode, RunSetup};

/// The commands of a run of `run_input` for up to `ticks` ticks, answering every offer as
/// `answer_all` does, between a tick and the next (and after the last).
pub fn play_run(ticks: u32) -> Vec<Command> {
    play_run_from(&trial_setup(), ticks)
}

/// `play_run` from `setup` instead of the trial's.
pub fn play_run_from(setup: &RunSetup, ticks: u32) -> Vec<Command> {
    play_run_in(setup, RunContent::bundled(), ticks)
}

/// `play_run_from` with `content` in place of the bundled files.
pub fn play_run_in(setup: &RunSetup, content: &RunContent, ticks: u32) -> Vec<Command> {
    play_run_with(setup, content, ticks, run_input)
}

/// `play_run_in` with `input` choosing each tick's buttons.
pub fn play_run_with(
    setup: &RunSetup,
    content: &RunContent,
    ticks: u32,
    input: impl Fn(&Engine, u32) -> Input,
) -> Vec<Command> {
    let mut engine = Engine::start_with(setup, content).unwrap();
    let mut out = Vec::new();
    let mut offers = 0;
    let between = |engine: &mut Engine, out: &mut Vec<Command>, offers: &mut usize| {
        answer_all(engine, offers, out);
    };
    for t in 0..ticks {
        if engine.outcome() != Outcome::Running {
            break;
        }
        between(&mut engine, &mut out, &mut offers);
        let input = input(&engine, t);
        engine.step(&[input, Default::default(), Default::default()]);
        out.push(Command::Step(input));
    }
    between(&mut engine, &mut out, &mut offers);
    out
}

/// The tick at whose end the run's first level-up comes: the one that raises the first stat.
/// Offers may open before it.
pub fn first_point_tick() -> u32 {
    let mut engine = Engine::start(&trial_setup()).unwrap();
    for command in play_run(STEER_FROM) {
        match command {
            Command::Step(input) => engine.step(&[input, Default::default(), Default::default()]),
            Command::Choose(index) => common::pick(&mut engine, index).unwrap(),
        }
        if engine.snapshot().progress.is_some_and(|p| p.level > 1) {
            return engine.tick();
        }
    }
    panic!("the script gains no level before tick {STEER_FROM}");
}

/// The scripted input of slot `slot` (1 or 2) of a party at `tick`: distinct patterns, so a
/// log carries each slot. Slots 1 and 2 are passive in the rules for now.
pub fn scripted_input(slot: u8, tick: u32) -> Input {
    match slot {
        1 => Input {
            move_y: 50,
            skill: tick % 90 < 30,
            ..Input::default()
        },
        _ => Input {
            move_x: -30,
            evade: tick % 120 < 10,
            ..Input::default()
        },
    }
}

/// The inputs of a party of `n` at `tick`: slot 0 played by the zone bot, the others scripted.
pub fn party_inputs(rec: &RunRecorder, tick: u32) -> [Input; 3] {
    let mut inputs = [Input::default(); 3];
    let present = rec.engine().present();
    for s in (0..3u8).filter(|s| present >> s & 1 == 1) {
        inputs[usize::from(s)] = match s {
            0 => run_input(rec.engine(), tick),
            _ => scripted_input(s, tick),
        };
    }
    inputs
}

/// Answers every open offer, each by the slot that holds it (option 0), one pick at a time.
pub fn answer_party_offers(rec: &mut RunRecorder) {
    while rec.engine().offer_open() {
        let present = rec.engine().present();
        let holder = (0..3u8)
            .filter(|s| present >> s & 1 == 1)
            .find(|i| {
                rec.engine()
                    .check_pick(PlayerIdx::new(*i).unwrap(), 0)
                    .is_ok()
            })
            .expect("an open offer belongs to a slot");
        rec.choose(PlayerIdx::new(holder).unwrap(), 0).unwrap();
    }
}

/// A change of the party at a tick.
#[derive(Clone, Copy, Debug)]
pub enum Change {
    Join(u8),
    Leave(u8),
}

/// A party of `n` played from `setup` for up to `ticks` ticks, every offer answered by its slot
/// (one pick at a time between ticks); each of `changes` (world tick, change) is applied before
/// that tick's step. In a timed run no offer waits for a pick: slot 0 answers an offer (option 0)
/// `answer_after` ticks after it opened, every second offer of its own, in the input of a tick;
/// the other slots never answer, and an offer nobody answers is closed by the timer. `None`
/// answers nothing.
pub fn play_party(
    setup: &RunSetup,
    n: u8,
    ticks: u32,
    changes: &[(u32, Change)],
    answer_after: Option<u32>,
) -> RunRecorder {
    let timed = setup.offer_mode == OfferMode::Timed;
    let mut rec = RunRecorder::start_party(setup, n).unwrap();
    let slot0 = PlayerIdx::SOLO;
    // The world tick slot 0's current offer opened on, and the offers it has met.
    let (mut open_since, mut met): (Option<u32>, u32) = (None, 0);
    for t in 0..ticks {
        for (_, change) in changes.iter().filter(|(at, _)| *at == t) {
            match *change {
                Change::Join(s) => rec.join(PlayerIdx::new(s).unwrap()).unwrap(),
                Change::Leave(s) => rec.leave(PlayerIdx::new(s).unwrap()).unwrap(),
            }
        }
        if !timed {
            answer_party_offers(&mut rec);
        }
        if rec.engine().outcome() != Outcome::Running {
            break;
        }
        let mut inputs = party_inputs(&rec, t);
        if timed {
            if rec.engine().check_pick(slot0, 0).is_ok() {
                let since = *open_since.get_or_insert_with(|| {
                    met += 1;
                    rec.engine().tick()
                });
                let waited = rec.engine().tick() - since;
                if met % 2 == 1 && answer_after.is_some_and(|n| waited >= n) {
                    inputs[0].pick = Some(0);
                    open_since = None;
                }
            } else {
                open_since = None;
            }
        }
        rec.step(&inputs).unwrap();
        // An offer taken (by a pick or by the timer) ends the wait: the next one counts afresh.
        if timed
            && rec
                .engine()
                .events()
                .iter()
                .any(|ev| matches!(ev, stella_rain_core::event::DomainEvent::RewardTaken { .. }))
        {
            open_since = None;
        }
    }
    if !timed {
        answer_party_offers(&mut rec);
    }
    rec
}

/// A party case of the corpus (`tests/corpus/current/parties/<name>/`): `party` players on
/// `setup`, played for up to `ticks` ticks by `play_party`.
pub struct PartyCase {
    pub name: &'static str,
    pub setup: RunSetup,
    pub party: u8,
    pub ticks: u32,
    /// Joins and leaves, by world tick.
    pub changes: Vec<(u32, Change)>,
    /// A timed case: the ticks slot 0 waits after an offer opens before it answers it (every
    /// second one); `None` answers none and leaves them to the timer.
    pub answer_after: Option<u32>,
}

impl PartyCase {
    /// The recording of the case.
    pub fn play(&self) -> RunRecorder {
        play_party(
            &self.setup,
            self.party,
            self.ticks,
            &self.changes,
            self.answer_after,
        )
    }
}

/// The party cases. Slots 1 and 2 get an offer of their own only in some modes and seeds (the
/// seeds are found where, in the middle of the run, each answers an offer of its own): the
/// time_limit mode for two and for three. In the tutorial both slots answer, but only as the run
/// is cleared; in time_limit slot 1 answers in the middle of the run and slot 2 is not offered one
/// (no seed of 1 to 300 gives both).
pub fn party_cases() -> Vec<PartyCase> {
    let at = |mode: &str, seed: u64| RunSetup {
        mode: mode.into(),
        seed,
        ..trial_setup()
    };
    let timed = |seed: u64| RunSetup {
        offer_mode: OfferMode::Timed,
        ..at("time_limit", seed)
    };
    vec![
        PartyCase {
            name: "two_time_limit",
            setup: at("time_limit", 40),
            party: 2,
            ticks: 3000,
            changes: vec![],
            answer_after: None,
        },
        PartyCase {
            name: "three_tutorial",
            setup: at("tutorial", 4),
            party: 3,
            ticks: 3000,
            changes: vec![],
            answer_after: None,
        },
        PartyCase {
            name: "three_time_limit",
            setup: at("time_limit", 4),
            party: 3,
            ticks: 3000,
            changes: vec![],
            answer_after: None,
        },
        // A solo run that takes slot 1 at tick 600; slot 1 answers an offer after it.
        PartyCase {
            name: "join_two",
            setup: at("time_limit", 1),
            party: 1,
            ticks: 3000,
            changes: vec![(600, Change::Join(1))],
            answer_after: None,
        },
        // A party of 3 that loses slot 1 (a hole, 0b101) and slot 2 (alone), then takes slot 2
        // back (0b101); slot 2 answers an offer inside the first hole window.
        PartyCase {
            name: "leave_hole",
            setup: at("time_limit", 2),
            party: 3,
            ticks: 3000,
            changes: vec![
                (900, Change::Leave(1)),
                (1500, Change::Leave(2)),
                (2100, Change::Join(2)),
            ],
            answer_after: None,
        },
        // A timed party of one: the world keeps running with an offer open; slot 0 answers every
        // second offer 100 ticks after it opened and the timer closes the rest.
        PartyCase {
            name: "timed_one",
            setup: timed(1),
            party: 1,
            ticks: 3000,
            changes: vec![],
            answer_after: Some(100),
        },
        // A timed party of two: slot 0 answers as above, slot 1 never does, so the timer
        // closes every offer of slot 1.
        PartyCase {
            name: "timed_two",
            setup: timed(1),
            party: 2,
            ticks: 3000,
            changes: vec![],
            answer_after: Some(100),
        },
    ]
}
