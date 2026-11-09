//! Rollback netcode for Stella Rain.
//!
//! A sans-IO session over the public API of `stella-rain-core`: it takes inputs and ticks in,
//! hands messages and re-simulation requests out, and owns no sockets, clocks, threads or
//! floats. Public items arrive one task at a time, each with a doc comment and a test: the wire
//! packets and their strict codec, the `Transport` interface, and the rollback `Session` with its
//! `Save` seam. The session is driven by local calls only; it has no transport, handshake, frame
//! advantage, checkpoint exchange or desync stop yet.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod predict;
pub mod save;
pub mod session;
pub mod transport;
pub mod wire;
