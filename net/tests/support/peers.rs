//! `n` net sessions that exchange real `Packet` bytes over a [`SimNet`], so a test can see them
//! converge on the straight run's hash under latency, jitter, loss, reordering and duplication,
//! and stop together on a desync.
//!
//! The library does the protocol (the handshake, frame advantage, the redundant send and ack, the
//! checkpoint exchange and the stop); this harness only moves bytes through the endpoints, calls
//! `advance` or `poll` once per round per peer, and keeps the books. A [`Link`] is the one place
//! a test changes a message: it can alter an input frame on its way out or drop checkpoints.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use stella_rain_core::id::PlayerIdx;
use stella_rain_core::run_replay::content_sha256;
use stella_rain_core::version::SIM_VERSION;
use stella_rain_net::session::party::{
    NetAdvance, NetConfig, NetError, NetSession, NetWait, PartySettings, Phase, StopReason,
};
use stella_rain_net::session::{DEFAULT_DELAY, DEFAULT_WINDOW};
use stella_rain_net::transport::{Transport, TransportError};
use stella_rain_net::wire::{Hello, InputFrame, PROTOCOL_VERSION, Packet};

use super::simnet::{Impair, SimEndpoint, SimNet};
use super::{Script, engine, equipment_setup, slot};

/// The most rounds a run may take.
pub const ROUND_LIMIT: u32 = 4000;

/// A change made to a frame before it is encoded: `from`, `to`, the frame.
pub type FrameTamper = Box<dyn FnMut(usize, usize, &mut InputFrame)>;

/// The `Hello` every honest peer of the test run sends.
pub fn hello() -> Hello {
    Hello {
        protocol: PROTOCOL_VERSION,
        sim_version: SIM_VERSION,
        pack_hash: content_sha256(equipment_setup().mode.as_str()).expect("a mode with content"),
        delay: DEFAULT_DELAY,
        window: DEFAULT_WINDOW,
    }
}

/// What the links of a run do to a message on its way out.
#[derive(Default)]
struct Rules {
    tamper: Option<FrameTamper>,
    drop_checkpoints_to: Vec<usize>,
}

/// One session's [`Transport`]: its simnet endpoint plus the run's [`Rules`].
pub struct Link {
    ep: SimEndpoint,
    me: usize,
    rules: Rc<RefCell<Rules>>,
}

impl Transport for Link {
    fn send(&mut self, to: PlayerIdx, packet: &[u8]) -> Result<(), TransportError> {
        let mut out = packet.to_vec();
        if let Ok(decoded) = Packet::decode(packet) {
            let mut rules = self.rules.borrow_mut();
            match decoded {
                Packet::Checkpoint(_) if rules.drop_checkpoints_to.contains(&to.get()) => {
                    return Ok(());
                }
                Packet::Input(mut frame) => {
                    if let Some(tamper) = rules.tamper.as_mut() {
                        tamper(self.me, to.get(), &mut frame);
                        out = Packet::Input(frame).encode().expect("a short packet");
                    }
                }
                _ => {}
            }
        }
        self.ep.send(to, &out)
    }

    fn recv(&mut self) -> Option<(PlayerIdx, Vec<u8>)> {
        self.ep.recv()
    }
}

/// What one `advance` call of a net session did.
#[derive(Clone, Copy, Debug)]
pub struct NetCall {
    /// The session's tick before the call.
    pub tick_before: u32,
    /// What the call returned.
    pub out: NetAdvance,
}

/// The sessions, the network between them and the books of the run.
pub struct Peers {
    /// The sessions, by slot.
    pub sessions: Vec<NetSession<Link>>,
    /// Every `advance` call of each session.
    pub calls: Vec<Vec<NetCall>>,
    /// The network.
    pub net: SimNet,
    /// The script every peer plays.
    pub script: Script,
    rules: Rc<RefCell<Rules>>,
    settings: PartySettings,
    max_skew: u32,
    stops: Vec<u32>,
    leaver: Option<(usize, u32)>,
    announced: bool,
    taken: Vec<u32>,
    per_round: Vec<u32>,
}

