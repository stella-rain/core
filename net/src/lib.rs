//! Rollback netcode for Stella Rain.
//!
//! A sans-IO session over the public API of `stella-rain-core`: it takes inputs and ticks in,
//! hands messages and re-simulation requests out, and owns no sockets, clocks, threads or
//! floats. Public items arrive one task at a time, each with a doc comment and a test: the wire
//! packets and their strict codec, and the `Transport` interface.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod transport;
pub mod wire;
