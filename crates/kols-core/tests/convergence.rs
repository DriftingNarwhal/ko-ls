//! P0 criteria 1, 3 and 4 — `design/07` §3, and the claim `design/05` §8 calls
//! the correctness claim of the whole design.
//!
//! Two nodes are simulated by two independently-built views over the same
//! records. That is the honest scope of these tests: they prove the *merge*
//! converges under any arrival order, which is where the claim actually lives.
//! What they do not exercise is the wire — real chunk transfer, DHT lookup and
//! NAT traversal are the protocol's own harness's job, and `kols-net` will need
//! its own tests before P0 can be called closed.

use intranet_crypto::{Hash, Timestamp};
use intranet_governance::{
    Capability, ContentType, EntryBody, GovernanceState, GroupId, LogEntry, MembershipAction,
    NetworkPolicy,
};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use kols_core::*;

/// Limits that refuse nothing, for tests about something other than §4.3.
///
/// Spelled out at each call rather than defaulted, because `ReaderLimits` has
/// no `Default` on purpose — a reader that silently enforced nothing is the
/// state this parameter exists to end.
const LAX: kols_core::ReaderLimits = kols_core::ReaderLimits::unbounded();

fn network() -> NetworkId {
    NetworkId::from_bytes([7u8; 32])
}

fn identity(byte: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([byte; 32])
        .identity_for(&network())
        .expect("derives")
}

fn channel() -> ChannelId {
    server_channel_id(&network(), &[9u8; 32])
}

fn placement() -> Placement {
    Placement {
        channel: channel(),
        category: None,
    }
}

/// A network where `everyone` may post, and identity 9 may moderate.
fn state(members: &[&PerNetworkIdentity], moderators: &[&PerNetworkIdentity]) -> GovernanceState {
    let founder = identity(1);
    let mut policy = NetworkPolicy::conservative_default();
    policy
        .content_type_allowlist
        .insert(ContentType::new(CHAT_LOG_CONTENT_TYPE));
    // Chat capabilities must be registered before they can be granted — an
    // unregistered extension name is refused, not assumed ordinary.
    policy
        .extension_capabilities
        .extend(kols_core::capabilities::namespaces());

    let mut chain = vec![LogEntry::create(
        &founder,
        None,
        Timestamp::from_millis(0),
        EntryBody::Genesis {
            network: network(),
            policy,
            everyone_capabilities: [
                Capability::ReadContent,
                Capability::publish(CHAT_LOG_CONTENT_TYPE),
                Capability::extension("chat:post:*"),
            ]
            .into_iter()
            .collect(),
        },
    )];

    let mut time = 10;
    let mut push = |chain: &mut Vec<LogEntry>, body| {
        let parent = chain.last().expect("genesis").hash();
        chain.push(LogEntry::create(
            &founder,
            Some(parent),
            Timestamp::from_millis(time),
            body,
        ));
        time += 1;
    };

    for member in members {
        push(
            &mut chain,
            EntryBody::MembershipChange {
                group: GroupId::everyone(),
                identity: member.id(),
                action: MembershipAction::Add { via_invite: None },
            },
        );
    }
    for moderator in moderators {
        push(
            &mut chain,
            EntryBody::DefineGroup {
                group: GroupId::new("moderators"),
                capabilities: intranet_governance::CapabilitySet::explicit([
                    Capability::ModerateContent,
                ]),
            },
        );
        push(
            &mut chain,
            EntryBody::MembershipChange {
                group: GroupId::new("moderators"),
                identity: moderator.id(),
                action: MembershipAction::Add { via_invite: None },
            },
        );
    }

    GovernanceState::replay(&chain).expect("replays")
}

fn message(text: &str) -> RecordBody {
    RecordBody::Message {
        body: text.to_owned(),
        reply_to: None,
        attachments: Vec::new(),
    }
}

/// Deterministic shuffle, so a failure is reproducible.
///
/// A random one would make this test tell a different story every run, which is
/// the opposite of what a convergence test is for.
fn shuffled<T>(mut items: Vec<T>, seed: u32) -> Vec<T> {
    let mut x = seed | 1;
    for i in (1..items.len()).rev() {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        items.swap(i, (x >> 16) as usize % (i + 1));
    }
    items
}

