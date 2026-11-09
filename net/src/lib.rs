//! Rollback netcode for Stella Rain (ADR-NET-1, ADR-NET-2).
//!
//! A sans-IO session over the public API of `stella-rain-core`: it takes inputs and ticks in,
//! hands messages and re-simulation requests out, and owns no sockets, clocks, threads or
//! floats. The crate has no public items yet; they arrive one task at a time, each with a doc
//! comment and a test.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
