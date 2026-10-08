//! Share codes (ADR-010, ADR-018): exact bytes in, exact bytes out, and nothing a hostile code
//! can do to blow up memory (ADR-020).

mod common;

use common::{clearing_inputs, stage_bytes};
use miniz_oxide::deflate::compress_to_vec_zlib;
use stella_rain_core::base64;
use stella_rain_core::replay::{MAX_REPLAY_BYTES, record, verify};
use stella_rain_core::rng::SplitMix64;
use stella_rain_core::share::{
    MAX_SHARE_CODE_CHARS, MAX_SHARE_DECOMPRESSED_BYTES, ShareError, decode, encode,
};
use stella_rain_core::validate::limits::MAX_STAGE_BYTES;

fn code_of(container: &str) -> String {
    base64::encode(&compress_to_vec_zlib(container.as_bytes(), 6))
}

#[test]
fn a_stage_and_its_replay_survive_a_share_code_byte_for_byte() {
    let stage = stage_bytes();
    let replay = record(&stage, &clearing_inputs()).unwrap().to_bytes();
    let code = encode(&stage, &replay).unwrap();
    assert!(code.is_ascii() && !code.contains(char::is_whitespace));
    let (stage_back, replay_back) = decode(&code).unwrap();
    assert_eq!(stage_back, stage);
    assert_eq!(replay_back, replay);
    // And the pair still verifies, which needs the exact bytes.
    assert!(verify(&stage_back, &replay_back).is_ok());
}

#[test]
fn awkward_text_survives() {
    let stage = "{\"title\": \"quotes \\\" backslash \\\\ tab\\t é 日本 😀\u{0}\"}\r\n ".as_bytes();
    let replay = b"  line one\nline two\r\n";
    let (s, r) = decode(&encode(stage, replay).unwrap()).unwrap();
    assert_eq!((s, r), (stage.to_vec(), replay.to_vec()));
    assert_eq!(
        decode(&encode(b"", b"").unwrap()).unwrap(),
        (vec![], vec![])
    );
}

#[test]
fn files_that_are_not_text_or_too_big_are_not_encoded() {
    assert_eq!(encode(&[0xff, 0xfe], b"{}"), Err(ShareError::NotText));
    assert_eq!(encode(b"{}", &[0x80]), Err(ShareError::NotText));
    assert_eq!(
        encode(&vec![b' '; MAX_STAGE_BYTES + 1], b"{}"),
        Err(ShareError::FileTooLarge)
    );
    assert_eq!(
        encode(b"{}", &vec![b' '; MAX_REPLAY_BYTES + 1]),
        Err(ShareError::FileTooLarge)
    );
}

#[test]
fn a_small_code_cannot_expand_into_a_large_file() {
    // 40 MB of one byte compresses to a few tens of kilobytes; decoding stops at the limit.
    let bomb = format!(
        "{{\"stage\":\"{}\",\"replay\":\"\"}}",
        "a".repeat(40_000_000)
    );
    let code = code_of(&bomb);
    assert!(
        code.len() < MAX_SHARE_CODE_CHARS,
        "{} characters",
        code.len()
    );
    assert_eq!(decode(&code), Err(ShareError::TooLarge));
}

#[test]
fn the_decompressed_limit_is_exact() {
    let around = |total: usize| {
        let overhead = "{\"stage\":\"\",\"replay\":\"\"}".len();
        code_of(&format!(
            "{{\"stage\":\"{}\",\"replay\":\"\"}}",
            "a".repeat(total - overhead)
        ))
    };
    // At the limit the data is let through (and then refused as a stage that is too big);
    // one byte more is stopped while decompressing.
    assert_eq!(
        decode(&around(MAX_SHARE_DECOMPRESSED_BYTES)),
        Err(ShareError::FileTooLarge)
    );
    assert_eq!(
        decode(&around(MAX_SHARE_DECOMPRESSED_BYTES + 1)),
        Err(ShareError::TooLarge)
    );
}

#[test]
fn each_file_inside_has_its_own_limit() {
    let json = |stage: usize, replay: usize| {
        code_of(&format!(
            "{{\"stage\":\"{}\",\"replay\":\"{}\"}}",
            "a".repeat(stage),
            "a".repeat(replay)
        ))
    };
    assert!(decode(&json(MAX_STAGE_BYTES, MAX_REPLAY_BYTES)).is_ok());
    assert_eq!(
        decode(&json(MAX_STAGE_BYTES + 1, 0)),
        Err(ShareError::FileTooLarge)
    );
    assert_eq!(
        decode(&json(0, MAX_REPLAY_BYTES + 1)),
        Err(ShareError::FileTooLarge)
    );
}

#[test]
fn a_code_over_the_length_limit_is_refused_before_decoding() {
    assert_eq!(
        decode(&"A".repeat(MAX_SHARE_CODE_CHARS + 1)),
        Err(ShareError::TooLong)
    );
}

#[test]
fn things_that_are_not_codes_are_refused() {
    assert!(matches!(decode("not base64!"), Err(ShareError::Base64(_))));
    assert!(matches!(decode(" Zg=="), Err(ShareError::Base64(_))));
    // Valid base64 that is not zlib data.
    assert_eq!(
        decode(&base64::encode(b"hello world")),
        Err(ShareError::Corrupt)
    );
    assert_eq!(decode(""), Err(ShareError::Corrupt));
    // Valid zlib data that is not the container.
    for text in [
        "[]",
        "{}",
        "{\"stage\":\"{}\"}",
        "{\"stage\":1,\"replay\":\"\"}",
        "{\"stage\":\"\",\"replay\":\"\",\"extra\":1}",
        "{\"stage\":\"\\ud800\",\"replay\":\"\"}",
    ] {
        assert_eq!(decode(&code_of(text)), Err(ShareError::Parse), "{text}");
    }
}

#[test]
fn damaged_codes_never_panic() {
    let stage = stage_bytes();
    let replay = record(&stage, &clearing_inputs()).unwrap().to_bytes();
    let code = encode(&stage, &replay).unwrap().into_bytes();
    let mut rng = SplitMix64::new(0x5A4E);
    let (mut refused, mut accepted) = (0, 0);
    for _ in 0..3000 {
        let mut bytes = code.clone();
        for _ in 0..=rng.below(2) {
            let at = rng.below(bytes.len() as u32) as usize;
            match rng.below(3) {
                0 => bytes[at] = b"ABCxyz019+/="[rng.below(12) as usize],
                1 => bytes.truncate(at.max(1)),
                _ => bytes.insert(at, b'A'),
            }
        }
        match decode(&String::from_utf8(bytes).unwrap()) {
            Ok(_) => accepted += 1,
            Err(_) => refused += 1,
        }
    }
    assert!(refused > accepted, "{refused} refused, {accepted} accepted");
}
