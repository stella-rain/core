//! `n` rollback sessions that exchange real `Packet` bytes over a [`SimNet`], so a test can see
//! them converge on the straight run's hash under latency, jitter, loss, reordering and
//! duplication.
//!
//! It is not a session layer: there is no handshake, no frame advantage and no checkpoint
//! exchange, and the redundant send (every round, each peer's unacknowledged inputs, an ack and,
//! once announced, a bye) is a test stand-in for the one the session layer will own.
#![allow(dead_code)]

use stella_rain_net::session::{Config, Session, Wait};
use stella_rain_net::transport::Transport;
use stella_rain_net::wire::{Ack, Bye, ByeReason, InputFrame, Packet};

use super::simnet::{Impair, SimEndpoint, SimNet};
use super::{CallStat, Script, engine, slot};

/// The most rounds a run may take.
pub const ROUND_LIMIT: u32 = 4000;

/// A change made to a frame before it is encoded: `from`, `to`, the frame.
pub type FrameTamper = Box<dyn FnMut(usize, usize, &mut InputFrame)>;

/// The sessions, the network between them and the books of the run.
pub struct Peers {
    /// The sessions, by slot.
    pub sessions: Vec<Session>,
    /// Every `advance` call of each session.
    pub calls: Vec<Vec<CallStat>>,
    /// The network.
    pub net: SimNet,
    /// The script every peer plays.
    pub script: Script,
    /// Changes a frame as it is sent, if set.
    pub tamper: Option<FrameTamper>,
    eps: Vec<SimEndpoint>,
    delay: u32,
    stops: Vec<u32>,
    leaver: Option<(usize, u32)>,
    announced: bool,
    taken: Vec<u32>,
    acked: Vec<[u32; 3]>,
}

impl Peers {
    /// `n` sessions on `script` over a net with `seed` and `impair`: delay 2, window 4, each
    /// stopping at tick 612.
    pub fn new(script: Script, n: u8, seed: u64, impair: Impair) -> Peers {
        let delay = 2;
        let net = SimNet::new(n, seed, impair);
        let sessions = (0..n)
            .map(|i| {
                let config = Config {
                    delay,
                    window: 4,
                    ..Config::new(slot(i))
                };
                Session::new(engine(n), config).expect("a valid config")
            })
            .collect();
        let count = usize::from(n);
        Peers {
            sessions,
            calls: vec![Vec::new(); count],
            eps: (0..n).map(|i| net.endpoint(i)).collect(),
            net,
            script,
            tamper: None,
            delay,
            stops: vec![612; count],
            leaver: None,
            announced: false,
            taken: vec![0; count],
            acked: vec![[0; 3]; count],
        }
    }

    /// Slot `who` leaves before tick `at`: it announces the leave `delay` calls ahead, stops
    /// advancing at `at` and keeps sending.
    pub fn leaver(mut self, who: usize, at: u32) -> Peers {
        self.stops[who] = at;
        self.leaver = Some((who, at));
        self
    }

    /// Whether session `i` has left: it is the leaver and has stopped.
    fn gone(&self, i: usize) -> bool {
        self.leaver
            .is_some_and(|(who, at)| who == i && self.sessions[i].tick() >= at)
    }

    /// Whether every session has stopped.
    pub fn done(&self) -> bool {
        self.sessions
            .iter()
            .zip(&self.stops)
            .all(|(s, stop)| s.tick() >= *stop)
    }

    /// Runs rounds, [`Peers::check`] after each, until `done` or [`ROUND_LIMIT`]; whether it
    /// finished.
    pub fn run(&mut self, oracle: &[u64]) -> bool {
        while !self.done() {
            if self.net.round() >= ROUND_LIMIT {
                return false;
            }
            self.round();
            self.check(oracle);
        }
        true
    }

    /// Drains the endpoint of session `me` and maps each packet onto the session.
    fn receive(&mut self, me: usize) {
        while let Some((from, bytes)) = self.eps[me].recv() {
            let packet = Packet::decode(&bytes).expect("an impairment never breaks a packet");
            let gone = self.gone(me);
            let round = self.net.round();
            let f = from.get();
            match packet {
                Packet::Input(_) | Packet::Bye(_) if gone => {}
                Packet::Input(frame) => self.sessions[me]
                    .receive_inputs(from, &frame)
                    .unwrap_or_else(|e| panic!("round {round}, peer {me} from {f}: frame: {e}")),
                Packet::Ack(ack) => {
                    assert_eq!(ack.slot.get(), me, "an ack names its receiver");
                    self.sessions[me]
                        .ack(from, ack.next_tick)
                        .unwrap_or_else(|e| panic!("round {round}, peer {me} from {f}: ack: {e}"));
                    self.acked[me][f] = self.acked[me][f].max(ack.next_tick);
                }
                Packet::Bye(bye) => {
                    assert_eq!((bye.slot, bye.reason), (from, ByeReason::Left));
                    self.sessions[me].leave(from, bye.tick).unwrap_or_else(|e| {
                        panic!("round {round}, peer {me} from {f}: leave: {e}")
                    });
                }
                other => panic!("the harness never sends {other:?}"),
            }
        }
    }

    /// One round: the net moves on, every peer reads, every peer advances once, every peer sends.
    pub fn round(&mut self) {
        self.net.step_round();
        let n = self.sessions.len();
        for me in 0..n {
            self.receive(me);
        }
        for i in 0..n {
            if let Some((who, at)) = self.leaver
                && who == i
                && !self.announced
                && self.taken[i] + self.delay == at
            {
                self.sessions[i]
                    .leave(slot(u8::try_from(i).expect("small")), at)
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
        for i in 0..n {
            for j in (0..n).filter(|j| *j != i) {
                let to = slot(u8::try_from(j).expect("small"));
                let mut packets = Vec::new();
                if let Some((who, at)) = self.leaver
                    && who == i
                    && self.announced
                {
                    packets.push(Packet::Bye(Bye {
                        slot: slot(u8::try_from(i).expect("small")),
                        tick: at,
                        reason: ByeReason::Left,
                    }));
                }
                packets.push(Packet::Ack(Ack {
                    slot: to,
                    next_tick: self.sessions[i].next_tick(to),
                }));
                if let Some(mut frame) = self.sessions[i].local_frame(self.acked[i][j]) {
                    if let Some(tamper) = self.tamper.as_mut() {
                        tamper(i, j, &mut frame);
                    }
                    packets.push(Packet::Input(frame));
                }
                for packet in packets {
                    let bytes = packet.encode().expect("a short packet");
                    self.eps[i].send(to, &bytes).expect("the net accepts it");
                }
            }
        }
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

    /// How many calls waited, for any reason, or for `reason` only.
    pub fn waits(&self, reason: Option<Wait>) -> usize {
        self.calls
            .iter()
            .flatten()
            .filter(|c| c.out.waiting.is_some() && (reason.is_none() || c.out.waiting == reason))
            .count()
    }
}
