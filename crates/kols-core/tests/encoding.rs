//! Conformance tests for `design/08-record-encoding.md` §10.
//!
//! These are the contract, not a description of the implementation. A change
//! that alters a vector below is a wire break: it must be recognised as one and
//! versioned, never re-blessed by updating the expected value.

use intranet_crypto::to_hex;
use kols_core::*;

/// A deterministic identity, so vectors are reproducible.
///
/// Test identities are derived from a fixed seed rather than generated: a
/// generated key would make every vector below change on every run, which would
/// defeat the entire purpose of having them.
fn identity(byte: u8) -> intranet_identity::PerNetworkIdentity {
    intranet_identity::MasterSeed::from_entropy([byte; 32])
        .identity_for(&network())
        .expect("derives")
}

fn network() -> intranet_identity::NetworkId {
    intranet_identity::NetworkId::from_bytes([7u8; 32])
}

fn channel() -> ChannelId {
    server_channel_id(&network(), &[9u8; 32])
}

fn message(text: &str) -> RecordBody {
    RecordBody::Message {
        body: text.to_owned(),
        reply_to: None,
        attachments: Vec::new(),
    }
}

// ── §10.1 fixed vectors ────────────────────────────────────────────────

#[test]
fn derived_identifiers_are_stable() {
    let net = network();
    let ch = channel();
    let author = identity(1).id();

    // If any of these change, every existing client stops finding existing
    // content — the derivations are how members locate each other's logs.
    assert_eq!(
        to_hex(ch.as_bytes()),
        "1bc2dec1a6cf8984e573c244620ce54922b6c68361c4c939e0bb83f0a984a2a3"
    );
    assert_eq!(
        to_hex(conversation_channel_id(&net).as_bytes()),
        "39f933a61a8951828f75862e4a88a495038244731a943660b5cd165539a8d573"
    );
    assert_eq!(
        to_hex(author_log_pointer(&ch, &author).as_bytes()),
        "1bbf02026f2d6cd2277c58ca1e6a5cec6b97c9cf2c8010c29d2c793c3b461983"
    );
}

/// The wire format of one complete record, field by field.
///
/// Split across lines so a diff shows *which* field moved: domain tag, channel,
/// author, device, HLC wall, HLC counter, discriminant, body, absent `reply_to`,
/// empty attachment count, then the 64-byte signature.
#[test]
fn record_encoding_is_stable() {
    let author = identity(1);
    let record = Record::create(&author, channel(), Hlc::new(1_700_000_000_000, 0), message("hi"));
    assert_eq!(
        to_hex(&record.canonical_bytes()),
        concat!(
            "0000000000000017696e7472616e65742e636861742d7265636f72642e7631",
            "1bc2dec1a6cf8984e573c244620ce54922b6c68361c4c939e0bb83f0a984a2a3",
            "198394e8dfadfe461f9b608d53cb75cc89b01ce6209df9e4e07d52ecc4597da9",
            "198394e8dfadfe461f9b608d53cb75cc89b01ce6209df9e4e07d52ecc4597da9",
            "0000018bcfe56800",
            "00000000",
            "01",
            "00000000000000026869",
            "00",
            "0000000000000000",
            "f38b0ebd14ca1818e2054e9a0627f869c5bdd402c95d69b6889bdd83e2e90936",
            "bf6e9bc35ea004e24729cb7e5f79b2926e7e865a923148abe710c46b36ebdc06",
        )
    );
}

// ── §10.2 round-trip ───────────────────────────────────────────────────

