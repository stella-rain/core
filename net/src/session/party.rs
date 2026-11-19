//! The net session: a [`Session`] behind a [`Transport`], with the handshake, the redundant send
//! and ack of inputs, frame advantage, the checkpoint exchange, the stop on desync and the
//! leader's end of a run. Every packet but the hello carries the run it belongs to; a session
//! plays run 0 until the leader restarts the run and drops a packet of any other run.

use std::collections::BTreeMap;

use stella_rain_core::engine::Engine;
use stella_rain_core::id::PlayerIdx;
use stella_rain_core::input::Input;
use stella_rain_core::run_replay::CHECKPOINT_TICKS;

use super::report::{DesyncReport, EndedRun};
use super::{ConfigError, DEFAULT_DELAY, DEFAULT_WINDOW, Session, SessionError, Wait};
use crate::save::{CloneRing, Save};
use crate::transport::Transport;
use crate::wire::{
    Ack, Bye, ByeReason, Checkpoint, CommandKind, Hello, InputFrame, LeaderCommand, Packet,
};

/// The first tick a Restart may be stamped at: a Restart below it is refused.
pub const MIN_RESTART_TICK: u32 = 60;
/// How far ahead of a peer (in ticks of lead difference) a session may run before it waits.
pub const DEFAULT_MAX_SKEW: u32 = 4;
/// Every how many send rounds the newest checkpoints are sent.
pub const CHECKPOINT_SEND_EVERY: u32 = 4;
/// How many of the newest checkpoints each such send holds.
pub const CHECKPOINTS_IN_FLIGHT: usize = 2;
/// How many checkpoints are kept per slot.
pub const CHECKPOINTS_KEPT: usize = 8;

/// The one delay and window of a party. Every peer must be built from the same value: the hello
/// carries both and the handshake refuses a peer whose delay or window differs (ADR-NET-6
/// point 3), so no input frame is exchanged between peers set up differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartySettings {
    /// Ticks between a local input being given and being stepped.
    pub delay: u32,
    /// The most ticks predicted past the confirmed one.
    pub window: u32,
}

impl Default for PartySettings {
    fn default() -> PartySettings {
        PartySettings {
            delay: DEFAULT_DELAY,
            window: DEFAULT_WINDOW,
        }
    }
}

/// How a [`NetSession`] is set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetConfig {
    /// The slot this session plays.
    pub local: PlayerIdx,
    /// The delay and window of the party.
    pub settings: PartySettings,
    /// What this session says about itself in the handshake. Its `delay` and `window` are
    /// replaced by those of `settings`, the one source.
    pub hello: Hello,
    /// The frame-advantage skew at which a session waits; `u32::MAX` turns the wait off.
    pub max_skew: u32,
}

impl NetConfig {
    /// The default settings and skew for the slot `local`, with `hello`.
    pub fn new(local: PlayerIdx, hello: Hello) -> NetConfig {
        NetConfig {
            local,
            settings: PartySettings::default(),
            hello,
            max_skew: DEFAULT_MAX_SKEW,
        }
    }
}

/// Why a [`NetConfig`] is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetConfigError {
    /// The hello names protocol 0 or `sim_version` 0.
    BadHello,
    /// The inner session refused the settings.
    Session(ConfigError),
}

impl std::fmt::Display for NetConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetConfigError::BadHello => f.write_str("the hello names protocol or sim_version 0"),
            NetConfigError::Session(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for NetConfigError {}

/// Where a session is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for every peer's hello.
    Handshake,
    /// Playing.
    Playing,
    /// Stopped for good: a refusal, a desync or the leader's end.
    Stopped,
}

/// Why an [`NetSession::advance`] did not step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetWait {
    /// Some peer's hello is still missing.
    Handshake,
    /// This session runs too far ahead of a peer (frame advantage).
    Ahead,
    /// The window of predicted ticks is full.
    Window,
    /// A peer has not acknowledged the local inputs a frame can carry.
    Unacked,
    /// This session has stepped to the tick the leader ends the run at and waits for the end.
    Ending,
}

impl From<Wait> for NetWait {
    fn from(wait: Wait) -> NetWait {
        match wait {
            Wait::Window => NetWait::Window,
            Wait::Unacked => NetWait::Unacked,
        }
    }
}

/// What one [`NetSession::advance`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetAdvance {
    /// Ticks re-simulated by this call.
    pub rolled_back: u32,
    /// Why no tick was stepped, if none was.
    pub waiting: Option<NetWait>,
}

