//! The wire codec through the public API only.

use stella_rain_net::wire::{
    Hello, MAX_FRAME_INPUTS, MAX_PACKET_LEN, PROTOCOL_VERSION, Packet, WireError,
};

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

const HELLO: &str = "01010000000200000000010203040506070809\
                     0a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn hello_packet() -> Packet {
    let mut pack_hash = [0u8; 32];
    for (i, b) in pack_hash.iter_mut().enumerate() {
        *b = u8::try_from(i).unwrap();
    }
    Packet::Hello(Hello {
        protocol: 1,
        sim_version: 2,
        pack_hash,
    })
}

/// Every vector the file knows; later steps add theirs.
fn vectors() -> Vec<Vec<u8>> {
    vec![hex(HELLO)]
}

#[test]
fn the_constants_are_what_the_protocol_says() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(MAX_PACKET_LEN, 1200);
    assert_eq!(MAX_FRAME_INPUTS, 32);
}

#[test]
fn framing_refuses_the_empty_the_unknown_and_the_too_long() {
    assert_eq!(Packet::decode(&[]), Err(WireError::Empty));
    for tag in [0x00u8, 0x06, 0xff] {
        assert_eq!(Packet::decode(&[tag]), Err(WireError::UnknownTag(tag)));
    }
    let mut long = vec![0u8; MAX_PACKET_LEN + 1];
    long[0] = 0x03;
    assert_eq!(Packet::decode(&long), Err(WireError::TooLong));
}

#[test]
fn trailing_and_missing_bytes_are_refused() {
    let mut more = hex(HELLO);
    more.push(0);
    assert_eq!(Packet::decode(&more), Err(WireError::TrailingBytes));
    let mut less = hex(HELLO);
    less.pop();
    assert_eq!(Packet::decode(&less), Err(WireError::Truncated));
}

#[test]
fn every_strict_prefix_of_every_vector_is_an_error() {
    for v in vectors() {
        for n in 0..v.len() {
            assert!(Packet::decode(&v[..n]).is_err(), "prefix {n} of {v:02x?}");
        }
    }
}

#[test]
fn a_hello_is_41_bytes_and_round_trips() {
    let p = hello_packet();
    let bytes = p.encode().unwrap();
    assert_eq!(bytes.len(), 41);
    assert_eq!(bytes, hex(HELLO));
    assert_eq!(Packet::decode(&bytes), Ok(p));
}

#[test]
fn a_zero_version_is_refused_both_ways() {
    for (protocol, sim_version) in [(0, 2), (1, 0)] {
        let p = Packet::Hello(Hello {
            protocol,
            sim_version,
            pack_hash: [0; 32],
        });
        assert_eq!(p.encode(), Err(WireError::ZeroVersion));
        let mut bytes = hex(HELLO);
        bytes[1..5].copy_from_slice(&protocol.to_le_bytes());
        bytes[5..9].copy_from_slice(&sim_version.to_le_bytes());
        assert_eq!(Packet::decode(&bytes), Err(WireError::ZeroVersion));
    }
}
