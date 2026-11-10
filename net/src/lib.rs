//! Rollback netcode for Stella Rain.
//!
//! A sans-IO session over the public API of `stella-rain-core`: it takes inputs and ticks in,
//! hands messages and re-simulation requests out, and owns no sockets, clocks, threads or
//! floats. The wire packets and their strict codec, the `Transport` interface, the rollback
//! `Session` with its `Save` seam, and over them `session::party::NetSession`: the handshake,
//! one delay and window per party, the redundant send and ack of inputs, frame advantage, the
//! checkpoint exchange, the stop on desync and the desync report. The handshake refuses a hello
//! whose protocol, `sim_version` or pack hash differs, and one whose delay or window differs from
//! the party's; a session is not built with a delay or window over 16 ticks. Not built yet: the
//! final hash is not exchanged, and the late join is left out.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod predict;
pub mod save;
pub mod session;
pub mod transport;
pub mod wire;