/// What a peer's hello differs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The protocol version.
    Protocol,
    /// The simulation version.
    SimVersion,
    /// The content pack.
    Pack,
    /// The input delay.
    Delay,
    /// The rollback window.
    Window,
}

/// Why a session stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// This session refused the hello of `slot`.
    Refused {
        /// The peer.
        slot: PlayerIdx,
        /// What differed.
        why: Refusal,
    },
    /// The peer in the slot refused this session's hello.
    RefusedBy(PlayerIdx),
    /// This session's checkpoint hash differs from `slot`'s at `tick`.
    Desync {
        /// The checkpoint tick.
        tick: u32,
        /// The peer whose hash differed.
        slot: PlayerIdx,
    },
    /// The peer in `slot` announced a desync at `tick`.
    PeerDesync {
        /// The announcing peer.
        slot: PlayerIdx,
        /// The tick it announced.
        tick: u32,
    },
    /// The leader cancelled the run.
    EndedByLeader(CommandKind),
}

/// Why [`NetSession::end_run`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndRunError {
    /// The session has stopped.
    Stopped,
    /// The handshake is not done.
    NotPlaying,
    /// This session is not slot 0.
    NotLeader,
    /// An end is already pending.
    AlreadyEnding,
    /// A restart needs [`MIN_RESTART_TICK`] confirmed ticks.
    TooEarly,
    /// A peer has announced a leave.
    PeerLeft,
}

impl std::fmt::Display for EndRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EndRunError::Stopped => "the session has stopped",
            EndRunError::NotPlaying => "the handshake is not done",
            EndRunError::NotLeader => "only slot 0 may end the run",
            EndRunError::AlreadyEnding => "an end is already pending",
            EndRunError::TooEarly => "a restart needs tick 60",
            EndRunError::PeerLeft => "a peer has left",
        })
    }
}

impl std::error::Error for EndRunError {}

/// Why a [`NetSession`] call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    /// The session has stopped.
    Stopped,
    /// The inner session refused a local call.
    Session(SessionError),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Stopped => f.write_str("the session has stopped"),
            NetError::Session(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for NetError {}

/// Counters of what the layer did with packets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetStats {
    /// Valid packets from a peer that were acted on. Every received message is counted once, as
    /// accepted, bad, refused or dropped.
    pub accepted: u64,
    /// Messages that did not decode.
    pub bad_packets: u64,
    /// Packets the session refused (a [`SessionError`] or a bad ack).
    pub refused: u64,
    /// Packets dropped: from a slot that is not a remote slot, before the sender's hello, a
    /// mismatching hello, or anything received once stopped.
    pub dropped: u64,
    /// Sends the transport failed.
    pub send_errors: u64,
    /// Checkpoints compared and equal.
    pub checkpoints_compared: u64,
}

/// What the session knows of one remote peer.
struct Peer {
    slot: PlayerIdx,
    /// Its hello matched ours.
    hello_ok: bool,
    /// A packet other than a hello has been accepted from it: it has our hello.
    alive: bool,
    /// An ack has been accepted from it: its lead can be measured.
    heard_ack: bool,
    /// The highest `next_tick` of its acks.
    acked: u32,
    /// The tick of its announced leave.
    leave: Option<u32>,
    /// Its checkpoints not yet compared, by tick.
    checkpoints: BTreeMap<u32, u64>,
    /// The tick of the last checkpoint compared with it.
    compared: u32,
    /// A packet of the next run was seen from it while a Restart was pending: it has restarted.
    ahead: bool,
    /// A packet of this run has been accepted from it since the last restart.
    caught_up: bool,
}

/// The run a packet belongs to; the hellos have none.
fn run_of(packet: &Packet) -> Option<u8> {
    match packet {
        Packet::Hello(_) | Packet::HelloV1(_) => None,
        Packet::Input(f) => Some(f.run),
        Packet::Ack(a) => Some(a.run),
        Packet::Checkpoint(c) => Some(c.run),
        Packet::Bye(b) => Some(b.run),
        Packet::Command(c) => Some(c.run),
    }
}

