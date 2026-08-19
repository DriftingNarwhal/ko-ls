//! P0 criterion 2: appending a message must transfer only the new tail chunks.
//!
//! `design/07` §3 exists to falsify the assumption the whole design rests on —
//! that an author's channel log can be an append-grown, segment-chained object
//! on this storage layer at chat message rates. These tests assert on **bytes
//! actually produced**, never on the design being right.

use intranet_crypto::Timestamp;
use intranet_governance::{
    Capability, ContentType, EntryBody, GovernanceState, GroupId, LogEntry, MembershipAction,
    NetworkPolicy,
};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use intranet_storage::{ChunkSpec, Dek};
use kols_core::*;

fn network() -> NetworkId {
    NetworkId::from_bytes([7u8; 32])
}

fn identity(byte: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([byte; 32])
        .identity_for(&network())
        .expect("derives")
}

/// A network where `everyone` may read and publish chat logs.
///
/// `chat-log` has to be on the allowlist *and* granted, which is Core §2.8's two
/// independent gates — a type being permitted on a network does not by itself
/// let anyone publish it.
fn chain(founder: &PerNetworkIdentity, members: &[&PerNetworkIdentity]) -> Vec<LogEntry> {
    let mut policy = NetworkPolicy::conservative_default();
    policy
        .content_type_allowlist
        .insert(ContentType::new(CHAT_LOG_CONTENT_TYPE));

    let mut chain = vec![LogEntry::create(
        founder,
        None,
        Timestamp::from_millis(0),
        EntryBody::Genesis {
            network: network(),
            policy,
            everyone_capabilities: [
                Capability::ReadContent,
                Capability::publish(CHAT_LOG_CONTENT_TYPE),
            ]
            .into_iter()
            .collect(),
        },
    )];
    for (i, member) in members.iter().enumerate() {
        let parent = chain.last().expect("genesis").hash();
        chain.push(LogEntry::create(
            founder,
            Some(parent),
            Timestamp::from_millis(10 + i as i64),
            EntryBody::MembershipChange {
                group: GroupId::everyone(),
                identity: member.id(),
                action: MembershipAction::Add { via_invite: None },
            },
        ));
    }
    chain
}

fn state(founder: &PerNetworkIdentity) -> GovernanceState {
    GovernanceState::replay(&chain(founder, &[])).expect("replays")
}

fn message(text: &str) -> RecordBody {
    RecordBody::Message {
        body: text.to_owned(),
        reply_to: None,
        attachments: Vec::new(),
    }
}

#[test]
fn appending_transfers_only_the_tail() {
    let author = identity(1);
    let state = state(&author);
    let channel = server_channel_id(&network(), &[9u8; 32]);
    let mut log = AuthorLog::open(
        &author,
        channel,
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::from_target(16 * 1024),
    );

    // Grow the segment well past one chunk, so "only the tail" is a claim with
    // something to prove. Messages are realistic chat length, not padding.
    let mut published = Vec::new();
    for i in 0..600u32 {
        let record = Record::create(
            &author,
            channel,
            Hlc::new(1_700_000_000_000, i),
            message(&format!(
                "message {i} — realistic chat length, a sentence or so of text."
            )),
        );
        published.push(log.append(&author, record, &state).expect("appends"));
    }

    let last = published.last().expect("published");
    let total = last.total_bytes();
    let moved = last.new_bytes();

    // Printed so the measurement is visible in CI output rather than only
    // asserted — the numbers are what tune segment thresholds later.
    println!(
        "append moved {moved} of {total} bytes across {} new chunk(s) of {}",
        last.new_chunks.len(),
        last.object.manifest.chunks.len()
    );

    assert!(
        total > 32 * 1024,
        "segment must span several chunks for this to prove anything, was {total} bytes"
    );
    // The property: a reader holding the previous version fetches the tail, not
    // the segment. If this fails, content-defined chunking is not behaving as
    // Storage §1.2 promises and the segment model needs rethinking.
    assert!(
        moved < total / 4,
        "appending moved {moved} of {total} bytes — delta-fetch is not working"
    );
    assert!(
        last.new_chunks.len() <= 2,
        "appending produced {} new chunks, expected at most 2",
        last.new_chunks.len()
    );
}