/// Two authors, a hundred interleaved messages.
fn conversation(a: &PerNetworkIdentity, b: &PerNetworkIdentity) -> Vec<Record> {
    (0..100u32)
        .map(|i| {
            let author = if i % 2 == 0 { a } else { b };
            Record::create(
                author,
                channel(),
                Hlc::new(1_700_000_000_000 + i64::from(i), 0),
                message(&format!("message {i}")),
            )
        })
        .collect()
}

// ── criterion 1: identical order on both nodes ─────────────────────────

#[test]
fn arrival_order_does_not_change_the_rendering() {
    let (a, b) = (identity(2), identity(3));
    let state = state(&[&a, &b], &[]);
    let authority = StateAuthority { state: &state };
    let records = conversation(&a, &b);

    let mut baseline = ChannelView::new(placement());
    baseline.admit(records.clone(), &authority, &LAX);
    let expected = baseline.render(&LAX, 0);
    assert_eq!(expected.len(), 100);

    // Every permutation we can afford to try must agree, including reversal —
    // the worst case for anything that accidentally depends on insertion order.
    for seed in 1..40u32 {
        let mut view = ChannelView::new(placement());
        view.admit(shuffled(records.clone(), seed), &authority, &LAX);
        assert_eq!(view.render(&LAX, 0), expected, "seed {seed} rendered differently");
    }

    let mut reversed = ChannelView::new(placement());
    reversed.admit(records.iter().rev().cloned().collect::<Vec<_>>(), &authority, &LAX);
    assert_eq!(reversed.render(&LAX, 0), expected);
}

#[test]
fn duplicate_delivery_is_idempotent() {
    let (a, b) = (identity(2), identity(3));
    let state = state(&[&a, &b], &[]);
    let authority = StateAuthority { state: &state };
    let records = conversation(&a, &b);

    let mut once = ChannelView::new(placement());
    once.admit(records.clone(), &authority, &LAX);

    // The live and durable paths overlap by design, so the same record arrives
    // twice as a matter of course rather than as an error case.
    let mut twice = ChannelView::new(placement());
    twice.admit(records.clone(), &authority, &LAX);
    twice.admit(shuffled(records, 7), &authority, &LAX);

    assert_eq!(twice.len(), once.len());
    assert_eq!(twice.render(&LAX, 0), once.render(&LAX, 0));
}

// ── criterion 3: partition, both post, heal, converge ──────────────────

