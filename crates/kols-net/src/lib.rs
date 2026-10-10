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
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    /// The pointer named a segment this node could not assemble.
    #[error("{} chunk(s) could not be fetched", .0.len())]
    ChunksUnavailable(Vec<intranet_storage::Cid>),
    /// The fetched bytes did not decode as a segment.
    #[error("segment did not decode: {0}")]
    Malformed(kols_core::CoreError),
    /// The storage layer refused an operation.
    #[error("storage refused: {0}")]
    Storage(intranet_storage::StorageError),
    /// No pointer for that author log is known yet.
    #[error("no pointer known for that author log")]
    NoSuchPointer,
    /// The segment is larger than the network's `chat:segment-max-bytes`.
    ///
    /// Spec 07 §2.2: a reader refuses it, so no author can make every reader
    /// fetch an arbitrarily large object. Judged from the manifest's declared
    /// length, before a single chunk is wanted.
    #[error("segment is {claimed} bytes, past this network's {limit}-byte ceiling")]
    TooLarge {
        /// The length the manifest declares.
        claimed: u64,
        /// The ceiling it exceeds.
        limit: usize,
    },
    /// The manifest names more chunks than it declares bytes, which no honest
    /// one does, since every chunk holds at least one.
    ///
    /// The cheap half of holding an *understated* length to the ceiling. Storage
    /// refuses a length that does not match the bytes, but only once they are
    /// all held, so a manifest declaring one byte over a long chunk list is
    /// otherwise fetched in full before it is refused. This bounds the list. It
    /// does not bound the chunks' own sizes, which only a byte budget in the
    /// fetch itself could (`STATUS.md` O28).
    #[error("segment manifest names {chunks} chunk(s) for {declared} byte(s)")]
    Implausible {
        /// How many chunks it names.
        chunks: usize,
        /// How many bytes it declares.
        declared: u64,
    },
}

impl NetError {
    /// Whether this is a refusal of the segment itself, as opposed to a segment
    /// not yet held. A refusal is final: nothing more about it is wanted, so a
    /// caller stops waiting for it rather than queueing it again.
    pub const fn refuses(&self) -> bool {
        matches!(self, Self::TooLarge { .. } | Self::Implausible { .. })
    }
}
