//! The index of this member's own readings — `design/05` §5.
//!
//! **A cache is only as good as the promise that it answers what the thing it
//! caches would.** These assert that equivalence directly rather than asserting
//! that the cache exists, because the failure worth catching is not a missing
//! file — that path rebuilds — but a file that is present and subtly wrong. A
//! wrong `last` produces a reading that is not greater than one already
//! published, which readers refuse; a wrong count lets a member write records the
//! network then drops.

mod common;

use kols_api::Command;
use kols_node::executor::Executor;
use kols_node::readings::OwnReadings;
use kols_node::workspace::Workspace;

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-read-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a workspace");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A keyed network with one channel, and a few records in it.
fn posted(name: &str, messages: usize) -> (Dir, Executor, kols_core::ChannelId) {
    let dir = Dir::new(name);
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("cost", Vec::new()).expect("creates");
    let root = store.root().to_path_buf();
    drop(store);
    // Nothing can be written without an epoch key, and only a running node
    // creates the founder's group (Core §3.3).
    common::brief_run(root.clone());
    let executor = Executor::open(root).expect("opens");

    let channel = match executor
        .submit(Command::CreateChannel {
            name: "general".to_owned(),
            category: None,
            privacy: kols_core::Privacy::Public,
            topic: String::new(),
        })
        .expect("creates a channel")
    {
        kols_api::Outcome::ChannelCreated { channel, .. } => channel,
        other => panic!("unexpected outcome: {other:?}"),
    };

    for n in 0..messages {
        executor
            .submit(Command::SendMessage {
                channel,
                body: format!("message {n}"),
                reply_to: None,
                attachments: Vec::new(),
            })
            .expect("posts");
    }
    (dir, executor, channel)
}

#[test]
fn the_index_answers_exactly_what_a_full_scan_would() {
    // The claim the whole thing rests on. Not "the file exists" — a present and
    // subtly wrong file is the failure, and rebuilding covers the absent one.
    //
    // **The expectation is computed from the records here rather than through
    // `OwnReadings::of`, and the first version of this test did the latter.**
    // That compares the cache against a rebuild sharing the same folding code,
    // so it catches a stale cache and passes happily on a fold that is wrong in
    // both — which a probe demonstrated by breaking `note` and watching this
    // stay green.
    let (_dir, executor, channel) = posted("equivalent", 12);
    let store = executor.store();
    let author = store.identity().expect("an identity").id();
    let records = store.own_records(&channel, &author).expect("scans");
    assert!(records.len() >= 12, "the fixture wrote something to scan");

    let cached = store.own_readings(&channel).expect("reads the index");

    assert_eq!(
        cached.last,
        records.iter().map(|record| record.hlc).max(),
        "the newest reading must be the newest reading"
    );
    assert_eq!(
        cached.last_message,
        records
            .iter()
            .filter(|record| record.body.class() == kols_core::RecordClass::Message)
            .map(|record| record.hlc)
            .max(),
        "and the newest message reading, which is what slowmode asks"
    );

    let newest = cached.last.expect("wrote something");
    let since = newest.wall_millis - 60_000;
    assert_eq!(
        cached.count_within(kols_core::RecordClass::Message, newest, 60_000),
        records
            .iter()
            .filter(|record| record.body.class() == kols_core::RecordClass::Message
                && record.hlc.wall_millis > since)
            .count(),
        "and the count inside the window, which is what the ceiling asks"
    );
}

#[test]
fn a_missing_index_is_rebuilt_from_the_records_rather_than_starting_empty() {
    // Which is what makes it a cache rather than a second source of truth — and
    // is not hypothetical: no store written before this existed has the file, so
    // every channel of every existing installation takes this path once.
    let (_dir, executor, channel) = posted("rebuild", 6);
    let store = executor.store();

    let before = store.own_readings(&channel).expect("reads");
    let path = store
        .root()
        .join("channels")
        .join(intranet_crypto::to_hex(channel.as_bytes()))
        .join("own-readings");
    assert!(path.exists(), "the index was written");
    std::fs::remove_file(&path).expect("removes it");

    let after = store.own_readings(&channel).expect("rebuilds");
    assert_eq!(before, after, "a rebuild reproduces what was cached");
    assert!(path.exists(), "and writes it back");
}

