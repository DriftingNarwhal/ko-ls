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

use crate::{ChannelId, CoreError, Record, Segment, author_log_pointer, author_segment_pointer};
use intranet_governance::{ContentType, GovernanceState, PointerId};
use intranet_identity::PerNetworkIdentity;
use intranet_storage::{AppendOnlyObject, Cid, ChunkSpec, Dek, EncodedObject, Manifest, MutablePointer};

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
    ///
    /// **Shared rather than copied**, because a publish per append would
    /// otherwise copy the whole segment each time — which is the same cost the
    /// incremental encoding was built to remove, arriving one layer up. The
    /// encoder mutates through `Arc::make_mut`, so a caller still holding a
    /// previous publish gets a copy at that moment and nobody sees an object
    /// change under them.
    pub object: std::sync::Arc<EncodedObject>,
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
    /// The segment's encoding, extended rather than redone on each append.
    ///
    /// Held behind an `Arc` so that handing it to a caller costs a refcount
    /// rather than a copy of the segment.
    ///
    /// Chunking and sealing the whole segment per append is what made a publish
    /// cost the size of everything already in it — the hundred-thousandth record
    /// paying for the ninety-nine thousand before it. Content-defined chunking
    /// settles every boundary but the last, so an append can only move the tail
    /// (Storage §1.3), and [`AppendOnlyObject`] is that property made usable.
    ///
    /// Reset wherever the bytes ahead of the records change — at a seal, which
    /// gives the segment a new sequence and predecessor, and at a rebase, which
    /// replaces the record set outright.
    encoder: std::sync::Arc<AppendOnlyObject>,
    /// Records pushed since the last publish.
    ///
    /// A pointer version is only reachable through the record before it, so a
    /// publish that covers several pushes has to walk the version forward once
    /// per record rather than jumping. Counting them is what lets pushing and
    /// publishing come apart without the version meaning something different
    /// than it did when every push published.
    unpublished: u64,
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

