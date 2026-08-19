//! The commands that actually move a message.

use crate::network;
use crate::store::Store;
use intranet_crypto::{Timestamp, to_hex};
use intranet_governance::{GovernanceState, LogEntry};
use intranet_identity::PerNetworkIdentity;
use intranet_storage::ChunkSpec;
use kols_core::{
    AuthorLog, Authority, ChannelEntry, ChannelEntryBody, ChannelId, ChannelKind, ChatPolicy,
    ChannelView, Hlc, Placement, Privacy, Record, RecordBody, StateAuthority,
};
use std::path::PathBuf;

/// Wall-clock now, in milliseconds.
///
/// The one place this program reads a clock. Everything downstream takes a
/// timestamp as an argument, deliberately, so ordering stays a function of
/// explicit inputs rather than of when code happened to run.
pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Defines a channel and appends it to the governance log.
pub fn create_channel(
    root: PathBuf,
    name: &str,
    private: bool,
    topic: &str,
) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let author = store.identity().map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;
    network::require_server(&state)?;

    // The nonce is what makes two channels of the same name distinct objects
    // rather than one; the id is derived from it and the network id, so nobody
    // can mint a channel id that collides with an existing one.
    let nonce = crate::random_32()?;
    let channel = kols_core::server_channel_id(store.network(), &nonce);

    let entry = ChannelEntry::new(
        channel,
        ChannelEntryBody::Definition {
            name: name.to_owned(),
            category: None,
            kind: ChannelKind::Text,
            privacy: if private {
                Privacy::Private
            } else {
                Privacy::Public
            },
            topic: topic.to_owned(),
            slowmode: 0,
        },
    );

    // Declares whichever capability this author actually holds, and refuses here
    // rather than producing a log entry every node would reject.
    let body = entry
        .to_app_entry(&state, &author.id(), None)
        .map_err(|refusal| {
            format!(
                "you cannot create a channel here: {refusal}.\n\
                 A founder grants this with chat:create-channel, at the network or a category."
            )
        })?;

    // Held across reading the head and writing, so a channel definition cannot
    // land as a sibling of a rotation the daemon appended in between — which
    // forks the log, and fork-choice then voids one of them.
    let _lock = store.lock().map_err(|e| e.to_string())?;
    let head = store.head().map_err(|e| e.to_string())?;
    let log_entry = LogEntry::create(&author, head, Timestamp::from_millis(now_millis()), body);
    store.append_entry(&log_entry).map_err(|e| e.to_string())?;

    // Replay before claiming success. An entry the log accepts structurally can
    // still be refused by replay, and reporting a channel that does not exist
    // would be worse than failing.
    let state = store.state().map_err(|e| e.to_string())?;
    let (channels, _) = network::channels(&store, &state).map_err(|e| e.to_string())?;
    if !channels.contains_key(&channel) {
        return Err("the entry was written but replay did not produce the channel".to_owned());
    }

    println!("created #{name}");
    println!("  id       {}", to_hex(channel.as_bytes()));
    println!(
        "  privacy  {}",
        if private {
            "private (roster keying is not implemented yet — see design/03 §3)"
        } else {
            "public"
        }
    );
    Ok(())
}

/// Lists channels as replay understands them.
pub fn list_channels(root: PathBuf) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;
    let (channels, refused) = network::channels(&store, &state).map_err(|e| e.to_string())?;

    if channels.is_empty() {
        println!("no channels yet. `kols channel create <name>`");
    }
    for channel in channels.values() {
        let mut flags = Vec::new();
        if channel.privacy == Privacy::Private {
            flags.push("private");
        }
        if channel.archived {
            flags.push("archived");
        }
        let suffix = if flags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", flags.join(", "))
        };
        println!(
            "#{:<20} {}{}",
            channel.name,
            &to_hex(channel.id.as_bytes())[..12],
            suffix
        );
        if !channel.topic.is_empty() {
            println!("  {}", channel.topic);
        }
    }

    // Surfaced rather than swallowed: a channel entry this build refuses is one
    // some other client may be showing, and silence would make the two look
    // like they agree.
    for refusal in refused {
        eprintln!("skipped an entry: {refusal}");
    }
    Ok(())
}