/// A [`Session`] behind a [`Transport`].
pub struct NetSession<T: Transport, S: Save = CloneRing> {
    session: Session<S>,
    transport: T,
    local: PlayerIdx,
    hello: Hello,
    max_skew: u32,
    party: u8,
    peers: Vec<Peer>,
    phase: Phase,
    stop: Option<StopReason>,
    report: Option<DesyncReport>,
    local_leave: Option<u32>,
    /// The run this session plays: 0 until a restart changes it (`wrapping_add(1)` each time). Every packet but the hello
    /// carries it and a packet of another run is dropped (ADR-NET-6).
    run: u8,
    round: u32,
    own: BTreeMap<u32, u64>,
    next_checkpoint: u32,
    stats: NetStats,
    /// The leader's end of this run, once stamped or accepted.
    ending: Option<LeaderCommand>,
    /// The latest run that ended, until the caller takes it.
    ended: Option<EndedRun>,
    /// Leader only: the Restart of the run just left, still sent to a peer not yet heard from
    /// in the new run (it may have lost the command).
    resend: Option<LeaderCommand>,
}

impl<T: Transport> NetSession<T, CloneRing> {
    /// A net session over `engine` at tick 0.
    pub fn new(
        engine: Engine,
        config: NetConfig,
        transport: T,
    ) -> Result<NetSession<T, CloneRing>, NetConfigError> {
        check_hello(&config)?;
        let session_config = session_config(&config);
        let session = Session::new(engine, session_config).map_err(NetConfigError::Session)?;
        Ok(NetSession::from_parts(session, config, transport))
    }
}

fn check_hello(config: &NetConfig) -> Result<(), NetConfigError> {
    if config.hello.protocol == 0 || config.hello.sim_version == 0 {
        return Err(NetConfigError::BadHello);
    }
    Ok(())
}

fn session_config(config: &NetConfig) -> super::Config {
    super::Config {
        local: config.local,
        delay: config.settings.delay,
        window: config.settings.window,
    }
}

impl<T: Transport, S: Save> NetSession<T, S> {
    /// A net session that keeps its states in `save`.
    pub fn with_save(
        engine: Engine,
        config: NetConfig,
        transport: T,
        save: S,
    ) -> Result<NetSession<T, S>, NetConfigError> {
        check_hello(&config)?;
        let session = Session::with_save(engine, session_config(&config), save)
            .map_err(NetConfigError::Session)?;
        Ok(NetSession::from_parts(session, config, transport))
    }

    fn from_parts(mut session: Session<S>, config: NetConfig, transport: T) -> NetSession<T, S> {
        // The log of confirmed ticks is what a desync report is built from.
        session.keep_confirmed();
        let present = session.engine().present();
        let peers = (0..3u8)
            .filter(|i| usize::from(*i) != config.local.get() && present >> i & 1 == 1)
            .filter_map(PlayerIdx::new)
            .map(|slot| Peer {
                slot,
                hello_ok: false,
                alive: false,
                heard_ack: false,
                acked: 0,
                leave: None,
                checkpoints: BTreeMap::new(),
                compared: 0,
                ahead: false,
                caught_up: false,
            })
            .collect();
        NetSession {
            session,
            transport,
            local: config.local,
            hello: Hello {
                delay: config.settings.delay,
                window: config.settings.window,
                ..config.hello
            },
            max_skew: config.max_skew,
            party: u8::try_from(present.count_ones()).unwrap_or(3),
            peers,
            phase: Phase::Handshake,
            stop: None,
            report: None,
            local_leave: None,
            run: 0,
            round: 0,
            own: BTreeMap::new(),
            next_checkpoint: CHECKPOINT_TICKS,
            stats: NetStats::default(),
            ending: None,
            ended: None,
            resend: None,
        }
    }

    /// Receives, steps one tick if the handshake, the frame advantage and the window allow, and
    /// sends a round. A call that waits (see [`NetWait`]) does not queue `local`: the caller
    /// gives the input again on its next call.
    pub fn advance(&mut self, local: &Input) -> Result<NetAdvance, NetError> {
        if self.phase == Phase::Stopped {
            // A lost stop announcement is repeated for as long as the caller calls.
            self.receive();
            self.send_round();
            return Err(NetError::Stopped);
        }
        self.receive();
        if self.phase == Phase::Stopped {
            self.send_round();
            return Err(NetError::Stopped);
        }
        let out = if self.phase == Phase::Handshake {
            NetAdvance {
                rolled_back: 0,
                waiting: Some(NetWait::Handshake),
            }
        } else if self.ending.is_some_and(|c| self.session.tick() >= c.tick) {
            // Stepped to the end tick: confirm what can be and wait for the end.
            let rolled_back = self.session.confirm().map_err(NetError::Session)?;
            NetAdvance {
                rolled_back,
                waiting: Some(NetWait::Ending),
            }
        } else if self.too_far_ahead() {
            let rolled_back = self.session.confirm().map_err(NetError::Session)?;
            NetAdvance {
                rolled_back,
                waiting: Some(NetWait::Ahead),
            }
        } else {
            let out = self.session.advance(local).map_err(NetError::Session)?;
            NetAdvance {
                rolled_back: out.rolled_back,
                waiting: out.waiting.map(NetWait::from),
            }
        };
        if self.phase == Phase::Playing {
            self.checkpoints();
            self.finish();
        }
        self.send_round();
        Ok(out)
    }