#[test]
fn a_partition_heals_to_one_history() {
    let (a, b) = (identity(2), identity(3));
    let state = state(&[&a, &b], &[]);
    let authority = StateAuthority { state: &state };

    // Both sides keep writing while split, at overlapping clock readings —
    // which is the case that matters, since disjoint readings would sort
    // themselves.
    let side_a: Vec<Record> = (0..25u32)
        .map(|i| {
            Record::create(
                &a,
                channel(),
                Hlc::new(1_700_000_000_000 + i64::from(i), 0),
                message(&format!("a{i}")),
            )
        })
        .collect();
    let side_b: Vec<Record> = (0..25u32)
        .map(|i| {
            Record::create(
                &b,
                channel(),
                Hlc::new(1_700_000_000_000 + i64::from(i), 0),
                message(&format!("b{i}")),
            )
        })
        .collect();

    let mut node_a = ChannelView::new(placement());
    node_a.admit(side_a.clone(), &authority, &LAX);
    let mut node_b = ChannelView::new(placement());
    node_b.admit(side_b.clone(), &authority, &LAX);
    assert_ne!(node_a.render(&LAX, 0), node_b.render(&LAX, 0), "the partition was not real");

    // Heal: each side learns the other's records, in different orders.
    node_a.admit(shuffled(side_b, 11), &authority, &LAX);
    node_b.admit(shuffled(side_a, 23), &authority, &LAX);

    let healed = node_a.render(&LAX, 0);
    assert_eq!(healed, node_b.render(&LAX, 0));
    assert_eq!(healed.len(), 50);

    // Concurrent records at identical readings are ordered by record hash — the
    // protocol's own tie-break, so two nodes never disagree about which came
    // first even though neither did.
    let ids: Vec<_> = healed.iter().map(|m| m.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_by_key(|id| (healed.iter().find(|m| m.id == *id).expect("present").hlc, *id));
    assert_eq!(ids, sorted);
}

// ── criterion 4: the reader refuses, not just the writer ───────────────

#[test]
fn a_non_member_is_refused_by_the_reader() {
    let member = identity(2);
    let stranger = identity(4);
    let state = state(&[&member], &[]);
    let authority = StateAuthority { state: &state };

    let mut view = ChannelView::new(placement());
    view.admit(
        vec![
            Record::create(&member, channel(), Hlc::new(1, 0), message("allowed")),
            Record::create(&stranger, channel(), Hlc::new(2, 0), message("refused")),
        ],
        &authority,
        &LAX,
    );

    assert_eq!(view.render(&LAX, 0).len(), 1);
    assert_eq!(view.rejected().len(), 1);
    assert_eq!(view.rejected()[0].1, Rejection::NotAMember);
}

#[test]
fn a_forged_signature_is_refused() {
    let member = identity(2);
    let state = state(&[&member], &[]);
    let authority = StateAuthority { state: &state };

    let mut forged = Record::create(&member, channel(), Hlc::new(1, 0), message("original"));
    forged.body = message("tampered");

    let mut view = ChannelView::new(placement());
    view.admit(vec![forged], &authority, &LAX);
    assert!(view.render(&LAX, 0).is_empty());
    assert_eq!(view.rejected()[0].1, Rejection::BadSignature);
}

#[test]
fn a_record_for_another_channel_is_refused() {
    let member = identity(2);
    let state = state(&[&member], &[]);
    let authority = StateAuthority { state: &state };
    let elsewhere = server_channel_id(&network(), &[1u8; 32]);

    let mut view = ChannelView::new(placement());
    view.admit(
        vec![Record::create(&member, elsewhere, Hlc::new(1, 0), message("hi"))],
        &authority,
        &LAX,
    );
    assert_eq!(view.rejected()[0].1, Rejection::WrongChannel);
}

// ── effects resolve deterministically ──────────────────────────────────

#[test]
fn edits_tombstones_reactions_and_redactions_are_order_independent() {
    let (author, other, moderator) = (identity(2), identity(3), identity(9));
    let state = state(&[&author, &other, &moderator], &[&moderator]);
    let authority = StateAuthority { state: &state };

    let first = Record::create(&author, channel(), Hlc::new(10, 0), message("first"));
    let second = Record::create(&author, channel(), Hlc::new(11, 0), message("second"));
    let target = first.id();

    let effects = vec![
        first.clone(),
        second.clone(),
        Record::create(
            &author,
            channel(),
            Hlc::new(12, 0),
            RecordBody::Edit {
                target,
                body: "first, revised".to_owned(),
            },
        ),
        Record::create(
            &other,
            channel(),
            Hlc::new(13, 0),
            RecordBody::Reaction {
                target,
                key: "👍".to_owned(),
                remove: false,
            },
        ),
        Record::create(
            &author,
            channel(),
            Hlc::new(14, 0),
            RecordBody::Reaction {
                target,
                key: "👍".to_owned(),
                remove: false,
            },
        ),
        // The moderator pins, not the author: pinning is `chat:moderate`
        // (`design/02` §2.2), and the reader enforces that now.
        Record::create(
            &moderator,
            channel(),
            Hlc::new(15, 0),
            RecordBody::Pin {
                target,
                remove: false,
            },
        ),
        Record::create(
            &moderator,
            channel(),
            Hlc::new(16, 0),
            RecordBody::Redaction {
                target: second.id(),
                governance_head: Hash::ZERO,
            },
        ),
    ];

    let mut expected: Option<Vec<RenderedMessage>> = None;
    for seed in 1..30u32 {
        let mut view = ChannelView::new(placement());
        view.admit(shuffled(effects.clone(), seed), &authority, &LAX);
        let rendered = view.render(&LAX, 0);
        match &expected {
            None => {
                let first_msg = &rendered[0];
                assert_eq!(first_msg.body, "first, revised");
                assert!(first_msg.edited);
                assert!(first_msg.pinned);
                assert_eq!(first_msg.reactions["👍"].len(), 2);
                assert!(rendered[1].redacted);
                assert!(!rendered[1].is_visible());
                expected = Some(rendered);
            }
            Some(expected) => assert_eq!(&rendered, expected, "seed {seed} diverged"),
        }
    }
}

#[test]
fn nobody_edits_or_withdraws_somebody_elses_message() {
    let (author, impostor) = (identity(2), identity(3));
    let state = state(&[&author, &impostor], &[]);
    let authority = StateAuthority { state: &state };

    let original = Record::create(&author, channel(), Hlc::new(10, 0), message("mine"));
    let target = original.id();

    let mut view = ChannelView::new(placement());
    view.admit(
        vec![
            original,
            Record::create(
                &impostor,
                channel(),
                Hlc::new(11, 0),
                RecordBody::Edit {
                    target,
                    body: "not yours to change".to_owned(),
                },
            ),
            Record::create(
                &impostor,
                channel(),
                Hlc::new(12, 0),
                RecordBody::Tombstone { target },
            ),
        ],
        &authority,
        &LAX,
    );

    let rendered = view.render(&LAX, 0);
    // The impostor's records are validly signed and they are a member, so they
    // are admitted — and then ignored, because authorship is checked where the
    // effect is applied. Both defences matter: one keeps the record set honest,
    // the other keeps the rendering honest.
    assert_eq!(rendered[0].body, "mine");
    assert!(!rendered[0].edited);
    assert!(rendered[0].is_visible());
}

#[test]
fn a_non_moderator_cannot_redact() {
    let (author, pretender) = (identity(2), identity(3));
    let state = state(&[&author, &pretender], &[]);
    let authority = StateAuthority { state: &state };

    let original = Record::create(&author, channel(), Hlc::new(10, 0), message("visible"));
    let target = original.id();

    let mut view = ChannelView::new(placement());
    view.admit(
        vec![
            original,
            Record::create(
                &pretender,
                channel(),
                Hlc::new(11, 0),
                RecordBody::Redaction {
                    target,
                    governance_head: Hash::ZERO,
                },
            ),
        ],
        &authority,
        &LAX,
    );

    assert!(view.render(&LAX, 0)[0].is_visible());
    assert_eq!(view.rejected()[0].1, Rejection::NotAModerator);
}

#[test]
fn a_non_moderator_cannot_pin() {
    // Found wiring `kols-api`'s gate to an executor. The boundary requires
    // `chat:moderate` to issue a pin, per `design/02` §2.2 — and this reader
    // admitted one under `chat:post`, so a modified client holding only posting
    // rights could pin and every conformant reader would honour it. A check the
    // writer makes and the reader does not is a check that does not exist.
    let (author, pretender) = (identity(2), identity(3));
    let state = state(&[&author, &pretender], &[]);
    let authority = StateAuthority { state: &state };

    let original = Record::create(&author, channel(), Hlc::new(10, 0), message("ordinary"));
    let target = original.id();

    let mut view = ChannelView::new(placement());
    view.admit(
        vec![
            original,
            Record::create(
                &pretender,
                channel(),
                Hlc::new(11, 0),
                RecordBody::Pin {
                    target,
                    remove: false,
                },
            ),
        ],
        &authority,
        &LAX,
    );

    assert!(!view.render(&LAX, 0)[0].pinned, "a pin from a non-moderator holds");
    assert_eq!(view.rejected()[0].1, Rejection::NotAModerator);
}

#[test]
fn a_moderator_can_pin() {
    let (author, moderator) = (identity(2), identity(3));
    let state = state(&[&author, &moderator], &[&moderator]);
    let authority = StateAuthority { state: &state };

    let original = Record::create(&author, channel(), Hlc::new(10, 0), message("worth keeping"));
    let target = original.id();

    let mut view = ChannelView::new(placement());
    view.admit(
        vec![
            original,
            Record::create(
                &moderator,
                channel(),
                Hlc::new(11, 0),
                RecordBody::Pin {
                    target,
                    remove: false,
                },
            ),
        ],
        &authority,
        &LAX,
    );

    assert!(view.rejected().is_empty(), "{:?}", view.rejected());
    assert!(view.render(&LAX, 0)[0].pinned);
}
