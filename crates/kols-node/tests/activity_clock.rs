//! D41's activity clock — `design/09` §2.
//!
//! The idle timer, a recorded sleep and "wrote since last seen" are times the
//! store keeps, stamped when a record is new or a conversation is woken. These
//! check the two paths that rewrite files without anything having happened: a
//! stale re-fold, which rebuilds a channel's arrival log, and a rebuilt
//! own-readings index. While the clock read modification times, the first woke
//! a conversation on opening it and the second could leave a spurious knock.

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

    let active = store.last_activity().expect("records landed");
    std::thread::sleep(std::time::Duration::from_millis(20));
    store
        .mark_asleep(std::time::SystemTime::now())
        .expect("records the sleep");
    std::thread::sleep(std::time::Duration::from_millis(20));

    // Different limits make every verdict stale, so this read re-folds the
    // channel and rewrites its arrival log.
    store
        .load(&channel, &limits(30), &kols_core::Window::opening(10))
        .expect("re-folds");

    assert_eq!(
        store.last_activity(),
        Some(active),
        "a re-fold moves nothing"
    );
    assert!(
        store.slept_since_activity(std::time::SystemTime::now()),
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

    std::thread::sleep(std::time::Duration::from_millis(20));
    store.set_peer_seen().expect("seen after the writes");
    let path = store
        .root()
        .join("channels")
        .join(intranet_crypto::to_hex(channel.as_bytes()))
        .join("own-readings");
    std::fs::remove_file(&path).expect("removes the index");
    std::thread::sleep(std::time::Duration::from_millis(20));
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