    /// Receives, confirms what can be confirmed and sends a round; steps nothing.
    pub fn poll(&mut self) -> Result<(), NetError> {
        self.receive();
        if self.phase == Phase::Playing {
            self.session.confirm().map_err(NetError::Session)?;
            self.checkpoints();
            self.finish();
        }
        self.send_round();
        Ok(())
    }

    /// Announces the local leave at the first tick without a local input and returns that tick.
    pub fn leave(&mut self) -> Result<u32, NetError> {
        if self.phase == Phase::Stopped {
            return Err(NetError::Stopped);
        }
        let tick = self.session.next_tick(self.local);
        self.session
            .leave(self.local, tick)
            .map_err(NetError::Session)?;
        self.local_leave = Some(tick);
        Ok(tick)
    }

    /// The phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Why the session stopped, once it has.
    pub fn stop_reason(&self) -> Option<StopReason> {
        self.stop
    }

    /// The inner session.
    pub fn session(&self) -> &Session<S> {
        &self.session
    }

    /// The counters.
    pub fn stats(&self) -> NetStats {
        self.stats
    }

    /// The run this session plays.
    pub fn run(&self) -> u8 {
        self.run
    }

    /// The tick and kind of the pending end, if there is one.
    pub fn ending(&self) -> Option<(u32, CommandKind)> {
        self.ending.map(|c| (c.tick, c.kind))
    }

    /// The latest run that ended (a restart or a cancel), once: a second call returns `None`
    /// until another run ends.
    pub fn take_ended_run(&mut self) -> Option<EndedRun> {
        self.ended.take()
    }

    /// Slot 0 ends the run at the tick it has confirmed and returns that tick (ADR-NET-3
    /// point 7). A Cancel finishes at once and the session stops; a Restart is sent to
    /// every peer and happens once the session has stepped to that tick.
    ///
    /// # Errors
    ///
    /// See [`EndRunError`]; a Cancel is never refused for the tick or for a peer that left.
    pub fn end_run(&mut self, kind: CommandKind) -> Result<u32, EndRunError> {
        match self.phase {
            Phase::Stopped => return Err(EndRunError::Stopped),
            Phase::Handshake => return Err(EndRunError::NotPlaying),
            Phase::Playing => {}
        }
        if self.local.get() != 0 {
            return Err(EndRunError::NotLeader);
        }
        if self.ending.is_some() {
            return Err(EndRunError::AlreadyEnding);
        }
        let tick = self.session.confirmed();
        if kind == CommandKind::Restart {
            if tick < MIN_RESTART_TICK {
                return Err(EndRunError::TooEarly);
            }
            if self.peers.iter().any(|p| p.leave.is_some()) {
                return Err(EndRunError::PeerLeft);
            }
        }
        self.ending = Some(LeaderCommand {
            run: self.run,
            slot: self.local,
            tick,
            kind,
        });
        self.finish();
        Ok(tick)
    }

    /// The desync report, once the session stopped on a desync.
    pub fn desync_report(&self) -> Option<&DesyncReport> {
        self.report.as_ref()
    }

