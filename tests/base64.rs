//! The strict base64 codec (RFC 4648) behind replays and share codes.

use stella_rain_core::base64::{Error, decode, encode};
use stella_rain_core::rng::SplitMix64;

#[test]
fn rfc_4648_vectors() {
    for (plain, coded) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(encode(plain.as_bytes()), coded);
        assert_eq!(decode(coded).unwrap(), plain.as_bytes());
    }
}

#[test]
fn every_byte_value_and_length_round_trips() {
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(decode(&encode(&all)).unwrap(), all);
    let mut rng = SplitMix64::new(64);
    for len in 0..300 {
        let bytes: Vec<u8> = (0..len).map(|_| rng.below(256) as u8).collect();
        assert_eq!(decode(&encode(&bytes)).unwrap(), bytes, "length {len}");
    }
    // The characters outside the letters and digits.
    assert_eq!(encode(&[0xfb, 0xff, 0xbf]), "+/+/");
}

#[test]
fn only_the_canonical_spelling_is_accepted() {
    // Length.
    for bad in ["Z", "Zg", "Zg=", "Zm9vY"] {
        assert_eq!(decode(bad), Err(Error::Length), "{bad}");
    }
    // Characters, padding in the wrong place, whitespace, other alphabets.
    for bad in [
        "Zm9v\n",
        "Zm 9",
        "Zm9-",
        "Zm9_",
        "=m9v",
        "Z=9v",
        "Zm=v",
        "Zg=A",
        "====",
        "Zm9v====",
        "Zm9vZg==Zm9v",
        "Zm9é",
        " Zg==",
    ] {
        assert!(decode(bad).is_err(), "{bad:?}");
    }
    // Padding in a chunk that is not the last.
    assert_eq!(decode("Zg==Zm9v"), Err(Error::Character));
    // Bits after the last byte must be zero: "Zh==" and "Zm9=" would decode like their
    // canonical forms "Zg==" and "Zm8=".
    assert_eq!(decode("Zh=="), Err(Error::TrailingBits));
    assert_eq!(decode("Zm9="), Err(Error::TrailingBits));
    assert!(decode("Zg==").is_ok() && decode("Zm8=").is_ok());
}

#[test]
fn random_text_never_panics() {
    let mut rng = SplitMix64::new(65);
    let alphabet = b"ABZaz09+/=-_ \n\xc3\xa9";
    for _ in 0..3000 {
        let len = rng.below(40) as usize;
        let text: String = (0..len)
            .map(|_| char::from(alphabet[rng.below(alphabet.len() as u32) as usize] & 0x7f))
            .collect();
        let _ = decode(&text);
    }
}
