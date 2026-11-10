//! Rollback netcode for Stella Rain.
//!
//! A sans-IO session over the public API of `stella-rain-core`: it takes inputs and ticks in,
//! hands messages and re-simulation requests out, and owns no sockets, clocks, threads or
//! floats. The wire packets and their strict codec, the `Transport` interface, the rollback
//! `Session` with its `Save` seam, and over them `session::party::NetSession`: the handshake,
//! one delay and window per party, the redundant send and ack of inputs, frame advantage, the
//! checkpoint exchange, the stop on desync and the desync report. Not built yet: a delay or
//! window mismatch is not refused at the handshake (the hello does not carry them), the final
//! hash is not exchanged, and the late join is left out.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod predict;
pub mod save;
pub mod session;
pub mod transport;
pub mod wire;
