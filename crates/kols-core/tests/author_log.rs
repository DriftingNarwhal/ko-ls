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
use intranet_storage::{ChunkSpec, Dek, MutablePointer};
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

    log.seal(sealed_cid, Dek::from_bytes([6u8; 32]));
    assert_eq!(log.segment().sequence, 1);
    assert_eq!(log.segment().previous, Some(sealed_cid));

    let second = log
        .append(
            &author,
            Record::create(&author, channel, Hlc::new(2, 0), message("in segment 1")),
            &state,
        )
        .expect("appends");
    second.pointer.verify().expect("pointer verifies");

    // **A segment addresses itself, not the log.** Each one lives under its own
    // pointer, so the second starts a fresh version line rather than continuing
    // the first's — and that is the point rather than an accident. A pointer
    // commits to one DEK for its whole life (Storage §1.2), so a log that kept
    // one pointer across every segment it ever wrote would have one key for its
    // whole history, and `design/01` §8 could only ever forget all of it at once.
    assert_ne!(second.pointer.pointer_id, first.pointer.pointer_id);
    assert_eq!(
        second.pointer.pointer_id,
        author_segment_pointer(&channel, &author.id(), 1)
    );
    assert_ne!(second.pointer.dek_commitment, first.pointer.dek_commitment);
}

