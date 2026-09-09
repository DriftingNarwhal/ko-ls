//! A page boundary must lose nothing and repeat nothing.
//!
//! **The bug this exists for is silent, and that is what makes it worth a file
//! of its own.** Records merge by reading and *then* by record id, because two
//! records can carry the same reading: the counter is monotonic per author and
//! device (spec 07 §2.6), so two members writing in the same millisecond collide
//! legitimately and neither is wrong.
//!
//! A boundary expressed as a reading alone cannot separate such a pair. Asking
//! for everything before it excludes **both** — the one already drawn and the
//! one never drawn — so a message disappears from the channel, nothing on screen
//! reveals it, and every layer involved behaves exactly as written. `design/09`
//! §4.4 argues it; [`kols_core::Cursor`] is the fix; this is the proof.

use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use kols_core::{
    ChannelId, Cursor, Hlc, MessageId, Record, RecordBody, server_channel_id,
};
use kols_store::{Projection, Verdict};

const NETWORK: NetworkId = NetworkId::from_bytes([7u8; 32]);

fn identity(n: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([n; 32]).identity_for(&NETWORK).unwrap()
}

fn channel() -> ChannelId {
    server_channel_id(&NETWORK, &[4u8; 32])
}

fn message(author: &PerNetworkIdentity, hlc: Hlc, body: &str) -> Record {
    Record::create(
        author,
        channel(),
        hlc,
        RecordBody::Message {
            body: body.to_owned(),
            reply_to: None,
            attachments: Vec::new(),
        },
    )
}

/// A projection holding `records`.
fn holding(records: &[Record]) -> Projection {
    let (projection, _) = Projection::in_memory().expect("opens");
    for record in records {
        projection.insert(record, Verdict::Admitted).expect("inserts");
    }
    projection
}

/// Every record, in the order the merge puts them.
fn merged(records: &[Record]) -> Vec<MessageId> {
    let mut ordered: Vec<&Record> = records.iter().collect();
    ordered.sort_by_key(|record| record.cursor());
    ordered.iter().map(|record| record.id()).collect()
}

/// Walks a whole channel `size` at a time, exactly as a reader scrolling does:
/// each page's oldest record is the boundary the next request names.
fn by_pages(projection: &Projection, records: &[Record], size: usize) -> Vec<MessageId> {
    let channel = channel();
    let at: std::collections::HashMap<MessageId, Cursor> =
        records.iter().map(|r| (r.id(), r.cursor())).collect();

    let mut seen: Vec<MessageId> = Vec::new();
    let mut before: Option<Cursor> = None;
    loop {
        let mut page = projection.before(&channel, before, size).expect("pages");
        let Some(oldest) = page.first().copied() else {
            break;
        };
        before = Some(at[&oldest]);
        page.append(&mut seen);
        seen = page;
    }
    seen
}

/// Two authors, one millisecond, and both of them post.
///
/// Not contrived: a busy channel produces this constantly, and the counter
/// cannot prevent it because it is per author.
fn tied() -> Vec<Record> {
    let a = identity(1);
    let b = identity(2);
    let mut records = vec![
        message(&a, Hlc::new(1_000, 0), "a first"),
        message(&b, Hlc::new(2_000, 0), "b tied"),
        message(&a, Hlc::new(2_000, 0), "a tied"),
        message(&b, Hlc::new(3_000, 0), "b last"),
    ];
    records.sort_by_key(kols_core::Record::cursor);
    records
}

#[test]
fn two_records_sharing_a_reading_are_a_real_case() {
    // The premise the rest of this file rests on. If this ever stops being true
    // the tests below would pass for the wrong reason — they would be walking a
    // channel with no boundary worth testing.
    let records = tied();
    let readings: std::collections::BTreeSet<_> = records.iter().map(|r| r.hlc).collect();
    assert!(
        readings.len() < records.len(),
        "these records were built to share a reading and no longer do"
    );
}

#[test]
fn paging_through_a_channel_loses_nothing_and_repeats_nothing() {
    let records = tied();
    let projection = holding(&records);

    // Every page size that can land *on* the tie, and one that cannot. A size of
    // two is the case the old boundary got wrong: the first page ends between
    // the two records that share a reading.
    for size in [1, 2, 3, 4, 10] {
        let walked = by_pages(&projection, &records, size);
        assert_eq!(
            walked,
            merged(&records),
            "walking {} records {size} at a time did not reproduce the channel",
            records.len()
        );
    }
}