    /// The transport, for a caller that owns its own pump.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn peer_index(&self, slot: PlayerIdx) -> Option<usize> {
        self.peers.iter().position(|p| p.slot == slot)
    }

    /// Takes every waiting message and maps it onto the session. Nothing a peer sends panics.
    fn receive(&mut self) {
        while let Some((from, bytes)) = self.transport.recv() {
            let Some(p) = self.peer_index(from) else {
                self.stats.dropped += 1;
                continue;
            };
            if self.phase == Phase::Stopped {
                self.receive_stopped(p, &bytes);
                continue;
            }
            match Packet::decode(&bytes) {
                Ok(packet) => self.map(p, packet),
                Err(_) => self.stats.bad_packets += 1,
            }
        }
    }

    /// A message that arrives once stopped: after the leader's cancel an ack of this run still
    /// counts (it moves the frames this session serves); everything else is dropped.
    fn receive_stopped(&mut self, p: usize, bytes: &[u8]) {
        let ended_by_leader = matches!(self.stop, Some(StopReason::EndedByLeader(_)));
        match Packet::decode(bytes) {
            Ok(Packet::Ack(ack)) if ended_by_leader && ack.run == self.run => {
                if self.take_ack(p, &ack) {
                    self.stats.accepted += 1;
                } else {
                    self.stats.refused += 1;
                }
            }
            _ => self.stats.dropped += 1,
        }
    }

    /// Takes an ack of peer `p`; whether it was good.
    fn take_ack(&mut self, p: usize, ack: &Ack) -> bool {
        let from = self.peers[p].slot;
        let ok = ack.slot == self.local && self.session.ack(from, ack.next_tick).is_ok();
        if ok {
            let peer = &mut self.peers[p];
            peer.acked = peer.acked.max(ack.next_tick);
            peer.heard_ack = true;
        }
        ok
    }

    /// Maps one packet of peer `p` (its slot is the path's, never the packet's).
    fn map(&mut self, p: usize, packet: Packet) {
        let from = self.peers[p].slot;
        if !self.peers[p].hello_ok
            && !matches!(
                packet,
                Packet::Hello(_) | Packet::HelloV1(_) | Packet::Bye(_)
            )
        {
            self.stats.dropped += 1;
            return;
        }
        // Every packet but a hello belongs to a run; another run's is valid but not for this
        // session. The hellos have no run, so a protocol 2 hello still reaches `refusal`.
        match run_of(&packet) {
            Some(run) if run != self.run => {
                // A peer that has already restarted: remembered for the barrier, then dropped.
                if run == self.run.wrapping_add(1)
                    && self.ending.is_some_and(|c| c.kind == CommandKind::Restart)
                {
                    self.peers[p].ahead = true;
                }
                self.stats.dropped += 1;
                return;
            }
            Some(_) => self.peers[p].caught_up = true,
            None => {}
        }
        let accepted = match packet {
            Packet::Hello(hello) => {
                if !self.peers[p].hello_ok {
                    if let Some(why) = refusal(&self.hello, &hello) {
                        self.stats.dropped += 1;
                        self.stop(StopReason::Refused { slot: from, why });
                        return;
                    }
                    self.peers[p].hello_ok = true;
                    if self.peers.iter().all(|q| q.hello_ok) {
                        self.phase = Phase::Playing;
                    }
                }
                self.stats.accepted += 1;
                return;
            }
            Packet::HelloV1(_) => {
                // A protocol 1 peer: refused if it is the first hello, ignored after a good one.
                self.stats.dropped += 1;
                if !self.peers[p].hello_ok {
                    self.stop(StopReason::Refused {
                        slot: from,
                        why: Refusal::Protocol,
                    });
                }
                return;
            }
            Packet::Input(frame) => self.session.receive_inputs(from, &frame).is_ok(),
            Packet::Ack(ack) => self.take_ack(p, &ack),
            Packet::Checkpoint(cp) => return self.take_checkpoint(p, cp),
            Packet::Bye(bye) => return self.take_bye(p, bye),
            // The run gate above has dropped a command of another run. Only slot 0 commands, in
            // its own name; a Restart needs `MIN_RESTART_TICK`; the tick must be one this session
            // can reach (an honest tick is never past its `next_tick`); a second command must be
            // the first one again.
            Packet::Command(cmd) => {
                let ok = from.get() == 0
                    && cmd.slot == from
                    && (cmd.kind != CommandKind::Restart || cmd.tick >= MIN_RESTART_TICK)
                    && cmd.tick <= self.session.next_tick(self.local)
                    && self.ending.is_none_or(|held| held == cmd);
                if ok {
                    self.ending = Some(cmd);
                }
                ok
            }
        };
        if accepted {
            self.peers[p].alive = true;
            self.stats.accepted += 1;
        } else {
            self.stats.refused += 1;
        }
    }

    fn take_bye(&mut self, p: usize, bye: Bye) {
        let from = self.peers[p].slot;
        match bye.reason {
            ByeReason::Left => {
                if bye.slot == from && self.session.leave(from, bye.tick).is_ok() {
                    self.peers[p].leave = Some(bye.tick);
                    self.stats.accepted += 1;
                } else {
                    self.stats.refused += 1;
                }
            }
            ByeReason::Refused => {
                self.stats.accepted += 1;
                self.stop(StopReason::RefusedBy(from));
            }
            ByeReason::Desync if !self.peers[p].hello_ok => self.stats.dropped += 1,
            ByeReason::Desync => {
                self.stats.accepted += 1;
                self.stop_desync(
                    StopReason::PeerDesync {
                        slot: from,
                        tick: bye.tick,
                    },
                    None,
                );
            }
        }
    }

    fn take_checkpoint(&mut self, p: usize, cp: Checkpoint) {
        if cp.tick > self.session.tick().saturating_add(CHECKPOINT_TICKS) {
            // No peer within the window is that far ahead: junk must not evict real entries.
            self.stats.refused += 1;
            return;
        }
        let peer = &mut self.peers[p];
        if cp.tick <= peer.compared || peer.leave.is_some_and(|l| l <= cp.tick) {
            self.stats.dropped += 1;
            return;
        }
        peer.alive = true;
        peer.checkpoints.insert(cp.tick, cp.hash);
        while peer.checkpoints.len() > CHECKPOINTS_KEPT {
            peer.checkpoints.pop_first();
        }
        self.stats.accepted += 1;
    }

    /// Whether some peer's skew has reached `max_skew`: this session runs ahead of it.
    fn too_far_ahead(&self) -> bool {
        let mine = i64::from(self.session.next_tick(self.local));
        self.peers.iter().any(|peer| {
            if !peer.heard_ack || peer.leave.is_some() {
                return false;
            }
            let theirs = i64::from(self.session.next_tick(peer.slot));
            let local_lead = mine - theirs;
            let remote_lead = theirs - i64::from(peer.acked);
            local_lead - remote_lead >= i64::from(self.max_skew)
        })
    }

    /// One send round to every peer still present, or the stop announcement once stopped.
    fn send_round(&mut self) {
        self.round += 1;
        if self.resend.is_some() && self.peers.iter().all(|p| p.caught_up) {
            self.resend = None;
        }
        for p in 0..self.peers.len() {
            let to = self.peers[p].slot;
            let mut packets = Vec::new();
            let ended_by_leader = matches!(self.stop, Some(StopReason::EndedByLeader(_)));
            if let Some(bye) = self.stop.and_then(|stop| self.stop_bye(stop)) {
                packets.push(Packet::Bye(bye));
            } else {
                let peer = &self.peers[p];
                if peer.leave.is_some_and(|l| self.session.tick() >= l) {
                    continue;
                }
                if !peer.alive && !ended_by_leader {
                    packets.push(Packet::Hello(self.hello));
                }
                if ended_by_leader {
                    // The run is over for this session, but a peer may still need what it holds:
                    // acks, the inputs it has not acked, and the leader's command.
                    packets.push(Packet::Ack(Ack {
                        run: self.run,
                        slot: to,
                        next_tick: self.session.next_tick(to),
                    }));
                    if let Some(frame) = self.session.local_frame(peer.acked) {
                        packets.push(Packet::Input(InputFrame {
                            run: self.run,
                            ..frame
                        }));
                    }
                    if let Some(cmd) = self.ending.filter(|_| self.local.get() == 0) {
                        packets.push(Packet::Command(cmd));
                    }
                } else if self.phase == Phase::Playing {
                    packets.push(Packet::Ack(Ack {
                        run: self.run,
                        slot: to,
                        next_tick: self.session.next_tick(to),
                    }));
                    if let Some(frame) = self.session.local_frame(peer.acked) {
                        packets.push(Packet::Input(InputFrame {
                            run: self.run,
                            ..frame
                        }));
                    }
                    if let Some(tick) = self.local_leave {
                        packets.push(Packet::Bye(Bye {
                            run: self.run,
                            slot: self.local,
                            tick,
                            reason: ByeReason::Left,
                        }));
                    }
                    self.checkpoint_packets(&mut packets);
                    // The leader repeats a pending Restart every round, and the one it has
                    // already made for a peer that has not answered in the new run.
                    if self.local.get() == 0 {
                        if let Some(cmd) = self.ending {
                            packets.push(Packet::Command(cmd));
                        } else if let Some(cmd) = self.resend.filter(|_| !peer.caught_up) {
                            packets.push(Packet::Command(cmd));
                        }
                    }
                }
            }
            for packet in packets {
                match packet.encode() {
                    Ok(bytes) => {
                        if self.transport.send(to, &bytes).is_err() {
                            self.stats.send_errors += 1;
                        }
                    }
                    Err(_) => self.stats.send_errors += 1,
                }
            }
        }
    }

    /// The bye that announces a stop; the leader's end is announced by its command, not a bye.
    fn stop_bye(&self, stop: StopReason) -> Option<Bye> {
        let (tick, reason) = match stop {
            StopReason::Refused { .. } | StopReason::RefusedBy(_) => (0, ByeReason::Refused),
            StopReason::Desync { tick, .. } | StopReason::PeerDesync { tick, .. } => {
                (tick, ByeReason::Desync)
            }
            StopReason::EndedByLeader(_) => return None,
        };
        Some(Bye {
            run: self.run,
            slot: self.local,
            tick,
            reason,
        })
    }

    /// Ends the run once this session has confirmed the end tick: a Cancel stops the session
    /// with the run it played.
    fn finish(&mut self) {
        let Some(cmd) = self.ending else {
            return;
        };
        if self.phase != Phase::Playing || self.session.confirmed() < cmd.tick {
            return;
        }
        match cmd.kind {
            CommandKind::Cancel => {
                let ticks = self.session.take_confirmed();
                self.ended = Some(EndedRun::new(self.run, cmd.tick, self.party, ticks));
                self.stop(StopReason::EndedByLeader(CommandKind::Cancel));
            }
            CommandKind::Restart => {
                // Every peer must hold what this run still needs from this session: it has left,
                // or has acked every input below T, or has restarted (a packet of the next run).
                let barrier = self
                    .peers
                    .iter()
                    .all(|p| p.leave.is_some() || p.acked >= cmd.tick || p.ahead);
                if barrier {
                    self.restart_in_place(cmd);
                }
            }
        }
    }

    /// Hands out the run that ended and starts the next one at tick 0, in place.
    fn restart_in_place(&mut self, cmd: LeaderCommand) {
        let ticks = self.session.restart();
        self.ended = Some(EndedRun::new(self.run, cmd.tick, self.party, ticks));
        self.resend = (self.local.get() == 0).then_some(cmd);
        self.run = self.run.wrapping_add(1);
        self.ending = None;
        self.own.clear();
        self.next_checkpoint = CHECKPOINT_TICKS;
        self.local_leave = None;
        for peer in &mut self.peers {
            peer.acked = 0;
            peer.heard_ack = false;
            peer.leave = None;
            peer.checkpoints.clear();
            peer.compared = 0;
            peer.ahead = false;
            peer.caught_up = false;
        }
    }

    /// Stops for good; the next send round announces it.
    fn stop(&mut self, reason: StopReason) {
        if self.phase != Phase::Stopped {
            self.phase = Phase::Stopped;
            self.stop = Some(reason);
        }
    }

    /// Records the checkpoints newly confirmed and compares them with the peers'.
    fn checkpoints(&mut self) {
        let confirmed = self.session.confirmed();
        while self.next_checkpoint <= confirmed {
            if let Some(hash) = self.session.hash_after(self.next_checkpoint) {
                self.own.insert(self.next_checkpoint, hash);
                while self.own.len() > CHECKPOINTS_KEPT {
                    self.own.pop_first();
                }
            }
            self.next_checkpoint += CHECKPOINT_TICKS;
        }
        let mut differ = None;
        'peers: for peer in &mut self.peers {
            let ready: Vec<u32> = peer
                .checkpoints
                .keys()
                .copied()
                .filter(|t| self.own.contains_key(t))
                .collect();
            for tick in ready {
                let (Some(theirs), Some(mine)) =
                    (peer.checkpoints.remove(&tick), self.own.get(&tick))
                else {
                    continue;
                };
                if theirs == *mine {
                    peer.compared = peer.compared.max(tick);
                    self.stats.checkpoints_compared += 1;
                } else {
                    differ = Some((tick, peer.slot, theirs));
                    break 'peers;
                }
            }
        }
        if let Some((tick, slot, theirs)) = differ {
            self.stop_desync(StopReason::Desync { tick, slot }, Some(theirs));
        }
    }

    /// The checkpoints a send round carries (every `CHECKPOINT_SEND_EVERY`-th round): the newest
    /// ones that are at most one interval behind the confirmed tick.
    fn checkpoint_packets(&self, packets: &mut Vec<Packet>) {
        if self.round % CHECKPOINT_SEND_EVERY != 0 {
            return;
        }
        let confirmed = self.session.confirmed();
        for (tick, hash) in self
            .own
            .iter()
            .rev()
            .filter(|(t, _)| **t + CHECKPOINT_TICKS >= confirmed)
            .take(CHECKPOINTS_IN_FLIGHT)
        {
            packets.push(Packet::Checkpoint(Checkpoint {
                run: self.run,
                slot: self.local,
                tick: *tick,
                hash: *hash,
            }));
        }
    }

    /// Stops on a desync and keeps the report: the confirmed log below the tick and both hashes.
    fn stop_desync(&mut self, reason: StopReason, remote_hash: Option<u64>) {
        if self.phase == Phase::Stopped {
            return;
        }
        let (StopReason::Desync { tick, slot } | StopReason::PeerDesync { slot, tick }) = reason
        else {
            return;
        };
        self.stop(reason);
        let mut ticks = self.session.take_confirmed();
        ticks.truncate(usize::try_from(tick).unwrap_or(usize::MAX));
        self.report = Some(DesyncReport {
            tick,
            slot,
            local_hash: self
                .own
                .get(&tick)
                .copied()
                .or_else(|| self.session.hash_after(tick)),
            remote_hash,
            party: self.party,
            ticks,
        });
    }
}

