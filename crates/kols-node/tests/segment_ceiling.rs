//! `chat:segment-max-bytes` on the writer's side — spec 07 §2.2.
//!
//! Readers refuse a segment past the network's ceiling, so a writer that
//! produced one would be publishing history nobody reads. Sealing keeps a
//! segment under it (`serve::should_seal`, tested beside it); what sealing
//! cannot help is a record too large to fit in any segment, and that is refused
//! when it is written.

mod common;

use kols_api::Command;
use kols_node::executor::Executor;
use kols_node::workspace::Workspace;

struct Dir(std::path::PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn keyed(name: &str) -> (Dir, Executor, kols_core::ChannelId) {
    let path = std::env::temp_dir().join(format!("kols-ceiling-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a workspace");
    let dir = Dir(path);
    let store = Workspace::at(dir.0.clone())
        .create("ceiling", Vec::new())
        .expect("creates");
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
    (dir, executor, channel)
}

fn say(executor: &Executor, channel: kols_core::ChannelId, body: &str) -> Result<(), String> {
    executor
        .submit(Command::SendMessage {
            channel,
            body: body.to_owned(),
            reply_to: None,
            attachments: Vec::new(),
        })
        .map(|_| ())
        .map_err(|err| err.to_string())
}

#[test]
fn a_record_no_segment_could_hold_is_refused_when_written() {
    let (_dir, executor, channel) = keyed("refuse");
    say(&executor, channel, "fits under the shipped ceiling").expect("posts");

    // A ceiling smaller than one short message: a segment holding only this
    // record, header and all, would still be past it.
    executor
        .submit(Command::SetChatSetting {
            setting: kols_core::ChatSetting::SegmentMaxBytes,
            value: 300,
        })
        .expect("sets the ceiling");
    let refusal = say(&executor, channel, "too large for any segment here")
        .expect_err("a record no reader would accept must not be written");
    assert!(
        refusal.contains("larger than a segment may be"),
        "{refusal}"
    );

    // And refused means not stored: the next ceiling that admits it finds one
    // record written, not two.
    executor
        .submit(Command::SetChatSetting {
            setting: kols_core::ChatSetting::SegmentMaxBytes,
            value: 4096,
        })
        .expect("raises the ceiling");
    say(&executor, channel, "fits again").expect("posts");
    let me = executor.store().identity().expect("has a seed").id();
    let kept = executor.store().own_records(&channel, &me).expect("reads");
    assert_eq!(kept.len(), 2, "the refused record was not kept");
}