impl Peers {
    /// `n` sessions on `script` over a net with `seed` and `impair`: the default settings, each
    /// stopping at tick 612.
    pub fn new(script: Script, n: u8, seed: u64, impair: Impair) -> Peers {
        let count = usize::from(n);
        let mut peers = Peers {
            sessions: Vec::new(),
            calls: vec![Vec::new(); count],
            net: SimNet::new(n, seed, impair),
            script,
            rules: Rc::new(RefCell::new(Rules::default())),
            settings: PartySettings::default(),
            max_skew: stella_rain_net::session::party::DEFAULT_MAX_SKEW,
            stops: vec![612; count],
            leaver: None,
            announced: false,
            taken: vec![0; count],
            per_round: vec![1; count],
        };
        peers.rebuild();
        peers
    }

    /// Builds the sessions again from the current settings; only before the first round.
    fn rebuild(&mut self) {
        let n = self.script.n;
        self.sessions = (0..n)
            .map(|i| {
                let mut config = NetConfig::new(slot(i), hello());
                config.settings = self.settings;
                config.max_skew = self.max_skew;
                let link = Link {
                    ep: self.net.endpoint(i),
                    me: usize::from(i),
                    rules: Rc::clone(&self.rules),
                };
                NetSession::new(engine(n), config, link).expect("a valid config")
            })
            .collect();
    }

    /// The party's delay and window for every session (the script must have been made for the
    /// same delay).
    pub fn settings(mut self, settings: PartySettings) -> Peers {
        self.settings = settings;
        self.rebuild();
        self
    }

    /// The frame-advantage skew of every session; `u32::MAX` turns the wait off.
    pub fn max_skew(mut self, max_skew: u32) -> Peers {
        self.max_skew = max_skew;
        self.rebuild();
        self
    }

    /// Slot `who` calls `advance` `calls` times per round (a faster clock).
    pub fn calls_per_round(mut self, who: usize, calls: u32) -> Peers {
        self.per_round[who] = calls;
        self
    }

    /// Every `Checkpoint` sent to slot `who` is dropped on its way out.
    pub fn drop_checkpoints_to(self, who: usize) -> Peers {
        self.rules.borrow_mut().drop_checkpoints_to.push(who);
        self
    }

    /// Slot `who` leaves before tick `at`: it announces the leave when its last input is queued,
    /// stops advancing at `at` and keeps polling.
    pub fn leaver(mut self, who: usize, at: u32) -> Peers {
        self.stops[who] = at;
        self.leaver = Some((who, at));
        self
    }

    /// Changes every input frame on its way out: `from`, `to`, the frame.
    pub fn set_tamper(&mut self, tamper: FrameTamper) {
        self.rules.borrow_mut().tamper = Some(tamper);
    }

    /// Whether every session has reached its stop tick or has stopped.
    pub fn done(&self) -> bool {
        self.sessions
            .iter()
            .zip(&self.stops)
            .all(|(s, stop)| s.phase() == Phase::Stopped || s.session().tick() >= *stop)
    }

    /// Runs rounds, [`Peers::check`] after each, until `done` or [`ROUND_LIMIT`]; whether it
    /// finished.
    pub fn run(&mut self, oracle: &[u64]) -> bool {
        self.run_with(|p| p.check(oracle))
    }

    /// Runs rounds, calling `each` after every one, until `done` or [`ROUND_LIMIT`]; whether it
    /// finished.
    pub fn run_with(&mut self, mut each: impl FnMut(&Peers)) -> bool {
        while !self.done() {
            if self.net.round() >= ROUND_LIMIT {
                return false;
            }
            self.round();
            each(self);
        }
        true
    }

