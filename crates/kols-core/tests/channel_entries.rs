//! Channel structure as `chat` application entries — spec 07 §1.3, §1.2, §3.8.
//!
//! Two of these tests exist because of what E2's generalised form moved onto
//! readers. The protocol gained one application entry rather than four
//! chat-shaped ones, which was the right call and cost two guarantees: the
//! protocol enforces the capability an entry *declares* rather than the right
//! one, and it cannot reject a channel entry in a conversation network because it
//! never decodes the payload. Both are now every conformant client's job, so both
//! are pinned here.

use intranet_governance::Capability;
use kols_core::*;

fn network() -> intranet_identity::NetworkId {
    intranet_identity::NetworkId::from_bytes([7u8; 32])
}

fn identity(byte: u8) -> intranet_identity::PerNetworkIdentityId {
    intranet_identity::MasterSeed::from_entropy([byte; 32])
        .identity_for(&network())
        .expect("derives")
        .id()
}

fn channel() -> ChannelId {
    server_channel_id(&network(), &[9u8; 32])
}

fn category() -> CategoryId {
    CategoryId::from_bytes([3u8; 32])
}

fn definition(category: Option<CategoryId>) -> ChannelEntry {
    ChannelEntry::new(
        channel(),
        ChannelEntryBody::Definition {
            name: "general".to_owned(),
            category,
            kind: ChannelKind::Text,
            privacy: Privacy::Public,
            topic: "anything at all".to_owned(),
            slowmode: 0,
        },
    )
}

fn cap(name: &str) -> Capability {
    Capability::extension(name.to_owned())
}

// ── encoding ───────────────────────────────────────────────────────────

