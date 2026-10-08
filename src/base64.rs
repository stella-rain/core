//! Standard base64 (RFC 4648, `+/` alphabet, `=` padding) for replays and share codes.
//!
//! Decoding is strict, because the text is untrusted and one meaning should have one spelling:
//! the length must be a multiple of four, only alphabet characters are allowed (no whitespace),
//! padding only at the end, and the unused bits must be zero.

use std::fmt;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a multiple of four characters.
    Length,
    /// A character outside the alphabet, or padding in the wrong place.
    Character,
    /// The bits after the last full byte are not zero.
    TrailingBits,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Length => "base64 text must be a multiple of four characters",
            Error::Character => "base64 text has a character that does not belong",
            Error::TrailingBits => "base64 text is not in its canonical form",
        })
    }
}

impl std::error::Error for Error {}

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sextet = |shift: u32| char::from(ALPHABET[((n >> shift) & 0x3f) as usize]);
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

fn sextet(b: u8) -> Option<u32> {
    match b {
        b'A'..=b'Z' => Some(u32::from(b - b'A')),
        b'a'..=b'z' => Some(u32::from(b - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(b - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

pub fn decode(text: &str) -> Result<Vec<u8>, Error> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(Error::Length);
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let chunks = bytes.len() / 4;
    for (i, chunk) in bytes.chunks_exact(4).enumerate() {
        let last = i + 1 == chunks;
        // Padding is only allowed in the last chunk, as `xx==` or `xxx=`.
        let padding = if last {
            chunk.iter().rev().take_while(|b| **b == b'=').count()
        } else {
            0
        };
        if padding > 2 {
            return Err(Error::Character);
        }
        let mut n = 0u32;
        for b in &chunk[..4 - padding] {
            n = (n << 6) | sextet(*b).ok_or(Error::Character)?;
        }
        n <<= 6 * padding as u32;
        let (b0, b1, b2) = ((n >> 16) as u8, (n >> 8) as u8, n as u8);
        match padding {
            0 => out.extend_from_slice(&[b0, b1, b2]),
            1 => {
                if b2 != 0 {
                    return Err(Error::TrailingBits);
                }
                out.extend_from_slice(&[b0, b1]);
            }
            _ => {
                if b1 != 0 || b2 != 0 {
                    return Err(Error::TrailingBits);
                }
                out.push(b0);
            }
        }
    }
    Ok(out)
}
