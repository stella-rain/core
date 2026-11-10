//! Shared test support for the net crate: the run every test starts from, a scripted bot, and an
//! oracle that steps a straight engine with no session in between. Each test binary uses part of
//! it, so unused items are allowed.
#![allow(dead_code)]

use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::{EquipAction, Input};
use stella_rain_core::run_setup::RunSetup;

/// The run every net test starts: the trial with the giant sword at `sim_version` 2, the
/// equipment mode.
pub fn equipment_setup() -> RunSetup {
    RunSetup {
        seed: 20_261_010,
        ..RunSetup::default()
    }
}

/// An engine of `n` players (1 to 3) at tick 0 on [`equipment_setup`].
pub fn engine(n: u8) -> Engine {
    Engine::start_party(&equipment_setup(), n).expect("the equipment setup starts")
}

/// The slot `i`.
pub fn slot(i: u8) -> PlayerIdx {
    PlayerIdx::new(i).expect("slot 0 to 2")
}

/// A small deterministic generator (xorshift64*).
pub struct Xorshift(pub u64);

impl Xorshift {
    /// A generator from `seed`; a zero seed is replaced.
    pub fn new(seed: u64) -> Xorshift {
        Xorshift(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// The next 64 bits.
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value below `bound` (not zero).
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// What every peer is to play: the effective input of each slot at each tick, and the ticks at
/// which a slot leaves. `eff[t]` is stepped at tick `t`; its `action` is made before the step.
#[derive(Clone, Debug)]
pub struct Script {
    /// The party size the script was made for.
    pub n: u8,
    /// The input of every slot at every tick.
    pub eff: Vec<[Input; 3]>,
    /// `(tick, slot)`: the slot leaves before tick `tick` is stepped.
    pub leaves: Vec<(u32, PlayerIdx)>,
}

impl Script {
    /// The number of ticks the script covers.
    pub fn ticks(&self) -> u32 {
        u32::try_from(self.eff.len()).expect("a short script")
    }
}

/// Applies one tick the way a recorder does: the leaves stamped at `tick` (slot order), the
/// actions in slot order (a refusal is ignored), then the step on inputs with `action` cleared.
pub fn apply_tick(
    engine: &mut Engine,
    tick: u32,
    inputs: &[Input; 3],
    leaves: &[(u32, PlayerIdx)],
) {
    let mut gone = [false; 3];
    for (t, who) in leaves {
        if *t == tick {
            gone[who.get()] = true;
        }
    }
    for (i, g) in gone.iter().enumerate() {
        if *g {
            let _ = engine.leave(slot(u8::try_from(i).expect("small")));
        }
    }
    for (i, input) in inputs.iter().enumerate() {
        let who = slot(u8::try_from(i).expect("small"));
        if engine.present() >> who.get() & 1 == 1
            && let Some(action) = input.action
        {
            let _ = engine.act(who, action);
        }
    }
    let mut cleared = *inputs;
    for input in &mut cleared {
        input.action = None;
    }
    engine.step(&cleared);
}

/// Every equipment action this bot would try, in a fixed order.
fn candidates() -> Vec<EquipAction> {
    let mut all = vec![EquipAction::TreePick { option: 0 }];
    for item in 0..4 {
        for slot in 0..6 {
            all.push(EquipAction::Equip { item, slot });
            all.push(EquipAction::Replace { item, slot });
        }
    }
    all.push(EquipAction::Extract { item: 0 });
    for star in 0..8 {
        all.push(EquipAction::Craft { star });
    }
    all
}

/// The bot's input for `who` at the tick `engine` is on: a wandering stick, held buttons now and
/// then, and every few ticks an equipment action the engine accepts (chosen among those it
/// accepts, so the kinds vary). Stick axes stay within -127 to 127 (the wire form).
fn bot_input(engine: &Engine, who: PlayerIdx, tick: u32, rng: &mut Xorshift) -> Input {
    let leg = tick / 24;
    let mut leg_rng =
        Xorshift::new(u64::from(leg) * 3 + u64::try_from(who.get()).expect("small") + 1);
    leg_rng.next();
    let axis = |r: &mut Xorshift| {
        i8::try_from(i64::try_from(r.below(255)).expect("small") - 127).expect("-127 to 127")
    };
    let mut input = Input {
        move_x: axis(&mut leg_rng),
        move_y: axis(&mut leg_rng),
        main: tick % 3 != 0,
        smash: tick % 50 > 44,
        evade: tick % 90 == 5,
        skill: tick % 70 == 30,
        assist: leg % 2 == 0,
        ..Input::default()
    };
    if tick % 5 == 0 {
        let accepted: Vec<EquipAction> = candidates()
            .into_iter()
            .filter(|a| engine.check_action(who, *a).is_ok())
            .collect();
        if !accepted.is_empty() {
            let chosen = usize::try_from(rng.below(accepted.len() as u64)).expect("small");
            input.action = Some(accepted[chosen]);
        }
    }
    input
}

/// Plays `n` players for `ticks` ticks with the bot and records the effective inputs. Ticks
/// below `delay` are neutral (every peer's first inputs are); a slot's inputs from its leave tick
/// on are neutral. `leave` is `(slot, tick)`.
pub fn script(n: u8, ticks: u32, delay: u32, leave: Option<(PlayerIdx, u32)>) -> Script {
    let leaves: Vec<(u32, PlayerIdx)> = leave.into_iter().map(|(who, t)| (t, who)).collect();
    let mut e = engine(n);
    let mut rng = Xorshift::new(0xC0FF_EE11);
    let mut eff = Vec::new();
    for t in 0..ticks {
        let mut row = [Input::default(); 3];
        if t >= delay {
            for i in 0..n {
                let who = slot(i);
                let left = leaves.iter().any(|(lt, w)| *w == who && *lt <= t);
                if !left && e.present() >> i & 1 == 1 {
                    row[usize::from(i)] = bot_input(&e, who, t, &mut rng);
                }
            }
        }
        apply_tick(&mut e, t, &row, &leaves);
        eff.push(row);
    }
    Script { n, eff, leaves }
}

/// The oracle: a fresh engine of `n` players stepped through the script, no session involved.
/// `hashes[k]` is the state hash after `k` ticks, so the vector is one longer than the script.
pub fn straight(script: &Script, n: u8) -> Vec<u64> {
    let mut e = engine(n);
    let mut hashes = vec![e.state_hash()];
    for (t, row) in script.eff.iter().enumerate() {
        let t = u32::try_from(t).expect("short");
        apply_tick(&mut e, t, row, &script.leaves);
        hashes.push(e.state_hash());
    }
    hashes
}

/// What a [`CountingSave`] has seen.
#[derive(Default)]
pub struct SaveLog {
    saved: std::collections::BTreeMap<u32, Engine>,
    /// Calls to `save`.
    pub saves: u32,
    /// Calls to `load`.
    pub loads: u32,
    /// The most saves held at once, measured after every `save`.
    pub max_held: usize,
}

impl SaveLog {
    /// How many saves are held now.
    pub fn held(&self) -> usize {
        self.saved.len()
    }
}

/// A [`Save`] with no capacity of its own: it holds what the session has not forgotten, so a
/// test sees how many saves the session keeps, and counts the calls. The handle is shared: keep
/// a clone to read the log while the session owns the other.
#[derive(Clone, Default)]
pub struct CountingSave(pub std::rc::Rc<std::cell::RefCell<SaveLog>>);

impl stella_rain_net::save::Save for CountingSave {
    fn save(&mut self, tick: u32, engine: &Engine) {
        let mut log = self.0.borrow_mut();
        log.saves += 1;
        log.saved.insert(tick, engine.clone());
        log.max_held = log.max_held.max(log.saved.len());
    }

    fn load(&mut self, tick: u32, engine: &mut Engine) -> bool {
        let mut log = self.0.borrow_mut();
        log.loads += 1;
        match log.saved.get(&tick) {
            Some(saved) => {
                *engine = saved.clone();
                true
            }
            None => false,
        }
    }

    fn forget_before(&mut self, tick: u32) {
        let mut log = self.0.borrow_mut();
        log.saved = log.saved.split_off(&tick);
    }
}

use stella_rain_net::session::{Advance, Config, Session, Wait};
use stella_rain_net::wire::{InputFrame, WireInput};

/// The wire form of `who`'s scripted inputs for ticks `from..=to`, as a peer sends them.
pub fn frame_of(script: &Script, who: u8, from: u32, to: u32) -> InputFrame {
    InputFrame {
        slot: slot(who),
        start_tick: from,
        inputs: (from..=to)
            .map(|t| WireInput::from_input(&script.eff[t as usize][usize::from(who)]))
            .collect(),
    }
}

/// What one `advance` call of a mesh session did.
#[derive(Clone, Copy, Debug)]
pub struct CallStat {
    /// The session's tick before the call.
    pub tick_before: u32,
    /// What the call returned.
    pub out: Advance,
}

enum Msg {
    Frame(InputFrame),
    Ack(u32),
    Leave(u32),
}

struct Pending {
    due: u32,
    seq: u64,
    from: usize,
    to: usize,
    msg: Msg,
}

/// The lateness, in rounds, of the message `from -> to` sent in `round`.
pub type Lateness<'a> = Box<dyn FnMut(usize, usize, u32) -> u32 + 'a>;
/// A change made to a frame as it is sent: `from`, `to`, the frame.
pub type Tamper<'a> = Box<dyn FnMut(usize, usize, &mut InputFrame) + 'a>;

/// `n` sessions that exchange frames and acks with a delay per message. One round: the messages
/// due are delivered, every session that has not reached its stop tick advances once on its
/// scripted input (a waiting call is tried again next round with the same input), then every
/// session sends each peer its unacknowledged inputs and an ack. No loss and no duplicate.
pub struct Mesh<'a> {
    /// The sessions, by slot.
    pub sessions: Vec<Session>,
    /// Every `advance` call of each session.
    pub calls: Vec<Vec<CallStat>>,
    /// The next round.
    pub round: u32,
    script: &'a Script,
    delay: u32,
    stops: Vec<u32>,
    leaver: Option<(usize, u32)>,
    announced: bool,
    taken: Vec<u32>,
    acked: Vec<[u32; 3]>,
    pending: Vec<Pending>,
    seq: u64,
    lateness: Lateness<'a>,
    /// Changes a frame as it is sent, if set.
    pub tamper: Option<Tamper<'a>>,
}

impl<'a> Mesh<'a> {
    /// `n` sessions on `script`, each stopping at `stop` ticks.
    pub fn new(
        script: &'a Script,
        n: u8,
        delay: u32,
        window: u32,
        stop: u32,
        lateness: Lateness<'a>,
    ) -> Mesh<'a> {
        let sessions = (0..n)
            .map(|i| {
                let config = Config {
                    delay,
                    window,
                    ..Config::new(slot(i))
                };
                Session::new(engine(n), config).expect("a valid config")
            })
            .collect();
        Mesh {
            sessions,
            calls: vec![Vec::new(); usize::from(n)],
            round: 0,
            script,
            delay,
            stops: vec![stop; usize::from(n)],
            leaver: None,
            announced: false,
            taken: vec![0; usize::from(n)],
            acked: vec![[0; 3]; usize::from(n)],
            pending: Vec::new(),
            seq: 0,
            lateness,
            tamper: None,
        }
    }

    /// Slot `who` leaves before tick `at`: it announces that `delay` calls ahead, stops
    /// advancing at `at` and is sent no more frames.
    pub fn leaver(mut self, who: usize, at: u32) -> Mesh<'a> {
        self.stops[who] = at;
        self.leaver = Some((who, at));
        self
    }