#[test]
fn every_body_round_trips() {
    let author = identity(1);
    let target = MessageId::from_bytes([3u8; 32]);
    let bodies = vec![
        message("plain"),
        RecordBody::Message {
            body: "with everything".to_owned(),
            reply_to: Some(target),
            attachments: vec![Attachment {
                manifest_cid: intranet_crypto::Hash::from_bytes([4u8; 32]),
                byte_len: 1234,
                media_type: "image/png".to_owned(),
                name: "screenshot.png".to_owned(),
            }],
        },
        RecordBody::Message {
            body: String::new(),
            reply_to: None,
            attachments: Vec::new(),
        },
        RecordBody::Edit {
            target,
            body: "revised".to_owned(),
        },
        RecordBody::Tombstone { target },
        RecordBody::Reaction {
            target,
            key: "👍".to_owned(),
            remove: false,
        },
        RecordBody::Pin {
            target,
            remove: true,
        },
        RecordBody::Redaction {
            target,
            governance_head: intranet_crypto::Hash::from_bytes([5u8; 32]),
        },
    ];

    for (i, body) in bodies.into_iter().enumerate() {
        let record = Record::create(&author, channel(), Hlc::new(1_000 + i as i64, 0), body);
        let decoded = Record::decode(&record.canonical_bytes()).expect("decodes");
        assert_eq!(decoded, record, "body {i} did not round-trip");
        assert_eq!(decoded.id(), record.id());
        decoded.verify_signature().expect("signature verifies");
    }
}

#[test]
fn segment_round_trips_and_re_emits_identically() {
    let author = identity(1);
    let mut segment = Segment::new(channel(), author.id(), 0, None);
    for i in 0..8 {
        segment.records.push(Record::create(
            &author,
            channel(),
            Hlc::new(2_000, i as u32),
            message(&format!("message {i}")),
        ));
    }

    let bytes = segment.canonical_bytes();
    let decoded = Segment::decode(&bytes).expect("decodes");
    assert_eq!(decoded, segment);
    // §6 point 2: a record read from a segment re-emits byte-identically, which
    // is what makes an id stable across the durable and live paths.
    assert_eq!(decoded.canonical_bytes(), bytes);
    decoded.verify().expect("verifies");
    assert!(decoded.hlcs_strictly_increase());
}

// ── §10.3 injectivity ──────────────────────────────────────────────────

#[test]
fn adjacent_fields_cannot_be_confused() {
    let author = identity(1);
    // The classic length-prefix failure: ("ab", "c") and ("a", "bc") must not
    // encode alike. Reaction carries two adjacent variable-length-ish fields.
    let a = Record::create(
        &author,
        channel(),
        Hlc::new(1, 0),
        RecordBody::Reaction {
            target: MessageId::from_bytes([1u8; 32]),
            key: "ab".to_owned(),
            remove: false,
        },
    );
    let b = Record::create(
        &author,
        channel(),
        Hlc::new(1, 0),
        RecordBody::Reaction {
            target: MessageId::from_bytes([1u8; 32]),
            key: "a".to_owned(),
            remove: false,
        },
    );
    assert_ne!(a.canonical_bytes(), b.canonical_bytes());
    assert_ne!(a.id(), b.id());
}

#[test]
fn variants_with_identical_payloads_do_not_collide() {
    let author = identity(1);
    let target = MessageId::from_bytes([1u8; 32]);
    // Tombstone and a hypothetical same-shaped body differ only by discriminant.
    let tombstone = Record::create(
        &author,
        channel(),
        Hlc::new(1, 0),
        RecordBody::Tombstone { target },
    );
    let pin = Record::create(
        &author,
        channel(),
        Hlc::new(1, 0),
        RecordBody::Pin {
            target,
            remove: false,
        },
    );
    assert_ne!(tombstone.canonical_bytes(), pin.canonical_bytes());
}

// ── §10.4 domain separation ────────────────────────────────────────────

#[test]
fn a_record_signature_does_not_verify_as_another_type() {
    let author = identity(1);
    let record = Record::create(&author, channel(), Hlc::new(1, 0), message("hi"));

    // The same bytes under a different domain tag must not verify. Without
    // domain separation a signature over one type could be replayed as another.
    let mut forged = intranet_crypto::Enc::domain("intranet.chat-segment.v1");
    forged.bytes(&record.canonical_bytes());
    assert!(
        author
            .id()
            .verifying_key()
            .verify(&forged, &record.signature)
            .is_err()
    );
}

