//! The `packs.json` manifest (FT1a, D-FETCH): one channel per core minor, read as untrusted
//! bytes. The fixtures are the shape content's `release.cjs` writes.

use stella_rain_core::CORE_MINOR;
use stella_rain_core::packs::{Packs, PacksError};

const H1: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const H2: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn channel(tag: &str, hash: &str, text: &str, url: &str) -> String {
    format!(
        r#""{tag}":{{"tag":"v{tag}.0","pack_sha256":"{hash}","text_sha256":{text},"url":"{url}"}}"#
    )
}

fn manifest(schema: u64, channels: &str) -> String {
    format!(r#"{{"schema":{schema},"seq":7,"channels":{{{channels}}}}}"#)
}

fn good(key: &str) -> String {
    channel(key, H1, "null", "https://example.com/pack.json")
}

fn load(text: &str) -> Result<Packs, PacksError> {
    Packs::from_bytes(text.as_bytes())
}

fn two_channels() -> String {
    let c2 = channel(
        "0.2",
        H1,
        &format!("\"{H2}\""),
        "https://example.com/0.2/pack.json",
    );
    let c3 = channel("0.3", H2, "null", "https://example.com/0.3/pack.json");
    manifest(1, &format!("{c2},{c3}"))
}

#[test]
fn a_two_channel_manifest_parses_and_picks_a_channel() {
    let packs = load(&two_channels()).expect("a valid manifest");
    assert_eq!(packs.seq(), 7);
    let c = packs.channel("0.2").expect("channel 0.2");
    assert_eq!(c.tag(), "v0.2.0");
    assert_eq!(c.url(), "https://example.com/0.2/pack.json");
    assert_eq!(c.pack_sha256()[0], 0x01);
    assert_eq!(c.pack_sha256()[31], 0xef);
    assert_eq!(c.text_sha256().map(|h| h[0]), Some(0xfe));
    let c3 = packs.channel("0.3").expect("channel 0.3");
    assert_eq!(c3.pack_sha256()[0], 0xfe);
    assert_eq!(c3.text_sha256(), None);
}

#[test]
fn a_channel_the_manifest_lacks_is_none() {
    let packs = load(&manifest(1, &good("0.2"))).unwrap();
    assert!(packs.channel("0.3").is_none());
    assert!(packs.channel("").is_none());
}

#[test]
fn an_empty_channel_list_parses() {
    let packs = load(&manifest(1, "")).unwrap();
    assert!(packs.channel(CORE_MINOR).is_none());
}

#[test]
fn a_manifest_over_the_cap_is_refused_before_parsing() {
    let bytes = vec![b' '; Packs::MAX_BYTES + 1];
    assert_eq!(
        Packs::from_bytes(&bytes),
        Err(PacksError::TooLarge {
            size: Packs::MAX_BYTES + 1,
            limit: 64 * 1024
        })
    );
}

#[test]
fn a_manifest_of_exactly_the_cap_is_parsed() {
    let mut text = manifest(1, &good("0.2"));
    text.push_str(&" ".repeat(Packs::MAX_BYTES - text.len()));
    assert!(load(&text).is_ok());
}

#[test]
fn an_unknown_field_is_a_parse_error() {
    let text = r#"{"schema":1,"seq":1,"channels":{},"extra":1}"#;
    assert!(matches!(load(text), Err(PacksError::Parse { .. })));
    let in_channel = manifest(
        1,
        &format!(
            r#""0.2":{{"tag":"v0.2.0","pack_sha256":"{H1}","text_sha256":null,"url":"https://a.b/c","x":1}}"#
        ),
    );
    assert!(matches!(load(&in_channel), Err(PacksError::Parse { .. })));
}

#[test]
fn not_json_is_a_parse_error() {
    assert!(matches!(load("nope"), Err(PacksError::Parse { .. })));
}

#[test]
fn schema_other_than_one_is_unsupported() {
    assert_eq!(
        load(&manifest(2, &good("0.2"))),
        Err(PacksError::UnsupportedSchema { schema: 2 })
    );
    // A newer schema may have fields this one rejects: the version is read first.
    let newer = r#"{"schema":2,"seq":1,"channels":{},"signature":"x"}"#;
    assert_eq!(
        load(newer),
        Err(PacksError::UnsupportedSchema { schema: 2 })
    );
}

#[test]
fn a_bad_channel_key_is_refused() {
    for key in ["0", "0.2.1", "v0.2", "0.", ".2", "a.b", "0.-2", "00.2 ", ""] {
        let text = manifest(1, &good(key));
        assert!(
            matches!(load(&text), Err(PacksError::ChannelKey { .. })),
            "{key:?}"
        );
    }
}

#[test]
fn a_bad_hash_is_refused() {
    let short = &H1[..63];
    let upper = H1.to_uppercase();
    let non_hex = format!("g{}", &H1[1..]);
    for bad in [short, upper.as_str(), non_hex.as_str(), ""] {
        let text = manifest(1, &channel("0.2", bad, "null", "https://a.b/c"));
        assert!(
            matches!(
                load(&text),
                Err(PacksError::Hash {
                    field: "pack_sha256",
                    ..
                })
            ),
            "{bad:?}"
        );
        let text = manifest(
            1,
            &channel("0.2", H1, &format!("\"{bad}\""), "https://a.b/c"),
        );
        assert!(
            matches!(
                load(&text),
                Err(PacksError::Hash {
                    field: "text_sha256",
                    ..
                })
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn a_url_must_be_https_and_short() {
    for url in [
        "http://a.b/c",
        "ftp://a.b",
        "https://",
        "a.b/c",
        "https://a b/c",
    ] {
        let text = manifest(1, &channel("0.2", H1, "null", url));
        assert!(
            matches!(load(&text), Err(PacksError::Url { .. })),
            "{url:?}"
        );
    }
    let long = format!("https://a.b/{}", "x".repeat(512 - "https://a.b/".len()));
    assert_eq!(long.len(), 512);
    assert!(load(&manifest(1, &channel("0.2", H1, "null", &long))).is_ok());
    let over = format!("{long}x");
    assert!(matches!(
        load(&manifest(1, &channel("0.2", H1, "null", &over))),
        Err(PacksError::Url { .. })
    ));
}

#[test]
fn a_bad_tag_is_refused() {
    for tag in ["", "v0.2.0 x", "v0.2.0/x", &"v".repeat(65)] {
        let text = manifest(
            1,
            &format!(
                r#""0.2":{{"tag":"{tag}","pack_sha256":"{H1}","text_sha256":null,"url":"https://a.b/c"}}"#
            ),
        );
        assert!(
            matches!(load(&text), Err(PacksError::Tag { .. })),
            "{tag:?}"
        );
    }
}

#[test]
fn seventeen_channels_are_refused_and_sixteen_parse() {
    let many = |n: u32| {
        (0..n)
            .map(|i| good(&format!("0.{i}")))
            .collect::<Vec<_>>()
            .join(",")
    };
    assert!(load(&manifest(1, &many(16))).is_ok());
    assert_eq!(
        load(&manifest(1, &many(17))),
        Err(PacksError::TooManyChannels {
            found: 17,
            limit: 16
        })
    );
}

#[test]
fn seq_is_a_u64() {
    let text = format!(r#"{{"schema":1,"seq":{},"channels":{{}}}}"#, u64::MAX);
    assert_eq!(load(&text).unwrap().seq(), u64::MAX);
    let neg = r#"{"schema":1,"seq":-1,"channels":{}}"#;
    assert!(matches!(load(neg), Err(PacksError::Parse { .. })));
}

#[test]
fn errors_do_not_echo_the_manifest() {
    let secret = "secret-text-from-the-file";
    let text = manifest(1, &channel("0.2", H1, "null", &format!("http://{secret}")));
    let message = load(&text).unwrap_err().to_string();
    assert!(!message.contains(secret), "{message}");
}

#[test]
fn core_minor_is_the_major_and_minor_of_the_package_version() {
    let version = env!("CARGO_PKG_VERSION");
    let mut parts = version.split('.');
    let expected = format!("{}.{}", parts.next().unwrap(), parts.next().unwrap());
    assert_eq!(CORE_MINOR, expected);
    // The key of the channel an app of this build reads is itself a valid channel key.
    let packs = load(&manifest(1, &good(CORE_MINOR))).unwrap();
    assert!(packs.channel(CORE_MINOR).is_some());
}

#[test]
fn a_channel_key_twice_is_a_parse_error() {
    let text = manifest(1, &format!("{},{}", good("0.2"), good("0.2")));
    assert!(matches!(load(&text), Err(PacksError::Parse { .. })));
}
