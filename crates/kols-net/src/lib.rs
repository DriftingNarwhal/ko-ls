//! Carrying a channel over the real transport.
//!
//! # What this crate is responsible for
//!
//! `kols-core` decides what a channel *is*: records, ordering, permissions,
//! rendering. It touches no I/O and knows nothing about libp2p. This crate is
//! the other half — putting a published segment where peers can fetch it, and
//! turning what a peer holds back into records.
//!
//! # The layering that a fetch actually depends on
//!
//! Three things must be true before a chunk fetch can succeed, and they are true
//! in this order — governance, then ledger, then fetch:
//!
//! 1. **Governance replay must admit the requester**, because serving is gated
//!    on `read-content` (Storage §5.4). A node whose log has not caught up
//!    refuses a peer it will later serve, correctly.
//! 2. **The holder must have advertised capacity**, because source selection
//!    drops a holder that never volunteered — the DHT finding somebody is not
//!    the same as that somebody having offered.
//! 3. **Then** the fetch can run.
//!
//! A fetch that mysteriously finds nothing is usually step 2, not a bug. The
//! protocol's own guidance says so, and this crate's tests are built to fail
//! loudly rather than hang when it is skipped.

#![deny(missing_docs)]

mod publish;

pub use publish::{
    PublishOutcome, fetch_segment, known_pointer, plan_fetch, publish_segment, wanted_chunks,
};

/// What can go wrong carrying a channel over the wire.
#[derive(Debug)]
pub enum NetError {
    /// The pointer named a segment this node could not assemble.
    ChunksUnavailable(Vec<intranet_storage::Cid>),
    /// The fetched bytes did not decode as a segment.
    Malformed(kols_core::CoreError),
    /// The storage layer refused an operation.
    Storage(intranet_storage::StorageError),
    /// No pointer for that author log is known yet.
    NoSuchPointer,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChunksUnavailable(cids) => {
                write!(f, "{} chunk(s) could not be fetched", cids.len())
            }
            Self::Malformed(err) => write!(f, "segment did not decode: {err}"),
            Self::Storage(err) => write!(f, "storage refused: {err}"),
            Self::NoSuchPointer => write!(f, "no pointer known for that author log"),
        }
    }
}

impl std::error::Error for NetError {}