    /// One round: the net moves on, then each peer in slot order announces a due leave and
    /// advances (or polls once it is finished or stopped).
    pub fn round(&mut self) {
        self.net.step_round();
        for i in 0..self.sessions.len() {
            if let Some((who, at)) = self.leaver
                && who == i
                && !self.announced
                && self.taken[i] + self.settings.delay == at
            {
                let stamp = self.sessions[i].leave().expect("the leaver announces");
                assert_eq!(stamp, at);
                self.announced = true;
            }
            let live = self.sessions[i].phase() != Phase::Stopped
                && self.sessions[i].session().tick() < self.stops[i];
            if !live || self.per_round[i] == 0 {
                let _ = self.sessions[i].poll();
                continue;
            }
            for _ in 0..self.per_round[i] {
                let tick_before = self.sessions[i].session().tick();
                if tick_before >= self.stops[i] {
                    break;
                }
                let k = self.taken[i] + self.settings.delay;
                let input = self
                    .script
                    .eff
                    .get(k as usize)
                    .map_or_else(Default::default, |row| row[i]);
                match self.sessions[i].advance(&input) {
                    Ok(out) => {
                        if out.waiting.is_none() {
                            self.taken[i] += 1;
                        }
                        self.calls[i].push(NetCall { tick_before, out });
                    }
                    Err(NetError::Stopped) => break,
                    Err(e) => panic!("round {}, peer {i}: {e}", self.net.round()),
                }
            }
        }
    }

    /// The window holds and every confirmed hash is the straight run's; a session stopped on a
    /// desync is skipped.
    pub fn check(&self, oracle: &[u64]) {
        self.check_below(oracle, u32::MAX);
    }

    /// As [`Peers::check`], for the sessions whose confirmed tick is at most `limit`.
    pub fn check_below(&self, oracle: &[u64], limit: u32) {
        for (i, s) in self.sessions.iter().enumerate() {
            if self.desync_stopped(i) {
                continue;
            }
            let s = s.session();
            assert!(
                s.tick() - s.confirmed() <= s.config().window,
                "session {i}: tick {} confirmed {}",
                s.tick(),
                s.confirmed()
            );
            if s.confirmed() <= limit {
                assert_eq!(
                    s.hash_after(s.confirmed()),
                    Some(oracle[s.confirmed() as usize]),
                    "session {i} at confirmed tick {}",
                    s.confirmed()
                );
            }
        }
    }

    /// Whether session `i` stopped on a desync (its own or a peer's).
    pub fn desync_stopped(&self, i: usize) -> bool {
        matches!(
            self.sessions[i].stop_reason(),
            Some(StopReason::Desync { .. } | StopReason::PeerDesync { .. })
        )
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

    /// Whether any call of a session other than `except` re-simulated across `tick`: it began
    /// at or before `tick` and had stepped past it.
    pub fn rolled_across(&self, tick: u32, except: usize) -> bool {
        self.calls
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != except)
            .flat_map(|(_, calls)| calls)
            .any(|c| c.tick_before - c.out.rolled_back <= tick && tick < c.tick_before)
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

    /// The ticks re-simulated by session `who` over the run.
    pub fn rolled_by(&self, who: usize) -> u64 {
        self.calls[who]
            .iter()
            .map(|c| u64::from(c.out.rolled_back))
            .sum()
    }

    /// How many calls of session `who`, or of all with `None`, waited for `reason`, or for any
    /// reason with `None`.
    pub fn waits_of(&self, who: Option<usize>, reason: Option<NetWait>) -> usize {
        self.calls
            .iter()
            .enumerate()
            .filter(|(i, _)| who.is_none_or(|w| w == *i))
            .flat_map(|(_, calls)| calls)
            .filter(|c| c.out.waiting.is_some() && (reason.is_none() || c.out.waiting == reason))
            .count()
    }

    /// How many calls waited, for any reason, or for `reason` only.
    pub fn waits(&self, reason: Option<NetWait>) -> usize {
        self.waits_of(None, reason)
    }
}

/// A tamper that flips `main` and negates `move_x` of every input of ticks `ticks` that slot
/// `from` sends to slot `to`, by tick, so every resend of a tick is the same.
pub fn flip(from: usize, to: usize, ticks: std::ops::Range<u32>) -> FrameTamper {
    Box::new(move |f, t, frame| {
        if f == from && t == to {
            for (tick, input) in (frame.start_tick..).zip(&mut frame.inputs) {
                if ticks.contains(&tick) {
                    input.main = !input.main;
                    input.move_x = input.move_x.saturating_neg();
                }
            }
        }
    })
}