#[test]
fn a_boundary_between_two_tied_records_keeps_the_undrawn_one() {
    // The failure stated directly rather than as a property, because it is the
    // one somebody would have had to notice by hand: a page ends on the newer of
    // two records sharing a reading, and the older one is what the next page
    // must start with.
    let records = tied();
    let projection = holding(&records);

    let first = projection.before(&channel(), None, 2).expect("pages");
    let boundary = records
        .iter()
        .find(|r| r.id() == first[0])
        .expect("the page names records this test made")
        .cursor();

    let next = projection
        .before(&channel(), Some(boundary), 2)
        .expect("pages");

    let tie = records
        .iter()
        .filter(|r| r.hlc == Hlc::new(2_000, 0))
        .map(kols_core::Record::id)
        .collect::<Vec<_>>();
    assert_eq!(tie.len(), 2, "the fixture holds a tie");

    let drawn: std::collections::BTreeSet<_> =
        first.iter().chain(next.iter()).copied().collect();
    for id in tie {
        assert!(
            drawn.contains(&id),
            "a record sharing its reading with the boundary was never drawn"
        );
    }
}

#[test]
fn walking_forward_is_the_exact_complement_of_walking_back() {
    // `after` is what re-reads a loaded range and what reaches forward from an
    // anchor, so it has to agree with `before` about where a position sits —
    // including on a tie, where an off-by-one would either duplicate the
    // boundary record or drop it.
    let records = tied();
    let projection = holding(&records);
    let order = merged(&records);

    for (n, id) in order.iter().enumerate() {
        let at = records
            .iter()
            .find(|r| r.id() == *id)
            .expect("known record")
            .cursor();

        let back = projection.before(&channel(), Some(at), usize::MAX).expect("pages");
        let forward = projection.after(&channel(), Some(at), usize::MAX).expect("pages");

        assert_eq!(back, order[..n], "everything before position {n}");
        assert_eq!(forward, order[n + 1..], "everything after position {n}");
        assert!(
            !back.contains(id) && !forward.contains(id),
            "a boundary record belongs to neither side, or it would be drawn twice"
        );
    }
}

#[test]
fn a_range_includes_both_of_its_ends() {
    // `between` is inclusive because a range is named by records that are *in*
    // it — the interface hands back the ends it drew. Exclusive ends would shed
    // one message from each side of the range on every tick.
    let records = tied();
    let projection = holding(&records);
    let order = merged(&records);

    let from = records.iter().find(|r| r.id() == order[1]).unwrap().cursor();
    let to = records.iter().find(|r| r.id() == order[2]).unwrap().cursor();

    let range = projection.between(&channel(), from, Some(to), usize::MAX).expect("reads");
    assert_eq!(range, vec![order[1], order[2]]);
}

#[test]
fn a_live_range_runs_to_the_tail() {
    // What makes a range *live*: no newer end, so anything that arrives after it
    // was drawn is inside it. This is the property a tail-only re-read would
    // have, and the next test is the one it would not.
    let records = tied();
    let projection = holding(&records);
    let order = merged(&records);
    let from = records.iter().find(|r| r.id() == order[0]).unwrap().cursor();

    assert_eq!(
        projection.between(&channel(), from, None, usize::MAX).expect("reads"),
        order
    );
}

#[test]
fn backfill_landing_inside_a_range_is_inside_it() {
    // **The decisive one.** A record recovered from a stale author's chain sorts
    // where its author's clock puts it, which is in the *past* — frequently
    // nowhere near the bottom. A re-read that asked only for the newest page
    // would never show it, and the message would sit in the store, correctly
    // ordered and permanently invisible.
    let records = tied();
    let projection = holding(&records);
    let order = merged(&records);
    let from = records.iter().find(|r| r.id() == order[0]).unwrap().cursor();

    let late = message(&identity(3), Hlc::new(1_500, 0), "recovered from an old chain");
    projection.insert(&late, Verdict::Admitted).expect("inserts");

    let range = projection.between(&channel(), from, None, usize::MAX).expect("reads");
    assert!(
        range.contains(&late.id()),
        "a record backfilled into the middle of a loaded range has to be in it"
    );
    assert_eq!(range.len(), order.len() + 1);
    // And in its place, not appended.
    assert_ne!(range.last(), Some(&late.id()));
}

#[test]
fn the_author_count_is_the_channels_and_not_the_pages() {
    // Whole-channel deliberately: a number that changed as somebody scrolled
    // would be worse than the query it saved (`design/09` §4.4).
    let records = tied();
    let projection = holding(&records);
    assert_eq!(projection.authors(&channel()).expect("counts"), 2);

    let page = projection.before(&channel(), None, 1).expect("pages");
    assert_eq!(page.len(), 1, "a page of one, and still two authors");
    assert_eq!(projection.authors(&channel()).expect("counts"), 2);
}