    /// Whether session `i` has left: it is the leaver and has stopped.
    fn gone(&self, i: usize) -> bool {
        self.leaver
            .is_some_and(|(who, at)| who == i && self.sessions[i].tick() >= at)
    }

    /// Whether every session has reached its stop tick. A session confirms ticks inside
    /// `advance`, so what arrives after a session stopped stays unconfirmed.
    pub fn done(&self) -> bool {
        self.sessions
            .iter()
            .zip(&self.stops)
            .all(|(s, stop)| s.tick() >= *stop)
    }

    /// Runs rounds until `done` or `limit` rounds have passed.
    pub fn run(&mut self, limit: u32) {
        for _ in 0..limit {
            if self.done() {
                break;
            }
            self.round();
        }
    }

    /// One round.
    pub fn round(&mut self) {
        let round = self.round;
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.due <= round);
        self.pending = later;
        let mut due = due;
        due.sort_by_key(|p| (p.due, p.seq));
        for p in due {
            if self.gone(p.to) && matches!(p.msg, Msg::Frame(_) | Msg::Leave(_)) {
                continue;
            }
            let from = slot(u8::try_from(p.from).expect("small"));
            match p.msg {
                Msg::Frame(frame) => self.sessions[p.to]
                    .receive_inputs(from, &frame)
                    .expect("a valid frame"),
                Msg::Ack(next) => {
                    self.sessions[p.to].ack(from, next).expect("a valid ack");
                    self.acked[p.to][p.from] = self.acked[p.to][p.from].max(next);
                }
                Msg::Leave(at) => self.sessions[p.to].leave(from, at).expect("a valid leave"),
            }
        }
        for i in 0..self.sessions.len() {
            if let Some((who, at)) = self.leaver
                && who == i
                && !self.announced
                && self.taken[i] + self.delay == at
            {
                let me = slot(u8::try_from(i).expect("small"));
                self.sessions[i]
                    .leave(me, at)
                    .expect("the leaver announces");
                self.announced = true;
            }
            if self.sessions[i].tick() < self.stops[i] {
                let k = self.taken[i] + self.delay;
                let input = self
                    .script
                    .eff
                    .get(k as usize)
                    .map_or_else(Default::default, |row| row[i]);
                let tick_before = self.sessions[i].tick();
                let out = self.sessions[i].advance(&input).expect("advance");
                if out.waiting.is_none() {
                    self.taken[i] += 1;
                }
                self.calls[i].push(CallStat { tick_before, out });
            }
        }
        for i in 0..self.sessions.len() {
            for j in 0..self.sessions.len() {
                if i == j {
                    continue;
                }
                let mut msgs = Vec::new();
                if self.announced && self.leaver.is_some_and(|(who, _)| who == i) {
                    msgs.push(Msg::Leave(self.leaver.map_or(0, |(_, at)| at)));
                }
                msgs.push(Msg::Ack(
                    self.sessions[i].next_tick(slot(u8::try_from(j).expect("small"))),
                ));
                if let Some(mut frame) = self.sessions[i].local_frame(self.acked[i][j]) {
                    if let Some(tamper) = self.tamper.as_mut() {
                        tamper(i, j, &mut frame);
                    }
                    msgs.push(Msg::Frame(frame));
                }
                for msg in msgs {
                    let lat = (self.lateness)(i, j, round);
                    self.seq += 1;
                    self.pending.push(Pending {
                        due: round + lat,
                        seq: self.seq,
                        from: i,
                        to: j,
                        msg,
                    });
                }
            }
        }
        self.round += 1;
    }

    /// The window holds and every confirmed hash is the straight run's.
    pub fn check(&self, oracle: &[u64]) {
        for (i, s) in self.sessions.iter().enumerate() {
            assert!(
                s.tick() - s.confirmed() <= s.config().window,
                "session {i}: tick {} confirmed {}",
                s.tick(),
                s.confirmed()
            );
            assert_eq!(
                s.hash_after(s.confirmed()),
                Some(oracle[s.confirmed() as usize]),
                "session {i} at confirmed tick {}",
                s.confirmed()
            );
        }
    }

    /// Whether any call re-simulated a tick that holds an action.
    pub fn rolled_over_an_action(&self) -> bool {
        self.calls.iter().flatten().any(|c| {
            (c.tick_before - c.out.rolled_back..c.tick_before).any(|t| {
                self.script.eff[t as usize]
                    .iter()
                    .any(|input| input.action.is_some())
            })
        })
    }

    /// The most ticks one call re-simulated.
    pub fn max_rolled(&self) -> u32 {
        self.calls
            .iter()
            .flatten()
            .map(|c| c.out.rolled_back)
            .max()
            .unwrap_or(0)
    }

    /// The ticks re-simulated over the run.
    pub fn total_rolled(&self) -> u64 {
        self.calls
            .iter()
            .flatten()
            .map(|c| u64::from(c.out.rolled_back))
            .sum()
    }

    /// How many calls waited, for any reason, or for `reason` only.
    pub fn waits(&self, reason: Option<Wait>) -> usize {
        self.calls
            .iter()
            .flatten()
            .filter(|c| c.out.waiting.is_some() && (reason.is_none() || c.out.waiting == reason))
            .count()
    }
}

pub mod peers;
pub mod simnet;
pub mod wiretap;