#[test]
fn every_channel_entry_round_trips() {
    let entries = [
        definition(None),
        definition(Some(category())),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Definition {
                name: "mods".to_owned(),
                category: Some(category()),
                kind: ChannelKind::Voice,
                privacy: Privacy::Private,
                topic: String::new(),
                slowmode: 21_600,
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Rename("renamed".to_owned()),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Recategorise(None),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Recategorise(Some(category())),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::SetTopic("a topic".to_owned()),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::SetSlowmode(30),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Archive,
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Delete,
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Membership {
                action: MembershipAction::Add,
                identity: identity(2),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Membership {
                action: MembershipAction::Remove,
                identity: identity(3),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Rotation {
                commit_ref: intranet_crypto::Hash::from_bytes([5u8; 32]),
                reason: "member removed".to_owned(),
            },
        ),
    ];

    for entry in entries {
        let bytes = entry.encode();
        let decoded = ChannelEntry::decode(entry.body.kind(), &bytes).expect("decodes");
        assert_eq!(decoded, entry, "round trip for {}", entry.body.kind());
        assert_eq!(decoded.encode(), bytes, "re-encoding is byte-identical");
    }
}

#[test]
fn no_two_channel_entries_encode_alike() {
    // Length-prefixing and discriminants exist to guarantee this. A collision
    // would mean two different pieces of channel structure hashing to one
    // governance entry id.
    let mut seen = std::collections::BTreeSet::new();
    let entries = [
        definition(None),
        definition(Some(category())),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Rename("general".to_owned()),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::SetTopic("general".to_owned()),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Archive,
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Delete,
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Membership {
                action: MembershipAction::Add,
                identity: identity(2),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Membership {
                action: MembershipAction::Remove,
                identity: identity(2),
            },
        ),
    ];
    for entry in entries {
        assert!(
            seen.insert(entry.encode()),
            "{} collided with an earlier encoding",
            entry.body.kind()
        );
    }
}

#[test]
fn a_channel_entry_does_not_decode_as_a_record() {
    // Domain separation, asserted rather than assumed. Both are chat payloads
    // that begin with a 32-byte channel id, so only the domain tag distinguishes
    // them at the front of the bytes.
    let entry = definition(None);
    assert!(
        Record::decode(&entry.encode()).is_err(),
        "a channel entry must not decode as a record"
    );
}

#[test]
fn a_kind_that_disagrees_with_its_payload_is_refused() {
    // The `kind` string rides in the entry envelope, which the protocol reads;
    // the discriminant is in bytes it never decodes. Checking them against each
    // other catches a change to one that could not have changed the other.
    let entry = definition(None);
    let bytes = entry.encode();

    assert!(ChannelEntry::decode("channel-definition", &bytes).is_ok());
    assert!(
        matches!(
            ChannelEntry::decode("channel-membership", &bytes),
            Err(CoreError::ChannelKindMismatch { .. })
        ),
        "a mismatched kind must be refused, not silently trusted"
    );
}

#[test]
fn an_unallocated_discriminant_is_refused_rather_than_skipped() {
    // The opposite of the rule for record kinds, and deliberately so. An unknown
    // record is retained, counted and not rendered; an unknown channel entry
    // carries structure, and a reader that skipped it would hold different
    // channel state from one that understood it.
    let mut bytes = definition(None).encode();
    let offset = find_kind_offset(&bytes);
    bytes[offset] = 0x7F;
    assert!(
        matches!(
            ChannelEntry::decode_payload(&bytes),
            Err(CoreError::UnknownChannelField(..))
        ),
        "an unallocated entry kind must be refused"
    );
}

/// Locates the entry-kind discriminant: it follows the domain tag and the
/// 32-byte channel id, and the domain tag is length-prefixed by `Enc::domain`.
fn find_kind_offset(bytes: &[u8]) -> usize {
    let tag = b"intranet.chat-channel-entry.v1";
    let start = bytes
        .windows(tag.len())
        .position(|w| w == tag)
        .expect("the domain tag is in the encoding");
    start + tag.len() + 32
}

// ── the two checks E2's generalisation moved onto readers ───────────────

#[test]
fn a_channel_entry_is_refused_in_a_conversation_network() {
    // Spec 07 §1.2. A conversation has exactly one implied channel, derived from
    // the network id and declared nowhere. The protocol carries `chat` payloads
    // without decoding them, so it cannot reach this verdict and does not claim
    // to — which makes this every conformant reader's job.
    let entry = definition(None);
    let declared = cap(&format!(
        "chat:create-channel:{}",
        intranet_crypto::to_hex(channel().as_bytes())
    ));

    assert_eq!(
        admit(
            &entry,
            CHAT_NAMESPACE,
            &declared,
            NetworkProfile::Conversation,
            None
        ),
        Err(ChannelRefusal::NotAServer)
    );
    assert!(
        admit(
            &entry,
            CHAT_NAMESPACE,
            &declared,
            NetworkProfile::Server,
            None
        )
        .is_ok(),
        "and is admitted in a server, so the refusal is the profile and not the entry"
    );
}

#[test]
fn an_entry_declaring_a_capability_it_does_not_need_is_refused() {
    // The gap the generic form opened, and the reason this check exists. The
    // protocol verified the author holds what the entry declared — so an author
    // holding only `chat:post:*`, the most ordinary grant a network issues, gets
    // an entry accepted into the log by declaring that. Only a reader that
    // understands `chat` knows a channel definition needed `chat:create-channel`.
    let entry = definition(None);
    let scope = intranet_crypto::to_hex(channel().as_bytes());

    let refusal = admit(
        &entry,
        CHAT_NAMESPACE,
        &cap("chat:post:*"),
        NetworkProfile::Server,
        None,
    );
    assert!(
        matches!(refusal, Err(ChannelRefusal::WrongCapability { .. })),
        "a definition must not be authorized by a posting grant: {refusal:?}"
    );

    assert!(
        admit(
            &entry,
            CHAT_NAMESPACE,
            &cap(&format!("chat:create-channel:{scope}")),
            NetworkProfile::Server,
            None
        )
        .is_ok()
    );
}

#[test]
fn creating_is_ordinary_and_changing_is_governance() {
    // Spec 07 §3.8's split, which is the decision this vocabulary turns on. A
    // definition grants nobody access to anything — a new private channel has an
    // empty roster until a membership entry adds someone — so it is authorized by
    // the ordinary verb. Everything that can widen access needs the governance
    // one, and must not be accepted under the ordinary one.
    let scope = intranet_crypto::to_hex(channel().as_bytes());
    let create = cap(&format!("chat:create-channel:{scope}"));
    let manage = cap(&format!("chat:manage-channel:{scope}"));

    let widening = [
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Membership {
                action: MembershipAction::Add,
                identity: identity(2),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Update {
                change: ChannelChange::Rename("elsewhere".to_owned()),
            },
        ),
        ChannelEntry::new(
            channel(),
            ChannelEntryBody::Rotation {
                commit_ref: intranet_crypto::Hash::from_bytes([5u8; 32]),
                reason: String::new(),
            },
        ),
    ];

    for entry in &widening {
        assert!(
            matches!(
                admit(entry, CHAT_NAMESPACE, &create, NetworkProfile::Server, None),
                Err(ChannelRefusal::WrongCapability { .. })
            ),
            "{} must not be authorized by the ordinary create verb",
            entry.body.kind()
        );
        assert!(
            admit(entry, CHAT_NAMESPACE, &manage, NetworkProfile::Server, None).is_ok(),
            "{} is authorized by manage-channel",
            entry.body.kind()
        );
    }

    // And the reverse: a definition is not gated behind governance power.
    assert!(
        admit(
            &definition(None),
            CHAT_NAMESPACE,
            &create,
            NetworkProfile::Server,
            None
        )
        .is_ok()
    );
}

#[test]
fn a_capability_for_another_channel_does_not_authorize_this_one() {
    // Exact-name matching gives this directly, and it is worth pinning because
    // E11's proposed prefix registration must not weaken it: a namespace entry
    // covering `chat:manage-channel:` would still have to refuse a *name* naming
    // somebody else's channel.
    let elsewhere = server_channel_id(&network(), &[11u8; 32]);
    let entry = ChannelEntry::new(
        channel(),
        ChannelEntryBody::Update {
            change: ChannelChange::Delete,
        },
    );

    let refusal = admit(
        &entry,
        CHAT_NAMESPACE,
        &cap(&format!(
            "chat:manage-channel:{}",
            intranet_crypto::to_hex(elsewhere.as_bytes())
        )),
        NetworkProfile::Server,
        None,
    );
    assert!(
        matches!(refusal, Err(ChannelRefusal::WrongCapability { .. })),
        "a grant for one channel must not carry to another: {refusal:?}"
    );
}

#[test]
fn scope_resolution_accepts_category_and_network_grants() {
    // `design/02` §3's order, one level deep. Category scope is the default a
    // network is expected to administer with, so an entry declaring it must be
    // accepted — otherwise every channel needs its own grant and the scaling
    // argument for categories collapses.
    let entry = ChannelEntry::new(
        channel(),
        ChannelEntryBody::Update {
            change: ChannelChange::Archive,
        },
    );
    let by_category = cap(&format!(
        "chat:manage-channel:cat:{}",
        intranet_crypto::to_hex(category().as_bytes())
    ));

    assert!(
        admit(
            &entry,
            CHAT_NAMESPACE,
            &by_category,
            NetworkProfile::Server,
            Some(&category())
        )
        .is_ok(),
        "a category grant authorizes a channel inside it"
    );
    assert!(
        matches!(
            admit(
                &entry,
                CHAT_NAMESPACE,
                &by_category,
                NetworkProfile::Server,
                None
            ),
            Err(ChannelRefusal::WrongCapability { .. })
        ),
        "but not a channel that is not in it"
    );
    assert!(
        admit(
            &entry,
            CHAT_NAMESPACE,
            &cap("chat:manage-channel:*"),
            NetworkProfile::Server,
            None
        )
        .is_ok(),
        "and the network-wide form authorizes anywhere"
    );
}

#[test]
fn a_definition_may_be_authorized_by_the_category_it_places_the_channel_in() {
    // The one case where the category comes from the entry rather than replayed
    // state: the channel does not exist yet, so there is nothing to look up.
    let entry = definition(Some(category()));
    assert!(
        admit(
            &entry,
            CHAT_NAMESPACE,
            &cap(&format!(
                "chat:create-channel:cat:{}",
                intranet_crypto::to_hex(category().as_bytes())
            )),
            NetworkProfile::Server,
            None
        )
        .is_ok()
    );
}

#[test]
fn a_foreign_namespace_is_refused() {
    assert!(matches!(
        admit(
            &definition(None),
            "calendar",
            &cap("chat:create-channel:*"),
            NetworkProfile::Server,
            None
        ),
        Err(ChannelRefusal::ForeignNamespace(_))
    ));
}

// ── bounds ─────────────────────────────────────────────────────────────

#[test]
fn oversized_fields_are_refused() {
    // The governance log is replayed in full by every joiner and never shrinks,
    // so these bound the log rather than the user. Not network policy for the
    // same reason: a network cannot loosen what every joiner has to replay.
    let long = "x".repeat(MAX_CHANNEL_NAME_BYTES + 1);
    let entry = ChannelEntry::new(
        channel(),
        ChannelEntryBody::Update {
            change: ChannelChange::Rename(long),
        },
    );
    assert!(matches!(
        entry.check_bounds(),
        Err(CoreError::TooLarge { .. })
    ));

    assert!(definition(None).check_bounds().is_ok());
}

#[test]
fn a_payload_stays_within_the_protocol_ceiling() {
    // Core §2.7.2 caps an application entry's payload at 8 KiB. The bounds above
    // have to keep the largest well-formed entry under it, or a client could
    // build something the log will not carry.
    let entry = ChannelEntry::new(
        channel(),
        ChannelEntryBody::Definition {
            name: "n".repeat(MAX_CHANNEL_NAME_BYTES),
            category: Some(category()),
            kind: ChannelKind::Stage,
            privacy: Privacy::Private,
            topic: "t".repeat(MAX_CHANNEL_TOPIC_BYTES),
            slowmode: u32::MAX,
        },
    );
    entry.check_bounds().expect("at the bounds, not over them");
    assert!(
        entry.encode().len() <= intranet_governance::MAX_APP_ENTRY_PAYLOAD_BYTES,
        "the largest well-formed entry must fit the protocol's payload ceiling"
    );
}

// ── frozen vectors ─────────────────────────────────────────────────────

/// The wire format of a channel definition, field by field.
///
/// Split across lines so a diff shows *which* field moved: domain tag, channel
/// id, entry discriminant, name, absent category, channel kind, privacy, topic,
/// slowmode.
///
/// **This is the contract, not a description of the code.** Spec 07 §3.8 is
/// normative and a governance entry's hash covers this payload, so a change here
/// is a wire break to be recognised and versioned — never re-blessed because a
/// test went red.
#[test]
fn channel_definition_encoding_is_stable() {
    let bytes = definition(None).encode();
    assert_eq!(
        intranet_crypto::to_hex(&bytes),
        concat!(
            "000000000000001e696e7472616e65742e636861742d6368616e6e656c2d656e7472792e7631",
            "1bc2dec1a6cf8984e573c244620ce54922b6c68361c4c939e0bb83f0a984a2a3",
            "01",
            "000000000000000767656e6572616c",
            "00",
            "01",
            "01",
            "000000000000000f616e797468696e6720617420616c6c",
            "00000000",
        )
    );
}

/// The other three kinds, pinned at their discriminants.
///
/// Less exhaustive than the definition above on purpose: what matters most for
/// these is that the leading bytes — domain, channel, kind — never shift, since
/// that is what makes one kind undecodable as another.
#[test]
fn every_channel_entry_kind_keeps_its_discriminant() {
    let cases = [
        (
            ChannelEntry::new(
                channel(),
                ChannelEntryBody::Update {
                    change: ChannelChange::Archive,
                },
            ),
            0x02u8,
            0x05u8,
        ),
        (
            ChannelEntry::new(
                channel(),
                ChannelEntryBody::Membership {
                    action: MembershipAction::Add,
                    identity: identity(2),
                },
            ),
            0x03,
            0x01,
        ),
        (
            ChannelEntry::new(
                channel(),
                ChannelEntryBody::Rotation {
                    commit_ref: intranet_crypto::Hash::from_bytes([5u8; 32]),
                    reason: String::new(),
                },
            ),
            0x04,
            0x05,
        ),
    ];

    for (entry, kind_tag, next) in cases {
        let bytes = entry.encode();
        let offset = find_kind_offset(&bytes);
        assert_eq!(bytes[offset], kind_tag, "{} kind tag", entry.body.kind());
        assert_eq!(entry.body.tag(), kind_tag);
        // The byte after the kind is the body's own first discriminant (or, for
        // rotation, the first byte of the commit reference).
        assert_eq!(bytes[offset + 1], next, "{} body", entry.body.kind());
    }
}

// ── the round trip through a governance entry ──────────────────────────

#[test]
fn read_refuses_what_admit_refuses_so_neither_check_can_be_skipped() {
    // The point of routing everything through one function: a client that
    // reaches a usable ChannelEntry has necessarily passed both checks. Decoding
    // bytes directly is still possible, but yields only a value and says so in
    // its name.
    use intranet_governance::EntryBody;

    let entry = definition(None);

    // A hand-built entry declaring a capability its kind does not need — the
    // exact case the protocol cannot catch, since it verified only that the
    // author holds what was declared.
    let forged = EntryBody::AppEntry {
        namespace: CHAT_NAMESPACE.to_owned(),
        kind: "channel-definition".to_owned(),
        required: cap("chat:post:*"),
        payload: entry.encode(),
    };
    assert!(matches!(
        ChannelEntry::read(&forged, NetworkProfile::Server, None),
        Err(ChannelRefusal::WrongCapability { .. })
    ));

    // And a body that is not an application entry at all.
    assert_eq!(
        ChannelEntry::read(
            &EntryBody::AppEntry {
                namespace: "calendar".to_owned(),
                kind: "event".to_owned(),
                required: cap("calendar:create:*"),
                payload: Vec::new(),
            },
            NetworkProfile::Server,
            None
        ),
        Err(ChannelRefusal::ForeignNamespace("calendar".to_owned())),
        "another application's entries are not this one's to read"
    );

    // And a log entry that is not an application entry at all — a client
    // replaying a whole log walks past these constantly.
    assert_eq!(
        ChannelEntry::read(
            &EntryBody::ContentTypePolicy {
                allowlist: Default::default(),
            },
            NetworkProfile::Server,
            None
        ),
        Err(ChannelRefusal::NotAnAppEntry)
    );
}

#[test]
fn a_truncated_payload_is_refused_rather_than_half_applied() {
    use intranet_governance::EntryBody;

    let entry = definition(None);
    let mut payload = entry.encode();
    payload.truncate(payload.len() - 4);

    assert!(matches!(
        ChannelEntry::read(
            &EntryBody::AppEntry {
                namespace: CHAT_NAMESPACE.to_owned(),
                kind: "channel-definition".to_owned(),
                required: entry.required(),
                payload,
            },
            NetworkProfile::Server,
            None
        ),
        Err(ChannelRefusal::Malformed(_))
    ));
}