#[test]
fn a_record_that_arrives_from_elsewhere_reaches_the_index() {
    // The case that decides whether this is safe. A record this member wrote
    // also arrives over the network — refetched from a segment this node
    // published and later lost, or written by another of their devices once
    // `05` §6 lands. An index that only saw the executor's writes would be
    // behind, and behind means `next_hlc` hands back a reading that is not
    // greater than one already published.
    let (_dir, executor, channel) = posted("elsewhere", 3);
    let store = executor.store();
    let identity = store.identity().expect("an identity");

    let before = store.own_readings(&channel).expect("reads");
    let ahead = kols_core::Hlc::new(before.last.expect("wrote").wall_millis + 90_000, 0);
    let arrived = kols_core::Record::create(
        &identity,
        channel,
        ahead,
        kols_core::RecordBody::Message {
            body: "written on the phone".to_owned(),
            reply_to: None,
            attachments: Vec::new(),
        },
    );
    // Straight into the store, as the sync and gossip paths do.
    assert!(store.put_record(&channel, &arrived).expect("stores"));

    let after = store.own_readings(&channel).expect("reads");
    assert_eq!(after.last, Some(ahead), "the index followed the store");
    assert_eq!(after.last_message, Some(ahead));
}

#[test]
fn the_next_reading_is_greater_than_the_last_however_the_clock_behaves() {
    // `next_hlc` is a pure function now and takes the last reading rather than a
    // segment, which is a fix as well as a saving: read off the open segment it
    // answered from nothing on a freshly sealed one, so the reading before the
    // seal was invisible and nothing would have caught the result going
    // backwards across the boundary.
    use kols_core::Hlc;
    use kols_node::chat::next_hlc;

    // Ordinary: the clock has moved on.
    assert_eq!(next_hlc(Some(Hlc::new(1_000, 4)), 2_000), Hlc::new(2_000, 0));
    // Same millisecond: the counter carries it.
    assert_eq!(next_hlc(Some(Hlc::new(2_000, 4)), 2_000), Hlc::new(2_000, 5));
    // Backwards: a clock that went back must not produce a reading that did.
    assert_eq!(next_hlc(Some(Hlc::new(9_000, 1)), 2_000), Hlc::new(9_000, 2));
    // Nothing written here yet.
    assert_eq!(next_hlc(None, 2_000), Hlc::new(2_000, 0));
}

#[test]
fn the_recent_list_stays_bounded_however_fast_a_member_writes() {
    // Bounded by time *and* by count, because time alone does not bound it: a
    // network may set no ceiling at all, and then nothing paces a member at all.
    // This file is rewritten on every record, so its size is a cost per send.
    let mut readings = OwnReadings::default();
    for n in 0..(kols_node::readings::MAX_RECENT * 3) {
        readings.note(
            kols_core::Hlc::new(1_000_000 + n as i64, 0),
            kols_core::RecordClass::Message,
        );
    }
    assert!(
        readings.recent.len() <= kols_node::readings::MAX_RECENT,
        "held {} readings",
        readings.recent.len()
    );
    // And the newest are the ones kept, since the question is always about the
    // recent past.
    assert_eq!(
        readings.last,
        Some(kols_core::Hlc::new(
            1_000_000 + (kols_node::readings::MAX_RECENT * 3 - 1) as i64,
            0
        ))
    );
}

#[test]
fn a_restart_forgets_that_a_log_was_announced_but_not_that_it_was_published() {
    // The two markers that let a publish pass skip a channel it has nothing new
    // for, and the reason they are not the same marker.
    //
    // **Chunks survive a restart and announcements do not.** The bytes are on
    // this disk; the provider records naming this node as a holder live in the
    // DHT and in a swarm that has just been replaced. A node that trusted a
    // persisted "announced recently" would come back holding content nobody
    // could find and look entirely healthy doing it — which is what this caught
    // when both markers were persisted: `three_nodes` waited twenty seconds for
    // a restarted keeper to publish and watched it skip.
    let (_dir, executor, channel) = posted("markers", 2);
    let store = executor.store();

    let reading = store.own_readings(&channel).expect("reads").last.expect("wrote");
    store.set_published_through(&channel, reading).expect("marks");
    store.set_last_announced(&channel, 1_700_000_000_000).expect("marks");

    assert_eq!(store.published_through(&channel), Some(reading));
    assert_eq!(store.last_announced(&channel), Some(1_700_000_000_000));

    store.forget_announcements().expect("forgets");

    assert_eq!(
        store.published_through(&channel),
        Some(reading),
        "what is published is a fact about the disk and survives"
    );
    assert_eq!(
        store.last_announced(&channel),
        None,
        "what is announced is a fact about a swarm that no longer exists"
    );
}