// ── §10.5 id stability ─────────────────────────────────────────────────

#[test]
fn signing_twice_yields_the_same_id() {
    let author = identity(1);
    let a = Record::create(&author, channel(), Hlc::new(42, 7), message("same"));
    let b = Record::create(&author, channel(), Hlc::new(42, 7), message("same"));
    // Ed25519 is deterministic (RFC 8032), which is what makes ids stable
    // despite being hashed over the signature.
    assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    assert_eq!(a.id(), b.id());
}

#[test]
fn different_authors_never_share_an_id() {
    let a = Record::create(&identity(1), channel(), Hlc::new(1, 0), message("hello"));
    let b = Record::create(&identity(2), channel(), Hlc::new(1, 0), message("hello"));
    assert_ne!(a.id(), b.id());
}

// ── rate class, §5.2 ───────────────────────────────────────────────────

#[test]
fn class_is_derivable_from_an_unknown_discriminant() {
    // The property that matters: a node that has never heard of these kinds
    // still classifies them correctly for rate limiting.
    assert_eq!(RecordClass::of(0x02), RecordClass::Message);
    assert_eq!(RecordClass::of(0x3F), RecordClass::Message);
    assert_eq!(RecordClass::of(0x40), RecordClass::Reaction);
    assert_eq!(RecordClass::of(0x7F), RecordClass::Reaction);
    assert_eq!(RecordClass::of(0x80), RecordClass::Control);
    assert_eq!(RecordClass::of(0xC0), RecordClass::Reserved);
    assert_eq!(RecordClass::of(0x00), RecordClass::Reserved);
}

#[test]
fn an_unknown_kind_decodes_as_unknown_rather_than_garbage() {
    let author = identity(1);
    let mut bytes = Record::create(&author, channel(), Hlc::new(1, 0), message("hi"))
        .canonical_bytes();
    // Flip the discriminant to something unallocated.
    // Locate the discriminant deterministically: length-prefixed domain tag,
    // then channel + author + device (32 each), then the 12-byte HLC.
    let idx = 8 + "intranet.chat-record.v1".len() + 96 + 12;
    bytes[idx] = 0x7E;
    assert!(matches!(
        Record::decode(&bytes),
        Err(CoreError::UnknownKind(0x7E))
    ));
}

// ── bounds ─────────────────────────────────────────────────────────────

#[test]
fn oversized_fields_are_refused_not_truncated() {
    let author = identity(1);
    let long = "x".repeat(DEFAULT_MAX_BODY_BYTES + 1);
    let record = Record::create(&author, channel(), Hlc::new(1, 0), message(&long));
    assert!(matches!(
        record.check_bounds(DEFAULT_MAX_BODY_BYTES),
        Err(CoreError::TooLarge { .. })
    ));

    let key = "k".repeat(MAX_REACTION_KEY_BYTES + 1);
    let reaction = Record::create(
        &author,
        channel(),
        Hlc::new(1, 0),
        RecordBody::Reaction {
            target: MessageId::from_bytes([1u8; 32]),
            key,
            remove: false,
        },
    );
    assert!(matches!(
        reaction.check_bounds(DEFAULT_MAX_BODY_BYTES),
        Err(CoreError::TooLarge { .. })
    ));
}

// ── hlc ────────────────────────────────────────────────────────────────

#[test]
fn hlc_advances_the_counter_when_the_clock_does_not() {
    let first = Hlc::next(1_000, None);
    assert_eq!(first, Hlc::new(1_000, 0));

    // Clock stalled: the counter must move, or two records share a reading and
    // both become invalid under §4.1.
    let second = Hlc::next(1_000, Some(first));
    assert_eq!(second, Hlc::new(1_000, 1));
    assert!(second > first);

    // Clock stepped backwards: still strictly increasing.
    let third = Hlc::next(900, Some(second));
    assert!(third > second);

    // Clock moved on: counter resets.
    assert_eq!(Hlc::next(2_000, Some(third)), Hlc::new(2_000, 0));
}