/// Publishes an author's **head index** for one channel.
///
/// # Why this exists
///
/// Segments live under [`author_segment_pointer`], one pointer and therefore one
/// key each — which is what lets old history be forgotten without forgetting all
/// of it (see [`AuthorLog::seal`]). That costs a reader the one thing a single
/// fixed pointer gave for free: knowing *which* segment is currently the head.
/// A reader can derive where segment `n` lives, but not what `n` is.
///
/// So `author_log_pointer` — the derivation a reader can always compute from
/// public information — names this instead: an otherwise empty segment whose
/// `sequence` is the head's. One indirection, and then everything else is
/// derivable again.
///
/// It carries no records deliberately. The index changes only when a segment is
/// sealed, not on every append, so a reader that already knows where the head is
/// refetches nothing while a conversation goes on.
///
/// # The version is the sequence
///
/// Not incidental. Two pointer records at the same version are settled by lower
/// record hash (Storage §2.2), so an index republished at version 0 with a newer
/// sequence would lose that coin-flip against the copy peers already hold, about
/// half the time, and the author's newer history would simply never be found.
/// Tying the version to the sequence makes every advance strictly supersede.
pub fn publish_head_index(
    author: &PerNetworkIdentity,
    channel: ChannelId,
    head_sequence: u64,
    dek: &Dek,
    spec: ChunkSpec,
    state: &GovernanceState,
) -> Result<Published, CoreError> {
    let author_id = author.id();
    let index = Segment::new(channel, author_id, head_sequence, None);
    let object = intranet_storage::encode(&index.canonical_bytes(), dek, spec);
    let pointer_id = author_log_pointer(&channel, &author_id);

    let mut pointer = MutablePointer::publish(
        author,
        pointer_id,
        ContentType::new(CHAT_LOG_CONTENT_TYPE),
        object.manifest_cid(),
        dek.commitment(),
        state,
    )
    .map_err(CoreError::Storage)?;
    // Walked up rather than set, because a version is only reachable through the
    // record that precedes it — there is no way to sign "version n" directly, and
    // there should not be: it is what stops a publisher from parking a pointer at
    // a version nothing can ever supersede.
    for _ in 0..head_sequence {
        pointer = pointer
            .update(author, object.manifest_cid(), state)
            .map_err(CoreError::Storage)?;
    }

    let new_chunks = object.manifest.chunks.clone();
    Ok(Published {
        object: std::sync::Arc::new(object),
        pointer,
        new_chunks,
    })
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
        let segment = Segment::new(channel, author_id, 0, None);
        let mut encoder = AppendOnlyObject::new(spec);
        encoder.extend(&segment.header_bytes(), &dek);
        let encoder = std::sync::Arc::new(encoder);
        Self {
            channel,
            pointer_id: author_segment_pointer(&channel, &author_id, 0),
            dek,
            spec,
            segment,
            pointer: None,
            previous_manifest: None,
            unpublished: 0,
            encoder,
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
        self.push(author, record)?;
        self.publish_current(author, state)
    }

    /// Appends a record **without** publishing.
    ///
    /// The same checks [`append`](Self::append) makes and none of its work. It
    /// exists because rebuilding a log means replaying every record the author
    /// ever wrote in the channel, and publishing per record makes that
    /// **quadratic**: each publish re-chunks, re-encrypts and re-hashes the
    /// whole segment so far, so replaying `n` records does `n²/2` records' worth
    /// of cryptography to reach a state one encode of the final segment produces
    /// identically — and then discards every intermediate answer.
    ///
    /// Measured before this existed: a send in a channel holding a thousand of
    /// the author's own records cost 298 ms, against 2.5 ms at twenty-five
    /// (`design/05` §5). Nothing bounded it — not sealing, which this walks
    /// past, and not retention, which drops what is published rather than what
    /// is stored.
    ///
    /// A caller that pushes must publish before the result is used for anything,
    /// which is why this returns nothing worth keeping: there is no half-written
    /// state to be handed around, only a segment that has not been encoded yet.
    pub fn push(&mut self, author: &PerNetworkIdentity, record: Record) -> Result<(), CoreError> {
        if record.channel != self.channel || record.author != author.id() {
            return Err(CoreError::BadSignature);
        }
        // Against this device's own last reading, not the segment's: after a
        // rebase (§`rebase`) a segment interleaves two devices, and the other
        // device's readings are not this one's to advance past.
        if let Some(last) = self
            .segment
            .records
            .iter()
            .rev()
            .find(|existing| existing.device == record.device)
            && record.hlc <= last.hlc
        {
            return Err(CoreError::NonMonotonicClock {
                previous: last.hlc,
                offered: record.hlc,
            });
        }

        let dek = self.dek.clone();
        std::sync::Arc::make_mut(&mut self.encoder).extend(&crate::framed(&record), &dek);
        self.segment.records.push(record);
        self.unpublished += 1;
        Ok(())
    }

    /// Seals the open segment and starts the next one under a fresh key.
    ///
    /// The new segment references the sealed one by CID, which is what makes
    /// history walkable backwards and gaps detectable without consulting the
    /// pointer's version history.
    ///
    /// # Why a new DEK, and a new pointer to hold it
    ///
    /// This is what `design/01` §8 needs to be able to drop old history without
    /// dropping all of it. A pointer commits to one DEK for its entire life —
    /// `MutablePointer::update` carries the commitment forward, deliberately,
    /// because Storage §1.2 fixes a DEK for its object's lifetime — so segments
    /// sharing a pointer share a key, and a key that opens the newest message
    /// also opens the oldest. Forgetting is then all-or-nothing.
    ///
    /// A pointer per segment gives a key per segment. An author who stops
    /// re-wrapping one segment makes exactly that segment dark (Storage §5.2)
    /// and leaves everything newer readable. The sealed segment keeps the key it
    /// was written under, so nothing is ever re-encrypted: `dek` changes only
    /// for the segment starting *now*, whose bytes do not exist yet.
    pub fn seal(&mut self, sealed_manifest_cid: Cid, next: Dek) {
        let sequence = self.segment.sequence + 1;
        self.segment = Segment::new(
            self.channel,
            self.segment.author,
            sequence,
            Some(sealed_manifest_cid),
        );
        self.pointer_id = author_segment_pointer(&self.channel, &self.segment.author, sequence);
        self.dek = next;
        // Both cleared because they describe the sealed segment, not this one. A
        // retained pointer would republish the new segment as the next *version*
        // of the old one, and a retained manifest would compute `new_chunks`
        // against an object encrypted under a key this segment does not use.
        self.pointer = None;
        self.previous_manifest = None;
        // The new segment has had nothing pushed to it. Zero already, since
        // sealing follows a publish, and set anyway rather than relying on a
        // caller's ordering to keep a version count honest.
        self.unpublished = 0;
        // A new sequence and a new predecessor mean different header bytes and a
        // different key, so nothing the previous encoding settled applies here.
        let dek = self.dek.clone();
        let mut fresh = AppendOnlyObject::new(self.spec);
        fresh.extend(&self.segment.header_bytes(), &dek);
        self.encoder = std::sync::Arc::new(fresh);
    }

    /// Encodes and signs the current segment as a new pointer version.
    ///
    /// **One encode however many records were pushed**, which is the whole point
    /// of [`push`](Self::push) existing beside [`append`](Self::append). The
    /// pointer version still advances once per record, because a version is only
    /// reachable through the one before it — but signing a small record `n`
    /// times is not the same order of work as encrypting a growing segment `n`
    /// times, and only the last of those pointers is ever announced.
    pub fn publish_current(
        &mut self,
        author: &PerNetworkIdentity,
        state: &GovernanceState,
    ) -> Result<Published, CoreError> {
        let object = self.encoder.shared();

        let new_chunks = match &self.previous_manifest {
            Some(previous) => object.new_chunks_since(previous),
            None => object.manifest.chunks.clone(),
        };

        // At least one, so publishing a segment nothing was pushed to still
        // advances the version — which is what an unconditional republish on
        // every tick has always done.
        let versions = self.unpublished.max(1);
        let mut pointer = match &self.pointer {
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
        for _ in 1..versions {
            pointer = pointer
                .update(author, object.manifest_cid(), state)
                .map_err(CoreError::Storage)?;
        }
        self.unpublished = 0;

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

    /// Recovers from losing a version collision, without losing records.
    ///
    /// # The gap this fills
    ///
    /// Storage §2.2 settles which of two same-version pointer records is
    /// canonical — lower record hash wins — and then says plainly that it
    /// "supplies no content-merge semantics", leaving what two concurrent edits
    /// mean together to the application layer. This is that decision, for author
    /// logs.
    ///
    /// It is easy because of how the log is shaped: a segment is a set of
    /// independently signed, content-addressed records, so merging two versions
    /// is a union by id and a sort, with no field-level conflict to invent a
    /// rule for. The loser adopts the winner's pointer and republishes the union
    /// at the next version. **Nothing is dropped** — which matters, because the
    /// loser's records were validly published and their author has no way to
    /// know they lost except by being told.
    ///
    /// The winner's manifest is recomputed locally rather than fetched: chunk
    /// encryption is deterministic per (chunk, DEK), so re-encoding the winning
    /// segment reproduces the winner's exact chunk set. That is what keeps
    /// `new_chunks` meaningful across a rebase instead of reporting the whole
    /// object.
    pub fn rebase(
        &mut self,
        author: &PerNetworkIdentity,
        canonical: &MutablePointer,
        canonical_segment: &Segment,
        state: &GovernanceState,
    ) -> Result<Published, CoreError> {
        if canonical.pointer_id != self.pointer_id
            || canonical_segment.channel != self.channel
            || canonical_segment.author != self.segment.author
        {
            return Err(CoreError::BadSignature);
        }

        let mut union: std::collections::BTreeMap<(crate::Hlc, crate::MessageId), Record> =
            canonical_segment
                .records
                .iter()
                .chain(self.segment.records.iter())
                .map(|record| ((record.hlc, record.id()), record.clone()))
                .collect();

        self.segment.sequence = canonical_segment.sequence;
        self.segment.previous = canonical_segment.previous;
        self.segment.records = std::mem::take(&mut union).into_values().collect();

        self.pointer = Some(canonical.clone());
        self.previous_manifest = Some(
            intranet_storage::encode(&canonical_segment.canonical_bytes(), &self.dek, self.spec)
                .manifest,
        );

        // **Rebuilt whole rather than extended, because this is not an append.**
        // A rebase replaces the record set with a union of two, so records may
        // land ahead of ones already encoded — the one case where nothing before
        // the tail is settled. Cheap by comparison and rare by construction: it
        // happens when two devices published the same log concurrently, which is
        // `design/01` §3.1.1's case and not a per-record one.
        let dek = self.dek.clone();
        let mut fresh = AppendOnlyObject::new(self.spec);
        fresh.extend(&self.segment.canonical_bytes(), &dek);
        self.encoder = std::sync::Arc::new(fresh);

        self.publish_current(author, state)
    }
}