#[test]
fn unchanged_chunks_keep_their_identifiers() {
    let author = identity(1);
    let state = state(&author);
    let channel = server_channel_id(&network(), &[9u8; 32]);
    let mut log = AuthorLog::open(
        &author,
        channel,
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::from_target(16 * 1024),
    );

    let mut before = Vec::new();
    for i in 0..400u32 {
        let record = Record::create(
            &author,
            channel,
            Hlc::new(1_700_000_000_000, i),
            message(&format!("message {i} with enough text to move the chunker along"),),
        );
        before = log
            .append(&author, record, &state)
            .expect("appends")
            .object
            .manifest
            .chunks
            .clone();
    }

    let after = log
        .append(
            &author,
            Record::create(
                &author,
                channel,
                Hlc::new(1_700_000_000_000, 400),
                message("one more"),
            ),
            &state,
        )
        .expect("appends")
        .object
        .manifest
        .chunks
        .clone();

    // Every chunk but the last few must be *identical*, not merely equivalent —
    // this is the deterministic-encryption requirement (Storage §1.2) doing its
    // job. Randomised nonces would give every chunk a new CID here.
    let shared = before.iter().filter(|cid| after.contains(cid)).count();
    println!("{shared} of {} chunks survived the append", before.len());
    assert!(
        shared >= before.len() - 1,
        "only {shared} of {} chunks survived the append",
        before.len()
    );
}

#[test]
fn a_sealed_segment_chains_to_its_successor() {
    let author = identity(1);
    let state = state(&author);
    let channel = server_channel_id(&network(), &[9u8; 32]);
    let mut log = AuthorLog::open(
        &author,
        channel,
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::default(),
    );

    let first = log
        .append(
            &author,
            Record::create(&author, channel, Hlc::new(1, 0), message("in segment 0")),
            &state,
        )
        .expect("appends");
    let sealed_cid = first.object.manifest_cid();

    log.seal(sealed_cid);
    assert_eq!(log.segment().sequence, 1);
    assert_eq!(log.segment().previous, Some(sealed_cid));

    let second = log
        .append(
            &author,
            Record::create(&author, channel, Hlc::new(2, 0), message("in segment 1")),
            &state,
        )
        .expect("appends");

    // Pointer versions advance monotonically across the seal, since the pointer
    // addresses the log rather than any one segment.
    assert!(second.pointer.version > first.pointer.version);
    second.pointer.verify().expect("pointer verifies");
}

#[test]
fn a_stalled_clock_is_refused_rather_than_published() {
    let author = identity(1);
    let state = state(&author);
    let channel = server_channel_id(&network(), &[9u8; 32]);
    let mut log = AuthorLog::open(
        &author,
        channel,
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::default(),
    );

    log.append(
        &author,
        Record::create(&author, channel, Hlc::new(5, 0), message("first")),
        &state,
    )
    .expect("appends");

    // Readers refuse a non-increasing reading (design/08 §4.1), so the writer
    // must refuse it too — otherwise an author publishes history nobody accepts
    // and the failure surfaces as unexplained non-propagation.
    let stalled = Record::create(&author, channel, Hlc::new(5, 0), message("same reading"));
    assert!(matches!(
        log.append(&author, stalled, &state),
        Err(CoreError::NonMonotonicClock { .. })
    ));
}

#[test]
fn an_author_without_the_capability_cannot_publish() {
    let author = identity(1);
    let state = state(&author);

    // A non-member: never added to `everyone`, so it holds no capability at all.
    let stranger = identity(2);
    let channel = server_channel_id(&network(), &[9u8; 32]);
    let mut stranger_log = AuthorLog::open(
        &stranger,
        channel,
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::default(),
    );

    let record = Record::create(&stranger, channel, Hlc::new(1, 0), message("hello"));
    assert!(matches!(
        stranger_log.append(&stranger, record, &state),
        Err(CoreError::Storage(_))
    ));
}