/// What a peer's hello differs in from ours, if anything.
fn refusal(ours: &Hello, theirs: &Hello) -> Option<Refusal> {
    if ours.protocol != theirs.protocol {
        Some(Refusal::Protocol)
    } else if ours.sim_version != theirs.sim_version {
        Some(Refusal::SimVersion)
    } else if ours.pack_hash != theirs.pack_hash {
        Some(Refusal::Pack)
    } else if ours.delay != theirs.delay {
        Some(Refusal::Delay)
    } else if ours.window != theirs.window {
        Some(Refusal::Window)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use stella_rain_core::engine::Engine;
    use stella_rain_core::id::PlayerIdx;
    use stella_rain_core::run_setup::RunSetup;
    use stella_rain_core::version::SIM_VERSION;

    use super::*;
    use crate::transport::TransportError;
    use crate::wire::PROTOCOL_VERSION;

    /// A transport that records every packet sent and receives nothing.
    struct Record(Rc<RefCell<Vec<(PlayerIdx, Packet)>>>);

    impl Transport for Record {
        fn send(&mut self, to: PlayerIdx, packet: &[u8]) -> Result<(), TransportError> {
            let decoded = Packet::decode(packet).expect("a valid packet");
            self.0.borrow_mut().push((to, decoded));
            Ok(())
        }

        fn recv(&mut self) -> Option<(PlayerIdx, Vec<u8>)> {
            None
        }
    }

    #[test]
    fn restart_wraps_the_run_number_at_255() {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let hello = Hello {
            protocol: PROTOCOL_VERSION,
            sim_version: SIM_VERSION,
            pack_hash: [0; 32],
            delay: DEFAULT_DELAY,
            window: DEFAULT_WINDOW,
        };
        let local = PlayerIdx::new(0).expect("slot 0");
        let engine = Engine::start_party(&RunSetup::default(), 2).expect("an engine");
        let mut s = NetSession::new(
            engine,
            NetConfig::new(local, hello),
            Record(Rc::clone(&sent)),
        )
        .expect("a session");
        s.phase = Phase::Playing;
        s.run = 255;
        s.ending = Some(LeaderCommand {
            run: 255,
            slot: local,
            tick: 0,
            kind: CommandKind::Restart,
        });
        s.finish();
        assert_eq!(s.run(), 0, "255 restarts to 0");
        assert_eq!(s.phase(), Phase::Playing);
        assert_eq!(s.ending(), None);
        let ended = s.take_ended_run().expect("the ended run");
        assert_eq!((ended.run, ended.tick, ended.party), (255, 0, 2));
        s.send_round();
        let sent = sent.borrow();
        assert!(
            sent.iter()
                .any(|(_, p)| matches!(p, Packet::Ack(a) if a.run == 0)),
            "the next packet carries run 0"
        );
        assert!(
            sent.iter()
                .any(|(_, p)| matches!(p, Packet::Command(c) if c.run == 255 && c.kind == CommandKind::Restart)),
            "the old command still carries run 255"
        );
    }
}
