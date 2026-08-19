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
    assert!(decoded.ordering_is_valid());
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

// ── chat policy (spec 07 §4.3, Core §2.6.2) ────────────────────────────

#[test]
fn an_undeclared_policy_reads_as_defaults_and_a_server() {
    let policy = intranet_governance::NetworkPolicy::conservative_default();
    let chat = ChatPolicy::of(&policy);

    // Absent means default, never refused — the asymmetry Core §2.6.2 draws
    // against the capability registry.
    assert_eq!(chat.message_rate_per_minute(), defaults::MESSAGE_RATE);
    assert_eq!(chat.segment_max_bytes(), defaults::SEGMENT_MAX_BYTES as usize);

    // And a network that never declared a profile is a server, so its existing
    // channel history stays valid.
    assert_eq!(chat.profile(), NetworkProfile::Server);
    assert!(chat.allows_channel_definitions());
}

#[test]
fn a_conversation_declares_itself_and_forbids_channels() {
    let mut policy = intranet_governance::NetworkPolicy::conservative_default();
    policy.app_policy.extend(conversation_genesis_values());
    let chat = ChatPolicy::of(&policy);

    assert_eq!(chat.profile(), NetworkProfile::Conversation);
    assert!(!chat.allows_channel_definitions());
}

#[test]
fn an_unrecognised_profile_reads_as_a_server() {
    let mut policy = intranet_governance::NetworkPolicy::conservative_default();
    policy.app_policy.insert(
        keys::PROFILE.to_owned(),
        intranet_governance::PolicyValue::Text("something-later".to_owned()),
    );
    // Same reasoning as an absent profile: refuse nothing a node may already
    // hold. A future profile this build cannot enforce must not silently
    // invalidate that network's history here.
    assert_eq!(ChatPolicy::of(&policy).profile(), NetworkProfile::Server);
}

#[test]
fn a_negative_size_falls_back_rather_than_wrapping() {
    let mut policy = intranet_governance::NetworkPolicy::conservative_default();
    policy.app_policy.insert(
        keys::SEGMENT_MAX_BYTES.to_owned(),
        intranet_governance::PolicyValue::Int(-1),
    );
    // -1 as usize is a limit nothing could exceed, so a nonsensical setting
    // would silently disable the bound instead of tightening it.
    assert_eq!(
        ChatPolicy::of(&policy).segment_max_bytes(),
        defaults::SEGMENT_MAX_BYTES as usize
    );
}

#[test]
fn a_wrongly_typed_value_falls_back_to_the_default() {
    let mut policy = intranet_governance::NetworkPolicy::conservative_default();
    policy.app_policy.insert(
        keys::MESSAGE_RATE.to_owned(),
        intranet_governance::PolicyValue::Text("thirty".to_owned()),
    );
    assert_eq!(
        ChatPolicy::of(&policy).message_rate_per_minute(),
        defaults::MESSAGE_RATE
    );
}

// ── retention (`design/01` §8) ─────────────────────────────────────────

#[test]
fn a_network_that_never_configured_retention_keeps_everything() {
    // The default, and the direction of the choice. Retention can be switched on
    // whenever a network decides it wants it; content already allowed to go dark
    // cannot be brought back. A network that never thinks about this keeping its
    // history is both what a chat product is expected to do and the recoverable
    // side of the mistake.
    let policy = intranet_governance::NetworkPolicy::conservative_default();
    let chat = ChatPolicy::of(&policy);

    assert_eq!(chat.retain_messages(), Retention::Forever);
    assert_eq!(chat.retain_attachments(), Retention::Forever);
    assert!(chat.retain_messages().covers(u32::MAX));
}

#[test]
fn messages_and_attachments_carry_separate_windows() {
    // The reason there are two. A message is bounded at 8 KiB with a capped
    // rate, so a million of them is a few gigabytes network-wide; one attachment
    // may be 25 MiB, ten to a message. A network bounding what it spends on
    // other members' disks means the attachments, and one shared window would
    // charge it the scrollback as well.
    let mut policy = intranet_governance::NetworkPolicy::conservative_default();
    policy.app_policy.insert(
        keys::RETAIN_ATTACHMENTS_DAYS.to_owned(),
        intranet_governance::PolicyValue::Int(30),
    );
    let chat = ChatPolicy::of(&policy);

    assert_eq!(chat.retain_messages(), Retention::Forever);
    assert_eq!(chat.retain_attachments(), Retention::Days(30));
    assert!(chat.retain_attachments().covers(30), "the window is inclusive");
    assert!(!chat.retain_attachments().covers(31));
}

#[test]
fn a_meaningless_window_reads_as_forever_rather_than_as_expire_now() {
    // Fail-safe, and the failure it guards is severe: a value that arrived
    // corrupted, or written by a client with a different idea of the key, must
    // not quietly start discarding a network's history. Zero is the documented
    // way to say "forever", and a negative or absurd value is treated the same
    // rather than being read as a window that has already passed.
    for value in [0i64, -1, -86_400, i64::MIN, i64::MAX] {
        let mut policy = intranet_governance::NetworkPolicy::conservative_default();
        policy.app_policy.insert(
            keys::RETAIN_MESSAGES_DAYS.to_owned(),
            intranet_governance::PolicyValue::Int(value),
        );
        assert_eq!(
            ChatPolicy::of(&policy).retain_messages(),
            Retention::Forever,
            "{value} must read as forever"
        );
    }
}

