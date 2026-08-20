//! Building an author's log out of the records this node stored.
//!
//! What is left here after the executor took the commands: the two facts about
//! an author log that both the executor and the daemon need, and the one clock
//! read in the program.

use crate::store::Store;
use intranet_governance::GovernanceState;
use intranet_identity::PerNetworkIdentity;
use intranet_storage::ChunkSpec;
use kols_core::{AuthorLog, ChannelId, Hlc};

/// Wall-clock now, in milliseconds.
///
/// The one place this program reads a clock. Everything downstream takes a
/// timestamp as an argument, deliberately, so ordering stays a function of
/// explicit inputs rather than of when code happened to run.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Rebuilds an author log from the records this node wrote.
///
/// An author log is single-writer and append-only, so replaying our own records
/// in order reproduces the same segment, the same chunks and the same CIDs —
/// chunk encryption is deterministic per (chunk, DEK), which is the same property
/// that makes a reader's delta-fetch work.
pub fn rebuild_log(
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
pub fn next_hlc(log: &AuthorLog, wall: i64) -> Hlc {
    match log.segment().records.last() {
        Some(last) if wall <= last.hlc.wall_millis => {
            Hlc::new(last.hlc.wall_millis, last.hlc.counter + 1)
        }
        _ => Hlc::new(wall, 0),
    }
}