/// Writes a message into a channel.
pub fn post(root: PathBuf, needle: &str, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("nothing to say".to_owned());
    }
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let author = store.identity().map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;
    let (channels, _) = network::channels(&store, &state).map_err(|e| e.to_string())?;
    let channel = network::resolve(&channels, needle)
        .ok_or_else(|| format!("no channel matching {needle:?}. `kols channel list`"))?;

    let placement = Placement {
        channel: channel.id,
        category: channel.category,
    };

    // The author's own client enforces the ceiling first, so a user who types
    // too fast is told rather than having their records silently refused by
    // every reader (`design/01` §10.2).
    let authority = StateAuthority { state: &state };
    if !authority.may_post(&author.id(), &placement) {
        return Err(format!(
            "you may not post in #{}. Posting needs chat:post at this channel, \
             its category or the network, and publish:chat-log alongside it.",
            channel.name
        ));
    }

    let policy = ChatPolicy::of(&state.policy);
    if text.len() > policy.message_max_bytes() {
        return Err(format!(
            "message is {} bytes, and this network's ceiling is {}",
            text.len(),
            policy.message_max_bytes()
        ));
    }

    let mut log = rebuild_log(&store, &author, channel.id, &state)?;
    let hlc = next_hlc(&log, now_millis());
    let record = Record::create(
        &author,
        channel.id,
        hlc,
        RecordBody::Message {
            body: text.to_owned(),
            reply_to: None,
            attachments: Vec::new(),
        },
    );
    let stored = record.clone();
    let published = log
        .append(&author, record, &state)
        .map_err(|err| format!("the record was refused: {err}"))?;

    store
        .put_record(&channel.id, &stored)
        .map_err(|e| e.to_string())?;

    println!("posted to #{}", channel.name);
    println!(
        "  moved {} of {} bytes",
        published.new_bytes(),
        published.total_bytes()
    );
    Ok(())
}

/// Renders a channel.
pub fn read(root: PathBuf, needle: &str) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;
    let (channels, _) = network::channels(&store, &state).map_err(|e| e.to_string())?;
    let channel = network::resolve(&channels, needle)
        .ok_or_else(|| format!("no channel matching {needle:?}. `kols channel list`"))?;

    let placement = Placement {
        channel: channel.id,
        category: channel.category,
    };
    let mut view = ChannelView::new(placement);
    let authority = StateAuthority { state: &state };

    // Only this member's own records, because that is all this node holds: a
    // second author's log arrives over the wire, which `kols serve` is for and
    // this build does not have yet. The merge is the same either way — a view is
    // a function of the admitted record set, not of where it came from.
    let records = store.records(&channel.id).map_err(|e| e.to_string())?;
    let authors: std::collections::BTreeSet<_> =
        records.iter().map(|record| record.author).collect();
    view.admit(records, &authority);

    let rendered = view.render();
    if rendered.is_empty() {
        println!("#{} is empty", channel.name);
    }
    for message in &rendered {
        println!(
            "[{}] {}  {}",
            stamp(message.hlc),
            message.author.short(),
            message.body
        );
    }

    for (id, rejection) in view.rejected() {
        eprintln!("refused {}: {rejection:?}", &to_hex(id.as_bytes())[..8]);
    }
    println!();
    println!(
        "{} message(s) from {} author(s). `kols serve` brings in what other members wrote.",
        rendered.len(),
        authors.len()
    );
    Ok(())
}

/// Rebuilds an author log from the records this node wrote.
///
/// An author log is single-writer and append-only, so replaying our own records
/// in order reproduces the same segment, the same chunks and the same CIDs —
/// chunk encryption is deterministic per (chunk, DEK), which is the same property
/// that makes a reader's delta-fetch work.
fn rebuild_log(
    store: &Store,
    author: &PerNetworkIdentity,
    channel: ChannelId,
    state: &GovernanceState,
) -> Result<AuthorLog, String> {
    // The DEK is per author log, not per channel: each author's log is its own
    // content object, and the pointer it publishes under is what the wrapping
    // binds to (Storage §5.3).
    let pointer = kols_core::author_log_pointer(&channel, &author.id());
    let dek = store.channel_dek(&pointer).map_err(|e| e.to_string())?;
    let mut log = AuthorLog::open(author, channel, dek, ChunkSpec::from_target(64 * 1024));
    for record in store
        .own_records(&channel, &author.id())
        .map_err(|e| e.to_string())?
    {
        log.append(author, record, state)
            .map_err(|err| format!("a stored record no longer appends: {err}"))?;
    }
    Ok(log)
}

/// The next reading for this author, strictly greater than their last.
///
/// Per (author, device) rather than per author — spec 07 §2.6, learned in P0
/// when a merged segment interleaving two devices declared every concurrent
/// recovery invalid.
fn next_hlc(log: &AuthorLog, wall: i64) -> Hlc {
    match log.segment().records.last() {
        Some(last) if wall <= last.hlc.wall_millis => {
            Hlc::new(last.hlc.wall_millis, last.hlc.counter + 1)
        }
        _ => Hlc::new(wall, 0),
    }
}

fn stamp(hlc: Hlc) -> String {
    let secs = hlc.wall_millis / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

