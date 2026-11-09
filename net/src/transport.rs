//! The `Transport` interface: how a session sends and receives one message at a time, whatever
//! the path underneath (direct, relayed, simulated). The engine never knows the path.

use std::fmt;

use stella_rain_core::id::PlayerIdx;

/// Why a [`Transport::send`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// The path to the peer is closed.
    Closed,
    /// The message is longer than [`crate::wire::MAX_PACKET_LEN`].
    TooLarge,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Closed => f.write_str("the transport is closed"),
            TransportError::TooLarge => {
                f.write_str("the message is over the maximum packet length")
            }
        }
    }
}

impl std::error::Error for TransportError {}

/// A message path to the other peers of a party, addressed by slot.
///
/// One call is one message. Delivery is best effort: a message may be lost, duplicated or
/// reordered, and the transport never retries one. `send` and `recv` never block; an
/// implementation owns no clock and no thread. A message over
/// [`crate::wire::MAX_PACKET_LEN`] is refused with [`TransportError::TooLarge`].
///
/// The slot `recv` returns is the one the path vouches for; it is not read from the packet, whose
/// own slot field a peer can set to anything. A peer is never the sender's own slot.
pub trait Transport {
    /// Sends one message to the peer in slot `to`.
    fn send(&mut self, to: PlayerIdx, packet: &[u8]) -> Result<(), TransportError>;

    /// The next message that arrived and the slot it came from, or `None` when nothing waits.
    fn recv(&mut self) -> Option<(PlayerIdx, Vec<u8>)>;
}