#[test]
fn a_configured_window_survives_encoding_like_any_other_policy_value() {
    // Retention is a validity-adjacent setting every node must agree on, so it
    // rides the app policy map (Core §2.6.2) and has to encode identically on
    // two nodes that built the same logical policy.
    let mut policy = intranet_governance::NetworkPolicy::conservative_default();
    policy.app_policy.insert(
        keys::RETAIN_MESSAGES_DAYS.to_owned(),
        intranet_governance::PolicyValue::Int(365),
    );
    policy.app_policy.insert(
        keys::RETAIN_ATTACHMENTS_DAYS.to_owned(),
        intranet_governance::PolicyValue::Int(30),
    );

    let mut rebuilt = intranet_governance::NetworkPolicy::conservative_default();
    for (key, value) in policy.app_policy.iter().rev() {
        rebuilt.app_policy.insert(key.clone(), value.clone());
    }

    let mut a = intranet_crypto::Enc::domain("test");
    let mut b = intranet_crypto::Enc::domain("test");
    policy.encode(&mut a);
    rebuilt.encode(&mut b);
    assert_eq!(a.finish(), b.finish(), "insertion order must not matter");
    assert_eq!(
        ChatPolicy::of(&rebuilt).retain_messages(),
        Retention::Days(365)
    );
}

// ── live delivery (spec 07 §5.2, §6.1) ─────────────────────────────────

fn epoch(byte: u8) -> intranet_storage::EpochKey {
    intranet_storage::EpochKey::from_bytes([byte; 32])
}

fn rotation(byte: u8) -> intranet_crypto::Hash {
    intranet_crypto::Hash::from_bytes([byte; 32])
}

#[test]
fn a_live_payload_round_trips_and_yields_the_identical_record() {
    // The property the whole live path rests on: what travels live and what a
    // segment carries months later are the same bytes. That is what makes a
    // record delivered either way independently verifiable and identically
    // addressed, so a duplicate is genuinely idempotent rather than merely
    // similar.
    let author = identity(1);
    let record = Record::create(&author, channel(), Hlc::new(1_700_000_000_000, 0), message("live"));

    let sealed = LivePayload::seal(&record, &epoch(7), rotation(3));
    let decoded = LivePayload::decode(&sealed.encode()).expect("round trips");
    assert_eq!(decoded, sealed);

    let opened = decoded.open(&epoch(7)).expect("opens under the sealing key");
    assert_eq!(opened, record);
    assert_eq!(
        opened.canonical_bytes(),
        record.canonical_bytes(),
        "byte-identical, so the same record from either path has one id"
    );
    assert_eq!(opened.id(), record.id());
}

#[test]
fn a_payload_does_not_open_under_the_wrong_epoch_or_the_wrong_rotation() {
    // The content key binds both, so a payload cannot be opened by somebody
    // holding a different epoch, and cannot be replayed across a rotation into a
    // window it was not published in.
    let author = identity(1);
    let record = Record::create(&author, channel(), Hlc::new(1, 0), message("bound"));
    let sealed = LivePayload::seal(&record, &epoch(7), rotation(3));

    assert!(
        sealed.open(&epoch(8)).is_err(),
        "another epoch's holder must not be able to read this"
    );

    let mistagged = LivePayload {
        rotation: rotation(4),
        ..sealed.clone()
    };
    assert!(
        mistagged.open(&epoch(7)).is_err(),
        "the rotation is bound into the key, not merely carried beside it"
    );
}

#[test]
fn a_record_cannot_be_relayed_into_another_channel() {
    // The same lift spec 07 §3.5 guards against inside a segment, on the live
    // path: a validly-signed record moved into another channel's payload. Its
    // signature stays genuine throughout, so checking signatures alone does not
    // catch it.
    let author = identity(1);
    let record = Record::create(&author, channel(), Hlc::new(1, 0), message("mine"));
    let sealed = LivePayload::seal(&record, &epoch(7), rotation(3));

    let elsewhere = server_channel_id(&network(), &[77u8; 32]);
    let relayed = LivePayload {
        channel: elsewhere,
        ..sealed
    };
    assert!(
        relayed.open(&epoch(7)).is_err(),
        "a record must belong to the channel whose key opened it"
    );
}

#[test]
fn a_tampered_payload_is_refused_rather_than_rendered() {
    let author = identity(1);
    let record = Record::create(&author, channel(), Hlc::new(1, 0), message("intact"));
    let mut sealed = LivePayload::seal(&record, &epoch(7), rotation(3));
    sealed.sealed[0] ^= 0x01;

    assert!(
        sealed.open(&epoch(7)).is_err(),
        "the AEAD is what detects this, and it must fail closed"
    );
}

#[test]
fn the_content_key_is_derived_and_stable_but_not_the_epoch_key() {
    // Derived from a key the member already holds, so it adds no trust
    // assumption — and distinct from it, so a live payload's key never *is* the
    // network's epoch key travelling under another name.
    let a = channel_content_key(&epoch(7), &channel(), &rotation(3));
    let again = channel_content_key(&epoch(7), &channel(), &rotation(3));
    assert_eq!(a.commitment(), again.commitment(), "derivation is stable");

    let other_channel =
        channel_content_key(&epoch(7), &server_channel_id(&network(), &[5u8; 32]), &rotation(3));
    let other_rotation = channel_content_key(&epoch(7), &channel(), &rotation(4));
    assert_ne!(a.commitment(), other_channel.commitment());
    assert_ne!(a.commitment(), other_rotation.commitment());

    // Compared by commitment, since a DEK does not expose its bytes — which is
    // the point of the type. A content key equal to the epoch key would make
    // these commitments equal too.
    let epoch_as_dek = intranet_storage::Dek::from_bytes(*epoch(7).expose_for_delivery());
    assert_ne!(
        a.commitment(),
        epoch_as_dek.commitment(),
        "the content key must be derived from the epoch key, not be it"
    );
}
