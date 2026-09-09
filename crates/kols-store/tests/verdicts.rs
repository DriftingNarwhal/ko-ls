//! The projection's verdicts must be the ones the whole-set pass reaches.
//!
//! **This is the claim the projection rests on.** `design/01` §10.4's rate pass
//! is a greedy fold over every record in merge order, and rendering a page
//! cannot run it — so the verdict is stored instead, decided one record at a
//! time from what is already stored. That is only sound if it reaches the same
//! answer, and "the same answer" has to be checked against the real pass rather
//! than against a second implementation of it.
//!
//! If these ever disagree, the visible symptom is a message that renders when
//! you scroll to it and vanishes when you load the channel whole — which is the
//! divergence `01` §10.1 makes these limits network policy to avoid, arriving
//! from inside one client.

use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use kols_core::{ChannelId, Hlc, ReaderLimits, Record, RecordBody, server_channel_id};
use kols_store::{Projection, Verdict};

const NETWORK: NetworkId = NetworkId::from_bytes([3u8; 32]);

fn identity(n: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([n; 32]).identity_for(&NETWORK).unwrap()
}

fn channel() -> ChannelId {
    server_channel_id(&NETWORK, &[9u8; 32])
}

fn message(author: &PerNetworkIdentity, channel: ChannelId, at: i64, n: usize) -> Record {
    Record::create(
        author,
        channel,
        Hlc::new(at, 0),
        RecordBody::Message {
            body: format!("message {n}"),
            reply_to: None,
            attachments: Vec::new(),
        },
    )
}

fn limits(rate: i64, slowmode: u32) -> ReaderLimits {
    ReaderLimits {
        message_rate_per_minute: rate,
        reaction_rate_per_minute: 60,
        message_max_bytes: 8 * 1024,
        max_future_skew_millis: 300_000,
        slowmode_seconds: slowmode,
    }
}

/// Folds `records` through the projection and returns what it decided.
fn folded(records: &[Record], limits: &ReaderLimits) -> Vec<Verdict> {
    let (projection, empty) = Projection::in_memory().expect("opens");
    assert!(empty, "a fresh projection holds nothing");
    let mut decided = Vec::new();
    for record in records {
        let verdict = kols_store::decide(&projection, record, limits).expect("decides");
        projection.insert(record, verdict).expect("inserts");
        decided.push(verdict);
    }
    decided
}

/// What `kols-core`'s pass makes of the same records, as a rendering would.
fn refused_by_the_real_pass(records: &[Record], limits: &ReaderLimits) -> Vec<bool> {
    // Far in the future, so nothing is *held* — held is a different verdict with
    // an expiring answer and is deliberately not the projection's business.
    let withheld = kols_core::withheld(records.iter(), limits, i64::MAX / 2);
    records
        .iter()
        .map(|record| withheld.refused.iter().any(|(id, _)| *id == record.id()))
        .collect()
}

fn agree(records: &[Record], limits: &ReaderLimits, why: &str) {
    let stored = folded(records, limits);
    let real = refused_by_the_real_pass(records, limits);
    let mismatch: Vec<_> = stored
        .iter()
        .zip(&real)
        .enumerate()
        .filter(|(_, (verdict, refused))| verdict.renders() == **refused)
        .map(|(at, (verdict, refused))| format!("#{at}: stored {verdict:?}, pass refused {refused}"))
        .collect();
    assert!(mismatch.is_empty(), "{why}: {mismatch:?}");
}

#[test]
fn a_burst_is_refused_in_the_same_places() {
    // Thirty a minute is the shipped ceiling and forty arrive inside one second,
    // so the tail of the burst is refused — and which records those are is what
    // has to match, not merely how many.
    let author = identity(1);
    let channel = channel();
    let records: Vec<_> = (0..40)
        .map(|n| message(&author, channel, 1_700_000_000_000 + n as i64, n))
        .collect();
    agree(&records, &limits(30, 0), "a burst inside one window");
}

#[test]
fn a_refused_record_does_not_cost_its_author_the_next_slot() {
    // `01` §10.4: a refused record does not itself occupy a slot, so one burst
    // costs an author exactly the records that exceeded the rate and does not
    // silence them afterwards. The window has to slide past the refusals.
    let author = identity(1);
    let channel = channel();
    let mut records: Vec<_> = (0..50)
        .map(|n| message(&author, channel, 1_700_000_000_000 + n as i64, n))
        .collect();
    // Then a long quiet, and one more well outside the window.
    records.push(message(&author, channel, 1_700_000_200_000, 99));
    agree(&records, &limits(30, 0), "a burst then a gap");
}

#[test]
fn the_window_slides_rather_than_counting_forever() {
    // Steady traffic that never exceeds the ceiling inside any one minute must
    // never be refused, however long it runs — the failure of a counter that
    // does not expire.
    let author = identity(1);
    let channel = channel();
    let records: Vec<_> = (0..120)
        .map(|n| message(&author, channel, 1_700_000_000_000 + (n as i64) * 5_000, n))
        .collect();
    let stored = folded(&records, &limits(30, 0));
    assert!(
        stored.iter().all(|verdict| verdict.renders()),
        "steady traffic inside the ceiling must all render"
    );
    agree(&records, &limits(30, 0), "steady traffic");
}

#[test]
fn two_authors_are_counted_apart() {
    // The ceiling is per author per channel. Interleaved bursts from two people
    // must not refuse each other, which a fold keyed on the channel alone would.
    let one = identity(1);
    let two = identity(2);
    let channel = channel();
    let mut records = Vec::new();
    for n in 0..40 {
        records.push(message(&one, channel, 1_700_000_000_000 + n as i64 * 2, n));
        records.push(message(&two, channel, 1_700_000_000_001 + n as i64 * 2, n));
    }
    records.sort_by_key(|record| (record.hlc, record.id()));
    agree(&records, &limits(30, 0), "two authors interleaved");
}

#[test]
fn slowmode_is_reached_at_the_same_records() {
    let author = identity(1);
    let channel = channel();
    let records: Vec<_> = (0..12)
        .map(|n| message(&author, channel, 1_700_000_000_000 + (n as i64) * 1_000, n))
        .collect();
    agree(&records, &limits(30, 5), "five-second slowmode");
}

#[test]
fn no_ceiling_refuses_nothing() {
    // Zero means *no limit* rather than a bound of zero — spec 07 §4.3's
    // sentinel, and the direction that refuses everything if read the other way.
    let author = identity(1);
    let channel = channel();
    let records: Vec<_> = (0..100)
        .map(|n| message(&author, channel, 1_700_000_000_000 + n as i64, n))
        .collect();
    let stored = folded(&records, &limits(0, 0));
    assert!(stored.iter().all(|verdict| verdict.renders()));
}
