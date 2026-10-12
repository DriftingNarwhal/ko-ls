//! D41's activity clock — `design/09` §2.
//!
//! The idle timer, a recorded sleep and "wrote since last seen" are stamps the
//! store keeps, written when a record is new or a conversation is woken. The
//! first two tests check the paths that rewrite files without anything having
//! happened: a stale re-fold, which rebuilds a channel's arrival log, and a
//! rebuilt own-readings index. While the clock read modification times, the
//! first woke a conversation on opening it and the second could leave a
//! spurious knock. The last two check a clock stepped back: while stamps were
//! ordered by time, a write after the step left no knock and a wake after it
//! did not wake.

mod common;

use kols_api::Command;
use kols_node::executor::Executor;
use kols_node::workspace::Workspace;

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-clock-{name}-{}", std::process::id()));
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

/// A keyed network with one channel this member has written in.
fn written(name: &str) -> (Dir, Executor, kols_core::ChannelId) {
    let dir = Dir::new(name);
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("clock", Vec::new()).expect("creates");
    let root = store.root().to_path_buf();
    drop(store);
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
    for n in 0..3 {
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

fn limits(slowmode_seconds: u32) -> kols_core::ReaderLimits {
    kols_core::ReaderLimits {
        message_rate_per_minute: 0,
        reaction_rate_per_minute: 0,
        message_max_bytes: 8 * 1024,
        max_future_skew_millis: 300_000,
        slowmode_seconds,
    }
}

#[test]
fn a_stale_refold_on_opening_is_not_activity() {
    // *Opening does not wake* (D41). A re-fold during a read rebuilds the
    // arrival log; that is the store repairing itself, not a message landing.
    let (_dir, executor, channel) = written("refold");
    let store = executor.store();
    store
        .load(&channel, &limits(0), &kols_core::Window::opening(10))
        .expect("folds");

    let active = store.activity().expect("records landed");
    store.mark_asleep(Some(active)).expect("records the sleep");

    // Different limits make every verdict stale, so this read re-folds the
    // channel and rewrites its arrival log.
    store
        .load(&channel, &limits(30), &kols_core::Window::opening(10))
        .expect("re-folds");

    assert_eq!(store.activity(), Some(active), "a re-fold moves nothing");
    assert!(
        store.slept_since_activity(),
        "and the recorded sleep still holds"
    );
}

#[test]
fn a_rebuilt_own_readings_index_is_not_a_write() {
    // A knock is owed for what this member wrote since the other side was last
    // seen. Rebuilding the index writes no record, so it must not count.
    let (_dir, executor, channel) = written("readings");
    let store = executor.store();
    let wrote = store.last_own_write().expect("wrote three");

    store.set_peer_seen().expect("seen after the writes");
    let path = store
        .root()
        .join("channels")
        .join(intranet_crypto::to_hex(channel.as_bytes()))
        .join("own-readings");
    std::fs::remove_file(&path).expect("removes the index");
    store.own_readings(&channel).expect("rebuilds it");
    assert!(path.exists(), "the index was written back");

    assert_eq!(
        store.last_own_write(),
        Some(wrote),
        "a rebuild moves nothing"
    );
    assert!(
        store.last_own_write() < store.peer_seen(),
        "so nothing is owed a knock"
    );
}

/// Moves the time a stamp file records by `by` (later when positive), leaving
/// the rest of the file alone: what a stamp written before the clock stepped
/// back looks like next to one written after.
fn shift(path: &std::path::Path, by: i64) {
    let text = std::fs::read_to_string(path).expect("a stamp");
    let (millis, rest) = text.split_once(' ').unwrap_or((text.trim(), ""));
    let millis: i64 = millis.trim().parse().expect("millis");
    let shifted = if rest.is_empty() {
        (millis + by).to_string()
    } else {
        format!("{} {rest}", millis + by)
    };
    std::fs::write(path, shifted).expect("rewrites the stamp");
}

const HOUR: i64 = 3600 * 1000;

#[test]
fn a_write_after_the_clock_steps_back_still_owes_a_knock() {
    // The other side was seen, the clock stepped back an hour, and this member
    // wrote. The write is newer than the sighting whatever the clock says, so
    // a knock is owed; comparing times would leave the message waiting until
    // somebody sent again.
    let (_dir, executor, channel) = written("knock-step");
    let store = executor.store();
    store.set_peer_seen().expect("seen");
    shift(&store.root().join("peer-seen"), HOUR);

    executor
        .submit(Command::SendMessage {
            channel,
            body: "after the step".to_owned(),
            reply_to: None,
            attachments: Vec::new(),
        })
        .expect("posts");
    assert!(
        store.last_own_write() > store.peer_seen(),
        "written after they were last seen, so a knock is owed"
    );
}

#[test]
fn a_wake_after_the_clock_steps_back_still_wakes() {
    // Put to sleep, then the clock stepped back an hour and a knock or a send
    // woke it. The wake is newer than the sleep whatever the clock says.
    let (_dir, executor, _channel) = written("wake-step");
    let store = executor.store();
    store
        .mark_asleep(store.activity())
        .expect("records the sleep");
    store.mark_awake().expect("a knock or a send");
    shift(&store.root().join("awake"), -HOUR);
    assert!(
        !store.slept_since_activity(),
        "woken after the sleep, so it is awake"
    );
}

#[test]
fn a_sleep_recorded_by_an_earlier_build_is_not_a_sleep() {
    // Earlier builds wrote the sleep as bare milliseconds. Read as a number,
    // that would outrank every activity for good and nothing could wake the
    // conversation; it must read as no sleep instead.
    let (_dir, executor, _channel) = written("old-sleep");
    let store = executor.store();
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_millis();
    std::fs::write(store.root().join("asleep"), millis.to_string()).expect("an old record");
    assert!(!store.slept_since_activity(), "not a sleep");
}
