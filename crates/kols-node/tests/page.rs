//! Reading a page instead of a channel — `design/05` §5.
//!
//! The projection's whole purpose is that a page costs a page. These drive the
//! store's read path end to end: fold the records in, ask for a page, and get
//! back those records, the ones acting on them, and what the rate pass refused.

mod common;

use kols_api::Command;
use kols_node::executor::Executor;
use kols_node::workspace::Workspace;

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-page-{name}-{}", std::process::id()));
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

fn channel_with(name: &str, messages: usize) -> (Dir, Executor, kols_core::ChannelId) {
    let dir = Dir::new(name);
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("page", Vec::new()).expect("creates");
    let root = store.root().to_path_buf();
    drop(store);
    common::brief_run(root.clone());
    let executor = Executor::open(root).expect("opens");

    executor
        .submit(Command::SetChatSetting {
            setting: kols_core::ChatSetting::MessageRate,
            value: 0,
        })
        .expect("no rate ceiling");
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

fn limits() -> kols_core::ReaderLimits {
    kols_core::ReaderLimits {
        message_rate_per_minute: 0,
        reaction_rate_per_minute: 0,
        message_max_bytes: 8 * 1024,
        max_future_skew_millis: 300_000,
        slowmode_seconds: 0,
    }
}

#[test]
fn a_page_is_the_newest_records_and_not_the_channel() {
    let (_dir, executor, channel) = channel_with("newest", 40);
    let store = executor.store();

    let (page, refused) = store
        .page(&channel, &limits(), None, 10)
        .expect("the index answers");

    assert_eq!(page.len(), 10, "a page of ten is ten records, not forty");
    assert!(refused.is_empty(), "nothing was over any ceiling");

    // The *newest* ten, in merge order. Taking the oldest would be the easy
    // mistake and would show somebody the start of a conversation when they
    // opened it.
    let bodies: Vec<_> = page
        .iter()
        .filter_map(|record| match &record.body {
            kols_core::RecordBody::Message { body, .. } => Some(body.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(bodies.first().map(String::as_str), Some("message 30"));
    assert_eq!(bodies.last().map(String::as_str), Some("message 39"));
}

#[test]
fn a_page_carries_what_acts_on_it_however_far_away_that_is() {
    // The reason the index needs a second key. An edit, withdrawal or reaction
    // may sit anywhere later in history, and a page that read only its own
    // records would render a message that had been withdrawn an hour after it.
    let (_dir, executor, channel) = channel_with("targets", 5);
    let store = executor.store();

    let first = store
        .page(&channel, &limits(), None, 5)
        .expect("the index answers")
        .0
        .first()
        .map(kols_core::Record::id)
        .expect("a record");

    // React to the oldest message, then bury it under later traffic.
    executor
        .submit(Command::React {
            channel,
            target: first,
            key: "wave".to_owned(),
            remove: false,
        })
        .expect("reacts");
    for n in 0..30 {
        executor
            .submit(Command::SendMessage {
                channel,
                body: format!("later {n}"),
                reply_to: None,
                attachments: Vec::new(),
            })
            .expect("posts");
    }

    // A page holding the oldest message must carry the reaction, which by now is
    // thirty records away from it.
    let (page, _) = store
        .page(&channel, &limits(), None, usize::MAX)
        .expect("the index answers");
    assert!(
        page.iter().any(|record| matches!(
            &record.body,
            kols_core::RecordBody::Reaction { target, .. } if *target == first
        )),
        "the reaction acting on the page's message has to come with it"
    );
}

#[test]
fn a_record_that_arrives_late_is_folded_in_rather_than_missed() {
    // The index is derived, so it has to notice records it did not see written.
    // Sending more and asking again is the ordinary case; the interesting half
    // is that the count is what notices, not an assumption that writes go
    // through one path.
    let (_dir, executor, channel) = channel_with("late", 4);
    let store = executor.store();

    let (before, _) = store.page(&channel, &limits(), None, usize::MAX).expect("answers");
    executor
        .submit(Command::SendMessage {
            channel,
            body: "afterwards".to_owned(),
            reply_to: None,
            attachments: Vec::new(),
        })
        .expect("posts");
    let (after, _) = store.page(&channel, &limits(), None, usize::MAX).expect("answers");

    assert_eq!(after.len(), before.len() + 1, "the new record was folded in");
    assert!(after.iter().any(|record| matches!(
        &record.body,
        kols_core::RecordBody::Message { body, .. } if body == "afterwards"
    )));
}

#[test]
fn a_refusal_survives_being_read_back() {
    // The verdict is decided once, in merge order, and stored — which is the
    // whole reason a page can be rendered without folding the channel.
    let (_dir, executor, channel) = channel_with("refused", 0);
    let store = executor.store();

    // A ceiling of two, and four messages inside one window.
    for n in 0..4 {
        executor
            .submit(Command::SendMessage {
                channel,
                body: format!("burst {n}"),
                reply_to: None,
                attachments: Vec::new(),
            })
            .expect("posts");
    }
    let strict = kols_core::ReaderLimits {
        message_rate_per_minute: 2,
        ..limits()
    };
    let (page, refused) = store
        .page(&channel, &strict, None, usize::MAX)
        .expect("answers");

    assert_eq!(page.len(), 4, "every record is still in the set");
    assert_eq!(refused.len(), 2, "and the two past the ceiling are refused");
}

#[test]
fn changing_the_limits_re_folds_rather_than_leaving_a_stale_refusal() {
    // A verdict is only true of the rules that produced it. A refusal left
    // standing after the ceiling was raised is a message hidden by a rule that
    // no longer exists, which nothing else in the system would ever correct.
    let (_dir, executor, channel) = channel_with("relaxed", 0);
    let store = executor.store();
    for n in 0..4 {
        executor
            .submit(Command::SendMessage {
                channel,
                body: format!("burst {n}"),
                reply_to: None,
                attachments: Vec::new(),
            })
            .expect("posts");
    }

    let strict = kols_core::ReaderLimits {
        message_rate_per_minute: 2,
        ..limits()
    };
    let (_, refused) = store.page(&channel, &strict, None, usize::MAX).expect("answers");
    assert_eq!(refused.len(), 2);

    // Raised. Nothing about the records changed, and every refusal must go.
    let (_, refused) = store.page(&channel, &limits(), None, usize::MAX).expect("answers");
    assert!(
        refused.is_empty(),
        "raising the ceiling has to un-refuse what it refused"
    );
}