#[test]
fn one_segments_key_does_not_open_another() {
    // The property per-segment keys exist for. Without it `design/01` §8 can
    // only forget a whole log: a key that opens the newest message opens the
    // oldest, so there is no "old history" to drop separately from the rest.
    let author = identity(1);
    let state = state(&author);
    let channel = server_channel_id(&network(), &[9u8; 32]);
    let written = Dek::from_bytes([5u8; 32]);
    let next = Dek::from_bytes([6u8; 32]);
    let mut log = AuthorLog::open(&author, channel, written, ChunkSpec::default());

    let first = log
        .append(
            &author,
            Record::create(&author, channel, Hlc::new(1, 0), message("in segment 0")),
            &state,
        )
        .expect("appends");
    let sealed = first.object.clone();

    log.seal(first.object.manifest_cid(), next.clone());
    let second = log
        .append(
            &author,
            Record::create(&author, channel, Hlc::new(2, 0), message("in segment 1")),
            &state,
        )
        .expect("appends");

    // Each opens its own.
    let sealed_blobs: std::collections::BTreeMap<_, _> = sealed.chunks.iter().cloned().collect();
    let open_blobs: std::collections::BTreeMap<_, _> =
        second.object.chunks.iter().cloned().collect();
    intranet_storage::decode(&sealed.manifest, &sealed_blobs, &Dek::from_bytes([5u8; 32]))
        .expect("the sealed segment opens under the key it was written with");
    intranet_storage::decode(&second.object.manifest, &open_blobs, &next)
        .expect("the open segment opens under its own key");

    // And neither opens the other. This is what makes forgetting one segment
    // leave the rest readable, rather than being all-or-nothing.
    assert!(
        intranet_storage::decode(&sealed.manifest, &sealed_blobs, &next).is_err(),
        "the new segment's key must not open history sealed before it"
    );
    assert!(
        intranet_storage::decode(
            &second.object.manifest,
            &open_blobs,
            &Dek::from_bytes([5u8; 32])
        )
        .is_err(),
        "a retired segment's key must not open what came after it"
    );
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

// ── criterion 5: version collision loses no records ────────────────────

/// Two writers on one identity, publishing concurrently.
///
/// The multi-device case (`design/05` §6): both hold the same identity and the
/// same derived pointer, neither has seen the other, and both publish at the
/// same version. The protocol settles which record is canonical; what the losing
/// side's *records* mean is left to us, and the answer must not be "they are
/// gone" — their author published validly and has no way to know they lost.
#[test]
fn a_version_collision_loses_no_records() {
    let author = identity(1);
    let state = state(&author);
    let channel = server_channel_id(&network(), &[9u8; 32]);
    let dek = || Dek::from_bytes([5u8; 32]);

    let mut left = AuthorLog::open(&author, channel, dek(), ChunkSpec::default());
    let mut right = AuthorLog::open(&author, channel, dek(), ChunkSpec::default());

    let left_published = left
        .append(
            &author,
            Record::create(&author, channel, Hlc::new(100, 0), message("from the laptop")),
            &state,
        )
        .expect("appends");
    let right_published = right
        .append(
            &author,
            Record::create(&author, channel, Hlc::new(101, 0), message("from the phone")),
            &state,
        )
        .expect("appends");

    // Same pointer, same version, different content: a genuine collision.
    assert_eq!(left_published.pointer.pointer_id, right_published.pointer.pointer_id);
    assert_eq!(left_published.pointer.version, right_published.pointer.version);

    let winner = MutablePointer::resolve(&left_published.pointer, &right_published.pointer);
    let winner_is_left = winner == &left_published.pointer;

    // Both sides compute the same winner, from the records alone.
    assert_eq!(
        MutablePointer::resolve(&right_published.pointer, &left_published.pointer),
        winner
    );

    let (winner_pointer, winner_segment, loser, loser_published) = if winner_is_left {
        (
            left_published.pointer.clone(),
            left.segment().clone(),
            &mut right,
            &right_published,
        )
    } else {
        (
            right_published.pointer.clone(),
            right.segment().clone(),
            &mut left,
            &left_published,
        )
    };

    let rebased = loser
        .rebase(&author, &winner_pointer, &winner_segment, &state)
        .expect("rebases");

    // The union, at the next version, in reading order.
    assert_eq!(rebased.pointer.version, winner_pointer.version + 1);
    assert!(rebased.pointer.supersedes(&winner_pointer));
    assert_eq!(loser.segment().records.len(), 2);
    assert!(loser.segment().ordering_is_valid());

    let bodies: Vec<_> = loser
        .segment()
        .records
        .iter()
        .map(|record| match &record.body {
            RecordBody::Message { body, .. } => body.clone(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(bodies, vec!["from the laptop", "from the phone"]);

    let _ = loser_published;
}

/// What a rebase costs, which depends on where the loser's records sort.
///
/// A segment is prefix-stable: chunks before the first change survive. So a
/// loser whose records sort *after* the winner's re-uploads only the tail, while
/// one whose records interleave rewrites from the first interleaved point on.
/// Neither is a correctness problem — no records are lost either way — but it is
/// worth knowing that a device coming back online after a long absence is the
/// cheap case, and two devices writing simultaneously is the expensive one.
#[test]
fn a_rebase_reuses_the_canonical_chunks_when_records_sort_after() {
    let author = identity(1);
    let state = state(&author);
    let channel = server_channel_id(&network(), &[9u8; 32]);

    // The canonical side, long enough to span several chunks.
    let mut canonical = AuthorLog::open(
        &author,
        channel,
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::from_target(16 * 1024),
    );
    for i in 0..600u32 {
        canonical
            .append(
                &author,
                Record::create(
                    &author,
                    channel,
                    Hlc::new(1_700_000_000_000, i),
                    message(&format!("canonical message {i} with a sentence of text")),
                ),
                &state,
            )
            .expect("appends");
    }
    let canonical_pointer = canonical.pointer().expect("published").clone();
    let canonical_segment = canonical.segment().clone();

    // The losing side, whose one record sorts after everything above.
    let mut loser = AuthorLog::open(
        &author,
        channel,
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::from_target(16 * 1024),
    );
    loser
        .append(
            &author,
            Record::create(
                &author,
                channel,
                Hlc::new(1_700_000_001_000, 0),
                message("arrived late"),
            ),
            &state,
        )
        .expect("appends");

    let rebased = loser
        .rebase(&author, &canonical_pointer, &canonical_segment, &state)
        .expect("rebases");

    println!(
        "rebase moved {} of {} bytes",
        rebased.new_bytes(),
        rebased.total_bytes()
    );
    assert_eq!(loser.segment().records.len(), 601);
    assert!(
        rebased.new_bytes() < rebased.total_bytes() / 4,
        "rebase moved {} of {} bytes — canonical chunks were not reused",
        rebased.new_bytes(),
        rebased.total_bytes()
    );
}

#[test]
fn a_rebase_is_idempotent_when_there_is_nothing_to_merge() {
    let author = identity(1);
    let state = state(&author);
    let channel = server_channel_id(&network(), &[9u8; 32]);

    let mut log = AuthorLog::open(&author, channel, Dek::from_bytes([5u8; 32]), ChunkSpec::default());
    let published = log
        .append(
            &author,
            Record::create(&author, channel, Hlc::new(1, 0), message("only")),
            &state,
        )
        .expect("appends");

    // Rebasing onto its own state must add nothing, so a node that reconciles
    // twice does not grow its log.
    let segment = log.segment().clone();
    let rebased = log
        .rebase(&author, &published.pointer, &segment, &state)
        .expect("rebases");

    assert_eq!(log.segment().records.len(), 1);
    assert_eq!(rebased.new_bytes(), 0, "an empty rebase moved bytes");
}
