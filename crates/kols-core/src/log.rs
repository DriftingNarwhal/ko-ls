//! Author logs — turning records into published storage objects.
//!
//! `design/01` §3 in code: an author's contributions to one channel are a chain
//! of segment objects behind one mutable pointer, whose id is derived rather
//! than allocated so any member can find it without a directory.
//!
//! # Why appending is cheap, and why that is the whole design
//!
//! Appending a record to an open segment re-encodes the *same object* under the
//! *same* DEK. Content-defined chunking splits on the plaintext's own bytes and
//! encryption is deterministic per (chunk, DEK), so every chunk before the edit
//! point re-derives to an identical CID and only the tail is new. A reader
//! therefore fetches the tail, not the segment.
//!
//! That property is what makes a chat log affordable on a storage layer built
//! for documents, and it is inherited rather than invented — Storage Spec §1.2
//! requires exactly this ordering (chunk plaintext, then encrypt
//! deterministically) for exactly this reason. [`AuthorLog::append`] returns
//! what actually changed, so a caller can assert the property rather than trust
//! it.

use crate::{ChannelId, CoreError, Record, Segment, author_log_pointer};
use intranet_governance::{ContentType, GovernanceState, PointerId};
use intranet_identity::PerNetworkIdentity;
use intranet_storage::{Cid, ChunkSpec, Dek, EncodedObject, Manifest, MutablePointer};

/// The content type an author log is published under.
///
/// Both publish gates apply to it like any other type (Core §2.8): the network's
/// allowlist must include `chat-log`, and the author must hold
/// `publish:chat-log`. Losing that capability freezes an author's existing logs
/// — they stay readable and servable, and take no new versions — which is
/// exactly the behaviour a timeout wants, and it comes free.
pub const CHAT_LOG_CONTENT_TYPE: &str = "chat-log";

/// What one publish of a segment produced.
#[derive(Debug, Clone)]
pub struct Published {
    /// The encoded object: manifest plus every chunk's ciphertext.
    pub object: EncodedObject,
    /// The signed pointer naming it.
    pub pointer: MutablePointer,
    /// Chunks this publish introduced that the previous version did not have.
    ///
    /// The measurement `design/07` §3 criterion 2 asserts on. For an append to a
    /// segment this should be one or two chunks regardless of how long the
    /// segment already is; anything else means content-defined chunking is not
    /// doing what the design assumes.
    pub new_chunks: Vec<Cid>,
}

impl Published {
    /// Bytes a reader holding the previous version must fetch.
    pub fn new_bytes(&self) -> usize {
        self.object
            .chunks
            .iter()
            .filter(|(cid, _)| self.new_chunks.contains(cid))
            .map(|(_, bytes)| bytes.len())
            .sum()
    }

    /// Bytes a reader holding nothing must fetch.
    pub fn total_bytes(&self) -> usize {
        self.object.chunks.iter().map(|(_, b)| b.len()).sum()
    }
}

/// One author's log for one channel, as it exists on the writing node.
///
/// Holds the open segment and the DEK. The DEK is fixed for the object's
/// lifetime by construction (Storage §1.2) — changing it would give every chunk
/// a new CID and destroy the delta property this type exists to provide.
///
/// `Debug` is hand-written rather than derived: `Dek` deliberately implements no
/// `Debug` and no serialization, so that key material cannot reach a log line by
/// accident. Deriving here would have required weakening that, which is exactly
/// backwards — the type's own `commitment()` is the thing safe to print.
pub struct AuthorLog {
    channel: ChannelId,
    pointer_id: PointerId,
    dek: Dek,
    spec: ChunkSpec,
    segment: Segment,
    pointer: Option<MutablePointer>,
    previous_manifest: Option<Manifest>,
}

impl std::fmt::Debug for AuthorLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorLog")
            .field("channel", &self.channel.short())
            .field("pointer_id", &self.pointer_id.short())
            .field("dek_commitment", &self.dek.commitment().short())
            .field("sequence", &self.segment.sequence)
            .field("records", &self.segment.records.len())
            .finish()
    }
}

impl AuthorLog {
    /// Opens a log for an author in a channel.
    pub fn open(
        author: &PerNetworkIdentity,
        channel: ChannelId,
        dek: Dek,
        spec: ChunkSpec,
    ) -> Self {
        let author_id = author.id();
        Self {
            channel,
            pointer_id: author_log_pointer(&channel, &author_id),
            dek,
            spec,
            segment: Segment::new(channel, author_id, 0, None),
            pointer: None,
            previous_manifest: None,
        }
    }

    /// The derived pointer this log publishes under.
    pub const fn pointer_id(&self) -> &PointerId {
        &self.pointer_id
    }

    /// The open segment.
    pub const fn segment(&self) -> &Segment {
        &self.segment
    }

    /// Appends a record and republishes the open segment.
    ///
    /// Refuses a record that does not belong to this log, and one whose reading
    /// does not strictly increase — both are invalid under `design/08` §4.1, and
    /// catching them here means an author never publishes history that readers
    /// will refuse.
    pub fn append(
        &mut self,
        author: &PerNetworkIdentity,
        record: Record,
        state: &GovernanceState,
    ) -> Result<Published, CoreError> {
        if record.channel != self.channel || record.author != author.id() {
            return Err(CoreError::BadSignature);
        }
        if let Some(last) = self.segment.records.last()
            && record.hlc <= last.hlc
        {
            return Err(CoreError::NonMonotonicClock {
                previous: last.hlc,
                offered: record.hlc,
            });
        }

        self.segment.records.push(record);
        self.publish(author, state)
    }

    /// Seals the open segment and starts the next one.
    ///
    /// The new segment references the sealed one by CID, which is what makes
    /// history walkable backwards and gaps detectable without consulting the
    /// pointer's version history.
    pub fn seal(&mut self, sealed_manifest_cid: Cid) {
        self.segment = Segment::new(
            self.channel,
            self.segment.author,
            self.segment.sequence + 1,
            Some(sealed_manifest_cid),
        );
        self.previous_manifest = None;
    }

    /// Encodes and signs the current segment as a new pointer version.
    fn publish(
        &mut self,
        author: &PerNetworkIdentity,
        state: &GovernanceState,
    ) -> Result<Published, CoreError> {
        let object = intranet_storage::encode(&self.segment.canonical_bytes(), &self.dek, self.spec);

        let new_chunks = match &self.previous_manifest {
            Some(previous) => object.new_chunks_since(previous),
            None => object.manifest.chunks.clone(),
        };

        let pointer = match &self.pointer {
            None => MutablePointer::publish(
                author,
                self.pointer_id,
                ContentType::new(CHAT_LOG_CONTENT_TYPE),
                object.manifest_cid(),
                self.dek.commitment(),
                state,
            ),
            Some(prior) => prior.update(author, object.manifest_cid(), state),
        }
        .map_err(CoreError::Storage)?;

        self.pointer = Some(pointer.clone());
        self.previous_manifest = Some(object.manifest.clone());

        Ok(Published {
            object,
            pointer,
            new_chunks,
        })
    }

    /// The current signed pointer, once anything has been published.
    pub const fn pointer(&self) -> Option<&MutablePointer> {
        self.pointer.as_ref()
    }
}
